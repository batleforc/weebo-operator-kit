//! `SecretStore` writing Kubernetes `Secret`s: created, then replaced
//! whole (guarded by `resourceVersion`), under a dedicated field manager
//! and with **no ownerReference**: an emitted Secret
//! often lives in another namespace than its (possibly cluster-scoped) CR,
//! so it is deleted explicitly by the CR's finalizer instead of by GC.
//!
//! Every emitted Secret carries the operator's `managed-by` label and an
//! owner annotation naming the emitting CR ([`SecretOwner::id`]). A Secret
//! without them — created by someone else — is never overwritten nor
//! deleted, nor is one owned by another CR. Secrets emitted before the
//! owner annotation existed (label only) are adopted by the first CR
//! writing them.

use std::collections::BTreeMap;

use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Client;
use kube::api::{Api, DeleteParams, PostParams, Preconditions};
use weebo_kit_api::{SecretStoreBackend, SecretTarget};
use weebo_kit_application::emit::check_target;
use weebo_kit_application::ports::{SecretOwner, SecretStore, SecretStoreError};

/// Marks Secrets an operator emitted, for `kubectl get secret -l`.
pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";

pub struct K8sSecretStore {
    client: Client,
    field_manager: &'static str,
    marks: Marks,
}

/// The label and annotation telling who emitted a `Secret`.
struct Marks {
    managed_by: &'static str,
    owner_annotation: &'static str,
}

/// Who a `Secret` belongs to, from `owner`'s point of view.
#[derive(Debug, PartialEq, Eq)]
enum Ownership {
    Owned,
    /// Another CR of this operator, or not this operator at all.
    Foreign(String),
}

impl K8sSecretStore {
    /// `field_manager`: the operator's field manager (e.g.
    /// `weebo-forgejo-operator`); `managed_by`: the value of the
    /// `app.kubernetes.io/managed-by` label on every emitted Secret;
    /// `owner_annotation`: the annotation naming the emitting CR (e.g.
    /// `forgejo.weebo.io/owner`).
    pub fn new(
        client: Client,
        field_manager: &'static str,
        managed_by: &'static str,
        owner_annotation: &'static str,
    ) -> Self {
        Self {
            client,
            field_manager,
            marks: Marks {
                managed_by,
                owner_annotation,
            },
        }
    }

    fn api(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(Api<Secret>, String), SecretStoreError> {
        check_target(target, owner).map_err(|e| SecretStoreError(e.to_string()))?;
        let ns = target
            .namespace
            .as_deref()
            .or(owner.namespace)
            .ok_or_else(|| {
                SecretStoreError(format!("secret target {:?} has no namespace", target.name))
            })?;
        Ok((
            Api::namespaced(self.client.clone(), ns),
            format!("{ns}/{}", target.name),
        ))
    }
}

impl Marks {
    fn ownership(&self, secret: &Secret, owner: &SecretOwner<'_>) -> Ownership {
        let meta = &secret.metadata;
        let label = meta.labels.as_ref().and_then(|l| l.get(MANAGED_BY_LABEL));
        let recorded = meta
            .annotations
            .as_ref()
            .and_then(|a| a.get(self.owner_annotation));
        match (label, recorded) {
            (Some(by), Some(id)) if by == self.managed_by && *id == owner.id() => Ownership::Owned,
            (Some(by), None) if by == self.managed_by => Ownership::Owned,
            (Some(by), Some(id)) if by == self.managed_by => Ownership::Foreign(id.clone()),
            _ => Ownership::Foreign("not managed by this operator".to_string()),
        }
    }

    fn desired(
        &self,
        name: &str,
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Secret {
        Secret {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                labels: Some(BTreeMap::from([(
                    MANAGED_BY_LABEL.to_string(),
                    self.managed_by.to_string(),
                )])),
                annotations: Some(BTreeMap::from([(
                    self.owner_annotation.to_string(),
                    owner.id(),
                )])),
                ..Default::default()
            },
            type_: Some("Opaque".to_string()),
            // `data`, not `stringData`: written whole on every change, so
            // keys no longer emitted go (stringData only ever merges).
            data: Some(
                data.iter()
                    .map(|(k, v)| (k.clone(), ByteString(v.clone().into_bytes())))
                    .collect(),
            ),
            ..Default::default()
        }
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
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        let (api, key) = self.api(target, owner)?;
        let desired = self.marks.desired(&target.name, owner, data);
        let current = api
            .get_opt(&target.name)
            .await
            .map_err(|e| SecretStoreError(format!("reading secret {key}: {e}")))?;
        match current {
            // A plain create: a Secret appearing in between is a conflict,
            // never silently taken over.
            None => api
                .create(
                    &PostParams {
                        field_manager: Some(self.field_manager.to_string()),
                        ..Default::default()
                    },
                    &desired,
                )
                .await
                .map(|_| ())
                .map_err(|e| SecretStoreError(format!("creating secret {key}: {e}"))),
            Some(secret) => match self.marks.ownership(&secret, owner) {
                Ownership::Foreign(by) => Err(SecretStoreError(format!(
                    "secret {key} already exists and does not belong to {} ({by}); \
                     delete it or pick another target",
                    owner.id()
                ))),
                Ownership::Owned => {
                    // Steady state: nothing to write.
                    if secret.data == desired.data
                        && secret
                            .metadata
                            .annotations
                            .as_ref()
                            .is_some_and(|a| a.contains_key(self.marks.owner_annotation))
                    {
                        return Ok(());
                    }
                    // A full replace guarded by the resourceVersion just read:
                    // `data` becomes exactly what is emitted (keys no longer
                    // emitted go), the rest of the object is kept, and a
                    // concurrent change makes it fail rather than be lost.
                    let mut updated = secret;
                    updated.data = desired.data;
                    updated.string_data = None;
                    let meta = &mut updated.metadata;
                    meta.labels
                        .get_or_insert_default()
                        .extend(desired.metadata.labels.unwrap_or_default());
                    meta.annotations
                        .get_or_insert_default()
                        .extend(desired.metadata.annotations.unwrap_or_default());
                    meta.managed_fields = None;
                    api.replace(
                        &target.name,
                        &PostParams {
                            field_manager: Some(self.field_manager.to_string()),
                            ..Default::default()
                        },
                        &updated,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| SecretStoreError(format!("writing secret {key}: {e}")))
                }
            },
        }
    }

    async fn exists(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<bool, SecretStoreError> {
        let (api, key) = self.api(target, owner)?;
        api.get_opt(&target.name)
            .await
            .map(|s| s.is_some_and(|s| self.marks.ownership(&s, owner) == Ownership::Owned))
            .map_err(|e| SecretStoreError(format!("reading secret {key}: {e}")))
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(), SecretStoreError> {
        let (api, key) = self.api(target, owner)?;
        let Some(secret) = api
            .get_opt(&target.name)
            .await
            .map_err(|e| SecretStoreError(format!("reading secret {key}: {e}")))?
        else {
            return Ok(());
        };
        if let Ownership::Foreign(by) = self.marks.ownership(&secret, owner) {
            tracing::info!(secret = %key, owner = %by, "leaving a secret this CR does not own");
            return Ok(());
        }
        // Preconditions: never delete a Secret replaced since the read.
        let params = DeleteParams {
            preconditions: Some(Preconditions {
                uid: secret.metadata.uid.clone(),
                resource_version: secret.metadata.resource_version.clone(),
            }),
            ..Default::default()
        };
        match api.delete(&target.name, &params).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
            Err(e) => Err(SecretStoreError(format!("deleting secret {key}: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> Marks {
        Marks {
            managed_by: "op",
            owner_annotation: "example.io/owner",
        }
    }

    fn secret(label: Option<&str>, owner: Option<&str>) -> Secret {
        Secret {
            metadata: ObjectMeta {
                labels: label.map(|l| BTreeMap::from([(MANAGED_BY_LABEL.into(), l.into())])),
                annotations: owner.map(|o| BTreeMap::from([("example.io/owner".into(), o.into())])),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    const OWNER: SecretOwner<'static> = SecretOwner {
        kind: "Token",
        namespace: Some("ns"),
        name: "app",
    };

    #[test]
    fn ownership_rules() {
        let m = marks();
        let owned = |s: &Secret| m.ownership(s, &OWNER) == Ownership::Owned;
        assert!(owned(&secret(Some("op"), Some("Token/ns/app"))));
        // Emitted before owner annotations existed.
        assert!(owned(&secret(Some("op"), None)));
        assert!(!owned(&secret(Some("op"), Some("Token/ns/other"))));
        assert!(!owned(&secret(None, None)));
        assert!(!owned(&secret(Some("helm"), Some("Token/ns/app"))));
    }

    #[test]
    fn desired_secret_carries_data_label_and_owner() {
        let m = marks();
        let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
        let secret = m.desired("name", &OWNER, &data);
        assert_eq!(m.ownership(&secret, &OWNER), Ownership::Owned);
        assert_eq!(secret.data.unwrap()["k"].0, b"v");
        assert!(secret.string_data.is_none());
    }
}
