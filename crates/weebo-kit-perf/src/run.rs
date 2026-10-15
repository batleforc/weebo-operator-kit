//! A perf run: the operator started against envtest, then a sequence of
//! phases — idle windows and loads — each measured on the process, on the
//! operator's reconcile metrics and on the apiserver's request counter.
//!
//! The shape that makes the numbers comparable, after a soak test's:
//!
//! ```text
//! start ─▶ warm-up load ─▶ idle "baseline" ─▶ loads… ─▶ idle "final" ─▶ stop
//! ```
//!
//! Leaks are judged on the two **idle** windows, the baseline taken after a
//! warm-up: measuring at startup would count every lazily-filled cache as
//! growth, and a window under load includes in-flight work the other one
//! doesn't.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kube::api::Api;
use kube::{Client, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;
use weebo_kit_testkit::envtest::EnvTestCluster;

use crate::operator::{OperatorMetrics, OperatorProcess};
use crate::process::{Sampler, Usage};
use crate::prom::{self, Scrape};
use crate::report::{PhaseKind, PhaseReport, Report};
use crate::workload::Workload;

/// The idle window leaks are measured from.
pub const BASELINE: &str = "baseline";
/// The idle window compared with [`BASELINE`].
pub const FINAL: &str = "final";

/// How often the process is sampled.
pub const SAMPLE_EVERY: Duration = Duration::from_millis(250);

/// Time given to the operator to release its lease and exit on SIGTERM.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PerfRun {
    label: String,
    client: Client,
    metrics: OperatorMetrics,
    operator: OperatorProcess,
    sampler: Sampler,
    startup: Duration,
    phases: Vec<PhaseReport>,
    harness_calls: Arc<AtomicU64>,
    background: Background,
    _dir: TempDir,
}

/// What the apiserver serves with no operator running — envtest's own
/// controllers (its identity lease, the `kubernetes` endpoints…): measured
/// before the operator starts, and taken out of every phase.
#[derive(Debug, Clone, Default)]
struct Background {
    /// Requests per second, in all.
    total: f64,
    /// Requests per second, by `verb resource`.
    by_key: std::collections::BTreeMap<String, f64>,
}

/// How long the background is measured (`WEEBO_PERF_BACKGROUND_SECS`).
const BACKGROUND_SECS: u64 = 15;

impl Background {
    async fn measure(client: &Client) -> Self {
        let secs = std::env::var("WEEBO_PERF_BACKGROUND_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(BACKGROUND_SECS);
        let scrape = || async {
            prom::scrape_apiserver(client)
                .await
                .expect("scraping the apiserver's metrics")
        };
        let before = scrape().await;
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let after = scrape().await;
        let per_sec = |v: f64| v / secs as f64;
        let mut by_key: std::collections::BTreeMap<String, f64> =
            prom::top_deltas(&before, &after, API_REQUESTS, API_KEY, usize::MAX)
                .into_iter()
                .collect();
        // The first scrape is the harness'.
        if let Some(own) = by_key.get_mut("GET ") {
            *own -= 1.0;
        }
        let total = by_key.values().sum::<f64>();
        by_key.values_mut().for_each(|v| *v = per_sec(*v));
        Self {
            total: per_sec(total),
            by_key,
        }
    }
}

const API_REQUESTS: &str = "apiserver_request_total";
const API_KEY: &[&str] = &["verb", "resource"];

/// Snapshot taken at the start of a phase.
struct Mark {
    at: Duration,
    operator: Scrape,
    apiserver: Scrape,
    harness_calls: u64,
}

impl PerfRun {
    /// Starts `command` (the operator binary) against `cluster` and waits
    /// for it to hold the leader lease. `label` names the report.
    pub async fn start(
        label: &str,
        cluster: &EnvTestCluster,
        command: tokio::process::Command,
        metrics: OperatorMetrics,
    ) -> Self {
        let client = cluster.client();
        let background = Background::measure(&client).await;
        let dir = TempDir::new(label);
        let kubeconfig = dir.0.join("kubeconfig.yaml");
        cluster.write_kubeconfig(&kubeconfig);
        let mut operator = OperatorProcess::spawn(command, &kubeconfig, &dir.0);
        let startup = operator.wait_ready(&metrics, Duration::from_secs(60)).await;
        let sampler = Sampler::start(operator.pid(), SAMPLE_EVERY);
        Self {
            label: label.to_string(),
            client,
            background,
            metrics,
            operator,
            sampler,
            startup,
            phases: Vec::new(),
            harness_calls: Arc::new(AtomicU64::new(0)),
            _dir: dir,
        }
    }

    /// Bulk operations on the kind of `api`, counted as the harness'.
    pub fn workload<K>(&self, api: Api<K>) -> Workload<K>
    where
        K: Resource<DynamicType = ()> + Clone + std::fmt::Debug + DeserializeOwned + Serialize,
    {
        Workload::new(api, self.harness_calls.clone())
    }

    /// An admin client on the cluster (not counted: prefer [`Self::workload`]).
    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Nothing happens for `duration`: what the operator costs at rest.
    /// Name it [`BASELINE`] or [`FINAL`] for the leak checks.
    pub async fn idle(&mut self, name: &str, duration: Duration) -> &PhaseReport {
        let mark = self.mark().await;
        tokio::time::sleep(duration).await;
        self.record(name, PhaseKind::Idle, mark, None).await
    }

    /// Runs `load` — which returns the time the operator took to converge
    /// on `objects` objects — and measures the operator meanwhile.
    pub async fn load(
        &mut self,
        name: &str,
        objects: usize,
        load: impl Future<Output = Duration>,
    ) -> &PhaseReport {
        let mark = self.mark().await;
        let converged = load.await;
        self.record(name, PhaseKind::Load, mark, Some((objects, converged)))
            .await
    }

    /// SIGTERM, and the report.
    pub async fn finish(self) -> Report {
        let shutdown = self.operator.stop(SHUTDOWN_TIMEOUT).await;
        Report {
            label: self.label,
            background_api_requests_per_min: self.background.total * 60.0,
            startup_secs: self.startup.as_secs_f64(),
            shutdown_secs: shutdown.as_secs_f64(),
            phases: self.phases,
            measured_at: Some(k8s_openapi::jiff::Timestamp::now().to_string()),
            kit_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }
    }

    async fn mark(&self) -> Mark {
        // The apiserver scrape itself is a request the end of the phase
        // will see: counted as the harness'.
        self.harness_calls.fetch_add(1, Ordering::Relaxed);
        Mark {
            at: self.sampler.now(),
            operator: self.operator.scrape().await,
            apiserver: self.apiserver().await,
            harness_calls: self.harness_calls.load(Ordering::Relaxed),
        }
    }

    async fn apiserver(&self) -> Scrape {
        prom::scrape_apiserver(&self.client)
            .await
            .expect("scraping the apiserver's metrics")
    }

    async fn record(
        &mut self,
        name: &str,
        kind: PhaseKind,
        mark: Mark,
        converged: Option<(usize, Duration)>,
    ) -> &PhaseReport {
        let end = self.sampler.now();
        let operator = self.operator.scrape().await;
        let apiserver = self.apiserver().await;
        let harness = self.harness_calls.load(Ordering::Relaxed) - mark.harness_calls + 1;
        let wall = (end - mark.at).as_secs_f64();

        let delta = |matching: &[(&str, &str)]| -> f64 {
            self.metrics
                .reconcile_total()
                .iter()
                .map(|n| operator.sum(n, matching) - mark.operator.sum(n, matching))
                .sum()
        };
        let reconciles = delta(&[]);
        let errors = delta(&[("result", "error")]);
        let p95 = self.metrics.reconcile_duration().iter().find_map(|n| {
            prom::quantile_between(
                &mark.operator.buckets(n, &[]),
                &operator.buckets(n, &[]),
                0.95,
            )
        });
        let api_total = apiserver.sum(API_REQUESTS, &[]) - mark.apiserver.sum(API_REQUESTS, &[]);
        let operator_requests =
            (api_total - harness as f64 - self.background.total * wall).max(0.0);
        let mut api_top: Vec<(String, f64)> = prom::top_deltas(
            &mark.apiserver,
            &apiserver,
            API_REQUESTS,
            API_KEY,
            usize::MAX,
        )
        .into_iter()
        .map(|(k, v)| {
            let bg = self.background.by_key.get(&k).copied().unwrap_or(0.0) * wall;
            (k, (v - bg).round())
        })
        .filter(|(_, v)| *v > 0.0)
        .collect();
        api_top.sort_by(|a, b| b.1.total_cmp(&a.1));
        api_top.truncate(8);

        self.phases.push(PhaseReport {
            name: name.to_string(),
            kind,
            wall_secs: wall,
            usage: Usage::of(&self.sampler.window(mark.at, end)),
            reconciles,
            reconcile_errors: errors,
            reconcile_p95_secs: p95,
            api_requests: operator_requests,
            api_requests_per_min: if wall > 0.0 {
                operator_requests / wall * 60.0
            } else {
                0.0
            },
            api_top,
            objects: converged.map(|c| c.0),
            convergence_secs: converged.map(|c| c.1.as_secs_f64()),
        });
        let phase = self.phases.last().expect("just pushed");
        tracing::info!(
            phase = %phase.name,
            rss_mib = phase.usage.rss_median_mib,
            cpu_millicores = phase.usage.cpu_mean_millicores,
            reconciles = phase.reconciles,
            api_requests = phase.api_requests,
            "phase measured"
        );
        phase
    }
}

/// A scratch directory (kubeconfig, operator log), removed with the run
/// unless `WEEBO_PERF_KEEP=1`.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("weebo-perf-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("perf scratch directory");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::env::var("WEEBO_PERF_KEEP").as_deref() != Ok("1") {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
