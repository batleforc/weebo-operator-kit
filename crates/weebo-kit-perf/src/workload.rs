//! The load an operator is put under: CRs created, waited on, changed and
//! deleted in bulk, through an admin client. Every apiserver call made here
//! is counted, so the run can tell the operator's own API traffic from the
//! harness'.

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Polling interval of the waits.
const POLL: Duration = Duration::from_millis(500);

/// Bulk operations on one kind.
pub struct Workload<K> {
    api: Api<K>,
    calls: Arc<AtomicU64>,
}

impl<K> Workload<K>
where
    K: Resource<DynamicType = ()> + Clone + Debug + DeserializeOwned + Serialize,
{
    pub(crate) fn new(api: Api<K>, calls: Arc<AtomicU64>) -> Self {
        Self { api, calls }
    }

    fn count(&self, n: u64) {
        self.calls.fetch_add(n, Ordering::Relaxed);
    }

    pub async fn create(&self, objects: &[K]) {
        for obj in objects {
            self.count(1);
            self.api
                .create(&PostParams::default(), obj)
                .await
                .unwrap_or_else(|e| panic!("creating {}: {e}", obj.name_any()));
        }
    }

    /// Waits until every object of `names` satisfies `done`: the time it
    /// took (the operator's convergence time, from the call).
    pub async fn wait_all(
        &self,
        names: &[String],
        timeout: Duration,
        done: impl Fn(&K) -> bool,
    ) -> Duration {
        let started = Instant::now();
        loop {
            self.count(1);
            let list = self
                .api
                .list(&ListParams::default())
                .await
                .expect("listing the workload");
            let pending: Vec<&String> = names
                .iter()
                .filter(|n| !list.items.iter().any(|o| o.name_any() == **n && done(o)))
                .collect();
            if pending.is_empty() {
                return started.elapsed();
            }
            assert!(
                started.elapsed() < timeout,
                "{} of {} objects not done within {timeout:?} (e.g. {:?})",
                pending.len(),
                names.len(),
                pending.iter().take(3).collect::<Vec<_>>()
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// A merge patch on every object (`patch(name)` builds it).
    pub async fn patch_all(&self, names: &[String], patch: impl Fn(&str) -> serde_json::Value) {
        for name in names {
            self.count(1);
            self.api
                .patch(name, &PatchParams::default(), &Patch::Merge(patch(name)))
                .await
                .unwrap_or_else(|e| panic!("patching {name}: {e}"));
        }
    }

    pub async fn delete_all(&self, names: &[String]) {
        for name in names {
            self.count(1);
            match self.api.delete(name, &DeleteParams::default()).await {
                Ok(_) => {}
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(e) => panic!("deleting {name}: {e}"),
            }
        }
    }

    /// Waits until none of `names` exists any more (finalizers done): the
    /// time it took.
    pub async fn wait_gone(&self, names: &[String], timeout: Duration) -> Duration {
        let started = Instant::now();
        loop {
            self.count(1);
            let list = self
                .api
                .list(&ListParams::default())
                .await
                .expect("listing the workload");
            let left = list
                .items
                .iter()
                .filter(|o| names.contains(&o.name_any()))
                .count();
            if left == 0 {
                return started.elapsed();
            }
            assert!(
                started.elapsed() < timeout,
                "{left} of {} objects still present after {timeout:?}",
                names.len()
            );
            tokio::time::sleep(POLL).await;
        }
    }
}

/// `prefix-0000`, `prefix-0001`…
pub fn names(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}-{i:04}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_sort_like_their_index() {
        let n = super::names("g", 12);
        assert_eq!(n[0], "g-0000");
        assert_eq!(n[11], "g-0011");
        let mut sorted = n.clone();
        sorted.sort();
        assert_eq!(sorted, n);
    }
}
