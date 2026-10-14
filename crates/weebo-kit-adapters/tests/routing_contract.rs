//! Layer-2: `RoutingSecretStore` against scripted HTTP (Vault, and the
//! Kubernetes API) — the Vault paths a CR may write to (its namespace's
//! subtree, or what the instance's grants open to it), the
//! ownership recorded in the KV custom metadata, and Vault resolved only
//! when a target needs it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::FutureExt;
use serde_json::json;
use weebo_kit_adapters::{
    K8sSecretStore, RoutingSecretStore, VaultConfig, VaultResolver, VaultSecretStore,
};
use weebo_kit_api::{SecretStoreBackend, SecretTarget, VaultPathGrant};
use weebo_kit_application::ports::{SecretOwner, SecretStore};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn envelope(data: serde_json::Value, auth: serde_json::Value) -> serde_json::Value {
    json!({
        "request_id": "r", "lease_id": "", "lease_duration": 0, "renewable": false,
        "data": data, "auth": auth, "wrap_info": null, "warnings": null,
    })
}

async fn vault(server: &MockServer) -> Arc<VaultSecretStore> {
    vault_with(server, "weebo-example", vec![]).await
}

async fn vault_with(
    server: &MockServer,
    path_prefix: &str,
    path_grants: Vec<VaultPathGrant>,
) -> Arc<VaultSecretStore> {
    Mock::given(method("POST"))
        .and(path("/v1/auth/kubernetes/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            serde_json::Value::Null,
            json!({
                "client_token": "vault-token", "accessor": "a", "policies": [],
                "token_policies": [], "metadata": {}, "lease_duration": 3600,
                "renewable": true, "entity_id": "", "token_type": "service", "orphan": true,
            }),
        )))
        .mount(server)
        .await;
    let config = VaultConfig {
        address: server.uri(),
        mount: "secret".into(),
        path_prefix: path_prefix.into(),
        path_grants,
        kubernetes_auth_role: "weebo-example".into(),
        kubernetes_auth_mount: "kubernetes".into(),
        ca_source: None,
    };
    Arc::new(
        VaultSecretStore::new(&config, "sa-jwt", None)
            .await
            .unwrap(),
    )
}

/// A Kubernetes client pointed at `server` (only Kubernetes targets reach
/// it).
fn kubernetes(server: &MockServer) -> Arc<K8sSecretStore> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = kube::Config::new(server.uri().parse().unwrap());
    let client = kube::Client::try_from(config).unwrap();
    Arc::new(K8sSecretStore::new(
        client,
        "weebo-example-operator",
        "weebo-example",
        "example.weebo.io/owner",
    ))
}

fn vault_target(name: &str, path: Option<&str>) -> SecretTarget {
    SecretTarget {
        backend: SecretStoreBackend::Vault,
        name: name.into(),
        namespace: None,
        path: path.map(String::from),
    }
}

/// The KV metadata of `p`, recording `owner`.
async fn owned_by(server: &MockServer, p: &str, owner: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/secret/metadata/{p}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({
                "cas_required": false, "created_time": "2026-10-06T00:00:00Z",
                "current_version": 1, "custom_metadata": {"owner": owner},
                "delete_version_after": "0s", "max_versions": 0, "oldest_version": 0,
                "updated_time": "2026-10-06T00:00:00Z", "versions": {},
            }),
            serde_json::Value::Null,
        )))
        .mount(server)
        .await;
}

const OWNER: SecretOwner<'static> = SecretOwner {
    kind: "Token",
    namespace: Some("team-a"),
    name: "ci",
    formerly: None,
};

#[tokio::test]
async fn a_namespaced_cr_stays_under_its_namespace_prefix() {
    let server = MockServer::start().await;
    let store = RoutingSecretStore::new(
        kubernetes(&server),
        Ok(vault(&server).await),
        SecretStoreBackend::Kubernetes,
    );
    let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
    for outside in [
        "weebo-example/team-b/ci",
        "other/team-a/ci",
        "weebo-example/team-a/../team-b/ci",
        "weebo-example/team-a-evil/ci",
    ] {
        let err = store
            .write(&vault_target("ci", Some(outside)), &OWNER, &data)
            .await
            .expect_err(outside);
        let refused = if outside.contains("..") {
            "\"..\" segment"
        } else {
            "allows it for namespace \"team-a\""
        };
        assert!(err.0.contains(refused), "{outside}: {}", err.0);
        assert!(
            store
                .delete(&vault_target("ci", Some(outside)), &OWNER)
                .await
                .is_err()
        );
    }
    // Nothing ever reached the KV engine.
    assert_eq!(kv_calls(&server).await, 0);
}

#[tokio::test]
async fn another_owners_value_is_neither_overwritten_nor_deleted() {
    let server = MockServer::start().await;
    let vault = vault(&server).await;
    owned_by(&server, "weebo-example/team-a/ci", "Token/team-a/other").await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/weebo-example/team-a/ci"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"data": {"k": "theirs"}, "metadata": {
                "created_time": "2026-10-06T00:00:00Z", "custom_metadata": null,
                "deletion_time": "", "destroyed": false, "version": 1}}),
            serde_json::Value::Null,
        )))
        .mount(&server)
        .await;
    for verb in ["POST", "DELETE"] {
        Mock::given(method(verb))
            .and(path("/v1/secret/data/weebo-example/team-a/ci"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method(verb))
            .and(path("/v1/secret/metadata/weebo-example/team-a/ci"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
    }
    let store = RoutingSecretStore::new(kubernetes(&server), Ok(vault), SecretStoreBackend::Vault);
    let target = vault_target("ci", None);
    let data = BTreeMap::from([("k".to_string(), "mine".to_string())]);
    let err = store.write(&target, &OWNER, &data).await.unwrap_err();
    assert!(err.0.contains("Token/team-a/other"), "{}", err.0);
    assert!(!store.exists(&target, &OWNER).await.unwrap());
    store.delete(&target, &OWNER).await.unwrap();
}

#[tokio::test]
async fn its_own_value_is_deleted() {
    let server = MockServer::start().await;
    let vault = vault(&server).await;
    owned_by(&server, "weebo-example/team-a/ci", "Token/team-a/ci").await;
    Mock::given(method("DELETE"))
        .and(path("/v1/secret/metadata/weebo-example/team-a/ci"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let store = RoutingSecretStore::new(kubernetes(&server), Ok(vault), SecretStoreBackend::Vault);
    store
        .delete(&vault_target("ci", None), &OWNER)
        .await
        .unwrap();
}

#[tokio::test]
async fn vault_is_resolved_once_and_only_when_a_target_needs_it() {
    let server = MockServer::start().await;
    let vault = vault(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/team-a/secrets/ci"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "reason": "NotFound", "code": 404, "message": "not found",
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/weebo-example/team-a/ci"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"errors": []})))
        .mount(&server)
        .await;
    let resolved = Arc::new(AtomicUsize::new(0));
    let resolver: VaultResolver = Box::new({
        let resolved = resolved.clone();
        move || {
            resolved.fetch_add(1, Ordering::SeqCst);
            async move { Ok(vault) }.boxed()
        }
    });
    let store = RoutingSecretStore::lazy(
        kubernetes(&server),
        resolver,
        SecretStoreBackend::Kubernetes,
    );
    let kubernetes_target = SecretTarget {
        backend: SecretStoreBackend::Kubernetes,
        name: "ci".into(),
        namespace: None,
        path: None,
    };
    assert!(!store.exists(&kubernetes_target, &OWNER).await.unwrap());
    assert_eq!(resolved.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        assert!(
            !store
                .exists(&vault_target("ci", None), &OWNER)
                .await
                .unwrap()
        );
    }
    assert_eq!(resolved.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unavailable_vault_only_fails_vault_targets() {
    let server = MockServer::start().await;
    let store = RoutingSecretStore::new(
        kubernetes(&server),
        Err("Vault is not configured".into()),
        SecretStoreBackend::Kubernetes,
    );
    let err = store
        .exists(&vault_target("ci", None), &OWNER)
        .await
        .unwrap_err();
    assert!(err.0.contains("not configured"), "{}", err.0);
}

/// Every KV call the server received.
async fn kv_calls(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().starts_with("/v1/secret/"))
        .count()
}

/// An empty KV path `p`: no value, no metadata, writes accepted.
async fn free(server: &MockServer, p: &str) {
    for kv in ["data", "metadata"] {
        Mock::given(method("GET"))
            .and(path(format!("/v1/secret/{kv}/{p}")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"errors": []})))
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(format!("/v1/secret/data/{p}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"created_time": "2026-10-14T00:00:00Z", "custom_metadata": null,
                   "deletion_time": "", "destroyed": false, "version": 1}),
            serde_json::Value::Null,
        )))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/secret/metadata/{p}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(server)
        .await;
}

fn grant(namespaces: &[&str], paths: &[&str]) -> VaultPathGrant {
    VaultPathGrant {
        namespaces: namespaces.iter().map(|s| s.to_string()).collect(),
        cluster_scoped: false,
        paths: paths.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn a_grant_opens_a_path_outside_the_namespace_subtree() {
    let server = MockServer::start().await;
    free(&server, "batlehub/auth").await;
    let vault = vault_with(
        &server,
        "weebo-example",
        vec![grant(&["team-*"], &["+/auth"])],
    )
    .await;
    let store = RoutingSecretStore::new(kubernetes(&server), Ok(vault), SecretStoreBackend::Vault);
    let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
    store
        .write(&vault_target("ci", Some("/batlehub/auth")), &OWNER, &data)
        .await
        .unwrap();
    // Only what the grant covers.
    let err = store
        .write(&vault_target("ci", Some("batlehub/auth/x")), &OWNER, &data)
        .await
        .unwrap_err();
    assert!(err.0.contains("no pathGrants entry"), "{}", err.0);
}

#[tokio::test]
async fn a_grant_never_hands_over_another_owners_value() {
    let server = MockServer::start().await;
    owned_by(&server, "batlehub/auth", "Token/team-b/other").await;
    let vault = vault_with(
        &server,
        "weebo-example",
        vec![grant(&["team-a"], &["batlehub/auth"])],
    )
    .await;
    let store = RoutingSecretStore::new(kubernetes(&server), Ok(vault), SecretStoreBackend::Vault);
    let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
    let err = store
        .write(&vault_target("ci", Some("batlehub/auth")), &OWNER, &data)
        .await
        .unwrap_err();
    assert!(err.0.contains("Token/team-b/other"), "{}", err.0);
    let writes = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path().starts_with("/v1/secret/"))
        .count();
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn an_empty_prefix_puts_default_paths_under_the_namespace() {
    let server = MockServer::start().await;
    free(&server, "team-a/ci").await;
    let vault = vault_with(&server, "", vec![]).await;
    assert_eq!(vault.default_path("team-a", "ci"), "team-a/ci");
    let store = RoutingSecretStore::new(kubernetes(&server), Ok(vault), SecretStoreBackend::Vault);
    let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
    store
        .write(&vault_target("ci", None), &OWNER, &data)
        .await
        .unwrap();
    // Explicit paths stay under `<namespace>/`.
    let err = store
        .write(&vault_target("ci", Some("team-b/ci")), &OWNER, &data)
        .await
        .unwrap_err();
    assert!(err.0.contains("outside \"team-a/\""), "{}", err.0);
}

#[tokio::test]
async fn a_cluster_scoped_owner_needs_its_namespace_subtree_or_a_cluster_grant() {
    let server = MockServer::start().await;
    let cluster_owner = SecretOwner::cluster("Token", "ci");
    let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
    let refused = vault_with(
        &server,
        "weebo-example",
        vec![grant(&["*"], &["platform/+"])],
    )
    .await;
    let store =
        RoutingSecretStore::new(kubernetes(&server), Ok(refused), SecretStoreBackend::Vault);
    for (namespace, p) in [
        (None, "platform/ci"),
        (Some("team-a"), "weebo-example/team-b/ci"),
    ] {
        let target = SecretTarget {
            namespace: namespace.map(String::from),
            ..vault_target("ci", Some(p))
        };
        let err = store
            .write(&target, &cluster_owner, &data)
            .await
            .unwrap_err();
        assert!(err.0.contains("cluster-scoped objects"), "{p}: {}", err.0);
    }
    assert_eq!(kv_calls(&server).await, 0);

    free(&server, "platform/ci").await;
    let granted = vault_with(
        &server,
        "weebo-example",
        vec![VaultPathGrant {
            namespaces: vec![],
            cluster_scoped: true,
            paths: vec!["platform/+".into()],
        }],
    )
    .await;
    let store =
        RoutingSecretStore::new(kubernetes(&server), Ok(granted), SecretStoreBackend::Vault);
    store
        .write(
            &vault_target("ci", Some("platform/ci")),
            &cluster_owner,
            &data,
        )
        .await
        .unwrap();
}
