//! `{{product}}LocalInstance` (namespaced): the same connectivity and token
//! check as a `{{product}}Instance`, with the token of its own namespace.
//! No finalizer either.

use std::sync::Arc;
use std::time::Instant;

use api::LocalInstance;
use application::ports::BoundInstance;
use application::use_cases::Outcome;
use application::use_cases::reconcile_instance::reconcile_instance;
use kube::api::Api;
use kube::runtime::controller::Action;
use kube::{Client, Resource, ResourceExt};

use super::{Ctx, Error, error_policy};

const KIND: &str = "{{product}}LocalInstance";

pub async fn run(client: Client, ctx: Arc<Ctx>) {
    let health = ctx.health.clone();
    let results = ctx
        .watches
        .controller(ctx.scope.api::<LocalInstance>(client), &ctx.scope)
        .graceful_shutdown_on(ctx.shutdown.wait())
        .run(reconcile, error_policy, ctx);
    super::drive(KIND, health, results).await;
}

async fn reconcile(obj: Arc<LocalInstance>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    if obj.meta().deletion_timestamp.is_some() {
        return Ok(Action::await_change());
    }
    let namespace = obj.namespace().unwrap_or_default();
    let api: Api<LocalInstance> = Api::namespaced(ctx.client.clone(), &namespace);
    let started = Instant::now();
    let bound = BoundInstance::local(namespace, obj.name_any());
    let outcome = match ctx.gateways.gateway_for(&bound).await {
        Ok(gateway) => reconcile_instance(gateway.as_ref()).await,
        Err(e) => Outcome::failed(e),
    };
    super::record_reconcile(KIND, started, &outcome);
    super::publish(&api, &ctx.recorder, obj.as_ref(), &outcome).await?;
    Ok(super::requeue_after(obj.as_ref(), &outcome))
}
