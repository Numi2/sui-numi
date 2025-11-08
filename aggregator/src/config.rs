// Configuration management module
// This file handles loading and parsing of configuration settings
// from environment variables, config files, and command-line arguments
//
// Numan Thabit 2025 Nov

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::str::FromStr;
use sui_deepbookv3::utils::config::Environment;
use sui_sdk::types::base_types::SuiAddress;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// gRPC fullnode endpoint, e.g. https://fullnode.mainnet.sui.io:443
    pub grpc_endpoint: Url,
    /// JSON-RPC endpoint for execute fallback, e.g. https://fullnode.mainnet.sui.io:443
    pub jsonrpc_endpoint: Url,
    /// GraphQL RPC + General-Purpose Indexer endpoint (optional)
    pub graphql_endpoint: Option<Url>,
    /// DeepBook public indexer (optional; defaults to Mysten Labs public indexer)
    pub deepbook_indexer: Option<Url>,
    /// Sui address of the trading account
    pub address: String,
    /// Hex-encoded 32-byte Ed25519 private key (do not use in prod; replace with HSM)
    pub ed25519_secret_hex: String,
    /// Concurrency control
    pub max_inflight: usize,
    /// Feature switch: use gRPC ExecuteTransaction
    pub use_grpc_execute: Option<bool>,
    /// DeepBook environment selector (mainnet/testnet)
    pub deepbook_env: Option<String>,
    /// BalanceManager object id (0x...)
    pub deepbook_manager_object: Option<String>,
    /// Label used when registering the BalanceManager with the SDK (defaults MANAGER_1)
    pub deepbook_manager_label: Option<String>,
    /// Sponsored transaction configuration (optional)
    pub sponsorship: Option<SponsorshipConfig>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let cfg = config::Config::builder()
            .add_source(config::Environment::default().separator("__"))
            .build()?;
        Ok(cfg.try_deserialize()?)
    }

    pub fn sui_address(&self) -> Result<SuiAddress> {
        SuiAddress::from_str(&self.address)
            .with_context(|| format!("invalid Sui address: {}", self.address))
    }

    pub fn deepbook_settings(&self) -> Result<Option<DeepBookSettings>> {
        let indexer = match &self.deepbook_indexer {
            Some(url) => url.clone(),
            None => return Ok(None),
        };

        let manager_object = self.deepbook_manager_object.clone().with_context(|| {
            "APP__DEEPBOOK_MANAGER_OBJECT is required when DeepBook indexer is set"
        })?;

        let manager_label = self
            .deepbook_manager_label
            .clone()
            .unwrap_or_else(|| "MANAGER_1".to_string());

        let env = self
            .deepbook_env
            .as_deref()
            .unwrap_or("mainnet")
            .to_ascii_lowercase();
        let environment = match env.as_str() {
            "mainnet" | "prod" => Environment::Mainnet,
            "testnet" => Environment::Testnet,
            other => bail!("unsupported deepbook environment: {other}"),
        };

        Ok(Some(DeepBookSettings {
            indexer,
            environment,
            balance_manager_object: manager_object,
            balance_manager_label: manager_label,
        }))
    }
}

#[derive(Debug, Clone)]
pub struct DeepBookSettings {
    pub indexer: Url,
    pub environment: Environment,
    pub balance_manager_object: String,
    pub balance_manager_label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SponsorshipConfig {
    /// Sponsor's Sui address
    pub sponsor_address: String,
    /// Hex-encoded 32-byte Ed25519 private key for sponsor (do not use in prod; replace with HSM)
    pub sponsor_key_hex: String,
    /// Per-user budget (in gas units)
    pub per_user_budget: Option<u64>,
    /// Per-transaction limit (in gas units)
    pub per_tx_limit: Option<u64>,
    /// Budget window duration in seconds (None = no reset)
    pub budget_window_seconds: Option<u64>,
    /// Max transactions per user per abuse detection window
    pub max_tx_per_window: Option<u64>,
    /// Max gas per user per abuse detection window
    pub max_gas_per_window: Option<u64>,
    /// Abuse detection window duration in seconds
    pub abuse_window_seconds: Option<u64>,
}

impl SponsorshipConfig {
    pub fn sponsor_address_parsed(&self) -> Result<SuiAddress> {
        SuiAddress::from_str(&self.sponsor_address)
            .with_context(|| format!("invalid sponsor address: {}", self.sponsor_address))
    }
}
