//! VRF (Verifiable Random Function) service for Cartridge.

use cartridge::vrf::{RequestContext, SignedOutsideExecution, VrfOutsideExecution};
use cartridge::VrfClient;
use katana_primitives::chain::ChainId;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::outside_execution::OutsideExecution;
use starknet::macros::selector;
use url::Url;

#[derive(Debug, Clone)]
pub struct VrfServiceConfig {
    pub rpc_url: Url,
    pub service_url: Url,
    pub vrf_contract: ContractAddress,
}

#[derive(Clone)]
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
        address: ContractAddress,
        outside_execution: &OutsideExecution,
        signature: &[Felt],
        chain_id: ChainId,
    ) -> Result<SignedOutsideExecution, StarknetApiError> {
        let vrf_outside_execution = match outside_execution {
            OutsideExecution::V2(v2) => VrfOutsideExecution::V2(v2.clone()),
            OutsideExecution::V3(v3) => VrfOutsideExecution::V3(v3.clone()),
        };

        let request = SignedOutsideExecution {
            address: address.into(),
            outside_execution: vrf_outside_execution,
            signature: signature.to_vec(),
        };

        let context = RequestContext {
            chain_id: chain_id.id().to_hex_string(),
            rpc_url: Some(self.rpc_url.clone()),
        };

        self.client.outside_execution(request, context).await.map_err(|err| {
            StarknetApiError::unexpected(format!("vrf outside_execution failed: {err}"))
        })
    }
}

pub(super) fn request_random_call(
    outside_execution: &OutsideExecution,
) -> Option<(katana_rpc_types::outside_execution::Call, usize)> {
    outside_execution
        .calls()
        .iter()
        .position(|call| call.selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}

pub(super) fn outside_execution_calls_len(outside_execution: &OutsideExecution) -> usize {
    match outside_execution {
        OutsideExecution::V2(v2) => v2.calls.len(),
        OutsideExecution::V3(v3) => v3.calls.len(),
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{felt, Felt};
    use katana_rpc_types::outside_execution::OutsideExecutionV2;

    use super::*;

    const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552");

    #[test]
    fn request_random_call_finds_position() {
        let vrf_address = ContractAddress::from(felt!("0x123"));
        let other_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("transfer"),
            calldata: vec![Felt::ONE],
        };
        let vrf_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("request_random"),
            calldata: vec![Felt::TWO],
        };

        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            execute_after: 0,
            execute_before: 100,
            calls: vec![other_call.clone(), vrf_call.clone()],
            nonce: Felt::THREE,
        });

        let (call, position) =
            request_random_call(&outside_execution).expect("request_random found");
        assert_eq!(position, 1);
        assert_eq!(call.selector, vrf_call.selector);
        assert_eq!(call.calldata, vrf_call.calldata);
    }
}
