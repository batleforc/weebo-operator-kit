//! KV v2 store authenticated through Vault's Kubernetes auth (the
//! operator's own ServiceAccount JWT — no static Vault token). The login
//! is cached for 80% of its lease by [`VaultLogins`], and a KV call
//! rejected as unauthorized re-logs-in once in place.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};
use vaultrs::api::kv2::requests::SetSecretMetadataRequest;
use vaultrs::client::{Client as _, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;
use vaultrs::{auth::kubernetes, kv2};
use weebo_kit_application::ports::SecretStoreError;

/// The operator's projected ServiceAccount token, exchanged for a Vault
/// token. Overridable with `VAULT_KUBERNETES_JWT_PATH`.
const DEFAULT_JWT_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

/// A Vault connection, as an operator's instance spec describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultConfig {
    pub address: String,
    /// KV v2 mount.
    pub mount: String,
    /// Default paths are `<path_prefix>/<namespace>/<name>`.
    pub path_prefix: String,
    pub kubernetes_auth_role: String,
    pub kubernetes_auth_mount: String,
    /// Identifies the CA bundle (e.g. `namespace/name/key` of its `Secret`
    /// plus a digest of its content), `None` for the system trust store.
    /// Only compared: a change forces a new login with the new CA, so it
    /// must change when the bundle's content does.
    pub ca_source: Option<String>,
}

/// Per-request budget for every Vault call, login included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct VaultSecretStore {
    /// Write-locked only to swap in a re-logged-in token.
    client: RwLock<VaultClient>,
    mount: String,
    path_prefix: String,
    kubernetes_auth_mount: String,
    kubernetes_auth_role: String,
    jwt: String,
    cache_ttl: Option<Duration>,
}

impl VaultSecretStore {
    /// `ca_pem`: PEM bundle from `spec.caSecretRef`, read by the caller.
    pub async fn new(
        spec: &VaultConfig,
        jwt: &str,
        ca_pem: Option<&[u8]>,
    ) -> Result<Self, SecretStoreError> {
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
            kubernetes_auth_mount: spec.kubernetes_auth_mount.clone(),
            kubernetes_auth_role: spec.kubernetes_auth_role.clone(),
            jwt: jwt.trim().to_string(),
            cache_ttl: cache_ttl_from_lease(auth.lease_duration, auth.renewable),
        })
    }

    /// How long the factory may reuse this store; `None` = don't cache.
    pub fn cache_ttl(&self) -> Option<Duration> {
        self.cache_ttl
    }

    /// `<pathPrefix>/<namespace>/<name>` — one secret per emitting CR, the
    /// same convention as the Kubernetes backend's naming.
    pub fn default_path(&self, namespace: &str, name: &str) -> String {
        format!("{}/{namespace}/{name}", self.path_prefix)
    }

    /// The login runs under a read guard (it doesn't use the token); the
    /// write lock is only held for the swap, never across a network call.
    async fn relogin(&self) -> Result<(), SecretStoreError> {
        let auth = {
            let client = self.client.read().await;
            kubernetes::login(
                &*client,
                &self.kubernetes_auth_mount,
                &self.kubernetes_auth_role,
                &self.jwt,
            )
            .await
            .map_err(|e| SecretStoreError(format!("Vault re-login: {e}")))?
        };
        self.client.write().await.set_token(&auth.client_token);
        Ok(())
    }

    /// The KV v2 custom-metadata key naming the emitting CR.
    pub const OWNER_METADATA_KEY: &'static str = "owner";

    /// `<pathPrefix>/<namespace>/`: the only subtree a namespaced CR may
    /// write to.
    pub fn namespace_prefix(&self, namespace: &str) -> String {
        format!("{}/{namespace}/", self.path_prefix)
    }

    /// The owner id recorded on `path` (`Ok(None)`: nothing stored, or
    /// stored before owners were recorded).
    pub async fn owner_of(&self, path: &str) -> Result<Option<String>, SecretStoreError> {
        let first = {
            let client = self.client.read().await;
            kv2::read_metadata(&*client, &self.mount, path).await
        };
        let result = match first {
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::read_metadata(&*client, &self.mount, path).await
            }
            other => other,
        };
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
        let result = match set().await {
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                set().await
            }
            other => other,
        };
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
        // Each guard is scoped to its block: `relogin` takes the write lock.
        let first = {
            let client = self.client.read().await;
            kv2::set(&*client, &self.mount, path, data).await
        };
        match first {
            Ok(_) => Ok(()),
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::set(&*client, &self.mount, path, data)
                    .await
                    .map(|_| ())
                    .map_err(|e| SecretStoreError(format!("Vault write {path}: {e}")))
            }
            Err(e) => Err(SecretStoreError(format!("Vault write {path}: {e}"))),
        }
    }

    /// `Ok(None)` when nothing is stored at `path`.
    pub async fn read_path(
        &self,
        path: &str,
    ) -> Result<Option<BTreeMap<String, String>>, SecretStoreError> {
        let first = {
            let client = self.client.read().await;
            kv2::read::<BTreeMap<String, String>>(&*client, &self.mount, path).await
        };
        let result = match first {
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::read::<BTreeMap<String, String>>(&*client, &self.mount, path).await
            }
            other => other,
        };
        match result {
            Ok(data) => Ok(Some(data)),
            Err(ClientError::APIError { code: 404, .. }) => Ok(None),
            Err(e) => Err(SecretStoreError(format!("Vault read {path}: {e}"))),
        }
    }

    /// Deletes every version and the metadata — not a recoverable soft
    /// delete. Idempotent.
    pub async fn delete_path(&self, path: &str) -> Result<(), SecretStoreError> {
        let first = {
            let client = self.client.read().await;
            kv2::delete_metadata(&*client, &self.mount, path).await
        };
        let result = match first {
            Err(e) if is_auth_error(&e) => {
                self.relogin().await?;
                let client = self.client.read().await;
                kv2::delete_metadata(&*client, &self.mount, path).await
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
            .map(Arc::new)
            .map_err(|e| e.0)
    }
}

/// 80% of the login lease; a zero lease or non-renewable token isn't
/// cached at all.
fn cache_ttl_from_lease(lease_duration: u64, renewable: bool) -> Option<Duration> {
    (lease_duration > 0 && renewable).then(|| Duration::from_secs(lease_duration * 4 / 5))
}

/// 401/403: the token is no good (expired or revoked).
fn is_auth_error(err: &ClientError) -> bool {
    matches!(err, ClientError::APIError { code, .. } if *code == 401 || *code == 403)
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
