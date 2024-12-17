//src/bootnode.rs
use anyhow::{Context, Result};
use prometheus_parse::Sample;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU16, Ordering},
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tracing::{info, warn, error};

use crate::{
    cli::Cli,
    metrics::{MetricsResult, MetricsStatus, TestResult, TestStatus},
};

const MIN_PORT: u16 = 49152;
const MAX_PORT: u16 = 65535;
const EMOJI_SUCCESS: &str = "✅";
const EMOJI_ERROR: &str = "❌";
const EMOJI_WARNING: &str = "⚠️";
const EMOJI_LOADING: &str = "⏳";
const EMOJI_ROCKET: &str = "🚀";
const EMOJI_NETWORK: &str = "🌐";

pub(crate) static NEXT_PORT: AtomicU16 = AtomicU16::new(MIN_PORT);

pub fn get_next_port() -> u16 {
    let current = NEXT_PORT.load(Ordering::Relaxed);
    let next = if current >= MAX_PORT { MIN_PORT } else { current + 1 };
    NEXT_PORT.store(next, Ordering::Relaxed);
    current
}

#[derive(Debug)]
pub struct NodeProcess {
    process: Child,
    data_dir: PathBuf,
    prometheus_port: u16,
    p2p_port: u16,
    start_time: Instant,
    operator: String,
    network: String,
    bootnode: String,
    cli: Cli,
}

pub async fn spawn_node(
    cli: &Cli,
    operator: &str,
    network: &str,
    bootnode: &str,
    command_id: &str,
) -> Result<NodeProcess> {
    let data_dir = cli.data_dir.join(format!("{}_{}", operator, network));
    std::fs::create_dir_all(&data_dir)?;

    let relaychain = if command_id == "parachain" {
        Some(network.split('-').last().context("Invalid network name")?)
    } else {
        None
    };

    let binary = if command_id == "parachain" {
        &cli.parachain_binary
    } else {
        &cli.polkadot_binary
    };

    let chain_spec = cli.chain_spec_dir.join(format!("{}.json", network));
    if !chain_spec.exists() {
        anyhow::bail!("Chain spec file does not exist: {:?}", chain_spec);
    }

    let prometheus_port = get_next_port();
    let p2p_port = get_next_port();

    info!(
        "{} Starting node for {}/{} {} prometheus: {}, p2p: {}",
        EMOJI_ROCKET, operator, network, EMOJI_NETWORK,
        prometheus_port, p2p_port
    );

    let mut cmd = Command::new(binary);
    cmd.args(&[
        "--no-hardware-benchmarks",
        "--no-mdns",
        "--prometheus-external",
        &format!("--prometheus-port={}", prometheus_port),
        &format!("--port={}", p2p_port),
        "-d",
    ])
    .arg(&data_dir)
    .arg("--chain")
    .arg(&chain_spec)
    .arg("--bootnodes")
    .arg(bootnode);

    if let Some(relay) = relaychain {
        cmd.arg("--relay-chain-rpc-urls")
            .arg(format!("wss://{}.dotters.network/", relay));
    }

    let process = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn node process")?;

    Ok(NodeProcess {
        process,
        data_dir,
        prometheus_port,
        p2p_port,
        start_time: Instant::now(),
        bootnode: bootnode.to_string(),
        operator: operator.to_string(),
        network: network.to_string(),
        cli: cli.clone(),
    })
}

impl NodeProcess {
    pub async fn check_discovered_peers(&self) -> Result<MetricsResult> {
        let metrics = self.fetch_metrics().await?;
        let peer_data = self.parse_peer_metrics(&metrics)?;
        Ok(self.create_metrics_result(peer_data))
    }

     async fn fetch_metrics(&self) -> Result<String> {
        let metrics_url = format!("http://127.0.0.1:{}/metrics", self.prometheus_port);
        let client = reqwest::Client::new();

        for attempt in 1..=3 {
            match self.try_fetch_metrics(&client, &metrics_url).await {
                Ok(metrics) => return Ok(metrics),
                Err(e) => {
                    info!("{} Fetch attempt {}/3 failed for {}/{} (bootnode: {}) on port {}: {}", 
                        EMOJI_WARNING, attempt, self.operator, self.network, 
                        self.bootnode, self.prometheus_port, e);
                    if attempt < 3 {
                        sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "Failed to fetch metrics after 3 attempts for {}/{} (bootnode: {}) on port {}", 
            self.operator, self.network, self.bootnode, self.prometheus_port
        ))
    }

    async fn try_fetch_metrics(&self, client: &reqwest::Client, url: &str) -> Result<String> {
        let response = client
            .get(url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Bad status: {}", response.status()));
        }

        Ok(response.text().await?)
    }

    fn parse_peer_metrics(&self, metrics: &str) -> Result<HashMap<String, u64>> {
        let lines = metrics.lines().map(|l| Ok(l.to_string()));
        let scrape = prometheus_parse::Scrape::parse(lines)?;
        let mut peer_data = HashMap::new();

        for Sample { metric, value, .. } in scrape.samples {
            if let Some(count) = self.extract_gauge_value(value) {
                match metric.as_str() {
                    "substrate_sub_libp2p_peerset_num_discovered" => {
                        peer_data.insert("discovered".to_string(), count);
                    }
                    "substrate_sub_libp2p_peers_count" => {
                        peer_data.insert("connected".to_string(), count);
                    }
                    _ => {}
                }
            }
        }

        Ok(peer_data)
    }

    fn extract_gauge_value(&self, value: prometheus_parse::Value) -> Option<u64> {
        if let prometheus_parse::Value::Gauge(count) = value {
            Some(count as u64)
        } else {
            None
        }
    }

    fn create_metrics_result(&self, peer_data: HashMap<String, u64>) -> MetricsResult {
        MetricsResult {
            peers: peer_data.get("discovered").copied().unwrap_or(0),
            peer_types: Some(peer_data.clone()),
            status: if peer_data.contains_key("discovered") {
                MetricsStatus::Available
            } else {
                MetricsStatus::NoMetricFound
            },
        }
    }

    async fn bootnode_is_working(&mut self, timeout: Duration) -> Result<(u64, TestStatus, Option<String>)> {
        sleep(Duration::from_secs(5)).await;
        let end_time = Instant::now() + timeout;

        while Instant::now() < end_time {
            match self.check_discovered_peers().await {
                Ok(metrics) => match metrics.status {
                    MetricsStatus::Available if metrics.peers >= self.cli.min_peers => {
                        info!("{} Bootnode working for {}/{} - discovered {} peers",
                            EMOJI_SUCCESS, self.operator, self.network, metrics.peers);
                        return Ok((metrics.peers, TestStatus::Success, None));
                    }
                    MetricsStatus::Available => {
                        sleep(Duration::from_secs(1)).await;
                    }
                    MetricsStatus::NoMetricFound => {
                        if self.start_time.elapsed() > Duration::from_secs(5) {
                            warn!("{} No metrics found for {}/{} (bootnode: {}) on port {}", 
                                EMOJI_WARNING, self.operator, self.network, 
                                self.bootnode, self.prometheus_port);
                            return Ok((0, TestStatus::NoMetricFound, None));
                        }
                        sleep(Duration::from_secs(1)).await;
                    }
                    MetricsStatus::Unavailable(error) => {
                        if self.start_time.elapsed() > Duration::from_secs(5) {
                            error!("{} Metrics unavailable for {}/{} (bootnode: {}) on port {}: {}", 
                                EMOJI_ERROR, self.operator, self.network, 
                                self.bootnode, self.prometheus_port, error);
                            return Ok((0, TestStatus::MetricsUnavailable, Some(error)));
                        }
                        sleep(Duration::from_secs(1)).await;
                    }
                },
                Err(e) => {
                    error!("{} Error checking peers for {}/{} (bootnode: {}) on port {}: {}", 
                        EMOJI_ERROR, self.operator, self.network, 
                        self.bootnode, self.prometheus_port, e);
                    return Ok((0, TestStatus::MetricsUnavailable, Some(e.to_string())));
                }
            }
        }

        warn!("{} Timeout waiting for peer discovery for {}/{} (bootnode: {}) on port {}", 
            EMOJI_WARNING, self.operator, self.network, 
            self.bootnode, self.prometheus_port);
        Ok((0, TestStatus::Timeout, None))
    }

    pub async fn cleanup(mut self) -> Result<()> {
        let _ = self.process.kill();
        sleep(Duration::from_secs(1)).await;
        if let Ok(None) = self.process.try_wait() {
            warn!("Process still running after graceful shutdown, force killing");
            let _ = self.process.kill();
        }
        std::fs::remove_dir_all(&self.data_dir)?;
        Ok(())
    }
}

pub async fn test_bootnode(
    cli: &Cli,
    operator: &str,
    network: &str,
    bootnode: &str,
    command_id: &str,
) -> Result<TestResult> {
    let start_time = Instant::now();

    info!("{} Testing bootnode {} for {}/{}", 
        EMOJI_LOADING, bootnode, operator, network);

    let mut node = match spawn_node(cli, operator, network, bootnode, command_id).await {
        Ok(node) => node,
        Err(e) => {
            error!("{} Node startup failed for {}/{}: {}", 
                EMOJI_ERROR, operator, network, e);
            return Ok(TestResult {
                id: operator.to_string(),
                network: network.to_string(),
                bootnode: bootnode.to_string(),
                valid: false,
                test_duration_ms: start_time.elapsed().as_millis() as u64,
                discovered_peers: 0,
                status: TestStatus::NodeStartupFailed,
                error_details: Some(e.to_string()),
            });
        }
    };

    let (discovered_peers, status, error_details) =
        node.bootnode_is_working(Duration::from_secs(cli.timeout)).await?;
    let duration = start_time.elapsed().as_millis() as u64;

    node.cleanup().await?;

    Ok(TestResult {
        id: operator.to_string(),
        network: network.to_string(),
        bootnode: bootnode.to_string(),
        valid: discovered_peers > cli.min_peers,
        test_duration_ms: duration,
        discovered_peers,
        status,
        error_details,
    })
}
