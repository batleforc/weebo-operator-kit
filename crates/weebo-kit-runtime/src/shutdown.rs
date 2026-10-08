//! Graceful shutdown: every controller stops taking new work, lets its
//! in-flight reconciles finish, and returns.

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::signal::unix::{SignalKind, signal};

/// Resolves once the process is asked to stop
/// (`Controller::graceful_shutdown_on`).
#[derive(Clone)]
pub struct Shutdown(Shared<BoxFuture<'static, ()>>);

impl Shutdown {
    pub fn new(trigger: impl Future<Output = ()> + Send + 'static) -> Self {
        Self(trigger.boxed().shared())
    }

    /// On SIGTERM or SIGINT. The binary is PID 1 in its container: without
    /// a handler, SIGTERM would be ignored and every rollout would wait for
    /// the SIGKILL.
    pub fn on_termination() -> std::io::Result<Self> {
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        Ok(Self::new(async move {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
            tracing::info!("termination requested, finishing in-flight reconciles");
        }))
    }

    /// For tests and tools whose controllers run until dropped.
    pub fn never() -> Self {
        Self::new(futures::future::pending())
    }

    pub fn wait(&self) -> impl Future<Output = ()> + Send + Sync + 'static {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_clone_sees_the_trigger() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let shutdown = Shutdown::new(async move {
            let _ = rx.await;
        });
        let waiter = tokio::spawn(shutdown.clone().wait());
        tx.send(()).unwrap();
        waiter.await.unwrap();
        shutdown.wait().await;
    }
}
