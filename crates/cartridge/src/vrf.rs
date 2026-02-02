//! For the VRF, the integration works as follows (if an execution from outside is targetting the
//! VRF provider contract):
//!
//! 1. The VRF provider contract is deployed (if not already deployed).
//!
//! 2. The Stark VRF proof is generated using the `Source` provided in the call. The seed is
//!    precomputed to match smart contract behavior <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L112>.
//!
//! 3. The original execution from outside call is then sandwitched between two VRF calls, one for
//!    submitting the randomness, and one to assert the correct consumption of the randomness.
//!
//! 4. When using the VRF, the user has the responsability to add a first call to target the VRF
//!    provider contract `request_random` entrypoint. This call sets which `Source` will be used
//!    to generate the randomness.
//!    <https://docs.cartridge.gg/vrf/overview#executing-vrf-transactions>
//!
//! In the current implementation, the VRF contract is deployed with a default private key, or read
//! from environment variable `KATANA_VRF_PRIVATE_KEY`. It is important to note that changing the
//! private key will result in a different VRF provider contract address.

use katana_primitives::cairo::ShortString;
use katana_primitives::{felt, Felt};
use katana_rpc_types::outside_execution::{OutsideExecutionV2, OutsideExecutionV3};
use serde::{Deserialize, Serialize};
use url::Url;

// Class hash of the VRF provider contract (fee estimation code commented, since currently Katana
// returns 0 for the fees): <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L124>
// The contract is compiled in
// `crates/controller/artifacts/cartridge_vrf_VrfProvider.contract_class.json`
pub const CARTRIDGE_VRF_CLASS_HASH: Felt =
    felt!("0x07007ea60938ff539f1c0772a9e0f39b4314cfea276d2c22c29a8b64f2a87a58");
pub const CARTRIDGE_VRF_SALT: ShortString = ShortString::from_ascii("cartridge_vrf");
pub const CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY: Felt = felt!("0x1");

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
    pub rpc_url: Option<String>,
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
    /// Creates a new [`VrfClient`] with the given base URL.
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
        #[derive(Debug, Clone, Serialize, Deserialize)]
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
