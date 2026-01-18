use amd_tee_registry::tee_registry::AMDTEERegistry;
use amd_tee_registry::tee_types::ProcessorType;
use katana_tee::KatanaTee;
use sncast_std::{DeclareResultTrait, FeeSettingsTrait, call, declare, deploy, get_nonce, invoke};
use starknet::{ClassHash, ContractAddress};


// // The example below uses a contract deployed to the Sepolia testnet
// const CONTRACT_ADDRESS: felt252 =
//     0x07e867f1fa6da2108dd2b3d534f1fbec411c5ec9504eb3baa1e49c7a0bef5ab5;

const SALT: felt252 = 0x1;

#[executable]
fn main() {
    // trusted_certs: Array<u256>,
    // processor_models: Array<ProcessorType>,
    // root_certs: Array<u256>,
    let mut amd_tee_calldata: Array<felt252> = array![];
    let trusted_certs: Array<u256> = array![];
    let processor_models: Array<ProcessorType> = array![];
    let root_certs: Array<u256> = array![];
    Serde::serialize(@trusted_certs, ref amd_tee_calldata);
    Serde::serialize(@processor_models, ref amd_tee_calldata);
    Serde::serialize(@root_certs, ref amd_tee_calldata);

    let amd_tee_registry_address = declare_and_deploy_contract("AMDTEERegistry", amd_tee_calldata);

    let mut katana_tee_calldata: Array<felt252> = array![];
    Serde::serialize(@amd_tee_registry_address, ref katana_tee_calldata);
    let katana_tee_address = declare_and_deploy_contract("KatanaTee", katana_tee_calldata);
}


pub fn declare_contract(contract_name: ByteArray) -> ClassHash {
    let declare_result = declare(
        contract_name.clone(), FeeSettingsTrait::estimate(), Option::Some(get_nonce('latest')),
    );

    let class_hash = *match (declare_result) {
        Result::Ok(ok_result) => ok_result.class_hash(),
        Result::Err(err_result) => { panic!("{:?}", err_result); },
    };
    println!("[{:}] Class hash 0x{:x}", contract_name, class_hash);
    class_hash
}


pub fn deploy_contract(class_hash: ClassHash, constructor_calldata: Array::<felt252>) -> ContractAddress {
    let deploy_result = deploy(
        class_hash,
        constructor_calldata,
        Option::Some(SALT),
        true,
        FeeSettingsTrait::estimate(),
        Option::None,
    );
    let contract_address = match deploy_result {
        Result::Ok(ok_result) => ok_result.contract_address,
        Result::Err(err_result) => { panic!("{:?}", err_result); },
    };
    println!("Contract address: 0x{:x}", contract_address);
    contract_address
}


pub fn declare_and_deploy_contract(contract_name: ByteArray, constructor_calldata: Array::<felt252>) -> ContractAddress {
    let class_hash = declare_contract(contract_name);
    let contract_address = deploy_contract(class_hash, constructor_calldata);
    contract_address
}