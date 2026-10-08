//! Reconcile-loop metrics. Recording through `opentelemetry::global` is a
//! no-op until [`crate::telemetry::init`] installs a real `MeterProvider`
//! (only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set), so controllers record
//! unconditionally.

use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use weebo_kit_domain::{Reason, Severity};

/// `<prefix>_reconcile_total` and `<prefix>_reconcile_duration_seconds`.
/// Build once per process (e.g. in a `OnceLock`).
pub struct ReconcileMetrics {
    total: Counter<u64>,
    duration: Histogram<f64>,
}

impl ReconcileMetrics {
    /// `meter`: the operator's meter name (`weebo-forgejo-operator`);
    /// `prefix`: its metric prefix (`weebo_forgejo`).
    pub fn new(meter: &'static str, prefix: &str) -> Self {
        let meter = opentelemetry::global::meter(meter);
        Self {
            total: meter
                .u64_counter(format!("{prefix}_reconcile_total"))
                .with_description("Reconcile runs, by kind, result and reason")
                .build(),
            duration: meter
                .f64_histogram(format!("{prefix}_reconcile_duration_seconds"))
                .with_description("Reconcile run duration")
                .with_unit("s")
                .build(),
        }
    }

    /// Records one reconcile run. `reason` is the `Ready` condition's reason
    /// the run ended on, so the label set stays bounded by the catalog.
    pub fn record<R: Reason>(&self, kind: &'static str, reason: R, elapsed: Duration) {
        let result = match reason.severity() {
            Severity::Advisory => "success",
            Severity::Blocking => "error",
        };
        let attrs = [
            KeyValue::new("kind", kind),
            KeyValue::new("result", result),
            KeyValue::new("reason", reason.as_str()),
        ];
        self.total.add(1, &attrs);
        self.duration.record(elapsed.as_secs_f64(), &attrs[..1]);
    }
}
