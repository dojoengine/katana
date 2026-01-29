use ark_ec::short_weierstrass::Affine;
use cainome_cairo_serde::ContractAddress;
use serde::{Deserialize, Serialize};
use stark_vrf::StarkCurve;
use starknet::{
    core::utils::cairo_short_string_to_felt,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Url},
    signers::LocalWallet,
};
use starknet_crypto::Felt;

use crate::{routes::outside_execution::Errors, state::AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestContext {
    pub chain_id: String,
    pub rpc_url: Option<String>,
}

#[derive(Debug)]
pub struct VrfContext {
    pub chain_id: Felt,
    pub provider: JsonRpcClient<HttpTransport>,
    //
    pub secret_key: String,
    pub public_key: Affine<StarkCurve>,
    pub vrf_account_address: ContractAddress,
    pub vrf_signer: LocalWallet,
}

impl VrfContext {
    pub fn build_from(
        request_context: RequestContext,
        app_state: &AppState,
    ) -> Result<Self, Errors> {
        let chain_id = cairo_short_string_to_felt(&request_context.chain_id)?;

        let rpc_url = match request_context.rpc_url {
            Some(rpc_url) => rpc_url,
            None => {
                if request_context.chain_id.as_str() == "SN_MAIN" {
                    "https://api.cartridge.gg/x/starknet/mainnet".into()
                } else if request_context.chain_id.as_str() == "SN_SEPOLIA" {
                    "https://api.cartridge.gg/x/starknet/sepolia".into()
                } else {
                    return Err(Errors::RequestContextError(
                        "no rpc_url provided".to_owned(),
                    ));
                }
            }
        };

        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&rpc_url)?));

        Ok(VrfContext {
            chain_id,
            provider,
            secret_key: app_state.secret_key.clone(),
            public_key: app_state.public_key,
            vrf_account_address: app_state.vrf_account_address,
            vrf_signer: app_state.vrf_signer.clone(),
        })
    }
}
