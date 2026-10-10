//! What an operator costs to run, measured before it ships: so one more
//! operator can go on a small cluster without wondering how much of it it
//! will take.
//!
//! [`PerfRun`] starts the operator's **real binary** against envtest (the
//! way it runs in a pod, no webhook), samples its process (`/proc`: RSS,
//! CPU, descriptors, threads), reads its reconcile metrics and the
//! apiserver's request counter, and measures each phase of a run — idle
//! windows and loads of CRs driven through [`Workload`]. The [`Report`] is
//! checked against a [`Budget`] ([`Budget::small_cluster`] by default),
//! written as Markdown and JSON, and compared with a previous run's JSON.
//!
//! ```no_run
//! # async fn demo(cluster: weebo_kit_testkit::envtest::EnvTestCluster) {
//! use std::time::Duration;
//! use weebo_kit_perf::{Budget, OperatorMetrics, PerfRun, BASELINE, FINAL};
//!
//! // In the operator's own test: `env!("CARGO_BIN_EXE_<binary>")`.
//! let command = tokio::process::Command::new("target/release/my-operator");
//! let mut run = PerfRun::start("my-operator", &cluster, command,
//!     OperatorMetrics::new("weebo_my")).await;
//! run.idle("startup", Duration::from_secs(30)).await;
//! // … a warm-up load, then:
//! run.idle(BASELINE, Duration::from_secs(60)).await;
//! // … the loads: run.load("burst", n, async { /* create, wait */ }).await
//! run.idle(FINAL, Duration::from_secs(60)).await;
//! run.finish().await.assert_within(&Budget::small_cluster());
//! # }
//! ```
//!
//! Numbers only mean something on a **release** build of the operator,
//! and a perf test is not a unit test: mark it `#[ignore]` and run it on
//! purpose (`cargo test --release … -- --ignored`).

pub mod operator;
pub mod process;
pub mod prom;
pub mod report;
pub mod run;
pub mod workload;

pub use operator::{OperatorMetrics, OperatorProcess};
pub use report::{Budget, Check, PhaseKind, PhaseReport, Report};
pub use run::{BASELINE, FINAL, PerfRun};
pub use workload::{Workload, names};
