// bootnode_p2p.rs - bootnode testing using litep2p (no binary required)
use anyhow::{Context, Result};
use litep2p::types::multiaddr::Multiaddr;
use std::time::{Duration, Instant};
use tracing::{error, info};

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

    // create p2p client and connect to bootnode
    let mut p2p_client = match P2PClient::new(&protocol_id, bootnode_addr).await {
        Ok(client) => client,
        Err(e) => {
            error!(
                "{} Failed to create p2p client for {}/{}: {}",
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

    // run peer discovery; exit as soon as min_peers are connected past the
    // stabilization window so healthy bootnodes don't sit the full timeout.
    // 3s stabilization gives Kademlia time to round-trip after connection.
    let discovery_result = match p2p_client
        .discover_peers(
            Duration::from_secs(cli.timeout),
            cli.min_peers as usize,
            Duration::from_secs(3),
        )
        .await
    {
        Ok(result) => result,
        Err(e) => {
            error!(
                "{} Peer discovery failed for {}/{}: {}",
                EMOJI_ERROR, operator, network, e
            );
            return Ok(TestResult {
                id: operator.to_string(),
                network: network.to_string(),
                bootnode: bootnode.to_string(),
                valid: false,
                test_duration_ms: start_time.elapsed().as_millis() as u64,
                discovered_peers: 0,
                status: TestStatus::MetricsUnavailable,
                error_details: Some(e.to_string()),
            });
        }
    };

    let test_duration_ms = start_time.elapsed().as_millis() as u64;
    let discovered_peers = discovery_result.discovered_peers as u64;
    let connected_peers = discovery_result.connected_peers as u64;

    // A bootnode is functional if we can connect to it and it responds to identify
    // The fact that DHT queries may fail is due to bootnode policies (restricting peer sharing
    // to full nodes), not a bootnode failure. If we connected and identified, it's working.
    let valid = connected_peers >= 1;
    let status = if valid {
        TestStatus::Success
    } else if discovered_peers == 0 {
        TestStatus::Timeout
    } else {
        TestStatus::InsufficientPeers // discovered but couldn't connect
    };

    info!(
        "{} Bootnode test completed for {}/{}: {} discovered, {} connected in {}ms",
        if valid { EMOJI_SUCCESS } else { EMOJI_ERROR },
        operator,
        network,
        discovered_peers,
        connected_peers,
        test_duration_ms
    );

    Ok(TestResult {
        id: operator.to_string(),
        network: network.to_string(),
        bootnode: bootnode.to_string(),
        valid,
        test_duration_ms,
        discovered_peers,
        status,
        error_details: None,
    })
}
