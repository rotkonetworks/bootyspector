// src/cli.rs
use anyhow::Result;
use clap::Parser;
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Parser, Debug, Clone)]
#[command(
    author,
    version,
    about = "Parallel bootnode tester for Polkadot networks"
)]
pub struct Cli {
    /// path to the polkadot-omni-node binary
    #[arg(long, default_value = "/usr/local/bin/polkadot-omni-node")]
    pub node_binary: PathBuf,

    #[arg(long, default_value = "/tmp/bootnode_tests")]
    pub output_dir: PathBuf,

    #[arg(long, default_value = "/tmp/bootnode_data")]
    pub data_dir: PathBuf,

    /// path to the chain spec directory
    #[arg(long, default_value = "./chain-spec")]
    pub chain_spec_dir: PathBuf,

    /// maximum number of concurrent tests
    #[arg(long, default_value = "1")]
    pub max_concurrent: usize,

    /// minimum number of peers to pass
    #[arg(long, default_value = "1")]
    pub min_peers: u64,

    /// test interval in seconds
    #[arg(long, default_value = "3600")]
    pub interval: u64,

    #[arg(long, default_value = "49615")]
    pub base_port: u16,

    #[arg(long, default_value = "9615")]
    pub prometheus_port: u16,

    /// test ttl in seconds
    #[arg(long, default_value = "30")]
    pub timeout: u64,

    /// path to the bootnodes config file
    #[arg(long, default_value = "bootnodes.json")]
    pub bootnodes_config: PathBuf,

    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub debug: bool,

    /// use smoldot light client instead of litep2p
    #[arg(long)]
    pub use_smoldot: bool,

    /// path to SQLite database file
    #[arg(long, default_value = "bootyspector.db")]
    pub database: PathBuf,

    /// sync chain specs from Parity repo before testing
    #[arg(long)]
    pub sync: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TomlConfig {
    pub node_binary: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub chain_spec_dir: Option<PathBuf>,
    pub max_concurrent: Option<usize>,
    pub base_port: Option<u16>,
    pub timeout: Option<u64>,
    pub bootnodes_config: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct BootnodesConfig {
    #[serde(flatten)]
    pub networks: std::collections::HashMap<String, NetworkConfig>,
}

#[derive(Debug, Deserialize)]
pub struct NetworkConfig {
    pub members: std::collections::HashMap<String, Vec<String>>,
}

impl Cli {
    pub fn merge_with_toml(&mut self, config: TomlConfig) {
        if let Some(v) = config.node_binary {
            self.node_binary = v;
        }
        if let Some(v) = config.output_dir {
            self.output_dir = v;
        }
        if let Some(v) = config.data_dir {
            self.data_dir = v;
        }
        if let Some(v) = config.chain_spec_dir {
            self.chain_spec_dir = v;
        }
        if let Some(v) = config.max_concurrent {
            self.max_concurrent = v;
        }
        if let Some(v) = config.base_port {
            self.base_port = v;
        }
        if let Some(v) = config.timeout {
            self.timeout = v;
        }
        if let Some(v) = config.bootnodes_config {
            self.bootnodes_config = v;
        }
    }

    pub fn load() -> Result<Self> {
        let mut cli = Self::parse();

        if let Some(config_path) = &cli.config {
            if let Ok(config_str) = fs::read_to_string(config_path) {
                if let Ok(toml_config) = toml::from_str::<TomlConfig>(&config_str) {
                    cli.merge_with_toml(toml_config);
                }
            }
        }

        Ok(cli)
    }
}
