//! What every weebo operator binary and its controllers need around the
//! business logic: telemetry (logs, traces, OTLP and Prometheus metrics),
//! leader election, graceful shutdown, health probes, the TLS server the
//! admission webhook runs on, reconcile metrics, the condition list a
//! status carries and the Events announcing every error — and the
//! skeleton every controller shares (triggers, concurrency, requeue,
//! finalizer, status publishing, reflector stores, watches shared by every
//! controller) and the `/validate`
//! endpoint.

pub mod admission;
pub mod conditions;
pub mod controller;
pub mod events;
pub mod health;
pub mod leader;
pub mod metrics;
pub mod pod;
pub mod runtime;
pub mod scope;
pub mod shared;
pub mod shutdown;
pub mod status;
pub mod store;
pub mod telemetry;
pub mod webhook_server;

pub use health::Health;
pub use scope::{ScopeError, WatchScope};
pub use shutdown::Shutdown;

/// The docs package quotes these values on every operator's site (its
/// metrics, Events and health pages) from a copy: it must not drift.
#[cfg(test)]
mod published_in_the_docs {
    use super::*;

    #[test]
    fn the_docs_package_quotes_the_runtime_as_it_is() {
        let published: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/operator-docs/src/reference/kit-runtime.json"
        ))
        .unwrap();
        let actual = serde_json::json!({
            "reconcile_duration_buckets_secs": metrics::DURATION_BUCKETS,
            "default_concurrency": controller::DEFAULT_CONCURRENCY,
            "default_worker_threads": runtime::DEFAULT_WORKER_THREADS,
            "steady_requeue_secs": controller::STEADY_REQUEUE_SECS,
            "retry_requeue_secs": controller::RETRY_REQUEUE_SECS,
            "status_retries": status::STATUS_RETRIES,
            "event_note_max_chars": events::MAX_NOTE_BYTES,
            "liveness_threshold_secs": health::DEFAULT_LIVENESS_THRESHOLD.as_secs(),
            "failure_streak_gap_secs": health::FAILURE_STREAK_GAP.as_secs(),
            "lease_ttl_secs": leader::LEASE_TTL.as_secs(),
            "lease_renew_secs": leader::LEASE_RENEW_INTERVAL.as_secs(),
            "shutdown_drain_secs": webhook_server::DEFAULT_DRAIN.as_secs(),
            "graceful_shutdown_secs": webhook_server::GRACEFUL_SHUTDOWN.as_secs(),
            "tls_reload_secs": webhook_server::TLS_RELOAD_INTERVAL.as_secs(),
            "webhook_port": webhook_server::DEFAULT_PORT,
        });
        assert_eq!(
            actual, published,
            "update packages/operator-docs/src/reference/kit-runtime.json"
        );
    }
}
