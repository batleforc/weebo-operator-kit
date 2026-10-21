//! The operator's two instance kinds — the cluster-scoped `I`
//! (`XInstance`) and the namespaced `L` (`XLocalInstance`) — as one
//! [`InstanceDirectory`]: what a CR's `instanceRef` designates
//! (`weebo_kit_application::instances::resolve_bound`), kept stable once
//! recorded.
//!
//! Read from live reflector `Store`s when the binary wires them in, from
//! the API otherwise (tests). The operator's watch scope applies: its
//! label selector, the namespace it is limited to, and whether it watches
//! cluster-scoped kinds at all (no `I` then).

use crate::instance::{LookupError, list_all, lookup};
use std::fmt::Debug;

use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::api::{Api, ListParams};
use kube::core::{Selector, SelectorExt};
use kube::runtime::reflector::Store;
use kube::{Client, Resource, ResourceExt};
use serde::de::DeserializeOwned;
pub use weebo_kit_api::SharesItself;
use weebo_kit_api::{BoundInstance, InstanceKind, InstanceRef};
use weebo_kit_application::instances::{
    Candidate, Candidates, InstanceDirectory, InstanceError, Request, resolve_bound,
};

/// Either instance kind, once resolved.
#[derive(Debug, Clone)]
pub enum Resolved<I, L> {
    Cluster(I),
    Local(L),
}

pub struct Instances<I, L>
where
    I: Resource<DynamicType = ()> + 'static,
    L: Resource<DynamicType = ()> + 'static,
{
    client: Client,
    /// `None`: the operator watches no cluster-scoped kind.
    cluster: Option<Option<Store<I>>>,
    local: Option<Store<L>>,
    watched_namespace: Option<String>,
    selector: Option<Selector>,
}

impl<I, L> Instances<I, L>
where
    I: Resource<DynamicType = (), Scope = ClusterResourceScope>
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    L: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + SharesItself
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    /// Both kinds, read from the API on every call (tests): wire the
    /// binary's reflectors in with [`Self::with_stores`].
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cluster: Some(None),
            local: None,
            watched_namespace: None,
            selector: None,
        }
    }

    /// The live stores of both kinds (`cluster: None` keeps reading `I`
    /// from the API).
    pub fn with_stores(mut self, cluster: Option<Store<I>>, local: Option<Store<L>>) -> Self {
        if self.cluster.is_some() {
            self.cluster = Some(cluster);
        }
        self.local = local;
        self
    }

    /// The operator's watch scope: the namespace it is limited to, its
    /// label selector, and whether it watches cluster-scoped kinds (when
    /// not, `I` is never read: the operator has no right to).
    pub fn with_scope(
        mut self,
        watched_namespace: Option<String>,
        selector: Option<Selector>,
        cluster_kinds: bool,
    ) -> Self {
        self.watched_namespace = watched_namespace;
        self.selector = selector;
        if !cluster_kinds {
            self.cluster = None;
        }
        self
    }

    fn selects<K: Resource>(&self, object: &K) -> bool {
        let Some(selector) = &self.selector else {
            return true;
        };
        let empty = Default::default();
        selector.matches(object.meta().labels.as_ref().unwrap_or(&empty))
    }

    fn params(&self) -> ListParams {
        match &self.selector {
            Some(selector) => ListParams::default().labels_from(selector),
            None => ListParams::default(),
        }
    }

    /// The live store once it has listed, `None` (read from the API) when
    /// its writer is gone or it still hasn't listed after
    /// [`STORE_READY_TIMEOUT`](crate::instance::STORE_READY_TIMEOUT) — a watch that
    /// can't list (RBAC) must not hang every reconcile.
    async fn ready<K: Resource<DynamicType = ()> + Clone + 'static>(
        store: Option<&Store<K>>,
    ) -> Option<&Store<K>> {
        let store = store?;
        crate::instance::wait_ready(store).await.then_some(store)
    }

    /// Every cluster-scoped instance in scope (none without cluster kinds).
    pub async fn cluster_instances(&self) -> Result<Vec<I>, LookupError> {
        let Some(store) = &self.cluster else {
            return Ok(Vec::new());
        };
        if let Some(store) = Self::ready(store.as_ref()).await {
            return Ok(store
                .state()
                .iter()
                .filter(|i| self.selects(i.as_ref()))
                .map(|i| (**i).clone())
                .collect());
        }
        list_all(&Api::<I>::all(self.client.clone()), &self.params())
            .await
            .map_err(lookup(format!("listing {}", I::kind(&()))))
    }

    /// The local instances of `namespace` in scope.
    pub async fn local_instances(&self, namespace: &str) -> Result<Vec<L>, LookupError> {
        if let Some(store) = Self::ready(self.local.as_ref()).await {
            return Ok(store
                .state()
                .iter()
                .filter(|l| l.namespace().as_deref() == Some(namespace) && self.selects(l.as_ref()))
                .map(|l| (**l).clone())
                .collect());
        }
        list_all(
            &Api::<L>::namespaced(self.client.clone(), namespace),
            &self.params(),
        )
        .await
        .map_err(lookup(format!("listing {} in {namespace:?}", L::kind(&()))))
    }

    /// The instance `bound` designates, `None` when it is gone (or out of
    /// scope).
    pub async fn get(&self, bound: &BoundInstance) -> Result<Option<Resolved<I, L>>, LookupError> {
        Ok(match (bound.kind, bound.namespace.as_deref()) {
            (InstanceKind::Instance, _) => self
                .cluster_instances()
                .await?
                .into_iter()
                .find(|i| i.name_any() == bound.name)
                .map(Resolved::Cluster),
            (InstanceKind::LocalInstance, Some(ns)) => self
                .local_instances(ns)
                .await?
                .into_iter()
                .find(|l| l.name_any() == bound.name)
                .map(Resolved::Local),
            (InstanceKind::LocalInstance, None) => None,
        })
    }

    /// What `request` may resolve among: the cluster instances, the local
    /// ones of the CR's namespace and of the one it names (within the
    /// namespace the operator is limited to), and the CR namespace's
    /// labels when a foreign local instance shares through a selector.
    async fn candidates(&self, request: Request<'_>) -> Result<Candidates<'_>, LookupError> {
        let mut candidates = Candidates {
            watched_namespace: self.watched_namespace.as_deref(),
            ..Default::default()
        };
        candidates.cluster = self
            .cluster_instances()
            .await?
            .iter()
            .map(|i| Candidate::cluster(i.name_any()))
            .collect();
        let visible = |ns: &str| self.watched_namespace.as_deref().is_none_or(|w| w == ns);
        let foreign = request.foreign_namespace();
        for ns in [request.cr_namespace, foreign].into_iter().flatten() {
            if !visible(ns) {
                continue;
            }
            candidates.local.extend(
                self.local_instances(ns)
                    .await?
                    .iter()
                    .map(|l| Candidate::local(ns, l.name_any(), l.shared_with().cloned())),
            );
        }
        let by_selector = candidates.local.iter().any(|c| {
            c.bound.namespace.as_deref() == foreign
                && c.shared_with
                    .as_ref()
                    .is_some_and(|s| s.namespace_selector.is_some())
        });
        if let (Some(cr_ns), Some(_), true) = (request.cr_namespace, foreign, by_selector) {
            let namespace = Api::<Namespace>::all(self.client.clone())
                .get_opt(cr_ns)
                .await
                .map_err(lookup(format!("reading namespace {cr_ns:?}")))?;
            candidates.cr_namespace_labels = namespace.and_then(|n| n.metadata.labels);
        }
        Ok(candidates)
    }
}

#[async_trait::async_trait]
impl<I, L> InstanceDirectory for Instances<I, L>
where
    I: Resource<DynamicType = (), Scope = ClusterResourceScope>
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    L: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + SharesItself
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    async fn resolve(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        recorded: Option<&BoundInstance>,
    ) -> Result<BoundInstance, InstanceError> {
        let request = Request {
            cr_namespace,
            instance_ref,
        };
        let candidates = self.candidates(request).await?;
        Ok(resolve_bound(request, &candidates, recorded)?)
    }
}
