//! One watch per kind, shared by every controller watching it (as its
//! primary kind or as a referenced one) and by the reads that need its
//! store: without it, each `Controller::watches` opens its own watch, and
//! every one of them LISTs when the controllers start, right after the
//! lease is taken.
//!
//! The binary spawns them before it campaigns for the lease and waits for
//! them to list ([`SharedWatches::wait_listed`], bounded by
//! [`LISTED_TIMEOUT`]): a standby replica keeps its stores warm, and the
//! new leader's controllers start from stores that already listed. A
//! controller subscribing gets every object of the store once it has
//! listed, then every change: what its own watch would have yielded.

use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Config;
use kube::runtime::reflector::{self, ObjectRef, Store};
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Client, Resource};
use serde::de::DeserializeOwned;
use tokio::sync::{broadcast, watch};

use crate::controller::{concurrency, controller, reconcile_triggers};
use crate::scope::KindScope;
use crate::{Health, WatchScope};

/// How long a replica waits for its shared watches to list before it
/// campaigns for the lease anyway (a watch that can't list is reported by
/// `/livez`, not waited for).
pub const LISTED_TIMEOUT: Duration = Duration::from_secs(30);

/// Changes a subscriber may fall behind by before it re-reads the whole
/// store (it then sees every object again: nothing is lost, the
/// controller's predicates drop what didn't change).
const BUFFER: usize = 256;

/// A change: the object, and whether it was deleted.
type Change<K> = (Arc<K>, bool);

/// A reflector whose changes are broadcast to its subscribers.
pub struct SharedWatch<K: Resource<DynamicType = ()> + 'static> {
    store: Store<K>,
    changes: broadcast::Sender<Change<K>>,
    /// `true` once the store has listed. Subscribers wait on this, not on
    /// `Store::wait_until_ready`: kube's readiness wakes one waiter only
    /// (a single oneshot), and the controller's runner waits on it too.
    listed: watch::Receiver<bool>,
}

impl<K: Resource<DynamicType = ()> + 'static> Clone for SharedWatch<K> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            changes: self.changes.clone(),
            listed: self.listed.clone(),
        }
    }
}

/// What a watch event broadcasts, and whether it says the store listed.
fn change<K>(event: watcher::Event<K>) -> (Option<Change<K>>, bool) {
    match event {
        watcher::Event::Apply(o) | watcher::Event::InitApply(o) => {
            (Some((Arc::new(o), false)), false)
        }
        watcher::Event::Delete(o) => (Some((Arc::new(o), true)), false),
        watcher::Event::Init => (None, false),
        watcher::Event::InitDone => (None, true),
    }
}

impl<K> SharedWatch<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    /// Watches `api` through `config` in the background: watch errors back
    /// off exponentially and are reported to `health` under `name`, as
    /// [`crate::store::spawn_reflector`]. Nothing is buffered while no one
    /// subscribes.
    pub fn spawn(
        api: Api<K>,
        config: watcher::Config,
        health: Arc<Health>,
        name: &'static str,
    ) -> Self {
        let (store, writer) = reflector::store::<K>();
        let (changes, _) = broadcast::channel(BUFFER);
        let (listed_tx, listed) = watch::channel(false);
        let sender = changes.clone();
        // The reflector applies each event to the store before yielding it.
        let stream = reflector::reflector(writer, watcher(api, config).default_backoff());
        tokio::spawn(async move {
            stream
                .for_each(|event| {
                    match event {
                        Ok(event) => {
                            health.ok(name);
                            let (change, done) = change(event);
                            if let Some(change) = change {
                                // No subscriber yet (standby): dropped.
                                let _ = sender.send(change);
                            }
                            if done {
                                listed_tx.send_replace(true);
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "{name} watch error");
                            health.failed(name, err.to_string());
                        }
                    }
                    futures::future::ready(())
                })
                .await;
        });
        Self {
            store,
            changes,
            listed,
        }
    }

    /// The live store (wait for it to list before trusting it empty).
    pub fn store(&self) -> Store<K> {
        self.store.clone()
    }

    /// Every object of the store once it has listed, then every change —
    /// the whole store again when this subscriber fell behind.
    fn subscribe(&self) -> impl Stream<Item = Change<K>> + Send + 'static {
        struct State<K: Resource<DynamicType = ()> + 'static> {
            changes: broadcast::Receiver<Change<K>>,
            listed: watch::Receiver<bool>,
            store: Store<K>,
            pending: VecDeque<Change<K>>,
            read: bool,
        }
        // Subscribed before the store is read: a change in between is seen
        // twice, never missed.
        let state = State {
            changes: self.changes.subscribe(),
            listed: self.listed.clone(),
            store: self.store.clone(),
            pending: VecDeque::new(),
            read: false,
        };
        futures::stream::unfold(state, |mut state| async move {
            loop {
                if let Some(change) = state.pending.pop_front() {
                    return Some((change, state));
                }
                if !state.read {
                    // Its watch gone before listing: the process is
                    // stopping.
                    state.listed.wait_for(|listed| *listed).await.ok()?;
                    let all = state.store.state().into_iter().map(|o| (o, false));
                    state.pending = all.collect();
                    state.read = true;
                    continue;
                }
                match state.changes.recv().await {
                    Ok(change) => return Some((change, state)),
                    Err(broadcast::error::RecvError::Lagged(_)) => state.read = false,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
    }

    /// What a `Controller::watches` on the kind yields: every object
    /// touched, deleted ones included.
    pub fn touched(&self) -> impl Stream<Item = Result<K, watcher::Error>> + Send + 'static {
        self.subscribe().map(|(o, _)| Ok(K::clone(&o)))
    }

    /// The objects applied, deletions left out: a controller's primary
    /// watch (`applied_objects`).
    fn applied(&self) -> impl Stream<Item = Result<K, watcher::Error>> + Send + 'static {
        self.subscribe()
            .filter_map(|(o, deleted)| futures::future::ready((!deleted).then(|| Ok(K::clone(&o)))))
    }

    /// The kit's [`controller`] on this watch: same triggers (spec,
    /// annotations, labels, finalizers, deletion — never the CR's own
    /// status patches) and the same concurrency.
    pub fn controller(&self) -> Controller<K> {
        let trigger = self
            .applied()
            .predicate_filter(reconcile_triggers(), Default::default());
        Controller::for_stream(trigger, self.store())
            .with_config(Config::default().concurrency(concurrency()))
    }
}

/// The binary's shared watches, one per kind. A kind without one (a
/// test's context built with `SharedWatches::default()`) is watched by
/// each controller on its own.
#[derive(Clone, Default)]
pub struct SharedWatches {
    /// `SharedWatch<K>` by `TypeId::of::<K>()`.
    kinds: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    listed: Vec<watch::Receiver<bool>>,
}

impl SharedWatches {
    /// The shared watch of `api`'s kind, through `scope`'s label selector
    /// (`scope.watcher_config()`), spawned unless the kind already has one;
    /// its store. Errors are reported to `health` under `name`.
    pub fn spawn<K>(
        &mut self,
        api: Api<K>,
        scope: &WatchScope,
        health: &Arc<Health>,
        name: &'static str,
    ) -> Store<K>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    {
        if let Some(shared) = self.get::<K>() {
            return shared.store();
        }
        let shared = SharedWatch::spawn(api, scope.watcher_config(), health.clone(), name);
        let store = shared.store();
        self.insert(shared);
        store
    }

    fn insert<K>(&mut self, shared: SharedWatch<K>)
    where
        K: Resource<DynamicType = ()> + Send + Sync + 'static,
    {
        self.listed.push(shared.listed.clone());
        self.kinds.insert(TypeId::of::<K>(), Arc::new(shared));
    }

    /// The shared watch of `K`, if the binary spawned one.
    pub fn get<K>(&self) -> Option<&SharedWatch<K>>
    where
        K: Resource<DynamicType = ()> + 'static,
    {
        self.kinds.get(&TypeId::of::<K>())?.downcast_ref()
    }

    /// The controller of `K`: on its shared watch when there is one, else
    /// the kit's [`controller`] on its own watch of `api` through `scope`.
    pub fn controller<K>(&self, api: Api<K>, scope: &WatchScope) -> Controller<K>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    {
        match self.get::<K>() {
            Some(shared) => shared.controller(),
            None => controller(api, scope),
        }
    }

    /// Resolves once every shared watch has listed (or is gone).
    pub async fn listed(&self) {
        for mut listed in self.listed.iter().cloned() {
            let _ = listed.wait_for(|listed| *listed).await;
        }
    }

    /// [`Self::listed`] within `timeout` (`false`, with a warning, when it
    /// is up): what the binary waits for before campaigning for the lease.
    pub async fn wait_listed(&self, timeout: Duration) -> bool {
        let listed = tokio::time::timeout(timeout, self.listed()).await.is_ok();
        if !listed {
            tracing::warn!("the shared watches have not all listed: going on anyway");
        }
        listed
    }
}

/// A watch on a kind the CRs reference, re-triggering the CRs `map`
/// returns: on the kind's shared watch when there is one, else its own
/// watch through `scope` — and skipped for a cluster-scoped kind when the
/// operator watches none (`scope.cluster_kinds()` false).
pub trait WatchesIn<C>: Sized
where
    C: Resource<DynamicType = ()> + Clone + Debug + Send + Sync + 'static,
{
    fn watches_in<K, I>(
        self,
        watches: &SharedWatches,
        scope: &WatchScope,
        client: &Client,
        map: impl Fn(K) -> I + Send + Sync + 'static,
    ) -> Self
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
        K::Scope: KindScope,
        I: IntoIterator<Item = ObjectRef<C>> + 'static,
        I::IntoIter: Send;
}

impl<C> WatchesIn<C> for Controller<C>
where
    C: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    fn watches_in<K, I>(
        self,
        watches: &SharedWatches,
        scope: &WatchScope,
        client: &Client,
        map: impl Fn(K) -> I + Send + Sync + 'static,
    ) -> Self
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
        K::Scope: KindScope,
        I: IntoIterator<Item = ObjectRef<C>> + 'static,
        I::IntoIter: Send,
    {
        if !<K::Scope as KindScope>::NAMESPACED && !scope.cluster_kinds() {
            return self;
        }
        if let Some(shared) = watches.get::<K>() {
            return self.watches_stream(shared.touched(), map);
        }
        self.watches(scope.api::<K>(client.clone()), scope.watcher_config(), map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ConfigMap, Secret};
    use kube::api::ObjectMeta;

    fn cm(name: &str) -> ConfigMap {
        ConfigMap {
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some("ns".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// A shared watch fed by hand: its writer, to apply events, and its
    /// "listed" signal.
    struct Fed<K: Resource<DynamicType = ()> + Clone + 'static> {
        shared: SharedWatch<K>,
        writer: reflector::store::Writer<K>,
        listed: watch::Sender<bool>,
    }

    fn fed<K>() -> Fed<K>
    where
        K: Resource<DynamicType = ()> + Clone + 'static,
    {
        let (store, writer) = reflector::store::<K>();
        let (changes, _) = broadcast::channel(BUFFER);
        let (listed, rx) = watch::channel(false);
        Fed {
            shared: SharedWatch {
                store,
                changes,
                listed: rx,
            },
            writer,
            listed,
        }
    }

    /// As the background task: the store, then the broadcast.
    fn apply(fed: &mut Fed<ConfigMap>, event: watcher::Event<ConfigMap>) {
        fed.writer.apply_watcher_event(&event);
        let (change, done) = change(event);
        if let Some(change) = change {
            let _ = fed.shared.changes.send(change);
        }
        if done {
            fed.listed.send_replace(true);
        }
    }

    fn names(changes: &[Change<ConfigMap>]) -> Vec<(String, bool)> {
        let mut names: Vec<_> = changes
            .iter()
            .map(|(o, d)| (o.metadata.name.clone().unwrap_or_default(), *d))
            .collect();
        names.sort();
        names
    }

    async fn drain<S: Stream + Unpin>(stream: &mut S) -> Vec<S::Item> {
        let mut seen = Vec::new();
        while let Ok(Some(item)) =
            tokio::time::timeout(Duration::from_millis(50), stream.next()).await
        {
            seen.push(item);
        }
        seen
    }

    #[tokio::test]
    async fn a_subscriber_sees_the_listed_objects_then_the_changes() {
        let mut fed = fed();
        let mut stream = Box::pin(fed.shared.subscribe());
        // Nothing before the store has listed.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), stream.next())
                .await
                .is_err()
        );
        apply(&mut fed, watcher::Event::Init);
        apply(&mut fed, watcher::Event::InitApply(cm("a")));
        apply(&mut fed, watcher::Event::InitDone);
        let first = vec![stream.next().await.unwrap()];
        // Listed: the store's object (and the buffered change, twice at
        // worst — never missed).
        assert_eq!(names(&first), [("a".into(), false)]);
        apply(&mut fed, watcher::Event::Delete(cm("a")));
        let seen = drain(&mut stream).await;
        assert!(names(&seen).contains(&("a".into(), true)));
    }

    #[tokio::test]
    async fn a_lagging_subscriber_rereads_the_whole_store() {
        let mut fed = fed();
        apply(&mut fed, watcher::Event::Init);
        apply(&mut fed, watcher::Event::InitDone);
        let mut stream = Box::pin(fed.shared.subscribe());
        for i in 0..BUFFER + 10 {
            apply(&mut fed, watcher::Event::Apply(cm(&format!("o{i}"))));
        }
        let names = names(&drain(&mut stream).await);
        for i in 0..BUFFER + 10 {
            assert!(names.contains(&(format!("o{i}"), false)), "o{i} missed");
        }
    }

    #[tokio::test]
    async fn the_primary_stream_leaves_deletions_out() {
        let mut fed = fed();
        apply(&mut fed, watcher::Event::Init);
        apply(&mut fed, watcher::Event::InitDone);
        let mut stream = Box::pin(fed.shared.applied());
        // Listed (empty): now waiting for changes.
        assert!(
            tokio::time::timeout(Duration::from_millis(50), stream.next())
                .await
                .is_err()
        );
        apply(&mut fed, watcher::Event::Apply(cm("a")));
        apply(&mut fed, watcher::Event::Delete(cm("a")));
        apply(&mut fed, watcher::Event::Apply(cm("b")));
        let seen: Vec<_> = drain(&mut stream)
            .await
            .into_iter()
            .map(|o| o.unwrap().metadata.name.unwrap_or_default())
            .collect();
        assert_eq!(seen, ["a", "b"]);
    }

    #[tokio::test]
    async fn the_watches_are_listed_once_every_one_has() {
        let none = SharedWatches::default();
        none.listed().await;

        let maps = fed::<ConfigMap>();
        let secrets = fed::<Secret>();
        let mut watches = SharedWatches::default();
        watches.insert(maps.shared.clone());
        watches.insert(secrets.shared.clone());
        assert!(!watches.wait_listed(Duration::from_millis(50)).await);
        maps.listed.send_replace(true);
        assert!(!watches.wait_listed(Duration::from_millis(50)).await);
        secrets.listed.send_replace(true);
        assert!(watches.wait_listed(Duration::from_millis(50)).await);
    }

    #[tokio::test]
    async fn each_kind_finds_its_own_watch() {
        let maps = fed::<ConfigMap>();
        let mut watches = SharedWatches::default();
        assert!(watches.get::<ConfigMap>().is_none());
        watches.insert(maps.shared.clone());
        assert!(watches.get::<ConfigMap>().is_some());
        assert!(watches.get::<Secret>().is_none());
    }

    #[tokio::test]
    async fn changes_are_dropped_while_no_one_subscribes() {
        let mut fed = fed();
        apply(&mut fed, watcher::Event::Apply(cm("a")));
        assert_eq!(fed.shared.changes.len(), 0);
    }
}
