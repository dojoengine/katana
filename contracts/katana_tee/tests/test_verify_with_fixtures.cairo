//! Integration tests for KatanaTee proof verification using fixture files.
//!
//! These tests use fork testing against Starknet mainnet/sepolia to access
//! the Garaga SP1 verifier contract.

use snforge_std::fs::{FileTrait, read_txt};
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;
use amd_tee_registry::tee_registry::AMDTEERegistry;
use katana_tee::{IKatanaTeeDispatcher, IKatanaTeeDispatcherTrait};

/// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet and sepolia)
const GARAGA_CLASS_HASH: felt252 = 0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

/// SP1 program ID for the AMD attestation verifier
const SP1_PROGRAM_ID_LOW: felt252 = 0x2c621bae91a0626796ce637f01c928d8;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x00d2342d2400bed28302507269281dcb;

/// Max time difference for attestation validation (1 year for testing with old fixtures)
const MAX_TIME_DIFF: u64 = 31536000;

/// Genoa ARK root cert hash (from tests/fixtures/root_certs.json)
const GENOA_ROOT_CERT_LOW: felt252 = 0x5bfe1d8f800cea2cf270c10d103db2f1;
const GENOA_ROOT_CERT_HIGH: felt252 = 0x4c6598d19c18719c5dfd4a7d335f674e;

/// ASK intermediate cert hash (from proof fixtures, certs[1])
const ASK_CERT_LOW: felt252 = 0xc4bb797cd2c97a63be3ec075136b6a5f;
const ASK_CERT_HIGH: felt252 = 0xd105403760701f8fee86fee3215a27d9;

/// Deploy the AMDTEERegistry contract for testing
fn deploy_amd_registry() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    //              trusted_certs (array), processor_models (array), root_certs (array)
    // Note: processor_models and root_certs must have the same length
    let mut calldata: Array<felt252> = array![
        GARAGA_CLASS_HASH,
        SP1_PROGRAM_ID_LOW,
        SP1_PROGRAM_ID_HIGH,
        MAX_TIME_DIFF.into(),
        // trusted_certs array (ASK intermediate cert)
        1,  // length = 1
        ASK_CERT_LOW,
        ASK_CERT_HIGH,
        // processor_models array (Genoa = 1)
        1,  // length = 1
        1,  // ProcessorType::Genoa
        // root_certs array (Genoa root cert hash)
        1,  // length = 1
        GENOA_ROOT_CERT_LOW,
        GENOA_ROOT_CERT_HIGH,
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
#[ignore] // Run with: make test-fork (requires MAINNET_RPC_URL + fixtures)
#[fork("MAINNET")]
fn test_verify_block_0() {
    // Deploy contracts
    println!("Deploying AMDTEERegistry contract");
    let registry_address = deploy_amd_registry();
    println!("Deploying KatanaTee contract");
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    println!("Loading calldata");
    // Load calldata from fixture
    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_0/calldata.txt");

    // Verify proof returns public inputs
    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 0 verification failed');
}

/// Test verification of block 1 proof
#[test]
#[ignore] // Run with: make test-fork (requires MAINNET_RPC_URL + fixtures)
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
#[ignore] // Run with: make test-fork (requires MAINNET_RPC_URL + fixtures)
#[fork("MAINNET")]
fn test_verify_block_2() {
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_2/calldata.txt");

    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 2 verification failed');
}
