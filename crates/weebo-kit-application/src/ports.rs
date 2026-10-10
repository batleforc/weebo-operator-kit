//! Secret-store ports: where the values an operator emits (generated
//! passwords and keys, OAuth2 credentials, tokens) are written.

use std::collections::BTreeMap;
use std::sync::Arc;

use weebo_kit_api::{SecretStoreBackend, SecretTarget};

#[derive(Debug, Clone, thiserror::Error)]
#[error("secret store: {0}")]
pub struct SecretStoreError(pub String);

/// The CR emitting a secret. Stores record [`SecretOwner::id`] on every
/// value they write and refuse to overwrite or delete a value recorded for
/// another owner — or, for Kubernetes, a `Secret` they didn't create: a CR
/// must never be able to take over a credential it doesn't own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretOwner<'a> {
    /// The CR's kind (e.g. `ForgejoAccessToken`).
    pub kind: &'a str,
    /// The CR's namespace, `None` for cluster-scoped CRs (whose targets
    /// must then name one). A namespaced CR may only write into its own
    /// namespace ([`crate::emit::check_target`]).
    pub namespace: Option<&'a str>,
    pub name: &'a str,
}

impl<'a> SecretOwner<'a> {
    pub fn namespaced(kind: &'a str, namespace: &'a str, name: &'a str) -> Self {
        Self {
            kind,
            namespace: Some(namespace),
            name,
        }
    }

    pub fn cluster(kind: &'a str, name: &'a str) -> Self {
        Self {
            kind,
            namespace: None,
            name,
        }
    }

    /// `<kind>/<namespace>/<name>`, or `<kind>/<name>` when cluster-scoped.
    /// Deliberately not the CR's UID: a CR deleted with `Orphan` and
    /// re-created under the same name takes its secrets back.
    pub fn id(&self) -> String {
        match self.namespace {
            Some(ns) => format!("{}/{ns}/{}", self.kind, self.name),
            None => format!("{}/{}", self.kind, self.name),
        }
    }
}

/// Writes emitted secrets to each target's backend.
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    /// Backend of the implicit target used when a CR lists none (the
    /// instance's `spec.secretStore.backend`).
    fn default_backend(&self) -> SecretStoreBackend;
    /// Fails when the target holds a value of another owner.
    async fn write(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError>;
    /// `true` only when the target holds a value of `owner`: anything else
    /// is a lost value to re-emit (and the write then reports the clash).
    async fn exists(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<bool, SecretStoreError>;
    /// Deletes the target's value only when `owner` wrote it; anything else
    /// is left alone. Idempotent.
    async fn delete(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(), SecretStoreError>;
}

/// Produces the `SecretStore` of a CR's `instanceRef` (`None` → the
/// cluster's only instance): Kubernetes always, Vault when the instance
/// configures it.
#[async_trait::async_trait]
pub trait SecretStoreFactory: Send + Sync {
    async fn store_for(
        &self,
        instance_ref: Option<&str>,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreError>;
}
