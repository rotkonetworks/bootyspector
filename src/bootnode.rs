//src/bootnode.rs
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU16, Ordering},
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tracing::{error, info, warn};

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
    let next = if current >= MAX_PORT {
        MIN_PORT
    } else {
        current + 1
    };
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
        EMOJI_ROCKET, operator, network, EMOJI_NETWORK, prometheus_port, p2p_port
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

    pub async fn check_discovered_peers(&self) -> Result<MetricsResult> {
        const MAX_RETRIES: u32 = 5;
        const INITIAL_BACKOFF: Duration = Duration::from_millis(100);

        for retry in 0..MAX_RETRIES {
            match self.fetch_metrics().await {
                Ok(metrics) => match self.parse_peer_metrics(&metrics) {
                    Ok(peer_data) => return Ok(self.create_metrics_result(peer_data)),
                    Err(e) => {
                        warn!(
                            "{} Failed to parse metrics on attempt {}/{}: {}",
                            EMOJI_WARNING,
                            retry + 1,
                            MAX_RETRIES,
                            e
                        );
                        if retry == MAX_RETRIES - 1 {
                            return Err(e);
                        }
                    }
                },
                Err(e) => {
                    warn!(
                        "{} Failed to fetch metrics on attempt {}/{}: {}",
                        EMOJI_WARNING,
                        retry + 1,
                        MAX_RETRIES,
                        e
                    );
                    if retry == MAX_RETRIES - 1 {
                        return Err(e);
                    }
                }
            }

            let backoff = INITIAL_BACKOFF * 2u32.pow(retry);
            let jitter = rand::random::<u64>() % 100;
            sleep(backoff + Duration::from_millis(jitter)).await;
        }

        Err(anyhow::anyhow!(
            "Failed to fetch metrics after {} attempts",
            MAX_RETRIES
        ))
    }

    async fn fetch_metrics(&self) -> Result<String> {
        let metrics_url = format!("http://127.0.0.1:{}/metrics", self.prometheus_port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()?;

        let response = match client
            .get(&metrics_url)
            .header("Accept", "text/plain")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let context = format!(
                        "Failed to connect to metrics endpoint for {}/{} (bootnode: {}, ports: prometheus={}, p2p={}): {}",
                        self.operator, self.network, self.bootnode, self.prometheus_port, self.p2p_port, e
                    );
                return Err(anyhow::anyhow!(context));
            }
        };

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                    "Bad status {} from metrics endpoint for {}/{} (bootnode: {}, ports: prometheus={}, p2p={})",
                    response.status(), self.operator, self.network, self.bootnode,
                    self.prometheus_port, self.p2p_port
            ));
        }

        match response.text().await {
            Ok(text) => Ok(text),
            Err(e) => Err(anyhow::anyhow!(
                    "Failed to read metrics response for {}/{} (bootnode: {}, ports: prometheus={}, p2p={}): {}",
                    self.operator, self.network, self.bootnode, self.prometheus_port, self.p2p_port, e
            ))
        }
    }

    fn parse_peer_metrics(&self, metrics: &str) -> Result<HashMap<String, u64>> {
        let mut peer_data = HashMap::new();

        for line in metrics.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }

            match self.parse_metric_line(line) {
                Ok(Some((metric, count))) => {
                    peer_data.insert(metric, count);
                }
                Ok(None) => continue,
                Err(e) => {
                    warn!(
                        "{} Failed to parse metric line '{}': {}",
                        EMOJI_WARNING, line, e
                    );
                }
            }
        }

        if peer_data.is_empty() {
            warn!(
                "{} No valid peer metrics found in response for {}/{}",
                EMOJI_WARNING, self.operator, self.network
            );
        }

        Ok(peer_data)
    }

    fn parse_metric_line(&self, line: &str) -> Result<Option<(String, u64)>> {
        if line.trim().is_empty() || line.starts_with('#') {
            return Ok(None);
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(None);
        }

        let metric_name = parts[0].split('{').next().unwrap_or("").trim();

        let value_str = parts.last().unwrap_or(&"0");

        let value = match value_str.parse::<f64>() {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };

        match metric_name {
            "substrate_sub_libp2p_peerset_num_discovered" => {
                Ok(Some(("discovered".to_string(), value as u64)))
            }
            "substrate_sub_libp2p_peers_count" => Ok(Some(("connected".to_string(), value as u64))),
            _ => Ok(None),
        }
    }

    async fn bootnode_is_working(
        &mut self,
        timeout: Duration,
    ) -> Result<(u64, TestStatus, Option<String>)> {
        sleep(Duration::from_secs(5)).await;
        let end_time = Instant::now() + timeout;
        let mut consecutive_failures = 0;
        const MAX_CONSECUTIVE_FAILURES: u32 = 3;

        while Instant::now() < end_time {
            match self.check_discovered_peers().await {
                Ok(metrics) => {
                    consecutive_failures = 0;
                    match metrics.status {
                        MetricsStatus::Available if metrics.peers >= self.cli.min_peers => {
                            info!(
                                "{} Bootnode working for {}/{} - discovered {} peers",
                                EMOJI_SUCCESS, self.operator, self.network, metrics.peers
                            );
                            return Ok((metrics.peers, TestStatus::Success, None));
                        }
                        MetricsStatus::Available => {
                            sleep(Duration::from_secs(1)).await;
                        }
                        MetricsStatus::NoMetricFound => {
                            consecutive_failures += 1;
                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                warn!(
                                    "{} No metrics found after {} consecutive attempts for {}/{}",
                                    EMOJI_WARNING,
                                    MAX_CONSECUTIVE_FAILURES,
                                    self.operator,
                                    self.network
                                );
                                return Ok((0, TestStatus::NoMetricFound, None));
                            }
                            sleep(Duration::from_secs(1)).await;
                        }
                        MetricsStatus::Unavailable(error) => {
                            consecutive_failures += 1;
                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                error!(
                                    "{} Metrics consistently unavailable for {}/{}: {}",
                                    EMOJI_ERROR, self.operator, self.network, error
                                );
                                return Ok((0, TestStatus::MetricsUnavailable, Some(error)));
                            }
                            sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        error!(
                            "{} Consistent errors checking peers for {}/{}: {}",
                            EMOJI_ERROR, self.operator, self.network, e
                        );
                        return Ok((0, TestStatus::MetricsUnavailable, Some(e.to_string())));
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }

        warn!(
            "{} Timeout waiting for peer discovery for {}/{}",
            EMOJI_WARNING, self.operator, self.network
        );
        Ok((0, TestStatus::Timeout, None))
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

    info!(
        "{} Testing bootnode {} for {}/{}",
        EMOJI_LOADING, bootnode, operator, network
    );

    let mut node = match spawn_node(cli, operator, network, bootnode, command_id).await {
        Ok(node) => node,
        Err(e) => {
            error!(
                "{} Node startup failed for {}/{}: {}",
                EMOJI_ERROR, operator, network, e
            );
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

    let (discovered_peers, status, error_details) = node
        .bootnode_is_working(Duration::from_secs(cli.timeout))
        .await?;

    let test_duration_ms = start_time.elapsed().as_millis() as u64;

    node.cleanup().await?;

    Ok(TestResult {
        id: operator.to_string(),
        network: network.to_string(),
        bootnode: bootnode.to_string(),
        valid: discovered_peers >= cli.min_peers,
        test_duration_ms,
        discovered_peers,
        status,
        error_details,
    })
}
