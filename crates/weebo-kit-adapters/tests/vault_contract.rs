//! Layer-2: the Vault KV v2 store (`VaultSecretStore`, what
//! `RoutingSecretStore` sends Vault targets to) against scripted HTTP —
//! Kubernetes-auth login, idempotent writes (no new KV version for
//! identical data), existence checks, deletes, and re-logins only on a
//! stale token.

use std::collections::BTreeMap;

use serde_json::json;
use weebo_kit_adapters::{VaultConfig, VaultSecretStore};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn spec(server: &MockServer) -> VaultConfig {
    VaultConfig {
        address: server.uri(),
        mount: "secret".into(),
        path_prefix: "weebo-example".into(),
        kubernetes_auth_role: "weebo-example".into(),
        kubernetes_auth_mount: "kubernetes".into(),
        ca_source: None,
    }
}

/// Vault wraps every response in this envelope (`vaultrs` requires the
/// top-level fields).
fn envelope(data: serde_json::Value, auth: serde_json::Value) -> serde_json::Value {
    json!({
        "request_id": "r", "lease_id": "", "lease_duration": 0, "renewable": false,
        "data": data, "auth": auth, "wrap_info": null, "warnings": null,
    })
}

async fn login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .and(body_partial_json(
            json!({"role": "weebo-example", "jwt": "sa-jwt"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            serde_json::Value::Null,
            json!({
                "client_token": "vault-token",
                "accessor": "a",
                "policies": [],
                "token_policies": [],
                "metadata": {},
                "lease_duration": 3600,
                "renewable": true,
                "entity_id": "",
                "token_type": "service",
                "orphan": true,
            }),
        )))
        .mount(server)
        .await;
}

/// `lookup-self` refused: the token itself is no longer valid.
async fn stale_token(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/auth/token/lookup-self"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]})),
        )
        .mount(server)
        .await;
}

/// `lookup-self` accepted: a 403 is the policy's, not the token's.
async fn valid_token(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/auth/token/lookup-self"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"id": "vault-token"}),
            serde_json::Value::Null,
        )))
        .mount(server)
        .await;
}

async fn logins(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/auth/kubernetes/login")
        .count()
}

fn kv_response(data: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(envelope(
        json!({
            "data": data,
            "metadata": {
                "created_time": "2026-10-06T00:00:00Z",
                "custom_metadata": null,
                "deletion_time": "",
                "destroyed": false,
                "version": 1
            }
        }),
        serde_json::Value::Null,
    ))
}

#[tokio::test]
async fn logs_in_and_caches_for_most_of_the_lease() {
    let server = MockServer::start().await;
    login(&server).await;
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt\n", None)
        .await
        .unwrap();
    assert_eq!(
        store.cache_ttl(),
        Some(std::time::Duration::from_secs(2880))
    );
    assert_eq!(
        store.default_path("team-a", "app"),
        "weebo-example/team-a/app"
    );
}

#[tokio::test]
async fn identical_data_is_not_rewritten() {
    let server = MockServer::start().await;
    login(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/weebo-example/team-a/app"))
        .respond_with(kv_response(json!({"EXAMPLE_TOKEN": "t"})))
        .mount(&server)
        .await;
    // A write would be a new KV version: it must not happen.
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/weebo-example/team-a/app"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    let data = BTreeMap::from([("EXAMPLE_TOKEN".to_string(), "t".to_string())]);
    store
        .write_path("weebo-example/team-a/app", &data)
        .await
        .unwrap();
}

#[tokio::test]
async fn new_data_is_written_with_the_vault_token() {
    let server = MockServer::start().await;
    login(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/custom/path"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"errors": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/custom/path"))
        .and(wiremock::matchers::header("X-Vault-Token", "vault-token"))
        .and(body_partial_json(json!({"data": {"EXAMPLE_TOKEN": "t2"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"created_time": "2026-10-06T00:00:00Z", "custom_metadata": null,
                   "deletion_time": "", "destroyed": false, "version": 2}),
            serde_json::Value::Null,
        )))
        .expect(1)
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    assert_eq!(store.read_path("custom/path").await.unwrap(), None);
    let data = BTreeMap::from([("EXAMPLE_TOKEN".to_string(), "t2".to_string())]);
    store.write_path("custom/path", &data).await.unwrap();
}

#[tokio::test]
async fn delete_removes_every_version_and_is_idempotent() {
    let server = MockServer::start().await;
    login(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/metadata/gone"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"errors": []})))
        .mount(&server)
        .await;
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    store.delete_path("gone").await.unwrap();
}

#[tokio::test]
async fn an_expired_token_relogs_in_once() {
    let server = MockServer::start().await;
    login(&server).await;
    stale_token(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .respond_with(kv_response(json!({"k": "v"})))
        .mount(&server)
        .await;
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    let read = store.read_path("p").await.unwrap().unwrap();
    assert_eq!(read["k"], "v");
    let logins = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/auth/kubernetes/login")
        .count();
    assert_eq!(logins, 2);
}

#[tokio::test]
async fn a_refused_login_names_the_mount_and_role() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]})),
        )
        .mount(&server)
        .await;
    let err = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .err()
        .expect("login must fail");
    assert!(err.0.contains("\"kubernetes\""), "{}", err.0);
    assert!(err.0.contains("\"weebo-example\""), "{}", err.0);
}

#[tokio::test]
async fn a_configured_ca_bundle_is_loaded() {
    let server = MockServer::start().await;
    login(&server).await;
    let ca = rcgen::generate_simple_self_signed(vec!["vault.local".into()])
        .unwrap()
        .cert
        .pem();
    // Plain HTTP to wiremock: the CA is loaded, just never needed.
    assert!(
        VaultSecretStore::new(&spec(&server), "sa-jwt", Some(ca.as_bytes()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn writes_and_deletes_relogin_once_on_an_expired_token() {
    let server = MockServer::start().await;
    login(&server).await;
    stale_token(&server).await;
    let denied =
        || ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]}));
    // Nothing stored yet (read), then one 403 per write and delete.
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"errors": []})))
        .mount(&server)
        .await;
    for (verb, p) in [
        ("POST", "/v1/secret/data/p"),
        ("DELETE", "/v1/secret/metadata/p"),
    ] {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(denied())
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/v1/secret/data/p"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"created_time": "2026-10-06T00:00:00Z", "custom_metadata": null,
                   "deletion_time": "", "destroyed": false, "version": 1}),
            serde_json::Value::Null,
        )))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/metadata/p"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    store
        .write_path("p", &BTreeMap::from([("k".to_string(), "v".to_string())]))
        .await
        .unwrap();
    store.delete_path("p").await.unwrap();
    let logins = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/auth/kubernetes/login")
        .count();
    assert_eq!(logins, 3, "the initial login plus one per refused call");
}

#[tokio::test]
async fn server_errors_are_errors_not_missing_secrets() {
    let server = MockServer::start().await;
    login(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"errors": ["boom"]})))
        .mount(&server)
        .await;
    for (verb, p) in [
        ("POST", "/v1/secret/data/p"),
        ("DELETE", "/v1/secret/metadata/p"),
    ] {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({"errors": ["boom"]})))
            .mount(&server)
            .await;
    }
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    assert!(store.read_path("p").await.is_err());
    assert!(
        store
            .write_path("p", &BTreeMap::from([("k".to_string(), "v".to_string())]))
            .await
            .is_err()
    );
    assert!(store.delete_path("p").await.is_err());
}

#[tokio::test]
async fn a_policy_denial_never_relogs_in() {
    let server = MockServer::start().await;
    login(&server).await;
    valid_token(&server).await;
    let denied =
        || ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]}));
    for (verb, p) in [
        ("GET", "/v1/secret/metadata/p"),
        ("POST", "/v1/secret/metadata/p"),
        ("GET", "/v1/secret/data/p"),
    ] {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(denied())
            .mount(&server)
            .await;
    }
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    // No `metadata` rights: ownership is neither checked nor recorded.
    assert_eq!(store.owner_of("p").await.unwrap(), None);
    store.set_owner("p", "Kind/ns/name").await.unwrap();
    assert!(store.read_path("p").await.is_err());
    assert_eq!(logins(&server).await, 1, "only the initial login");
}

#[tokio::test]
async fn without_metadata_rights_delete_soft_deletes_the_current_version() {
    let server = MockServer::start().await;
    login(&server).await;
    valid_token(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/metadata/p"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/data/p"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap();
    store.delete_path("p").await.unwrap();
    assert_eq!(logins(&server).await, 1);
}

#[tokio::test]
async fn a_relogin_reads_the_service_account_token_again() {
    let server = MockServer::start().await;
    login(&server).await;
    stale_token(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .and(body_partial_json(json!({"jwt": "rotated-jwt"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            serde_json::Value::Null,
            json!({
                "client_token": "vault-token-2", "accessor": "a", "policies": [],
                "token_policies": [], "metadata": {}, "lease_duration": 3600,
                "renewable": true, "entity_id": "", "token_type": "service", "orphan": true,
            }),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .and(wiremock::matchers::header("X-Vault-Token", "vault-token"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"errors": ["permission denied"]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/p"))
        .and(wiremock::matchers::header("X-Vault-Token", "vault-token-2"))
        .respond_with(kv_response(json!({"k": "v"})))
        .mount(&server)
        .await;
    let jwt = std::env::temp_dir().join(format!("weebo-kit-jwt-{}", std::process::id()));
    std::fs::write(&jwt, "rotated-jwt\n").unwrap();
    let store = VaultSecretStore::new(&spec(&server), "sa-jwt", None)
        .await
        .unwrap()
        .with_jwt_path(&jwt);
    let read = store.read_path("p").await.unwrap().unwrap();
    std::fs::remove_file(&jwt).unwrap();
    assert_eq!(read["k"], "v");
}
