// src/metrics.rs

// f.g. prometheus alerting rules:
/*
groups:
- name: bootnode_alerts
 rules:
 - alert: BootnodeDown
   expr: bootnode_status == 0
   for: 5m
   labels:
     severity: critical
   annotations:
     summary: "Bootnode {{ $labels.provider }}/{{ $labels.network }} is down"
     description: "Bootnode has failed with reason: {{ $labels.failure_reason }}"

 - alert: SlowBootnodeChecks
   expr: bootnode_check_duration_ms > 30000
   for: 5m
   labels:
     severity: warning
   annotations:
     summary: "Slow bootnode checks"
     description: "Check duration > 30s for {{ $labels.provider }}/{{ $labels.network }}"
*/
use anyhow::Result;
use prometheus::{Encoder, IntGaugeVec, Registry, TextEncoder};
use serde::Serialize;
use std::sync::Arc;
use tracing::error;
use warp::Filter;

#[derive(Debug)]
pub struct MetricsResult {
    pub peers: u64,
    pub status: MetricsStatus,
}

#[derive(Debug)]
pub enum MetricsStatus {
    Available,
    NoMetricFound,
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
    bootnode_status: IntGaugeVec,
    last_check_duration: IntGaugeVec,
}

impl MetricsState {
    pub fn new() -> Result<(Self, Registry)> {
        let registry = Registry::new();

        let bootnode_status = IntGaugeVec::new(
            prometheus::opts!(
                "bootnode_status",
                "Current bootnode status with reason (1=working, 0=failed)"
            ),
            &["network", "provider", "bootnode", "failure_reason"],
        )?;

        let last_check_duration = IntGaugeVec::new(
            prometheus::opts!(
                "bootnode_check_duration_ms",
                "Duration of last check in milliseconds"
            ),
            &["network", "provider", "bootnode"],
        )?;

        registry.register(Box::new(bootnode_status.clone()))?;
        registry.register(Box::new(last_check_duration.clone()))?;

        Ok((
            Self {
                bootnode_status,
                last_check_duration,
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
        let reason = if result.valid {
            "none"
        } else {
            match result.status {
                TestStatus::NodeStartupFailed => "startup_failed",
                TestStatus::MetricsUnavailable => "metrics_unavailable",
                TestStatus::NoMetricFound => "no_metrics",
                TestStatus::Timeout => "timeout",
                TestStatus::Success => unreachable!(),
            }
        };

        self.bootnode_status
            .with_label_values(&[network, provider, bootnode, reason])
            .set(if result.valid { 1 } else { 0 });

        self.last_check_duration
            .with_label_values(&[network, provider, bootnode])
            .set(result.test_duration_ms as i64);
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
