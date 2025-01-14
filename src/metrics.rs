// src/metrics.rs
use anyhow::Result;
use prometheus::{Encoder, IntGaugeVec, Registry, TextEncoder};
use serde::Serialize;
use std::{collections::HashMap, sync::Arc};
use tracing::error;
use warp::Filter;

#[derive(Debug)]
pub struct MetricsResult {
    pub peers: u64,
    pub peer_types: Option<HashMap<String, u64>>,
    pub status: MetricsStatus,
}

#[derive(Debug)]
pub enum MetricsStatus {
    Available,
    NoMetricFound,
    Unavailable(String),
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum TestStatus {
    Success,
    MetricsUnavailable,
    NoMetricFound,
    Timeout,
    NodeStartupFailed,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub id: String,
    pub network: String,
    pub bootnode: String,
    pub valid: bool,
    pub test_duration_ms: u64,
    pub discovered_peers: u64,
    pub status: TestStatus,
    pub error_details: Option<String>,
}

#[derive(Clone)]
pub struct MetricsState {
    discovered_peers: IntGaugeVec,
    test_duration: IntGaugeVec,
    test_success: IntGaugeVec,
    connection_type_success: IntGaugeVec,
    failure_reasons: IntGaugeVec,
    dns_resolution_time: IntGaugeVec,
    chain_sync_progress: IntGaugeVec,
    peer_count_by_type: IntGaugeVec,
    peer_connections: IntGaugeVec,
    network_state: IntGaugeVec,
}

impl MetricsState {
    pub fn new() -> Result<(Self, Registry)> {
        let registry = Registry::new();

        let discovered_peers = IntGaugeVec::new(
            prometheus::opts!("bootnode_discovered_peers", "Number of peers discovered"),
            &["network", "provider", "bootnode_type", "protocol"],
        )?;

        let test_duration = IntGaugeVec::new(
            prometheus::opts!("bootnode_test_duration_ms", "Test duration in milliseconds"),
            &["network", "provider", "bootnode_type"],
        )?;

        let test_success = IntGaugeVec::new(
            prometheus::opts!("bootnode_test_success", "Test success status"),
            &["network", "provider", "bootnode_type"],
        )?;

        let connection_type_success = IntGaugeVec::new(
            prometheus::opts!("bootnode_connection_success", "Connection success by type"),
            &["network", "provider", "protocol"],
        )?;

        let failure_reasons = IntGaugeVec::new(
            prometheus::opts!("bootnode_failure_reasons", "Detailed failure reasons"),
            &["network", "provider", "reason"],
        )?;

        let dns_resolution_time = IntGaugeVec::new(
            prometheus::opts!("bootnode_dns_resolution_ms", "DNS resolution time"),
            &["network", "provider", "hostname"],
        )?;

        let chain_sync_progress = IntGaugeVec::new(
            prometheus::opts!("bootnode_chain_sync_progress", "Chain sync progress"),
            &["network", "provider"],
        )?;

        let peer_count_by_type = IntGaugeVec::new(
            prometheus::opts!("bootnode_peer_count_by_type", "Peer count by type"),
            &["network", "provider", "peer_type"],
        )?;

        let peer_connections = IntGaugeVec::new(
            prometheus::opts!(
                "bootnode_peer_connections",
                "Current number of peer connections"
            ),
            &["network", "provider", "direction"],
        )?;

        let network_state = IntGaugeVec::new(
            prometheus::opts!("bootnode_network_state", "Network state indicators"),
            &["network", "provider", "state_type"],
        )?;

        // Register all metrics
        registry.register(Box::new(discovered_peers.clone()))?;
        registry.register(Box::new(test_duration.clone()))?;
        registry.register(Box::new(test_success.clone()))?;
        registry.register(Box::new(connection_type_success.clone()))?;
        registry.register(Box::new(failure_reasons.clone()))?;
        registry.register(Box::new(dns_resolution_time.clone()))?;
        registry.register(Box::new(chain_sync_progress.clone()))?;
        registry.register(Box::new(peer_count_by_type.clone()))?;
        registry.register(Box::new(peer_connections.clone()))?;
        registry.register(Box::new(network_state.clone()))?;

        Ok((
            Self {
                discovered_peers,
                test_duration,
                test_success,
                connection_type_success,
                failure_reasons,
                dns_resolution_time,
                chain_sync_progress,
                peer_count_by_type,
                peer_connections,
                network_state,
            },
            registry,
        ))
    }

    pub fn record_test_result(
        &self,
        network: &str,
        provider: &str,
        bootnode: &str,
        result: &TestResult,
    ) {
        let bootnode_type = if network.contains('-') {
            "parachain"
        } else {
            "relay"
        };
        let protocol = if bootnode.contains("/wss/") {
            "websocket"
        } else {
            "tcp"
        };

        // Record discovered peers
        self.discovered_peers
            .with_label_values(&[network, provider, bootnode_type, protocol])
            .set(result.discovered_peers as i64);

        // Record test duration
        self.test_duration
            .with_label_values(&[network, provider, bootnode_type])
            .set(result.test_duration_ms as i64);

        // Record test success
        self.test_success
            .with_label_values(&[network, provider, bootnode_type])
            .set(if result.valid { 1 } else { 0 });

        // Record connection success by protocol
        self.connection_type_success
            .with_label_values(&[network, provider, protocol])
            .set(if result.valid { 1 } else { 0 });

        // Record failure reasons if any
        if let Some(error) = &result.error_details {
            self.failure_reasons
                .with_label_values(&[network, provider, error])
                .inc();
        }

        // Record network state
        self.network_state
            .with_label_values(&[network, provider, "active"])
            .set(if result.valid { 1 } else { 0 });
    }

    pub fn _record_peer_counts(
        &self,
        network: &str,
        provider: &str,
        peer_types: &HashMap<String, u64>,
    ) {
        for (peer_type, count) in peer_types {
            self.peer_count_by_type
                .with_label_values(&[network, provider, peer_type])
                .set(*count as i64);
        }
    }

    pub fn _record_dns_resolution(
        &self,
        network: &str,
        provider: &str,
        hostname: &str,
        duration_ms: u64,
    ) {
        self.dns_resolution_time
            .with_label_values(&[network, provider, hostname])
            .set(duration_ms as i64);
    }

    pub fn _record_chain_sync(&self, network: &str, provider: &str, progress: u64) {
        self.chain_sync_progress
            .with_label_values(&[network, provider])
            .set(progress as i64);
    }

    pub fn _record_peer_connections(
        &self,
        network: &str,
        provider: &str,
        inbound: u64,
        outbound: u64,
    ) {
        self.peer_connections
            .with_label_values(&[network, provider, "inbound"])
            .set(inbound as i64);
        self.peer_connections
            .with_label_values(&[network, provider, "outbound"])
            .set(outbound as i64);
    }
}

pub struct MetricsHandle {
    pub state: Arc<MetricsState>,
    pub registry: Registry,
}

impl MetricsHandle {
    pub fn new() -> Result<Self> {
        let (state, registry) = MetricsState::new()?;
        Ok(Self {
            state: Arc::new(state),
            registry,
        })
    }
    pub async fn serve(self, port: u16) -> Result<()> {
        let metrics_route = warp::path!("metrics").map(move || {
            let encoder = TextEncoder::new();
            let metric_families = self.registry.gather();
            let mut buffer = Vec::new();
            encoder
                .encode(&metric_families, &mut buffer)
                .unwrap_or_else(|e| {
                    error!("Failed to encode metrics: {}", e);
                });
            String::from_utf8(buffer).unwrap_or_else(|e| {
                error!("Failed to convert metrics to string: {}", e);
                String::from("# Error encoding metrics")
            })
        });

        warp::serve(metrics_route).run(([127, 0, 0, 1], port)).await;
        Ok(())
    }
}
