//! What the operator watches: the whole cluster or one namespace
//! (`WEEBO_WATCH_NAMESPACE`), optionally only the objects matching a label
//! selector (`WEEBO_WATCH_LABEL_SELECTOR`, a JSON `LabelSelector`).
//!
//! One scope for everything the operator sees: its controllers, the
//! watches on the kinds they reference and the instance resolution all go
//! through it, so an object outside it is invisible (a reference to it is
//! "not found"). The namespace only narrows the namespaced kinds; a
//! cluster-scoped kind stays watched cluster-wide — unless the operator
//! runs without cluster-scoped kinds at all (`WEEBO_WATCH_CLUSTER_KINDS=
//! false`, full namespaced: Roles only, no ClusterRole).
//!
//! The label selector splits the work between releases, it is not a
//! security boundary: RBAC cannot filter by label.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::api::{Api, ListParams};
use kube::core::{Selector, SelectorExt};
use kube::runtime::watcher;
use kube::{Client, Resource};

/// The namespace watched; unset or empty: the whole cluster.
pub const NAMESPACE_ENV: &str = "WEEBO_WATCH_NAMESPACE";
/// A `LabelSelector` as JSON (`{"matchLabels": {...}, "matchExpressions":
/// [...]}`); unset, empty or `{}`: every object.
pub const LABEL_SELECTOR_ENV: &str = "WEEBO_WATCH_LABEL_SELECTOR";
/// `false`: no cluster-scoped kind is watched (needs a namespace); unset:
/// `true`.
pub const CLUSTER_KINDS_ENV: &str = "WEEBO_WATCH_CLUSTER_KINDS";
/// `true`: a CR may reference a CR of another namespace — when that one
/// shares itself (its `spec.sharedWith`). Unset: `false`.
pub const CROSS_NAMESPACE_REFS_ENV: &str = "WEEBO_CROSS_NAMESPACE_REFS";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchScope {
    namespace: Option<String>,
    selector: Option<Selector>,
    no_cluster_kinds: bool,
    cross_namespace_refs: bool,
}

impl WatchScope {
    /// The whole cluster, every object: what an operator watched before
    /// scopes existed.
    pub fn cluster() -> Self {
        Self::default()
    }

    /// The namespaced kinds in `namespace` only.
    pub fn namespace(namespace: impl Into<String>) -> Self {
        Self {
            namespace: Some(namespace.into()),
            ..Self::default()
        }
    }

    /// No cluster-scoped kind at all: the operator runs on the Roles of
    /// its namespace only. Needs [`Self::namespace`].
    pub fn without_cluster_kinds(mut self) -> anyhow::Result<Self> {
        if self.namespace.is_none() {
            anyhow::bail!("{CLUSTER_KINDS_ENV}=false needs {NAMESPACE_ENV}");
        }
        self.no_cluster_kinds = true;
        Ok(self)
    }

    /// Only the objects `selector` matches. An empty selector matches
    /// everything.
    pub fn with_labels(mut self, selector: LabelSelector) -> anyhow::Result<Self> {
        let selector = Selector::try_from(selector)
            .map_err(|e| anyhow::anyhow!("invalid label selector: {}", e.0))?;
        self.selector = (!selector.selects_all()).then_some(selector);
        Ok(self)
    }

    /// This scope without its label selector, every other setting kept
    /// (namespace, cluster-scoped kinds, cross-namespace references): for
    /// what must see every object of a kind, whichever release selects it
    /// (a policy outside the selector still applies).
    pub fn without_selector(&self) -> Self {
        Self {
            selector: None,
            ..self.clone()
        }
    }

    /// References across namespaces allowed (still subject to the
    /// target's consent). Never for an operator limited to one namespace:
    /// it doesn't see the others.
    pub fn with_cross_namespace_refs(mut self) -> Self {
        self.cross_namespace_refs = true;
        self
    }

    /// Whether a CR may reference a CR of another namespace (that shares
    /// itself with it).
    pub fn cross_namespace_refs(&self) -> bool {
        self.cross_namespace_refs && self.namespace.is_none()
    }

    /// From [`NAMESPACE_ENV`], [`LABEL_SELECTOR_ENV`],
    /// [`CLUSTER_KINDS_ENV`] and [`CROSS_NAMESPACE_REFS_ENV`]. An invalid
    /// value fails the start rather than silently watching more.
    pub fn from_env() -> anyhow::Result<Self> {
        let scope = Self::from_values(
            std::env::var(NAMESPACE_ENV).ok().as_deref(),
            std::env::var(LABEL_SELECTOR_ENV).ok().as_deref(),
            std::env::var(CLUSTER_KINDS_ENV).ok().as_deref(),
        )?;
        match std::env::var(CROSS_NAMESPACE_REFS_ENV)
            .ok()
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("false") => Ok(scope),
            Some("true") => Ok(scope.with_cross_namespace_refs()),
            Some(other) => {
                anyhow::bail!("{CROSS_NAMESPACE_REFS_ENV}: {other:?} is neither true nor false")
            }
        }
    }

    fn from_values(
        namespace: Option<&str>,
        selector: Option<&str>,
        cluster_kinds: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut scope = match namespace.map(str::trim).filter(|ns| !ns.is_empty()) {
            Some(ns) => Self::namespace(ns),
            None => Self::cluster(),
        };
        if let Some(json) = selector.map(str::trim).filter(|s| !s.is_empty()) {
            let selector: LabelSelector = serde_json::from_str(json)
                .map_err(|e| anyhow::anyhow!("{LABEL_SELECTOR_ENV}: {e}"))?;
            scope = scope.with_labels(selector)?;
        }
        match cluster_kinds.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("true") => Ok(scope),
            Some("false") => scope.without_cluster_kinds(),
            Some(other) => {
                anyhow::bail!("{CLUSTER_KINDS_ENV}: {other:?} is neither true nor false")
            }
        }
    }

    /// Whether cluster-scoped kinds are watched.
    pub fn cluster_kinds(&self) -> bool {
        !self.no_cluster_kinds
    }

    /// `K` may be watched: called at start for every kind the operator
    /// needs, so a cluster-scoped kind under a scope without them fails
    /// there instead of looping on 403s.
    pub fn ensure<K>(&self) -> anyhow::Result<()>
    where
        K: Resource<DynamicType = ()>,
        K::Scope: KindScope,
    {
        if self.no_cluster_kinds && !<K::Scope as KindScope>::NAMESPACED {
            anyhow::bail!(
                "{} is cluster-scoped and this operator runs without cluster-scoped kinds \
                 ({CLUSTER_KINDS_ENV}=false)",
                K::kind(&())
            );
        }
        Ok(())
    }

    /// The namespace watched, `None` for the whole cluster.
    pub fn watched_namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// The label selector, `None` when every object is watched.
    pub fn selector(&self) -> Option<&Selector> {
        self.selector.as_ref()
    }

    /// The `Api` to list and watch `K` through: its namespace for a
    /// namespaced kind when one is watched, the whole cluster otherwise.
    /// Writes to a namespaced object still need `Api::namespaced` on its
    /// own namespace.
    pub fn api<K>(&self, client: Client) -> Api<K>
    where
        K: Resource<DynamicType = ()>,
        K::Scope: KindScope,
    {
        <K::Scope as KindScope>::api(client, self.namespace.as_deref())
    }

    /// Every watch of the operator: the label selector, server-side.
    pub fn watcher_config(&self) -> watcher::Config {
        match &self.selector {
            Some(selector) => watcher::Config::default().labels_from(selector),
            None => watcher::Config::default(),
        }
    }

    /// Every list of the operator: the label selector, server-side.
    pub fn list_params(&self) -> ListParams {
        match &self.selector {
            Some(selector) => ListParams::default().labels_from(selector),
            None => ListParams::default(),
        }
    }

    /// Whether `object` is in the scope. A `get` by name doesn't filter by
    /// label: what it returns is checked here.
    pub fn selects<K: Resource>(&self, object: &K) -> bool {
        let meta = object.meta();
        let in_namespace = match (&self.namespace, &meta.namespace) {
            (Some(watched), Some(ns)) => watched == ns,
            // Cluster-scoped objects, or the whole cluster watched.
            _ => true,
        };
        in_namespace && self.selects_labels(meta.labels.as_ref())
    }

    /// Whether objects carrying `labels` match the label selector.
    pub fn selects_labels(
        &self,
        labels: Option<&std::collections::BTreeMap<String, String>>,
    ) -> bool {
        let Some(selector) = &self.selector else {
            return true;
        };
        let empty = Default::default();
        selector.matches(labels.unwrap_or(&empty))
    }
}

/// Builds the `Api` of a kind according to its scope (namespaced or
/// cluster), known at compile time from `K::Scope`.
pub trait KindScope {
    /// Whether the kind lives in a namespace.
    const NAMESPACED: bool;

    fn api<K>(client: Client, namespace: Option<&str>) -> Api<K>
    where
        K: Resource<DynamicType = (), Scope = Self>;
}

impl KindScope for NamespaceResourceScope {
    const NAMESPACED: bool = true;

    fn api<K>(client: Client, namespace: Option<&str>) -> Api<K>
    where
        K: Resource<DynamicType = (), Scope = Self>,
    {
        match namespace {
            Some(ns) => Api::namespaced(client, ns),
            None => Api::all(client),
        }
    }
}

impl KindScope for ClusterResourceScope {
    const NAMESPACED: bool = false;

    fn api<K>(client: Client, _: Option<&str>) -> Api<K>
    where
        K: Resource<DynamicType = (), Scope = Self>,
    {
        Api::all(client)
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{ConfigMap, Namespace};
    use kube::api::ObjectMeta;

    use super::*;

    fn config_map(ns: &str, labels: &[(&str, &str)]) -> ConfigMap {
        ConfigMap {
            metadata: ObjectMeta {
                name: Some("c".into()),
                namespace: Some(ns.into()),
                labels: Some(
                    labels
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn nothing_set_is_the_whole_cluster() {
        assert_eq!(
            WatchScope::from_values(None, None, None).unwrap(),
            WatchScope::cluster()
        );
        assert_eq!(
            WatchScope::from_values(Some(" "), Some("{}"), Some("")).unwrap(),
            WatchScope::cluster()
        );
        assert!(WatchScope::cluster().selects(&config_map("any", &[])));
    }

    #[test]
    fn a_namespace_narrows_namespaced_objects_only() {
        let scope = WatchScope::from_values(Some("team-a"), None, None).unwrap();
        assert_eq!(scope.watched_namespace(), Some("team-a"));
        assert!(scope.selects(&config_map("team-a", &[])));
        assert!(!scope.selects(&config_map("team-b", &[])));
        let cluster_scoped = Namespace {
            metadata: ObjectMeta {
                name: Some("n".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(scope.selects(&cluster_scoped));
    }

    #[test]
    fn the_label_selector_is_parsed_and_matched() {
        let scope = WatchScope::from_values(
            None,
            Some(
                r#"{"matchLabels": {"weebo.io/shard": "a"},
                    "matchExpressions": [{"key": "tier", "operator": "NotIn", "values": ["test"]}]}"#,
            ),
            None,
        )
        .unwrap();
        assert!(scope.selects(&config_map("x", &[("weebo.io/shard", "a")])));
        assert!(!scope.selects(&config_map("x", &[("weebo.io/shard", "b")])));
        assert!(!scope.selects(&config_map(
            "x",
            &[("weebo.io/shard", "a"), ("tier", "test")]
        )));
        assert!(!scope.selects(&config_map("x", &[])));
        assert_eq!(
            scope.list_params().label_selector.as_deref(),
            Some("weebo.io/shard=a,tier notin (test)")
        );
    }

    #[test]
    fn an_invalid_selector_fails_the_start() {
        assert!(WatchScope::from_values(None, Some("shard=a"), None).is_err());
        assert!(
            WatchScope::from_values(
                None,
                Some(r#"{"matchExpressions": [{"key": "k", "operator": "Bogus"}]}"#),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn cross_namespace_refs_need_the_whole_cluster() {
        assert!(!WatchScope::cluster().cross_namespace_refs());
        assert!(
            WatchScope::cluster()
                .with_cross_namespace_refs()
                .cross_namespace_refs()
        );
        assert!(
            !WatchScope::namespace("a")
                .with_cross_namespace_refs()
                .cross_namespace_refs()
        );
    }

    #[test]
    fn without_cluster_kinds_needs_a_namespace_and_refuses_cluster_kinds() {
        assert!(WatchScope::from_values(None, None, Some("false")).is_err());
        assert!(WatchScope::from_values(Some("a"), None, Some("nope")).is_err());
        let full = WatchScope::from_values(Some("a"), None, Some("false")).unwrap();
        assert!(!full.cluster_kinds());
        assert!(full.ensure::<ConfigMap>().is_ok());
        let err = full.ensure::<Namespace>().unwrap_err().to_string();
        assert!(err.contains("Namespace is cluster-scoped"), "{err}");
        assert!(WatchScope::cluster().ensure::<Namespace>().is_ok());
    }

    #[test]
    fn without_selector_keeps_everything_else() {
        let scope = WatchScope::from_values(
            Some("a"),
            Some(r#"{"matchLabels": {"weebo.io/shard": "a"}}"#),
            Some("false"),
        )
        .unwrap()
        .with_cross_namespace_refs();
        let all = scope.without_selector();
        assert!(all.selector().is_none());
        assert!(all.watcher_config().label_selector.is_none());
        assert!(all.selects(&config_map("a", &[])));
        assert!(!scope.selects(&config_map("a", &[])));
        assert_eq!(all.watched_namespace(), Some("a"));
        assert!(!all.cluster_kinds());
        let cluster = WatchScope::cluster().with_cross_namespace_refs();
        assert!(cluster.without_selector().cross_namespace_refs());
        assert_eq!(cluster.without_selector(), cluster);
    }

    // Each kind knows its scope at compile time.
    const _: () = assert!(<<ConfigMap as Resource>::Scope as KindScope>::NAMESPACED);
    const _: () = assert!(!<<Namespace as Resource>::Scope as KindScope>::NAMESPACED);
}
