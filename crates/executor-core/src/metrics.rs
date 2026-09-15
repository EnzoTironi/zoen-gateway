//! Process metrics. Implementations must be wait-free on the hot path.

use std::sync::atomic::{AtomicU64, Ordering};

/// Names for the built-in counters. Stable strings for exporters.
pub mod names {
    /// Successful executes.
    pub const EXECUTE_OK: &str = "executor.execute.ok";
    /// Failed executes (domain or infra).
    pub const EXECUTE_ERR: &str = "executor.execute.err";
    /// Overload rejections.
    pub const EXECUTE_OVERLOAD: &str = "executor.execute.overload";
    /// Timeouts.
    pub const EXECUTE_TIMEOUT: &str = "executor.execute.timeout";
    /// Cancellations.
    pub const EXECUTE_CANCEL: &str = "executor.execute.cancel";
    /// Current in-flight executes (gauge).
    pub const IN_FLIGHT: &str = "executor.execute.in_flight";
}

/// Wait-free counters an exporter can scrape.
pub trait Metrics: Send + Sync {
    /// Add `delta` to a named counter.
    fn counter(&self, name: &'static str, delta: u64);
    /// Set a named gauge.
    fn gauge(&self, name: &'static str, value: i64);
    /// Record a latency sample in milliseconds.
    fn observe_ms(&self, name: &'static str, ms: u64);
}

/// No-op sink. Default so a library user is not forced to pick an exporter.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopMetrics;

impl Metrics for NoopMetrics {
    fn counter(&self, _name: &'static str, _delta: u64) {}
    fn gauge(&self, _name: &'static str, _value: i64) {}
    fn observe_ms(&self, _name: &'static str, _ms: u64) {}
}

/// Atomic counters suitable for a single process (and tests).
#[derive(Debug, Default)]
pub struct AtomicMetrics {
    execute_ok: AtomicU64,
    execute_err: AtomicU64,
    execute_overload: AtomicU64,
    execute_timeout: AtomicU64,
    execute_cancel: AtomicU64,
    in_flight: AtomicU64,
    last_execute_ms: AtomicU64,
}

impl AtomicMetrics {
    /// Empty counters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot for tests / `/metrics` JSON.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            execute_ok: self.execute_ok.load(Ordering::Relaxed),
            execute_err: self.execute_err.load(Ordering::Relaxed),
            execute_overload: self.execute_overload.load(Ordering::Relaxed),
            execute_timeout: self.execute_timeout.load(Ordering::Relaxed),
            execute_cancel: self.execute_cancel.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
            last_execute_ms: self.last_execute_ms.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time view of [`AtomicMetrics`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricsSnapshot {
    /// `execute` completions that returned [`crate::Outcome::Completed`].
    pub execute_ok: u64,
    /// `execute` that failed (not overload/timeout/cancel).
    pub execute_err: u64,
    /// Overload rejections.
    pub execute_overload: u64,
    /// Timeouts.
    pub execute_timeout: u64,
    /// Cancellations.
    pub execute_cancel: u64,
    /// In-flight now.
    pub in_flight: u64,
    /// Last execute latency in ms.
    pub last_execute_ms: u64,
}

impl Metrics for AtomicMetrics {
    fn counter(&self, name: &'static str, delta: u64) {
        let slot = match name {
            names::EXECUTE_OK => &self.execute_ok,
            names::EXECUTE_ERR => &self.execute_err,
            names::EXECUTE_OVERLOAD => &self.execute_overload,
            names::EXECUTE_TIMEOUT => &self.execute_timeout,
            names::EXECUTE_CANCEL => &self.execute_cancel,
            _ => return,
        };
        slot.fetch_add(delta, Ordering::Relaxed);
    }

    fn gauge(&self, name: &'static str, value: i64) {
        if name == names::IN_FLIGHT {
            self.in_flight
                .store(u64::try_from(value.max(0)).unwrap_or(0), Ordering::Relaxed);
        }
    }

    fn observe_ms(&self, name: &'static str, ms: u64) {
        if name == "executor.execute.ms" {
            self.last_execute_ms.store(ms, Ordering::Relaxed);
        }
    }
}
