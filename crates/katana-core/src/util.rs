use std::{fs, path::PathBuf, sync::Arc};

use blockifier::execution::contract_class::ContractClass;
use starknet_api::{
    core::{ChainId, ClassHash, ContractAddress, Nonce},
    hash::{pedersen_hash_array, StarkFelt, StarkHash},
    stark_felt,
    transaction::{Calldata, ContractAddressSalt, Fee},
};

pub fn get_contract_class(contract_path: &str) -> ContractClass {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), contract_path].iter().collect();
    let raw_contract_class = fs::read_to_string(path).unwrap();
    serde_json::from_str(&raw_contract_class).unwrap()
}

pub fn compute_invoke_transaction_v1_hash(
    contract_address: ContractAddress,
    calldata: Calldata,
    nonce: Nonce,
    max_fee: Fee,
    chain_id: ChainId,
) -> StarkHash {
    pedersen_hash_array(&[
        stark_felt!("0x696e766f6b65"),                     // "invoke"
        stark_felt!("0x1"),                                // version
        *contract_address.0.key(),                         // sender
        stark_felt!("0x0"),                                // entry_point_selector
        pedersen_hash_array(&calldata.0),                  // calldata
        stark_felt!(format!("{:#x}", max_fee.0).as_str()), // max_fee
        stark_felt!(chain_id.as_hex().as_str()),           // chain_id
        nonce.0,                                           // nonce
    ])
}

pub fn compute_deploy_account_transaction_hash(
    contract_address: ContractAddress,
    constructor_calldata: Calldata,
    class_hash: ClassHash,
    contract_address_salt: ContractAddressSalt,
    nonce: Nonce,
    max_fee: Fee,
    chain_id: ChainId,
) -> StarkHash {
    let mut calldata_hash = vec![class_hash.0, contract_address_salt.0];
    calldata_hash.append(&mut Arc::try_unwrap(constructor_calldata.0).unwrap());

    pedersen_hash_array(&[
        stark_felt!("0x6465706c6f795f6163636f756e74"), // "deploy_account"
        stark_felt!("0x1"),                            // version
        *contract_address.0.key(),
        stark_felt!("0x0"),
        pedersen_hash_array(&calldata_hash),
        stark_felt!(format!("{:#x}", max_fee.0).as_str()), // max_fee
        stark_felt!(chain_id.as_hex().as_str()),           // chain_id
        nonce.0,                                           // nonce
    ])
}

#[allow(unused)]
pub fn compute_declare_v2_transaction_hash(
    contract_address: ContractAddress,
    class_hash: ClassHash,
    nonce: Nonce,
    max_fee: Fee,
    chain_id: ChainId,
    compiled_class_hash: StarkHash,
) -> StarkHash {
    pedersen_hash_array(&[
        stark_felt!("0x6465636c617265"), // "declare"
        stark_felt!("0x2"),              // version
        *contract_address.0.key(),
        stark_felt!("0x0"),
        pedersen_hash_array(&[class_hash.0]),
        stark_felt!(format!("{:#x}", max_fee.0).as_str()), // max_fee
        stark_felt!(chain_id.as_hex().as_str()),           // chain_id
        nonce.0,                                           // nonce
        compiled_class_hash,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_api::{core::PatriciaKey, patricia_key};

    #[test]
    fn test_compute_invoke_transaction_v1_hash() {
        let tx_hash = compute_invoke_transaction_v1_hash(
            ContractAddress(patricia_key!(
                "0x6e848defdc99f11be165536eeba72c9faa24fc19f756749f52e23acd7c90149"
            )),
            Calldata(Arc::new(vec![
                stark_felt!("0x1"),
                stark_felt!("0x67c358ec1181fc1e19daeebae1029cb478bb71917a5659a83a361c012fe3b6b"),
                stark_felt!("0x2f0b3c5710379609eb5495f1ecd348cb28167711b73609fe565a72734550354"),
                stark_felt!("0x0"),
                stark_felt!("0x1"),
                stark_felt!("0x1"),
                stark_felt!("0x0"),
            ])),
            Nonce(stark_felt!("0x7")),
            Fee(0x19fa39aa99000),
            ChainId("SN_MAIN".to_string()),
        );

        assert_eq!(
            tx_hash,
            stark_felt!("0x752b21e30667d124ff100ebfcbc3c2cd822c944dbcae49d6631cc478d74a1b7")
        );
    }

    #[test]
    fn test_compute_deploy_account_transaction_hash() {
        let tx_hash = compute_deploy_account_transaction_hash(
            ContractAddress(patricia_key!(
                "0x4039bc67e2bbd3c903c341089bfea3c836785500bfb22ba75d310f872957d0e"
            )),
            Calldata(Arc::new(vec![
                stark_felt!("0x33434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2"),
                stark_felt!("0x79dc0da7c54b95f10aa182ad0a46400db63156920adb65eca2654c0945a463"),
                stark_felt!("0x2"),
                stark_felt!("0x665014077991a930f7b22986fea5543e8c27f01043baa40b79a26d462e7c8fc"),
                stark_felt!("0x0"),
            ])),
            ClassHash(stark_felt!(
                "0x25ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918"
            )),
            ContractAddressSalt(stark_felt!(
                "0x665014077991a930f7b22986fea5543e8c27f01043baa40b79a26d462e7c8fc"
            )),
            Nonce(stark_felt!("0x0")),
            Fee(0x1dca7bc64fdb8),
            ChainId("SN_MAIN".to_string()),
        );

        assert_eq!(
            tx_hash,
            stark_felt!("0x485343cf6e2ec156cbefdaa7375de38e265547fea23c320e47c9ab9f16ef654")
        );
    }
}
