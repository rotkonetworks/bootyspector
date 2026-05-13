// bootnode_p2p.rs - bootnode testing using litep2p (no binary required)
use anyhow::{Context, Result};
use litep2p::types::multiaddr::Multiaddr;
use rand::Rng;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::{
    cli::Cli,
    metrics::{TestResult, TestStatus},
    p2p::P2PClient,
};

const EMOJI_SUCCESS: &str = "✅";
const EMOJI_ERROR: &str = "❌";
const EMOJI_LOADING: &str = "⏳";

pub async fn test_bootnode_p2p(
    cli: &Cli,
    operator: &str,
    network: &str,
    bootnode: &str,
) -> Result<TestResult> {
    let start_time = Instant::now();

    info!(
        "{} Testing bootnode {} for {}/{} using p2p",
        EMOJI_LOADING, bootnode, operator, network
    );

    // parse bootnode multiaddr
    let bootnode_addr: Multiaddr = bootnode
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bootnode multiaddr {}: {}", bootnode, e))?;

    // get chain spec path and read protocol ID
    let chain_spec_path = cli.chain_spec_dir.join(format!("{}.json", network));
    let protocol_id = if chain_spec_path.exists() {
        let spec_content = tokio::fs::read_to_string(&chain_spec_path).await
            .with_context(|| format!("failed to read chain spec from {:?}", chain_spec_path))?;
        let spec: serde_json::Value = serde_json::from_str(&spec_content)
            .context("failed to parse chain spec JSON")?;

        spec.get("protocolId")
            .and_then(|v| v.as_str())
            .unwrap_or(network)
            .to_string()
    } else {
        // fallback to network name if no chain spec
        network.to_string()
    };

    info!("using protocol ID: {} for network: {}", protocol_id, network);

    let first = attempt(cli, operator, network, bootnode, &bootnode_addr, &protocol_id).await;
    if first.valid {
        return Ok(finalize(operator, network, bootnode, first, start_time));
    }

    // retry once on failure to separate transient flakes from real outages.
    // jitter avoids 8 retries hitting the same operator in the same instant
    // when concurrency is high.
    let jitter_ms = rand::thread_rng().gen_range(500..2500);
    warn!(
        "↻ retrying {}/{} after {}ms (first attempt: {:?})",
        operator, network, jitter_ms, first.status
    );
    tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

    let second = attempt(cli, operator, network, bootnode, &bootnode_addr, &protocol_id).await;
    Ok(finalize(operator, network, bootnode, second, start_time))
}

struct AttemptResult {
    valid: bool,
    discovered_peers: u64,
    status: TestStatus,
    error_details: Option<String>,
}

async fn attempt(
    cli: &Cli,
    operator: &str,
    network: &str,
    bootnode: &str,
    bootnode_addr: &Multiaddr,
    protocol_id: &str,
) -> AttemptResult {
    let mut p2p_client = match P2PClient::new(protocol_id, bootnode_addr.clone()).await {
        Ok(client) => client,
        Err(e) => {
            error!(
                "{} Failed to create p2p client for {}/{}: {}",
                EMOJI_ERROR, operator, network, e
            );
            return AttemptResult {
                valid: false,
                discovered_peers: 0,
                status: TestStatus::NodeStartupFailed,
                error_details: Some(e.to_string()),
            };
        }
    };

    // run peer discovery; exit as soon as min_peers are connected past the
    // stabilization window so healthy bootnodes don't sit the full timeout.
    // 5s stabilization gives Noise + protocol negotiation time to complete
    // on slower operators (3s was too tight; uniformly failed amforc).
    let discovery_result = match p2p_client
        .discover_peers(
            Duration::from_secs(cli.timeout),
            cli.min_peers as usize,
            Duration::from_secs(5),
        )
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!(
                "{} Peer discovery failed for {}/{}: {}",
                EMOJI_ERROR, operator, network, e
            );
            return AttemptResult {
                valid: false,
                discovered_peers: 0,
                status: TestStatus::MetricsUnavailable,
                error_details: Some(e.to_string()),
            };
        }
    };

    let discovered_peers = discovery_result.discovered_peers as u64;
    let connected_peers = discovery_result.connected_peers as u64;
    let valid = connected_peers >= 1;
    let status = if valid {
        TestStatus::Success
    } else if discovered_peers == 0 {
        TestStatus::Timeout
    } else {
        TestStatus::InsufficientPeers
    };

    let _ = bootnode; // suppress unused in attempt; consumed by finalize via outer scope
    AttemptResult {
        valid,
        discovered_peers,
        status,
        // surface the first dial-failure against the bootnode itself if any
        // (e.g. PeerIdMismatch — bootnode peer id rotated but the multiaddr
        // entry in bootnodes.json wasn't updated). Without this, failures
        // come back as a generic "insufficient_peers" with no error string.
        error_details: if valid { None } else { discovery_result.bootnode_dial_error },
    }
}

fn finalize(
    operator: &str,
    network: &str,
    bootnode: &str,
    a: AttemptResult,
    start_time: Instant,
) -> TestResult {
    let test_duration_ms = start_time.elapsed().as_millis() as u64;
    info!(
        "{} Bootnode test completed for {}/{}: {} discovered, valid={} in {}ms",
        if a.valid { EMOJI_SUCCESS } else { EMOJI_ERROR },
        operator,
        network,
        a.discovered_peers,
        a.valid,
        test_duration_ms
    );
    TestResult {
        id: operator.to_string(),
        network: network.to_string(),
        bootnode: bootnode.to_string(),
        valid: a.valid,
        test_duration_ms,
        discovered_peers: a.discovered_peers,
        status: a.status,
        error_details: a.error_details,
    }
}
