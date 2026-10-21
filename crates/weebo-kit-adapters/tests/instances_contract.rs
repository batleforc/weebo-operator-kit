//! Layer-2: `Instances` (the operator's two instance kinds as one
//! `InstanceDirectory`) against a scripted Kubernetes API — listed from
//! the API (no stores wired), within the operator's scope, and a recorded
//! binding kept.

#![expect(clippy::unwrap_used, reason = "test helpers fail the test on purpose")]

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use weebo_kit_adapters::{Instances, LocalInstances, SharesItself};
use weebo_kit_api::{BoundInstance, InstanceRef, SharedWith};
use weebo_kit_application::instances::{InstanceDirectory, InstanceError, ResolveError};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "example.weebo.io", version = "v1", kind = "ExampleInstance")]
pub struct ExampleInstanceSpec {
    pub url: String,
}

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "example.weebo.io",
    version = "v1",
    kind = "ExampleLocalInstance",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct ExampleLocalInstanceSpec {
    pub url: String,
    #[serde(default)]
    pub shared_with: Option<SharedWith>,
}

impl SharesItself for ExampleLocalInstance {
    fn shared_with(&self) -> Option<&SharedWith> {
        self.spec.shared_with.as_ref()
    }
}

type Directory = Instances<ExampleInstance, ExampleLocalInstance>;

fn list(kind: &str, items: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "apiVersion": "example.weebo.io/v1",
        "kind": format!("{kind}List"),
        "metadata": {"resourceVersion": "1"},
        "items": items,
    }))
}

fn cluster(name: &str) -> serde_json::Value {
    json!({
        "apiVersion": "example.weebo.io/v1", "kind": "ExampleInstance",
        "metadata": {"name": name}, "spec": {"url": "https://example.test"},
    })
}

fn local(namespace: &str, name: &str, shared_with: serde_json::Value) -> serde_json::Value {
    json!({
        "apiVersion": "example.weebo.io/v1", "kind": "ExampleLocalInstance",
        "metadata": {"name": name, "namespace": namespace},
        "spec": {"url": "https://example.test", "sharedWith": shared_with},
    })
}

async fn api(clusters: &[&str], locals: &[(&str, &str, serde_json::Value)]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apis/example.weebo.io/v1/exampleinstances"))
        .respond_with(list(
            "ExampleInstance",
            clusters.iter().map(|n| cluster(n)).collect(),
        ))
        .mount(&server)
        .await;
    for ns in ["team-a", "team-b"] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/apis/example.weebo.io/v1/namespaces/{ns}/examplelocalinstances"
            )))
            .respond_with(list(
                "ExampleLocalInstance",
                locals
                    .iter()
                    .filter(|(n, _, _)| *n == ns)
                    .map(|(n, name, shared)| local(n, name, shared.clone()))
                    .collect(),
            ))
            .mount(&server)
            .await;
    }
    server
}

fn directory(server: &MockServer) -> Directory {
    // Err: another test installed it first, which is as good.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let config = kube::Config::new(server.uri().parse().unwrap());
    Instances::new(kube::Client::try_from(config).unwrap())
}

#[tokio::test]
async fn without_a_ref_the_recorded_local_instance_stays() {
    let server = api(&["prod"], &[("team-a", "own", json!(null))]).await;
    let d = directory(&server);
    let own = BoundInstance::local("team-a", "own");
    // Never bound: the cluster instance is the default.
    assert_eq!(
        d.resolve(Some("team-a"), None, None).await,
        Ok(BoundInstance::cluster("prod"))
    );
    // Bound to the local one before `prod` existed: it stays.
    assert_eq!(
        d.resolve(Some("team-a"), None, Some(&own)).await,
        Ok(own.clone())
    );
}

#[tokio::test]
async fn a_namespaced_operator_never_reads_cluster_instances() {
    let server = api(&["prod"], &[("team-a", "own", json!(null))]).await;
    let d = directory(&server).with_scope(Some("team-a".into()), None, false);
    assert_eq!(
        d.resolve(Some("team-a"), None, None).await,
        Ok(BoundInstance::local("team-a", "own"))
    );
    let cluster_reads = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/apis/example.weebo.io/v1/exampleinstances")
        .count();
    assert_eq!(cluster_reads, 0);
    // Another namespace's local instance is outside its scope.
    let r = InstanceRef {
        name: "shared".into(),
        kind: None,
        namespace: Some("team-b".into()),
    };
    assert!(matches!(
        d.resolve(Some("team-a"), Some(&r), None).await,
        Err(InstanceError::Resolve(ResolveError::OutsideScope(_)))
    ));
}

#[tokio::test]
async fn a_foreign_local_instance_shares_through_a_namespace_selector() {
    let server = api(
        &[],
        &[(
            "team-b",
            "shared",
            json!({"namespaceSelector": {"matchLabels": {"tier": "front"}}}),
        )],
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1", "kind": "Namespace",
            "metadata": {"name": "team-a", "labels": {"tier": "front"}},
        })))
        .mount(&server)
        .await;
    let d = directory(&server);
    let r = InstanceRef {
        name: "shared".into(),
        kind: None,
        namespace: Some("team-b".into()),
    };
    assert_eq!(
        d.resolve(Some("team-a"), Some(&r), None).await,
        Ok(BoundInstance::local("team-b", "shared"))
    );
    // Not selected: refused.
    assert!(matches!(
        d.resolve(None, Some(&r), None).await,
        Err(InstanceError::Resolve(ResolveError::NotShared { .. }))
    ));
}

#[tokio::test]
async fn an_unreadable_kind_is_a_kubernetes_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "reason": "Forbidden", "code": 403, "message": "forbidden",
        })))
        .mount(&server)
        .await;
    let d = directory(&server);
    assert!(matches!(
        d.resolve(Some("team-a"), None, None).await,
        Err(InstanceError::Kubernetes(_))
    ));
}

fn local_only(server: &MockServer) -> LocalInstances<ExampleLocalInstance> {
    // Err: another test installed it first, which is as good.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let config = kube::Config::new(server.uri().parse().unwrap());
    LocalInstances::new(kube::Client::try_from(config).unwrap())
}

#[tokio::test]
async fn a_local_only_directory_never_reads_a_cluster_kind() {
    let server = api(&["prod"], &[("team-a", "own", json!(null))]).await;
    let d = local_only(&server);
    assert_eq!(
        d.resolve(Some("team-a"), None, None).await,
        Ok(BoundInstance::local("team-a", "own"))
    );
    assert_eq!(
        d.get(&BoundInstance::local("team-a", "own"))
            .await
            .unwrap()
            .map(|i| i.spec.url),
        Some("https://example.test".to_string())
    );
    let cluster_reads = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/apis/example.weebo.io/v1/exampleinstances")
        .count();
    assert_eq!(cluster_reads, 0);
}

#[tokio::test]
async fn a_local_only_directory_shares_as_the_operator_says() {
    let server = api(&[], &[("team-b", "shared", json!(null))]).await;
    let d = local_only(&server);
    let r = InstanceRef {
        name: "shared".into(),
        kind: None,
        namespace: Some("team-b".into()),
    };
    // `resolve` shares with no one.
    assert!(matches!(
        d.resolve(Some("team-a"), Some(&r), None).await,
        Err(InstanceError::Resolve(ResolveError::NotShared { .. }))
    ));
    // Shared with every namespace by the operator's own rule.
    let everyone = |_: &ExampleLocalInstance| {
        Some(SharedWith {
            namespace_selector: Some(Default::default()),
            ..Default::default()
        })
    };
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "team-a"},
        })))
        .mount(&server)
        .await;
    assert_eq!(
        d.resolve_with(Some("team-a"), Some(&r), None, everyone)
            .await,
        Ok(BoundInstance::local("team-b", "shared"))
    );
}

#[tokio::test]
async fn a_list_read_from_the_api_follows_its_pages() {
    let server = MockServer::start().await;
    let page = |items: serde_json::Value, next: Option<&str>| {
        ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "example.weebo.io/v1", "kind": "ExampleInstanceList",
            "metadata": {"resourceVersion": "1", "continue": next},
            "items": items,
        }))
    };
    Mock::given(method("GET"))
        .and(path("/apis/example.weebo.io/v1/exampleinstances"))
        .and(query_param("limit", "500"))
        .and(query_param_is_missing("continue"))
        .respond_with(page(json!([cluster("one")]), Some("page-2")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/apis/example.weebo.io/v1/exampleinstances"))
        .and(query_param("continue", "page-2"))
        .respond_with(page(json!([cluster("two")]), None))
        .mount(&server)
        .await;
    let names: Vec<_> = directory(&server)
        .cluster_instances()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.metadata.name.unwrap())
        .collect();
    assert_eq!(names, ["one", "two"]);
}
