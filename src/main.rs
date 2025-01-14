// main.rs
mod bootnode;
mod cli;
mod metrics;

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
    bootnode::{test_bootnode, NEXT_PORT},
    cli::Cli,
    metrics::{MetricsHandle, TestResult},
};

async fn run_test_cycle(
    cli: &Cli,
    bootnodes: &cli::BootnodesConfig,
    metrics_state: Arc<metrics::MetricsState>,
    semaphore: Arc<Semaphore>,
) -> Result<TestCycleSummary> {
    let mut tasks = Vec::new();
    let mut total_tests = 0;

    for (network, network_config) in &bootnodes.networks {
        let command_id = network_config.command_id.clone();
        for (operator, bootnodes) in &network_config.members {
            for bootnode in bootnodes {
                total_tests += 1;
                let cli = cli.clone();
                let network = network.clone();
                let operator = operator.clone();
                let bootnode = bootnode.clone();
                let command_id = command_id.clone();
                let semaphore = Arc::clone(&semaphore);
                let metrics = Arc::clone(&metrics_state);

                tasks.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await?;
                    let result =
                        test_bootnode(&cli, &operator, &network, &bootnode, &command_id).await?;

                    metrics.record_test_result(&network, &operator, &bootnode, &result);
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::load()?;

    let log_level = if cli.debug {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    tracing_subscriber::fmt().with_max_level(log_level).init();

    let metrics_handle = MetricsHandle::new()?;
    let metrics_state = metrics_handle.state.clone();

    // metrics server
    tokio::spawn(metrics_handle.serve(cli.prometheus_port));

    NEXT_PORT.store(cli.base_port, Ordering::SeqCst);
    fs::create_dir_all(&cli.output_dir)?;

    let bootnodes: cli::BootnodesConfig = serde_json::from_reader(
        File::open(&cli.bootnodes_config).context("Failed to open bootnodes config")?,
    )?;

    let semaphore = Arc::new(Semaphore::new(cli.max_concurrent));

    // continuous cycles
    info!("Starting continuous bootnode testing...");
    loop {
        let cycle_start = std::time::Instant::now();

        match run_test_cycle(&cli, &bootnodes, metrics_state.clone(), semaphore.clone()).await {
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
