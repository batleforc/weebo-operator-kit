//! The operator's instance CRs (`XInstance`, cluster-scoped): read from a
//! live reflector `Store` when the binary wires one in (no per-reconcile
//! apiserver call), from the API otherwise (tests), and the one a CR's
//! `instanceRef` designates.

use std::fmt::Debug;

use kube::api::Api;
use kube::runtime::reflector::Store;
use kube::{Client, Resource, ResourceExt};
use serde::de::DeserializeOwned;

pub struct InstanceResolver<I: Resource<DynamicType = ()> + 'static> {
    client: Client,
    store: Option<Store<I>>,
}

impl<I> InstanceResolver<I>
where
    I: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    pub fn new(client: Client, store: Option<Store<I>>) -> Self {
        Self { client, store }
    }

    /// The live store, unless its writer is gone (shutdown): then the API.
    async fn ready_store(&self) -> Option<&Store<I>> {
        let store = self.store.as_ref()?;
        store.wait_until_ready().await.ok()?;
        Some(store)
    }

    /// Every instance in the cluster.
    pub async fn list(&self) -> Result<Vec<I>, String> {
        if let Some(store) = self.ready_store().await {
            return Ok(store.state().iter().map(|i| (**i).clone()).collect());
        }
        Api::<I>::all(self.client.clone())
            .list(&Default::default())
            .await
            .map(|l| l.items)
            .map_err(|e| format!("listing {}: {e}", I::kind(&())))
    }

    /// The instance `name`. `Ok(None)`: it doesn't exist.
    pub async fn get(&self, name: &str) -> Result<Option<I>, String> {
        if let Some(store) = self.ready_store().await {
            return Ok(store.find(|i| i.name_any() == name).map(|i| (*i).clone()));
        }
        Api::<I>::all(self.client.clone())
            .get_opt(name)
            .await
            .map_err(|e| format!("reading {} {name:?}: {e}", I::kind(&())))
    }
}

/// Why no instance could be selected — the operator maps it onto its
/// reason codes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    #[error("{kind} {name:?} not found")]
    NotFound { kind: String, name: String },
    /// No `instanceRef`, and no instance at all.
    #[error("no {kind} exists")]
    NoInstance { kind: String },
    /// No `instanceRef`, and several instances to choose from.
    #[error("{count} {kind}s exist; set spec.instanceRef")]
    Ambiguous { kind: String, count: usize },
}

/// `instance_ref` names one instance; without it the cluster must have
/// exactly one.
pub fn select<'a, I: Resource<DynamicType = ()>>(
    instances: &'a [I],
    instance_ref: Option<&str>,
) -> Result<&'a I, SelectError> {
    let kind = || I::kind(&()).to_string();
    match instance_ref {
        Some(name) => instances
            .iter()
            .find(|i| i.meta().name.as_deref() == Some(name))
            .ok_or_else(|| SelectError::NotFound {
                kind: kind(),
                name: name.to_string(),
            }),
        None => match instances {
            [only] => Ok(only),
            [] => Err(SelectError::NoInstance { kind: kind() }),
            many => Err(SelectError::Ambiguous {
                kind: kind(),
                count: many.len(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::Namespace;

    use super::*;

    fn named(name: &str) -> Namespace {
        let mut ns = Namespace::default();
        ns.metadata.name = Some(name.into());
        ns
    }

    #[test]
    fn select_by_ref_or_the_only_one() {
        let two = [named("a"), named("b")];
        assert_eq!(select(&two, Some("b")).unwrap().name_any(), "b");
        assert_eq!(
            select(&two, Some("c")).unwrap_err().to_string(),
            "Namespace \"c\" not found"
        );
        assert!(matches!(
            select(&two, None),
            Err(SelectError::Ambiguous { count: 2, .. })
        ));
        assert_eq!(select(&two[..1], None).unwrap().name_any(), "a");
        assert!(matches!(
            select::<Namespace>(&[], None),
            Err(SelectError::NoInstance { .. })
        ));
    }
}
