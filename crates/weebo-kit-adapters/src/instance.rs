//! The operator's instance CRs (`XInstance`, cluster-scoped): read from a
//! live reflector `Store` when the binary wires one in (no per-reconcile
//! apiserver call), from the API otherwise (tests), and the one a CR's
//! `instanceRef` designates. With a label selector (the operator's watch
//! scope), an instance outside it doesn't exist for the operator.

use std::fmt::Debug;

use kube::api::{Api, ListParams};
use kube::core::{Selector, SelectorExt};
use kube::runtime::reflector::Store;
use kube::{Client, Resource, ResourceExt};
use serde::de::DeserializeOwned;

/// How long a reconcile waits for a reflector `Store` to finish its first
/// list before reading from the API instead.
pub const STORE_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often [`wait_synced`] re-checks a store that hasn't listed.
pub const STORE_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Whether `store` has listed within `timeout`: `false` when its writer is
/// gone (shutdown) or the time is up — the caller then reads the API: a
/// store that hasn't listed is never trusted empty, and a watch that can't
/// list doesn't hang every reconcile.
///
/// Re-checks every [`STORE_POLL`] instead of awaiting
/// `Store::wait_until_ready` once: kube's readiness wakes only the last
/// task that polled it (a single oneshot), so every other concurrent
/// waiter would sit out the whole `timeout`.
pub async fn wait_synced<K: Resource<DynamicType = ()> + Clone + 'static>(
    store: &Store<K>,
    timeout: std::time::Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout(STORE_POLL, store.wait_until_ready()).await {
            Ok(ready) => return ready.is_ok(),
            Err(_) if tokio::time::Instant::now() >= deadline => {
                tracing::warn!(
                    kind = %K::kind(&()),
                    "the watch has not listed yet: reading from the API"
                );
                return false;
            }
            Err(_) => {}
        }
    }
}

/// [`wait_synced`] within [`STORE_READY_TIMEOUT`].
pub(crate) async fn wait_ready<K: Resource<DynamicType = ()> + Clone + 'static>(
    store: &Store<K>,
) -> bool {
    wait_synced(store, STORE_READY_TIMEOUT).await
}

pub struct InstanceResolver<I: Resource<DynamicType = ()> + 'static> {
    client: Client,
    store: Option<Store<I>>,
    selector: Option<Selector>,
}

impl<I> InstanceResolver<I>
where
    I: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    pub fn new(client: Client, store: Option<Store<I>>) -> Self {
        Self {
            client,
            store,
            selector: None,
        }
    }

    /// Only the instances `selector` matches (the operator's
    /// `WatchScope::selector`); the store should come from a watch with
    /// the same selector.
    pub fn with_selector(mut self, selector: Option<Selector>) -> Self {
        self.selector = selector;
        self
    }

    fn selects(&self, instance: &I) -> bool {
        let Some(selector) = &self.selector else {
            return true;
        };
        let empty = Default::default();
        selector.matches(instance.meta().labels.as_ref().unwrap_or(&empty))
    }

    /// The live store, unless its writer is gone (shutdown) or it hasn't
    /// listed in time ([`wait_ready`]): then the API.
    async fn ready_store(&self) -> Option<&Store<I>> {
        let store = self.store.as_ref()?;
        wait_ready(store).await.then_some(store)
    }

    /// Every instance in the cluster.
    pub async fn list(&self) -> Result<Vec<I>, String> {
        if let Some(store) = self.ready_store().await {
            return Ok(store
                .state()
                .iter()
                .filter(|i| self.selects(i))
                .map(|i| (**i).clone())
                .collect());
        }
        let params = match &self.selector {
            Some(selector) => ListParams::default().labels_from(selector),
            None => ListParams::default(),
        };
        Api::<I>::all(self.client.clone())
            .list(&params)
            .await
            .map(|l| l.items)
            .map_err(|e| format!("listing {}: {e}", I::kind(&())))
    }

    /// The instance `name`. `Ok(None)`: it doesn't exist.
    pub async fn get(&self, name: &str) -> Result<Option<I>, String> {
        if let Some(store) = self.ready_store().await {
            return Ok(store
                .find(|i| i.name_any() == name && self.selects(i))
                .map(|i| (*i).clone()));
        }
        Api::<I>::all(self.client.clone())
            .get_opt(name)
            .await
            .map(|i| i.filter(|i| self.selects(i)))
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

    #[tokio::test(start_paused = true)]
    async fn a_store_that_never_lists_falls_back_to_the_api() {
        let (store, _writer) = kube::runtime::reflector::store::<Namespace>();
        let start = tokio::time::Instant::now();
        assert!(!wait_ready(&store).await);
        assert!(start.elapsed() >= STORE_READY_TIMEOUT);
    }

    #[tokio::test]
    async fn a_store_whose_writer_is_gone_falls_back_at_once() {
        let (store, writer) = kube::runtime::reflector::store::<Namespace>();
        drop(writer);
        assert!(!wait_ready(&store).await);
    }

    #[tokio::test]
    async fn every_concurrent_waiter_is_answered_once_the_store_lists() {
        let (store, mut writer) = kube::runtime::reflector::store::<Namespace>();
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                tokio::spawn(async move { wait_ready(&store).await })
            })
            .collect();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        writer.apply_watcher_event(&kube::runtime::watcher::Event::Init);
        writer.apply_watcher_event(&kube::runtime::watcher::Event::InitDone);
        for waiter in waiters {
            let ready = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
                .await
                .expect("a waiter was never woken")
                .unwrap();
            assert!(ready);
        }
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
