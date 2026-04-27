use tokio_util::sync::CancellationToken;

/// Long-running background services that support graceful shutdown.
pub trait Daemon: Send {
    fn shutdown_handle(&self) -> ShutdownHandle;

    fn start(self);
}

/// A type for background running services to detect cancellation requests made to them via
/// [`ShutdownHandle`], and for them to signal they've ended execution, either through such a
/// requested shutdown or voluntary exit.
#[derive(Debug, Default, Clone)]
pub struct FinishHandle {
    cancellation: CancellationToken,
    finish: CancellationToken,
}

/// A type for requesting cancellation of background running services and waiting for them to have
/// ended execution, either through such a requested shutdown or voluntary exit.
#[derive(Debug, Clone)]
pub struct ShutdownHandle {
    cancellation: CancellationToken,
    finish: CancellationToken,
}

impl FinishHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            cancellation: self.cancellation.clone(),
            finish: self.finish.clone(),
        }
    }

    /// Signals that the service has finish executing.
    pub fn finish(&self) {
        self.finish.cancel();
    }

    /// Checks whether any shutdown request has been made via [`shutdown`].
    pub fn is_shutdown_requested(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Waits asynchronously for a shutdown request to be made via [`shutdown`].
    ///
    /// This function is typically used by background services to detect cancellation and start
    /// gracefully winding up.
    pub async fn shutdown_requested(&self) {
        self.cancellation.cancelled().await
    }
}

impl ShutdownHandle {
    /// Request a shutdown without waiting for the process to finish.
    ///
    /// Call [`finished`] to wait for the process to finish.
    pub fn shutdown(&self) {
        self.cancellation.cancel();
    }

    /// Waits asynchronously for the service to finish execution, either through a requested
    /// shutdown or voluntary exit.
    pub async fn finished(&self) {
        self.finish.cancelled().await
    }
}
