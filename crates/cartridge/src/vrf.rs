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

use std::str::FromStr;

use ark_ec::short_weierstrass::Affine;
use katana_primitives::cairo::ShortString;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_rpc_types::outside_execution::{OutsideExecutionV2, OutsideExecutionV3};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use stark_vrf::{generate_public_key, BaseField, StarkCurve, StarkVRF};
use tracing::trace;
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
    pub gamma_x: String,
    pub gamma_y: String,
    pub c: String,
    pub s: String,
    pub sqrt_ratio: String,
    pub rnd: String,
}

#[derive(Debug, Clone)]
pub struct VrfContext {
    private_key: Felt,
    public_key: Affine<StarkCurve>,
    contract_address: ContractAddress,
}

impl VrfContext {
    /// Creates a new [`VrfContext`] with the given private key and provider address.
    pub fn new(private_key: Felt, provider: ContractAddress) -> Self {
        let public_key = generate_public_key(private_key.to_biguint().into());

        let contract_address = compute_vrf_address(
            provider,
            Felt::from_hex(&format(public_key.x)).unwrap(),
            Felt::from_hex(&format(public_key.y)).unwrap(),
        );

        Self { private_key, public_key, contract_address }
    }

    /// Get the public key x and y coordinates as Felt values.
    pub fn get_public_key_xy_felts(&self) -> (Felt, Felt) {
        let x = Felt::from_hex(&format(self.public_key.x)).unwrap();
        let y = Felt::from_hex(&format(self.public_key.y)).unwrap();

        (x, y)
    }

    /// Retruns the address of the VRF contract.
    pub fn address(&self) -> ContractAddress {
        self.contract_address
    }

    /// Returns the private key of the VRF.
    pub fn private_key(&self) -> Felt {
        self.private_key
    }

    /// Returns the public key of the VRF.
    pub fn public_key(&self) -> &Affine<StarkCurve> {
        &self.public_key
    }

    /// Computes a VRF proof for the given seed.
    pub fn stark_vrf(&self, seed: Felt) -> anyhow::Result<StarkVrfProof> {
        let private_key = self.private_key.to_string();
        let public_key = self.public_key;

        let seed = vec![BaseField::from(seed.to_biguint())];
        let ecvrf = StarkVRF::new(public_key)?;
        let proof = ecvrf.prove(&private_key.parse().unwrap(), seed.as_slice())?;
        let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed.as_slice());
        let rnd = ecvrf.proof_to_hash(&proof)?;

        let beta = ecvrf.proof_to_hash(&proof)?;

        trace!(target: "cartridge", seed = ?seed[0], random_value = %format(beta), "Computing VRF proof.");

        Ok(StarkVrfProof {
            gamma_x: format(proof.0.x),
            gamma_y: format(proof.0.y),
            c: format(proof.1),
            s: format(proof.2),
            sqrt_ratio: format(sqrt_ratio_hint),
            rnd: format(rnd),
        })
    }
}

/// Computes the deterministic VRF contract address from the provider address and the public
/// key coordinates.
fn compute_vrf_address(
    provider_addrss: ContractAddress,
    public_key_x: Felt,
    public_key_y: Felt,
) -> ContractAddress {
    get_contract_address(
        CARTRIDGE_VRF_SALT.into(),
        CARTRIDGE_VRF_CLASS_HASH,
        &[*provider_addrss, public_key_x, public_key_y],
        Felt::ZERO,
    )
    .into()
}

/// Formats the given value as a hexadecimal string.
fn format<T: std::fmt::Display>(v: T) -> String {
    let int = BigInt::from_str(&format!("{v}")).unwrap();
    format!("0x{}", int.to_str_radix(16))
}

// -------------------------------------------------------------------------------------
// VRF HTTP Client
// -------------------------------------------------------------------------------------

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

/// Request body for POST /proof endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRequest {
    pub seed: Vec<String>,
}

/// Request context for outside execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub chain_id: String,
    pub rpc_url: Option<String>,
}

/// Request body for POST /outside_execution endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutsideExecutionRequest {
    pub request: SignedOutsideExecution,
    pub context: RequestContext,
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
    pub async fn proof(&self, request: &ProofRequest) -> Result<StarkVrfProof, VrfClientError> {
        let url = self.url.join("/proof")?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(request)
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
        request: &OutsideExecutionRequest,
    ) -> Result<SignedOutsideExecution, VrfClientError> {
        let url = self.url.join("/outside_execution")?;
        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(request)
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
