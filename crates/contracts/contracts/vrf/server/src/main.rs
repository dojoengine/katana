pub mod oracle;
pub mod routes;
pub mod state;
pub mod utils;

#[cfg(test)]
pub mod tests {
    pub mod setup;
    pub mod test_info;
    pub mod test_outisde_execution;
}

use crate::routes::outside_execution::vrf_outside_execution;
use crate::routes::proof::vrf_proof;
use crate::state::AppState;
use crate::{routes::info::vrf_info, state::SharedState};
use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use std::sync::{Arc, RwLock};
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::debug;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// http host
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// http port
    #[arg(short, long, default_value_t = 3000)]
    port: u64,

    /// Secret key
    #[arg(short, long, required = true)]
    secret_key: u64,

    /// Account Address
    #[arg(long, required = true)]
    account_address: String,

    /// Account Private Key
    #[arg(long, required = true)]
    account_private_key: String,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            host: "0.0.0.0".into(),
            port: 3000,
            account_address: "0x123".into(),
            account_private_key: "0x420".into(),
            secret_key: 420,
        }
    }
}

#[allow(dead_code)]
impl Args {
    fn with_host(mut self, host: &str) -> Args {
        self.host = host.into();
        self
    }
    fn with_port(mut self, port: u64) -> Args {
        self.port = port;
        self
    }
    fn with_account_address(mut self, account_address: &str) -> Args {
        self.account_address = account_address.into();
        self
    }
    fn with_account_private_key(mut self, account_private_key: &str) -> Args {
        self.account_private_key = account_private_key.into();
        self
    }
    fn with_secret_key(mut self, secret_key: u64) -> Args {
        self.secret_key = secret_key;
        self
    }
}

pub async fn create_app(app_state: AppState) -> Router {
    let shared_state = SharedState(Arc::new(RwLock::new(app_state)));
    Router::new()
        .route("/", get("OK"))
        .route("/info", get(vrf_info))
        .route("/proof", post(vrf_proof))
        .route("/outside_execution", post(vrf_outside_execution))
        .layer(TraceLayer::new_for_http())
        .with_state(shared_state)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let app_state = AppState::new().await;
    let app = create_app(app_state).await;

    let bind_addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("Failed to bind to host/port, port already in use by another process. Change the host/port or terminate the other process.");

    debug!("Server started on http://{}", bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
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
}
