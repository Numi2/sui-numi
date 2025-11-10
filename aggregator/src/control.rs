// Control plane: admission control and circuit breakers
//
// Provides simple concurrency limiting, rate limiting, and per-route-class
// circuit breakers with sliding-window failure tracking.
//
// Numan Thabit 2025 Nov

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tracing::debug;

#[derive(Clone)]
pub struct AdmissionControl {
    max_inflight: Arc<Semaphore>,
    // Simple rate limiter: allow up to rate_per_sec within a 1s sliding window
    inner: Arc<Mutex<RateLimiter>>,
}

struct RateLimiter {
    rate_per_sec: u32,
    timestamps: VecDeque<Instant>,
    window: Duration,
}

impl AdmissionControl {
    pub fn new(max_inflight: usize, rate_per_sec: Option<u32>) -> Self {
        let rl = RateLimiter {
            rate_per_sec: rate_per_sec.unwrap_or(200),
            timestamps: VecDeque::with_capacity(256),
            window: Duration::from_secs(1),
        };
        Self {
            max_inflight: Arc::new(Semaphore::new(max_inflight)),
            inner: Arc::new(Mutex::new(rl)),
        }
    }

    /// Acquire an admission permit respecting max inflight and rate limit.
    pub async fn acquire(&self) -> AdmissionPermit {
        // Rate limit loop
        loop {
            let mut guard = self.inner.lock().await;
            let now = Instant::now();
            while let Some(front) = guard.timestamps.front() {
                if now.duration_since(*front) > guard.window {
                    guard.timestamps.pop_front();
                } else {
                    break;
                }
            }
            if (guard.timestamps.len() as u32) < guard.rate_per_sec {
                guard.timestamps.push_back(now);
                break;
            }
            drop(guard);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let permit = self
            .max_inflight
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed");
        AdmissionPermit { _permit: permit }
    }
}

pub struct AdmissionPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[derive(Clone)]
pub struct CircuitBreakers {
    inner: Arc<Mutex<HashMap<String, Breaker>>>,
}

#[derive(Clone)]
struct Breaker {
    window: VecDeque<bool>, // true=failure, false=success
    max_window: usize,
    threshold: f32,
    min_samples: usize,
    open_until: Option<Instant>,
    open_cooldown: Duration,
}

impl Default for CircuitBreakers {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl CircuitBreakers {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_open(&self, class: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let b = inner
            .entry(class.to_string())
            .or_insert_with(Breaker::default);
        if let Some(until) = b.open_until {
            if Instant::now() < until {
                return true;
            } else {
                b.open_until = None;
            }
        }
        false
    }

    pub async fn record_success(&self, class: &str) {
        self.record(class, false).await;
    }

    pub async fn record_failure(&self, class: &str) {
        self.record(class, true).await;
    }

    async fn record(&self, class: &str, failure: bool) {
        let mut inner = self.inner.lock().await;
        let b = inner
            .entry(class.to_string())
            .or_insert_with(Breaker::default);
        if b.window.len() == b.max_window {
            b.window.pop_front();
        }
        b.window.push_back(failure);

        let samples = b.window.len();
        if samples >= b.min_samples {
            let fails = b.window.iter().filter(|x| **x).count();
            let rate = fails as f32 / samples as f32;
            if rate >= b.threshold && b.open_until.is_none() {
                b.open_until = Some(Instant::now() + b.open_cooldown);
                debug!(class = %class, rate = rate, samples = samples, "circuit opened");
            }
        }
    }
}

impl Default for Breaker {
    fn default() -> Self {
        Self {
            window: VecDeque::with_capacity(100),
            max_window: 100,
            threshold: 0.5,
            min_samples: 20,
            open_until: None,
            open_cooldown: Duration::from_secs(5),
        }
    }
}
