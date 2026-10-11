//! What a run measured, checked against a [`Budget`], written as Markdown
//! (for a PR comment or a job summary) and JSON (the baseline the next run
//! compares with).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::process::Usage;
use crate::run::{BASELINE, FINAL};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseKind {
    Idle,
    Load,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseReport {
    pub name: String,
    pub kind: PhaseKind,
    pub wall_secs: f64,
    pub usage: Usage,
    /// Reconcile runs during the phase (every kind).
    pub reconciles: f64,
    pub reconcile_errors: f64,
    /// 95th percentile of a reconcile's duration, over the phase.
    pub reconcile_p95_secs: Option<f64>,
    /// apiserver requests made by the operator (the harness' own removed).
    pub api_requests: f64,
    pub api_requests_per_min: f64,
    /// The largest increases of `apiserver_request_total`, by `verb
    /// resource`, background taken out — the harness' own requests still in
    /// (its LISTs of the workload kind, POST/PATCH/DELETE): what to read
    /// when the API traffic is over budget.
    #[serde(default)]
    pub api_top: Vec<(String, f64)>,
    /// Load phases: how many objects, and how long until all converged.
    pub objects: Option<usize>,
    pub convergence_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub label: String,
    /// Spawn → leader lease held (controllers running).
    pub startup_secs: f64,
    /// SIGTERM → exit.
    pub shutdown_secs: f64,
    /// The apiserver's own traffic with no operator (envtest's controllers),
    /// taken out of every phase's `api_requests`.
    #[serde(default)]
    pub background_api_requests_per_min: f64,
    pub phases: Vec<PhaseReport>,
    /// When the run finished (UTC, RFC 3339) — absent from reports
    /// written before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_at: Option<String>,
    /// The `weebo-kit-perf` version that measured it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_version: Option<String>,
}

/// What an operator may cost to sit on a small cluster next to others.
/// Every limit is "fail above"; `None` leaves it unchecked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub startup_secs: Option<f64>,
    pub shutdown_secs: Option<f64>,
    /// Median RSS of every idle phase.
    pub idle_rss_mib: Option<f64>,
    /// Peak RSS over the whole run.
    pub peak_rss_mib: Option<f64>,
    /// Mean CPU of every idle phase.
    pub idle_cpu_millicores: Option<f64>,
    /// CPU time per object of a load phase, milliseconds. (A burst may
    /// use every core for a second; what it costs is the CPU it burns.)
    pub cpu_ms_per_object: Option<f64>,
    /// The operator's apiserver traffic at rest (watches, lease renewals…).
    pub idle_api_requests_per_min: Option<f64>,
    /// apiserver requests per object of a load phase.
    pub api_requests_per_object: Option<f64>,
    /// Convergence time per 100 objects of a load phase.
    pub convergence_secs_per_100: Option<f64>,
    /// Reconcile p95 in any phase.
    pub reconcile_p95_secs: Option<f64>,
    /// Leaks, [`FINAL`] vs [`BASELINE`] idle window.
    pub rss_growth_pct: Option<f64>,
    pub fd_growth: Option<u64>,
    pub thread_growth: Option<u64>,
}

impl Budget {
    /// Defaults sized for a small cluster (a few operators on a 2-4 GiB
    /// node): an operator that idles under 64 MiB and 10 millicores, peaks
    /// under 192 MiB, keeps its API traffic at rest to a few lease
    /// renewals and gives back what a load took.
    pub fn small_cluster() -> Self {
        Self {
            startup_secs: Some(15.0),
            shutdown_secs: Some(15.0),
            idle_rss_mib: Some(64.0),
            peak_rss_mib: Some(192.0),
            idle_cpu_millicores: Some(10.0),
            cpu_ms_per_object: Some(50.0),
            idle_api_requests_per_min: Some(60.0),
            api_requests_per_object: Some(20.0),
            convergence_secs_per_100: Some(60.0),
            reconcile_p95_secs: Some(2.0),
            rss_growth_pct: Some(10.0),
            fd_growth: Some(16),
            thread_growth: Some(4),
        }
    }
}

/// One limit of the budget, against what was measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub measured: f64,
    pub limit: f64,
    pub ok: bool,
}

impl Report {
    fn phase(&self, name: &str) -> Option<&PhaseReport> {
        self.phases.iter().find(|p| p.name == name)
    }

    fn of(&self, kind: PhaseKind) -> impl Iterator<Item = &PhaseReport> {
        self.phases.iter().filter(move |p| p.kind == kind)
    }

    /// Every limit `budget` sets, measured.
    pub fn check(&self, budget: &Budget) -> Vec<Check> {
        let mut out = Vec::new();
        let mut add = |name: String, measured: f64, limit: Option<f64>| {
            if let Some(limit) = limit {
                out.push(Check {
                    name,
                    measured,
                    limit,
                    ok: measured <= limit,
                });
            }
        };
        add("startup (s)".into(), self.startup_secs, budget.startup_secs);
        add(
            "shutdown (s)".into(),
            self.shutdown_secs,
            budget.shutdown_secs,
        );
        for p in self.of(PhaseKind::Idle) {
            add(
                format!("{}: RSS (MiB)", p.name),
                p.usage.rss_median_mib,
                budget.idle_rss_mib,
            );
            add(
                format!("{}: CPU (m)", p.name),
                p.usage.cpu_mean_millicores,
                budget.idle_cpu_millicores,
            );
            add(
                format!("{}: API req/min", p.name),
                p.api_requests_per_min,
                budget.idle_api_requests_per_min,
            );
        }
        for p in self.of(PhaseKind::Load) {
            if let Some(n) = p.objects.filter(|n| *n > 0) {
                add(
                    format!("{}: CPU (ms/object)", p.name),
                    p.usage.cpu_seconds * 1000.0 / n as f64,
                    budget.cpu_ms_per_object,
                );
                add(
                    format!("{}: API req/object", p.name),
                    p.api_requests / n as f64,
                    budget.api_requests_per_object,
                );
                if let Some(c) = p.convergence_secs {
                    add(
                        format!("{}: convergence (s/100)", p.name),
                        c / n as f64 * 100.0,
                        budget.convergence_secs_per_100,
                    );
                }
            }
        }
        for p in &self.phases {
            if let Some(p95) = p.reconcile_p95_secs {
                add(
                    format!("{}: reconcile p95 (s)", p.name),
                    p95,
                    budget.reconcile_p95_secs,
                );
            }
        }
        let peak = self
            .phases
            .iter()
            .map(|p| p.usage.rss_peak_mib)
            .fold(0.0, f64::max);
        add("peak RSS (MiB)".into(), peak, budget.peak_rss_mib);
        if let (Some(base), Some(fin)) = (self.phase(BASELINE), self.phase(FINAL)) {
            let (b, f) = (base.usage, fin.usage);
            if b.rss_median_mib > 0.0 {
                add(
                    "idle RSS growth (%)".into(),
                    (f.rss_median_mib - b.rss_median_mib) / b.rss_median_mib * 100.0,
                    budget.rss_growth_pct,
                );
            }
            add(
                "idle fd growth".into(),
                f.fds_max as f64 - b.fds_max as f64,
                budget.fd_growth.map(|v| v as f64),
            );
            add(
                "idle thread growth".into(),
                f.threads_max as f64 - b.threads_max as f64,
                budget.thread_growth.map(|v| v as f64),
            );
        }
        out
    }

    /// The report as Markdown: the checks, then one row per phase, with
    /// the change from `baseline` (a previous run's JSON) when given.
    pub fn to_markdown(&self, budget: &Budget, baseline: Option<&Report>) -> String {
        let checks = self.check(budget);
        let failed = checks.iter().filter(|c| !c.ok).count();
        let mut md = String::new();
        let _ = writeln!(md, "## Operator resources — `{}`\n", self.label);
        if let (Some(at), Some(kit)) = (&self.measured_at, &self.kit_version) {
            let _ = writeln!(md, "Measured {at} with weebo-kit-perf {kit}.\n");
        }
        let _ = writeln!(
            md,
            "{} — startup {:.1} s, shutdown {:.1} s.\n",
            if failed == 0 {
                "**Within budget**".to_string()
            } else {
                format!("**{failed} limit(s) exceeded**")
            },
            self.startup_secs,
            self.shutdown_secs
        );
        let _ = writeln!(
            md,
            "| check | measured | limit | |\n| --- | ---: | ---: | --- |"
        );
        for c in &checks {
            let _ = writeln!(
                md,
                "| {} | {:.2} | {:.2} | {} |",
                c.name,
                c.measured,
                c.limit,
                if c.ok { "ok" } else { "**over**" }
            );
        }
        let _ = writeln!(
            md,
            "\n| phase | s | objects | RSS median / peak (MiB) | RSS slope (MiB/min) | CPU mean / peak (m) | fds | threads | reconciles (errors) | p95 (s) | API req (/min) | converged (s) |\n\
             | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
        );
        for p in &self.phases {
            let was = baseline.and_then(|b| b.phase(&p.name));
            let u = p.usage;
            let _ = writeln!(
                md,
                "| {} | {:.0} | {} | {:.1}{} / {:.1} | {:.2} | {:.1}{} / {:.0} | {} | {} | {:.0} ({:.0}) | {} | {:.0} ({:.1}) | {} |",
                p.name,
                p.wall_secs,
                p.objects.map_or("".into(), |n| n.to_string()),
                u.rss_median_mib,
                delta(was.map(|w| w.usage.rss_median_mib), u.rss_median_mib),
                u.rss_peak_mib,
                u.rss_slope_mib_per_min,
                u.cpu_mean_millicores,
                delta(
                    was.map(|w| w.usage.cpu_mean_millicores),
                    u.cpu_mean_millicores
                ),
                u.cpu_peak_millicores,
                u.fds_max,
                u.threads_max,
                p.reconciles,
                p.reconcile_errors,
                p.reconcile_p95_secs
                    .map_or("".into(), |v| format!("{v:.3}")),
                p.api_requests,
                p.api_requests_per_min,
                p.convergence_secs.map_or("".into(), |v| format!("{v:.1}")),
            );
        }
        if baseline.is_some() {
            let _ = writeln!(md, "\n_(±%: change from the baseline run, report-only.)_");
        }
        let _ = writeln!(
            md,
            "\n<details><summary>apiserver requests by verb and resource (harness included, background of {:.0}/min removed)</summary>\n",
            self.background_api_requests_per_min
        );
        for p in &self.phases {
            let top: Vec<String> = p
                .api_top
                .iter()
                .map(|(k, v)| format!("{k} {v:.0}"))
                .collect();
            let _ = writeln!(md, "- **{}**: {}", p.name, top.join(", "));
        }
        let _ = writeln!(md, "\n</details>");
        md
    }

    /// Writes `<label>.json` and `<label>.md` into `$WEEBO_PERF_OUT`, else
    /// the workspace's `target/perf`, comparing with
    /// `$WEEBO_PERF_BASELINE` (a previous `<label>.json`) when set.
    /// Returns the Markdown.
    pub fn write(&self, budget: &Budget) -> String {
        let dir = std::env::var_os("WEEBO_PERF_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(default_out_dir);
        let baseline =
            std::env::var_os("WEEBO_PERF_BASELINE").and_then(|p| Self::load(Path::new(&p)));
        let md = self.to_markdown(budget, baseline.as_ref());
        std::fs::create_dir_all(&dir).expect("perf output directory");
        std::fs::write(
            dir.join(format!("{}.json", self.label)),
            serde_json::to_string_pretty(self).expect("report serializes"),
        )
        .expect("perf JSON written");
        std::fs::write(dir.join(format!("{}.md", self.label)), &md).expect("perf report written");
        md
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// Writes the report, prints it, and panics when a limit is exceeded —
    /// unless `WEEBO_PERF_REPORT_ONLY=1` (a first run on new hardware,
    /// before its noise is known).
    pub fn assert_within(&self, budget: &Budget) {
        let md = self.write(budget);
        println!("{md}");
        let over: Vec<_> = self.check(budget).into_iter().filter(|c| !c.ok).collect();
        if over.is_empty() || std::env::var("WEEBO_PERF_REPORT_ONLY").as_deref() == Ok("1") {
            return;
        }
        let list: Vec<String> = over
            .iter()
            .map(|c| format!("{}: {:.2} > {:.2}", c.name, c.measured, c.limit))
            .collect();
        panic!("operator over budget:\n  {}", list.join("\n  "));
    }
}

/// `<workspace>/target/perf`: a test runs in its package's directory, the
/// workspace is the nearest ancestor holding a `Cargo.lock`.
fn default_out_dir() -> PathBuf {
    let here = std::env::current_dir().unwrap_or_default();
    let root = here
        .ancestors()
        .find(|d| d.join("Cargo.lock").exists())
        .unwrap_or(&here);
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("perf")
}

fn delta(was: Option<f64>, now: f64) -> String {
    match was {
        Some(w) if w > 0.0 => format!(" ({:+.0}%)", (now - w) / w * 100.0),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(name: &str, kind: PhaseKind, rss: f64, fds: u64) -> PhaseReport {
        PhaseReport {
            name: name.into(),
            kind,
            wall_secs: 60.0,
            usage: Usage {
                samples: 10,
                rss_peak_mib: rss + 5.0,
                rss_median_mib: rss,
                cpu_seconds: 1.0,
                cpu_mean_millicores: 3.0,
                cpu_peak_millicores: 20.0,
                rss_slope_mib_per_min: 0.0,
                fds_max: fds,
                threads_max: 8,
            },
            reconciles: 100.0,
            reconcile_errors: 0.0,
            reconcile_p95_secs: Some(0.05),
            api_requests: 400.0,
            api_requests_per_min: 12.0,
            api_top: vec![("GET leases".into(), 12.0)],
            objects: (kind == PhaseKind::Load).then_some(100),
            convergence_secs: (kind == PhaseKind::Load).then_some(10.0),
        }
    }

    fn report(final_rss: f64) -> Report {
        Report {
            label: "test".into(),
            startup_secs: 2.0,
            shutdown_secs: 1.0,
            background_api_requests_per_min: 20.0,
            measured_at: Some("2026-10-11T10:00:00Z".into()),
            kit_version: Some("0.0.0".into()),
            phases: vec![
                phase(BASELINE, PhaseKind::Idle, 30.0, 20),
                phase("burst", PhaseKind::Load, 40.0, 25),
                phase(FINAL, PhaseKind::Idle, final_rss, 21),
            ],
        }
    }

    #[test]
    fn a_frugal_operator_is_within_the_small_cluster_budget() {
        let checks = report(31.0).check(&Budget::small_cluster());
        assert!(checks.iter().all(|c| c.ok), "{checks:#?}");
        let per_object = checks
            .iter()
            .find(|c| c.name == "burst: API req/object")
            .unwrap();
        assert_eq!(per_object.measured, 4.0);
    }

    #[test]
    fn idle_memory_that_isnt_given_back_is_a_leak() {
        let checks = report(36.0).check(&Budget::small_cluster());
        let growth = checks
            .iter()
            .find(|c| c.name == "idle RSS growth (%)")
            .unwrap();
        assert!(!growth.ok);
        assert!((growth.measured - 20.0).abs() < 1e-9);
    }

    /// The docs package renders this budget on every operator's site (its
    /// "resource budget" page) from a copy: it must not drift.
    #[test]
    fn the_docs_package_publishes_this_budget() {
        let published: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/operator-docs/src/reference/small-cluster-budget.json"
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(Budget::small_cluster()).unwrap(),
            published,
            "update packages/operator-docs/src/reference/small-cluster-budget.json"
        );
    }

    #[test]
    fn markdown_names_the_overruns_and_the_baseline_delta() {
        let base = report(31.0);
        let now = report(36.0);
        let md = now.to_markdown(&Budget::small_cluster(), Some(&base));
        assert!(md.contains("1 limit(s) exceeded"), "{md}");
        assert!(
            md.contains("Measured 2026-10-11T10:00:00Z with weebo-kit-perf 0.0.0"),
            "{md}"
        );
        assert!(md.contains("**over**"));
        assert!(md.contains("(+16%)"), "{md}");
        let json = serde_json::to_string(&now).unwrap();
        assert_eq!(serde_json::from_str::<Report>(&json).unwrap(), now);
    }
}
