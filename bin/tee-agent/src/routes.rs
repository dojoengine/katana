//! HTTP API routes for tee-agent.

use crate::vm::{StartRequest, VmManager, VmStatus};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared application state
pub type AppState = Arc<VmManager>;

// ============================================================================
// Request/Response types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub status: String,
    pub rpc_url: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct StopResponse {
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_lines")]
    pub lines: usize,
}

fn default_lines() -> usize {
    100
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub logs: String,
}

// ============================================================================
// Route handlers
// ============================================================================

/// POST /start - Start a new Katana VM
pub async fn start_vm(
    State(vm_manager): State<AppState>,
    Json(req): Json<StartRequest>,
) -> impl IntoResponse {
    tracing::info!(
        "Starting VM: fork_block={}, fork_provider={}, port={}",
        req.fork_block,
        req.fork_provider,
        req.port
    );

    match vm_manager.start(req).await {
        Ok(rpc_url) => {
            let status = vm_manager.status().await;
            (
                StatusCode::OK,
                Json(StartResponse {
                    status: "started".to_string(),
                    rpc_url,
                    pid: status.pid,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to start VM: {:#}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("{:#}", e),
                }),
            )
                .into_response()
        }
    }
}

/// POST /stop - Stop the running VM
pub async fn stop_vm(State(vm_manager): State<AppState>) -> impl IntoResponse {
    tracing::info!("Stopping VM");

    match vm_manager.stop().await {
        Ok(()) => (
            StatusCode::OK,
            Json(StopResponse {
                status: "stopped".to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to stop VM: {:#}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("{:#}", e),
                }),
            )
                .into_response()
        }
    }
}

/// GET /status - Get current VM status
pub async fn get_status(State(vm_manager): State<AppState>) -> impl IntoResponse {
    let status: VmStatus = vm_manager.status().await;
    (StatusCode::OK, Json(status))
}

/// GET /logs - Get serial console logs
pub async fn get_logs(
    State(vm_manager): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> impl IntoResponse {
    match vm_manager.logs(query.lines).await {
        Ok(logs) => (StatusCode::OK, Json(LogsResponse { logs })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("{:#}", e),
            }),
        )
            .into_response(),
    }
}

/// GET /health - Health check endpoint
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}
