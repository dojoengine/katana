//! Integration tests for KatanaTee proof verification using fixture files.
//!
//! These tests use fork testing against Starknet mainnet/sepolia to access
//! the Garaga SP1 verifier contract.

use snforge_std::fs::{FileTrait, read_txt};
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;
use katana_tee::{IKatanaTeeDispatcher, IKatanaTeeDispatcherTrait};

/// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet and sepolia)
const GARAGA_CLASS_HASH: felt252 = 0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

/// SP1 program ID for the AMD attestation verifier
const SP1_PROGRAM_ID_LOW: felt252 = 0x2c621bae91a0626796ce637f01c928d8;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x00d2342d2400bed28302507269281dcb;

/// Max time difference for attestation validation (1 day)
const MAX_TIME_DIFF: u64 = 86400;

/// Deploy the AMDTEERegistry contract for testing
fn deploy_amd_registry() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    //              trusted_certs (array), processor_models (array), root_certs (array)
    let mut calldata: Array<felt252> = array![
        GARAGA_CLASS_HASH,
        SP1_PROGRAM_ID_LOW,
        SP1_PROGRAM_ID_HIGH,
        MAX_TIME_DIFF.into(),
        0,  // trusted_certs array length = 0
        2,  // processor_models array length = 2
        0,  // Milan = 0
        1,  // Genoa = 1
        0,  // root_certs array length = 0 (would need real hashes for full verification)
    ];

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Deploy the KatanaTee contract for testing
fn deploy_katana_tee(registry_address: ContractAddress) -> ContractAddress {
    let contract = declare("KatanaTee").unwrap().contract_class();

    let mut calldata: Array<felt252> = array![];
    calldata.append(registry_address.into());

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Load calldata from a fixture file
fn load_calldata_from_fixture(path: ByteArray) -> Array<felt252> {
    let file = FileTrait::new(path);
    read_txt(@file)
}

/// Test verification of block 0 proof
#[test]
#[ignore] // TODO: Enable after fixtures are generated and RPC access is configured
#[fork("MAINNET")]
fn test_verify_block_0() {
    // Deploy contracts
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    // Load calldata from fixture
    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_0/calldata.txt");

    // Verify proof returns public inputs
    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 0 verification failed');
}

/// Test verification of block 1 proof
#[test]
#[ignore] // TODO: Enable after fixtures are generated and RPC access is configured
#[fork("MAINNET")]
fn test_verify_block_1() {
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_1/calldata.txt");

    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 1 verification failed');
}

/// Test verification of block 2 proof
#[test]
#[ignore] // TODO: Enable after fixtures are generated and RPC access is configured
#[fork("MAINNET")]
fn test_verify_block_2() {
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_2/calldata.txt");

    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 2 verification failed');
}
