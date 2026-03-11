use katana_primitives::Felt;
use katana_rpc_types::outside_execution::{OutsideExecutionV2, OutsideExecutionV3};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct StarkVrfProof {
    pub gamma_x: Felt,
    pub gamma_y: Felt,
    pub c: Felt,
    pub s: Felt,
    pub sqrt_ratio: Felt,
    pub rnd: Felt,
}

/// OutsideExecution enum with tagged serialization for VRF server compatibility.
///
/// Different from `katana_rpc_types::OutsideExecution` which uses untagged serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VrfOutsideExecution {
    V2(OutsideExecutionV2),
    V3(OutsideExecutionV3),
}

/// A signed outside execution request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedOutsideExecution {
    pub address: Felt,
    pub outside_execution: VrfOutsideExecution,
    pub signature: Vec<Felt>,
}

/// Response from GET /info endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoResponse {
    pub public_key_x: String,
    pub public_key_y: String,
}

/// Request context for outside execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub chain_id: String,
    pub rpc_url: Option<Url>,
}

/// Error type for VRF client operations.
#[derive(thiserror::Error, Debug)]
pub enum VrfClientError {
    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("HTTP request error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("server error: {0}")]
    Server(String),
}

/// HTTP client for interacting with the VRF server.
#[derive(Debug, Clone)]
pub struct VrfClient {
    url: Url,
    client: reqwest::Client,
}

impl VrfClient {
    /// Creates a new [`VrfClient`] with the given URL.
    pub fn new(url: Url) -> Self {
        Self { url, client: reqwest::Client::new() }
    }

    /// Health check - GET /
    ///
    /// Returns `Ok(())` if server responds with "OK".
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn health_check(&self) -> Result<(), VrfClientError> {
        let response = self.client.get(self.url.clone()).send().await?;
        let text = response.text().await?;

        if text.trim() == "OK" {
            Ok(())
        } else {
            Err(VrfClientError::Server(format!("unexpected response: {text}")))
        }
    }

    /// Get VRF public key info - GET /info
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn info(&self) -> Result<InfoResponse, VrfClientError> {
        let url = self.url.join("/info")?;
        let response = self.client.get(url).send().await?;
        let info: InfoResponse = response.json().await?;
        Ok(info)
    }

    /// Generate VRF proof - POST /proof
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn proof(&self, seed: Vec<String>) -> Result<StarkVrfProof, VrfClientError> {
        #[derive(Debug, Serialize)]
        struct ProofRequest {
            seed: Vec<String>,
        }

        let url = self.url.join("/proof")?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&ProofRequest { seed })
            .send()
            .await?;

        #[derive(Debug, Deserialize)]
        struct ProofResponse {
            result: StarkVrfProof,
        }

        Ok(response.json::<ProofResponse>().await?.result)
    }

    /// Process outside execution with VRF - POST /outside_execution
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn outside_execution(
        &self,
        request: SignedOutsideExecution,
        context: RequestContext,
    ) -> Result<SignedOutsideExecution, VrfClientError> {
        #[derive(Debug, Serialize)]
        struct OutsideExecutionRequest {
            request: SignedOutsideExecution,
            context: RequestContext,
        }

        let url = self.url.join("/outside_execution")?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&OutsideExecutionRequest { request, context })
            .send()
            .await?;

        // Check for error responses (server returns 404 with JSON error for failures)
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(VrfClientError::Server(error_text));
        }

        #[derive(Debug, Deserialize)]
        struct OutsideExecutionResponse {
            result: SignedOutsideExecution,
        }

        Ok(response.json::<OutsideExecutionResponse>().await?.result)
    }
}
