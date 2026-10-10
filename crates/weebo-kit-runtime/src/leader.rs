//! Leader election over a `coordination.k8s.io` `Lease`: only one replica
//! may reconcile — two would both attempt-create the same remote object.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::Client;
use kube::api::{Api, PostParams};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// 3:1 ttl:renew — survives a couple of missed renewals (GC pause, apiserver
/// hiccup) without losing leadership spuriously ([`hold`]).
pub const LEASE_TTL: Duration = Duration::from_secs(15);
pub const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

static LEADER: AtomicBool = AtomicBool::new(false);

/// Whether this process currently holds the leader lease (for a gauge).
pub fn is_leader() -> bool {
    LEADER.load(Ordering::Relaxed)
}

/// The held lease, renewed in the background.
pub struct Leadership {
    lease: Arc<LeaseLock>,
    renewal: JoinHandle<()>,
}

impl Leadership {
    /// Hands the lease over right away instead of making the next leader
    /// wait out the TTL. Call once every controller returned.
    pub async fn release(self) {
        self.renewal.abort();
        LEADER.store(false, Ordering::Relaxed);
        match self.lease.step_down().await {
            Ok(()) => tracing::info!("released the leader lease"),
            Err(err) => tracing::warn!(error = %err, "releasing the leader lease failed"),
        }
    }
}

/// Blocks until this replica holds the lease, then keeps renewing it in
/// the background and **exits the process** the moment it may be lost.
/// The lease is `$LEASE_NAME` (default `default_lease_name`) in
/// `$POD_NAMESPACE` (default `default`).
pub async fn acquire_leadership(
    client: &Client,
    holder_id: String,
    default_lease_name: &str,
) -> Leadership {
    let namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let lease_name = std::env::var("LEASE_NAME").unwrap_or_else(|_| default_lease_name.to_string());
    let lease = Arc::new(self::lease(
        client.clone(),
        &namespace,
        &lease_name,
        holder_id,
        LEASE_TTL,
    ));

    tracing::info!(%namespace, lease = %lease_name, "waiting for the leader lease");
    acquire(&lease, LEASE_RENEW_INTERVAL).await;
    LEADER.store(true, Ordering::Relaxed);
    tracing::info!("acquired the leader lease");

    let renewal = tokio::spawn({
        let lease = lease.clone();
        async move {
            let why = hold(&lease, LEASE_RENEW_INTERVAL, LEASE_TTL).await;
            LEADER.store(false, Ordering::Relaxed);
            tracing::error!("{why}, exiting");
            std::process::exit(1);
        }
    });
    Leadership { lease, renewal }
}

pub fn lease(
    client: Client,
    namespace: &str,
    name: &str,
    holder_id: String,
    ttl: Duration,
) -> LeaseLock {
    LeaseLock {
        api: Api::namespaced(client, namespace),
        name: name.to_string(),
        holder: holder_id,
        ttl,
        observed: Mutex::new(None),
    }
}

/// A `coordination.k8s.io/v1` `Lease` used as a lock, client-go style:
///
/// - every write carries the `resourceVersion` it was computed from, so of
///   two replicas taking over an expired lease at once only one succeeds
///   (the other gets a conflict) — no split brain;
/// - expiry is judged on this replica's **monotonic clock**: the lease is
///   free once its record hasn't changed for `leaseDurationSeconds` since
///   this replica first saw that record — skew between the nodes' wall
///   clocks never matters. A record stale by more than [`STALE_FACTOR`]
///   TTLs by the wall clock (a leader long gone) is taken without waiting.
pub struct LeaseLock {
    api: Api<Lease>,
    name: String,
    holder: String,
    ttl: Duration,
    observed: Mutex<Option<(String, Instant)>>,
}

/// See [`LeaseLock`].
pub const STALE_FACTOR: u32 = 3;

impl LeaseLock {
    fn spec(&self, transitions: i32, acquired: Option<MicroTime>) -> LeaseSpec {
        let now = MicroTime(Timestamp::now());
        LeaseSpec {
            holder_identity: Some(self.holder.clone()),
            lease_duration_seconds: Some(self.ttl.as_secs().max(1) as i32),
            acquire_time: Some(acquired.unwrap_or_else(|| now.clone())),
            renew_time: Some(now),
            lease_transitions: Some(transitions),
            ..Default::default()
        }
    }

    /// When this replica first saw the lease's current record.
    fn observed_since(&self, lease: &Lease) -> Instant {
        let version = lease.metadata.resource_version.clone().unwrap_or_default();
        let mut observed = self.observed.lock().unwrap_or_else(|p| p.into_inner());
        match &*observed {
            Some((v, at)) if *v == version => *at,
            _ => {
                let now = Instant::now();
                *observed = Some((version, now));
                now
            }
        }
    }

    /// Replaces the lease with `spec`, guarded by the `resourceVersion` it
    /// was read at. `Ok(false)` on a conflict: someone else wrote first.
    async fn write(&self, mut lease: Lease, spec: LeaseSpec) -> Result<bool, kube::Error> {
        lease.spec = Some(spec);
        match self
            .api
            .replace(&self.name, &PostParams::default(), &lease)
            .await
        {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(e)) if e.code == 409 => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `Ok(true)` when this replica holds the lease after the call.
    pub async fn try_acquire_or_renew(&self) -> Result<bool, kube::Error> {
        let Some(lease) = self.api.get_opt(&self.name).await? else {
            let lease = Lease {
                metadata: ObjectMeta {
                    name: Some(self.name.clone()),
                    ..Default::default()
                },
                spec: Some(self.spec(0, None)),
            };
            return match self.api.create(&PostParams::default(), &lease).await {
                Ok(_) => Ok(true),
                Err(kube::Error::Api(e)) if e.code == 409 => Ok(false),
                Err(e) => Err(e),
            };
        };
        let observed_since = self.observed_since(&lease);
        let spec = lease.spec.clone().unwrap_or_default();
        let transitions = spec.lease_transitions.unwrap_or(0);
        let holder = spec.holder_identity.as_deref().filter(|h| !h.is_empty());

        if holder == Some(self.holder.as_str()) {
            let renewed = self.spec(transitions, spec.acquire_time.clone());
            return self.write(lease, renewed).await;
        }
        if holder.is_some() {
            let ttl = spec
                .lease_duration_seconds
                .map_or(self.ttl, |s| Duration::from_secs(s.max(0) as u64));
            let expired_locally = observed_since.elapsed() >= ttl;
            let stale = spec.renew_time.as_ref().is_some_and(|r| {
                let age = Timestamp::now().duration_since(r.0);
                age.as_secs() > (ttl * STALE_FACTOR).as_secs() as i64
            });
            if !expired_locally && !stale {
                return Ok(false);
            }
        }
        let acquired = self.spec(transitions + 1, None);
        self.write(lease, acquired).await
    }

    /// Clears the holder if it is still this replica, so the next leader
    /// doesn't wait out the TTL.
    pub async fn step_down(&self) -> Result<(), kube::Error> {
        let Some(lease) = self.api.get_opt(&self.name).await? else {
            return Ok(());
        };
        let mut spec = lease.spec.clone().unwrap_or_default();
        if spec.holder_identity.as_deref() != Some(self.holder.as_str()) {
            return Ok(());
        }
        // Empty, not absent: what client-go (and the previous
        // implementation) leave behind, and what tools expect.
        spec.holder_identity = Some(String::new());
        spec.renew_time = None;
        self.write(lease, spec).await.map(|_| ())
    }
}

/// Blocks until this replica holds the lease.
pub async fn acquire(lease: &LeaseLock, retry: Duration) {
    loop {
        match lease.try_acquire_or_renew().await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::error!(error = %err, "leader lease acquisition failed, retrying"),
        }
        tokio::time::sleep(retry).await;
    }
}

/// Renews the lease every `renew` and returns as soon as it may be lost —
/// taken over, or not renewed for so long that another replica could hold
/// it by now: the caller must stop reconciling at once. A failed or hanging
/// renewal alone is retried until then, so an apiserver hiccup shorter
/// than the lease's `ttl` doesn't depose the leader.
pub async fn hold(lease: &LeaseLock, renew: Duration, ttl: Duration) -> String {
    hold_with(
        || async {
            lease
                .try_acquire_or_renew()
                .await
                .map_err(|err| err.to_string())
        },
        renew,
        ttl,
    )
    .await
}

/// `renew_once`: `Ok(true)` renewed, `Ok(false)` someone else holds it.
async fn hold_with<F, Fut>(mut renew_once: F, renew: Duration, ttl: Duration) -> String
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    // Another replica may take over `ttl` after the last successful renewal
    // (counted from when it was sent — its `renewTime` can't be earlier).
    // Half a renew interval of slack absorbs clock skew between replicas.
    let window = ttl.saturating_sub(renew / 2);
    // A failed renewal is retried sooner than the next regular one.
    let retry = renew / 5;
    let mut renewed_at = Instant::now();
    let mut next = renewed_at + renew;
    loop {
        let deadline = renewed_at + window;
        tokio::time::sleep_until(next.min(deadline)).await;
        if Instant::now() >= deadline {
            return "could not renew the leader lease before it could expire".to_string();
        }
        let sent = Instant::now();
        match tokio::time::timeout_at(deadline, renew_once()).await {
            Ok(Ok(true)) => {
                renewed_at = sent;
                next = sent + renew;
            }
            Ok(Ok(false)) => return "lost the leader lease".to_string(),
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "leader lease renewal failed, retrying");
                next = Instant::now() + retry;
            }
            Err(_) => {
                return "the leader lease renewal did not complete before the lease could expire"
                    .to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::pending;

    use super::*;

    const TTL: Duration = Duration::from_secs(15);
    const RENEW: Duration = Duration::from_secs(5);

    #[tokio::test(start_paused = true)]
    async fn transient_renewal_failures_are_tolerated() {
        let calls = Cell::new(0);
        let held = tokio::time::timeout(
            Duration::from_secs(120),
            hold_with(
                || {
                    calls.set(calls.get() + 1);
                    // Two failures in a row, then fine again.
                    let ok = !matches!(calls.get(), 2 | 3);
                    async move {
                        if ok {
                            Ok(true)
                        } else {
                            Err("apiserver hiccup".into())
                        }
                    }
                },
                RENEW,
                TTL,
            ),
        )
        .await;
        assert!(held.is_err(), "gave up the lease: {:?}", held.unwrap());
        assert!(calls.get() > 5);
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_failures_end_before_the_lease_can_expire() {
        let start = Instant::now();
        let why = hold_with(|| async { Err("apiserver down".into()) }, RENEW, TTL).await;
        assert!(why.contains("expire"), "{why}");
        assert!(start.elapsed() < TTL, "{:?}", start.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_renewal_ends_before_the_lease_can_expire() {
        let start = Instant::now();
        let why = hold_with(pending::<Result<bool, String>>, RENEW, TTL).await;
        assert!(why.contains("expire"), "{why}");
        assert!(start.elapsed() < TTL, "{:?}", start.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn a_taken_over_lease_ends_at_once() {
        let start = Instant::now();
        let why = hold_with(|| async { Ok(false) }, RENEW, TTL).await;
        assert!(why.contains("lost"), "{why}");
        assert!(start.elapsed() <= RENEW);
    }
}
