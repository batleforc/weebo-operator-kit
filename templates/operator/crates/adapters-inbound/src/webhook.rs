//! The HTTPS router the kit's `webhook_server` serves on every replica
//! (`WEBHOOK_PORT`, default 8443): the probes — `/readyz`, `/livez`,
//! `/healthz` — and `/scope`, the kit's `ScopeGuard` (the chart's
//! `scope.<group>` webhook, rendered with a `watch.labelSelector`).
//!
//! An admission webhook goes here too: `POST /validate` through
//! `weebo_kit_runtime::admission::router` (an `Admission` impl naming the
//! operator's reason code for a denial), typically enforcing a
//! cluster-scoped `{{product}}NamespacePolicy` CRD — which namespaces may
//! create which kinds on which instance — with the kit's
//! `weebo_kit_domain::allow_list`; the reconcilers then re-check the same
//! rules, a policy may change after admission.

use std::sync::Arc;

use axum::Router;
use domain::error::ReasonCode;
use weebo_kit_runtime::admission::{ScopeGuard, scope_router};
use weebo_kit_runtime::{Health, WatchScope};

use crate::controller::FINALIZER;

pub fn router(health: Arc<Health>, scope: WatchScope) -> Router {
    let guard = ScopeGuard {
        scope,
        finalizer: FINALIZER,
        annotations: api::ANNOTATIONS,
        reason: ReasonCode::LeavesWatchScope,
    };
    weebo_kit_runtime::health::routes(health).merge(scope_router(Arc::new(guard)))
}
