//! Integration test: CRs written when `spec.instanceRef` was a bare name
//! are rewritten as `{name: …}` by the kit's migration (the operator's
//! `migrate-instance-ref` command), after the new CRD is applied.

use std::time::Duration;

use api::Object;
use api::object::InstanceRef;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, ApiResource, DynamicObject, PostParams};
use kube::{Client, CustomResourceExt};
use serde_json::json;
use weebo_kit_adapters::migrate::migrate_instance_refs;

const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(250);

/// Retries `attempt` until it succeeds: a CRD's new schema reaches the
/// apiserver's handlers asynchronously.
async fn eventually<T, E: std::fmt::Debug, F: std::future::Future<Output = Result<T, E>>>(
    what: &str,
    mut attempt: impl FnMut() -> F,
) -> T {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        match attempt().await {
            Ok(v) => return v,
            Err(e) if tokio::time::Instant::now() > deadline => panic!("{what}: {e:?}"),
            Err(_) => tokio::time::sleep(POLL).await,
        }
    }
}

/// Replaces the Object CRD's versions with `versions`.
async fn set_versions(crds: &Api<CustomResourceDefinition>, versions: serde_json::Value) {
    let mut crd = serde_json::to_value(crds.get(Object::crd_name()).await.unwrap()).unwrap();
    crd["spec"]["versions"] = versions;
    let crd: CustomResourceDefinition = serde_json::from_value(crd).unwrap();
    crds.replace(Object::crd_name(), &PostParams::default(), &crd)
        .await
        .unwrap();
}

#[tokio::test]
async fn bare_name_instance_refs_are_rewritten_as_objects() {
    let cluster = testkit::envtest::start().await;
    let client: Client = cluster.client();
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let current = serde_json::to_value(crds.get(Object::crd_name()).await.unwrap()).unwrap();
    let versions = current["spec"]["versions"].clone();

    // The CRD as it was: `instanceRef` a bare string.
    let mut old = versions.clone();
    old[0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"]["instanceRef"] =
        json!({"type": "string"});
    set_versions(&crds, old).await;
    let kind = ApiResource::erase::<Object>(&());
    let dynamic: Api<DynamicObject> = Api::namespaced_with(client.clone(), "default", &kind);
    let legacy: DynamicObject = serde_json::from_value(json!({
        "apiVersion": kind.api_version, "kind": kind.kind,
        "metadata": {"name": "legacy", "namespace": "default"},
        "spec": {"instanceRef": "main"}
    }))
    .unwrap();
    let pp = PostParams::default();
    eventually("creating a legacy object", || dynamic.create(&pp, &legacy)).await;

    // The new CRD, then the migration.
    set_versions(&crds, versions).await;
    let dry = migrate_instance_refs(&client, std::slice::from_ref(&kind), "test", true)
        .await
        .unwrap();
    assert_eq!(dry.migrated, ["{{product}}Object/default/legacy"]);
    eventually("migrating", || {
        migrate_instance_refs(&client, std::slice::from_ref(&kind), "test", false)
    })
    .await;

    let objects: Api<Object> = Api::namespaced(client.clone(), "default");
    let migrated = eventually("reading the migrated object", || objects.get("legacy")).await;
    assert_eq!(migrated.spec.instance_ref, Some(InstanceRef::named("main")));
    // Replayable: nothing left to do.
    let again = migrate_instance_refs(&client, &[kind], "test", false)
        .await
        .unwrap();
    assert!(again.migrated.is_empty());
    assert_eq!(again.unchanged, 1);
}
