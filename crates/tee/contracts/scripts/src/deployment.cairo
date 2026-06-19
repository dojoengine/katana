use amd_tee_registry::tee_registry::AMDTEERegistry;
use amd_tee_registry::tee_types::ProcessorType;
use katana_tee::KatanaTee;
use sncast_std::{DeclareResultTrait, FeeSettingsTrait, declare, deploy, get_nonce};
use snforge_std::fs::{FileParser, FileTrait};
use starknet::{ClassHash, ContractAddress};

const SALT: felt252 = 0x1;

// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet)
const GARAGA_CLASS_HASH: felt252 =
    0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

// SP1 program ID
const SP1_PROGRAM_ID_LOW: felt252 = 0x8323ce49dba9b22fc128157fb9cb4ff0;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x008d500940a54e9411d515f14090769b;

#[derive(Drop, Serde)]
struct RootCerts {
    genoa_ark_hash_high: felt252,
    genoa_ark_hash_low: felt252,
    milan_ark_hash_high: felt252,
    milan_ark_hash_low: felt252,
}

#[executable]
fn main() {
    // Load root certs from fixture
    let file = FileTrait::new("../amd_root_certs.json");
    let certs: RootCerts = FileParser::<RootCerts>::parse_json(@file)
        .expect('Failed to load root_certs.json');

    let milan_root = u256 {
        low: certs.milan_ark_hash_low.try_into().unwrap(),
        high: certs.milan_ark_hash_high.try_into().unwrap(),
    };
    let genoa_root = u256 {
        low: certs.genoa_ark_hash_low.try_into().unwrap(),
        high: certs.genoa_ark_hash_high.try_into().unwrap(),
    };

    println!("Loaded root certs from fixture:");
    println!("  Milan: 0x{:x}", milan_root);
    println!("  Genoa: 0x{:x}", genoa_root);

    // Build calldata - LIVE MODE (empty trusted_certs)
    let verifier_class_hash: ClassHash = GARAGA_CLASS_HASH.try_into().unwrap();
    let sp1_program_id: u256 = u256 {
        low: SP1_PROGRAM_ID_LOW.try_into().unwrap(), high: SP1_PROGRAM_ID_HIGH.try_into().unwrap(),
    };
    let max_time_diff: u64 = 86400; // 24h (match deploy_sncast.sh)
    let trusted_certs: Array<u256> = array![]; // Live mode - empty
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan, ProcessorType::Genoa];
    let root_certs: Array<u256> = array![milan_root, genoa_root];

    let storage_commitment_proxy: ContractAddress = 0.try_into().unwrap();

    let mut amd_tee_calldata: Array<felt252> = array![];
    Serde::serialize(@verifier_class_hash, ref amd_tee_calldata);
    Serde::serialize(@sp1_program_id, ref amd_tee_calldata);
    Serde::serialize(@max_time_diff, ref amd_tee_calldata);
    Serde::serialize(@trusted_certs, ref amd_tee_calldata);
    Serde::serialize(@processor_models, ref amd_tee_calldata);
    Serde::serialize(@root_certs, ref amd_tee_calldata);
    Serde::serialize(@storage_commitment_proxy, ref amd_tee_calldata);

    let amd_tee_registry_address = declare_and_deploy_contract("AMDTEERegistry", amd_tee_calldata);

    // Deploy KatanaTee
    let mut katana_tee_calldata: Array<felt252> = array![];
    Serde::serialize(@amd_tee_registry_address, ref katana_tee_calldata);
    let _katana_tee_address = declare_and_deploy_contract("KatanaTee", katana_tee_calldata);
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


pub fn deploy_contract(
    class_hash: ClassHash, constructor_calldata: Array<felt252>,
) -> ContractAddress {
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


pub fn declare_and_deploy_contract(
    contract_name: ByteArray, constructor_calldata: Array<felt252>,
) -> ContractAddress {
    let class_hash = declare_contract(contract_name);
    let contract_address = deploy_contract(class_hash, constructor_calldata);
    contract_address
}
