// src/cli.rs
use anyhow::Result;
use serde::Deserialize;
use std::{path::PathBuf, fs};
use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Parallel bootnode tester for Polkadot networks")]
pub struct Cli {
    #[arg(long, default_value = "/usr/local/bin/polkadot")]
    pub polkadot_binary: PathBuf,

    #[arg(long, default_value = "/usr/local/bin/polkadot-parachain")]
    pub parachain_binary: PathBuf,

    #[arg(long, default_value = "/usr/local/bin/encointer")]
    pub encointer_binary: PathBuf,

    #[arg(long, default_value = "/tmp/bootnode_tests")]
    pub output_dir: PathBuf,

    #[arg(long, default_value = "/tmp/bootnode_data")]
    pub data_dir: PathBuf,

    #[arg(long, default_value = "./chain-spec")]
    pub chain_spec_dir: PathBuf,

    #[arg(long, default_value = "10")]
    pub max_concurrent: usize,

    #[arg(long, default_value = "49615")]
    pub base_port: u16,

    #[arg(long, default_value = "9615")]
    pub prometheus_port: u16,

    #[arg(long, default_value = "30")]
    pub timeout: u64,

    #[arg(long, default_value = "bootnodes.json")]
    pub bootnodes_config: PathBuf,

    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub debug: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TomlConfig {
    pub polkadot_binary: Option<PathBuf>,
    pub parachain_binary: Option<PathBuf>,
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
    #[serde(rename = "commandId")]
    pub command_id: String,
    pub members: std::collections::HashMap<String, Vec<String>>,
}

impl Cli {
    pub fn merge_with_toml(&mut self, config: TomlConfig) {
        if let Some(v) = config.polkadot_binary { self.polkadot_binary = v; }
        if let Some(v) = config.parachain_binary { self.parachain_binary = v; }
        if let Some(v) = config.output_dir { self.output_dir = v; }
        if let Some(v) = config.data_dir { self.data_dir = v; }
        if let Some(v) = config.chain_spec_dir { self.chain_spec_dir = v; }
        if let Some(v) = config.max_concurrent { self.max_concurrent = v; }
        if let Some(v) = config.base_port { self.base_port = v; }
        if let Some(v) = config.timeout { self.timeout = v; }
        if let Some(v) = config.bootnodes_config { self.bootnodes_config = v; }
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
