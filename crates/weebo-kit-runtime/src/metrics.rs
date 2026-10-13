//! Reconcile-loop metrics. Recording through `opentelemetry::global` is a
//! no-op until [`crate::telemetry::init`] installs a real `MeterProvider`
//! (when `OTEL_EXPORTER_OTLP_ENDPOINT` or `WEEBO_METRICS_ADDR` is set), so
//! controllers record unconditionally.

use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use weebo_kit_domain::{Reason, Severity};

/// Upper bounds of the reconcile-duration buckets, seconds: a reconcile
/// takes milliseconds to seconds — OTel's default boundaries (0, 5, 10,
/// 25… meant for milliseconds) would put every one of them in the first
/// bucket and make any quantile read ~5 s.
pub const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// `<prefix>_reconcile_total` and `<prefix>_reconcile_duration_seconds`, as
/// Prometheus names them: the instruments are `<prefix>_reconcile` and
/// `<prefix>_reconcile_duration` (unit `s`), and the Prometheus exporter
/// adds `_total` to a counter and the unit to a histogram itself — naming
/// them with the suffixes exported `…_total_total` and
/// `…_seconds_seconds`. Build once per process (e.g. in a `OnceLock`).
pub struct ReconcileMetrics {
    total: Counter<u64>,
    duration: Histogram<f64>,
}

impl ReconcileMetrics {
    /// `meter`: the operator's meter name (`weebo-example-operator`);
    /// `prefix`: its metric prefix (`weebo_example`).
    pub fn new(meter: &'static str, prefix: &str) -> Self {
        Self::with_meter(&opentelemetry::global::meter(meter), prefix)
    }

    /// On an explicit `meter` rather than the global provider's.
    pub fn with_meter(meter: &opentelemetry::metrics::Meter, prefix: &str) -> Self {
        Self {
            total: meter
                .u64_counter(format!("{prefix}_reconcile"))
                .with_description("Reconcile runs, by kind, result and reason")
                .build(),
            duration: meter
                .f64_histogram(format!("{prefix}_reconcile_duration"))
                .with_description("Reconcile run duration")
                .with_unit("s")
                .with_boundaries(DURATION_BUCKETS.to_vec())
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

#[cfg(test)]
mod tests {
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use prometheus::{Encoder, TextEncoder};
    use weebo_kit_domain::Reason;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Ok;

    impl Reason for Ok {
        const SUCCESS: Self = Ok;
        fn as_str(self) -> &'static str {
            "Reconciled"
        }
        fn severity(self) -> Severity {
            Severity::Advisory
        }
    }

    #[test]
    fn prometheus_names_carry_each_suffix_once() {
        let registry = prometheus::Registry::new();
        let provider = SdkMeterProvider::builder()
            .with_reader(
                opentelemetry_prometheus::exporter()
                    .with_registry(registry.clone())
                    .without_target_info()
                    .build()
                    .unwrap(),
            )
            .build();
        let metrics = ReconcileMetrics::with_meter(&provider.meter("test"), "weebo_test");
        metrics.record("Thing", Ok, Duration::from_millis(20));

        let mut out = Vec::new();
        TextEncoder::new()
            .encode(&registry.gather(), &mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("weebo_test_reconcile_total{"), "{text}");
        assert!(
            text.contains("weebo_test_reconcile_duration_seconds_bucket{"),
            "{text}"
        );
        assert!(!text.contains("_total_total") && !text.contains("_seconds_seconds"));
        // A 20 ms reconcile lands in a bucket that tells it apart.
        assert!(text.contains("le=\"0.025\"} 1"), "{text}");
        assert!(text.contains("le=\"0.01\"} 0"), "{text}");
    }
}
