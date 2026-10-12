//! Which instance a CR lives on: its `instanceRef` resolved against the
//! instances the operator sees, according to its perimeter (the whole
//! cluster, or one namespace), and kept stable once recorded.
//!
//! - **operator limited to one namespace**: a namespaced CR only uses the
//!   local instances of its own namespace; a cluster-scoped CR only the
//!   cluster-scoped instances;
//! - **cluster-wide operator**: by name, the cluster-scoped instance
//!   first, then the local instance of the CR's namespace; `kind` and
//!   `namespace` pin one. Another namespace's local instance must share
//!   itself (`sharedWith`) — its token is that namespace's.
//! - **no `instanceRef`**: the only cluster-scoped instance, else the only
//!   local instance of the CR's namespace.
//!
//! Errors stay neutral ([`ResolveError`]): the operator maps them onto its
//! reason codes.

use std::collections::BTreeMap;

pub use weebo_kit_api::{BoundInstance, InstanceKind, InstanceRef, SharedWith};

/// An instance the operator sees, and what it shares (local ones).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub bound: BoundInstance,
    pub shared_with: Option<SharedWith>,
}

impl Candidate {
    pub fn cluster(name: impl Into<String>) -> Self {
        Self {
            bound: BoundInstance::cluster(name),
            shared_with: None,
        }
    }

    pub fn local(
        namespace: impl Into<String>,
        name: impl Into<String>,
        shared_with: Option<SharedWith>,
    ) -> Self {
        Self {
            bound: BoundInstance::local(namespace, name),
            shared_with,
        }
    }
}

/// What is resolved: the CR's namespace (`None`: cluster-scoped) and its
/// `instanceRef`.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub cr_namespace: Option<&'a str>,
    pub instance_ref: Option<&'a InstanceRef>,
}

impl Request<'_> {
    /// The namespace of the local instance this request names explicitly in
    /// another namespace than the CR's own — the case needing that local
    /// instance's consent, and the CR namespace's labels when the consent
    /// is a selector.
    pub fn foreign_namespace(&self) -> Option<&str> {
        let ns = self.instance_ref?.namespace.as_deref()?;
        (Some(ns) != self.cr_namespace).then_some(ns)
    }
}

/// What the operator sees of the instances.
#[derive(Debug, Clone, Default)]
pub struct Candidates<'a> {
    /// The namespace the operator is limited to, `None`: the whole cluster.
    pub watched_namespace: Option<&'a str>,
    /// Every cluster-scoped instance (none when the operator runs without
    /// cluster-scoped kinds).
    pub cluster: Vec<Candidate>,
    /// The local instances of the namespaces this request may use: the
    /// CR's own, and the one its `instanceRef` names.
    pub local: Vec<Candidate>,
    /// Labels of the CR's namespace, when a foreign local instance shares
    /// itself through a selector.
    pub cr_namespace_labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The instance named doesn't exist (or is outside the operator's
    /// scope).
    #[error("{kind} {name:?} not found")]
    NotFound { kind: String, name: String },
    /// No `instanceRef`, and no instance to default to.
    #[error("no instance exists for this object; set spec.instanceRef")]
    NoInstance,
    /// No `instanceRef`, and several instances to choose from.
    #[error("{count} {kind}s could serve this object; set spec.instanceRef")]
    Ambiguous { kind: String, count: usize },
    /// The `instanceRef` can't be satisfied as written (a `namespace` with
    /// `kind: Instance`, a cluster-scoped object naming a local instance
    /// without its namespace).
    #[error("invalid instanceRef: {0}")]
    InvalidRef(String),
    /// The instance named is outside what this operator may use: a
    /// cluster-scoped instance or another namespace for an operator
    /// limited to one namespace.
    #[error("{0}")]
    OutsideScope(String),
    /// Another namespace's local instance that doesn't share itself with
    /// this object.
    #[error("{instance} is not shared with {with}")]
    NotShared { instance: String, with: String },
    /// The name now resolves to another instance than the one recorded in
    /// the status (a cluster-scoped instance created under a local one's
    /// name): not followed.
    #[error(
        "spec.instanceRef {name:?} now designates {now}, not the {recorded} this object is bound to; \
         set spec.instanceRef.kind to choose"
    )]
    BindingChanged {
        name: String,
        recorded: Box<BoundInstance>,
        now: Box<BoundInstance>,
    },
}

/// The instance `request` designates among `candidates`.
pub fn resolve(
    request: Request<'_>,
    candidates: &Candidates<'_>,
) -> Result<BoundInstance, ResolveError> {
    let namespaced_operator = candidates.watched_namespace.is_some();
    let Some(r) = request.instance_ref else {
        return default_instance(request, candidates);
    };
    if r.namespace.is_some() && r.kind == Some(InstanceKind::Instance) {
        return Err(ResolveError::InvalidRef(
            "namespace only applies to kind LocalInstance".into(),
        ));
    }
    let kind = r
        .kind
        .or_else(|| r.namespace.is_some().then_some(InstanceKind::LocalInstance));
    let wants_cluster = match (request.cr_namespace, kind) {
        (_, Some(InstanceKind::Instance)) => true,
        (_, Some(InstanceKind::LocalInstance)) => false,
        (None, None) => true,
        // By name: a cluster-wide operator tries the cluster instance
        // first; one limited to a namespace never does.
        (Some(_), None) => {
            !namespaced_operator && find(&candidates.cluster, None, &r.name).is_some()
        }
    };
    if wants_cluster {
        if namespaced_operator && request.cr_namespace.is_some() {
            return Err(ResolveError::OutsideScope(format!(
                "this operator is limited to namespace {:?}: its namespaced objects only use \
                 LocalInstances",
                candidates.watched_namespace.unwrap_or_default()
            )));
        }
        return find(&candidates.cluster, None, &r.name)
            .map(|c| c.bound.clone())
            .ok_or_else(|| not_found(InstanceKind::Instance, None, &r.name));
    }
    let namespace = match (r.namespace.as_deref(), request.cr_namespace) {
        (Some(ns), _) | (None, Some(ns)) => ns,
        (None, None) => {
            return Err(ResolveError::InvalidRef(
                "a cluster-scoped object must set spec.instanceRef.namespace to use a LocalInstance"
                    .into(),
            ));
        }
    };
    if let Some(watched) = candidates.watched_namespace
        && watched != namespace
    {
        return Err(ResolveError::OutsideScope(format!(
            "this operator is limited to namespace {watched:?}: it can't use a LocalInstance of \
             {namespace:?}"
        )));
    }
    let found = find(&candidates.local, Some(namespace), &r.name)
        .ok_or_else(|| not_found(InstanceKind::LocalInstance, Some(namespace), &r.name))?;
    if request.cr_namespace != Some(namespace) {
        check_shared(found, request.cr_namespace, candidates)?;
    }
    Ok(found.bound.clone())
}

/// No `instanceRef`: the only cluster-scoped instance, else (namespaced
/// CR) the only local instance of its namespace.
fn default_instance(
    request: Request<'_>,
    candidates: &Candidates<'_>,
) -> Result<BoundInstance, ResolveError> {
    let only = |kind: InstanceKind, found: Vec<&Candidate>| match found.as_slice() {
        [one] => Ok(Some(one.bound.clone())),
        [] => Ok(None),
        many => Err(ResolveError::Ambiguous {
            kind: kind.as_str().to_string(),
            count: many.len(),
        }),
    };
    let namespaced_operator = candidates.watched_namespace.is_some();
    if request.cr_namespace.is_none() || !namespaced_operator {
        let cluster = candidates.cluster.iter().collect();
        if let Some(one) = only(InstanceKind::Instance, cluster)? {
            return Ok(one);
        }
    }
    if let Some(ns) = request.cr_namespace {
        let local = candidates
            .local
            .iter()
            .filter(|c| c.bound.namespace.as_deref() == Some(ns))
            .collect();
        if let Some(one) = only(InstanceKind::LocalInstance, local)? {
            return Ok(one);
        }
    }
    Err(ResolveError::NoInstance)
}

fn find<'a>(
    candidates: &'a [Candidate],
    namespace: Option<&str>,
    name: &str,
) -> Option<&'a Candidate> {
    candidates
        .iter()
        .find(|c| c.bound.name == name && c.bound.namespace.as_deref() == namespace)
}

fn not_found(kind: InstanceKind, namespace: Option<&str>, name: &str) -> ResolveError {
    ResolveError::NotFound {
        kind: kind.as_str().to_string(),
        name: match namespace {
            Some(ns) => format!("{ns}/{name}"),
            None => name.to_string(),
        },
    }
}

/// A local instance used from elsewhere: it must list the CR's namespace,
/// select it, or accept cluster-scoped objects.
fn check_shared(
    found: &Candidate,
    cr_namespace: Option<&str>,
    candidates: &Candidates<'_>,
) -> Result<(), ResolveError> {
    let allowed = crate::sharing::shares_with(
        found.shared_with.as_ref(),
        cr_namespace,
        candidates.cr_namespace_labels.as_ref(),
    );
    if allowed {
        return Ok(());
    }
    Err(ResolveError::NotShared {
        instance: found.bound.to_string(),
        with: match cr_namespace {
            Some(ns) => format!("namespace {ns:?}"),
            None => "cluster-scoped objects".into(),
        },
    })
}

/// Once recorded in the status, an instance stays: when `instanceRef`
/// names it without `kind` and the same name now resolves to another
/// instance, the change is refused. Naming another instance, or setting
/// `kind`, rebinds.
pub fn check_binding(
    instance_ref: Option<&InstanceRef>,
    recorded: Option<&BoundInstance>,
    resolved: BoundInstance,
) -> Result<BoundInstance, ResolveError> {
    let Some(recorded) = recorded else {
        return Ok(resolved);
    };
    let pinned = instance_ref.is_some_and(|r| r.kind.is_some());
    if pinned || recorded.name != resolved.name || *recorded == resolved {
        return Ok(resolved);
    }
    Err(ResolveError::BindingChanged {
        name: resolved.name.clone(),
        recorded: Box::new(recorded.clone()),
        now: Box::new(resolved),
    })
}

/// The operator's view of its instances: resolves a CR's `instanceRef`
/// (then [`check_binding`] against what its status recorded).
#[async_trait::async_trait]
pub trait InstanceDirectory: Send + Sync {
    async fn resolve(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        recorded: Option<&BoundInstance>,
    ) -> Result<BoundInstance, InstanceError>;
}

/// Why no instance could be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstanceError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// Listing the instances (or reading a namespace) failed — usually
    /// transient.
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};

    use super::*;

    fn by_name(name: &str) -> InstanceRef {
        InstanceRef::named(name)
    }

    fn pinned(name: &str, kind: InstanceKind, ns: Option<&str>) -> InstanceRef {
        InstanceRef {
            name: name.into(),
            kind: Some(kind),
            namespace: ns.map(String::from),
        }
    }

    fn req<'a>(ns: Option<&'a str>, r: Option<&'a InstanceRef>) -> Request<'a> {
        Request {
            cr_namespace: ns,
            instance_ref: r,
        }
    }

    fn cluster_wide(cluster: &[&str], local: &[(&str, &str)]) -> Candidates<'static> {
        Candidates {
            watched_namespace: None,
            cluster: cluster.iter().map(|n| Candidate::cluster(*n)).collect(),
            local: local
                .iter()
                .map(|(ns, n)| Candidate::local(*ns, *n, None))
                .collect(),
            cr_namespace_labels: None,
        }
    }

    #[test]
    fn a_cluster_wide_operator_prefers_the_cluster_instance_then_the_local_one() {
        let c = cluster_wide(&["main"], &[("team-a", "main"), ("team-a", "own")]);
        let r = by_name("main");
        assert_eq!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Ok(BoundInstance::cluster("main"))
        );
        let r = by_name("own");
        assert_eq!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Ok(BoundInstance::local("team-a", "own"))
        );
        let r = pinned("main", InstanceKind::LocalInstance, None);
        assert_eq!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Ok(BoundInstance::local("team-a", "main"))
        );
        let r = by_name("nope");
        assert!(matches!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Err(ResolveError::NotFound { .. })
        ));
    }

    #[test]
    fn a_namespaced_operator_only_uses_its_local_instances() {
        let mut c = cluster_wide(&["main"], &[("team-a", "main")]);
        c.watched_namespace = Some("team-a");
        let r = by_name("main");
        assert_eq!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Ok(BoundInstance::local("team-a", "main"))
        );
        let r = pinned("main", InstanceKind::Instance, None);
        assert!(matches!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Err(ResolveError::OutsideScope(_))
        ));
        let r = pinned("main", InstanceKind::LocalInstance, Some("team-b"));
        assert!(matches!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Err(ResolveError::OutsideScope(_))
        ));
        // Its cluster-scoped kinds still use the cluster instances.
        let r = by_name("main");
        assert_eq!(
            resolve(req(None, Some(&r)), &c),
            Ok(BoundInstance::cluster("main"))
        );
        // No ref: the namespace's only local instance, never the cluster one.
        assert_eq!(
            resolve(req(Some("team-a"), None), &c),
            Ok(BoundInstance::local("team-a", "main"))
        );
    }

    #[test]
    fn without_a_ref_the_only_instance() {
        let c = cluster_wide(&["main"], &[("team-a", "own")]);
        assert_eq!(
            resolve(req(Some("team-a"), None), &c),
            Ok(BoundInstance::cluster("main"))
        );
        let c = cluster_wide(&[], &[("team-a", "own")]);
        assert_eq!(
            resolve(req(Some("team-a"), None), &c),
            Ok(BoundInstance::local("team-a", "own"))
        );
        assert_eq!(
            resolve(req(Some("team-b"), None), &c),
            Err(ResolveError::NoInstance)
        );
        let c = cluster_wide(&["a", "b"], &[]);
        assert_eq!(
            resolve(req(None, None), &c),
            Err(ResolveError::Ambiguous {
                kind: "Instance".into(),
                count: 2
            })
        );
    }

    #[test]
    fn invalid_refs_are_explained() {
        let c = cluster_wide(&["main"], &[("team-a", "main")]);
        let r = pinned("main", InstanceKind::Instance, Some("team-a"));
        assert!(matches!(
            resolve(req(Some("team-a"), Some(&r)), &c),
            Err(ResolveError::InvalidRef(_))
        ));
        let r = pinned("main", InstanceKind::LocalInstance, None);
        assert!(matches!(
            resolve(req(None, Some(&r)), &c),
            Err(ResolveError::InvalidRef(_))
        ));
    }

    #[test]
    fn a_foreign_local_instance_must_share_itself() {
        let mut c = cluster_wide(&[], &[]);
        c.local = vec![Candidate::local(
            "team-a",
            "main",
            Some(SharedWith {
                namespaces: vec!["team-b".into()],
                namespace_selector: Some(LabelSelector {
                    match_expressions: Some(vec![LabelSelectorRequirement {
                        key: "tier".into(),
                        operator: "In".into(),
                        values: Some(vec!["front".into()]),
                    }]),
                    ..Default::default()
                }),
                cluster_scoped: false,
            }),
        )];
        let r = pinned("main", InstanceKind::LocalInstance, Some("team-a"));
        assert_eq!(
            req(Some("team-b"), Some(&r)).foreign_namespace(),
            Some("team-a")
        );
        assert_eq!(
            resolve(req(Some("team-b"), Some(&r)), &c),
            Ok(BoundInstance::local("team-a", "main"))
        );
        assert_eq!(
            resolve(req(Some("team-c"), Some(&r)), &c),
            Err(ResolveError::NotShared {
                instance: "LocalInstance team-a/main".into(),
                with: "namespace \"team-c\"".into()
            })
        );
        c.cr_namespace_labels = Some(BTreeMap::from([("tier".into(), "front".into())]));
        assert!(resolve(req(Some("team-c"), Some(&r)), &c).is_ok());
        assert!(matches!(
            resolve(req(None, Some(&r)), &c),
            Err(ResolveError::NotShared { .. })
        ));
        // Its own namespace never needs consent.
        assert_eq!(req(Some("team-a"), Some(&r)).foreign_namespace(), None);
        assert!(resolve(req(Some("team-a"), Some(&r)), &c).is_ok());
    }

    #[test]
    fn a_recorded_binding_is_not_silently_replaced() {
        let local = BoundInstance::local("team-a", "main");
        let cluster = BoundInstance::cluster("main");
        let r = by_name("main");
        assert_eq!(
            check_binding(Some(&r), None, cluster.clone()),
            Ok(cluster.clone())
        );
        assert_eq!(
            check_binding(Some(&r), Some(&local), local.clone()),
            Ok(local.clone())
        );
        assert!(matches!(
            check_binding(Some(&r), Some(&local), cluster.clone()),
            Err(ResolveError::BindingChanged { .. })
        ));
        let r = pinned("main", InstanceKind::Instance, None);
        assert_eq!(
            check_binding(Some(&r), Some(&local), cluster.clone()),
            Ok(cluster.clone())
        );
        let other = BoundInstance::cluster("other");
        let r = by_name("other");
        assert_eq!(
            check_binding(Some(&r), Some(&local), other.clone()),
            Ok(other)
        );
    }
}
