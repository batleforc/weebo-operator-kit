//! Publishing a reconcile's result on the CR: one merge patch on the
//! `status` subresource — the condition list, `observedGeneration` on
//! success, the operator's own fields (remote id…) — retried while the
//! apiserver is unavailable, then the Events announcing it
//! ([`crate::events::announce_outcome`]). And [`record`]: a status field
//! written as soon as the remote write it records, with the same retries.

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

use crate::conditions::{TypedCondition, conditions_with};
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
/// not a patch it refuses, nor one that can't be built or read (the same
/// (de)serialization error would come back).
pub fn transient(err: &kube::Error) -> bool {
    match err {
        kube::Error::Api(e) => e.code == 429 || e.code >= 500,
        kube::Error::HyperError(_)
        | kube::Error::Service(_)
        | kube::Error::ReadEvents(_)
        | kube::Error::Auth(_) => true,
        _ => false,
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
    publish_with(api, recorder, field_manager, obj, previous, update, &[]).await
}

/// [`publish`], with conditions of the operator's own types after `Ready`
/// and the advisories ([`TypedCondition`]): written in the same patch, so
/// the next publish keeps them instead of dropping them.
pub async fn publish_with<K, R>(
    api: &Api<K>,
    recorder: &kube::runtime::events::Recorder,
    field_manager: &str,
    obj: &K,
    previous: &[Condition],
    update: StatusUpdate<'_, R>,
    extra: &[TypedCondition<R>],
) -> Result<Vec<Condition>, kube::Error>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Serialize,
    R: Reason,
{
    let generation = obj.meta().generation;
    let conds = conditions_with(
        previous,
        update.ready,
        update.advisories,
        extra,
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
    let name = obj.name_any();
    let params = PatchParams::apply(field_manager);
    retrying(|| api.patch_status(&name, &params, &patch)).await?;
    events::announce_outcome(recorder, &obj.object_ref(&()), previous, &conds).await;
    Ok(conds)
}

/// Merges `fields` into the `status` of `name` — one merge patch, retried
/// on a transient error like [`publish`]'s. For what a reconcile must
/// record as soon as it did it, outside the final [`publish`]: the id of
/// a remote object it just created, a hash of what it just wrote remotely.
/// A lost patch there leaves the CR at odds with its own remote object
/// (the next reconcile sees a "conflict" with itself); retrying in place
/// narrows that window to an apiserver unavailable for the whole backoff.
/// `fields` serializes to a JSON object of status fields (`null` drops
/// one); a value that can't be serialized fails at once
/// (`kube::Error::SerdeError`), never retried. Returns the patched object.
pub async fn record<K, T>(
    api: &Api<K>,
    field_manager: &str,
    name: &str,
    fields: &T,
) -> Result<K, kube::Error>
where
    K: Clone + Debug + DeserializeOwned,
    T: Serialize + ?Sized,
{
    let fields = serde_json::to_value(fields).map_err(kube::Error::SerdeError)?;
    let patch = Patch::Merge(serde_json::json!({ "status": fields }));
    let params = PatchParams::apply(field_manager);
    retrying(|| api.patch_status(name, &params, &patch)).await
}

/// `op`, tried again up to [`STATUS_RETRIES`] times on a [`transient`]
/// error, with a doubling backoff (0.5 s, 1 s, 2 s).
async fn retrying<T, F, Fut>(mut op: F) -> Result<T, kube::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, kube::Error>>,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(done) => return Ok(done),
            Err(err) if attempt < STATUS_RETRIES && transient(&err) => {
                attempt = attempt.saturating_add(1);
                tracing::warn!(error = %err, attempt, "status patch failed, retrying");
                let backoff = 250u64.saturating_mul(2u64.saturating_pow(attempt));
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
            Err(err) => return Err(err),
        }
    }
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

    #[tokio::test(start_paused = true)]
    async fn patches_are_retried_on_transient_errors_only() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        // Two 503s, then success.
        let done = retrying(|| async {
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 | 1 => Err(api_error(503)),
                _ => Ok(7),
            }
        })
        .await;
        assert_eq!(done.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // Unavailable throughout: given up after the retries.
        calls.store(0, Ordering::SeqCst);
        let failed: Result<(), _> = retrying(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(api_error(503))
        })
        .await;
        assert!(failed.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), STATUS_RETRIES + 1);
        // Refused or unreadable: never retried.
        for err in [
            || api_error(422),
            || kube::Error::SerdeError(serde_json::from_str::<u8>("x").unwrap_err()),
        ] {
            calls.store(0, Ordering::SeqCst);
            let failed: Result<(), _> = retrying(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(err())
            })
            .await;
            assert!(failed.is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn only_unavailability_is_retried() {
        assert!(transient(&api_error(503)));
        assert!(transient(&api_error(429)));
        assert!(!transient(&api_error(422)));
        assert!(!transient(&api_error(404)));
        let serde = serde_json::from_str::<u8>("x").unwrap_err();
        assert!(!transient(&kube::Error::SerdeError(serde)));
        let io = std::io::Error::other("reset");
        assert!(transient(&kube::Error::ReadEvents(io)));
    }
}
