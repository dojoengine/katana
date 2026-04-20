//! VRF (Verifiable Random Function) service for Cartridge.

use cartridge::vrf::{RequestContext, VrfClient};
use katana_primitives::chain::ChainId;
use katana_primitives::ContractAddress;
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_types::SignedOutsideExecution;
use url::Url;

#[derive(Debug, Clone)]
pub struct VrfServiceConfig {
    pub rpc_url: Url,
    pub service_url: Url,
    pub vrf_contract: ContractAddress,
}

#[derive(Debug, Clone)]
pub struct VrfService {
    client: VrfClient,
    account_address: ContractAddress,
    rpc_url: Url,
}

impl VrfService {
    pub fn new(config: VrfServiceConfig) -> Self {
        Self {
            client: VrfClient::new(config.service_url),
            account_address: config.vrf_contract,
            rpc_url: config.rpc_url,
        }
    }

    pub fn account_address(&self) -> ContractAddress {
        self.account_address
    }

    /// Delegates outside execution to the VRF server.
    ///
    /// The VRF server handles seed computation, proof generation, and signing.
    pub async fn outside_execution(
        &self,
        outside_execution: &SignedOutsideExecution,
        chain_id: ChainId,
    ) -> Result<SignedOutsideExecution, CartridgeApiError> {
        let context = RequestContext {
            chain_id: chain_id.id().to_hex_string(),
            rpc_url: Some(self.rpc_url.clone()),
        };

        self.client
            .outside_execution(outside_execution, &context)
            .await
            .map_err(|err| CartridgeApiError::VrfExecutionFailed { reason: err.to_string() })
    }
}
