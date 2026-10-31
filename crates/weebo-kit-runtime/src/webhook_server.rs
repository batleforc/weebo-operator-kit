//! The HTTPS server an admission webhook is served on, on every replica
//! (not only the leader): with `failurePolicy: Fail`, a leader-only webhook
//! would turn every failover into an outage for the governed kinds.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

/// Default of `WEBHOOK_PORT`: the HTTPS port `/validate` and the probes are
/// served on.
pub const DEFAULT_PORT: u16 = 8443;

/// In-flight requests get this long to finish once draining is over.
pub const GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(10);

/// A request still running after this gets a 503: the apiserver waits 30 s
/// at most (`timeoutSeconds`), nobody reads a later answer.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn bounded(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!(timeout = ?REQUEST_TIMEOUT, "admission request timed out");
            axum::response::IntoResponse::into_response(axum::http::StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// The running server; [`WebhookServer::stopped`] resolves once it has
/// drained after shutdown (immediately when nothing is served).
pub struct WebhookServer(Option<JoinHandle<()>>);

impl WebhookServer {
    pub async fn stopped(self) {
        if let Some(task) = self.0
            && let Err(e) = task.await
        {
            tracing::error!(error = %e, "the webhook server task panicked");
        }
    }
}

fn drain_from_env() -> Duration {
    let Ok(raw) = std::env::var("WEEBO_SHUTDOWN_DRAIN_SECONDS") else {
        return DEFAULT_DRAIN;
    };
    raw.parse().map_or_else(
        |e| {
            tracing::warn!(value = %raw, error = %e, "invalid WEEBO_SHUTDOWN_DRAIN_SECONDS, using the default");
            DEFAULT_DRAIN
        },
        Duration::from_secs,
    )
}

/// What the server does when its certificate isn't mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingCertificate {
    /// Serve nothing and report ready: a local run, where reconcilers
    /// re-check what admission would have (what [`spawn`] does).
    Disable,
    /// Fail the start: a webhook whose configuration routes the apiserver
    /// to it must not come up ready without serving.
    Fail,
}

/// Where and how [`spawn_with`] serves. [`WebhookServerConfig::from_env`]
/// is what [`spawn`] reads; an operator taking these from its own command
/// line builds the struct instead.
#[derive(Debug, Clone)]
pub struct WebhookServerConfig {
    pub addr: SocketAddr,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    /// How long the server keeps answering after shutdown starts.
    pub drain: Duration,
    pub missing_certificate: MissingCertificate,
}

impl WebhookServerConfig {
    /// `WEBHOOK_PORT` (default 8443) on every interface,
    /// `WEBHOOK_TLS_CERT_FILE`/`WEBHOOK_TLS_KEY_FILE` (default
    /// `/etc/webhook/certs/tls.{crt,key}`), `WEEBO_SHUTDOWN_DRAIN_SECONDS`
    /// (default 5), and [`MissingCertificate::Disable`].
    pub fn from_env() -> Self {
        let port: u16 = std::env::var("WEBHOOK_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Self {
            addr: SocketAddr::from(([0, 0, 0, 0], port)),
            cert_file: std::env::var("WEBHOOK_TLS_CERT_FILE")
                .unwrap_or_else(|_| "/etc/webhook/certs/tls.crt".to_string())
                .into(),
            key_file: std::env::var("WEBHOOK_TLS_KEY_FILE")
                .unwrap_or_else(|_| "/etc/webhook/certs/tls.key".to_string())
                .into(),
            drain: drain_from_env(),
            missing_certificate: MissingCertificate::Disable,
        }
    }
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
///
/// [`spawn_with`] fed with [`WebhookServerConfig::from_env`].
pub async fn spawn(
    router: axum::Router,
    health: Arc<Health>,
    shutdown: Shutdown,
) -> std::io::Result<WebhookServer> {
    spawn_with(WebhookServerConfig::from_env(), router, health, shutdown).await
}

/// [`spawn`], served where `config` says. The certificate is re-read
/// every [`TLS_RELOAD_INTERVAL`]; with [`MissingCertificate::Fail`], a
/// certificate that isn't there is a `NotFound` error and nothing starts.
pub async fn spawn_with(
    config: WebhookServerConfig,
    router: axum::Router,
    health: Arc<Health>,
    shutdown: Shutdown,
) -> std::io::Result<WebhookServer> {
    let WebhookServerConfig {
        addr,
        cert_file: cert,
        key_file: key,
        drain,
        missing_certificate,
    } = config;
    if !Path::new(&cert).exists() {
        if missing_certificate == MissingCertificate::Fail {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("webhook certificate {} not found", cert.display()),
            ));
        }
        tracing::warn!(cert = %cert.display(), "no webhook certificate, admission webhook disabled");
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
            tracing::info!(seconds = drain.as_secs(), "draining the admission webhook");
            tokio::time::sleep(drain).await;
            handle.graceful_shutdown(Some(GRACEFUL_SHUTDOWN));
        }
    });

    let router = router.layer(axum::middleware::from_fn(bounded));
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

#[cfg(test)]
mod tests {
    use axum::routing::post;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_request_past_the_timeout_gets_a_503() {
        let router = axum::Router::new()
            .route(
                "/validate",
                post(|| async {
                    tokio::time::sleep(REQUEST_TIMEOUT * 2).await;
                    "late"
                }),
            )
            .layer(axum::middleware::from_fn(bounded));
        let request = axum::http::Request::post("/validate")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    fn missing(missing_certificate: MissingCertificate) -> WebhookServerConfig {
        WebhookServerConfig {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            cert_file: "/nonexistent/tls.crt".into(),
            key_file: "/nonexistent/tls.key".into(),
            drain: Duration::ZERO,
            missing_certificate,
        }
    }

    #[tokio::test]
    async fn a_missing_certificate_fails_the_start_when_asked_to() {
        let health = Arc::new(Health::new(Duration::from_secs(60)));
        let result = spawn_with(
            missing(MissingCertificate::Fail),
            axum::Router::new(),
            health.clone(),
            Shutdown::never(),
        )
        .await;
        let err = result.err().expect("the start must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(
            !health.ready(),
            "a webhook that serves nothing must not report ready"
        );
    }

    #[tokio::test]
    async fn a_missing_certificate_disables_the_server_by_default() {
        let health = Arc::new(Health::new(Duration::from_secs(60)));
        let server = spawn_with(
            missing(MissingCertificate::Disable),
            axum::Router::new(),
            health.clone(),
            Shutdown::never(),
        )
        .await
        .expect("disabled, not failed");
        assert!(server.0.is_none());
        assert!(health.ready());
    }
}
