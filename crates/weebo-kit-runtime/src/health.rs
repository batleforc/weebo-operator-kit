//! Readiness and liveness of an operator process, served next to the
//! admission webhook ([`routes`]).
//!
//! - **ready**: the process serves admission requests (set once the webhook
//!   listens, cleared as soon as shutdown starts so endpoints drain first);
//! - **live**: no component kept failing for longer than the threshold (a
//!   watch erroring in a loop) — restarting the pod is then the remedy.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;

/// Default time a component may keep failing before the process is
/// reported dead.
pub const DEFAULT_LIVENESS_THRESHOLD: Duration = Duration::from_secs(300);

/// A failure streak with no new failure for this long is over: components
/// that only report failures (a watch erroring, then silently fine again)
/// heal on their own.
pub const FAILURE_STREAK_GAP: Duration = Duration::from_secs(120);

struct Streak {
    since: Instant,
    last: Instant,
    why: String,
}

pub struct Health {
    ready: AtomicBool,
    failing: Mutex<HashMap<String, Streak>>,
    threshold: Duration,
    gap: Duration,
}

impl Default for Health {
    fn default() -> Self {
        Self::new(DEFAULT_LIVENESS_THRESHOLD)
    }
}

impl Health {
    pub fn new(threshold: Duration) -> Self {
        Self::with_gap(threshold, FAILURE_STREAK_GAP)
    }

    pub fn with_gap(threshold: Duration, gap: Duration) -> Self {
        Self {
            ready: AtomicBool::new(false),
            failing: Mutex::new(HashMap::new()),
            threshold,
            gap,
        }
    }

    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Relaxed);
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// `component` works again (no-op when it wasn't failing).
    pub fn ok(&self, component: &str) {
        self.failing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(component);
    }

    /// `component` failed; the first of a streak of failures starts the
    /// clock, later ones only refresh it. A streak ends with [`Self::ok`]
    /// or after the gap ([`FAILURE_STREAK_GAP`]) without a failure.
    pub fn failed(&self, component: &str, why: impl Into<String>) {
        let (why, now, gap) = (why.into(), Instant::now(), self.gap);
        let mut failing = self.failing.lock().unwrap_or_else(|p| p.into_inner());
        match failing.get_mut(component) {
            Some(s) => {
                if now.duration_since(s.last) >= gap {
                    s.since = now;
                }
                s.last = now;
                s.why = why;
            }
            None => {
                failing.insert(
                    component.to_string(),
                    Streak {
                        since: now,
                        last: now,
                        why,
                    },
                );
            }
        }
    }

    /// `Err` names the components failing past the threshold.
    pub fn live(&self) -> Result<(), String> {
        let failing = self.failing.lock().unwrap_or_else(|p| p.into_inner());
        let dead: Vec<String> = failing
            .iter()
            .filter(|(_, s)| s.since.elapsed() >= self.threshold && s.last.elapsed() < self.gap)
            .map(|(c, s)| {
                format!(
                    "{c}: failing for {}s: {}",
                    s.since.elapsed().as_secs(),
                    s.why
                )
            })
            .collect();
        if dead.is_empty() {
            Ok(())
        } else {
            Err(dead.join("\n"))
        }
    }
}

/// `GET /readyz`, `GET /livez`, and `GET /healthz` (alias of `/readyz`).
pub fn routes<S>(health: Arc<Health>) -> Router<S> {
    Router::new()
        .route("/readyz", get(readyz))
        .route("/healthz", get(readyz))
        .route("/livez", get(livez))
        .with_state(health)
}

async fn readyz(State(health): State<Arc<Health>>) -> (StatusCode, &'static str) {
    if health.ready() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}

async fn livez(State(health): State<Arc<Health>>) -> (StatusCode, String) {
    match health.live() {
        Ok(()) => (StatusCode::OK, "ok".to_string()),
        Err(why) => (StatusCode::SERVICE_UNAVAILABLE, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_kill_only_past_the_threshold() {
        let health = Health::new(Duration::from_millis(20));
        assert!(health.live().is_ok());
        health.failed("watch", "boom");
        assert!(health.live().is_ok());
        std::thread::sleep(Duration::from_millis(30));
        health.failed("watch", "boom again");
        let why = health.live().unwrap_err();
        assert!(why.contains("watch") && why.contains("boom again"), "{why}");
        health.ok("watch");
        assert!(health.live().is_ok());
    }

    #[test]
    fn a_streak_without_new_failures_heals() {
        let health = Health::with_gap(Duration::from_millis(10), Duration::from_millis(40));
        health.failed("watch", "boom");
        std::thread::sleep(Duration::from_millis(15));
        health.failed("watch", "boom");
        assert!(health.live().is_err());
        std::thread::sleep(Duration::from_millis(50));
        assert!(health.live().is_ok());
    }

    #[test]
    fn readiness_toggles() {
        let health = Health::default();
        assert!(!health.ready());
        health.set_ready(true);
        assert!(health.ready());
    }
}
