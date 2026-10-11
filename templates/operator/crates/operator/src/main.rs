//! The `{{project-name}}-operator` binary: wires the adapters into the
//! controllers, then runs them on the kit's runtime (telemetry, health,
//! graceful shutdown, leader election, HTTPS probes).

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx, Shutdown};
use adapters_inbound::webhook;
use adapters_outbound::HttpGatewayFactory;
use api::Instance;
use futures::StreamExt;
use kube::Client;
use kube::api::Api;
use kube::runtime::events::{Recorder, Reporter};
use kube::runtime::{WatchStreamExt, reflector, watcher};
use weebo_kit_runtime::{Health, leader, telemetry, webhook_server};

/// Default `LEASE_NAME`. Keep it across releases: old and new pods of a
/// rollout must compete for the same lease.
const DEFAULT_LEASE_NAME: &str = "{{project-name}}-operator-leader";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry = telemetry::init(controller::FIELD_MANAGER)?;
    let result = run(&telemetry).await;
    if let Err(err) = &result {
        tracing::error!(error = %err, "operator failed");
    }
    telemetry.shutdown();
    result
}

async fn run(telemetry: &telemetry::Telemetry) -> anyhow::Result<()> {
    // `ring` and `aws-lc-rs` are both linked transitively (`kube`'s rustls
    // vs. reqwest's); rustls refuses to pick a default provider when more
    // than one is present.
    let _ = rustls::crypto::ring::default_provider().install_default();

    telemetry.serve_metrics().await?;
    register_leader_gauge();

    let client = Client::try_default().await?;

    // The binary is PID 1 in its container: without a handler, SIGTERM
    // would be ignored and every rollout would wait for the SIGKILL.
    let shutdown = Shutdown::on_termination()?;
    let health = Arc::new(Health::default());

    // Every gateway resolution reads a {{product}}Instance; one shared
    // reflector serves them from memory instead of a GET per reconcile.
    // Watch errors back off exponentially instead of spinning.
    let (instances, writer) = reflector::store::<Instance>();
    let instance_reflector = reflector::reflector(
        writer,
        watcher(
            Api::<Instance>::all(client.clone()),
            watcher::Config::default(),
        )
        .default_backoff(),
    );
    tokio::spawn({
        let health = health.clone();
        async move {
            instance_reflector
                .for_each(|event| {
                    match event {
                        Ok(_) => health.ok("{{product}}Instance reflector"),
                        Err(err) => {
                            tracing::warn!(error = %err, "{{product}}Instance reflector watch error");
                            health.failed("{{product}}Instance reflector", err.to_string());
                        }
                    }
                    futures::future::ready(())
                })
                .await;
        }
    });

    let holder_id = std::env::var("HOSTNAME").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let ctx = Arc::new(Ctx {
        client: client.clone(),
        gateways: Arc::new(HttpGatewayFactory::new(client.clone(), Some(instances))),
        recorder: Recorder::new(
            client.clone(),
            Reporter {
                controller: controller::FIELD_MANAGER.into(),
                instance: Some(holder_id.clone()),
            },
        ),
        shutdown: shutdown.clone(),
        health: health.clone(),
    });

    // The HTTPS server (probes, and an admission webhook once there is
    // one) runs on every replica, before and regardless of leadership: a
    // standby must stay ready, and a leader-only webhook would turn every
    // failover into an outage. On shutdown it keeps answering while
    // endpoints drain.
    let server =
        webhook_server::spawn(webhook::router(health.clone()), health, shutdown.clone()).await?;

    // The controllers, in contrast, run on one replica at a time: two
    // active reconcilers could both create the same remote object.
    let leadership = tokio::select! {
        leadership = leader::acquire_leadership(&client, holder_id, DEFAULT_LEASE_NAME) => leadership,
        () = shutdown.wait() => {
            server.stopped().await;
            return Ok(());
        }
    };

    tracing::info!("starting controllers");
    tokio::join!(
        controller::instance::run(client.clone(), ctx.clone()),
        controller::object::run(client, ctx),
    );
    // Every controller returns once the shutdown signal arrived and its
    // in-flight reconciles finished: hand the lease over right away
    // instead of making the next leader wait out the TTL.
    leadership.release().await;
    // Bounded: the drain started with the signal, long before.
    let _ = tokio::time::timeout(Duration::from_secs(30), server.stopped()).await;
    Ok(())
}

/// `<prefix>_leader`: 1 on the replica holding the lease.
fn register_leader_gauge() {
    // The callback lives as long as the meter provider: the handle can go.
    let _ = opentelemetry::global::meter(controller::FIELD_MANAGER)
        .u64_observable_gauge(format!("{}_leader", controller::METRIC_PREFIX))
        .with_description("1 when this replica holds the leader lease")
        .with_callback(|observer| observer.observe(u64::from(leader::is_leader()), &[]))
        .build();
}
