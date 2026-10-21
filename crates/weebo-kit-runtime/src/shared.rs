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

use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use kube::api::Api;
use kube::runtime::Controller;
use kube::runtime::controller::Config;
use kube::runtime::reflector::{self, ObjectRef, Store};
use kube::runtime::utils::Predicate;
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
                            if let Some(change) = change
                                && sender.send(change).is_err()
                            {
                                // No subscriber yet (standby): the store has it.
                                tracing::trace!("{name}: change dropped, no subscriber");
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

/// The binary's shared watches: one per kind and label selector (a kind
/// may have one through the operator's selector and one without it,
/// [`WatchScope::without_selector`]). A kind without one (a test's context
/// built with `SharedWatches::default()`) is watched by each controller on
/// its own.
#[derive(Clone, Default)]
pub struct SharedWatches {
    /// The `SharedWatch<K>`s by `TypeId::of::<K>()`, in spawn order, each
    /// with the label selector it watches through (`None`: every object).
    kinds: HashMap<TypeId, Vec<(Option<String>, AnyWatch)>>,
    listed: Vec<watch::Receiver<bool>>,
}

/// A `SharedWatch<K>` of some kind `K`.
type AnyWatch = Arc<dyn Any + Send + Sync>;

/// The label selector `scope` watches through: what tells two watches of
/// a kind apart.
fn selector_of(scope: &WatchScope) -> Option<String> {
    scope.watcher_config().label_selector
}

impl SharedWatches {
    /// The shared watch of `api`'s kind through `scope`'s label selector
    /// (`scope.watcher_config()`), spawned unless the kind already has one
    /// through that selector; its store. Errors are reported to `health`
    /// under `name`. Through `scope.without_selector()`, the kind's watch
    /// of every object — the same watch as through `scope` when the
    /// operator has no selector.
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
        let selector = selector_of(scope);
        if let Some(shared) = self.find::<K>(selector.as_deref()) {
            return shared.store();
        }
        let shared = SharedWatch::spawn(api, scope.watcher_config(), health.clone(), name);
        let store = shared.store();
        self.insert(selector, shared);
        store
    }

    fn insert<K>(&mut self, selector: Option<String>, shared: SharedWatch<K>)
    where
        K: Resource<DynamicType = ()> + Send + Sync + 'static,
    {
        self.listed.push(shared.listed.clone());
        self.kinds
            .entry(TypeId::of::<K>())
            .or_default()
            .push((selector, Arc::new(shared)));
    }

    /// The shared watch of `K` through exactly `selector`.
    fn find<K>(&self, selector: Option<&str>) -> Option<&SharedWatch<K>>
    where
        K: Resource<DynamicType = ()> + 'static,
    {
        self.kinds
            .get(&TypeId::of::<K>())?
            .iter()
            .find(|(s, _)| s.as_deref() == selector)?
            .1
            .downcast_ref()
    }

    /// The shared watch of `K`, if the binary spawned one — the first
    /// spawned when it has several ([`Self::get_in`] tells them apart).
    pub fn get<K>(&self) -> Option<&SharedWatch<K>>
    where
        K: Resource<DynamicType = ()> + 'static,
    {
        self.kinds
            .get(&TypeId::of::<K>())?
            .first()?
            .1
            .downcast_ref()
    }

    /// The shared watch of `K` through `scope`'s label selector, if the
    /// binary spawned one (`scope.without_selector()`: its watch of every
    /// object).
    pub fn get_in<K>(&self, scope: &WatchScope) -> Option<&SharedWatch<K>>
    where
        K: Resource<DynamicType = ()> + 'static,
    {
        self.find(selector_of(scope).as_deref())
    }

    /// A shared watch of `K` that sees at least what `scope` does: its
    /// watch through `scope`'s selector, else its watch of every object.
    fn covering<K>(&self, scope: &WatchScope) -> Option<&SharedWatch<K>>
    where
        K: Resource<DynamicType = ()> + 'static,
    {
        self.get_in(scope).or_else(|| self.find(None))
    }

    /// The controller of `K`: on its shared watch through `scope`'s label
    /// selector when there is one, else the kit's [`controller`] on its own
    /// watch of `api` through `scope` (never on a watch through another
    /// selector: it would reconcile objects outside the scope).
    pub fn controller<K>(&self, api: Api<K>, scope: &WatchScope) -> Controller<K>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
    {
        match self.get_in::<K>(scope) {
            Some(shared) => shared.controller(),
            None => controller(api, scope),
        }
    }

    /// Every object of `K` touched (deleted ones included), as a
    /// `Controller::watches_stream` takes it: from its shared watch through
    /// `scope`'s selector, else its shared watch of every object (more
    /// objects, never fewer), else a watch of its own through `scope`.
    /// `None` for a cluster-scoped kind when the operator watches none
    /// (`scope.cluster_kinds()` false).
    pub fn touched<K>(
        &self,
        scope: &WatchScope,
        client: &Client,
    ) -> Option<BoxStream<'static, Result<K, watcher::Error>>>
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
        K::Scope: KindScope,
    {
        if skipped::<K>(scope) {
            return None;
        }
        Some(match self.covering::<K>(scope) {
            Some(shared) => shared.touched().boxed(),
            None => watcher(scope.api::<K>(client.clone()), scope.watcher_config())
                .default_backoff()
                .touched_objects()
                .boxed(),
        })
    }

    /// Resolves once every shared watch has listed (or is gone).
    pub async fn listed(&self) {
        for mut listed in self.listed.iter().cloned() {
            if listed.wait_for(|listed| *listed).await.is_err() {
                tracing::debug!("a shared watch ended before it listed");
            }
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

/// Whether `K` is left unwatched: cluster-scoped, and `scope` watches no
/// cluster-scoped kind.
fn skipped<K>(scope: &WatchScope) -> bool
where
    K: Resource,
    K::Scope: KindScope,
{
    !<K::Scope as KindScope>::NAMESPACED && !scope.cluster_kinds()
}

/// A watch on a kind the CRs reference, re-triggering the CRs `map`
/// returns: on the kind's shared watch through `scope`'s selector, else
/// its shared watch of every object, else its own watch through `scope` —
/// and skipped for a cluster-scoped kind when the operator watches none
/// (`scope.cluster_kinds()` false). Implemented by `Controller` only.
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

    /// [`Self::watches_in`], re-triggering only on the changes `predicate`
    /// sees (`predicate_filter`: an object whose hash didn't change since
    /// its last change is dropped, e.g. a hash of the status field the CRs
    /// render).
    fn watches_in_filtered<K, I, P>(
        self,
        watches: &SharedWatches,
        scope: &WatchScope,
        client: &Client,
        predicate: P,
        map: impl Fn(K) -> I + Send + Sync + 'static,
    ) -> Self
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
        K::Scope: KindScope,
        P: Predicate<K> + Send + 'static,
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
        if skipped::<K>(scope) {
            return self;
        }
        if let Some(shared) = watches.covering::<K>(scope) {
            return self.watches_stream(shared.touched(), map);
        }
        self.watches(scope.api::<K>(client.clone()), scope.watcher_config(), map)
    }

    fn watches_in_filtered<K, I, P>(
        self,
        watches: &SharedWatches,
        scope: &WatchScope,
        client: &Client,
        predicate: P,
        map: impl Fn(K) -> I + Send + Sync + 'static,
    ) -> Self
    where
        K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
        K::Scope: KindScope,
        P: Predicate<K> + Send + 'static,
        I: IntoIterator<Item = ObjectRef<C>> + 'static,
        I::IntoIter: Send,
    {
        match watches.touched::<K>(scope, client) {
            Some(touched) => {
                self.watches_stream(touched.predicate_filter(predicate, Default::default()), map)
            }
            None => self,
        }
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
            // A test without a subscriber drops it, as the real task does.
            fed.shared.changes.send(change).ok();
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
        watches.insert(None, maps.shared.clone());
        watches.insert(None, secrets.shared.clone());
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
        watches.insert(None, maps.shared.clone());
        assert!(watches.get::<ConfigMap>().is_some());
        assert!(watches.get::<Secret>().is_none());
    }

    fn selected() -> WatchScope {
        let labels = [("weebo.io/shard".to_string(), "a".to_string())];
        WatchScope::namespace("ns")
            .with_labels(
                k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                    match_labels: Some(labels.into_iter().collect()),
                    ..Default::default()
                },
            )
            .unwrap()
    }

    /// A client that never connects: building streams and controllers only.
    fn offline() -> Client {
        // Err: another test installed it first, which is as good.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        Client::try_from(kube::Config::new("http://127.0.0.1:1".parse().unwrap())).unwrap()
    }

    #[tokio::test]
    async fn a_kind_has_one_watch_per_selector() {
        let scope = selected();
        let all = scope.without_selector();
        let (every, only) = (fed::<ConfigMap>(), fed::<ConfigMap>());
        let mut watches = SharedWatches::default();
        assert!(watches.get_in::<ConfigMap>(&scope).is_none());
        watches.insert(selector_of(&all), every.shared.clone());
        // Only the watch of every object: what `scope` sees is covered,
        // the controller of `scope` isn't on it.
        assert!(watches.get_in::<ConfigMap>(&scope).is_none());
        assert!(watches.get_in::<ConfigMap>(&all).is_some());
        assert!(watches.covering::<ConfigMap>(&scope).is_some());
        watches.insert(selector_of(&scope), only.shared.clone());
        let same = |a: Option<&SharedWatch<ConfigMap>>, b: &Fed<ConfigMap>| {
            a.is_some_and(|a| a.listed.same_channel(&b.shared.listed))
        };
        assert!(same(watches.get_in(&scope), &only));
        assert!(same(watches.get_in(&all), &every));
        assert!(same(watches.covering(&scope), &only));
        // `get`: the first spawned.
        assert!(same(watches.get(), &every));
        assert!(watches.get_in::<Secret>(&all).is_none());
        // Both are waited for.
        every.listed.send_replace(true);
        assert!(!watches.wait_listed(Duration::from_millis(50)).await);
        only.listed.send_replace(true);
        assert!(watches.wait_listed(Duration::from_millis(50)).await);
    }

    #[test]
    fn without_a_selector_both_scopes_share_one_watch() {
        let scope = WatchScope::namespace("ns");
        assert_eq!(selector_of(&scope), selector_of(&scope.without_selector()));
        assert_eq!(selector_of(&WatchScope::cluster()), None);
        assert!(selector_of(&selected()).is_some());
    }

    #[tokio::test]
    async fn spawn_adds_a_watch_only_for_a_new_selector() {
        let client = offline();
        let health = Arc::new(Health::default());
        let count = |w: &SharedWatches| w.kinds.get(&TypeId::of::<ConfigMap>()).map_or(0, Vec::len);
        let mut watches = SharedWatches::default();
        for scope in [selected(), selected().without_selector(), selected()] {
            watches.spawn(
                scope.api::<ConfigMap>(client.clone()),
                &scope,
                &health,
                "cm",
            );
        }
        assert_eq!((count(&watches), watches.listed.len()), (2, 2));
        // No selector: the scope and its selector-free copy are one watch.
        let mut watches = SharedWatches::default();
        let scope = WatchScope::namespace("ns");
        for scope in [scope.clone(), scope.without_selector()] {
            watches.spawn(
                scope.api::<ConfigMap>(client.clone()),
                &scope,
                &health,
                "cm",
            );
        }
        assert_eq!(count(&watches), 1);
    }

    #[tokio::test]
    async fn a_cluster_kind_is_skipped_without_cluster_kinds() {
        use k8s_openapi::api::core::v1::Namespace;
        let client = offline();
        let watches = SharedWatches::default();
        let full = WatchScope::namespace("ns").without_cluster_kinds().unwrap();
        assert!(watches.touched::<Namespace>(&full, &client).is_none());
        assert!(watches.touched::<ConfigMap>(&full, &client).is_some());
        assert!(
            watches
                .touched::<Namespace>(&WatchScope::cluster(), &client)
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_filtered_stream_only_yields_the_changes_the_predicate_sees() {
        let mut fed = fed::<ConfigMap>();
        let mut watches = SharedWatches::default();
        let scope = WatchScope::namespace("ns");
        watches.insert(selector_of(&scope), fed.shared.clone());
        apply(&mut fed, watcher::Event::Init);
        apply(&mut fed, watcher::Event::InitDone);
        let touched = watches.touched::<ConfigMap>(&scope, &offline()).unwrap();
        // A hash of `data` only: a change of the labels is dropped.
        let data = |o: &ConfigMap| {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut h = DefaultHasher::new();
            o.data.hash(&mut h);
            Some(h.finish())
        };
        let mut stream = Box::pin(touched.predicate_filter(data, Default::default()));
        let with = |data: &str, label: &str| {
            let mut o = cm("a");
            o.data = Some([("k".to_string(), data.to_string())].into());
            o.metadata.labels = Some([("l".to_string(), label.to_string())].into());
            o
        };
        // Listed (empty): now waiting for changes.
        assert!(drain(&mut stream).await.is_empty());
        apply(&mut fed, watcher::Event::Apply(with("1", "x")));
        apply(&mut fed, watcher::Event::Apply(with("1", "y")));
        apply(&mut fed, watcher::Event::Apply(with("2", "y")));
        let seen: Vec<_> = drain(&mut stream)
            .await
            .into_iter()
            .map(|o| o.unwrap().data.unwrap_or_default()["k"].clone())
            .collect();
        assert_eq!(seen, ["1", "2"]);
    }

    #[tokio::test]
    async fn changes_are_dropped_while_no_one_subscribes() {
        let mut fed = fed();
        apply(&mut fed, watcher::Event::Apply(cm("a")));
        assert_eq!(fed.shared.changes.len(), 0);
    }
}
