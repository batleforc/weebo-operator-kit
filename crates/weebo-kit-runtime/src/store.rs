//! In-memory stores of the kinds an operator reads on every reconcile
//! (instances, policies, referenced CRs): one watch per kind instead of a
//! request per reconcile.

use std::fmt::Debug;
use std::sync::Arc;

use futures::StreamExt;
use kube::Resource;
use kube::api::Api;
use kube::runtime::reflector::{self, Store};
use kube::runtime::{WatchStreamExt, watcher};
use serde::de::DeserializeOwned;

use crate::{Health, WatchScope};

/// A reflector on `api` (from `scope.api::<K>()`), through `scope`'s label
/// selector, run in the background: the store serves every read from
/// memory, watch errors back off exponentially instead of spinning and are
/// reported to `health` under `name` (a watch erroring in a loop fails
/// `/livez`; any event received clears it). Wait for it before trusting an
/// empty store with `weebo_kit_adapters::wait_synced`, not a bare
/// `store.wait_until_ready()`: kube's readiness wakes one waiter only.
pub fn spawn_reflector<K>(
    api: Api<K>,
    scope: &WatchScope,
    health: Arc<Health>,
    name: &'static str,
) -> Store<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Send + Sync + 'static,
{
    let (store, writer) = reflector::store::<K>();
    let stream = reflector::reflector(
        writer,
        watcher(api, scope.watcher_config()).default_backoff(),
    );
    tokio::spawn(async move {
        stream
            .for_each(|event| {
                match event {
                    Ok(_) => health.ok(name),
                    Err(err) => {
                        tracing::warn!(error = %err, "{name} watch error");
                        health.failed(name, err.to_string());
                    }
                }
                futures::future::ready(())
            })
            .await;
    });
    store
}
