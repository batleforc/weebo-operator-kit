//! What the operator process costs the node, read from `/proc/<pid>` (Linux
//! only): resident memory, CPU time, open descriptors, threads — the same
//! numbers the kubelet and the container runtime account it on.

use std::time::{Duration, Instant};

/// `/proc/<pid>/stat` CPU times are in `USER_HZ`, fixed at 100 for the
/// userspace ABI whatever the kernel's own tick rate.
const USER_HZ: f64 = 100.0;

/// One reading of the process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Since the sampler started.
    pub at: Duration,
    /// Resident set size, bytes (`VmRSS`).
    pub rss_bytes: u64,
    /// User + system CPU time consumed since the process started, seconds.
    pub cpu_seconds: f64,
    pub fds: u64,
    pub threads: u64,
}

/// Reads `/proc/<pid>` once. `None` once the process is gone.
pub fn read(pid: u32, started: Instant) -> Option<Reading> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.count() as u64;
    Some(Reading {
        at: started.elapsed(),
        rss_bytes: status_kib(&status, "VmRSS:")? * 1024,
        cpu_seconds: cpu_seconds(&stat)?,
        fds,
        threads: status_kib(&status, "Threads:")?,
    })
}

/// A numeric `/proc/<pid>/status` field (`VmRSS:   1234 kB` → 1234).
fn status_kib(status: &str, field: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(field))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `utime + stime` of `/proc/<pid>/stat`. The command name (field 2) is in
/// parentheses and may hold spaces: fields are counted after its last `)`.
fn cpu_seconds(stat: &str) -> Option<f64> {
    let after = &stat[stat.rfind(')')? + 1..];
    let mut fields = after.split_whitespace();
    // After `)`: state is field 3, so utime (14) and stime (15) are the
    // 12th and 13th.
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some((utime + stime) as f64 / USER_HZ)
}

/// Readings of one process at a fixed interval, in the background.
pub struct Sampler {
    readings: std::sync::Arc<std::sync::Mutex<Vec<Reading>>>,
    task: tokio::task::JoinHandle<()>,
    started: Instant,
}

impl Sampler {
    pub fn start(pid: u32, every: Duration) -> Self {
        let readings = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let started = Instant::now();
        let task = tokio::spawn({
            let readings = readings.clone();
            async move {
                let mut tick = tokio::time::interval(every);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    match read(pid, started) {
                        Some(r) => readings.lock().unwrap().push(r),
                        None => return,
                    }
                }
            }
        });
        Self {
            readings,
            task,
            started,
        }
    }

    /// Time since the sampler started — the clock [`Reading::at`] is on.
    pub fn now(&self) -> Duration {
        self.started.elapsed()
    }

    /// The readings taken in `[from, to]`.
    pub fn window(&self, from: Duration, to: Duration) -> Vec<Reading> {
        self.readings
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.at >= from && r.at <= to)
            .copied()
            .collect()
    }

    pub fn all(&self) -> Vec<Reading> {
        self.readings.lock().unwrap().clone()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A window of readings, summarized.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub samples: usize,
    pub rss_peak_mib: f64,
    pub rss_median_mib: f64,
    /// CPU time consumed over the window, seconds.
    pub cpu_seconds: f64,
    /// Average CPU over the window, in millicores (1000 = one core busy).
    pub cpu_mean_millicores: f64,
    /// Busiest interval between two consecutive readings, in millicores.
    pub cpu_peak_millicores: f64,
    /// Least-squares slope of RSS over the window, MiB per minute: what
    /// keeps growing while it runs.
    pub rss_slope_mib_per_min: f64,
    pub fds_max: u64,
    pub threads_max: u64,
}

const MIB: f64 = 1024.0 * 1024.0;

impl Usage {
    pub fn of(readings: &[Reading]) -> Self {
        if readings.is_empty() {
            return Self::default();
        }
        let mut rss: Vec<f64> = readings.iter().map(|r| r.rss_bytes as f64 / MIB).collect();
        let slope = slope(readings);
        rss.sort_by(f64::total_cmp);
        let (first, last) = (readings[0], readings[readings.len() - 1]);
        let span = (last.at - first.at).as_secs_f64();
        let cpu_seconds = last.cpu_seconds - first.cpu_seconds;
        let cpu_mean = if span > 0.0 {
            cpu_seconds / span * 1000.0
        } else {
            0.0
        };
        let cpu_peak = readings
            .windows(2)
            .filter_map(|w| {
                let dt = (w[1].at - w[0].at).as_secs_f64();
                (dt > 0.0).then(|| (w[1].cpu_seconds - w[0].cpu_seconds) / dt * 1000.0)
            })
            .fold(0.0, f64::max);
        Self {
            samples: readings.len(),
            rss_peak_mib: rss[rss.len() - 1],
            rss_median_mib: rss[rss.len() / 2],
            cpu_seconds,
            cpu_mean_millicores: cpu_mean,
            cpu_peak_millicores: cpu_peak,
            rss_slope_mib_per_min: slope,
            fds_max: readings.iter().map(|r| r.fds).max().unwrap_or(0),
            threads_max: readings.iter().map(|r| r.threads).max().unwrap_or(0),
        }
    }
}

fn slope(readings: &[Reading]) -> f64 {
    let n = readings.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let xs: Vec<f64> = readings.iter().map(|r| r.at.as_secs_f64() / 60.0).collect();
    let ys: Vec<f64> = readings.iter().map(|r| r.rss_bytes as f64 / MIB).collect();
    let (mx, my) = (xs.iter().sum::<f64>() / n, ys.iter().sum::<f64>() / n);
    let num: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let den: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    if den == 0.0 { 0.0 } else { num / den }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(secs: u64, rss_mib: u64, cpu: f64) -> Reading {
        Reading {
            at: Duration::from_secs(secs),
            rss_bytes: rss_mib * 1024 * 1024,
            cpu_seconds: cpu,
            fds: 10 + secs,
            threads: 4,
        }
    }

    #[test]
    fn proc_fields_are_parsed() {
        let status = "Name:\top\nVmRSS:\t   2048 kB\nThreads:\t7\n";
        assert_eq!(status_kib(status, "VmRSS:"), Some(2048));
        assert_eq!(status_kib(status, "Threads:"), Some(7));
        // A command name with spaces and parentheses.
        let stat = "42 (my (op) x) S 1 2 3 4 5 6 7 8 9 10 150 50 0 0 20 0 9 0 100";
        assert_eq!(cpu_seconds(stat), Some(2.0));
    }

    #[test]
    fn this_process_can_be_read() {
        let me = read(std::process::id(), Instant::now()).expect("/proc/self");
        assert!(me.rss_bytes > 0 && me.threads >= 1 && me.fds >= 3);
    }

    #[test]
    fn usage_summarizes_a_window() {
        // 60 s at 100 → 130 MiB, 1.5 s of CPU over 60 s = 25 millicores.
        let window = [r(0, 100, 0.0), r(30, 110, 0.3), r(60, 130, 1.5)];
        let u = Usage::of(&window);
        assert_eq!(u.samples, 3);
        assert_eq!(u.rss_peak_mib, 130.0);
        assert_eq!(u.rss_median_mib, 110.0);
        assert!((u.cpu_seconds - 1.5).abs() < 1e-9);
        assert!((u.cpu_mean_millicores - 25.0).abs() < 1e-9);
        assert!((u.cpu_peak_millicores - 40.0).abs() < 1e-9);
        assert!((u.rss_slope_mib_per_min - 30.0).abs() < 1e-9);
        assert_eq!(u.fds_max, 70);
        assert_eq!(Usage::of(&[]), Usage::default());
    }
}
