//! Integration test: a real `kube-apiserver` + `etcd` (envtest), the real
//! `{{product}}Object` controller running against it, the gateway swapped
//! for an in-memory {{product}} (`testkit::FakeGateway`). The template for
//! every other kind's controller test.

use std::sync::Arc;
use std::time::Duration;

use adapters_inbound::controller::{self, Ctx, Shutdown};
use api::Object;
use api::object::ObjectSpec;
use k8s_openapi::api::core::v1::Event;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use kube::runtime::events::{Recorder, Reporter};
use testkit::{FakeGateway, StaticGatewayFactory};
use weebo_kit_runtime::Health;

const TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(200);

fn ctx(client: Client, gateway: Arc<FakeGateway>) -> Arc<Ctx> {
    Arc::new(Ctx {
        client: client.clone(),
        gateways: Arc::new(StaticGatewayFactory::new(gateway).without("missing")),
        recorder: Recorder::new(
            client,
            Reporter {
                controller: controller::FIELD_MANAGER.into(),
                instance: None,
            },
        ),
        shutdown: Shutdown::never(),
        health: Arc::new(Health::default()),
    })
}

fn object(name: &str, instance: &str) -> Object {
    Object::new(
        name,
        ObjectSpec {
            instance_ref: instance.into(),
            name: None,
            description: Some("from envtest".into()),
        },
    )
}

/// `Ready`'s status and reason, once there is one.
fn ready(object: &Object) -> Option<(String, String)> {
    let c = weebo_kit_api::status::ready(&object.status.as_ref()?.conditions)?;
    Some((c.status.clone(), c.reason.clone()))
}

/// Waits for an Event of `reason` on `name`, returns its type.
async fn wait_for_event(client: &Client, name: &str, reason: &str) -> String {
    let events: Api<Event> = Api::namespaced(client.clone(), "default");
    let params = ListParams::default().fields(&format!("involvedObject.name={name}"));
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let found = events
                .list(&params)
                .await
                .expect("events must be listable")
                .items
                .into_iter()
                .find(|e| e.reason.as_deref() == Some(reason));
            if let Some(event) = found {
                return event.type_.unwrap_or_default();
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {reason} Event on {name} within {TIMEOUT:?}"))
}

#[tokio::test]
async fn object_lifecycle_creates_reports_and_deletes_the_remote_object() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let cluster = testkit::envtest::start().await;
    let client = cluster.client();
    let gateway = Arc::new(FakeGateway::default());
    tokio::spawn(controller::object::run(
        client.clone(),
        ctx(client.clone(), gateway.clone()),
    ));
    let objects: Api<Object> = Api::namespaced(client.clone(), "default");

    // Synced: the remote object exists, its id is recorded, `Ready: True`.
    objects
        .create(&PostParams::default(), &object("docs", "main"))
        .await
        .unwrap();
    let remote_id = testkit::polling::wait_for(&objects, "docs", TIMEOUT, POLL, |o| {
        o.status.as_ref()?.remote_id.clone()
    })
    .await;
    assert_eq!(remote_id, "1");
    let synced = objects.get("docs").await.unwrap();
    assert_eq!(ready(&synced), Some(("True".into(), "Reconciled".into())));
    assert_eq!(
        gateway.get("docs").and_then(|o| o.description).as_deref(),
        Some("from envtest")
    );
    assert_eq!(wait_for_event(&client, "docs", "Created").await, "Normal");

    // An unknown instance: `Ready: False` with its reason, and a Warning.
    objects
        .create(&PostParams::default(), &object("orphan", "missing"))
        .await
        .unwrap();
    let failed = testkit::polling::wait_for(&objects, "orphan", TIMEOUT, POLL, ready).await;
    assert_eq!(failed, ("False".into(), "InstanceNotFound".into()));
    assert_eq!(
        wait_for_event(&client, "orphan", "InstanceNotFound").await,
        "Warning"
    );

    // Deleted: the finalizer removes the remote object, then lets go.
    objects
        .delete("docs", &DeleteParams::default())
        .await
        .unwrap();
    testkit::polling::wait_for_absence(&objects, "docs", TIMEOUT, POLL).await;
    assert!(gateway.get("docs").is_none());
    assert_eq!(wait_for_event(&client, "docs", "Deleted").await, "Normal");

    // Never created anything: released without touching the instance.
    objects
        .delete("orphan", &DeleteParams::default())
        .await
        .unwrap();
    testkit::polling::wait_for_absence(&objects, "orphan", TIMEOUT, POLL).await;
}
