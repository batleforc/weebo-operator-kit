//! `SecretStore` writing Kubernetes `Secret`s. Server-side apply with a
//! dedicated field manager and **no ownerReference**: an emitted Secret
//! often lives in another namespace than its (possibly cluster-scoped) CR,
//! so it is deleted explicitly by the CR's finalizer instead of by GC.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::Client;
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use weebo_kit_api::{SecretStoreBackend, SecretTarget};
use weebo_kit_application::ports::{SecretStore, SecretStoreError};

/// Marks Secrets an operator emitted, for `kubectl get secret -l`.
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";

pub struct K8sSecretStore {
    client: Client,
    field_manager: &'static str,
    managed_by: &'static str,
}

impl K8sSecretStore {
    /// `field_manager`: the operator's server-side-apply manager (e.g.
    /// `weebo-forgejo-operator`); `managed_by`: the value of the
    /// `app.kubernetes.io/managed-by` label on every emitted Secret.
    pub fn new(client: Client, field_manager: &'static str, managed_by: &'static str) -> Self {
        Self {
            client,
            field_manager,
            managed_by,
        }
    }

    fn api(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<Api<Secret>, SecretStoreError> {
        let ns = target
            .namespace
            .as_deref()
            .or(default_namespace)
            .ok_or_else(|| {
                SecretStoreError(format!("secret target {:?} has no namespace", target.name))
            })?;
        Ok(Api::namespaced(self.client.clone(), ns))
    }
}

/// Used directly only by tests; operators go through the routing store,
/// which sends Kubernetes targets here.
#[async_trait::async_trait]
impl SecretStore for K8sSecretStore {
    fn default_backend(&self) -> SecretStoreBackend {
        SecretStoreBackend::Kubernetes
    }

    async fn write(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        let api = self.api(target, default_namespace)?;
        let secret = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": target.name,
                "labels": { MANAGED_BY_LABEL: self.managed_by },
            },
            "type": "Opaque",
            "stringData": data,
        });
        api.patch(
            &target.name,
            &PatchParams::apply(self.field_manager).force(),
            &Patch::Apply(&secret),
        )
        .await
        .map(|_| ())
        .map_err(|e| SecretStoreError(format!("writing secret {}: {e}", target.name)))
    }

    async fn exists(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<bool, SecretStoreError> {
        self.api(target, default_namespace)?
            .get_opt(&target.name)
            .await
            .map(|s| s.is_some())
            .map_err(|e| SecretStoreError(format!("reading secret {}: {e}", target.name)))
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<(), SecretStoreError> {
        match self
            .api(target, default_namespace)?
            .delete(&target.name, &DeleteParams::default())
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
            Err(e) => Err(SecretStoreError(format!(
                "deleting secret {}: {e}",
                target.name
            ))),
        }
    }
}
