//! Layer-2: `KubeOwnedObjects` and `KubePodLogs` against a scripted
//! Kubernetes API — an object of another owner is never applied over, a
//! pod reading an unlabelled `Secret` or a foreign image is refused before
//! any write, an owned object is applied with its owner reference, and
//! logs are read bounded, with timestamps.

use serde_json::json;
use weebo_kit_adapters::{KubeOwnedObjects, KubePodLogs, WorkloadSettings};
use weebo_kit_application::workload::{
    LogQuery, OwnedObject, OwnedObjects, Owner, PodLogs, WorkloadError,
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LABEL: &str = "example.weebo.io/credential";

fn client(server: &MockServer) -> kube::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = kube::Config::new(server.uri().parse().unwrap());
    kube::Client::try_from(config).unwrap()
}

fn objects(server: &MockServer) -> KubeOwnedObjects {
    KubeOwnedObjects::new(
        client(server),
        WorkloadSettings {
            field_manager: "weebo-example-operator".into(),
            managed_by: "weebo-example".into(),
            owner_api_version: "example.weebo.io/v1alpha1".into(),
            owner_kind: "ExampleLocalInstance".into(),
            credential_label: LABEL.into(),
            allowed_images: vec!["registry.example/app".into()],
        },
    )
}

fn owner() -> Owner {
    Owner {
        namespace: "team".into(),
        name: "hub".into(),
        uid: "u-1".into(),
    }
}

fn not_found() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "reason": "NotFound", "code": 404, "message": "not found",
    }))
}

fn service() -> OwnedObject {
    OwnedObject {
        api_version: "v1".into(),
        kind: "Service".into(),
        name: "hub".into(),
        manifest: json!({
            "apiVersion": "v1", "kind": "Service",
            "metadata": {"name": "hub", "namespace": "team"},
            "spec": {"ports": [{"port": 80}]},
        }),
        create_only: false,
    }
}

fn deployment(image: &str, secret: &str) -> OwnedObject {
    OwnedObject {
        api_version: "apps/v1".into(),
        kind: "Deployment".into(),
        name: "hub".into(),
        manifest: json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "hub", "namespace": "team"},
            "spec": {"template": {"spec": {
                "containers": [{"name": "app", "image": image}],
                "volumes": [{"name": "s", "secret": {"secretName": secret}}],
            }}},
        }),
        create_only: false,
    }
}

async fn writes(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PATCH")
        .count()
}

#[tokio::test]
async fn an_owned_object_is_applied_with_its_owner_reference() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team/services/hub"))
        .respond_with(not_found())
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/namespaces/team/services/hub"))
        .and(query_param("fieldManager", "weebo-example-operator"))
        .and(query_param("force", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(service().manifest))
        .expect(1)
        .mount(&server)
        .await;
    objects(&server)
        .apply(&owner(), &[service()])
        .await
        .unwrap();
    let patch = &server.received_requests().await.unwrap()[1];
    let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
    assert_eq!(
        body["metadata"]["ownerReferences"][0],
        json!({
            "apiVersion": "example.weebo.io/v1alpha1", "kind": "ExampleLocalInstance",
            "name": "hub", "uid": "u-1", "controller": true, "blockOwnerDeletion": true,
        })
    );
}

#[tokio::test]
async fn an_object_of_another_owner_is_never_taken_over() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team/services/hub"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1", "kind": "Service",
            "metadata": {"name": "hub", "namespace": "team", "labels": {"app": "theirs"}},
        })))
        .mount(&server)
        .await;
    let e = objects(&server)
        .apply(&owner(), &[service()])
        .await
        .unwrap_err();
    assert!(matches!(e, WorkloadError::Conflict(_)), "{e}");
    assert_eq!(writes(&server).await, 0);
}

#[tokio::test]
async fn pods_off_the_rules_are_refused_before_any_write() {
    let server = MockServer::start().await;
    // A foreign image: refused without a read.
    let e = objects(&server)
        .apply(&owner(), &[service(), deployment("evil/x:1", "s")])
        .await
        .unwrap_err();
    assert!(matches!(e, WorkloadError::Refused(_)), "{e}");
    assert!(e.to_string().contains("evil/x:1"), "{e}");
    // A Secret without the label.
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team/secrets/s"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
            "metadata": {"name": "s", "namespace": "team"},
        })))
        .mount(&server)
        .await;
    let e = objects(&server)
        .apply(
            &owner(),
            &[service(), deployment("registry.example/app:1", "s")],
        )
        .await
        .unwrap_err();
    assert!(matches!(e, WorkloadError::SecretNotAllowed(_)), "{e}");
    assert!(e.to_string().contains(LABEL), "{e}");
    assert_eq!(writes(&server).await, 0);
}

#[tokio::test]
async fn logs_are_read_bounded_with_timestamps() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team/pods/hub-1/log"))
        .and(query_param("timestamps", "true"))
        .and(query_param("limitBytes", "1024"))
        .and(query_param("sinceTime", "2026-10-16T10:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "2026-10-16T10:00:01.5Z {\"fields\":{\"message\":\"Configuration reloaded\"}}\n",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let lines = KubePodLogs::new(client(&server))
        .read(
            "team",
            "hub-1",
            &LogQuery {
                container: None,
                since: Some("2026-10-16T10:00:00Z".parse().unwrap()),
                limit_bytes: Some(1024),
            },
        )
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].text.contains("Configuration reloaded"));
}
