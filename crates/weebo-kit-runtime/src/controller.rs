//! The skeleton every controller of every operator shares: what triggers a
//! reconcile, how many run at once, how the result stream is drained, how
//! long until the next one, what an error announces, and the finalizer's
//! force-orphan way out. The reconcile itself stays the operator's.

use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use k8s_openapi::api::core::v1::ObjectReference;
use kube::Resource;
use kube::api::Api;
use kube::runtime::controller::{self as runtime_controller, Action};
use kube::runtime::events::Recorder;
use kube::runtime::predicates;
use kube::runtime::reflector::{self, ObjectRef};
use kube::runtime::utils::Predicate;
use kube::runtime::{Controller, WatchStreamExt, watcher};
use weebo_kit_api::Annotations;
use weebo_kit_domain::Reason;

use crate::events;
use crate::{Health, WatchScope};

/// Default of `WEEBO_RECONCILE_CONCURRENCY`: reconciles running at once per
/// controller. Bounded so a startup sweep or a policy change (re-checking
/// every CR of a kind) can't flood the remote system and the apiserver.
pub const DEFAULT_CONCURRENCY: u16 = 4;

/// Steady-state drift check after a success, seconds.
pub const STEADY_REQUEUE_SECS: u64 = 300;
/// Retry after a failure (often a dependency not ready yet), seconds.
pub const RETRY_REQUEUE_SECS: u64 = 30;
/// The soonest a reconcile is requeued, seconds: a due time already past
/// (`due_in: Some(0)`) never spins the controller.
pub const MIN_REQUEUE_SECS: u64 = 1;

/// `WEEBO_RECONCILE_CONCURRENCY`, or [`DEFAULT_CONCURRENCY`].
pub fn concurrency() -> u16 {
    std::env::var("WEEBO_RECONCILE_CONCURRENCY")
        .ok()
        .and_then(|c| c.parse().ok())
        .filter(|c| *c > 0)
        .unwrap_or(DEFAULT_CONCURRENCY)
}

/// Hash of `deletionTimestamp`: a deletion must always reach the finalizer.
fn deletion<K: Resource>(obj: &K) -> Option<u64> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    obj.meta().deletion_timestamp.as_ref().map(|t| {
        let mut h = DefaultHasher::new();
        t.0.to_string().hash(&mut h);
        h.finish()
    })
}

/// The controller of `api`'s kind. Its primary watch only fires on what
/// reconciling acts on — spec (`generation`), annotations, labels,
/// finalizers, deletion — never on the CR's own status patches, which
/// would otherwise re-trigger it (and, through watches on referenced
/// kinds, its dependents) in a loop. Watch errors back off exponentially;
/// [`concurrency`] reconciles at once.
///
/// `api` comes from `scope.api::<K>()`; `scope` adds its label selector to
/// the watch. The `watches` on referenced kinds go through the same scope
/// (`scope.api()`, `scope.watcher_config()`).
pub fn controller<K>(api: Api<K>, scope: &WatchScope) -> Controller<K>
where
    K: Resource<DynamicType = ()>
        + Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    let (reader, writer) = reflector::store();
    let trigger = watcher(api, scope.watcher_config())
        .default_backoff()
        .reflect(writer)
        .applied_objects()
        .predicate_filter(
            predicates::generation
                .combine(predicates::annotations)
                .combine(predicates::labels)
                .combine(predicates::finalizers)
                .combine(deletion),
            Default::default(),
        );
    Controller::for_stream(trigger, reader)
        .with_config(runtime_controller::Config::default().concurrency(concurrency()))
}

/// Drains a controller's result stream until shutdown, reporting watch
/// failures to `health` (a watch erroring in a loop fails `/livez`).
/// Reconcile failures are logged; they show on the CR (status, Events).
/// A reconcile succeeding says nothing of the watch (it runs from the
/// requeue timers even while the watch is broken): the watch's failure
/// streak only ends after [`crate::health::FAILURE_STREAK_GAP`] without a
/// new error.
pub async fn drive<K, E, S>(kind: &'static str, health: Arc<Health>, results: S)
where
    K: Resource,
    E: std::error::Error + Send + Sync + 'static,
    S: Stream<Item = Result<(ObjectRef<K>, Action), runtime_controller::Error<E, watcher::Error>>>,
{
    results
        .for_each(|res| {
            match res {
                Ok(_) => {}
                Err(runtime_controller::Error::QueueError(err)) => {
                    tracing::warn!(kind, error = %err, "watch error");
                    health.failed(kind, err.to_string());
                }
                Err(err) => tracing::warn!(kind, error = %err, "reconcile failed"),
            }
            futures::future::ready(())
        })
        .await;
}

/// `secs` ±20%: CRs created or restarted together don't stay in lockstep.
pub fn jittered(secs: u64) -> Duration {
    let spread = secs / 5;
    if spread == 0 {
        return Duration::from_secs(secs.max(1));
    }
    // No RNG needed: the clock's nanoseconds are spread enough.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    Duration::from_secs((secs - spread + nanos % (2 * spread + 1)).max(1))
}

/// When to reconcile again after a reconcile that reached its status:
/// [`STEADY_REQUEUE_SECS`] after a success — or a failure only a change of
/// the CR can clear (a change triggers a reconcile by itself) —,
/// [`RETRY_REQUEUE_SECS`] after any other failure; jittered. `due_in`
/// (seconds, e.g. a token expiring) comes first when sooner.
pub fn requeue_after(steady: bool, due_in: Option<u64>) -> Action {
    let base = if steady {
        STEADY_REQUEUE_SECS
    } else {
        RETRY_REQUEUE_SECS
    };
    match due_in {
        Some(due) if due < base => Action::requeue(Duration::from_secs(due.max(MIN_REQUEUE_SECS))),
        _ => Action::requeue(jittered(base)),
    }
}

/// The body of every `error_policy`: an error that stopped a reconcile
/// before its status (API call, finalizer, cleanup) is logged, announced by
/// a `Warning` Event on the CR (`reason`, `action` = [`events::RECONCILE`]
/// or [`events::CLEANUP`]) and retried in ~[`RETRY_REQUEUE_SECS`].
pub fn retry_after_error<R: Reason>(
    recorder: &Recorder,
    object: ObjectReference,
    reason: R,
    action: &'static str,
    error: &dyn std::fmt::Display,
) -> Action {
    let after = jittered(RETRY_REQUEUE_SECS);
    tracing::warn!(%error, retry_in = after.as_secs(), "reconcile error");
    events::spawn_error(recorder, object, reason, action, error.to_string());
    Action::requeue(after)
}

/// A finalizer's cleanup, unless the CR carries `<domain>/force-orphan:
/// "true"`; either way the release is announced (`released`, a `Normal`
/// Event). A failed cleanup is returned: `error_policy` warns on the CR.
/// [`finalize_with`] with nothing to run on every deletion.
pub async fn finalize<K, R, E>(
    obj: &K,
    recorder: &Recorder,
    annotations: &Annotations,
    released: R,
    cleanup: impl Future<Output = Result<(), E>>,
) -> Result<Action, E>
where
    K: Resource<DynamicType = ()>,
    R: Reason,
{
    finalize_with(obj, recorder, annotations, released, cleanup, async {
        Ok(())
    })
    .await
}

/// [`finalize`], plus `always`: what the operator must undo on every
/// deletion, force-orphaned or not — what it granted *on the cluster*
/// for the CR rather than on the remote system (a `ClusterRoleBinding`
/// given to an instance, a `NetworkPolicy`), which nothing else would
/// remove once the finalizer is gone. `force-orphan` only means "leave
/// the remote objects". `always` runs after `cleanup` succeeded (or was
/// skipped); a failure of either is returned before the release and the
/// whole finalizer runs again, so both must be idempotent.
pub async fn finalize_with<K, R, E>(
    obj: &K,
    recorder: &Recorder,
    annotations: &Annotations,
    released: R,
    cleanup: impl Future<Output = Result<(), E>>,
    always: impl Future<Output = Result<(), E>>,
) -> Result<Action, E>
where
    K: Resource<DynamicType = ()>,
    R: Reason,
{
    let orphaned = annotations.force_orphan(obj.meta());
    if orphaned {
        tracing::warn!(
            kind = %K::kind(&()),
            name = %kube::ResourceExt::name_any(obj),
            "force-orphaned: releasing the finalizer without cleanup"
        );
    }
    release(orphaned, cleanup, always).await?;
    let note = if orphaned {
        format!(
            "force-orphaned ({}): released without cleanup",
            annotations.key(weebo_kit_api::annotations::FORCE_ORPHAN)
        )
    } else {
        "cleanup finished, finalizer released".to_string()
    };
    events::announce_change(
        recorder,
        &obj.object_ref(&()),
        released,
        events::CLEANUP,
        &note,
    )
    .await;
    Ok(Action::await_change())
}

/// `cleanup` unless `orphaned`, then `always`.
async fn release<E>(
    orphaned: bool,
    cleanup: impl Future<Output = Result<(), E>>,
    always: impl Future<Output = Result<(), E>>,
) -> Result<(), E> {
    if !orphaned {
        cleanup.await?;
    }
    always.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_stays_within_twenty_percent() {
        for _ in 0..100 {
            let d = jittered(300).as_secs();
            assert!((240..=360).contains(&d), "{d}");
        }
        assert_eq!(jittered(3), Duration::from_secs(3));
        assert_eq!(jittered(0), Duration::from_secs(1));
    }

    #[test]
    fn requeue_follows_the_outcome_and_a_due_time() {
        let secs = |a: Action| format!("{a:?}");
        assert!(secs(requeue_after(false, Some(5))).contains("5s"));
        // A due time already past: the floor, not a hot loop.
        assert!(secs(requeue_after(true, Some(0))).contains("1s"));
        // A due time later than the cadence doesn't delay it.
        let steady = requeue_after(true, Some(10_000));
        assert!(!format!("{steady:?}").contains("10000"));
    }

    #[tokio::test]
    async fn the_always_hook_runs_even_when_orphaning() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let cleaned = AtomicU32::new(0);
        let always = AtomicU32::new(0);
        let run = |orphaned: bool, cleanup_fails: bool| {
            let (cleaned, always) = (&cleaned, &always);
            release(
                orphaned,
                async move {
                    cleaned.fetch_add(1, Ordering::SeqCst);
                    if cleanup_fails {
                        Err("cleanup")
                    } else {
                        Ok(())
                    }
                },
                async move {
                    always.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
        };
        run(false, false).await.unwrap();
        assert_eq!(
            (
                cleaned.load(Ordering::SeqCst),
                always.load(Ordering::SeqCst)
            ),
            (1, 1)
        );
        // Force-orphaned: no cleanup, the hook still runs.
        run(true, true).await.unwrap();
        assert_eq!(
            (
                cleaned.load(Ordering::SeqCst),
                always.load(Ordering::SeqCst)
            ),
            (1, 2)
        );
        // A failed cleanup stops before the hook (the finalizer runs again).
        assert_eq!(run(false, true).await, Err("cleanup"));
        assert_eq!(
            (
                cleaned.load(Ordering::SeqCst),
                always.load(Ordering::SeqCst)
            ),
            (2, 2)
        );
        // A failed hook is returned too.
        let failed: Result<(), &str> = release(true, async { Ok(()) }, async { Err("hook") }).await;
        assert_eq!(failed, Err("hook"));
    }

    #[test]
    fn concurrency_defaults_to_four() {
        // SAFETY: the variable is only read by `concurrency`, here.
        unsafe { std::env::remove_var("WEEBO_RECONCILE_CONCURRENCY") };
        assert_eq!(concurrency(), DEFAULT_CONCURRENCY);
    }
}
