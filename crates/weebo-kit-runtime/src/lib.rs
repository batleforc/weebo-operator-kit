//! What every weebo operator binary and its controllers need around the
//! business logic: telemetry, leader election, graceful shutdown, the TLS
//! server the admission webhook runs on, reconcile metrics and the
//! condition list a status carries.

pub mod conditions;
pub mod leader;
pub mod metrics;
pub mod shutdown;
pub mod telemetry;
pub mod webhook_server;

pub use shutdown::Shutdown;
