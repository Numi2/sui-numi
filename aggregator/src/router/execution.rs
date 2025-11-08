// Execution engine - compiles routes to PTBs, signs, submits, and handles retries
// This file implements the execution plane that submits through Transaction Driver
// with idempotent retry logic
//
// Numan Thabit 2025 Nov

use crate::errors::AggrError;
use crate::router::routes::RoutePlan;
use crate::router::validator::ValidatorSelector;
use crate::signing::sign_tx_bcs_ed25519_to_serialized_signature;
use crate::sponsorship::{SponsorshipManager, SponsorshipRequest};
use crate::transport::grpc::GrpcClients;
use crate::transport::jsonrpc::JsonRpc;
use crate::venues::adapter::DeepBookAdapter;
use anyhow::{Context, Result};
use backoff::{future::retry, ExponentialBackoff};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use crate::transport::grpc::sui::rpc::v2::ExecutedTransaction;
use tracing::{info, warn};

/// Execution statistics for monitoring
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecutionStats {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub avg_effects_time_ms: Option<f64>,
    pub avg_checkpoint_time_ms: Option<f64>,
    pub success_rate: f64,
}

/// Execution result with timing information
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub digest: String,
    pub executed: ExecutedTransaction,
    /// Time from submission to effects observed (milliseconds)
    pub effects_time_ms: f64,
    /// Time from submission to checkpoint inclusion (milliseconds)
    pub checkpoint_time_ms: Option<f64>,
}

/// Execution engine that compiles routes to PTBs and executes them
pub struct ExecutionEngine {
    deepbook: Option<Arc<DeepBookAdapter>>,
    grpc: Arc<tokio::sync::Mutex<GrpcClients>>,
    jsonrpc: Arc<JsonRpc>,
    validator_selector: Arc<ValidatorSelector>,
    secret_key_hex: String,
    /// User's Sui address (derived from secret key or from config)
    user_address: sui_sdk::types::base_types::SuiAddress,
    /// Set of transaction digests we've seen (for idempotent retries)
    seen_digests: Arc<tokio::sync::RwLock<HashSet<String>>>,
    /// Use gRPC execution if available
    use_grpc_execute: bool,
    /// Optional sponsorship manager for sponsored transactions
    sponsorship: Option<Arc<SponsorshipManager>>,
    /// Execution statistics
    total_executions: AtomicU64,
    successful_executions: AtomicU64,
    failed_executions: AtomicU64,
    total_effects_time_ms: AtomicU64, // Sum of all effects times in milliseconds (as u64 * 1000 for precision)
    total_checkpoint_time_ms: AtomicU64, // Sum of all checkpoint times in milliseconds
    checkpoint_count: AtomicU64,
}

impl ExecutionEngine {
    pub fn new(
        deepbook: Option<Arc<DeepBookAdapter>>,
        grpc: GrpcClients,
        jsonrpc: JsonRpc,
        validator_selector: Arc<ValidatorSelector>,
        secret_key_hex: String,
        user_address: sui_sdk::types::base_types::SuiAddress,
        use_grpc_execute: bool,
    ) -> Self {
        Self {
            deepbook,
            grpc: Arc::new(tokio::sync::Mutex::new(grpc)),
            jsonrpc: Arc::new(jsonrpc),
            validator_selector,
            secret_key_hex,
            user_address,
            seen_digests: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            use_grpc_execute,
            sponsorship: None,
            total_executions: AtomicU64::new(0),
            successful_executions: AtomicU64::new(0),
            failed_executions: AtomicU64::new(0),
            total_effects_time_ms: AtomicU64::new(0),
            total_checkpoint_time_ms: AtomicU64::new(0),
            checkpoint_count: AtomicU64::new(0),
        }
    }

    /// Set sponsorship manager for sponsored transactions
    pub fn with_sponsorship(mut self, sponsorship: Arc<SponsorshipManager>) -> Self {
        self.sponsorship = Some(sponsorship);
        self
    }

    /// Execute a route plan
    pub async fn execute(&self, plan: &RoutePlan) -> Result<ExecutionResult> {
        self.execute_with_sponsorship(plan, false).await
    }

    /// Set sponsorship manager for sponsored transactions
    pub fn set_sponsorship(&self, _sponsorship: Arc<SponsorshipManager>) {
        // Note: This requires interior mutability, so we'll need to wrap sponsorship in Arc<Mutex<Option<...>>>
        // For now, sponsorship should be set during construction
        warn!("set_sponsorship called but sponsorship is immutable after construction");
    }

    /// Get execution statistics
    pub fn get_stats(&self) -> ExecutionStats {
        let total = self.total_executions.load(Ordering::Relaxed);
        let successful = self.successful_executions.load(Ordering::Relaxed);
        let failed = self.failed_executions.load(Ordering::Relaxed);
        let total_effects_ms = self.total_effects_time_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        let total_checkpoint_ms = self.total_checkpoint_time_ms.load(Ordering::Relaxed) as f64 / 1000.0;
        let checkpoint_count = self.checkpoint_count.load(Ordering::Relaxed);

        ExecutionStats {
            total_executions: total,
            successful_executions: successful,
            failed_executions: failed,
            avg_effects_time_ms: if successful > 0 {
                Some(total_effects_ms / successful as f64)
            } else {
                None
            },
            avg_checkpoint_time_ms: if checkpoint_count > 0 {
                Some(total_checkpoint_ms / checkpoint_count as f64)
            } else {
                None
            },
            success_rate: if total > 0 {
                successful as f64 / total as f64
            } else {
                0.0
            },
        }
    }

    /// Execute a route plan with optional sponsorship
    #[tracing::instrument(skip_all, fields(uses_sponsorship = use_sponsorship))]
    pub async fn execute_with_sponsorship(
        &self,
        plan: &RoutePlan,
        use_sponsorship: bool,
    ) -> Result<ExecutionResult> {
        self.total_executions.fetch_add(1, Ordering::Relaxed);
        // 1. Compile route to PTB (may be gasless if sponsorship is enabled)
        let (tx_bcs, is_sponsored) = if use_sponsorship && self.sponsorship.is_some() {
            self.compile_route_sponsored(plan).await?
        } else {
            (self.compile_route(plan).await?, false)
        };

        // 2. Sign transaction(s)
        let signatures = if is_sponsored {
            // For sponsored transactions, we need both user and sponsor signatures
            self.sign_sponsored_transaction(&tx_bcs).await?
        } else {
            // Regular transaction: just user signature
            let (signature_bytes, _pubkey) = sign_tx_bcs_ed25519_to_serialized_signature(
                &tx_bcs,
                &self.secret_key_hex,
            )
            .map_err(|e| AggrError::Signing(e.to_string()))?;
            vec![signature_bytes]
        };

        // 3. Compute transaction digest (for idempotency check)
        let digest = self.compute_digest(&tx_bcs)?;

        // 4. Check if we've already seen this digest (idempotent retry)
        {
            let seen = self.seen_digests.read().await;
            if seen.contains(&digest) {
                warn!(
                    digest = %digest,
                    "transaction digest already seen, skipping duplicate execution"
                );
                self.failed_executions.fetch_add(1, Ordering::Relaxed);
                anyhow::bail!("transaction already executed: {}", digest);
            }
        }

        // 5. Submit and wait for execution
        let submit_start = Instant::now();
        let executed = match self.submit_with_retry(tx_bcs, signatures).await {
            Ok(executed) => executed,
            Err(e) => {
                self.failed_executions.fetch_add(1, Ordering::Relaxed);
                return Err(e);
            }
        };
        let submit_duration = submit_start.elapsed();

        // 6. Record digest to prevent duplicate execution
        {
            let mut seen = self.seen_digests.write().await;
            seen.insert(digest.clone());
        }

        // 7. Extract timing information
        let effects_time_ms = submit_duration.as_secs_f64() * 1000.0;

        // Record effects time for validator selection
        if let Some(endpoint) = self.validator_selector.select_best().await {
            self.validator_selector
                .record_effects_time(&endpoint, effects_time_ms)
                .await;
        }

        // Update statistics
        self.successful_executions.fetch_add(1, Ordering::Relaxed);
        self.total_effects_time_ms.fetch_add((effects_time_ms * 1000.0) as u64, Ordering::Relaxed);
        
        let checkpoint_time_ms = None; // TODO: Track checkpoint inclusion
        if let Some(checkpoint_ms) = checkpoint_time_ms {
            self.total_checkpoint_time_ms.fetch_add((checkpoint_ms * 1000.0) as u64, Ordering::Relaxed);
            self.checkpoint_count.fetch_add(1, Ordering::Relaxed);
        }

        info!(
            digest = %digest,
            effects_ms = effects_time_ms,
            uses_shared = plan.uses_shared_objects,
            sponsored = is_sponsored,
            "route executed successfully"
        );

        Ok(ExecutionResult {
            digest,
            executed,
            effects_time_ms,
            checkpoint_time_ms,
        })
    }

    /// Compile a route plan into a PTB (BCS TransactionData bytes)
    async fn compile_route(&self, plan: &RoutePlan) -> Result<Vec<u8>> {
        match &plan.route {
            crate::router::routes::Route::DeepBookSingle(req) => {
                let adapter = self
                    .deepbook
                    .as_ref()
                    .context("DeepBook adapter not available")?;
                adapter
                    .build_limit_order_ptb_bcs(req, false)
                    .await
                    .context("build DeepBook limit order PTB")
            }
            crate::router::routes::Route::MultiVenueSplit { .. } => {
                anyhow::bail!("multi-venue routes not yet implemented")
            }
            crate::router::routes::Route::CancelReplace { .. } => {
                anyhow::bail!("cancel-replace routes not yet implemented")
            }
            crate::router::routes::Route::FlashLoanArb { .. } => {
                anyhow::bail!("flash-loan routes not yet implemented")
            }
        }
    }

    /// Compile a route plan into a sponsored PTB
    /// Returns (tx_bcs, is_sponsored)
    async fn compile_route_sponsored(&self, plan: &RoutePlan) -> Result<(Vec<u8>, bool)> {
        let sponsorship = self
            .sponsorship
            .as_ref()
            .context("sponsorship not available")?;

        // Check if sponsorship is allowed
        let req = SponsorshipRequest {
            user_address: self.user_address,
            route_plan_id: format!("{:?}", plan.route),
            estimated_gas: plan.estimated_gas,
            created_at: Instant::now(),
        };

        if !sponsorship.can_sponsor(&req).await? {
            warn!("sponsorship not allowed, falling back to regular transaction");
            return Ok((self.compile_route(plan).await?, false));
        }

        // Build gasless transaction
        match &plan.route {
            crate::router::routes::Route::DeepBookSingle(req) => {
                let adapter = self
                    .deepbook
                    .as_ref()
                    .context("DeepBook adapter not available")?;

                // Build gasless PTB (programmable transaction)
                let (programmable, _sender) = adapter
                    .build_limit_order_ptb_gasless(req)
                    .await
                    .context("build gasless DeepBook limit order PTB")?;

                // Resolve sponsor gas coin ObjectRefs
                let gas_coin_ids = sponsorship.gas_coin_ids().await;
                if gas_coin_ids.is_empty() {
                    anyhow::bail!("no sponsor gas coins available");
                }
                let gas_object_refs = adapter
                    .object_refs_for_ids(&gas_coin_ids)
                    .await
                    .context("resolve sponsor gas object refs")?;

                // Build TransactionData with sponsor gas; do not sign yet
                let tx_bcs = sponsorship
                    .build_sponsored_transaction_data(
                        programmable,
                        self.user_address,
                        gas_object_refs,
                        plan.estimated_gas.max(10_000_000), // fallback minimum
                    )
                    .await
                    .context("build sponsored transaction data")?;

                Ok((tx_bcs, true))
            }
            _ => {
                anyhow::bail!("sponsored transactions not yet implemented for this route type")
            }
        }
    }

    /// Sign a sponsored transaction (user + sponsor signatures)
    async fn sign_sponsored_transaction(&self, tx_bcs: &[u8]) -> Result<Vec<Vec<u8>>> {
        let sponsorship = self
            .sponsorship
            .as_ref()
            .context("sponsorship not available")?;

        // User signs
        let (user_sig, _) = sign_tx_bcs_ed25519_to_serialized_signature(
            tx_bcs,
            &self.secret_key_hex,
        )
        .map_err(|e| AggrError::Signing(format!("user signing failed: {}", e)))?;

        // Sponsor signs
        let sponsor_sig = sponsorship.sign_sponsored_transaction(tx_bcs)?;

        Ok(vec![user_sig, sponsor_sig])
    }

    /// Submit transaction with idempotent retry logic
    async fn submit_with_retry(
        &self,
        tx_bcs: Vec<u8>,
        signatures: Vec<Vec<u8>>,
    ) -> Result<ExecutedTransaction> {
        let backoff = ExponentialBackoff {
            initial_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(5),
            max_elapsed_time: Some(Duration::from_secs(30)),
            multiplier: 2.0,
            ..Default::default()
        };

        let grpc_clone = self.grpc.clone();
        let jsonrpc_clone = self.jsonrpc.clone();
        let use_grpc = self.use_grpc_execute;
        
        retry(backoff, || {
            let tx_bcs = tx_bcs.clone();
            let signatures = signatures.clone();
            let grpc = grpc_clone.clone();
            let jsonrpc = jsonrpc_clone.clone();
            let use_grpc_exec = use_grpc;
            async move {
                let result = if use_grpc_exec {
                    Self::submit_grpc_internal(&grpc, &tx_bcs, &signatures).await
                } else {
                    Self::submit_jsonrpc_internal(&jsonrpc, &tx_bcs, &signatures).await
                };
                result.map_err(backoff::Error::transient)
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("submission failed after retries: {}", e))
    }

    /// Internal helper for gRPC submission (used by retry logic)
    async fn submit_grpc_internal(
        grpc: &Arc<tokio::sync::Mutex<GrpcClients>>,
        tx_bcs: &[u8],
        signatures: &[Vec<u8>],
    ) -> Result<ExecutedTransaction> {
        #[cfg(feature = "grpc-exec")]
        {
            use crate::transport::grpc::sui::rpc::v2::{Bcs, SignatureScheme, UserSignature};
            let mut grpc_guard = grpc.lock().await;
            
            // Convert all signatures to UserSignature format
            let user_signatures: Vec<UserSignature> = signatures
                .iter()
                .map(|sig_bytes| UserSignature {
                    bcs: Some(Bcs {
                        name: Some("sui.types.Signature".to_string()),
                        value: Some(sig_bytes.clone()),
                    }),
                    scheme: Some(SignatureScheme::Ed25519 as i32),
                    ..Default::default()
                })
                .collect();

            grpc_guard.execute_ptb(tx_bcs.to_vec(), user_signatures)
                .await
                .context("gRPC execute transaction")
        }

        #[cfg(not(feature = "grpc-exec"))]
        {
            let _ = (grpc, tx_bcs, signatures); // Suppress unused warnings when feature is disabled
            anyhow::bail!("gRPC execution not enabled (requires 'grpc-exec' feature)")
        }
    }

    /// Internal helper for JSON-RPC submission (used by retry logic)
    #[allow(unused_variables)]
    async fn submit_jsonrpc_internal(
        jsonrpc: &Arc<JsonRpc>,
        tx_bcs: &[u8],
        signatures: &[Vec<u8>],
    ) -> Result<ExecutedTransaction> {
        use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine as _};
        
        // Convert all signatures to base64
        let sigs_b64: Vec<String> = signatures
            .iter()
            .map(|sig_bytes| B64.encode(sig_bytes))
            .collect();

        let _resp = jsonrpc
            .execute_tx_block(tx_bcs, &sigs_b64)
            .await
            .map_err(|e| AggrError::Transport(e.to_string()))?;

        // JSON-RPC execution is supported but ExecutedTransaction conversion
        // requires parsing the full JSON response structure.
        // For now, return an error indicating gRPC should be used for full functionality.
        // In production, implement full JSON-RPC response parsing.
        anyhow::bail!(
            "JSON-RPC execution succeeded but ExecutedTransaction conversion not fully implemented. \
             Use gRPC execution (--features grpc-exec) for full functionality. Digest: {:?}",
            _resp.digest
        );
    }


    /// Compute transaction digest from BCS bytes
    fn compute_digest(&self, tx_bcs: &[u8]) -> Result<String> {
        use blake2::{Blake2b512, Digest};
        let mut hasher = Blake2b512::new();
        hasher.update(tx_bcs);
        let hash = hasher.finalize();
        Ok(hex::encode(&hash[..32]))
    }
}

