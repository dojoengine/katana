// utils.rs
use crate::core_contract::constants::{
    BOOTLOADER_PROGRAM_HASH, INITIAL_BLOCK_HASH, INITIAL_BLOCK_NUMBER, INITIAL_STATE_ROOT,
    LAYOUT_BRIDGE_PROGRAM_HASH, SNOS_PROGRAM_HASH,
};
use anyhow::anyhow;
use anyhow::Result;
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use dojo_utils::{Declarer, Deployer, Invoker, LabeledClass, TransactionResult, TxnConfig};
use starknet::accounts::{Account, SingleOwnerAccount};
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::{contract::SierraClass, Call, Felt, FlattenedSierraClass};
use starknet::macros::{selector, short_string};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::LocalWallet;
use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};
use std::{fs, path::Path};
use tracing::{debug, trace, warn};

pub fn compute_starknet_os_config_hash(chain_id: Felt, fee_token: Felt) -> Felt {
    const STARKNET_OS_CONFIG_VERSION: Felt = short_string!("StarknetOsConfig3");

    compute_hash_on_elements(&[STARKNET_OS_CONFIG_VERSION, chain_id, fee_token])
}

pub async fn declare_contract(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_name: &str,
    contract_path: &Path,
) -> Result<(Felt, TransactionResult)> {
    let txn_config = TxnConfig::default();

    let mut declarer = Declarer::new(account, txn_config);
    let class = prepare_class(contract_path, true)?;
    let labeled = LabeledClass {
        label: class.label.clone(),
        casm_class_hash: class.casm_class_hash,
        class: class.class.clone(),
    };
    declarer.add_class(labeled);
    let mut results = declarer.declare_all().await?;
    let tx_result = results.remove(0);

    match &tx_result {
        TransactionResult::Noop => {
            debug!(contract = %contract_name, "Contract already declared.");
        }
        TransactionResult::Hash(hash) => {
            debug!(contract = %contract_name, "Contract declared.");
            trace!(tx_hash = ?hash, "Tx hash");
        }
        TransactionResult::HashReceipt(hash, receipt) => {
            debug!(contract = %contract_name, "Contract declared.");
            trace!(tx_hash = ?hash, block = receipt.block.block_number(), "Declared on block");
        }
    }
    Ok((class.class_hash, tx_result))
}

pub async fn declare_contract_from_bytes(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_name: &str,
    contract_bytes: &[u8],
) -> Result<(Felt, TransactionResult)> {
    let txn_config = TxnConfig::default();

    let mut declarer = Declarer::new(account, txn_config);
    let class = prepare_class_from_bytes(contract_bytes, true, contract_name.to_string())?;
    let labeled = LabeledClass {
        label: class.label.clone(),
        casm_class_hash: class.casm_class_hash,
        class: class.class.clone(),
    };
    declarer.add_class(labeled);
    let mut results = declarer.declare_all().await?;
    let tx_result = results.remove(0);

    match &tx_result {
        TransactionResult::Noop => {
            debug!(contract = %contract_name, "Contract already declared.");
        }
        TransactionResult::Hash(hash) => {
            debug!(contract = %contract_name, "Contract declared.");
            trace!(tx_hash = ?hash, "Tx hash");
        }
        TransactionResult::HashReceipt(hash, receipt) => {
            debug!(contract = %contract_name, "Contract declared.");
            trace!(tx_hash = ?hash, block = receipt.block.block_number(), "Declared on block");
        }
    }
    Ok((class.class_hash, tx_result))
}

pub async fn deploy_core_contract(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_name: &str,
    class_hash: Felt,
    salt: Felt,
) -> Result<(Felt, TransactionResult)> {
    let constructor_calldata: Vec<Felt> = vec![
        // owner.
        account.address(),
        // state root.
        INITIAL_STATE_ROOT,
        // block_number must be magic value for genesis block.
        INITIAL_BLOCK_NUMBER,
        // block_hash.
        INITIAL_BLOCK_HASH,
    ];

    deploy_contract(
        account,
        contract_name,
        class_hash,
        salt,
        &constructor_calldata,
    )
    .await
}

pub async fn deploy_contract(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_name: &str,
    class_hash: Felt,
    salt: Felt,
    constructor_calldata: &[Felt],
) -> Result<(Felt, TransactionResult)> {
    let txn_config = dojo_utils::TxnConfig {
        receipt: true,
        ..Default::default()
    };

    let deployer = Deployer::new(account.clone(), txn_config);

    match deployer
        .deploy_via_udc(class_hash, salt, constructor_calldata, Felt::ZERO)
        .await
    {
        Ok((contract_address, transaction_result)) => {
            match &transaction_result {
                TransactionResult::Noop => {
                    debug!(contract = %contract_name, "Contract already deployed.");
                }
                TransactionResult::Hash(hash) => {
                    debug!(contract = %contract_name, "Contract deployed.");
                    trace!(tx_hash = ?hash, "Tx hash");
                }
                TransactionResult::HashReceipt(hash, receipt) => {
                    debug!(contract = %contract_name, "Contract deployed.");
                    trace!(tx_hash = ?hash, block = receipt.block.block_number(), "At block");
                }
            }
            Ok((contract_address, transaction_result))
        }
        Err(e) => {
            let address = try_extract_already_deployed_address(&e)?;
            if let Some(address) = address {
                warn!(
                    contract = %contract_name,
                    address = ?address,
                    "Contract already deployed at address."
                );
                // Match the "already declared" semantics: represent an
                // idempotent no-op with TransactionResult::Noop so the
                // caller can serialize it uniformly.
                return Ok((address, TransactionResult::Noop));
            }
            Err(anyhow!("{} deployment failed: {:?}", contract_name, e))
        }
    }
}

fn try_extract_already_deployed_address<E: std::fmt::Debug>(e: &E) -> anyhow::Result<Option<Felt>> {
    const MARKER: &str = "already deployed at address ";
    let msg = format!("{:?}", e);

    let i = match msg.find(MARKER) {
        Some(i) => i + MARKER.len(),
        None => return Ok(None),
    };

    let rest = &msg[i..];
    let end = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_ascii_hexdigit() || c == 'x'))
        .map(|(j, _)| j)
        .unwrap_or(rest.len());

    let addr = &rest[..end];
    if !addr.starts_with("0x") {
        return Ok(None);
    }

    Ok(Some(Felt::from_hex(addr)?))
}

pub async fn set_program_info(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_address: Felt,
    snos_config_hash: Felt,
) -> Result<TransactionResult> {
    let txn_config = TxnConfig::default();
    let invoker = Invoker::new(account, txn_config);

    let call = Call {
        to: contract_address,
        selector: selector!("set_program_info"),
        calldata: vec![
            BOOTLOADER_PROGRAM_HASH,
            snos_config_hash,
            SNOS_PROGRAM_HASH,
            LAYOUT_BRIDGE_PROGRAM_HASH,
        ],
    };
    let tx = invoker.invoke(call).await.unwrap();
    Ok(tx)
}

pub async fn set_fact_registry(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_address: Felt,
    fact_registry_address: Felt,
) -> Result<TransactionResult> {
    let txn_config = TxnConfig::default();
    let invoker = Invoker::new(account, txn_config);

    let call = Call {
        to: contract_address,
        selector: selector!("set_facts_registry"),
        calldata: vec![fact_registry_address],
    };
    let tx = invoker.invoke(call).await.unwrap();
    Ok(tx)
}

#[derive(Debug, Clone)]
struct PreparedClass {
    label: String,
    class_hash: Felt,
    casm_class_hash: Felt,
    class: FlattenedSierraClass,
}

fn prepare_class(path: &Path, use_blake2s: bool) -> Result<PreparedClass> {
    let data = fs::read(path)?;
    prepare_class_from_bytes(&data, use_blake2s, path.display().to_string())
}

fn prepare_class_from_bytes(
    data: &[u8],
    use_blake2s: bool,
    label: String,
) -> Result<PreparedClass> {
    let sierra: SierraClass = serde_json::from_slice(data)?;
    let class_hash = sierra.class_hash()?;
    let flattened = sierra.clone().flatten()?;

    let casm_hash = casm_class_hash_from_bytes(data, use_blake2s)?;

    Ok(PreparedClass {
        label,
        class_hash,
        casm_class_hash: casm_hash,
        class: flattened,
    })
}

fn casm_class_hash_from_bytes(data: &[u8], use_blake2s: bool) -> Result<Felt> {
    let sierra_class: ContractClass = serde_json::from_slice(data)?;
    let extraced_sierra_class = sierra_class.extract_sierra_program(false)?;
    let casm_class = CasmContractClass::from_contract_class(
        sierra_class,
        extraced_sierra_class,
        false,
        usize::MAX,
    )?;

    let hash_version = if use_blake2s {
        HashVersion::V2
    } else {
        HashVersion::V1
    };
    let hash = casm_class.hash(&hash_version);

    Ok(Felt::from_bytes_be(&hash.0.to_bytes_be()))
}

#[cfg(test)]
mod test {
    use super::*;
    use starknet::core::chain_id::{MAINNET, SEPOLIA};
    use starknet::core::types::Felt;
    use starknet::macros::felt;
    const STRK_FEE_TOKEN: Felt =
        felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

    #[test]
    fn calculate_config_hash_mainnet() {
        let expected = felt!("0x70c7b342f93155315d1cb2da7a4e13a3c2430f51fb5696c1b224c3da5508dfb");
        let chain = MAINNET;

        let computed = compute_starknet_os_config_hash(chain, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }

    #[test]
    fn calculate_config_hash_testnet() {
        let expected = felt!("0x1b9900f77ff5923183a7795fcfbb54ed76917bc1ddd4160cc77fa96e36cf8c5");
        let chain = SEPOLIA;
        let computed = compute_starknet_os_config_hash(chain, STRK_FEE_TOKEN);

        assert_eq!(computed, expected);
    }
}
