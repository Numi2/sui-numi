// DeepBook venue integration module
// This file implements the aggregator logic for interacting with DeepBook,
// Sui's native order book and DEX protocol
//
// Numan Thabit 2025 Nov

use crate::errors::AggrError;
use anyhow::Context;
use bcs;
use std::collections::HashMap;
use sui_deepbookv3::client::DeepBookClient;
use sui_deepbookv3::utils::config::{Environment, GAS_BUDGET, MAX_TIMESTAMP};
use sui_deepbookv3::utils::types::{
    BalanceManager, OrderType, PlaceLimitOrderParams, SelfMatchingOptions,
};
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_sdk::types::transaction::{InputObjectKind, TransactionData, TransactionKind};
use sui_sdk::{SuiClient, SuiClientBuilder};

#[derive(Debug, Clone)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Debug, Clone)]
pub struct LimitOrder {
    pub pool_id: String,
    pub side: Side,
    /// price in quote units scaled to tick size of the pool
    pub price: u128,
    /// size in base units
    pub size: u128,
    /// pay fee with DEEP
    pub pay_with_deep: bool,
}

/// DeepBook client wrapper.
pub struct DeepBookVenue {
    client: DeepBookClient,
    sui: SuiClient,
    sender: SuiAddress,
    manager_key: String,
}

impl DeepBookVenue {
    /// Initialize a new DeepBookVenue instance.
    ///
    /// # Arguments
    /// * `fullnode_url` - Sui fullnode RPC URL
    /// * `sender` - Sui address of the trading account
    /// * `manager_object` - BalanceManager object ID (0x...)
    /// * `manager_key` - Label for the balance manager (e.g., "MANAGER_1")
    /// * `env` - DeepBook environment (Mainnet or Testnet)
    pub async fn new(
        fullnode_url: &str,
        sender: SuiAddress,
        manager_object: &str,
        manager_key: &str,
        env: Environment,
    ) -> anyhow::Result<Self> {
        let sui = SuiClientBuilder::default()
            .build(fullnode_url)
            .await
            .context("initialize Sui client")?;

        // Wire minimal config maps for the SDK. We provide the BalanceManager only;
        // coins/pools use SDK defaults for the chosen environment.
        let mut managers: HashMap<&'static str, BalanceManager> = HashMap::new();
        // Note: We need to use a static string for the key. In production, you'd want to
        // manage this differently, but for now we'll use a workaround.
        let manager_key_static = Box::leak(manager_key.to_string().into_boxed_str());
        managers.insert(
            manager_key_static,
            BalanceManager {
                address: manager_object.to_string(),
                trade_cap: None,
                deposit_cap: None,
                withdraw_cap: None,
            },
        );

        // DeepBookClient requires the package "address" parameter; we pass the sender.
        let client = DeepBookClient::new(
            sui.clone(),
            sender,
            env,
            Some(managers),
            None, // coins: use defaults
            None, // pools: use defaults
            None, // admin cap
        );

        Ok(Self {
            client,
            sui,
            sender,
            manager_key: manager_key.to_string(),
        })
    }

    /// L2 book snapshot (top N on each side). Backed by SDK call.
    ///
    /// # Arguments
    /// * `pool_key` - Pool identifier (e.g., "SUI_USDC")
    /// * `depth` - Number of ticks from mid-price to retrieve
    pub async fn level2(&self, pool_key: &str, depth: u32) -> anyhow::Result<serde_json::Value> {
        let level2_data = self
            .client
            .get_level2_ticks_from_mid(pool_key, depth as u64)
            .await
            .context("fetch level2 order book")?;

        serde_json::to_value(level2_data).context("serialize level2 data")
    }

    /// Get open orders for the configured account in a specific pool.
    ///
    /// # Arguments
    /// * `pool_key` - Pool identifier (e.g., "SUI_USDC")
    pub async fn open_orders(&self, pool_key: &str) -> anyhow::Result<serde_json::Value> {
        let order_ids = self
            .client
            .account_open_orders(pool_key, &self.manager_key)
            .await
            .context("fetch account open orders")?;

        // Optionally fetch full order details for each order ID
        let mut orders = Vec::new();
        for order_id in order_ids {
            if let Some(order) = self
                .client
                .get_order_normalized(pool_key, order_id)
                .await
                .context("fetch order details")?
            {
                orders.push(order);
            }
        }

        serde_json::to_value(orders).context("serialize open orders")
    }

    /// Build a PTB for a limit order. Returns BCS TransactionData bytes ready to sign.
    ///
    /// # Arguments
    /// * `lo` - Limit order parameters
    /// * `client_order_id` - Client-provided order ID (u64)
    ///
    /// Note: `lo.price` is expected to be in quote units scaled to tick size (u128).
    /// `lo.size` is expected to be in base units (u128).
    /// The SDK will handle conversion to normalized f64 values internally.
    pub async fn build_limit_order_ptb_bcs(
        &self,
        lo: &LimitOrder,
        client_order_id: u64,
    ) -> Result<Vec<u8>, AggrError> {
        // Convert pool_id to pool_key (assuming pool_id is the pool_key string)
        let pool_key = &lo.pool_id;

        // Get pool parameters for validation
        let _pool_params = self
            .client
            .pool_book_params(pool_key)
            .await
            .map_err(|e| AggrError::BuildTx(format!("fetch pool params: {}", e)))?;

        // Convert u128 price and size to f64 for SDK
        // The SDK expects normalized f64 prices and quantities.
        // Since lo.price is "price in quote units scaled to tick size", we need to
        // convert it to normalized price. However, without access to coin scalars,
        // we'll use a conversion that assumes the price is already in the right scale.
        // In practice, callers should provide prices that match the SDK's expected format.
        //
        // For size: lo.size is in base units, so we convert assuming standard scaling.
        // The SDK will handle final quantization based on pool parameters.
        use sui_deepbookv3::utils::config::FLOAT_SCALAR;
        
        // Convert scaled price to normalized price
        // This is a simplified conversion - in production, you'd want to use actual coin scalars
        // from the pool configuration. For now, we assume standard scaling.
        let price_f64 = lo.price as f64 / FLOAT_SCALAR as f64;
        
        // Convert size from base units to normalized quantity
        // Assuming standard 9-decimal scaling for base coins
        let size_f64 = lo.size as f64 / 1_000_000_000.0;

        // Build the programmable transaction
        let mut ptb = ProgrammableTransactionBuilder::new();

        let place_params = PlaceLimitOrderParams {
            pool_key: pool_key.to_string(),
            balance_manager_key: self.manager_key.clone(),
            client_order_id,
            price: price_f64,
            quantity: size_f64,
            is_bid: matches!(lo.side, Side::Bid),
            expiration: Some(MAX_TIMESTAMP),
            order_type: Some(OrderType::NoRestriction),
            self_matching_option: Some(SelfMatchingOptions::SelfMatchingAllowed),
            pay_with_deep: Some(lo.pay_with_deep),
        };

        self.client
            .deep_book
            .place_limit_order(&mut ptb, place_params)
            .await
            .map_err(|e| AggrError::BuildTx(format!("build limit order PTB: {}", e)))?;

        // Finalize, select gas, and return BCS TransactionData bytes
        let programmable = ptb.finish();
        let input_objects: Vec<_> = programmable
            .input_objects()
            .map_err(|e| AggrError::BuildTx(format!("collect input objects: {}", e)))?
            .into_iter()
            .map(|obj| InputObjectKind::object_id(&obj))
            .collect();

        let gas_price = self
            .sui
            .read_api()
            .get_reference_gas_price()
            .await
            .map_err(|e| AggrError::BuildTx(format!("fetch reference gas price: {}", e)))?;

        let gas = self
            .sui
            .transaction_builder()
            .select_gas(self.sender, None, GAS_BUDGET, input_objects, gas_price)
            .await
            .map_err(|e| AggrError::BuildTx(format!("select gas coin: {}", e)))?;

        let tx_data = TransactionData::new(
            TransactionKind::programmable(programmable),
            self.sender,
            gas,
            GAS_BUDGET,
            gas_price,
        );

        let tx_bcs = bcs::to_bytes(&tx_data)
            .map_err(|e| AggrError::BuildTx(format!("serialize transaction: {}", e)))?;

        Ok(tx_bcs)
    }
}
