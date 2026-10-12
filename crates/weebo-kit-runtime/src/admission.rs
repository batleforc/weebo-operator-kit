//! The validating admission webhook's `/validate` endpoint: decoding the
//! review, the decisions counter, the denial shape (403 + reason code),
//! failing closed — around the operator's own decision ([`Admission`]).
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
use weebo_kit_api::Annotations;
use weebo_kit_domain::Reason;

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

    /// The reason of a request that can't be decided on at all (no
    /// object).
    fn undecidable(&self) -> Self::Reason;
}

/// The typed CR behind a review's `DynamicObject`.
pub fn typed<K: DeserializeOwned>(object: &DynamicObject) -> Result<K, String> {
    let value = serde_json::to_value(object).map_err(|e| e.to_string())?;
    serde_json::from_value(value).map_err(|e| format!("object does not match the CRD: {e}"))
}

struct AdmissionState<A> {
    admission: Arc<A>,
    decisions: Counter<u64>,
}

/// `POST /validate` deciding through `admission`, plus the kit's
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
) -> Result<(), Denial<A::Reason>> {
    if req.operation == Operation::Delete {
        return Ok(());
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
        return Ok(());
    }
    let namespace = req.namespace.as_deref().unwrap_or_default();
    admission.admit(&req.kind.kind, namespace, object).await
}

fn respond<R: Reason>(
    req: &AdmissionRequest<DynamicObject>,
    decision: Result<(), Denial<R>>,
) -> AdmissionReview<DynamicObject> {
    match decision {
        Ok(()) => AdmissionResponse::from(req).into_review(),
        Err(denial) => {
            let mut response = AdmissionResponse::from(req).deny(denial.message);
            response.result.code = 403;
            response.result.reason = denial.reason.as_str().to_string();
            response.into_review()
        }
    }
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
    let decision = guard.check(&req);
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

    #[tokio::test]
    async fn deletes_and_spec_preserving_updates_always_pass() {
        let delete = request("DELETE", "team-b", 1, None);
        assert_eq!(decide(&OnlyTeamA, &delete).await, Ok(()));
        let finalizer_removal = request("UPDATE", "team-b", 1, Some(1));
        assert_eq!(decide(&OnlyTeamA, &finalizer_removal).await, Ok(()));
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
        #[allow(dead_code)]
        struct Strict {
            missing: String,
        }
        assert!(typed::<Strict>(req.object.as_ref().unwrap()).is_err());
    }
}
