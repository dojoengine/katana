use std::sync::Arc;

use starknet_core::{
    types::{Felt, InvokeTransactionResult},
    utils::get_udc_deployed_address,
};
use starknet_rust::{
    accounts::SingleOwnerAccount,
    contract::{ContractFactory, UdcSelector},
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::LocalWallet,
};
use tracing::info;

pub async fn deploy(
    account: &SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Option<Felt>,
    unique: bool,
) -> Result<(Option<InvokeTransactionResult>, Felt), Box<dyn std::error::Error + Send + Sync>> {
    let udc = UdcSelector::New;
    let contract_factory = ContractFactory::new_with_udc(class_hash, account, udc);
    let salt = salt.unwrap_or_else(|| Felt::from_hex_unchecked("0x0"));
    let address = get_udc_deployed_address(
        salt,
        class_hash,
        &starknet_core::utils::UdcUniqueness::NotUnique,
        &constructor_calldata,
    );
    info!("Deploying contract to address: {:?}", address);
    match contract_factory
        .deploy_v3(constructor_calldata, salt, unique)
        .send()
        .await
    {
        Ok(tx) => Ok((Some(tx), address)),
        Err(e) => {
            let msg = format!("{:?}", e);
            if msg.contains("already deployed at address") {
                tracing::warn!("Contract already deployed at address: {:?}", address);
                Ok((None, address))
            } else {
                Err(e.into())
            }
        }
    }
}
