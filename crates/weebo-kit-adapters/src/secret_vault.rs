//! KV v2 store authenticated through Vault's Kubernetes auth (the
//! operator's own ServiceAccount JWT — no static Vault token). The login
//! is cached for 80% of its lease by [`VaultLogins`], and a KV call
//! rejected because the token is no longer valid re-logs-in once in place
//! (with the ServiceAccount token read again: a projected one rotates). A
//! call the policy denies to a valid token never re-logs-in.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};
use vaultrs::api::kv2::requests::SetSecretMetadataRequest;
use vaultrs::client::{Client as _, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;
use vaultrs::{auth::kubernetes, kv2, token};
use weebo_kit_api::{SecretKeyRef, VaultPathGrant};
use weebo_kit_application::ports::SecretStoreError;
use weebo_kit_domain::vault_paths::{self, PathGrant};

/// The operator's projected ServiceAccount token, exchanged for a Vault
/// token. Overridable with `VAULT_KUBERNETES_JWT_PATH`.
const DEFAULT_JWT_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// A Vault connection, as an operator's instance spec describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultConfig {
    pub address: String,
    /// KV v2 mount.
    pub mount: String,
    /// Default paths are `<path_prefix>/<namespace>/<name>` (`<namespace>/<name>`
    /// when empty).
    pub path_prefix: String,
    /// The paths open to objects outside their namespace's subtree.
    pub path_grants: Vec<VaultPathGrant>,
    pub kubernetes_auth_role: String,
    pub kubernetes_auth_mount: String,
    /// Identifies the CA bundle (e.g. `namespace/name/key` of its `Secret`
    /// plus a digest of its content), `None` for the system trust store.
    /// Only compared: a change forces a new login with the new CA, so it
    /// must change when the bundle's content does.
    pub ca_source: Option<String>,
}

impl VaultConfig {
    /// The [`ca_source`](Self::ca_source) of a CA bundle read from
    /// `secret_ref`: `namespace/name/key@sha256:<digest of pem>`.
    pub fn ca_source_of(secret_ref: &SecretKeyRef, pem: &[u8]) -> String {
        format!(
            "{}/{}/{}@sha256:{:x}",
            secret_ref.namespace,
            secret_ref.name,
            secret_ref.key,
            Sha256::digest(pem)
        )
    }
}

/// Per-request budget for every Vault call, login included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct VaultSecretStore {
    /// Write-locked only to swap in a re-logged-in token.
    client: RwLock<VaultClient>,
    mount: String,
    path_prefix: String,
    path_grants: Vec<VaultPathGrant>,
    kubernetes_auth_mount: String,
    kubernetes_auth_role: String,
    jwt: String,
    /// Read again on every re-login when set: a projected ServiceAccount
    /// token is rotated by the kubelet, the one read at login may be
    /// expired by then.
    jwt_path: Option<std::path::PathBuf>,
    cache_ttl: Option<Duration>,
}

impl VaultSecretStore {
    /// `ca_pem`: PEM bundle from `spec.caSecretRef`, read by the caller.
    /// Invalid `path_grants` fail before any login (the CRD schema refuses
    /// them already; this is the net for an older schema).
    pub async fn new(
        spec: &VaultConfig,
        jwt: &str,
        ca_pem: Option<&[u8]>,
    ) -> Result<Self, SecretStoreError> {
        vault_paths::validate(&path_grants(&spec.path_grants))
            .map_err(|e| SecretStoreError(format!("Vault connection: {e}")))?;
        let mut builder = VaultClientSettingsBuilder::default();
        builder.address(&spec.address);
        // vaultrs has no default timeout: a black-holed Vault would hang
        // every secret write sharing this (cached) store.
        builder.timeout(Some(REQUEST_TIMEOUT));
        // vaultrs only reads CA certs from file paths: the PEM lives in a
        // temp file just long enough for `VaultClient::new` to load it.
        let _ca_file = match ca_pem {
            Some(pem) => {
                let guard = TempCaFile::write(pem)?;
                builder.ca_certs(vec![guard.path_string()]);
                Some(guard)
            }
            None => None,
        };
        let settings = builder
            .build()
            .map_err(|e| SecretStoreError(format!("Vault client settings: {e}")))?;
        let mut client = VaultClient::new(settings)
            .map_err(|e| SecretStoreError(format!("Vault client: {e}")))?;

        let auth = kubernetes::login(
            &client,
            &spec.kubernetes_auth_mount,
            &spec.kubernetes_auth_role,
            jwt.trim(),
        )
        .await
        .map_err(|e| {
            SecretStoreError(format!(
                "Vault Kubernetes auth login (mount {:?}, role {:?}): {e}",
                spec.kubernetes_auth_mount, spec.kubernetes_auth_role
            ))
        })?;
        client.set_token(&auth.client_token);

        Ok(Self {
            client: RwLock::new(client),
            mount: spec.mount.clone(),
            path_prefix: spec.path_prefix.clone(),
            path_grants: spec.path_grants.clone(),
            kubernetes_auth_mount: spec.kubernetes_auth_mount.clone(),
            kubernetes_auth_role: spec.kubernetes_auth_role.clone(),
            jwt: jwt.trim().to_string(),
            jwt_path: None,
            cache_ttl: cache_ttl_from_lease(auth.lease_duration, auth.renewable),
        })
    }

    /// Re-logins read the ServiceAccount token from `path` again (falling
    /// back on the one given to [`Self::new`] when it can't be read).
    pub fn with_jwt_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.jwt_path = Some(path.into());
        self
    }

    /// How long the factory may reuse this store; `None` = don't cache.
    pub fn cache_ttl(&self) -> Option<Duration> {
        self.cache_ttl
    }

    /// `<pathPrefix>/<namespace>/<name>` — one secret per emitting CR, the
    /// same convention as the Kubernetes backend's naming.
    pub fn default_path(&self, namespace: &str, name: &str) -> String {
        vault_paths::default_path(&self.path_prefix, namespace, name)
    }

    /// The normalized `path` when `writer` may write it: under
    /// `<pathPrefix>/<namespace>/`, or opened by a grant
    /// ([`vault_paths::check`]).
    pub fn check_path<'p>(
        &self,
        writer: vault_paths::Writer<'_>,
        namespace: Option<&str>,
        path: &'p str,
    ) -> Result<&'p str, vault_paths::PathDenied> {
        let grants = path_grants(&self.path_grants);
        vault_paths::check(&self.path_prefix, &grants, writer, namespace, path)
    }

    /// The login runs under a read guard (it doesn't use the token); the
    /// write lock is only held for the swap, never across a network call.
    async fn relogin(&self) -> Result<(), SecretStoreError> {
        let fresh = self
            .jwt_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok());
        let jwt = fresh.as_deref().map_or(self.jwt.as_str(), str::trim);
        let auth = {
            let client = self.client.read().await;
            kubernetes::login(
                &*client,
                &self.kubernetes_auth_mount,
                &self.kubernetes_auth_role,
                jwt,
            )
            .await
            .map_err(|e| SecretStoreError(format!("Vault re-login: {e}")))?
        };
        self.client.write().await.set_token(&auth.client_token);
        Ok(())
    }

    /// Whether `err` comes from a token that is no longer valid. Vault
    /// answers 403 both to an expired or revoked token and to a valid one
    /// its policy denies: a 403 is only a stale token when `lookup-self`
    /// (granted by Vault's default policy) fails too — a policy denial
    /// never mints a new token.
    async fn token_is_stale(&self, err: &ClientError) -> bool {
        match err {
            ClientError::APIError { code: 401, .. } => true,
            ClientError::APIError { code: 403, .. } => {
                let client = self.client.read().await;
                token::lookup_self(&*client)
                    .await
                    .is_err_and(|e| is_auth_error(&e))
            }
            _ => false,
        }
    }

    /// `op`, run again once after a re-login when its token was stale.
    async fn with_relogin<T, F, Fut>(
        &self,
        op: F,
    ) -> Result<Result<T, ClientError>, SecretStoreError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, ClientError>>,
    {
        match op().await {
            Err(e) if self.token_is_stale(&e).await => {
                self.relogin().await?;
                Ok(op().await)
            }
            other => Ok(other),
        }
    }

    /// The KV v2 custom-metadata key naming the emitting CR.
    pub const OWNER_METADATA_KEY: &'static str = "owner";

    /// `<pathPrefix>/<namespace>/`: the subtree a namespace's CRs write to
    /// without a grant.
    pub fn namespace_prefix(&self, namespace: &str) -> String {
        vault_paths::subtree(&self.path_prefix, namespace)
    }

    /// The owner id recorded on `path` (`Ok(None)`: nothing stored, or
    /// stored before owners were recorded).
    pub async fn owner_of(&self, path: &str) -> Result<Option<String>, SecretStoreError> {
        let result = self
            .with_relogin(|| async {
                let client = self.client.read().await;
                kv2::read_metadata(&*client, &self.mount, path).await
            })
            .await?;
        match result {
            Ok(meta) => Ok(meta
                .custom_metadata
                .and_then(|m| m.get(Self::OWNER_METADATA_KEY).cloned())),
            Err(ClientError::APIError { code: 404, .. }) => Ok(None),
            // A policy without `read` on `<mount>/metadata/*`: ownership
            // can't be checked, only the path confinement applies.
            Err(ClientError::APIError { code: 403, .. }) => {
                tracing::warn!(%path, "no read access to the KV metadata: secret ownership is not checked");
                Ok(None)
            }
            Err(e) => Err(SecretStoreError(format!("Vault read metadata {path}: {e}"))),
        }
    }

    /// Records `owner` on `path`'s metadata. Best effort: a policy without
    /// `create`/`update` on `<mount>/metadata/*` only loses the ownership
    /// record (warned), not the write.
    pub async fn set_owner(&self, path: &str, owner: &str) -> Result<(), SecretStoreError> {
        let set = || async {
            let mut opts = SetSecretMetadataRequest::builder();
            opts.custom_metadata(HashMap::from([(
                Self::OWNER_METADATA_KEY.to_string(),
                owner.to_string(),
            )]));
            let client = self.client.read().await;
            kv2::set_metadata(&*client, &self.mount, path, Some(&mut opts)).await
        };
        let result = self.with_relogin(set).await?;
        match result {
            Ok(()) => Ok(()),
            Err(ClientError::APIError { code: 403, .. }) => {
                tracing::warn!(%path, "no write access to the KV metadata: secret ownership is not recorded");
                Ok(())
            }
            Err(e) => Err(SecretStoreError(format!(
                "Vault write metadata {path}: {e}"
            ))),
        }
    }

    /// KV v2 `set` creates a new version even for identical data, so the
    /// current document is compared first: steady-state reconciles never
    /// churn the version history.
    pub async fn write_path(
        &self,
        path: &str,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        if self.read_path(path).await.ok().flatten().as_ref() == Some(data) {
            return Ok(());
        }
        // Each guard is scoped to its call: `relogin` takes the write lock.
        self.with_relogin(|| async {
            let client = self.client.read().await;
            kv2::set(&*client, &self.mount, path, data).await
        })
        .await?
        .map(|_| ())
        .map_err(|e| SecretStoreError(format!("Vault write {path}: {e}")))
    }

    /// `Ok(None)` when nothing is stored at `path`.
    pub async fn read_path(
        &self,
        path: &str,
    ) -> Result<Option<BTreeMap<String, String>>, SecretStoreError> {
        let result = self
            .with_relogin(|| async {
                let client = self.client.read().await;
                kv2::read::<BTreeMap<String, String>>(&*client, &self.mount, path).await
            })
            .await?;
        match result {
            Ok(data) => Ok(Some(data)),
            Err(ClientError::APIError { code: 404, .. }) => Ok(None),
            Err(e) => Err(SecretStoreError(format!("Vault read {path}: {e}"))),
        }
    }

    /// Deletes every version and the metadata — not a recoverable soft
    /// delete — given `delete` on `<mount>/metadata/*`. Without it, best
    /// effort: the current version is soft-deleted (`delete` on
    /// `<mount>/data/*`), the older ones stay in the history (warned).
    /// Idempotent.
    pub async fn delete_path(&self, path: &str) -> Result<(), SecretStoreError> {
        let result = self
            .with_relogin(|| async {
                let client = self.client.read().await;
                kv2::delete_metadata(&*client, &self.mount, path).await
            })
            .await?;
        let result = match result {
            Err(ClientError::APIError { code: 403, .. }) => {
                tracing::warn!(%path, "no delete access to the KV metadata: only the current version is deleted, older ones stay");
                let client = self.client.read().await;
                kv2::delete_latest(&*client, &self.mount, path).await
            }
            other => other,
        };
        match result {
            Ok(()) | Err(ClientError::APIError { code: 404, .. }) => Ok(()),
            Err(e) => Err(SecretStoreError(format!("Vault delete {path}: {e}"))),
        }
    }
}

enum Cached {
    Login {
        config: VaultConfig,
        store: Arc<VaultSecretStore>,
        expires: Instant,
    },
    /// A failed login, replayed until `expires` instead of hammering an
    /// unreachable or misconfigured Vault on every reconcile.
    Failed {
        config: VaultConfig,
        error: String,
        expires: Instant,
    },
}

/// How long a failed login is replayed before the next attempt.
pub const LOGIN_FAILURE_TTL: Duration = Duration::from_secs(15);

/// Vault logins cached per instance for 80% of their lease (each login
/// mints a Vault token that lives its whole TTL — logging in on every
/// reconcile churned tokens), and redone when the instance's Vault
/// settings change. Logins of different instances never wait for each
/// other: each instance has its own lock, held across its login only.
#[derive(Default)]
pub struct VaultLogins {
    slots: std::sync::Mutex<HashMap<String, Arc<Mutex<Option<Cached>>>>>,
}

impl VaultLogins {
    /// The store of instance `key`, logging in when none is cached for
    /// `config`. `load_ca` (the PEM of the configured CA, if any) only runs
    /// on a login.
    pub async fn get<F, Fut>(
        &self,
        key: &str,
        config: &VaultConfig,
        load_ca: F,
    ) -> Result<Arc<VaultSecretStore>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<Vec<u8>>, String>>,
    {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(key.to_string())
            .or_default()
            .clone();
        let mut cached = slot.lock().await;
        let now = Instant::now();
        match &*cached {
            Some(Cached::Login {
                config: c,
                store,
                expires,
            }) if c == config && *expires > now => return Ok(store.clone()),
            Some(Cached::Failed {
                config: c,
                error,
                expires,
            }) if c == config && *expires > now => return Err(error.clone()),
            _ => {}
        }

        match Self::login(config, load_ca).await {
            Ok(store) => {
                *cached = store.cache_ttl().map(|ttl| Cached::Login {
                    config: config.clone(),
                    store: store.clone(),
                    expires: Instant::now() + ttl,
                });
                Ok(store)
            }
            Err(error) => {
                *cached = Some(Cached::Failed {
                    config: config.clone(),
                    error: error.clone(),
                    expires: Instant::now() + LOGIN_FAILURE_TTL,
                });
                Err(error)
            }
        }
    }

    async fn login<F, Fut>(
        config: &VaultConfig,
        load_ca: F,
    ) -> Result<Arc<VaultSecretStore>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<Vec<u8>>, String>>,
    {
        let jwt_path =
            std::env::var("VAULT_KUBERNETES_JWT_PATH").unwrap_or_else(|_| DEFAULT_JWT_PATH.into());
        let jwt = std::fs::read_to_string(&jwt_path)
            .map_err(|e| format!("reading the ServiceAccount token {jwt_path}: {e}"))?;
        let ca = load_ca().await?;
        VaultSecretStore::new(config, &jwt, ca.as_deref())
            .await
            .map(|store| Arc::new(store.with_jwt_path(jwt_path)))
            .map_err(|e| e.0)
    }
}

/// 80% of the login lease; a zero lease or non-renewable token isn't
/// cached at all.
fn cache_ttl_from_lease(lease_duration: u64, renewable: bool) -> Option<Duration> {
    (lease_duration > 0 && renewable).then(|| Duration::from_secs(lease_duration * 4 / 5))
}

/// 401/403: the token is no good (expired or revoked), or — 403 — the
/// policy denies the call ([`VaultSecretStore::token_is_stale`] tells).
fn is_auth_error(err: &ClientError) -> bool {
    matches!(err, ClientError::APIError { code, .. } if *code == 401 || *code == 403)
}

/// The domain's view of the instance's grants.
fn path_grants(grants: &[VaultPathGrant]) -> Vec<PathGrant<'_>> {
    grants
        .iter()
        .map(|g| PathGrant {
            namespaces: &g.namespaces,
            cluster_scoped: g.cluster_scoped,
            paths: &g.paths,
        })
        .collect()
}

/// A CA PEM in a temp file for one `VaultClient::new`, removed on drop.
struct TempCaFile {
    path: std::path::PathBuf,
}

impl TempCaFile {
    fn write(pem: &[u8]) -> Result<Self, SecretStoreError> {
        let path =
            std::env::temp_dir().join(format!("weebo-vault-ca-{}.pem", uuid::Uuid::new_v4()));
        std::fs::write(&path, pem).map_err(|e| {
            SecretStoreError(format!("writing Vault CA to {}: {e}", path.display()))
        })?;
        Ok(Self { path })
    }

    fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TempCaFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_caching_rules() {
        assert_eq!(
            cache_ttl_from_lease(100, true),
            Some(Duration::from_secs(80))
        );
        assert_eq!(cache_ttl_from_lease(0, true), None);
        assert_eq!(cache_ttl_from_lease(100, false), None);
    }

    #[test]
    fn ca_source_changes_with_the_bundle() {
        let r = SecretKeyRef {
            name: "ca".into(),
            namespace: "vault".into(),
            key: "ca.crt".into(),
        };
        let a = VaultConfig::ca_source_of(&r, b"one");
        assert!(a.starts_with("vault/ca/ca.crt@sha256:"), "{a}");
        assert_ne!(a, VaultConfig::ca_source_of(&r, b"two"));
    }

    #[test]
    fn temp_ca_file_is_removed_on_drop() {
        let path = {
            let guard = TempCaFile::write(b"pem").unwrap();
            let path = std::path::PathBuf::from(guard.path_string());
            assert_eq!(std::fs::read(&path).unwrap(), b"pem");
            path
        };
        assert!(!path.exists());
    }
}
