//! The skeleton every controller of every operator shares: what triggers a
//! reconcile, how many run at once, how the result stream is drained, how
//! long until the next one, what an error announces, and the finalizer's
//! force-orphan way out. The reconcile itself stays the operator's.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
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
/// Retry after a failure (often a dependency not ready yet), seconds;
/// doubled for each retry of the same CR that fails again, up to
/// [`STEADY_REQUEUE_SECS`] (a failure seen sooner, on a watch event, is
/// not counted).
pub const RETRY_REQUEUE_SECS: u64 = 30;
/// CRs whose failure streak is tracked at once; past it, the streaks idle
/// for twice [`STEADY_REQUEUE_SECS`] (a CR deleted while failing, without
/// a finalizer) are dropped, else a newly failing CR retries every
/// [`RETRY_REQUEUE_SECS`].
pub const MAX_FAILURE_STREAKS: usize = 4096;
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

/// What re-triggers a reconcile: spec (`generation`), annotations, labels,
/// finalizers, deletion — never the CR's own status patches.
pub(crate) fn reconcile_triggers<K: Resource + 'static>() -> impl Predicate<K> + 'static {
    predicates::generation
        .combine(predicates::annotations)
        .combine(predicates::labels)
        .combine(predicates::finalizers)
        .combine(deletion)
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
/// (`scope.api()`, `scope.watcher_config()`). The binary's
/// [`crate::shared::SharedWatches`] give the same controller on one watch
/// per kind shared by every controller.
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
        .predicate_filter(reconcile_triggers(), Default::default());
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
    let offset = nanos
        .checked_rem(spread.saturating_mul(2).saturating_add(1))
        .unwrap_or(0);
    Duration::from_secs(secs.saturating_sub(spread).saturating_add(offset).max(1))
}

/// Consecutive failures per CR uid: a CR that keeps failing (remote down,
/// bad credentials) backs off instead of calling the remote system,
/// patching its status and bumping an Event every 30 s forever. Ended by a
/// steady reconcile or the finalizer's release.
static FAILURE_STREAKS: LazyLock<Mutex<HashMap<String, Streak>>> = LazyLock::new(Default::default);

#[derive(Clone, Copy)]
struct Streak {
    failures: u32,
    last: std::time::Instant,
}

/// [`RETRY_REQUEUE_SECS`] doubled `failures` times, up to [`STEADY_REQUEUE_SECS`].
fn backoff_secs(failures: u32) -> u64 {
    RETRY_REQUEUE_SECS
        .saturating_mul(2u64.saturating_pow(failures))
        .min(STEADY_REQUEUE_SECS)
}

/// The retry delay of one more failure of `uid`, counted.
fn failed(uid: Option<&str>) -> u64 {
    failed_at(uid, std::time::Instant::now())
}

/// [`failed`] at `now`. A failure seen before the previous retry was due
/// (a watch event: the CR's own workloads rolling out, a referenced Secret
/// changing) is not counted: only retries back off, not bursts of events.
fn failed_at(uid: Option<&str>, now: std::time::Instant) -> u64 {
    let Some(uid) = uid else {
        return RETRY_REQUEUE_SECS;
    };
    let mut streaks = FAILURE_STREAKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if streaks.len() >= MAX_FAILURE_STREAKS && !streaks.contains_key(uid) {
        let idle = Duration::from_secs(STEADY_REQUEUE_SECS.saturating_mul(2));
        streaks.retain(|_, s| s.last.elapsed() < idle);
        if streaks.len() >= MAX_FAILURE_STREAKS {
            return RETRY_REQUEUE_SECS;
        }
    }
    let streak = streaks.entry(uid.to_string()).or_insert(Streak {
        failures: 0,
        last: now,
    });
    if let Some(previous) = streak.failures.checked_sub(1) {
        let current = backoff_secs(previous);
        // The jittered retry fires at 80% of `current` at the soonest.
        let due = Duration::from_secs(current.saturating_sub(current / 5));
        if now.saturating_duration_since(streak.last) < due {
            return current;
        }
    }
    let secs = backoff_secs(streak.failures);
    streak.failures = streak.failures.saturating_add(1);
    streak.last = now;
    secs
}

/// Ends `uid`'s failure streak.
fn recovered(uid: Option<&str>) {
    if let Some(uid) = uid {
        FAILURE_STREAKS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(uid);
    }
}

/// Ends `object`'s failure streak, for a reconcile that succeeded and
/// chose its own [`Action`] rather than [`requeue_after`]'s: the next
/// failure is retried after [`RETRY_REQUEUE_SECS`] again, not after the
/// backoff the previous streak had reached.
pub fn reconciled<K: Resource>(object: &K) {
    recovered(object.meta().uid.as_deref());
}

/// When to reconcile `object` again after a reconcile that reached its
/// status: [`STEADY_REQUEUE_SECS`] after a success — or a failure only a
/// change of the CR can clear (a change triggers a reconcile by itself) —,
/// [`RETRY_REQUEUE_SECS`] after any other failure, doubled per retry that
/// fails again up to [`STEADY_REQUEUE_SECS`]; jittered. `due_in` (seconds, e.g.
/// a token expiring) comes first when sooner.
pub fn requeue_after<K: Resource>(object: &K, steady: bool, due_in: Option<u64>) -> Action {
    let uid = object.meta().uid.as_deref();
    let base = if steady {
        recovered(uid);
        STEADY_REQUEUE_SECS
    } else {
        failed(uid)
    };
    match due_in {
        Some(due) if due < base => Action::requeue(Duration::from_secs(due.max(MIN_REQUEUE_SECS))),
        _ => Action::requeue(jittered(base)),
    }
}

/// The body of every `error_policy`: an error that stopped a reconcile
/// before its status (API call, finalizer, cleanup) is logged, announced by
/// a `Warning` Event on the CR (`reason`, `action` = [`events::RECONCILE`]
/// or [`events::CLEANUP`]) and retried in ~[`RETRY_REQUEUE_SECS`], backing
/// off as [`requeue_after`] does (one streak per CR, both counted).
pub fn retry_after_error<R: Reason>(
    recorder: &Recorder,
    object: ObjectReference,
    reason: R,
    action: &'static str,
    error: &dyn std::fmt::Display,
) -> Action {
    let after = jittered(failed(object.uid.as_deref()));
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
    recovered(obj.meta().uid.as_deref());
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

    fn cr(uid: &str) -> k8s_openapi::api::core::v1::ConfigMap {
        k8s_openapi::api::core::v1::ConfigMap {
            metadata: kube::api::ObjectMeta {
                uid: Some(uid.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn requeue_follows_the_outcome_and_a_due_time() {
        let obj = cr("requeue-due");
        let secs = |a: Action| format!("{a:?}");
        assert!(secs(requeue_after(&obj, false, Some(5))).contains("5s"));
        // A due time already past: the floor, not a hot loop.
        assert!(secs(requeue_after(&obj, true, Some(0))).contains("1s"));
        // A due time later than the cadence doesn't delay it.
        let steady = requeue_after(&obj, true, Some(10_000));
        assert!(!format!("{steady:?}").contains("10000"));
    }

    #[test]
    fn failures_back_off_until_a_steady_reconcile() {
        assert_eq!(
            (0..6).map(backoff_secs).collect::<Vec<_>>(),
            [30, 60, 120, 240, 300, 300]
        );
        assert_eq!(backoff_secs(u32::MAX), STEADY_REQUEUE_SECS);
        let uid = Some("backoff-streak");
        let start = std::time::Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        // Retries 30 s then 60 s apart: each one counts.
        assert_eq!(
            [
                failed_at(uid, at(0)),
                failed_at(uid, at(30)),
                failed_at(uid, at(90))
            ],
            [30, 60, 120]
        );
        // Events before the 120 s retry is due: the same failure, not counted.
        assert_eq!(
            [failed_at(uid, at(91)), failed_at(uid, at(150))],
            [120, 120]
        );
        assert_eq!(failed_at(uid, at(210)), 240);
        recovered(uid);
        assert_eq!(failed_at(uid, at(211)), 30);
        recovered(uid);
        // No uid (an object not from the API): no streak to keep.
        assert_eq!([failed(None), failed(None)], [30, 30]);
    }

    #[test]
    fn a_successful_reconcile_ends_the_streak() {
        let obj = cr("reconciled-streak");
        let uid = Some("reconciled-streak");
        let start = std::time::Instant::now();
        assert_eq!(failed_at(uid, start), 30);
        assert_eq!(failed_at(uid, start + Duration::from_secs(30)), 60);
        reconciled(&obj);
        assert_eq!(failed_at(uid, start + Duration::from_secs(31)), 30);
        recovered(uid);
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
    #[expect(unsafe_code, reason = "remove_var is unsafe since edition 2024")]
    fn concurrency_defaults_to_four() {
        // SAFETY: the variable is only read by `concurrency`, here.
        unsafe { std::env::remove_var("WEEBO_RECONCILE_CONCURRENCY") };
        assert_eq!(concurrency(), DEFAULT_CONCURRENCY);
    }
}
