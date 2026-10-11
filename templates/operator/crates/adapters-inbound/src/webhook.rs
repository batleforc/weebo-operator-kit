//! The HTTPS router the kit's `webhook_server` serves on every replica
//! (`WEBHOOK_PORT`, default 8443): for now only the probes — `/readyz`,
//! `/livez`, `/healthz`.
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
use weebo_kit_runtime::Health;

pub fn router(health: Arc<Health>) -> Router {
    weebo_kit_runtime::health::routes(health)
}
