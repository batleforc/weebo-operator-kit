//! Publishing a reconcile's result on the CR: one merge patch on the
//! `status` subresource — the condition list, `observedGeneration` on
//! success, the operator's own fields (remote id…) — retried while the
//! apiserver is unavailable, then the Events announcing it
//! ([`crate::events::announce_outcome`]).

use std::fmt::Debug;
use std::time::Duration;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, Patch, PatchParams};
use kube::{Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use weebo_kit_api::Condition;
use weebo_kit_domain::{Advisory, Failure, Reason};

use crate::conditions::conditions;
use crate::events;

/// Extra attempts of a failed status patch.
pub const STATUS_RETRIES: u32 = 3;

/// What a reconcile ended on.
pub struct StatusUpdate<'a, R> {
    /// `Ok(message)` for `Ready: True`.
    pub ready: &'a Result<String, Failure<R>>,
    pub advisories: &'a [Advisory<R>],
    /// The operator's own status fields (`remoteId`…), merged as is; a
    /// `null` drops a field.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// Worth retrying: the apiserver unreachable, overloaded or timing out —
/// not a patch it refuses.
pub fn transient(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(e) => e.code == 429 || e.code >= 500,
        _ => true,
    }
}

/// Patches `update` onto `obj` (read fresh from the apiserver: its
/// conditions are the previous ones, its generation the observed one) and
/// announces it. `observedGeneration` only advances on success, so a
/// change that failed to apply is sent again. Retried in place on a
/// transient error: a lost patch after a create would leave the new remote
/// object untracked. Returns the conditions written.
pub async fn publish<K, R>(
    api: &Api<K>,
    recorder: &kube::runtime::events::Recorder,
    field_manager: &str,
    obj: &K,
    previous: &[Condition],
    update: StatusUpdate<'_, R>,
) -> Result<Vec<Condition>, kube::Error>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Serialize,
    R: Reason,
{
    let generation = obj.meta().generation;
    let conds = conditions(
        previous,
        update.ready,
        update.advisories,
        generation,
        &Time(Timestamp::now()),
    );
    let mut status = update.fields;
    status.insert(
        "conditions".into(),
        serde_json::to_value(&conds).unwrap_or_default(),
    );
    if update.ready.is_ok() {
        status.insert("observedGeneration".into(), generation.into());
    }
    let patch = Patch::Merge(serde_json::json!({ "status": status }));
    let mut attempt = 0;
    loop {
        match api
            .patch_status(&obj.name_any(), &PatchParams::apply(field_manager), &patch)
            .await
        {
            Ok(_) => break,
            Err(err) if attempt < STATUS_RETRIES && transient(&err) => {
                attempt += 1;
                tracing::warn!(error = %err, attempt, "status patch failed, retrying");
                tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(attempt))).await;
            }
            Err(err) => return Err(err),
        }
    }
    events::announce_outcome(recorder, &obj.object_ref(&()), previous, &conds).await;
    Ok(conds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_error(code: u16) -> kube::Error {
        kube::Error::Api(
            kube::core::Status::failure("x", "y")
                .with_code(code)
                .boxed(),
        )
    }

    #[test]
    fn only_unavailability_is_retried() {
        assert!(transient(&api_error(503)));
        assert!(transient(&api_error(429)));
        assert!(!transient(&api_error(422)));
        assert!(!transient(&api_error(404)));
    }
}
