// Error types and error handling module
// This file defines custom error types and error conversion logic
// for the ultra-aggr project
//
// Numan Thabit 2025 Nov

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AggrError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("signing error: {0}")]
    Signing(String),
    #[error("build tx error: {0}")]
    BuildTx(String),
    #[error("backoff exhausted")]
    BackoffExhausted,
}
