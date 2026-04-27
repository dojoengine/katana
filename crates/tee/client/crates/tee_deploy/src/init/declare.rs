use std::{fs, sync::Arc};

use cairo_lang_starknet_classes::{
    casm_contract_class::CasmContractClass, contract_class::ContractClass,
};
use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};
use starknet_core::types::{contract::SierraClass, Felt};
use starknet_rust::{
    accounts::{Account, SingleOwnerAccount},
    core::types::DeclareTransactionResult,
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::LocalWallet,
};
use tracing::{info, warn};

pub async fn declare_contract(
    account: &SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    contract_class_path: &str,
) -> Result<(Option<DeclareTransactionResult>, Felt), Box<dyn std::error::Error + Send + Sync>> {
    let contract_class_bytes = fs::read(contract_class_path)?;

    let deserialized_class: SierraClass = serde_json::from_slice(&contract_class_bytes)?;

    let flattened = Arc::new(deserialized_class.flatten()?);
    let class_hash = flattened.class_hash();
    info!("Class hash: {:?}", class_hash);

    let casm_class = casm_class_hash_from_bytes(&contract_class_bytes, true);

    match account.declare_v3(flattened, casm_class).send().await {
        Ok(tx) => {
            info!(
                "Declaration transaction sent. Tx hash: {:#x}",
                tx.transaction_hash
            );
            Ok((Some(tx), class_hash))
        }
        Err(e) => {
            let error_msg = format!("{:?}", e);
            if error_msg.contains("is already declared") {
                warn!("Class is already declared (class hash: {:?})", class_hash);
                Ok((None, class_hash))
            } else {
                Err(e.into())
            }
        }
    }
}

fn casm_class_hash_from_bytes(data: &[u8], use_blake2s: bool) -> Felt {
    let sierra_class: ContractClass = serde_json::from_slice(data).unwrap();
    let casm_class =
        CasmContractClass::from_contract_class(sierra_class, false, usize::MAX).unwrap();

    let hash_version = if use_blake2s {
        HashVersion::V2
    } else {
        HashVersion::V1
    };
    let hash = casm_class.hash(&hash_version);

    Felt::from_bytes_be(&hash.0.to_bytes_be())
}
