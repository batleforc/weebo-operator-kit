//! Layer-2: `LeaseLock` against a scripted Kubernetes API — taking a free
//! lease or one left unchanged for its TTL (on this replica's monotonic
//! clock, whatever its `renewTime` says), leaving a live one alone, losing
//! a race on the `resourceVersion`, renewing, and stepping down.

use std::sync::atomic::{AtomicUsize, Ordering};
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
    lease_at("7", holder, renewed, transitions)
}

fn lease_at(
    version: &str,
    holder: &str,
    renewed: Timestamp,
    transitions: i32,
) -> serde_json::Value {
    json!({
        "apiVersion": "coordination.k8s.io/v1", "kind": "Lease",
        "metadata": {"name": "op-leader", "namespace": "ops", "resourceVersion": version},
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

/// Lets `by` pass on this replica's monotonic clock. The clock is only
/// paused around the jump: paused while a request is in flight, tokio would
/// auto-advance it into the HTTP client's timeouts.
async fn wait(by: Duration) {
    tokio::time::pause();
    tokio::time::advance(by).await;
    tokio::time::resume();
}

#[tokio::test]
async fn a_lease_unchanged_for_its_ttl_is_taken_over() {
    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now(), 3)).await;
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
    let lock = lock(&server, "me");
    // First sighting of the record: its TTL starts now, here.
    assert!(!lock.try_acquire_or_renew().await.unwrap());
    wait(Duration::from_secs(10)).await;
    assert!(!lock.try_acquire_or_renew().await.unwrap());
    wait(Duration::from_secs(6)).await;
    assert!(lock.try_acquire_or_renew().await.unwrap());
}

#[tokio::test]
async fn a_renewed_lease_is_never_taken_whatever_its_renew_time() {
    // The holder's clock runs an hour behind: its renewTime looks long
    // stale, but the record keeps changing — the holder is alive.
    let server = MockServer::start().await;
    let calls = AtomicUsize::new(7);
    Mock::given(method("GET"))
        .and(path(LEASE))
        .respond_with(move |_: &wiremock::Request| {
            let version = calls.fetch_add(1, Ordering::SeqCst).to_string();
            ok(lease_at(&version, "other", Timestamp::now() - 1.hour(), 3))
        })
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let lock = lock(&server, "me");
    for _ in 0..4 {
        assert!(!lock.try_acquire_or_renew().await.unwrap());
        wait(Duration::from_secs(10)).await;
    }
}

#[tokio::test]
async fn losing_the_race_on_the_resource_version_is_not_an_error() {
    let server = MockServer::start().await;
    current(&server, lease_json("other", Timestamp::now(), 3)).await;
    Mock::given(method("PUT"))
        .and(path(LEASE))
        .respond_with(status(409, "Conflict"))
        .expect(1)
        .mount(&server)
        .await;
    let lock = lock(&server, "me");
    assert!(!lock.try_acquire_or_renew().await.unwrap());
    wait(Duration::from_secs(16)).await;
    assert!(!lock.try_acquire_or_renew().await.unwrap());
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
