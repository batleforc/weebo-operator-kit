//! The HTTPS server an admission webhook is served on, on every replica
//! (not only the leader): with `failurePolicy: Fail`, a leader-only webhook
//! would turn every failover into an outage for the governed kinds.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::task::JoinHandle;

use crate::Shutdown;
use crate::health::Health;

/// cert-manager rotates the webhook certificate in place; re-reading it on
/// a fixed interval picks rotations up without a restart.
pub const TLS_RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// Default of `WEEBO_SHUTDOWN_DRAIN_SECONDS`: how long the server keeps
/// answering after SIGTERM, while the apiserver still routes admission
/// requests to this pod (endpoint removal lags the termination).
pub const DEFAULT_DRAIN: Duration = Duration::from_secs(5);

/// In-flight requests get this long to finish once draining is over.
pub const GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(10);

/// The running server; [`WebhookServer::stopped`] resolves once it has
/// drained after shutdown (immediately when nothing is served).
pub struct WebhookServer(Option<JoinHandle<()>>);

impl WebhookServer {
    pub async fn stopped(self) {
        if let Some(task) = self.0 {
            let _ = task.await;
        }
    }
}

fn drain() -> Duration {
    std::env::var("WEEBO_SHUTDOWN_DRAIN_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(DEFAULT_DRAIN, Duration::from_secs)
}

/// Serves `router` over TLS when the certificate is mounted
/// (`WEBHOOK_TLS_CERT_FILE`/`WEBHOOK_TLS_KEY_FILE`, default
/// `/etc/webhook/certs/tls.{crt,key}`) on `WEBHOOK_PORT` (default 8443),
/// and marks `health` ready once listening. Without a certificate — local
/// runs — nothing is served (reconcilers re-check the allow-list on their
/// own anyway) and `health` is ready at once.
///
/// On `shutdown`, `health` turns not-ready, the server keeps answering for
/// `WEEBO_SHUTDOWN_DRAIN_SECONDS` (default 5), then finishes in-flight
/// requests. The process exits if the server fails.
pub async fn spawn(
    router: axum::Router,
    health: Arc<Health>,
    shutdown: Shutdown,
) -> anyhow::Result<WebhookServer> {
    let cert = std::env::var("WEBHOOK_TLS_CERT_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.crt".to_string());
    let key = std::env::var("WEBHOOK_TLS_KEY_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.key".to_string());
    if !Path::new(&cert).exists() {
        tracing::warn!(%cert, "no webhook certificate, admission webhook disabled");
        health.set_ready(true);
        tokio::spawn({
            let health = health.clone();
            let shutdown = shutdown.clone();
            async move {
                shutdown.wait().await;
                health.set_ready(false);
            }
        });
        return Ok(WebhookServer(None));
    }
    let port: u16 = std::env::var("WEBHOOK_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8443);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let tls = RustlsConfig::from_pem_file(&cert, &key).await?;

    tokio::spawn({
        let tls = tls.clone();
        async move {
            let mut interval = tokio::time::interval(TLS_RELOAD_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = tls.reload_from_pem_file(&cert, &key).await {
                    tracing::error!(error = %err, "webhook certificate reload failed, keeping the previous one");
                }
            }
        }
    });

    let handle = Handle::new();
    tokio::spawn({
        let handle = handle.clone();
        let health = health.clone();
        async move {
            shutdown.wait().await;
            health.set_ready(false);
            let drain = drain();
            tracing::info!(seconds = drain.as_secs(), "draining the admission webhook");
            tokio::time::sleep(drain).await;
            handle.graceful_shutdown(Some(GRACEFUL_SHUTDOWN));
        }
    });

    let server = axum_server::bind_rustls(addr, tls).handle(handle.clone());
    let task = tokio::spawn(async move {
        if let Err(err) = server.serve(router.into_make_service()).await {
            tracing::error!(error = %err, "admission webhook stopped");
            std::process::exit(1);
        }
        tracing::info!("admission webhook stopped");
    });
    if handle.listening().await.is_some() {
        tracing::info!(%addr, "admission webhook listening");
        health.set_ready(true);
    }
    Ok(WebhookServer(Some(task)))
}
