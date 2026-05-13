// main.rs
mod bootnode;
mod bootnode_p2p;
mod bootnode_smoldot;
mod cli;
mod db;
mod metrics;
mod p2p;
mod smoldot_client;
mod sync;

use anyhow::{Context, Result};
use futures::future::join_all;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{sync::Semaphore, time::sleep};
use tracing::{error, info};

use crate::{
    bootnode::NEXT_PORT,
    bootnode_p2p::test_bootnode_p2p,
    bootnode_smoldot::test_bootnode_smoldot,
    cli::Cli,
    metrics::{MetricsHandle, TestResult},
};

fn get_chain_spec_path(cli: &Cli, network: &str) -> Option<String> {
    let spec_path = cli.chain_spec_dir.join(format!("{}.json", network));
    if spec_path.exists() {
        Some(spec_path.to_string_lossy().to_string())
    } else {
        None
    }
}

async fn load_bootnodes_from_db(database: &db::Database) -> Result<cli::BootnodesConfig> {
    let bootnodes = database.get_all_active_bootnodes().await?;

    let mut networks: std::collections::HashMap<String, cli::NetworkConfig> =
        std::collections::HashMap::new();

    for bootnode in bootnodes {
        let network_config = networks
            .entry(bootnode.network.clone())
            .or_insert_with(|| cli::NetworkConfig {
                members: std::collections::HashMap::new(),
            });

        let operator = bootnode.operator.unwrap_or_else(|| "unknown".to_string());
        let member_bootnodes = network_config
            .members
            .entry(operator)
            .or_insert_with(Vec::new);

        member_bootnodes.push(bootnode.multiaddr);
    }

    Ok(cli::BootnodesConfig { networks })
}

async fn run_test_cycle(
    cli: &Cli,
    bootnodes: &cli::BootnodesConfig,
    metrics_state: Arc<metrics::MetricsState>,
    semaphore: Arc<Semaphore>,
    database: db::Database,
) -> Result<TestCycleSummary> {
    let mut tasks = Vec::new();
    let mut total_tests = 0;

    for (network, network_config) in &bootnodes.networks {
        for (operator, bootnodes) in &network_config.members {
            for bootnode in bootnodes {
                total_tests += 1;
                let cli = cli.clone();
                let network = network.clone();
                let operator = operator.clone();
                let bootnode = bootnode.clone();
                let semaphore = Arc::clone(&semaphore);
                let metrics = Arc::clone(&metrics_state);
                let db = database.clone();

                tasks.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await?;

                    let result = if cli.use_smoldot {
                        if let Some(chain_spec_path) = get_chain_spec_path(&cli, &network) {
                            test_bootnode_smoldot(&cli, &operator, &network, &bootnode, &chain_spec_path).await?
                        } else {
                            error!("chain spec not found for network: {}", network);
                            return Err(anyhow::anyhow!("chain spec not found for network: {}", network));
                        }
                    } else {
                        test_bootnode_p2p(&cli, &operator, &network, &bootnode).await?
                    };

                    metrics.record_test_result(&network, &operator, &bootnode, &result);

                    // record to database
                    if let Ok(bootnodes_in_db) = db.get_active_bootnodes(&network).await {
                        if let Some(bootnode_record) = bootnodes_in_db.iter().find(|b| b.multiaddr == bootnode) {
                            let status_str = match result.status {
                                metrics::TestStatus::Success => "success",
                                metrics::TestStatus::MetricsUnavailable => "metrics_unavailable",
                                metrics::TestStatus::NoMetricFound => "no_metric_found",
                                metrics::TestStatus::Timeout => "timeout",
                                metrics::TestStatus::NodeStartupFailed => "node_startup_failed",
                                metrics::TestStatus::InsufficientPeers => "insufficient_peers",
                            }.to_string();

                            let test_result = db::TestResult {
                                bootnode_id: bootnode_record.id,
                                test_time: chrono::Utc::now(),
                                success: result.valid,
                                discovered_peers: Some(result.discovered_peers as i64),
                                connected_peers: None,  // not tracked in metrics::TestResult
                                test_duration_ms: Some(result.test_duration_ms as i64),
                                status: Some(status_str),
                                error_details: result.error_details.clone(),
                            };
                            let _ = db.record_test_result(&test_result).await;
                        }
                    }

                    Ok::<_, anyhow::Error>(result)
                }));
            }
        }
    }

    let mut success_count = 0;
    let mut failed_tests = Vec::new();

    for result in join_all(tasks).await {
        match result? {
            Ok(test_result) => {
                if test_result.valid {
                    success_count += 1;
                } else {
                    failed_tests.push((
                        test_result.network.clone(),
                        test_result.id.clone(),
                        test_result.bootnode.clone(),
                    ));
                }
                update_results(
                    &cli.output_dir.join("results.json"),
                    &test_result.id,
                    &test_result.network,
                    &test_result,
                )
                .await?;
                update_status(
                    &cli.output_dir.join("bootnodes-status.json"),
                    &test_result.id,
                    &test_result.network,
                    &test_result,
                )
                .await?;
            }
            Err(e) => {
                error!("Test failed: {}", e);
            }
        }
    }

    Ok(TestCycleSummary {
        total_tests,
        success_count,
        failed_tests,
    })
}

#[derive(Debug)]
struct TestCycleSummary {
    total_tests: usize,
    success_count: usize,
    failed_tests: Vec<(String, String, String)>, // (network, operator, bootnode)
}

async fn update_results(
    output_file: &Path,
    operator: &str,
    network: &str,
    result: &TestResult,
) -> Result<()> {
    let content = if output_file.exists() {
        fs::read_to_string(output_file)?
    } else {
        "{}".to_string()
    };

    let mut json: serde_json::Value = serde_json::from_str(&content)?;

    if let serde_json::Value::Object(ref mut map) = json {
        let operator_obj = map
            .entry(operator)
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .context("Invalid JSON structure")?;

        operator_obj.insert(network.to_string(), serde_json::to_value(result)?);
    }

    let tmp_file = output_file.with_extension("tmp");
    let mut file = File::create(&tmp_file)?;
    file.write_all(serde_json::to_string_pretty(&json)?.as_bytes())?;
    fs::rename(tmp_file, output_file)?;

    Ok(())
}

/// per-multiaddr status, keyed by network -> operator -> multiaddr.
/// mirrors the shape of bootnodes.json so it can be diffed/joined directly.
/// not deduplicated by (op, net) like results.json, so every transport
/// (tcp/wss) gets its own verdict.
async fn update_status(
    output_file: &Path,
    operator: &str,
    network: &str,
    result: &TestResult,
) -> Result<()> {
    let content = if output_file.exists() {
        fs::read_to_string(output_file)?
    } else {
        "{}".to_string()
    };

    let mut json: serde_json::Value = serde_json::from_str(&content)?;

    if let serde_json::Value::Object(ref mut map) = json {
        let net_obj = map
            .entry(network)
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .context("Invalid JSON structure (network)")?;
        let op_obj = net_obj
            .entry(operator)
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .context("Invalid JSON structure (operator)")?;

        let status_str = match result.status {
            metrics::TestStatus::Success => "success",
            metrics::TestStatus::MetricsUnavailable => "metrics_unavailable",
            metrics::TestStatus::NoMetricFound => "no_metric_found",
            metrics::TestStatus::Timeout => "timeout",
            metrics::TestStatus::NodeStartupFailed => "node_startup_failed",
            metrics::TestStatus::InsufficientPeers => "insufficient_peers",
        };

        op_obj.insert(
            result.bootnode.clone(),
            serde_json::json!({
                "working": result.valid,
                "status": status_str,
                "duration_ms": result.test_duration_ms,
                "error": result.error_details,
            }),
        );
    }

    let tmp_file = output_file.with_extension("status.tmp");
    let mut file = File::create(&tmp_file)?;
    file.write_all(serde_json::to_string_pretty(&json)?.as_bytes())?;
    fs::rename(tmp_file, output_file)?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // install rustls crypto provider for websocket TLS support
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::load()?;

    // keep our own crate at info/debug but mute the libp2p/smoldot firehose.
    // a single test cycle at default `info` was producing 40+ GB of logs from
    // litep2p trace events; RUST_LOG still overrides if you want it back.
    let default_filter = if cli.debug {
        "debug,litep2p=info,smoldot=info,warp=info"
    } else {
        "info,litep2p=warn,smoldot=warn,warp=warn"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // initialize database
    let db_path = cli.database.to_str().unwrap_or("bootyspector.db");
    let database = db::Database::new(&format!("sqlite://{}", db_path))
        .await
        .context("failed to initialize database")?;

    // sync chain specs and bootnodes if requested
    if cli.sync {
        info!("syncing chain specs and bootnodes from official sources");
        let syncer = sync::ChainSpecSync::new(database.clone());

        syncer.sync_all().await?;

        // also sync manual bootnodes if config file provided
        if cli.bootnodes_config.exists() {
            syncer
                .sync_manual_bootnodes(cli.bootnodes_config.to_str().unwrap())
                .await?;
        }

        info!("sync completed");
    }

    let metrics_handle = MetricsHandle::new()?;
    let metrics_state = metrics_handle.state.clone();

    // metrics server
    tokio::spawn(metrics_handle.serve(cli.prometheus_port));

    NEXT_PORT.store(cli.base_port, Ordering::SeqCst);
    fs::create_dir_all(&cli.output_dir)?;

    // load bootnodes from database OR from config file (backward compatibility)
    let bootnodes = if cli.bootnodes_config.exists() {
        serde_json::from_reader(
            File::open(&cli.bootnodes_config).context("Failed to open bootnodes config")?,
        )?
    } else {
        // load from database
        load_bootnodes_from_db(&database).await?
    };

    let semaphore = Arc::new(Semaphore::new(cli.max_concurrent));

    // continuous cycles
    info!("Starting continuous bootnode testing...");
    loop {
        let cycle_start = std::time::Instant::now();

        match run_test_cycle(&cli, &bootnodes, metrics_state.clone(), semaphore.clone(), database.clone()).await {
            Ok(summary) => {
                info!(
                    "Test cycle completed: {}/{} successful, {} failed. Cycle duration: {:?}",
                    summary.success_count,
                    summary.total_tests,
                    summary.failed_tests.len(),
                    cycle_start.elapsed(),
                );

                if !summary.failed_tests.is_empty() {
                    info!("Failed bootnodes:");
                    for (network, operator, bootnode) in summary.failed_tests {
                        info!("- {}/{}: {}", operator, network, bootnode);
                    }
                }
            }
            Err(e) => {
                error!("Test cycle failed: {}", e);
            }
        }

        // Wait before starting the next cycle
        // Calculate delay to maintain consistent cycle time
        let cycle_duration = cycle_start.elapsed();
        let target_cycle_time = Duration::from_secs(cli.interval);
        if cycle_duration < target_cycle_time {
            let delay = target_cycle_time - cycle_duration;
            info!("Waiting {:?} before next cycle", delay);
            sleep(delay).await;
        } else {
            info!("Cycle took longer than target time, starting next cycle immediately");
        }
    }
}
