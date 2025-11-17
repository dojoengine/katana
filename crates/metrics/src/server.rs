use core::fmt;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response};
use tokio::sync::oneshot;
use tracing::info;

use crate::exporters::Exporter;
use crate::{Error, Report};

/// A handle to the metrics server.
#[derive(Debug)]
pub struct MetricsServerHandle {
    /// The actual address that the server is bound to.
    addr: SocketAddr,
    /// The shutdown sender to stop the server.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// The task handle to wait for server completion.
    task_handle: tokio::task::JoinHandle<Result<(), Error>>,
}

impl MetricsServerHandle {
    /// Tell the server to stop without waiting for the server to stop.
    pub fn stop(&mut self) -> Result<(), Error> {
        if let Some(tx) = self.shutdown_tx.take() {
            // Ignore error if receiver already dropped
            let _ = tx.send(());
            Ok(())
        } else {
            Err(Error::AlreadyStopped)
        }
    }

    /// Wait until the server has stopped.
    pub async fn stopped(self) -> Result<(), Error> {
        self.task_handle.await.map_err(|e| Error::JoinError(e.to_string()))?
    }

    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }
}

/// A helper trait for defining the type for hooks that are called when the metrics are being
/// collected by the server.
trait Hook: Fn() + Send + Sync {}
impl<T: Fn() + Send + Sync> Hook for T {}

/// A shared hook that can be cloned.
type SharedHook = Arc<dyn Hook<Output = ()>>;
/// A list of shared hooks.
type Hooks = Vec<SharedHook>;

/// Server for serving metrics.
// TODO: allow configuring the server executor to allow cancelling on invidiual connection tasks.
// See, [hyper::server::server::Builder::executor]
pub struct Server<MetricsExporter> {
    /// Hooks or callable functions for collecting metrics in the cases where
    /// the metrics are not being collected in the main program flow.
    ///
    /// These are called when metrics are being served through the server.
    hooks: Hooks,
    /// The exporter that is used to export the collected metrics.
    exporter: MetricsExporter,
}

impl<MetricsExporter> Server<MetricsExporter>
where
    MetricsExporter: Exporter + 'static,
{
    /// Creates a new metrics server using the given exporter.
    pub fn new(exporter: MetricsExporter) -> Self {
        Self { exporter, hooks: Vec::new() }
    }

    /// Add new metrics reporter to the server.
    pub fn with_reports<I>(mut self, reports: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn Report>>,
    {
        // convert the report types into callable hooks, wrapping in Arc for sharing
        let hooks = reports.into_iter().map(|r| Arc::new(move || r.report()) as SharedHook);
        self.hooks.extend(hooks);
        self
    }

    pub fn with_process_metrics(mut self) -> Self {
        use crate::sys::process::{collect_memory_stats, describe_memory_stats};

        let process = metrics_process::Collector::default();
        process.describe();
        describe_memory_stats();

        let hooks: Hooks =
            vec![Arc::new(collect_memory_stats), Arc::new(move || process.collect())];

        self.hooks.extend(hooks);
        self
    }

    /// Starts an endpoint at the given address to serve Prometheus metrics.
    ///
    /// Returns a handle that can be used to stop the server and wait for it to finish.
    pub async fn start(&self, addr: SocketAddr) -> Result<MetricsServerHandle, Error> {
        // Clone the hooks (clones the Arc references, not the closures themselves)
        let hooks = self.hooks.clone();
        let exporter = self.exporter.clone();

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let server = hyper::Server::try_bind(&addr)
            .map_err(|_| Error::FailedToBindAddress { addr })?
            .serve(make_service_fn(move |_| {
                let hooks = hooks.clone();
                let exporter = exporter.clone();
                async move {
                    Ok::<_, Infallible>(service_fn(move |_: Request<Body>| {
                        let hooks = hooks.clone();
                        let exporter = exporter.clone();
                        async move {
                            // need to call the hooks to collect metrics before exporting them
                            for hook in &hooks {
                                hook();
                            }
                            // export the metrics from the installed exporter and send as response
                            let metrics = Body::from(exporter.export());
                            Ok::<_, Infallible>(Response::new(metrics))
                        }
                    }))
                }
            }));

        let actual_addr = server.local_addr();

        // Spawn the server with graceful shutdown
        let task_handle = tokio::spawn(async move {
            server
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .map_err(Error::Server)
        });

        info!(target: "metrics", addr = %actual_addr, "Metrics server started.");

        Ok(MetricsServerHandle { addr: actual_addr, shutdown_tx: Some(shutdown_tx), task_handle })
    }
}

impl<MetricsExporter> fmt::Debug for Server<MetricsExporter> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("hooks", &format_args!("{} hook(s)", self.hooks.len()))
            .field("exporter", &"<exporter>")
            .finish()
    }
}
