//! TEE Agent - HTTP API for managing Katana TEE VM
//!
//! This agent runs on the TEE machine and provides a REST API to:
//! - Start Katana VM with configurable fork parameters
//! - Stop the running VM
//! - Check VM status
//! - Retrieve serial console logs

mod config;
mod routes;
mod vm;

use axum::{Router, routing::{get, post}};
use clap::Parser;
use config::Config;
use routes::AppState;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;
use vm::VmManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    // Parse CLI arguments
    let config = Config::parse();

    info!("TEE Agent starting...");
    info!("  Boot dir: {:?}", config.boot_dir);
    info!("  Host RPC port: {}", config.host_rpc_port);
    info!("  Dry-run mode: {}", config.dry_run);

    // Validate boot components (unless dry-run)
    if !config.dry_run {
        if let Err(e) = config.validate_boot_components() {
            tracing::warn!("Boot components validation failed: {}", e);
            tracing::warn!("VM start will fail until boot components are available");
        }
    }

    // Create VM manager
    let vm_manager = Arc::new(VmManager::new(config.clone()));

    // Build router
    let app = Router::new()
        .route("/start", post(routes::start_vm))
        .route("/stop", post(routes::stop_vm))
        .route("/status", get(routes::get_status))
        .route("/logs", get(routes::get_logs))
        .route("/health", get(routes::health))
        .with_state(vm_manager.clone() as AppState)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Start server
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("TEE Agent listening on http://{}", addr);
    info!("");
    info!("Endpoints:");
    info!("  POST /start  - Start VM with fork parameters");
    info!("  POST /stop   - Stop running VM");
    info!("  GET  /status - Get VM status");
    info!("  GET  /logs   - Get serial console logs");
    info!("  GET  /health - Health check");

    // Run server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(vm_manager))
        .await?;

    info!("TEE Agent stopped");
    Ok(())
}

/// Handle shutdown signals (Ctrl+C, SIGTERM)
async fn shutdown_signal(vm_manager: Arc<VmManager>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, cleaning up...");

    // Stop VM if running
    if let Err(e) = vm_manager.stop().await {
        tracing::debug!("VM stop during shutdown: {}", e);
    }
}
