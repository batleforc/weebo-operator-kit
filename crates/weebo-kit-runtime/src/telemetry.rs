//! Logging, traces and metrics.

use std::net::SocketAddr;

use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::routing::get;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use prometheus::{Encoder, TextEncoder};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

/// The installed providers, flushed by [`Telemetry::shutdown`].
#[derive(Default)]
pub struct Telemetry {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    prometheus: Option<(SocketAddr, prometheus::Registry)>,
}

/// Always initializes plain `tracing` (JSON lines when `LOG_FORMAT=json`).
/// Metrics go to a global `MeterProvider` with:
///
/// - an OTLP trace exporter and periodic OTLP metrics exporter when
///   `OTEL_EXPORTER_OTLP_ENDPOINT` is set;
/// - a Prometheus registry when `WEEBO_METRICS_ADDR` is set (e.g.
///   `0.0.0.0:9090`), served by [`Telemetry::serve_metrics`].
///
/// Every `opentelemetry::global::meter(...)` call is safe to make
/// unconditionally — the global API is a no-op recorder until a real
/// `MeterProvider` is installed. `service` names the tracer (e.g.
/// `weebo-forgejo-operator`).
pub fn init(service: &'static str) -> anyhow::Result<Telemetry> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = std::env::var("LOG_FORMAT").is_ok_and(|f| f.eq_ignore_ascii_case("json"));
    let otlp = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();
    let mut telemetry = Telemetry::default();

    let otel_layer = if otlp {
        let span_exporter = SpanExporter::builder().with_tonic().build()?;
        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(span_exporter)
            .build();
        let tracer = opentelemetry::trace::TracerProvider::tracer(&tracer_provider, service);
        telemetry.tracer_provider = Some(tracer_provider);
        Some(tracing_opentelemetry::layer().with_tracer(tracer))
    } else {
        None
    };
    let (fmt_json, fmt_text) = if json {
        (Some(tracing_subscriber::fmt::layer().json()), None)
    } else {
        (None, Some(tracing_subscriber::fmt::layer()))
    };
    Registry::default()
        .with(env_filter)
        .with(fmt_json)
        .with(fmt_text)
        .with(otel_layer)
        .try_init()?;

    let metrics_addr = match std::env::var("WEEBO_METRICS_ADDR") {
        Ok(addr) if !addr.is_empty() => Some(
            addr.parse::<SocketAddr>()
                .map_err(|e| anyhow::anyhow!("WEEBO_METRICS_ADDR {addr:?}: {e}"))?,
        ),
        _ => None,
    };
    if otlp || metrics_addr.is_some() {
        let mut builder = SdkMeterProvider::builder();
        if otlp {
            builder =
                builder.with_periodic_exporter(MetricExporter::builder().with_tonic().build()?);
        }
        if let Some(addr) = metrics_addr {
            let registry = prometheus::Registry::new();
            builder = builder.with_reader(
                opentelemetry_prometheus::exporter()
                    .with_registry(registry.clone())
                    .without_target_info()
                    .build()?,
            );
            telemetry.prometheus = Some((addr, registry));
        }
        let meter_provider = builder.build();
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        telemetry.meter_provider = Some(meter_provider);
    }
    Ok(telemetry)
}

impl Telemetry {
    /// Serves `GET /metrics` (Prometheus text format, plain HTTP) on
    /// `WEEBO_METRICS_ADDR` when it is set; on every replica, so a standby
    /// shows up too (with its leader gauge at 0). The process exits if the
    /// server stops.
    pub async fn serve_metrics(&self) -> anyhow::Result<()> {
        let Some((addr, registry)) = self.prometheus.clone() else {
            return Ok(());
        };
        let router = Router::new().route(
            "/metrics",
            get(move || {
                let registry = registry.clone();
                async move {
                    let mut body = Vec::new();
                    let encoder = TextEncoder::new();
                    match encoder.encode(&registry.gather(), &mut body) {
                        Ok(()) => ([(CONTENT_TYPE, encoder.format_type().to_string())], body),
                        Err(err) => {
                            tracing::warn!(error = %err, "encoding metrics failed");
                            ([(CONTENT_TYPE, "text/plain".to_string())], Vec::new())
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(%addr, "metrics listening");
        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                tracing::error!(error = %err, "metrics server stopped");
                std::process::exit(1);
            }
        });
        Ok(())
    }

    /// Flushes the pending spans and metrics; call before exiting.
    pub fn shutdown(self) {
        if let Some(provider) = self.tracer_provider
            && let Err(err) = provider.shutdown()
        {
            eprintln!("flushing traces failed: {err}");
        }
        if let Some(provider) = self.meter_provider
            && let Err(err) = provider.shutdown()
        {
            eprintln!("flushing metrics failed: {err}");
        }
    }
}
