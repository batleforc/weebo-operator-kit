//! The HTTPS server an admission webhook is served on, on every replica
//! (not only the leader): with `failurePolicy: Fail`, a leader-only webhook
//! would turn every failover into an outage for the governed kinds.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;

/// cert-manager rotates the webhook certificate in place; re-reading it on
/// a fixed interval picks rotations up without a restart.
pub const TLS_RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// Serves `router` over TLS when the certificate is mounted
/// (`WEBHOOK_TLS_CERT_FILE`/`WEBHOOK_TLS_KEY_FILE`, default
/// `/etc/webhook/certs/tls.{crt,key}`) on `WEBHOOK_PORT` (default 8443).
/// Without it — local runs — nothing is served and `Ok(false)` is returned
/// with a warning: reconcilers re-check the allow-list on their own anyway.
/// The process exits if the server stops.
pub async fn spawn(router: axum::Router) -> anyhow::Result<bool> {
    let cert = std::env::var("WEBHOOK_TLS_CERT_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.crt".to_string());
    let key = std::env::var("WEBHOOK_TLS_KEY_FILE")
        .unwrap_or_else(|_| "/etc/webhook/certs/tls.key".to_string());
    if !Path::new(&cert).exists() {
        tracing::warn!(%cert, "no webhook certificate, admission webhook disabled");
        return Ok(false);
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

    tracing::info!(%addr, "admission webhook listening");
    tokio::spawn(async move {
        if let Err(err) = axum_server::bind_rustls(addr, tls)
            .serve(router.into_make_service())
            .await
        {
            tracing::error!(error = %err, "admission webhook stopped");
            std::process::exit(1);
        }
    });
    Ok(true)
}
