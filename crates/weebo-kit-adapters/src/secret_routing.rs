//! The `SecretStore` of one instance: every target goes to its own
//! backend — the Kubernetes API, or the instance's Vault connection.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::sync::OnceCell;
use weebo_kit_api::{SecretStoreBackend, SecretTarget};
use weebo_kit_application::emit::check_target;
use weebo_kit_application::ports::{SecretOwner, SecretStore, SecretStoreError};

use crate::secret_k8s::K8sSecretStore;
use crate::secret_vault::VaultSecretStore;

type VaultResult = Result<Arc<VaultSecretStore>, String>;

/// Resolves the instance's Vault store, on first use only.
pub type VaultResolver = Box<dyn FnOnce() -> BoxFuture<'static, VaultResult> + Send + Sync>;

pub struct RoutingSecretStore {
    kubernetes: Arc<K8sSecretStore>,
    vault: OnceCell<VaultResult>,
    resolve_vault: std::sync::Mutex<Option<VaultResolver>>,
    default_backend: SecretStoreBackend,
}

impl RoutingSecretStore {
    /// `vault`: `Err` holds why Vault is unavailable (not configured, login
    /// failed), reported only when a target actually needs it — a broken
    /// Vault never blocks Kubernetes targets.
    pub fn new(
        kubernetes: Arc<K8sSecretStore>,
        vault: VaultResult,
        default_backend: SecretStoreBackend,
    ) -> Self {
        Self {
            kubernetes,
            vault: OnceCell::new_with(Some(vault)),
            resolve_vault: std::sync::Mutex::new(None),
            default_backend,
        }
    }

    /// Like [`Self::new`], but Vault is only resolved (logged in to) when
    /// a target first needs it: a CR with Kubernetes targets only never
    /// waits on Vault.
    pub fn lazy(
        kubernetes: Arc<K8sSecretStore>,
        resolve_vault: VaultResolver,
        default_backend: SecretStoreBackend,
    ) -> Self {
        Self {
            kubernetes,
            vault: OnceCell::new(),
            resolve_vault: std::sync::Mutex::new(Some(resolve_vault)),
            default_backend,
        }
    }

    async fn vault(&self) -> Result<&VaultSecretStore, SecretStoreError> {
        self.vault
            .get_or_init(|| async {
                let resolve = self
                    .resolve_vault
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                match resolve {
                    Some(resolve) => resolve().await,
                    None => Err("Vault is not configured".to_string()),
                }
            })
            .await
            .as_deref()
            .map_err(|why| SecretStoreError(why.clone()))
    }

    /// The target's KV path. A namespaced owner stays under its own
    /// `<pathPrefix>/<namespace>/`, explicit `path` included.
    fn vault_path(
        vault: &VaultSecretStore,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<String, SecretStoreError> {
        check_target(target, owner).map_err(|e| SecretStoreError(e.to_string()))?;
        if let Some(path) = &target.path {
            if let Some(ns) = owner.namespace {
                let prefix = vault.namespace_prefix(ns);
                let normalized = path.trim_start_matches('/');
                if !normalized.starts_with(&prefix) || normalized.split('/').any(|s| s == "..") {
                    return Err(SecretStoreError(format!(
                        "Vault path {path:?} of target {:?} is outside {prefix:?}, \
                         the only subtree a CR in namespace {ns:?} may write to",
                        target.name
                    )));
                }
            }
            return Ok(path.clone());
        }
        let ns = target
            .namespace
            .as_deref()
            .or(owner.namespace)
            .ok_or_else(|| {
                SecretStoreError(format!(
                    "Vault target {:?} needs a path or a namespace",
                    target.name
                ))
            })?;
        Ok(vault.default_path(ns, &target.name))
    }

    /// `Ok(true)` when `path` is free or `owner`'s (values written before
    /// owners were recorded count as the writer's).
    async fn vault_owned(
        vault: &VaultSecretStore,
        path: &str,
        owner: &SecretOwner<'_>,
    ) -> Result<Result<(), String>, SecretStoreError> {
        Ok(match vault.owner_of(path).await? {
            Some(other) if other != owner.id() => Err(other),
            _ => Ok(()),
        })
    }
}

#[async_trait::async_trait]
impl SecretStore for RoutingSecretStore {
    fn default_backend(&self) -> SecretStoreBackend {
        self.default_backend
    }

    async fn write(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        match target.backend {
            SecretStoreBackend::Kubernetes => self.kubernetes.write(target, owner, data).await,
            SecretStoreBackend::Vault => {
                let vault = self.vault().await?;
                let path = Self::vault_path(vault, target, owner)?;
                if let Err(other) = Self::vault_owned(vault, &path, owner).await? {
                    return Err(SecretStoreError(format!(
                        "Vault path {path} belongs to {other}, not {}",
                        owner.id()
                    )));
                }
                vault.write_path(&path, data).await?;
                vault.set_owner(&path, &owner.id()).await
            }
        }
    }

    async fn exists(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<bool, SecretStoreError> {
        match target.backend {
            SecretStoreBackend::Kubernetes => self.kubernetes.exists(target, owner).await,
            SecretStoreBackend::Vault => {
                let vault = self.vault().await?;
                let path = Self::vault_path(vault, target, owner)?;
                Ok(vault.read_path(&path).await?.is_some()
                    && Self::vault_owned(vault, &path, owner).await?.is_ok())
            }
        }
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(), SecretStoreError> {
        match target.backend {
            SecretStoreBackend::Kubernetes => self.kubernetes.delete(target, owner).await,
            SecretStoreBackend::Vault => {
                let vault = self.vault().await?;
                let path = Self::vault_path(vault, target, owner)?;
                if let Err(other) = Self::vault_owned(vault, &path, owner).await? {
                    tracing::info!(%path, owner = %other, "leaving a Vault value this CR does not own");
                    return Ok(());
                }
                vault.delete_path(&path).await
            }
        }
    }
}
