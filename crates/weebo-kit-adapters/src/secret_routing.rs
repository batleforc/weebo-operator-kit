//! The `SecretStore` of one instance: every target goes to its own
//! backend — the Kubernetes API, or the instance's Vault connection.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::sync::OnceCell;
use weebo_kit_api::{SecretStoreBackend, SecretTarget};
use weebo_kit_application::emit::check_target;
use weebo_kit_application::ports::{SecretOwner, SecretStore, SecretStoreError};
use weebo_kit_domain::vault_paths::Writer;

use crate::secret_k8s::K8sSecretStore;
use crate::secret_vault::{CasWrite, PathMetadata, VaultSecretStore};

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

    /// The target's KV path, normalized. An explicit `path` stays under
    /// `<pathPrefix>/<namespace>/` (the owner's namespace, or the one a
    /// cluster-scoped owner's target names) unless the instance's grants
    /// open it to the owner.
    fn vault_path(
        vault: &VaultSecretStore,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<String, SecretStoreError> {
        check_target(target, owner).map_err(|e| SecretStoreError(e.to_string()))?;
        let ns = target.namespace.as_deref().or(owner.namespace);
        if let Some(path) = &target.path {
            let writer = match owner.namespace {
                Some(ns) => Writer::Namespaced(ns),
                None => Writer::Cluster,
            };
            return vault
                .check_path(writer, ns, path)
                .map(String::from)
                .map_err(|e| SecretStoreError(format!("target {:?}: {e}", target.name)));
        }
        let ns = ns.ok_or_else(|| {
            SecretStoreError(format!(
                "Vault target {:?} needs a path or a namespace",
                target.name
            ))
        })?;
        Ok(vault.default_path(ns, &target.name))
    }

    /// `Ok(true)` when `path` is free or `owner`'s — under its id or the
    /// one it had before (values written before owners were recorded count
    /// as the writer's).
    async fn vault_owned(
        vault: &VaultSecretStore,
        path: &str,
        owner: &SecretOwner<'_>,
    ) -> Result<Result<(), String>, SecretStoreError> {
        Ok(match vault.owner_of(path).await? {
            Some(other) if !owner.is(&other) => Err(other),
            _ => Ok(()),
        })
    }
}

impl RoutingSecretStore {
    /// Writes `data` to `path` for `owner`, the owner recorded with it.
    /// Several objects may target one path (a grant opens it to a whole
    /// namespace pattern), and three Vault calls (read the owner, write,
    /// record the owner) are not atomic; so:
    ///
    /// 1. a path owned by another object is refused;
    /// 2. a path not yet `owner`'s (free, or holding a value written before
    ///    owners were recorded: adopted) is claimed first — the owner
    ///    recorded, then read back: of two concurrent claims, the one read
    ///    back wins, the other is refused like in 1;
    /// 3. the value is written with check-and-set on the version read with
    ///    the owner (`0` on a new path): a writer that raced past 2 loses
    ///    here, reads the owner again and is refused (or, owner unchanged,
    ///    gets a conflict error retried on its next reconcile);
    /// 4. after the write, the owner is read once more: a claim taken over
    ///    in the meantime is reported as the refusal of 1 (the new owner's
    ///    next write replaces the value).
    ///
    /// Without `read` on `<mount>/metadata/*` ownership isn't checked: the
    /// value is written as is, the owner recorded best effort.
    async fn vault_write(
        vault: &VaultSecretStore,
        path: &str,
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        let id = owner.id();
        let refused = |other: &str| {
            SecretStoreError(format!("Vault path {path} belongs to {other}, not {id}"))
        };
        let version = match vault.metadata(path).await? {
            // The owner can't be read back: recorded with each new
            // version only, an unchanged value writes nothing at all.
            PathMetadata::Unreadable => {
                if vault.write_path(path, data).await? {
                    vault.set_owner(path, &id).await?;
                }
                return Ok(());
            }
            PathMetadata::Present {
                owner: Some(other), ..
            } if !owner.is(&other) => return Err(refused(&other)),
            PathMetadata::Present {
                owner: Some(recorded),
                current_version,
            } if recorded == id => current_version,
            claimed => {
                let seen = match claimed {
                    PathMetadata::Present {
                        current_version, ..
                    } => current_version,
                    _ => 0,
                };
                vault.set_owner(path, &id).await?;
                match vault.metadata(path).await? {
                    PathMetadata::Present {
                        owner: Some(other), ..
                    } if !owner.is(&other) => return Err(refused(&other)),
                    PathMetadata::Present {
                        current_version, ..
                    } => current_version,
                    // The claim isn't readable back (no `create` on the
                    // metadata, a metadata-only entry not kept): the
                    // version read first.
                    PathMetadata::Absent | PathMetadata::Unreadable => seen,
                }
            }
        };
        let taken_over = |recorded: Option<String>| match recorded {
            Some(other) if !owner.is(&other) => Some(other),
            _ => None,
        };
        match vault.write_path_cas(path, data, version).await? {
            CasWrite::Unchanged => Ok(()),
            CasWrite::Written => match taken_over(vault.owner_of(path).await?) {
                Some(other) => Err(refused(&other)),
                None => Ok(()),
            },
            CasWrite::Conflict => match taken_over(vault.owner_of(path).await?) {
                Some(other) => Err(refused(&other)),
                None => Err(SecretStoreError(format!(
                    "Vault path {path} was written by someone else while {id} wrote it; retrying"
                ))),
            },
        }
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
                Self::vault_write(vault, &path, owner, data).await
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
