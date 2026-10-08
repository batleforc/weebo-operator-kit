//! Leader election over a `coordination.k8s.io` `Lease`: only one replica
//! may reconcile — two would both attempt-create the same remote object.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use kube::Client;
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// 3:1 ttl:renew — survives a couple of missed renewals (GC pause, apiserver
/// hiccup) without losing leadership spuriously ([`hold`]).
pub const LEASE_TTL: Duration = Duration::from_secs(15);
pub const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

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

    tracing::info!("waiting for the leader lease");
    acquire(&lease, LEASE_RENEW_INTERVAL).await;
    tracing::info!("acquired the leader lease");

    let renewal = tokio::spawn({
        let lease = lease.clone();
        async move {
            let why = hold(&lease, LEASE_RENEW_INTERVAL, LEASE_TTL).await;
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
    LeaseLock::new(
        client,
        namespace,
        LeaseLockParams {
            holder_id,
            lease_name: name.to_string(),
            lease_ttl: ttl,
        },
    )
}

/// Blocks until this replica holds the lease.
pub async fn acquire(lease: &LeaseLock, retry: Duration) {
    loop {
        match lease.try_acquire_or_renew().await {
            Ok(LeaseLockResult::Acquired(_)) => return,
            Ok(LeaseLockResult::NotAcquired(_)) => {}
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
            match lease.try_acquire_or_renew().await {
                Ok(LeaseLockResult::Acquired(_)) => Ok(true),
                Ok(LeaseLockResult::NotAcquired(_)) => Ok(false),
                Err(err) => Err(err.to_string()),
            }
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
