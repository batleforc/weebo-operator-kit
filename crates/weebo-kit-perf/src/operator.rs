//! The operator under test: its real (release) binary, against envtest,
//! exactly as it runs in a pod — not an in-process controller, whose
//! memory and threads couldn't be told apart from the test's.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::prom;

/// How the harness recognizes the operator's own metrics.
#[derive(Debug, Clone)]
pub struct OperatorMetrics {
    /// Metric prefix given to the kit's `ReconcileMetrics`
    /// (`weebo_authentik` → `weebo_authentik_reconcile_total`).
    pub prefix: String,
    /// The gauge at 1 on the replica holding the leader lease
    /// (`weebo_authentik_leader`): the operator is up once it reads 1.
    pub leader_gauge: String,
}

impl OperatorMetrics {
    /// `<prefix>_leader` as the leader gauge.
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            leader_gauge: format!("{prefix}_leader"),
        }
    }

    /// `<prefix>_reconcile_total` — and the `…_total_total` kits before
    /// 0.5.0 exported.
    pub fn reconcile_total(&self) -> [String; 2] {
        let name = format!("{}_reconcile_total", self.prefix);
        [format!("{name}_total"), name]
    }

    /// `<prefix>_reconcile_duration_seconds`, or the older
    /// `…_seconds_seconds`.
    pub fn reconcile_duration(&self) -> [String; 2] {
        let name = format!("{}_reconcile_duration_seconds", self.prefix);
        [format!("{name}_seconds"), name]
    }
}

pub struct OperatorProcess {
    child: tokio::process::Child,
    metrics_addr: SocketAddr,
    log: PathBuf,
    spawned: Instant,
}

impl OperatorProcess {
    /// Spawns `command` (the operator binary, with whatever environment the
    /// operator needs) against the cluster of `kubeconfig`. Adds
    /// `KUBECONFIG`, `WEEBO_METRICS_ADDR` (a free local port),
    /// `POD_NAMESPACE=default` and `HOSTNAME` when unset, and sends its
    /// output to `<dir>/operator.log`. No webhook certificate is given: the
    /// webhook stays off, its cost isn't the controllers'.
    pub fn spawn(mut command: tokio::process::Command, kubeconfig: &Path, dir: &Path) -> Self {
        let metrics_addr = free_local_addr();
        let log = dir.join("operator.log");
        let out = std::fs::File::create(&log).expect("operator log file");
        command
            .env("KUBECONFIG", kubeconfig)
            .env("WEEBO_METRICS_ADDR", metrics_addr.to_string())
            .env("WEBHOOK_TLS_CERT_FILE", dir.join("no-webhook-cert"))
            .stdin(Stdio::null())
            .stdout(out.try_clone().expect("log handle"))
            .stderr(out)
            .kill_on_drop(true);
        if std::env::var_os("POD_NAMESPACE").is_none() {
            command.env("POD_NAMESPACE", "default");
        }
        if std::env::var_os("HOSTNAME").is_none() {
            command.env("HOSTNAME", "weebo-perf");
        }
        let child = command.spawn().expect("the operator binary must spawn");
        Self {
            child,
            metrics_addr,
            log,
            spawned: Instant::now(),
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id().expect("the operator is running")
    }

    pub fn metrics_addr(&self) -> SocketAddr {
        self.metrics_addr
    }

    pub async fn scrape(&self) -> prom::Scrape {
        prom::scrape(self.metrics_addr)
            .await
            .unwrap_or_else(|e| panic!("scraping the operator's metrics: {e}\n{}", self.log_tail()))
    }

    /// Waits until the operator holds the leader lease (its controllers
    /// run): the startup time, from spawn.
    pub async fn wait_ready(&mut self, metrics: &OperatorMetrics, timeout: Duration) -> Duration {
        let deadline = self.spawned + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "the operator exited during startup ({status})\n{}",
                    self.log_tail()
                );
            }
            if let Ok(scrape) = prom::scrape(self.metrics_addr).await
                && scrape.value(&metrics.leader_gauge, &[]) == Some(1.0)
            {
                return self.spawned.elapsed();
            }
            assert!(
                Instant::now() < deadline,
                "the operator did not hold the leader lease within {timeout:?} \
                 (is {} exported?)\n{}",
                metrics.leader_gauge,
                self.log_tail()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// SIGTERM, then waits for the process to exit: the shutdown time
    /// (in-flight reconciles finished, lease released). Killed when it
    /// takes longer than `timeout` — reported as `timeout`.
    pub async fn stop(mut self, timeout: Duration) -> Duration {
        let started = Instant::now();
        let pid = self.pid();
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(_) => started.elapsed(),
            Err(_) => {
                tracing::warn!(?timeout, "the operator ignored SIGTERM, killing it");
                let _ = self.child.kill().await;
                timeout
            }
        }
    }

    /// The last lines of the operator's output, for failure messages.
    pub fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(40)..].join("\n");
        format!("--- {} (last 40 lines) ---\n{tail}", self.log.display())
    }
}

fn free_local_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free local port");
    listener.local_addr().expect("its address")
}
