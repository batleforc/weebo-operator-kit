//! Layer-2: `LeaseLock` against a scripted Kubernetes API — taking a free
//! or long-stale lease, leaving a live one alone, losing a race on the
//! `resourceVersion`, renewing, and stepping down.

use std::time::Duration;

use k8s_openapi::jiff::{Timestamp, ToSpan};
use serde_json::json;
use weebo_kit_runtime::leader::{LeaseLock, lease};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LEASE: &str = "/apis/coordination.k8s.io/v1/namespaces/ops/leases/op-leader";

fn lock(server: &MockServer, holder: &str) -> LeaseLock {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = kube::Config::new(server.uri().parse().unwrap());
    let client = kube::Client::try_from(config).unwrap();
    lease(
        client,
        "ops",
        "op-leader",
        holder.to_string(),
        Duration::from_secs(15),
    )
}

fn micro_time(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn lease_json(holder: &str, renewed: Timestamp, transitions: i32) -> serde_json::Value {
    json!({
        "apiVersion": "coordination.k8s.io/v1", "kind": "Lease",
        "metadata": {"name": "op-leader", "namespace": "ops", "resourceVersion": "7"},
        "spec": {
            "holderIdentity": holder, "leaseDurationSeconds": 15,
            "acquireTime": micro_time(renewed), "renewTime": micro_time(renewed),
            "leaseTransitions": transitions,
        },
    })
}

fn status(code: u16, reason: &str) -> ResponseTemplate {
    ResponseTemplate::new(code).set_body_json(json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "reason": reason, "code": code, "message": reason,
    }))
}

fn ok(lease: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(lease)
}

async fn current(server: &MockServer, lease: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(LEASE))
        .respond_with(ok(lease))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_missing_lease_is_created() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(LEASE))
        .respond_with(status(404, "NotFound"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apis/coordination.k8s.io/v1/namespaces/ops/leases"))
        .and(body_partial_json(json!({"spec": {"holderIdentity": "me"}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(lease_json(
            "me",
            Timestamp::now(),
            0,
        )))
        .expect(1)
        .mount(&server)
        .await;
    assert!(lock(&server, "me").try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn a_live_lease_of_another_replica_is_left_alone() {
    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now(), 3)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    assert!(!lock(&server, "me").try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn a_long_stale_lease_is_taken_over() {
    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now() - 1.hour(), 3)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .and(body_partial_json(json!({
            "metadata": {"resourceVersion": "7"},
            "spec": {"holderIdentity": "me", "leaseTransitions": 4},
        })))
        .respond_with(ok(lease_json("me", Timestamp::now(), 4)))
        .expect(1)
        .mount(&server)
        .await;
    assert!(lock(&server, "me").try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn losing_the_race_on_the_resource_version_is_not_an_error() {
    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now() - 1.hour(), 3)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .respond_with(status(409, "Conflict"))
        .mount(&server)
        .await;
    assert!(!lock(&server, "me").try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn the_holder_renews_and_keeps_its_acquire_time() {
    let server = MockServer::start().await;
    let held = lease_json("me", Timestamp::now(), 2);
    let acquired = held["spec"]["acquireTime"].clone();
    current(&server, held).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .and(body_partial_json(json!({
            "spec": {"holderIdentity": "me", "leaseTransitions": 2, "acquireTime": acquired},
        })))
        .respond_with(ok(lease_json("me", Timestamp::now(), 2)))
        .expect(1)
        .mount(&server)
        .await;
    assert!(lock(&server, "me").try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn stepping_down_clears_only_its_own_holder() {
    let server = MockServer::start().await;
    current(&server, lease_json("me", Timestamp::now(), 2)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .and(body_partial_json(json!({"spec": {"holderIdentity": ""}})))
        .respond_with(ok(lease_json("", Timestamp::now(), 2)))
        .expect(1)
        .mount(&server)
        .await;
    lock(&server, "me").step_down().await.unwrap();

    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now(), 2)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    lock(&server, "me").step_down().await.unwrap();
}
