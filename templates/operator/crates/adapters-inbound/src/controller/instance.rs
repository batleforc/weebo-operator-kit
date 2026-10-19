//! `{{product}}Instance` (cluster-scoped): reconciling it is a
//! connectivity and token check (`reconcile_instance`). No finalizer: there is nothing remote to clean
//! up. Deleting an instance still in use leaves its objects unable to
//! clean up — they then need `force-orphan`. A real operator may hold the
//! deletion with a finalizer while CRs are bound to it.

use std::sync::Arc;
use std::time::Instant;

use api::Instance;
use application::ports::BoundInstance;
use application::use_cases::Outcome;
use application::use_cases::reconcile_instance::reconcile_instance;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::{Client, Resource, ResourceExt};

use super::{Ctx, Error, error_policy};

const KIND: &str = "{{product}}Instance";

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(ctx.scope.api::<Instance>(client), &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive(KIND, health, results).await;
}

async fn reconcile(obj: Arc<Instance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if obj.meta().deletion_timestamp.is_some() {
        return Ok(Action::await_change());
    }
    let api: Api<Instance> = Api::all(ctx.client.clone());
    let started = Instant::now();
    let bound = BoundInstance::cluster(obj.name_any());
    let outcome = match ctx.gateways.gateway_for(&bound).await {
        Ok(gateway) => reconcile_instance(gateway.as_ref()).await,
        Err(e) => Outcome::failed(e),
    };
    super::record_reconcile(KIND, started, &outcome);
    super::publish(&api, &ctx.recorder, obj.as_ref(), &outcome).await?;
    Ok(super::requeue_after(&outcome))
}
