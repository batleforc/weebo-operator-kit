//! Bringing existing remote objects under an operator: the importer writes
//! a CR per object, then **adopts** it — creates (or takes over) the CR
//! paused (`<domain>/import-pending`), seeds its status with the remote
//! object's identifier, and unpauses it. The operator never sees the CR
//! without its identifier, so it can't create a duplicate next to the
//! object being imported.
//!
//! What the identifier is and where the status keeps it is the operator's:
//! it hands over the status fields to seed, and reads the identifier a CR
//! already holds.

use crate::instance::{LookupError, lookup};
use std::fmt::Debug;

use kube::api::{Api, Patch, PatchParams};
use kube::{Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use weebo_kit_api::Annotations;
use weebo_kit_api::annotations::IMPORT_PENDING;

/// What adopting one CR did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adoption {
    /// Created or taken over, seeded, unpaused.
    Adopted,
    /// Already bound to this remote object (a re-run).
    AlreadyBound,
}

/// Why a CR could not be adopted.
#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error(transparent)]
    Kubernetes(#[from] LookupError),
    #[error("already bound to {owned}, not {remote_id}")]
    BoundElsewhere { owned: String, remote_id: String },
    #[error(
        "exists but is bound to nothing yet — the operator may be creating its object right \
         now; delete it, or re-run with --force to take it over"
    )]
    Unbound,
    #[error("serializing the CR: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// What a run of adoptions did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub adopted: usize,
    /// Already bound to the same remote object.
    pub skipped: usize,
    /// Reported as they happen.
    pub failed: usize,
}

impl Summary {
    pub fn record(&mut self, result: &Result<Adoption, AdoptError>) {
        match result {
            Ok(Adoption::Adopted) => self.adopted = self.adopted.saturating_add(1),
            Ok(Adoption::AlreadyBound) => self.skipped = self.skipped.saturating_add(1),
            Err(_) => self.failed = self.failed.saturating_add(1),
        }
    }
}

/// Adopts CRs for one operator.
#[derive(Debug, Clone, Copy)]
pub struct Adopter {
    /// The importer's field manager (`weebo-x-importer`).
    pub field_manager: &'static str,
    pub annotations: Annotations,
}

impl Adopter {
    /// Adopts `cr` (its status is ignored: `seed` is written instead) as
    /// the CR of remote object `remote_id`. `bound_to` reads the remote
    /// identifier a CR already holds. A CR already in the cluster:
    ///
    /// - bound to `remote_id`: nothing to do (an earlier run paused by a
    ///   failure after seeding is unpaused);
    /// - bound to another object: an error;
    /// - bound to nothing: refused unless `force` or paused by an earlier
    ///   run — the operator may be creating its object right now.
    pub async fn adopt<K>(
        &self,
        api: &Api<K>,
        cr: &K,
        remote_id: &str,
        seed: serde_json::Value,
        bound_to: impl Fn(&K) -> Option<String>,
        force: bool,
    ) -> Result<Adoption, AdoptError>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + Serialize + DeserializeOwned,
    {
        let name = cr.name_any();
        let pending = self.annotations.key(IMPORT_PENDING);
        let existing = api
            .get_opt(&name)
            .await
            .map_err(lookup("reading the CR".into()))?;
        if let Some(existing) = existing {
            let paused = existing.annotations().contains_key(&pending);
            match bound_to(&existing) {
                Some(owned) if owned == remote_id => {
                    if paused {
                        self.unpause(api, &name).await?;
                    }
                    return Ok(Adoption::AlreadyBound);
                }
                Some(owned) => {
                    return Err(AdoptError::BoundElsewhere {
                        owned,
                        remote_id: remote_id.to_string(),
                    });
                }
                None if paused || force => {}
                None => return Err(AdoptError::Unbound),
            }
        }

        let mut cr = cr.clone();
        cr.annotations_mut().insert(pending, "true".into());
        let mut body = serde_json::to_value(&cr)?;
        if let Some(obj) = body.as_object_mut() {
            obj.remove("status");
        }
        api.patch(
            &name,
            &PatchParams::apply(self.field_manager).force(),
            &Patch::Apply(&body),
        )
        .await
        .map_err(lookup("creating the CR".into()))?;
        api.patch_status(
            &name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "status": seed })),
        )
        .await
        .map_err(lookup(
            "seeding status (the CR stays paused until a re-run)".into(),
        ))?;
        self.unpause(api, &name).await?;
        Ok(Adoption::Adopted)
    }

    async fn unpause<K>(&self, api: &Api<K>, name: &str) -> Result<(), AdoptError>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned,
    {
        let mut annotations = serde_json::Map::new();
        annotations.insert(
            self.annotations.key(IMPORT_PENDING),
            serde_json::Value::Null,
        );
        api.patch(
            name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "metadata": { "annotations": annotations } })),
        )
        .await
        .map(|_| ())
        .map_err(lookup("unpausing the CR".into()))?;
        Ok(())
    }
}

/// `items` as one multi-document YAML stream (`---` between documents).
pub fn to_yaml<T: Serialize>(items: &[T]) -> Result<String, serde_norway::Error> {
    let mut out = String::new();
    for item in items {
        out.push_str("---\n");
        out.push_str(&serde_norway::to_string(item)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_each_outcome() {
        let mut s = Summary::default();
        s.record(&Ok(Adoption::Adopted));
        s.record(&Ok(Adoption::AlreadyBound));
        s.record(&Err(AdoptError::Unbound));
        assert_eq!(
            s,
            Summary {
                adopted: 1,
                skipped: 1,
                failed: 1
            }
        );
    }

    #[test]
    fn yaml_streams_separate_documents() {
        let yaml = to_yaml(&[json!({"a": 1}), json!({"b": 2})]).unwrap();
        assert_eq!(yaml, "---\na: 1\n---\nb: 2\n");
    }
}
