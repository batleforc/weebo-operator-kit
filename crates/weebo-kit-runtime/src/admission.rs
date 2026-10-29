//! The validating admission webhook's `/validate` endpoint: decoding the
//! review, the decisions counter, the denial shape (403 + reason code),
//! failing closed, the warnings `kubectl` prints — around the operator's
//! own decision ([`Admission`]).
//! And `/scope` ([`ScopeGuard`]): no object the operator manages leaves
//! its label selector by accident.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use kube::core::DynamicObject;
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview, Operation};
use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use serde::de::DeserializeOwned;
use weebo_kit_api::{Annotations, SecretStoreBackend, SecretTarget, VaultPathGrant};
use weebo_kit_application::emit;
use weebo_kit_application::ports::SecretOwner;
use weebo_kit_domain::Reason;
use weebo_kit_domain::vault_paths::{self, PathGrant, Writer};

use crate::{Health, WatchScope};

/// Why a request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial<R> {
    pub reason: R,
    pub message: String,
}

impl<R> Denial<R> {
    pub fn new(reason: R, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

/// An operator's admission decision.
#[async_trait::async_trait]
pub trait Admission: Send + Sync + 'static {
    type Reason: Reason;

    /// Decides on `object`, of kind `kind`, created or updated in
    /// `namespace`. A kind the operator doesn't govern must be refused:
    /// anything not understood fails closed.
    async fn admit(
        &self,
        kind: &str,
        namespace: &str,
        object: &DynamicObject,
    ) -> Result<(), Denial<Self::Reason>>;

    /// [`admit`](Self::admit), plus the warnings of an admitted object:
    /// returned in the response's `warnings`, which `kubectl` prints (a
    /// deprecated field, a setting that will be ignored…). What `/validate`
    /// calls. By default `admit`, with no warning: override it to warn, and
    /// have `admit` keep the decision alone (e.g.
    /// `self.admit_with_warnings(…).await.map(drop)`).
    async fn admit_with_warnings(
        &self,
        kind: &str,
        namespace: &str,
        object: &DynamicObject,
    ) -> Result<Vec<String>, Denial<Self::Reason>> {
        self.admit(kind, namespace, object)
            .await
            .map(|()| Vec::new())
    }

    /// The reason of a request that can't be decided on at all (no
    /// object).
    fn undecidable(&self) -> Self::Reason;
}

/// A review's object that isn't the CR it claims to be.
#[derive(Debug, thiserror::Error)]
#[error("object does not match the CRD: {0}")]
pub struct NotTheCrd(#[from] serde_json::Error);

/// The typed CR behind a review's `DynamicObject`.
pub fn typed<K: DeserializeOwned>(object: &DynamicObject) -> Result<K, NotTheCrd> {
    Ok(serde_json::from_value(serde_json::to_value(object)?)?)
}

struct AdmissionState<A> {
    admission: Arc<A>,
    decisions: Counter<u64>,
}

/// `POST /validate` deciding through `admission`
/// ([`Admission::admit_with_warnings`]), plus the kit's
/// `/readyz`, `/livez`, `/healthz` probes. `meter`/`prefix` name the
/// decisions counter (`<prefix>_webhook_decisions_total`, by `kind` and
/// `result`).
///
/// - a `DELETE` is always allowed: revoking access must never strand a
///   CR's cleanup;
/// - an `UPDATE` leaving `spec` alone (finalizer removal, labels) is
///   allowed: it references nothing new, and blocking it would leave a CR
///   whose namespace lost its policy stuck in `Terminating` — reconcilers
///   re-check before any remote call anyway.
pub fn router<A: Admission>(
    admission: Arc<A>,
    health: Arc<Health>,
    meter: &'static str,
    prefix: &str,
) -> Router {
    let decisions = opentelemetry::global::meter(meter)
        .u64_counter(format!("{prefix}_webhook_decisions"))
        .with_description("Admission webhook decisions, by kind and result (allow/deny)")
        .build();
    let state = Arc::new(AdmissionState {
        admission,
        decisions,
    });
    Router::new()
        .route("/validate", post(validate::<A>))
        .with_state(state)
        .merge(crate::health::routes(health))
}

async fn validate<A: Admission>(
    State(state): State<Arc<AdmissionState<A>>>,
    Json(review): Json<AdmissionReview<DynamicObject>>,
) -> Json<AdmissionReview<DynamicObject>> {
    let kind = review
        .request
        .as_ref()
        .map(|r| r.kind.kind.clone())
        .unwrap_or_default();
    let answer = handle(state.admission.as_ref(), review).await;
    let allowed = answer.response.as_ref().is_some_and(|r| r.allowed);
    state.decisions.add(
        1,
        &[
            KeyValue::new("kind", kind),
            KeyValue::new("result", if allowed { "allow" } else { "deny" }),
        ],
    );
    Json(answer)
}

/// One review, answered: what `/validate` does, without HTTP nor the
/// counter (for the operator's own tests).
pub async fn handle<A: Admission>(
    admission: &A,
    review: AdmissionReview<DynamicObject>,
) -> AdmissionReview<DynamicObject> {
    let Ok(req): Result<AdmissionRequest<DynamicObject>, _> = review.try_into() else {
        return AdmissionResponse::invalid("AdmissionReview without a request").into_review();
    };
    let decision = decide(admission, &req).await;
    respond(&req, decision)
}

async fn decide<A: Admission>(
    admission: &A,
    req: &AdmissionRequest<DynamicObject>,
) -> Result<Vec<String>, Denial<A::Reason>> {
    if req.operation == Operation::Delete {
        return Ok(Vec::new());
    }
    let Some(object) = req.object.as_ref() else {
        return Err(Denial::new(
            admission.undecidable(),
            "request carries no object",
        ));
    };
    if let Some(old) = req.old_object.as_ref()
        && old.data.get("spec") == object.data.get("spec")
    {
        return Ok(Vec::new());
    }
    let namespace = req.namespace.as_deref().unwrap_or_default();
    admission
        .admit_with_warnings(&req.kind.kind, namespace, object)
        .await
}

/// The review answering `req`: allowed with its warnings (none: no
/// `warnings` field), or denied with a 403 and the reason code.
fn respond<R: Reason>(
    req: &AdmissionRequest<DynamicObject>,
    decision: Result<Vec<String>, Denial<R>>,
) -> AdmissionReview<DynamicObject> {
    match decision {
        Ok(warnings) => {
            let mut response = AdmissionResponse::from(req);
            response.warnings = (!warnings.is_empty()).then_some(warnings);
            response.into_review()
        }
        Err(denial) => {
            let mut response = AdmissionResponse::from(req).deny(denial.message);
            response.result.code = 403;
            response.result.reason = denial.reason.as_str().to_string();
            response.into_review()
        }
    }
}

/// What the Vault paths check reads of the instance's Vault connection.
#[derive(Debug, Clone, Copy)]
pub struct VaultPaths<'a> {
    pub path_prefix: &'a str,
    pub path_grants: &'a [VaultPathGrant],
}

/// Refuses the Vault targets the secret stores would refuse, with their
/// message: any Vault target when the instance has no Vault connection
/// (`vault: None`); a target breaking the namespace rules
/// ([`emit::check_target`]: a namespaced object naming another namespace,
/// a cluster-scoped one with neither a namespace nor a path); an explicit
/// `path` outside `<pathPrefix>/<namespace>/` that no grant opens to the
/// object. `namespace` is the object's own, `None` when cluster-scoped:
/// the subtree is the object's namespace, or the one its target names when
/// it is cluster-scoped. Kubernetes targets are left alone (the namespace
/// rules of every target: [`emit::check_targets`]). Call it once the
/// object's instance is resolved: an instance not found is the reconcile's
/// to report.
pub fn check_vault_targets(
    targets: &[SecretTarget],
    namespace: Option<&str>,
    vault: Option<VaultPaths<'_>>,
) -> Result<(), String> {
    // Only the namespace matters to the rules (kind and name are not read).
    let owner = match namespace {
        Some(ns) => SecretOwner::namespaced("", ns, ""),
        None => SecretOwner::cluster("", ""),
    };
    for target in targets
        .iter()
        .filter(|t| t.backend == SecretStoreBackend::Vault)
    {
        let Some(vault) = vault else {
            return Err(format!(
                "secret target {:?} writes to Vault, but the instance configures no Vault connection",
                target.name
            ));
        };
        emit::check_target(target, &owner).map_err(|e| e.to_string())?;
        let Some(path) = &target.path else {
            continue;
        };
        let grants: Vec<PathGrant<'_>> = vault
            .path_grants
            .iter()
            .map(|g| PathGrant {
                namespaces: &g.namespaces,
                cluster_scoped: g.cluster_scoped,
                paths: &g.paths,
            })
            .collect();
        let writer = namespace.map_or(Writer::Cluster, Writer::Namespaced);
        // The stores' subtree: the object's own namespace (a namespaced
        // object's target may only repeat it), the target's for a
        // cluster-scoped one.
        let subtree = namespace.or(target.namespace.as_deref());
        vault_paths::check(vault.path_prefix, &grants, writer, subtree, path)
            .map_err(|e| format!("secret target {:?}: {e}", target.name))?;
    }
    Ok(())
}

/// Refuses an `UPDATE` that takes an object out of the operator's label
/// selector while it still carries the operator's finalizer: the operator
/// would stop seeing it, its remote object would no longer be managed and
/// its deletion would hang in `Terminating`. The way out is the
/// `force-orphan` annotation (in the same update or before), or moving it
/// to another release's selector in one write and removing the
/// finalizer by hand. Served at `POST /scope` by [`scope_router`], for
/// every kind of the operator (the chart's `scope.<group>` webhook).
pub struct ScopeGuard<R> {
    pub scope: WatchScope,
    /// The operator's finalizer.
    pub finalizer: &'static str,
    /// Where `force-orphan` is read.
    pub annotations: Annotations,
    /// The operator's reason code for the refusal.
    pub reason: R,
}

impl<R: Reason> ScopeGuard<R> {
    /// The decision on one request. Anything but such an `UPDATE` passes.
    pub fn check(&self, req: &AdmissionRequest<DynamicObject>) -> Result<(), Denial<R>> {
        if req.operation != Operation::Update || self.scope.selector().is_none() {
            return Ok(());
        }
        let (Some(object), Some(old)) = (req.object.as_ref(), req.old_object.as_ref()) else {
            return Ok(());
        };
        let leaves = self.scope.selects_labels(old.metadata.labels.as_ref())
            && !self.scope.selects_labels(object.metadata.labels.as_ref());
        let managed = object
            .metadata
            .finalizers
            .as_ref()
            .is_some_and(|f| f.iter().any(|f| f == self.finalizer));
        if !leaves || !managed || self.annotations.force_orphan(&object.metadata) {
            return Ok(());
        }
        Err(Denial::new(
            self.reason,
            format!(
                "this change takes the object out of the operator's label selector while it \
                 still carries the {} finalizer: it would no longer be managed and its \
                 deletion would hang; set the {} annotation first, or remove the \
                 finalizer once another release manages it",
                self.finalizer,
                self.annotations
                    .key(weebo_kit_api::annotations::FORCE_ORPHAN),
            ),
        ))
    }
}

/// `POST /scope` deciding through `guard`. Merged into the operator's
/// webhook router; without a label selector every request passes.
pub fn scope_router<R: Reason + Send + Sync + 'static>(guard: Arc<ScopeGuard<R>>) -> Router {
    Router::new()
        .route("/scope", post(scope::<R>))
        .with_state(guard)
}

async fn scope<R: Reason + Send + Sync + 'static>(
    State(guard): State<Arc<ScopeGuard<R>>>,
    Json(review): Json<AdmissionReview<DynamicObject>>,
) -> Json<AdmissionReview<DynamicObject>> {
    let Ok(req): Result<AdmissionRequest<DynamicObject>, _> = review.try_into() else {
        return Json(AdmissionResponse::invalid("AdmissionReview without a request").into_review());
    };
    let decision = guard.check(&req).map(|()| Vec::new());
    Json(respond(&req, decision))
}

#[cfg(test)]
mod tests {
    use weebo_kit_domain::Severity;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Code {
        Ok,
        NotAllowed,
    }

    impl Reason for Code {
        const SUCCESS: Self = Code::Ok;
        fn as_str(self) -> &'static str {
            match self {
                Code::Ok => "Reconciled",
                Code::NotAllowed => "NamespaceNotAllowed",
            }
        }
        fn severity(self) -> Severity {
            Severity::Blocking
        }
    }

    /// Allows `team-a` only.
    struct OnlyTeamA;

    #[async_trait::async_trait]
    impl Admission for OnlyTeamA {
        type Reason = Code;
        async fn admit(&self, kind: &str, ns: &str, _: &DynamicObject) -> Result<(), Denial<Code>> {
            if kind == "Thing" && ns == "team-a" {
                Ok(())
            } else {
                Err(Denial::new(
                    Code::NotAllowed,
                    format!("{kind} not allowed in {ns}"),
                ))
            }
        }
        fn undecidable(&self) -> Code {
            Code::NotAllowed
        }
    }

    fn request(
        operation: &str,
        ns: &str,
        spec: i32,
        old_spec: Option<i32>,
    ) -> AdmissionRequest<DynamicObject> {
        let object = |s: i32| {
            serde_json::json!({"apiVersion": "x.weebo.io/v1", "kind": "Thing",
                "metadata": {"name": "t", "namespace": ns}, "spec": {"v": s}})
        };
        let review: AdmissionReview<DynamicObject> = serde_json::from_value(serde_json::json!({
            "apiVersion": "admission.k8s.io/v1", "kind": "AdmissionReview",
            "request": {
                "uid": "11111111-1111-1111-1111-111111111111",
                "namespace": ns,
                "kind": {"group": "x.weebo.io", "version": "v1", "kind": "Thing"},
                "resource": {"group": "x.weebo.io", "version": "v1", "resource": "things"},
                "operation": operation,
                "userInfo": {},
                "object": object(spec),
                "oldObject": old_spec.map(object),
            }
        }))
        .unwrap();
        review.try_into().unwrap()
    }

    #[tokio::test]
    async fn denials_carry_403_and_the_reason() {
        let req = request("CREATE", "team-b", 1, None);
        let review = respond(&req, decide(&OnlyTeamA, &req).await);
        let response = review.response.unwrap();
        assert!(!response.allowed);
        assert_eq!(response.result.code, 403);
        assert_eq!(response.result.reason, "NamespaceNotAllowed");
        assert_eq!(response.result.message, "Thing not allowed in team-b");

        let ok = request("CREATE", "team-a", 1, None);
        assert!(
            respond(&ok, decide(&OnlyTeamA, &ok).await)
                .response
                .unwrap()
                .allowed
        );
    }

    /// [`OnlyTeamA`], warning about a `spec.v` above 1.
    struct WarnsOnBigV;

    #[async_trait::async_trait]
    impl Admission for WarnsOnBigV {
        type Reason = Code;
        async fn admit(
            &self,
            kind: &str,
            ns: &str,
            object: &DynamicObject,
        ) -> Result<(), Denial<Code>> {
            self.admit_with_warnings(kind, ns, object).await.map(drop)
        }
        async fn admit_with_warnings(
            &self,
            kind: &str,
            ns: &str,
            object: &DynamicObject,
        ) -> Result<Vec<String>, Denial<Code>> {
            OnlyTeamA.admit(kind, ns, object).await?;
            let big = object.data["spec"]["v"].as_i64().is_some_and(|v| v > 1);
            Ok(big
                .then(|| "spec.v above 1 is ignored".to_string())
                .into_iter()
                .collect())
        }
        fn undecidable(&self) -> Code {
            Code::NotAllowed
        }
    }

    /// The review, through the `/validate` route (JSON in and out).
    async fn validate_json<A: Admission>(
        admission: A,
        review: serde_json::Value,
    ) -> serde_json::Value {
        use tower::ServiceExt;
        let app = router(
            Arc::new(admission),
            Arc::new(Health::default()),
            "test",
            "test",
        );
        let response = app
            .oneshot(
                axum::http::Request::post("/validate")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(review.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn review_json(ns: &str, spec: i32) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "admission.k8s.io/v1", "kind": "AdmissionReview",
            "request": {
                "uid": "11111111-1111-1111-1111-111111111111",
                "namespace": ns,
                "kind": {"group": "x.weebo.io", "version": "v1", "kind": "Thing"},
                "resource": {"group": "x.weebo.io", "version": "v1", "resource": "things"},
                "operation": "CREATE",
                "userInfo": {},
                "object": {"apiVersion": "x.weebo.io/v1", "kind": "Thing",
                    "metadata": {"name": "t", "namespace": ns}, "spec": {"v": spec}},
            }
        })
    }

    #[tokio::test]
    async fn warnings_of_an_admitted_object_reach_the_review() {
        let warned = validate_json(WarnsOnBigV, review_json("team-a", 2)).await;
        assert_eq!(warned["response"]["allowed"], true);
        assert_eq!(
            warned["response"]["warnings"],
            serde_json::json!(["spec.v above 1 is ignored"])
        );
        let quiet = validate_json(WarnsOnBigV, review_json("team-a", 1)).await;
        assert_eq!(quiet["response"]["allowed"], true);
        assert!(quiet["response"].get("warnings").is_none(), "{quiet}");
        let denied = validate_json(WarnsOnBigV, review_json("team-b", 2)).await;
        assert_eq!(denied["response"]["allowed"], false);
        assert_eq!(denied["response"]["status"]["code"], 403);
        assert!(denied["response"].get("warnings").is_none(), "{denied}");
    }

    #[tokio::test]
    async fn an_admission_that_only_admits_sends_no_warning() {
        // `OnlyTeamA` implements `admit` alone: the provided
        // `admit_with_warnings` decides the same, without a warning.
        let req = request("CREATE", "team-a", 2, None);
        assert_eq!(
            OnlyTeamA
                .admit_with_warnings("Thing", "team-a", req.object.as_ref().unwrap())
                .await,
            Ok(vec![])
        );
        let allowed = validate_json(OnlyTeamA, review_json("team-a", 2)).await;
        assert_eq!(allowed["response"]["allowed"], true);
        assert!(allowed["response"].get("warnings").is_none(), "{allowed}");
        let denied = validate_json(OnlyTeamA, review_json("team-b", 2)).await;
        assert_eq!(denied["response"]["allowed"], false);
        assert_eq!(
            denied["response"]["status"]["reason"],
            "NamespaceNotAllowed"
        );
    }

    #[tokio::test]
    async fn deletes_and_spec_preserving_updates_always_pass() {
        let delete = request("DELETE", "team-b", 1, None);
        assert_eq!(decide(&OnlyTeamA, &delete).await, Ok(vec![]));
        let finalizer_removal = request("UPDATE", "team-b", 1, Some(1));
        assert_eq!(decide(&OnlyTeamA, &finalizer_removal).await, Ok(vec![]));
        let spec_change = request("UPDATE", "team-b", 2, Some(1));
        assert!(decide(&OnlyTeamA, &spec_change).await.is_err());
    }

    fn labelled(
        operation: &str,
        labels: serde_json::Value,
        old_labels: serde_json::Value,
        annotations: serde_json::Value,
    ) -> AdmissionRequest<DynamicObject> {
        let object = |labels: &serde_json::Value| {
            serde_json::json!({"apiVersion": "x.weebo.io/v1", "kind": "Thing",
                "metadata": {"name": "t", "namespace": "a", "labels": labels,
                    "annotations": annotations, "finalizers": ["x.weebo.io/finalizer"]},
                "spec": {"v": 1}})
        };
        let review: AdmissionReview<DynamicObject> = serde_json::from_value(serde_json::json!({
            "apiVersion": "admission.k8s.io/v1", "kind": "AdmissionReview",
            "request": {
                "uid": "11111111-1111-1111-1111-111111111111",
                "namespace": "a",
                "kind": {"group": "x.weebo.io", "version": "v1", "kind": "Thing"},
                "resource": {"group": "x.weebo.io", "version": "v1", "resource": "things"},
                "operation": operation,
                "userInfo": {},
                "object": object(&labels),
                "oldObject": object(&old_labels),
            }
        }))
        .unwrap();
        review.try_into().unwrap()
    }

    fn guard(selector: &str) -> ScopeGuard<Code> {
        let scope = WatchScope::cluster()
            .with_labels(serde_json::from_str(selector).unwrap())
            .unwrap();
        ScopeGuard {
            scope,
            finalizer: "x.weebo.io/finalizer",
            annotations: Annotations::new("x.weebo.io"),
            reason: Code::NotAllowed,
        }
    }

    #[test]
    fn leaving_the_selector_with_the_finalizer_is_refused() {
        use serde_json::json;
        let guard = guard(r#"{"matchLabels": {"shard": "a"}}"#);
        let leaves = labelled(
            "UPDATE",
            json!({"shard": "b"}),
            json!({"shard": "a"}),
            json!({}),
        );
        let denial = guard.check(&leaves).unwrap_err();
        assert_eq!(denial.reason, Code::NotAllowed);
        assert!(denial.message.contains("x.weebo.io/force-orphan"));

        let orphaned = labelled(
            "UPDATE",
            json!({"shard": "b"}),
            json!({"shard": "a"}),
            json!({"x.weebo.io/force-orphan": "true"}),
        );
        assert_eq!(guard.check(&orphaned), Ok(()));
        let enters = labelled(
            "UPDATE",
            json!({"shard": "a"}),
            json!({"shard": "b"}),
            json!({}),
        );
        assert_eq!(guard.check(&enters), Ok(()));
        let stays = labelled(
            "UPDATE",
            json!({"shard": "a", "x": "y"}),
            json!({"shard": "a"}),
            json!({}),
        );
        assert_eq!(guard.check(&stays), Ok(()));
    }

    #[test]
    fn without_a_selector_the_guard_lets_everything_through() {
        use serde_json::json;
        let mut guard = guard("{}");
        guard.scope = WatchScope::cluster();
        let leaves = labelled("UPDATE", json!({}), json!({"shard": "a"}), json!({}));
        assert_eq!(guard.check(&leaves), Ok(()));
    }

    #[test]
    fn typed_decodes_or_explains() {
        #[derive(serde::Deserialize)]
        struct Thing {
            spec: serde_json::Value,
        }
        let req = request("CREATE", "a", 3, None);
        let thing: Thing = typed(req.object.as_ref().unwrap()).unwrap();
        assert_eq!(thing.spec["v"], 3);
        #[derive(Debug, serde::Deserialize)]
        #[expect(dead_code, reason = "only deserialized, to be refused")]
        struct Strict {
            missing: String,
        }
        assert!(typed::<Strict>(req.object.as_ref().unwrap()).is_err());
    }

    fn vault_target(name: &str, path: Option<&str>) -> SecretTarget {
        SecretTarget {
            backend: SecretStoreBackend::Vault,
            name: name.into(),
            namespace: None,
            path: path.map(String::from),
        }
    }

    #[test]
    fn vault_targets_are_checked_like_the_stores_do() {
        let grants = [VaultPathGrant {
            namespaces: vec!["auth".into()],
            cluster_scoped: false,
            paths: vec!["shared/auth".into()],
        }];
        let vault = Some(VaultPaths {
            path_prefix: "weebo",
            path_grants: &grants,
        });
        let granted = [vault_target("app", Some("shared/auth"))];
        assert_eq!(check_vault_targets(&granted, Some("auth"), vault), Ok(()));
        let err = check_vault_targets(&granted, Some("team-a"), vault).unwrap_err();
        assert!(err.starts_with("secret target \"app\": "), "{err}");
        assert!(err.contains("namespace \"team-a\""), "{err}");
        let own = [
            vault_target("app", Some("weebo/team-a/app")),
            vault_target("x", None),
        ];
        assert_eq!(check_vault_targets(&own, Some("team-a"), vault), Ok(()));
        assert!(check_vault_targets(&granted, None, vault).is_err());
    }

    #[test]
    fn vault_targets_follow_the_stores_namespace_rules() {
        let grants = [VaultPathGrant {
            namespaces: vec!["team-*".into()],
            cluster_scoped: false,
            paths: vec!["shared/auth".into()],
        }];
        let vault = Some(VaultPaths {
            path_prefix: "weebo",
            path_grants: &grants,
        });
        // A namespaced object naming another namespace: refused like the
        // stores do (CrossNamespace), even with a path in that subtree.
        let elsewhere = [SecretTarget {
            namespace: Some("team-b".into()),
            ..vault_target("x", Some("weebo/team-b/x"))
        }];
        let err = check_vault_targets(&elsewhere, Some("team-a"), vault).unwrap_err();
        assert!(
            err.contains("may only write into its own namespace"),
            "{err}"
        );
        // Its own namespace repeated: fine.
        let own = [SecretTarget {
            namespace: Some("team-a".into()),
            ..vault_target("x", Some("weebo/team-a/x"))
        }];
        assert_eq!(check_vault_targets(&own, Some("team-a"), vault), Ok(()));
        // The subtree is the object's namespace, not the target's.
        let other_subtree = [vault_target("x", Some("weebo/team-b/x"))];
        assert!(check_vault_targets(&other_subtree, Some("team-a"), vault).is_err());
        // A cluster-scoped object: neither a namespace nor a path.
        let err = check_vault_targets(&[vault_target("x", None)], None, vault).unwrap_err();
        assert!(err.contains("needs a namespace"), "{err}");
        // With a namespace, its subtree; with a path only, a cluster grant.
        let named = [SecretTarget {
            namespace: Some("team-b".into()),
            ..vault_target("x", None)
        }];
        assert_eq!(check_vault_targets(&named, None, vault), Ok(()));
        let in_subtree = [SecretTarget {
            namespace: Some("team-b".into()),
            ..vault_target("x", Some("weebo/team-b/x"))
        }];
        assert_eq!(check_vault_targets(&in_subtree, None, vault), Ok(()));
        // Kubernetes targets are not this check's.
        let kubernetes = [SecretTarget {
            backend: SecretStoreBackend::Kubernetes,
            namespace: Some("team-b".into()),
            ..vault_target("x", None)
        }];
        assert_eq!(
            check_vault_targets(&kubernetes, Some("team-a"), vault),
            Ok(())
        );
    }

    #[test]
    fn a_vault_target_needs_a_vault_connection() {
        let kubernetes = [SecretTarget {
            backend: SecretStoreBackend::Kubernetes,
            ..vault_target("app", None)
        }];
        assert_eq!(check_vault_targets(&kubernetes, Some("a"), None), Ok(()));
        let err = check_vault_targets(&[vault_target("app", None)], Some("a"), None).unwrap_err();
        assert!(err.contains("no Vault connection"), "{err}");
    }
}
