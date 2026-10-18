//! Secret-store ports: where the values an operator emits (generated
//! passwords and keys, OAuth2 credentials, tokens) are written.

use std::collections::BTreeMap;
use std::sync::Arc;

use weebo_kit_api::{BoundInstance, SecretStoreBackend, SecretTarget};

#[derive(Debug, Clone, thiserror::Error)]
#[error("secret store: {0}")]
pub struct SecretStoreError(pub String);

/// A Kubernetes read made for the use-case (a referenced CR, a policy, a
/// `Secret`) failed — usually transient, retried like any error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Kubernetes API error: {0}")]
pub struct ClusterError(pub String);

/// The CR emitting a secret. Stores record [`SecretOwner::id`] on every
/// value they write and refuse to overwrite or delete a value recorded for
/// another owner — or, for Kubernetes, a `Secret` they didn't create: a CR
/// must never be able to take over a credential it doesn't own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretOwner<'a> {
    /// The CR's kind (e.g. `ExampleAccessToken`).
    pub kind: &'a str,
    /// The CR's namespace, `None` for cluster-scoped CRs (whose targets
    /// must then name one). A namespaced CR may only write into its own
    /// namespace ([`crate::emit::check_target`]).
    pub namespace: Option<&'a str>,
    pub name: &'a str,
    /// The id this CR wrote its values under before (a CR moved between
    /// scopes: `Kind/name` → `Kind/<namespace>/name`). A value recorded for
    /// it is this CR's, and is rewritten under [`Self::id`] — only when it
    /// names the same kind and name (the scope alone may differ): it comes
    /// from an annotation anyone editing the CR can set, and must not hand
    /// over another CR's values.
    pub formerly: Option<&'a str>,
}

impl<'a> SecretOwner<'a> {
    pub fn namespaced(kind: &'a str, namespace: &'a str, name: &'a str) -> Self {
        Self {
            kind,
            namespace: Some(namespace),
            name,
            formerly: None,
        }
    }

    pub fn cluster(kind: &'a str, name: &'a str) -> Self {
        Self {
            kind,
            namespace: None,
            name,
            formerly: None,
        }
    }

    /// The id it wrote its values under before, if any (see
    /// [`Self::formerly`]).
    pub fn formerly(mut self, id: Option<&'a str>) -> Self {
        self.formerly = id;
        self
    }

    /// Whether a value recorded for `recorded` is this owner's.
    pub fn is(&self, recorded: &str) -> bool {
        recorded == self.id() || (self.formerly == Some(recorded) && self.was(recorded))
    }

    /// `id` is this CR's kind and name, under any scope (`Kind/name` or
    /// `Kind/<namespace>/name`).
    fn was(&self, id: &str) -> bool {
        let mut parts = id.split('/');
        let (Some(kind), Some(name)) = (parts.next(), parts.next_back()) else {
            return false;
        };
        kind == self.kind && name == self.name && parts.count() <= 1
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
    /// Fails when the target holds a value of another owner. A target
    /// already holding `data` for `owner` is left untouched — no new
    /// `resourceVersion`, no new KV version: callers may write on every
    /// reconcile, a credential is only rewritten when it changed.
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

/// The namespace policies — one rule list per policy object, the rules as
/// the operator's policy CRD spells them (`R`) — for the admission
/// webhook and the reconcilers' own re-check. An empty list: no policy
/// exists, everything is allowed.
#[async_trait::async_trait]
pub trait PolicySource<R>: Send + Sync {
    async fn policies(&self) -> Result<Vec<Vec<R>>, ClusterError>;
}

/// Produces the `SecretStore` of the instance a CR is bound to (resolved
/// first, `crate::instances`): Kubernetes always, Vault when the instance
/// configures it. A local instance's Vault paths stay under
/// `<pathPrefix>/<its namespace>/`: the operator's Vault identity is
/// shared by every namespace.
#[async_trait::async_trait]
pub trait SecretStoreFactory: Send + Sync {
    async fn store_for(
        &self,
        instance: &BoundInstance,
    ) -> Result<Arc<dyn SecretStore>, SecretStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_former_id_only_counts_for_the_same_kind_and_name() {
        let owner = SecretOwner::namespaced("Token", "team-a", "ci");
        assert!(owner.is("Token/team-a/ci"));
        assert!(!owner.is("Token/ci"));
        assert!(owner.formerly(Some("Token/ci")).is("Token/ci"));
        assert!(
            owner
                .formerly(Some("Token/team-b/ci"))
                .is("Token/team-b/ci")
        );
        // Another CR's id, set through the annotation: refused.
        assert!(
            !owner
                .formerly(Some("Token/team-a/other"))
                .is("Token/team-a/other")
        );
        assert!(!owner.formerly(Some("Key/ci")).is("Key/ci"));
        assert!(!owner.formerly(Some("Token/x/y/ci")).is("Token/x/y/ci"));
        assert!(!owner.formerly(Some("Token")).is("Token"));
    }
}
