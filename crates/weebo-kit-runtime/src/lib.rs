//! What every weebo operator binary and its controllers need around the
//! business logic: telemetry (logs, traces, OTLP and Prometheus metrics),
//! leader election, graceful shutdown, health probes, the TLS server the
//! admission webhook runs on, reconcile metrics, the condition list a
//! status carries and the Events announcing every error — and the
//! skeleton every controller shares (triggers, concurrency, requeue,
//! finalizer, status publishing) and the `/validate` endpoint.

pub mod admission;
pub mod conditions;
pub mod controller;
pub mod events;
pub mod health;
pub mod leader;
pub mod metrics;
pub mod shutdown;
pub mod status;
pub mod telemetry;
pub mod webhook_server;

pub use health::Health;
pub use shutdown::Shutdown;
