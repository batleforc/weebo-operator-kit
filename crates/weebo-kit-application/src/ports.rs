//! Secret-store ports: where the values an operator emits (generated
//! passwords and keys, OAuth2 credentials, tokens) are written.

use std::collections::BTreeMap;
use std::sync::Arc;

use weebo_kit_api::{SecretStoreBackend, SecretTarget};

#[derive(Debug, Clone, thiserror::Error)]
#[error("secret store: {0}")]
pub struct SecretStoreError(pub String);

/// Writes emitted secrets to each target's backend. `default_namespace` is
/// the emitting CR's own namespace, `None` for cluster-scoped CRs (whose
/// targets must then name one).
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    /// Backend of the implicit target used when a CR lists none (the
    /// instance's `spec.secretStore.backend`).
    fn default_backend(&self) -> SecretStoreBackend;
    async fn write(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError>;
    async fn exists(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<bool, SecretStoreError>;
    async fn delete(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
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
