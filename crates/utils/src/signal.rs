use std::io;

use tokio::signal::ctrl_c;

/// Returns a future for awaiting on OS signals to be received - `SIGTERM` (Unix only), `SIGINT`.
///
/// Can be used to handle graceful shutdowns.
pub async fn wait_shutdown_signals() {
    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c() => {},
        _ = sigterm() => {},
    }

    #[cfg(not(unix))]
    tokio::select! {
        _ = ctrl_c() => {},
    }
}

/// Returns a future that can be awaited to wait for the `SIGTERM` signal.
#[cfg(unix)]
async fn sigterm() -> io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    signal(SignalKind::terminate())?.recv().await;
    Ok(())
}
