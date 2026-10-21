//! An [`InstanceDirectory`] over one namespaced instance kind `L` only,
//! for an operator without a cluster-scoped instance kind (the instance
//! *is* a workload the operator runs in a namespace). [`Instances`]
//! (crate::Instances) needs both kinds; this is its local half, on the
//! same pure resolution (`resolve_bound`), with no cluster candidate.
//!
//! What an instance shares with other namespaces is the operator's to
//! say, per call: [`LocalInstances::resolve_with`] takes it as a function
//! (a kind bound only through the instance's consent selector, another
//! kind never). [`InstanceDirectory::resolve`] shares with no one.
//!
//! Read from a live reflector `Store` when the binary wires one in, from
//! the API otherwise (tests). The operator's watch scope applies: its
//! label selector and the namespace it is limited to.

use crate::instance::{LookupError, list_all, lookup};
use std::fmt::Debug;

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::{Api, ListParams};
use kube::core::{Selector, SelectorExt};
use kube::runtime::reflector::Store;
use kube::{Client, Resource, ResourceExt};
use serde::de::DeserializeOwned;
use weebo_kit_api::{BoundInstance, InstanceKind, InstanceRef, SharedWith};
use weebo_kit_application::instances::{
    Candidate, Candidates, InstanceDirectory, InstanceError, Request, resolve_bound,
};

pub struct LocalInstances<L>
where
    L: Resource<DynamicType = ()> + 'static,
{
    client: Client,
    store: Option<Store<L>>,
    watched_namespace: Option<String>,
    selector: Option<Selector>,
}

impl<L> LocalInstances<L>
where
    L: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    /// Read from the API on every call (tests): wire the binary's
    /// reflector in with [`Self::with_store`].
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: None,
            watched_namespace: None,
            selector: None,
        }
    }

    pub fn with_store(mut self, store: Store<L>) -> Self {
        self.store = Some(store);
        self
    }

    /// The operator's watch scope: the namespace it is limited to and its
    /// label selector.
    pub fn with_scope(
        mut self,
        watched_namespace: Option<String>,
        selector: Option<Selector>,
    ) -> Self {
        self.watched_namespace = watched_namespace;
        self.selector = selector;
        self
    }

    fn selects(&self, instance: &L) -> bool {
        let Some(selector) = &self.selector else {
            return true;
        };
        let empty = Default::default();
        selector.matches(instance.meta().labels.as_ref().unwrap_or(&empty))
    }

    /// The instances of `namespace` in scope.
    pub async fn instances(&self, namespace: &str) -> Result<Vec<L>, LookupError> {
        if self
            .watched_namespace
            .as_deref()
            .is_some_and(|w| w != namespace)
        {
            return Ok(Vec::new());
        }
        if let Some(store) = &self.store
            && crate::instance::wait_ready(store).await
        {
            return Ok(store
                .state()
                .iter()
                .filter(|i| i.namespace().as_deref() == Some(namespace) && self.selects(i))
                .map(|i| (**i).clone())
                .collect());
        }
        let params = match &self.selector {
            Some(selector) => ListParams::default().labels_from(selector),
            None => ListParams::default(),
        };
        list_all(
            &Api::<L>::namespaced(self.client.clone(), namespace),
            &params,
        )
        .await
        .map_err(lookup(format!("listing {} in {namespace:?}", L::kind(&()))))
    }

    /// The instance `bound` designates, `None` when it is gone (or out of
    /// scope, or not a local instance).
    pub async fn get(&self, bound: &BoundInstance) -> Result<Option<L>, LookupError> {
        let (InstanceKind::LocalInstance, Some(namespace)) =
            (bound.kind, bound.namespace.as_deref())
        else {
            return Ok(None);
        };
        Ok(self
            .instances(namespace)
            .await?
            .into_iter()
            .find(|i| i.name_any() == bound.name))
    }

    /// Resolves a CR of `cr_namespace` (`instanceRef`, the binding
    /// recorded), each instance sharing itself with other namespaces as
    /// `sharing` says.
    pub async fn resolve_with(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        recorded: Option<&BoundInstance>,
        sharing: impl Fn(&L) -> Option<SharedWith> + Send + Sync,
    ) -> Result<BoundInstance, InstanceError> {
        let request = Request {
            cr_namespace,
            instance_ref,
        };
        let mut candidates = Candidates {
            watched_namespace: self.watched_namespace.as_deref(),
            ..Default::default()
        };
        let namespaces = [cr_namespace, request.foreign_namespace()];
        let mut needs_labels = false;
        for namespace in namespaces.into_iter().flatten() {
            for instance in self.instances(namespace).await? {
                let shared = sharing(&instance);
                needs_labels |= Some(namespace) != cr_namespace
                    && shared
                        .as_ref()
                        .is_some_and(|s| s.namespace_selector.is_some());
                candidates
                    .local
                    .push(Candidate::local(namespace, instance.name_any(), shared));
            }
        }
        if needs_labels && let Some(namespace) = cr_namespace {
            candidates.cr_namespace_labels = Api::<Namespace>::all(self.client.clone())
                .get_opt(namespace)
                .await
                .map_err(|e| {
                    InstanceError::Kubernetes(format!("reading namespace {namespace}: {e}"))
                })?
                .map(|ns| ns.metadata.labels.unwrap_or_default());
        }
        Ok(resolve_bound(request, &candidates, recorded)?)
    }
}

#[async_trait::async_trait]
impl<L> InstanceDirectory for LocalInstances<L>
where
    L: Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Clone
        + Debug
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    /// The CR's own namespace only: an `instanceRef` naming another
    /// namespace resolves to nothing it may use.
    async fn resolve(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        recorded: Option<&BoundInstance>,
    ) -> Result<BoundInstance, InstanceError> {
        self.resolve_with(cr_namespace, instance_ref, recorded, |_| None)
            .await
    }
}
