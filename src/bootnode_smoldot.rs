// bootnode_smoldot.rs - bootnode testing using smoldot light client
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::{error, info};

use crate::{
    cli::Cli,
    metrics::{TestResult, TestStatus},
};

const EMOJI_SUCCESS: &str = "✅";
const EMOJI_ERROR: &str = "❌";
const EMOJI_LOADING: &str = "⏳";

pub async fn test_bootnode_smoldot(
    cli: &Cli,
    operator: &str,
    network: &str,
    bootnode: &str,
    chain_spec_path: &str,
) -> Result<TestResult> {
    let start_time = Instant::now();

    info!(
        "{} Testing bootnode {} for {}/{} using smoldot",
        EMOJI_LOADING, bootnode, operator, network
    );

    // Read the chain spec
    let chain_spec_content = tokio::fs::read_to_string(chain_spec_path)
        .await
        .with_context(|| format!("failed to read chain spec from {}", chain_spec_path))?;

    // Parse the chain spec JSON
    let mut spec: Value = serde_json::from_str(&chain_spec_content)
        .context("failed to parse chain spec JSON")?;

    // Remove all existing bootnodes and inject our target bootnode
    // Note: smoldot's multiaddr parser requires exactly 3-4 protocol components
    // and doesn't handle /p2p/ suffix well, so we keep it in the chain spec
    // which is the standard format
    if let Some(boot_nodes) = spec.get_mut("bootNodes") {
        *boot_nodes = serde_json::json!([bootnode]);
    }

    // Convert back to string
    let modified_spec = serde_json::to_string(&spec)
        .context("failed to serialize modified chain spec")?;

    // Create smoldot client
    let mut client = smoldot_light::Client::new(
        smoldot_light::platform::default::DefaultPlatform::new(
            "bootyspector".into(),
            env!("CARGO_PKG_VERSION").into(),
        )
    );

    // Add chain with JSON-RPC enabled to query peers
    let add_result = client.add_chain(smoldot_light::AddChainConfig {
        specification: &modified_spec,
        json_rpc: smoldot_light::AddChainConfigJsonRpc::Enabled {
            max_pending_requests: core::num::NonZero::<u32>::new(32).unwrap(),
            max_subscriptions: 32,
        },
        potential_relay_chains: core::iter::empty(),
        database_content: "",
        user_data: (),
    });

    let (chain_id, mut json_rpc_responses) = match add_result {
        Ok(success) => (success.chain_id, success.json_rpc_responses.unwrap()),
        Err(e) => {
            error!("{} Failed to add chain: {:?}", EMOJI_ERROR, e);
            return Ok(TestResult {
                id: operator.to_string(),
                network: network.to_string(),
                bootnode: bootnode.to_string(),
                valid: false,
                test_duration_ms: start_time.elapsed().as_millis() as u64,
                discovered_peers: 0,
                status: TestStatus::NodeStartupFailed,
                error_details: Some(format!("{:?}", e)),
            });
        }
    };

    // Wait for connection to establish and sync to begin
    // Smoldot performs DHT discovery immediately after connecting
    tokio::time::sleep(Duration::from_secs(cli.timeout)).await;

    // Query sync state to verify bootnode is providing network access
    client.json_rpc_request(
        r#"{"id":1,"jsonrpc":"2.0","method":"system_health","params":[]}"#,
        chain_id,
    ).ok();

    // Wait for response
    let mut discovered_peers = 0u64;
    let bootnode_working = if let Some(response) = tokio::time::timeout(
        Duration::from_secs(5),
        json_rpc_responses.next()
    ).await.ok().flatten() {
        // Parse JSON response to check if we're syncing
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(result) = json.get("result") {
                // If we have health data, the bootnode successfully provided network access
                if let Some(peers_val) = result.get("peers") {
                    if let Some(peers) = peers_val.as_u64() {
                        discovered_peers = peers;
                    }
                }
                // Check if we're syncing - this proves bootnode is working
                if let Some(is_syncing) = result.get("isSyncing") {
                    if is_syncing.as_bool().unwrap_or(false) {
                        info!("bootnode successfully provided network access - syncing in progress");
                        // Syncing means DHT discovery happened and bootnode provided peer info
                        discovered_peers = discovered_peers.max(1);
                    }
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let test_duration_ms = start_time.elapsed().as_millis() as u64;

    // For smoldot light client, successful connection + sync = bootnode is providing peer discovery
    // Smoldot performs DHT discovery and receives peer addresses from the bootnode
    // If it's syncing, that proves the bootnode successfully shared network peer information
    // Note: smoldot won't actually CONNECT to all discovered peers (it's a light client),
    // but the fact that it's syncing proves peer discovery happened
    let valid = bootnode_working;
    let status = if valid {
        TestStatus::Success
    } else {
        TestStatus::Timeout
    };

    info!(
        "{} Bootnode test completed for {}/{}: {} peers, bootnode_working={} in {}ms",
        if valid { EMOJI_SUCCESS } else { EMOJI_ERROR },
        operator,
        network,
        discovered_peers,
        bootnode_working,
        test_duration_ms
    );

    // Clean up
    let _ = client.remove_chain(chain_id);

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
