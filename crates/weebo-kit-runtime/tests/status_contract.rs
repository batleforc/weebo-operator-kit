//! Layer-2: `status::record` against a scripted Kubernetes API — one merge
//! patch on the `status` subresource carrying the given fields, retried
//! while the apiserver is unavailable, never on a refusal.

#![expect(clippy::unwrap_used, reason = "test helpers fail the test on purpose")]

use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use serde_json::json;
use weebo_kit_runtime::status;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STATUS: &str = "/api/v1/namespaces/team-a/pods/app/status";

fn api(server: &MockServer) -> Api<Pod> {
    // Err: another test installed it first, which is as good.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let config = kube::Config::new(server.uri().parse().unwrap());
    Api::namespaced(kube::Client::try_from(config).unwrap(), "team-a")
}

fn failure(code: u16) -> ResponseTemplate {
    ResponseTemplate::new(code).set_body_json(json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "reason": "x", "code": code, "message": "x",
    }))
}

fn patched() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "app", "namespace": "team-a"},
        "status": {"message": "recorded"},
    }))
}

#[tokio::test]
async fn a_status_record_survives_a_brief_outage() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(STATUS))
        .respond_with(failure(503))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(STATUS))
        .and(header("content-type", "application/merge-patch+json"))
        .and(body_json(
            json!({"status": {"message": "recorded", "reason": null}}),
        ))
        .respond_with(patched())
        .expect(1)
        .mount(&server)
        .await;
    let fields = json!({"message": "recorded", "reason": null});
    let pod = status::record(&api(&server), "weebo-example-operator", "app", &fields)
        .await
        .unwrap();
    assert_eq!(pod.status.unwrap().message.as_deref(), Some("recorded"));
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_refused_status_record_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(STATUS))
        .respond_with(failure(422))
        .expect(1)
        .mount(&server)
        .await;
    let err = status::record(
        &api(&server),
        "weebo-example-operator",
        "app",
        &json!({"message": "x"}),
    )
    .await
    .unwrap_err();
    assert!(!status::transient(&err));
}

#[tokio::test]
async fn fields_that_cannot_be_serialized_fail_before_any_request() {
    struct Broken;
    impl serde::Serialize for Broken {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("broken"))
        }
    }
    let server = MockServer::start().await;
    let err = status::record(&api(&server), "weebo-example-operator", "app", &Broken)
        .await
        .unwrap_err();
    assert!(matches!(err, kube::Error::SerdeError(_)), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}
