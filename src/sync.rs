// sync.rs - sync chain specs and bootnodes from Parity's chainspecs repo
use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::db::Database;

/// mapping of network names to their chain spec URLs in the official repo
pub const CHAINSPEC_MAPPINGS: &[(&str, &str, Option<&str>)] = &[
    // Polkadot relay + system parachains
    ("polkadot", "https://paritytech.github.io/chainspecs/polkadot/relaychain/chainspec.json", None),
    ("asset-hub-polkadot", "https://paritytech.github.io/chainspecs/polkadot/parachain/asset-hub/chainspec.json", Some("polkadot")),
    ("bridge-hub-polkadot", "https://paritytech.github.io/chainspecs/polkadot/parachain/bridge-hub/chainspec.json", Some("polkadot")),
    ("collectives-polkadot", "https://paritytech.github.io/chainspecs/polkadot/parachain/collectives/chainspec.json", Some("polkadot")),

    // Kusama relay + system parachains
    ("kusama", "https://paritytech.github.io/chainspecs/kusama/relaychain/chainspec.json", None),
    ("asset-hub-kusama", "https://paritytech.github.io/chainspecs/kusama/parachain/asset-hub/chainspec.json", Some("kusama")),
    ("bridge-hub-kusama", "https://paritytech.github.io/chainspecs/kusama/parachain/bridge-hub/chainspec.json", Some("kusama")),
    ("coretime-kusama", "https://paritytech.github.io/chainspecs/kusama/parachain/coretime/chainspec.json", Some("kusama")),
    ("people-kusama", "https://paritytech.github.io/chainspecs/kusama/parachain/people/chainspec.json", Some("kusama")),
    ("encointer-kusama", "https://paritytech.github.io/chainspecs/kusama/parachain/encointer/chainspec.json", Some("kusama")),

    // Westend relay + system parachains
    ("westend", "https://paritytech.github.io/chainspecs/westend/relaychain/chainspec.json", None),
    ("asset-hub-westend", "https://paritytech.github.io/chainspecs/westend/parachain/asset-hub/chainspec.json", Some("westend")),
    ("bridge-hub-westend", "https://paritytech.github.io/chainspecs/westend/parachain/bridge-hub/chainspec.json", Some("westend")),
    ("collectives-westend", "https://paritytech.github.io/chainspecs/westend/parachain/collectives/chainspec.json", Some("westend")),
    ("coretime-westend", "https://paritytech.github.io/chainspecs/westend/parachain/coretime/chainspec.json", Some("westend")),
    ("people-westend", "https://paritytech.github.io/chainspecs/westend/parachain/people/chainspec.json", Some("westend")),

    // Paseo relay + system parachains
    ("paseo", "https://paritytech.github.io/chainspecs/paseo/relaychain/chainspec.json", None),
    ("asset-hub-paseo", "https://paritytech.github.io/chainspecs/paseo/parachain/asset-hub/chainspec.json", Some("paseo")),
    ("bridge-hub-paseo", "https://paritytech.github.io/chainspecs/paseo/parachain/bridge-hub/chainspec.json", Some("paseo")),
    ("coretime-paseo", "https://paritytech.github.io/chainspecs/paseo/parachain/coretime/chainspec.json", Some("paseo")),
    ("people-paseo", "https://paritytech.github.io/chainspecs/paseo/parachain/people/chainspec.json", Some("paseo")),
];

pub struct ChainSpecSync {
    db: Database,
    client: reqwest::Client,
}

impl ChainSpecSync {
    pub fn new(db: Database) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to create HTTP client");

        Self { db, client }
    }

    /// sync all known chain specs from the official repo
    pub async fn sync_all(&self) -> Result<()> {
        info!("syncing chain specs from Parity repository");

        for (network, url, relay_chain) in CHAINSPEC_MAPPINGS {
            match self.sync_chain_spec(network, url, *relay_chain).await {
                Ok(_) => info!("synced chain spec for {}", network),
                Err(e) => warn!("failed to sync chain spec for {}: {}", network, e),
            }
        }

        Ok(())
    }

    /// sync a single chain spec and extract bootnodes
    pub async fn sync_chain_spec(
        &self,
        network: &str,
        url: &str,
        relay_chain: Option<&str>,
    ) -> Result<()> {
        info!("fetching chain spec for {} from {}", network, url);

        // fetch chain spec
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("failed to fetch chain spec")?;

        let content = response
            .text()
            .await
            .context("failed to read chain spec content")?;

        // parse JSON
        let spec: Value =
            serde_json::from_str(&content).context("failed to parse chain spec JSON")?;

        // extract protocol ID
        let protocol_id = spec
            .get("protocolId")
            .and_then(|v| v.as_str())
            .unwrap_or(network);

        // calculate hash
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let spec_hash = format!("{:x}", hasher.finalize());

        // extract bootnodes
        let bootnodes = spec
            .get("bootNodes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // strip bootnodes from spec content for storage
        let mut spec_without_bootnodes = spec.clone();
        if let Some(obj) = spec_without_bootnodes.as_object_mut() {
            obj.insert("bootNodes".to_string(), Value::Array(vec![]));
        }
        let spec_content = serde_json::to_string(&spec_without_bootnodes)?;

        // upsert chain spec
        self.db
            .upsert_chain_spec(
                network,
                relay_chain,
                protocol_id,
                url,
                Some(&spec_content),
                Some(&spec_hash),
            )
            .await?;

        info!(
            "extracted {} bootnodes from {} chain spec",
            bootnodes.len(),
            network
        );

        // upsert bootnodes
        let mut current_addrs = Vec::new();
        for multiaddr in &bootnodes {
            current_addrs.push(multiaddr.clone());

            // determine protocol from multiaddr
            let protocol = if multiaddr.contains("/wss") {
                "wss"
            } else if multiaddr.contains("/ws") {
                "ws"
            } else if multiaddr.contains("/tcp") {
                "tcp"
            } else {
                "unknown"
            };

            self.db
                .upsert_bootnode(network, None, multiaddr, protocol, "chainspec")
                .await?;
        }

        // deactivate bootnodes that are no longer in the spec
        let deactivated = self
            .db
            .deactivate_missing_bootnodes(network, &current_addrs)
            .await?;

        if deactivated > 0 {
            info!("deactivated {} bootnodes for {}", deactivated, network);
        }

        Ok(())
    }

    /// sync manually configured bootnodes from a JSON file
    pub async fn sync_manual_bootnodes(&self, config_path: &str) -> Result<()> {
        info!("syncing manual bootnodes from {}", config_path);

        let content = tokio::fs::read_to_string(config_path)
            .await
            .context("failed to read bootnodes config")?;

        let config: Value =
            serde_json::from_str(&content).context("failed to parse bootnodes config")?;

        let networks = config
            .as_object()
            .context("bootnodes config must be an object")?;

        for (network, network_config) in networks {
            let members = network_config
                .get("members")
                .and_then(|v| v.as_object())
                .context("network config must have members object")?;

            for (operator, bootnodes) in members {
                let bootnode_list = bootnodes
                    .as_array()
                    .context("bootnodes must be an array")?;

                for bootnode in bootnode_list {
                    let multiaddr = bootnode.as_str().context("bootnode must be a string")?;

                    // determine protocol
                    let protocol = if multiaddr.contains("/wss") {
                        "wss"
                    } else if multiaddr.contains("/ws") {
                        "ws"
                    } else if multiaddr.contains("/tcp") {
                        "tcp"
                    } else {
                        "unknown"
                    };

                    self.db
                        .upsert_bootnode(network, Some(operator), multiaddr, protocol, "manual")
                        .await?;
                }
            }
        }

        Ok(())
    }
}
