//! One `kube::runtime::Controller` per CRD, on the kit's skeleton
//! (triggers, concurrency, requeue, finalizer, Events, metrics). The
//! reconcile bodies delegate to `application::use_cases`.

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use api::Status;
use api::status::Condition;
use application::ports::{BoundInstance, GatewayFactory, InstanceDirectory};
use application::use_cases::Outcome;
use domain::Failure;
use domain::error::ReasonCode;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::events::Recorder;
use kube::{Client, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;
use weebo_kit_runtime::events;
use weebo_kit_runtime::metrics::ReconcileMetrics;
use weebo_kit_runtime::shared::SharedWatches;
use weebo_kit_runtime::status::StatusUpdate;
use weebo_kit_runtime::{Health, WatchScope};

/// Resolves once the process is asked to stop: every controller then stops
/// taking new work, lets its in-flight reconciles finish, and its `run`
/// returns.
pub use weebo_kit_runtime::Shutdown;
/// The kit's controller skeleton: a watch filtered to what reconciling
/// acts on (never the CR's own status patches), bounded concurrency
/// (`WEEBO_RECONCILE_CONCURRENCY`), the result stream drained into the
/// liveness.
pub use weebo_kit_runtime::controller::{controller, drive};

pub mod instance;
pub mod local_instance;
pub mod object;

/// Field manager of the status patches, meter and Event reporter name.
pub const FIELD_MANAGER: &str = "{{project-name}}-operator";
pub const FINALIZER: &str = "{{product | downcase}}.weebo.io/finalizer";
/// Prefix of every metric: `<prefix>_reconcile_total`, `<prefix>_leader`…
pub const METRIC_PREFIX: &str = "{{crate_name}}";

/// Shared across every controller. Concrete adapters are built and
/// injected once, in `operator::main` (tests inject fakes).
pub struct Ctx {
    pub client: Client,
    pub gateways: Arc<dyn GatewayFactory>,
    /// Which instance a CR lives on (the kit's `Instances`, on the watch
    /// scope).
    pub instances: Arc<dyn InstanceDirectory>,
    /// One recorder for the whole process: the kit folds repeated Events
    /// into a series only within a recorder.
    pub recorder: Recorder,
    pub shutdown: Shutdown,
    /// Liveness: controllers report their watch errors here.
    pub health: Arc<Health>,
    /// What the controllers watch (`WEEBO_WATCH_NAMESPACE`,
    /// `WEEBO_WATCH_LABEL_SELECTOR`): every watch goes through it.
    pub scope: WatchScope,
    /// The binary's shared watches ([`spawn_watches`]): one per kind for
    /// every controller and every store read (`SharedWatches::default()`:
    /// each controller watches on its own). A controller watching a
    /// referenced kind goes through them too
    /// (`weebo_kit_runtime::shared::WatchesIn::watches_in`).
    pub watches: SharedWatches,
}

/// One watch per kind, on `scope`, for every controller and the instance
/// stores (no `{{product}}Instance` without cluster-scoped kinds). Spawned
/// before the lease is campaigned for: a standby keeps them warm.
pub fn spawn_watches(client: &Client, scope: &WatchScope, health: &Arc<Health>) -> SharedWatches {
    let mut watches = SharedWatches::default();
    if scope.cluster_kinds() {
        watches.spawn(
            scope.api::<api::Instance>(client.clone()),
            scope,
            health,
            "{{product}}Instance watch",
        );
    }
    watches.spawn(
        scope.api::<api::LocalInstance>(client.clone()),
        scope,
        health,
        "{{product}}LocalInstance watch",
    );
    watches.spawn(
        scope.api::<api::Object>(client.clone()),
        scope,
        health,
        "{{product}}Object watch",
    );
    watches
}

/// What stopped a reconcile before (or instead of) its status.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    /// Anything the finalizer helper reports: the reconcile's own error
    /// (`ApplyFailed`/`CleanupFailed`) or adding/removing the finalizer.
    #[error("{0}")]
    Finalizer(Box<kube::runtime::finalizer::Error<Error>>),
    /// A cleanup that couldn't delete the remote object: the finalizer
    /// stays until it can (or the CR is force-orphaned).
    #[error("{0}")]
    Cleanup(Failure),
}

impl Error {
    /// The reason code of the Event announcing this error, and whether it
    /// happened reconciling or cleaning up (the Event's `action`).
    pub fn reason(&self) -> (ReasonCode, &'static str) {
        use kube::runtime::finalizer::Error as Finalizer;
        match self {
            Error::Kube(_) => (ReasonCode::KubernetesApiError, events::RECONCILE),
            Error::Cleanup(failure) => (failure.reason, events::CLEANUP),
            Error::Finalizer(err) => match err.as_ref() {
                Finalizer::ApplyFailed(inner) => inner.reason(),
                Finalizer::CleanupFailed(inner) => (inner.reason().0, events::CLEANUP),
                _ => (ReasonCode::KubernetesApiError, events::RECONCILE),
            },
        }
    }
}

/// Every reconcile that errors out is announced by a `Warning` Event on
/// its CR, then retried in ~30s, backing off while it keeps failing (the
/// kit's).
pub fn error_policy<K>(obj: Arc<K>, err: &Error, ctx: Arc<Ctx>) -> Action
where
    K: Resource<DynamicType = ()>,
{
    let (reason, action) = err.reason();
    weebo_kit_runtime::controller::retry_after_error(
        &ctx.recorder,
        obj.object_ref(&()),
        reason,
        action,
        err,
    )
}

/// Steady-state drift check after a success, a quick retry after a
/// failure, backing off while `obj` keeps failing (jittered — the kit's).
pub fn requeue_after<K: Resource>(obj: &K, outcome: &Outcome) -> Action {
    weebo_kit_runtime::controller::requeue_after(obj, outcome.ready.is_ok(), None)
}

/// The kit's `ReconcileMetrics`. A no-op until the telemetry installs a
/// real `MeterProvider`.
pub fn metrics() -> &'static ReconcileMetrics {
    static METRICS: OnceLock<ReconcileMetrics> = OnceLock::new();
    METRICS.get_or_init(|| ReconcileMetrics::new(FIELD_MANAGER, METRIC_PREFIX))
}

/// `<prefix>_reconcile_total` / `_duration_seconds`, and
/// `<prefix>_remote_writes_total` when the reconcile wrote to {{product}}.
pub fn record_reconcile(kind: &'static str, started: Instant, outcome: &Outcome) {
    metrics().record(kind, outcome.reason(), started.elapsed());
    if let Some(write) = outcome.write {
        metrics().record_write(kind, write);
    }
}

/// The `status` of every CRD here is a [`Status`].
pub trait HasStatus {
    fn status(&self) -> Option<&Status>;

    fn remote_id(&self) -> Option<&str> {
        self.status()?.remote_id.as_deref()
    }

    /// The instance recorded at the last resolution.
    fn bound_instance(&self) -> Option<&BoundInstance> {
        self.status()?.instance.as_ref()
    }
}

impl HasStatus for api::Instance {
    fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
}

impl HasStatus for api::LocalInstance {
    fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
}

impl HasStatus for api::Object {
    fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
}

/// Publishes `outcome` on `obj` (read fresh from the API server: its
/// conditions are the previous ones). The kit's: one merge patch
/// (conditions, `observedGeneration` on success, `remoteId`), retried while
/// the apiserver is unavailable, then a `Warning` on every failed reconcile
/// and a `Normal` for every condition change. One more Event when the
/// remote object was just created, or put back after a drift (naming the
/// fields).
pub async fn publish<K>(
    api: &Api<K>,
    recorder: &Recorder,
    obj: &K,
    outcome: &Outcome,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = ()> + HasStatus + Clone + Debug + DeserializeOwned + Serialize,
{
    let previous: &[Condition] = obj
        .status()
        .map(|s| s.conditions.as_slice())
        .unwrap_or_default();
    let mut fields = serde_json::Map::new();
    if let Some(id) = &outcome.remote_id {
        fields.insert("remoteId".into(), id.clone().into());
    }
    if let Some(instance) = &outcome.instance {
        let instance = serde_json::to_value(instance).expect("a BoundInstance serializes");
        fields.insert("instance".into(), instance);
    }
    weebo_kit_runtime::status::publish(
        api,
        recorder,
        FIELD_MANAGER,
        obj,
        previous,
        StatusUpdate {
            ready: &outcome.ready,
            advisories: &[],
            fields,
        },
    )
    .await?;
    if let Some(id) = &outcome.remote_id
        && obj.remote_id().is_none()
    {
        events::announce_change(
            recorder,
            &obj.object_ref(&()),
            ReasonCode::Created,
            events::RECONCILE,
            &format!("created in {{product}} as {id}"),
        )
        .await;
    }
    // A spec change being applied is not a drift: only a remote object
    // that moved on its own is announced.
    let spec_changed = obj.meta().generation != obj.status().and_then(|s| s.observed_generation);
    if !spec_changed {
        events::announce_drift(
            recorder,
            &obj.object_ref(&()),
            ReasonCode::DriftCorrected,
            &outcome.drift,
        )
        .await;
    }
    Ok(())
}

/// A finalizer's cleanup — unless the CR carries
/// `{{product | downcase}}.weebo.io/force-orphan: "true"`, the way out for
/// a CR whose cleanup can't succeed any more (its instance gone) — then a
/// `Released` Event (the kit's).
pub async fn finalize<K>(
    obj: &K,
    ctx: &Ctx,
    cleanup: impl Future<Output = Result<(), Error>>,
) -> Result<Action, Error>
where
    K: Resource<DynamicType = ()>,
{
    weebo_kit_runtime::controller::finalize(
        obj,
        &ctx.recorder,
        &api::ANNOTATIONS,
        ReasonCode::Released,
        cleanup,
    )
    .await
}

#[cfg(test)]
mod tests {
    use kube::runtime::finalizer::Error as Finalizer;

    use super::*;

    #[test]
    fn errors_map_to_a_reason_and_an_action() {
        let unreachable = || Error::Cleanup(Failure::new(ReasonCode::InstanceUnreachable, "x"));
        assert_eq!(
            unreachable().reason(),
            (ReasonCode::InstanceUnreachable, events::CLEANUP)
        );
        assert_eq!(
            Error::Finalizer(Box::new(Finalizer::CleanupFailed(unreachable()))).reason(),
            (ReasonCode::InstanceUnreachable, events::CLEANUP)
        );
        assert_eq!(
            Error::Finalizer(Box::new(Finalizer::UnnamedObject)).reason(),
            (ReasonCode::KubernetesApiError, events::RECONCILE)
        );
    }
}
