// Metrics and observability module
// This file handles collection and reporting of performance metrics,
// statistics, and monitoring data for the aggregator
//
// Numan Thabit 2025 Nov

use once_cell::sync::Lazy;
use prometheus::{register_counter_vec, register_histogram_vec, CounterVec, HistogramVec};

pub static REQ_LATENCY: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "aggr_request_latency_seconds",
        "latency for upstream calls",
        &["service", "method"]
    )
    .unwrap()
});

pub static REQ_ERRORS: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "aggr_request_errors_total",
        "errors by upstream",
        &["service", "method"]
    )
    .unwrap()
});
