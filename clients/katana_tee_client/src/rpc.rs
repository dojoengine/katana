//! Katana RPC Client for TEE Attestation
//!
//! This module provides functionality to fetch TEE attestation quotes
//! from a Katana node's RPC endpoint.

use crate::{Error, TeeQuoteResponse};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Katana RPC client for fetching TEE attestations.
#[derive(Debug, Clone)]
pub struct KatanaRpcClient {
    /// The RPC endpoint URL
    url: String,
    /// HTTP client
    client: reqwest::Client,
}

impl KatanaRpcClient {
    /// Create a new RPC client with the given endpoint URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a new RPC client from environment variable.
    ///
    /// Reads from `KATANA_RPC_URL` environment variable, falling back to
    /// `http://localhost:5050` if not set.
    pub fn from_env() -> Self {
        let url =
            std::env::var("KATANA_RPC_URL").unwrap_or_else(|_| "http://localhost:5050".to_string());
        Self::new(url)
    }

    /// Get the RPC URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Fetch a TEE attestation quote from the Katana node.
    pub async fn fetch_attestation(&self, prev_block_number: u64, block_number: u64) -> Result<TeeQuoteResponse, Error> {
        info!("Fetching TEE attestation from {}", self.url);

        #[derive(Serialize)]
        struct Request<'a> {
            jsonrpc: &'a str,
            method: &'a str,
            params: [u64; 2],
            id: u64,
        }

        #[derive(Deserialize)]
        struct Response {
            result: Option<TeeQuoteResponse>,
            error: Option<RpcError>,
        }

        #[derive(Deserialize)]
        struct RpcError {
            code: i64,
            message: String,
        }

        let request = Request {
            jsonrpc: "2.0",
            method: "tee_generateQuote",
            params: [prev_block_number, block_number],
            id: 1,
        };

        debug!("Sending RPC request to {}", self.url);

        let response = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Rpc(format!("Failed to send request: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Rpc(format!(
                "HTTP error {}: {}",
                status.as_u16(),
                body
            )));
        }

        let json_response: Response = response
            .json()
            .await
            .map_err(|e| Error::Rpc(format!("Failed to parse response: {}", e)))?;

        if let Some(error) = json_response.error {
            return Err(Error::Rpc(format!(
                "RPC error {}: {}",
                error.code, error.message
            )));
        }

        let result = json_response
            .result
            .ok_or_else(|| Error::Rpc("Empty result in RPC response".to_string()))?;

        info!(
            "Received attestation for block {} ({} bytes)",
            result.block_number,
            result.quote.len() / 2 - 1
        );

        Ok(result)
    }

    /// Fetch attestation (blocking version for non-async contexts).
    pub fn fetch_attestation_blocking(&self, prev_block_number: u64, block_number: u64) -> Result<TeeQuoteResponse, Error> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| Error::Rpc(format!("Failed to create runtime: {}", e)))?;
        rt.block_on(self.fetch_attestation(prev_block_number, block_number))
    }

    /// Alias for `fetch_attestation` for consistency with RPC method name.
    pub async fn generate_quote(&self, prev_block_number: u64, block_number: u64) -> Result<TeeQuoteResponse, Error> {
        self.fetch_attestation(prev_block_number, block_number).await
    }
}
