// GraphQL RPC + General-Purpose Indexer integration
// This file implements the GraphQL client for structured queries and historical joins
// backed by Sui's general-purpose indexer that transforms checkpoints into a relational store
//
// Numan Thabit 2025 Nov

use crate::metrics::{REQ_ERRORS, REQ_LATENCY};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;
use url::Url;

/// GraphQL RPC client for querying the General-Purpose Indexer
#[derive(Clone)]
pub struct GraphQLRpc {
    endpoint: Url,
    client: reqwest::Client,
}

impl GraphQLRpc {
    pub fn new(endpoint: Url) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .brotli(true)
            .build()
            .context("build HTTP client for GraphQL RPC")?;

        Ok(Self { endpoint, client })
    }

    /// Execute a GraphQL query
    async fn execute_query<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: serde_json::Value,
        operation_name: &str,
    ) -> Result<T> {
        let _timer = REQ_LATENCY
            .with_label_values(&["graphql", operation_name])
            .start_timer();

        let request_body = serde_json::json!({
            "query": query,
            "variables": variables,
            "operationName": operation_name,
        });

        let response = self
            .client
            .post(self.endpoint.clone())
            .json(&request_body)
            .send()
            .await
            .context("send GraphQL request")?;

        let status = response.status();
        if !status.is_success() {
            REQ_ERRORS
                .with_label_values(&["graphql", operation_name])
                .inc();
            return Err(anyhow::anyhow!(
                "GraphQL request failed with status: {}",
                status
            ));
        }

        let response_body: GraphQLResponse<T> = response
            .json()
            .await
            .context("parse GraphQL response JSON")?;

        if let Some(errors) = &response_body.errors {
            REQ_ERRORS
                .with_label_values(&["graphql", operation_name])
                .inc();
            warn!(
                operation = operation_name,
                errors = ?errors,
                "GraphQL query returned errors"
            );
            return Err(anyhow::anyhow!(
                "GraphQL errors: {}",
                errors
                    .iter()
                    .map(|e| e.message.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        response_body.data.context("missing GraphQL response data")
    }

    /// Query checkpoints with optional filters
    pub async fn query_checkpoints(
        &self,
        filter: Option<CheckpointFilter>,
        first: Option<u64>,
        after: Option<String>,
    ) -> Result<CheckpointConnection> {
        let query = r#"
            query Checkpoints($filter: CheckpointFilter, $first: Int, $after: String) {
                checkpoints(filter: $filter, first: $first, after: $after) {
                    nodes {
                        sequenceNumber
                        digest
                        timestampMs
                        previousCheckpointDigest
                        epochId
                        networkTotalTransactions
                    }
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                }
            }
        "#;

        let mut variables = serde_json::json!({});
        if let Some(f) = filter {
            if let Some(seq) = f.checkpoint_sequence_number {
                variables["filter"] = serde_json::json!({
                    "checkpointSequenceNumber": {
                        "eq": seq
                    }
                });
            }
        }
        if let Some(f) = first {
            variables["first"] = serde_json::json!(f);
        }
        if let Some(a) = after {
            variables["after"] = serde_json::json!(a);
        }

        let response: GraphQLCheckpointsResponse =
            self.execute_query(query, variables, "Checkpoints").await?;

        Ok(response.checkpoints)
    }

    /// Query transactions with filters
    pub async fn query_transactions(
        &self,
        filter: Option<TransactionFilter>,
        first: Option<u64>,
        after: Option<String>,
    ) -> Result<TransactionConnection> {
        let query = r#"
            query Transactions($filter: TransactionFilter, $first: Int, $after: String) {
                transactions(filter: $filter, first: $first, after: $after) {
                    nodes {
                        digest
                        effects {
                            status {
                                __typename
                            }
                            gasUsed {
                                computationCost
                                storageCost
                                storageRebate
                            }
                        }
                        sender {
                            address
                        }
                        gasInput {
                            gasPayment {
                                objectId
                            }
                        }
                        kind {
                            __typename
                            ... on ProgrammableTransactionBlock {
                                transactions {
                                    __typename
                                }
                            }
                        }
                    }
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                }
            }
        "#;

        let mut variables = serde_json::json!({});
        if let Some(f) = filter {
            if let Some(digest) = f.transaction_digest {
                variables["filter"] = serde_json::json!({
                    "transactionDigest": {
                        "eq": digest
                    }
                });
            }
        }
        if let Some(f) = first {
            variables["first"] = serde_json::json!(f);
        }
        if let Some(a) = after {
            variables["after"] = serde_json::json!(a);
        }

        let response: GraphQLTransactionsResponse =
            self.execute_query(query, variables, "Transactions").await?;

        Ok(response.transactions)
    }

    /// Query objects with filters
    pub async fn query_objects(
        &self,
        filter: Option<ObjectFilter>,
        first: Option<u64>,
        after: Option<String>,
    ) -> Result<ObjectConnection> {
        let query = r#"
            query Objects($filter: ObjectFilter, $first: Int, $after: String) {
                objects(filter: $filter, first: $first, after: $after) {
                    nodes {
                        objectId
                        version
                        digest
                        owner {
                            __typename
                            ... on AddressOwner {
                                owner {
                                    address
                                }
                            }
                            ... on ObjectOwner {
                                owner {
                                    address
                                }
                            }
                        }
                        previousTransaction
                        storageRebate
                        bcs
                    }
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                }
            }
        "#;

        let mut variables = serde_json::json!({});
        if let Some(f) = filter {
            let mut filter_obj = serde_json::json!({});
            if let Some(oid) = f.object_id {
                filter_obj["objectId"] = serde_json::json!({
                    "eq": oid
                });
            }
            if let Some(owner) = f.owner {
                match owner {
                    OwnerFilter::Address(addr) => {
                        filter_obj["owner"] = serde_json::json!({
                            "AddressOwner": {
                                "owner": {
                                    "eq": addr
                                }
                            }
                        });
                    }
                    OwnerFilter::Object(oid) => {
                        filter_obj["owner"] = serde_json::json!({
                            "ObjectOwner": {
                                "owner": {
                                    "eq": oid
                                }
                            }
                        });
                    }
                    OwnerFilter::Shared => {
                        filter_obj["owner"] = serde_json::json!("Shared");
                    }
                }
            }
            if !filter_obj.is_null() {
                variables["filter"] = filter_obj;
            }
        }
        if let Some(f) = first {
            variables["first"] = serde_json::json!(f);
        }
        if let Some(a) = after {
            variables["after"] = serde_json::json!(a);
        }

        let response: GraphQLObjectsResponse =
            self.execute_query(query, variables, "Objects").await?;

        Ok(response.objects)
    }

    /// Query events with filters
    pub async fn query_events(
        &self,
        filter: Option<EventFilter>,
        first: Option<u64>,
        after: Option<String>,
    ) -> Result<EventConnection> {
        let query = r#"
            query Events($filter: EventFilter, $first: Int, $after: String) {
                events(filter: $filter, first: $first, after: $after) {
                    nodes {
                        id
                        transactionDigest
                        sender {
                            address
                        }
                        timestampMs
                        bcs
                    }
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                }
            }
        "#;

        let mut variables = serde_json::json!({});
        if let Some(f) = filter {
            if let Some(digest) = f.transaction_digest {
                variables["filter"] = serde_json::json!({
                    "transactionDigest": {
                        "eq": digest
                    }
                });
            }
        }
        if let Some(f) = first {
            variables["first"] = serde_json::json!(f);
        }
        if let Some(a) = after {
            variables["after"] = serde_json::json!(a);
        }

        let response: GraphQLEventsResponse =
            self.execute_query(query, variables, "Events").await?;

        Ok(response.events)
    }

    /// Get checkpoint by sequence number (for historical queries)
    pub async fn get_checkpoint(&self, sequence_number: u64) -> Result<Option<Checkpoint>> {
        let connection = self
            .query_checkpoints(
                Some(CheckpointFilter {
                    checkpoint_sequence_number: Some(sequence_number),
                }),
                Some(1),
                None,
            )
            .await?;

        Ok(connection.nodes.into_iter().next())
    }

    /// Get transaction by digest
    pub async fn get_transaction(&self, digest: String) -> Result<Option<Transaction>> {
        let connection = self
            .query_transactions(
                Some(TransactionFilter {
                    transaction_digest: Some(digest),
                }),
                Some(1),
                None,
            )
            .await?;

        Ok(connection.nodes.into_iter().next())
    }

    /// Get object by ID
    pub async fn get_object(&self, object_id: String) -> Result<Option<Object>> {
        let connection = self
            .query_objects(
                Some(ObjectFilter {
                    object_id: Some(object_id),
                    owner: None,
                }),
                Some(1),
                None,
            )
            .await?;

        Ok(connection.nodes.into_iter().next())
    }

    /// Historical join: Get all transactions in a checkpoint range
    /// This is useful for compliance queries and historical analysis
    pub async fn get_transactions_in_checkpoint_range(
        &self,
        start_sequence: u64,
        end_sequence: u64,
        limit: Option<u64>,
    ) -> Result<Vec<Transaction>> {
        // Note: This is a simplified implementation. A full implementation would
        // need to paginate through checkpoints and join with transactions.
        // The GraphQL indexer should support this via nested queries.
        let mut all_transactions = Vec::new();
        let mut current_seq = start_sequence;

        while current_seq <= end_sequence {
            if self.get_checkpoint(current_seq).await?.is_none() {
                current_seq += 1;
                continue;
            }

            // Query transactions for this checkpoint
            // In practice, the GraphQL schema should support nested queries
            // like checkpoint { transactions { ... } }
            let transactions = self
                .query_transactions(None, limit, None)
                .await
                .context("query transactions for checkpoint range")?;

            all_transactions.extend(transactions.nodes);

            if let Some(limit) = limit {
                if all_transactions.len() >= limit as usize {
                    break;
                }
            }

            current_seq += 1;
        }

        Ok(all_transactions)
    }

    /// Compliance query: Get all transactions for a specific address within a time range
    /// This is useful for audit trails and compliance reporting
    pub async fn get_address_transactions(
        &self,
        address: &str,
        _start_timestamp_ms: Option<u64>,
        _end_timestamp_ms: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Vec<Transaction>> {
        // Query transactions with sender filter
        // Note: The actual GraphQL schema may need to support timestamp filtering
        // This is a placeholder that demonstrates the pattern
        let connection = self
            .query_transactions(None, limit, None)
            .await
            .context("query transactions for address")?;

        // Filter by sender address (in production, this should be done in GraphQL query)
        let filtered: Vec<Transaction> = connection
            .nodes
            .into_iter()
            .filter(|tx| {
                tx.sender
                    .as_ref()
                    .map(|s| s.address == address)
                    .unwrap_or(false)
            })
            .collect();

        Ok(filtered)
    }

    /// Compliance query: Get all events for a transaction digest
    /// Useful for tracing transaction effects and event emissions
    pub async fn get_transaction_events(&self, transaction_digest: &str) -> Result<Vec<Event>> {
        let connection = self
            .query_events(
                Some(EventFilter {
                    transaction_digest: Some(transaction_digest.to_string()),
                }),
                None,
                None,
            )
            .await?;

        Ok(connection.nodes)
    }

    /// Get latest checkpoint sequence number
    /// Useful for tracking chain progress and determining query ranges
    pub async fn get_latest_checkpoint(&self) -> Result<Option<Checkpoint>> {
        let connection = self.query_checkpoints(None, Some(1), None).await?;

        Ok(connection.nodes.into_iter().next())
    }
}

// GraphQL Response wrappers

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Part of GraphQL error response structure
struct GraphQLError {
    message: String,
    #[serde(default)]
    locations: Option<Vec<GraphQLLocation>>,
    #[serde(default)]
    path: Option<Vec<serde_json::Value>>,
}

#[allow(dead_code)] // Part of GraphQL error response structure
#[derive(Debug, Deserialize)]
struct GraphQLLocation {
    line: u32,
    column: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLCheckpointsResponse {
    checkpoints: CheckpointConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLTransactionsResponse {
    transactions: TransactionConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLObjectsResponse {
    objects: ObjectConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLEventsResponse {
    events: EventConnection,
}

// Type aliases and filter structs for easier API usage

#[derive(Debug, Clone)]
pub struct CheckpointFilter {
    pub checkpoint_sequence_number: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TransactionFilter {
    pub transaction_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ObjectFilter {
    pub object_id: Option<String>,
    pub owner: Option<OwnerFilter>,
}

#[derive(Debug, Clone)]
pub enum OwnerFilter {
    Address(String),
    Object(String),
    Shared,
}

#[derive(Debug, Clone)]
pub struct EventFilter {
    pub transaction_digest: Option<String>,
}

// Response types (simplified - in production these would match the GraphQL schema exactly)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointConnection {
    pub nodes: Vec<Checkpoint>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionConnection {
    pub nodes: Vec<Transaction>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectConnection {
    pub nodes: Vec<Object>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventConnection {
    pub nodes: Vec<Event>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub sequence_number: u64,
    pub digest: String,
    pub timestamp_ms: Option<u64>,
    pub previous_checkpoint_digest: Option<String>,
    pub epoch_id: Option<u64>,
    pub network_total_transactions: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub digest: String,
    pub effects: Option<TransactionEffects>,
    pub sender: Option<SenderAddress>,
    pub gas_input: Option<GasInput>,
    pub kind: Option<TransactionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SenderAddress {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEffects {
    pub status: Option<TransactionStatus>,
    pub gas_used: Option<GasCostSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStatus {
    #[serde(rename = "__typename")]
    pub typename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasCostSummary {
    pub computation_cost: Option<u64>,
    pub storage_cost: Option<u64>,
    pub storage_rebate: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasInput {
    pub gas_payment: Option<Vec<GasPayment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasPayment {
    pub object_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionKind {
    #[serde(rename = "__typename")]
    pub typename: Option<String>,
    pub programmable_transaction_block: Option<ProgrammableTransactionBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammableTransactionBlock {
    pub transactions: Option<Vec<TransactionCommand>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionCommand {
    #[serde(rename = "__typename")]
    pub typename: Option<String>,
    // Simplified - actual structure depends on Sui's GraphQL schema
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Object {
    pub object_id: String,
    pub version: Option<u64>,
    pub digest: Option<String>,
    pub owner: Option<ObjectOwner>,
    pub previous_transaction: Option<String>,
    pub storage_rebate: Option<u64>,
    pub bcs: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectOwner {
    #[serde(rename = "__typename")]
    pub typename: Option<String>,
    // Simplified - actual structure depends on Sui's GraphQL schema
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: Option<String>,
    pub transaction_digest: Option<String>,
    pub sender: Option<EventSenderAddress>,
    pub timestamp_ms: Option<u64>,
    pub bcs: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSenderAddress {
    pub address: String,
}
