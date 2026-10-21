//! Migration of the CRs written when `spec.instanceRef` was a bare name:
//! `instanceRef: main` becomes `instanceRef: {name: main}` (no `kind`, so
//! the instance is found by name exactly as before).
//!
//! Offline, once: stop the operator, apply the new CRDs, run this, start
//! the operator. A structural schema can't accept both shapes, so the old
//! objects can't be read by the new operator until they are rewritten.
//! Replayable: an object already migrated is left alone.
//!
//! A CEL transition rule on `instanceRef` (`self == oldSelf`) blocks the
//! rewrite: the apiserver evaluates it against the stored string. Drop it
//! in the release that migrates; the reconcilers' recorded binding
//! (`status.instance`) keeps the guarantee.

use crate::instance::{LookupError, list_all, lookup};
use kube::api::{Api, ApiResource, DynamicObject, Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde_json::{Value, json};

/// What a run did, by `Kind/[namespace/]name`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Rewritten (or, on a dry run, to rewrite).
    pub migrated: Vec<String>,
    /// Already an object, or no `instanceRef`.
    pub unchanged: usize,
}

/// The new `spec.instanceRef` of an object written with the bare name,
/// `None` when it needs no change.
pub fn migrated_instance_ref(object: &Value) -> Option<Value> {
    match object.get("spec")?.get("instanceRef")? {
        Value::String(name) => Some(json!({ "name": name })),
        _ => None,
    }
}

/// Why a migration run stopped.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(transparent)]
    Kubernetes(#[from] LookupError),
    #[error("serializing an object: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Rewrites every object of `kinds` (cluster-wide) whose
/// `spec.instanceRef` is still a bare name; `dry_run` only lists them.
/// Stops at the first failed patch: rerunning resumes.
pub async fn migrate_instance_refs(
    client: &Client,
    kinds: &[ApiResource],
    field_manager: &str,
    dry_run: bool,
) -> Result<Migration, MigrateError> {
    let mut migration = Migration::default();
    for kind in kinds {
        let objects = list_all(
            &Api::<DynamicObject>::all_with(client.clone(), kind),
            &Default::default(),
        )
        .await
        .map_err(lookup(format!("listing {}", kind.kind)))?;
        for object in objects {
            let value = serde_json::to_value(&object)?;
            let Some(instance_ref) = migrated_instance_ref(&value) else {
                migration.unchanged = migration.unchanged.saturating_add(1);
                continue;
            };
            let name = object.name_any();
            let id = match object.namespace() {
                Some(ns) => format!("{}/{ns}/{name}", kind.kind),
                None => format!("{}/{name}", kind.kind),
            };
            if !dry_run {
                let api = match object.namespace() {
                    Some(ns) => Api::<DynamicObject>::namespaced_with(client.clone(), &ns, kind),
                    None => Api::<DynamicObject>::all_with(client.clone(), kind),
                };
                let patch = json!({ "spec": { "instanceRef": instance_ref } });
                api.patch(
                    &name,
                    &PatchParams::apply(field_manager),
                    &Patch::Merge(&patch),
                )
                .await
                .map_err(lookup(format!("migrating {id}")))?;
            }
            migration.migrated.push(id);
        }
    }
    Ok(migration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bare_name_is_rewritten() {
        assert_eq!(
            migrated_instance_ref(&json!({"spec": {"instanceRef": "main"}})),
            Some(json!({"name": "main"}))
        );
        assert_eq!(
            migrated_instance_ref(&json!({"spec": {"instanceRef": {"name": "main"}}})),
            None
        );
        assert_eq!(migrated_instance_ref(&json!({"spec": {}})), None);
        assert_eq!(migrated_instance_ref(&json!({})), None);
    }
}
