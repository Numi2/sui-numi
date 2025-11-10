// Validator selector with EWMA-based latency tracking
// This file implements validator selection based on rolling effects-latency telemetry
// to minimize end-to-end confirmation time
//
// Numan Thabit 2025 Nov

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Validator endpoint identifier
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ValidatorId {
    pub endpoint: String,
}

/// Latency statistics for a validator
#[derive(Debug, Clone)]
struct ValidatorStats {
    /// EWMA of effects time (time from submission to effects observed)
    pub effects_ewma_ms: f64,
    /// Number of observations
    pub observations: u64,
    /// Last update timestamp
    pub last_update: Instant,
    /// Whether validator is considered healthy
    pub healthy: bool,
}

impl ValidatorStats {
    fn new() -> Self {
        Self {
            effects_ewma_ms: 500.0, // Initial estimate: 500ms
            observations: 0,
            last_update: Instant::now(),
            healthy: true,
        }
    }

    /// Update EWMA with new observation
    /// alpha controls the smoothing factor (0.0 to 1.0)
    fn update_ewma(&mut self, observed_ms: f64, alpha: f64) {
        if self.observations == 0 {
            self.effects_ewma_ms = observed_ms;
        } else {
            self.effects_ewma_ms = alpha * observed_ms + (1.0 - alpha) * self.effects_ewma_ms;
        }
        self.observations += 1;
        self.last_update = Instant::now();
    }
}

/// Validator selector that tracks latency and selects optimal validators
pub struct ValidatorSelector {
    validators: Arc<RwLock<HashMap<ValidatorId, ValidatorStats>>>,
    /// EWMA smoothing factor (typically 0.1-0.3)
    alpha: f64,
    /// Maximum age before stats are considered stale (seconds)
    max_staleness_secs: u64,
    /// Minimum observations before validator is considered reliable
    min_observations: u64,
}

impl ValidatorSelector {
    pub fn new(alpha: f64, max_staleness_secs: u64, min_observations: u64) -> Self {
        Self {
            validators: Arc::new(RwLock::new(HashMap::new())),
            alpha,
            max_staleness_secs,
            min_observations,
        }
    }

    /// Register a validator endpoint
    pub async fn register(&self, endpoint: String) {
        let id = ValidatorId { endpoint };
        let mut validators = self.validators.write().await;
        validators.entry(id).or_insert_with(ValidatorStats::new);
    }

    /// Record an effects time observation for a validator
    pub async fn record_effects_time(&self, endpoint: &str, effects_time_ms: f64) {
        let id = ValidatorId {
            endpoint: endpoint.to_string(),
        };
        let mut validators = self.validators.write().await;
        if let Some(stats) = validators.get_mut(&id) {
            stats.update_ewma(effects_time_ms, self.alpha);
            debug!(
                endpoint = %endpoint,
                effects_ms = effects_time_ms,
                ewma_ms = stats.effects_ewma_ms,
                observations = stats.observations,
                "updated validator effects time"
            );
        } else {
            warn!(
                endpoint = %endpoint,
                "recorded effects time for unregistered validator"
            );
        }
    }

    /// Mark a validator as unhealthy (e.g., after repeated failures)
    pub async fn mark_unhealthy(&self, endpoint: &str) {
        let id = ValidatorId {
            endpoint: endpoint.to_string(),
        };
        let mut validators = self.validators.write().await;
        if let Some(stats) = validators.get_mut(&id) {
            stats.healthy = false;
            warn!(endpoint = %endpoint, "marked validator as unhealthy");
        }
    }

    /// Mark a validator as healthy
    pub async fn mark_healthy(&self, endpoint: &str) {
        let id = ValidatorId {
            endpoint: endpoint.to_string(),
        };
        let mut validators = self.validators.write().await;
        if let Some(stats) = validators.get_mut(&id) {
            stats.healthy = true;
        }
    }

    /// Select the best validator based on EWMA latency
    pub async fn select_best(&self) -> Option<String> {
        let validators = self.validators.read().await;
        let now = Instant::now();

        let mut candidates: Vec<_> = validators
            .iter()
            .filter(|(_, stats)| {
                stats.healthy
                    && stats.observations >= self.min_observations
                    && now.duration_since(stats.last_update).as_secs() < self.max_staleness_secs
            })
            .collect();

        if candidates.is_empty() {
            // Fallback: return any validator, even if stats are incomplete
            return validators
                .iter()
                .filter(|(_, stats)| stats.healthy)
                .min_by(|(_, a), (_, b)| {
                    a.effects_ewma_ms
                        .partial_cmp(&b.effects_ewma_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(id, _)| id.endpoint.clone());
        }

        candidates.sort_by(|(_, a), (_, b)| {
            a.effects_ewma_ms
                .partial_cmp(&b.effects_ewma_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.first().map(|(id, stats)| {
            debug!(
                endpoint = %id.endpoint,
                ewma_ms = stats.effects_ewma_ms,
                observations = stats.observations,
                "selected best validator"
            );
            id.endpoint.clone()
        })
    }

    /// Get current statistics for all validators
    pub async fn stats(&self) -> HashMap<String, (f64, u64, bool)> {
        let validators = self.validators.read().await;
        validators
            .iter()
            .map(|(id, stats)| {
                (
                    id.endpoint.clone(),
                    (stats.effects_ewma_ms, stats.observations, stats.healthy),
                )
            })
            .collect()
    }
}

impl Default for ValidatorSelector {
    fn default() -> Self {
        Self::new(0.2, 300, 5) // alpha=0.2, 5min staleness, 5 min observations
    }
}
