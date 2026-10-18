//! `{{product}}Object`: the remote object is compared with the spec on
//! every reconcile, written only when it differs, and deleted by the
//! finalizer when this CR created it.

use std::sync::Arc;
use std::time::Instant;

use api::Object;
use application::ports::BoundInstance;
use application::use_cases::reconcile_object::{delete_object, reconcile_object};
use application::use_cases::{Outcome, instance_failure};
use domain::Failure;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::runtime::finalizer::{Event as FinalizerEvent, finalizer};
use kube::{Client, Resource, ResourceExt};
use weebo_kit_runtime::events;

use super::{Ctx, Error, FINALIZER, HasStatus, error_policy};

const KIND: &str = "{{product}}Object";

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let health = ctx.health.clone();
    let results = super::controller(ctx.scope.api::<Object>(client), &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive(KIND, health, results).await;
}

async fn reconcile(obj: Arc<Object>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    let namespace = obj.namespace().unwrap_or_default();
    let api: Api<Object> = Api::namespaced(ctx.client.clone(), &namespace);
    finalizer(&api, FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(object) => apply(&api, &object, &ctx).await,
            FinalizerEvent::Cleanup(object) => {
                super::finalize(object.as_ref(), &ctx, cleanup(&api, &object, &ctx)).await
            }
        }
    })
    .await
    .map_err(|e| Error::Finalizer(Box::new(e)))
}

async fn apply(api: &Api<Object>, object: &Object, ctx: &Ctx) -> Result<Action, Error> {
    // Re-read from the API server rather than trusted from the watch
    // cache: the status patch of the previous reconcile may not have
    // reached it yet, and a stale `remoteId` would mis-announce a
    // creation.
    let current = api.get(&object.name_any()).await?;
    let started = Instant::now();
    // A refused resolution leaves `status.instance` as it was.
    let outcome = match resolve(&current, ctx).await {
        Err(e) => Outcome::failed(instance_failure(e)),
        Ok(bound) => match ctx.gateways.gateway_for(&bound).await {
            Ok(gateway) => reconcile_object(&current, gateway.as_ref()).await,
            Err(e) => Outcome::failed(e),
        }
        .on(bound),
    };
    super::record_reconcile(KIND, started, &outcome);
    super::publish(api, &ctx.recorder, &current, &outcome).await?;
    Ok(super::requeue_after(&outcome))
}

async fn cleanup(api: &Api<Object>, object: &Object, ctx: &Ctx) -> Result<(), Error> {
    // Fresh, like `apply`: a CR deleted right after its first reconcile
    // may still lack `remoteId` in the watch cache.
    let Some(object) = api.get_opt(&object.name_any()).await? else {
        return Ok(());
    };
    let object = &object;
    let Some(remote_id) = object.remote_id() else {
        // Never created anything: nothing to delete, even with the
        // instance gone.
        return Ok(());
    };
    // The instance it was bound to, not whatever its ref designates now.
    let bound = match object.bound_instance() {
        Some(bound) => bound.clone(),
        None => resolve(object, ctx)
            .await
            .map_err(|e| Error::Cleanup(instance_failure(e)))?,
    };
    let gateway = ctx
        .gateways
        .gateway_for(&bound)
        .await
        .map_err(|e| Error::Cleanup(Failure::from(e)))?;
    let deleted = delete_object(object, Some(remote_id), gateway.as_ref())
        .await
        .map_err(Error::Cleanup)?;
    if deleted {
        super::metrics().record_write(KIND, domain::RemoteWrite::Deleted);
        let name = object.remote_name();
        let note = format!("deleted {name:?} from {{product}}");
        events::announce_change(
            &ctx.recorder,
            &object.object_ref(&()),
            domain::error::ReasonCode::Deleted,
            events::CLEANUP,
            &note,
        )
        .await;
    }
    Ok(())
}

/// The instance `object` lives on: its `instanceRef` resolved (the kit's
/// rules), and kept to what its status recorded.
async fn resolve(
    object: &Object,
    ctx: &Ctx,
) -> Result<BoundInstance, application::ports::InstanceError> {
    ctx.instances
        .resolve(
            object.namespace().as_deref(),
            object.spec.instance_ref.as_ref(),
            object.bound_instance(),
        )
        .await
}
