//! The tokio runtime an operator runs on.

/// Default of `WEEBO_WORKER_THREADS`: tokio worker threads, whatever the
/// node.
///
/// tokio's own default is one worker per core, and the chart sets no CPU
/// limit (which that default would follow). Each worker gets its own
/// allocator arena, so on a 16-core node the same load left the idle RSS
/// 14-15% higher after every create/churn/delete cycle, over
/// `Budget::small_cluster()`'s 10%, which was measured with 4 workers.
/// Reconciles wait on the apiserver and the remote API, not on CPU.
pub const DEFAULT_WORKER_THREADS: usize = 4;

/// `WEEBO_WORKER_THREADS`, or [`DEFAULT_WORKER_THREADS`].
pub fn worker_threads() -> usize {
    std::env::var("WEEBO_WORKER_THREADS")
        .ok()
        .and_then(|n| n.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_WORKER_THREADS)
}

/// A multi-thread runtime with [`worker_threads`] workers, I/O and time
/// enabled: what `#[tokio::main]` builds, with the worker count bounded.
pub fn build() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads())
        .enable_all()
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_threads_come_from_the_environment_or_default_to_four() {
        // SAFETY: the variable is only read by `worker_threads`, here.
        unsafe { std::env::remove_var("WEEBO_WORKER_THREADS") };
        assert_eq!(worker_threads(), DEFAULT_WORKER_THREADS);
        for (value, expected) in [("2", 2), ("0", 4), ("many", 4), ("16", 16)] {
            unsafe { std::env::set_var("WEEBO_WORKER_THREADS", value) };
            assert_eq!(worker_threads(), expected, "{value}");
        }
        unsafe { std::env::remove_var("WEEBO_WORKER_THREADS") };
        let runtime = build().unwrap();
        assert_eq!(runtime.metrics().num_workers(), DEFAULT_WORKER_THREADS);
    }
}
