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

/// JSON-RPC request structure
#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: Vec<()>,
    id: u64,
}

/// JSON-RPC response structure
#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    result: Option<T>,
    error: Option<JsonRpcError>,
}

/// JSON-RPC error structure
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl KatanaRpcClient {
    /// Create a new RPC client with the given endpoint URL.
    ///
    /// # Arguments
    /// * `url` - The Katana RPC endpoint URL (e.g., "http://localhost:5050")
    ///
    /// # Example
    /// ```no_run
    /// use katana_tee_client::rpc::KatanaRpcClient;
    ///
    /// let client = KatanaRpcClient::new("http://localhost:5050");
    /// ```
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
    ///
    /// This calls the `tee_generateQuote` RPC method which returns:
    /// - The SEV-SNP attestation report (1184 bytes)
    /// - The current state root
    /// - The current block hash
    /// - The current block number
    ///
    /// # Example
    /// ```no_run
    /// use katana_tee_client::rpc::KatanaRpcClient;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = KatanaRpcClient::new("http://185.26.9.157:5050");
    /// let quote = client.fetch_attestation().await?;
    /// println!("Block: {}", quote.block_number);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_attestation(&self) -> Result<TeeQuoteResponse, Error> {
        info!("Fetching TEE attestation from {}", self.url);

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "tee_generateQuote",
            params: vec![],
            id: 1,
        };

        debug!("Sending RPC request: {:?}", request);

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

        let json_response: JsonRpcResponse<TeeQuoteResponse> = response
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
            result.quote.len() / 2 - 1 // Approximate byte count from hex
        );

        Ok(result)
    }

    /// Fetch attestation (blocking version for non-async contexts).
    ///
    /// This is useful when you need to call from a synchronous context.
    pub fn fetch_attestation_blocking(&self) -> Result<TeeQuoteResponse, Error> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| Error::Rpc(format!("Failed to create runtime: {}", e)))?;
        rt.block_on(self.fetch_attestation())
    }

    /// Alias for `fetch_attestation` for consistency with RPC method name.
    pub async fn generate_quote(&self) -> Result<TeeQuoteResponse, Error> {
        self.fetch_attestation().await
    }
}
