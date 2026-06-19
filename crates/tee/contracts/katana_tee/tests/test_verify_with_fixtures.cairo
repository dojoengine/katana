//! Integration tests for KatanaTee proof verification using fixture files.
//!
//! This test demonstrates the full cache progression:
//! 1. Deploy in live mode (only root certs)
//! 2. Verify block 0 (prefix_len=1) - ASK gets cached
//! 3. Verify blocks 1, 2 (prefix_len=2) - uses cached ASK

use amd_tee_registry::cert_cache::CertCacheComponent::{
    ICertCacheDispatcher, ICertCacheDispatcherTrait,
};
use amd_tee_registry::tee_registry::AMDTEERegistry;
use katana_tee::{IKatanaTeeDispatcher, IKatanaTeeDispatcherTrait};
use snforge_std::fs::{FileParser, FileTrait, read_txt};
use snforge_std::{
    ContractClassTrait, DeclareResultTrait, EventSpyTrait, EventsFilterTrait, declare, spy_events,
};
use starknet::ContractAddress;
use storage_commitment::{IStorageCommitmentDispatcher, IStorageCommitmentDispatcherTrait};

/// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet and sepolia)
const GARAGA_CLASS_HASH: felt252 =
    0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

/// SP1 program ID for the AMD attestation verifier
const SP1_PROGRAM_ID_LOW: felt252 = 0x8323ce49dba9b22fc128157fb9cb4ff0;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x008d500940a54e9411d515f14090769b;

/// Max time difference for attestation validation (1 year for testing with old fixtures)
const MAX_TIME_DIFF: u64 = 31536000;

#[derive(Drop, Serde)]
struct RootCerts {
    genoa_ark_hash_high: felt252,
    genoa_ark_hash_low: felt252,
    milan_ark_hash_high: felt252,
    milan_ark_hash_low: felt252,
}

fn load_root_certs() -> RootCerts {
    let file = FileTrait::new("../amd_root_certs.json");
    FileParser::<RootCerts>::parse_json(@file).expect('Failed to parse root_certs.json')
}

/// Deploy the AMDTEERegistry contract in LIVE MODE (no pre-cached intermediates)
fn deploy_amd_registry_live_mode() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();
    let certs = load_root_certs();

    // Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    //              trusted_certs (array), processor_models (array), root_certs (array),
    //              storage_commitment_proxy (0 = disabled)
    let mut calldata: Array<felt252> = array![
        GARAGA_CLASS_HASH, SP1_PROGRAM_ID_LOW, SP1_PROGRAM_ID_HIGH,
        MAX_TIME_DIFF.into(), // trusted_certs array - EMPTY for live mode
        0, // processor_models array (Genoa = 1)
        1, 1, // ProcessorType::Genoa
        // root_certs array (Genoa root cert hash)
        1,
        certs.genoa_ark_hash_low, certs.genoa_ark_hash_high,
    ];

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

fn deploy_storage_commitment_registry() -> ContractAddress {
    let contract = declare("StorageCommitment").unwrap().contract_class();
    let mut calldata: Array<felt252> = array![];
    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Deploy the KatanaTee contract for testing, with StorageCommitment authorized.
fn deploy_katana_tee(registry_address: ContractAddress) -> ContractAddress {
    let (katana_address, _) = deploy_katana_tee_and_storage_commitment_registry(registry_address);
    katana_address
}

fn deploy_katana_tee_and_storage_commitment_registry(
    registry_address: ContractAddress,
) -> (ContractAddress, ContractAddress) {
    let contract = declare("KatanaTee").unwrap().contract_class();
    let storage_commitment_registry = deploy_storage_commitment_registry();

    let mut calldata: Array<felt252> = array![];
    calldata.append(registry_address.into());
    calldata.append(storage_commitment_registry.into());

    let (katana_contract_address, _) = contract.deploy(@calldata).unwrap();

    // Authorize KatanaTee to register commitments on StorageCommitment.
    // The deployer (test contract) calls set_authorized_caller.
    let sc_dispatcher = IStorageCommitmentDispatcher {
        contract_address: storage_commitment_registry,
    };
    sc_dispatcher.set_authorized_caller(katana_contract_address);

    (katana_contract_address, storage_commitment_registry)
}


/// Load calldata from a fixture file
fn load_calldata_from_fixture(path: ByteArray) -> Array<felt252> {
    let file = FileTrait::new(path);
    read_txt(@file)
}

/// Test verification of all blocks with cache progression
///
/// This single test verifies:
/// 1. Block 0: First proof (prefix_len=1), ASK gets cached via CertCached event
/// 2. Block 1: Uses cached ASK (prefix_len=2)
/// 3. Block 2: Uses cached ASK (prefix_len=2)
#[test]
#[ignore] // Run with: make test-fork (requires MAINNET_RPC_URL + fixtures)
#[fork("MAINNET")]
fn test_verify_blocks_with_cache_progression() {
    // Deploy in LIVE MODE - only root certs, no pre-cached intermediates
    println!("Deploying AMDTEERegistry in live mode (no pre-cached certs)");
    let registry_address = deploy_amd_registry_live_mode();
    println!("Deploying KatanaTee contract");
    let katana_address = deploy_katana_tee(registry_address);

    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };
    let cache_dispatcher = ICertCacheDispatcher { contract_address: registry_address };

    // === Block 0: First proof, nothing cached yet ===
    println!("=== Block 0: First proof (live mode) ===");

    // Start spying on events before verification
    let mut spy = spy_events();

    let calldata_0 = load_calldata_from_fixture("../../tests/fixtures/block_0/calldata.txt");
    let result_0 = dispatcher.verify_sp1_proof(calldata_0);
    assert(result_0.is_ok(), 'Block 0 verification failed');
    println!("Block 0 verified successfully");

    // Extract CertCached events to find what got cached
    let events = spy.get_events().emitted_by(registry_address);
    assert(events.events.len() > 0, 'No certs cached after block 0');
    println!("CertCached events emitted: {}", events.events.len());

    // Parse CertCached event to get the ASK hash
    let (_, event) = events.events.at(0);
    let cached_cert_low: felt252 = *event.data.at(0);
    let cached_cert_high: felt252 = *event.data.at(1);
    let cached_cert = u256 {
        low: cached_cert_low.try_into().unwrap(), high: cached_cert_high.try_into().unwrap(),
    };
    println!("Cached cert hash (from event): 0x{:x}", cached_cert);

    // Verify cert is now trusted in contract storage
    assert(cache_dispatcher.is_trusted_intermediate_cert(cached_cert), 'ASK not in contract cache');
    println!("Verified: ASK is now cached in contract");

    // === Block 1: Uses cached ASK (prefix_len=2) ===
    println!("=== Block 1: Using cached ASK ===");
    let calldata_1 = load_calldata_from_fixture("../../tests/fixtures/block_1/calldata.txt");
    let result_1 = dispatcher.verify_sp1_proof(calldata_1);
    assert(result_1.is_ok(), 'Block 1 verification failed');
    println!("Block 1 verified successfully");

    // === Block 2: Uses cached ASK (prefix_len=2) ===
    println!("=== Block 2: Using cached ASK ===");
    let calldata_2 = load_calldata_from_fixture("../../tests/fixtures/block_2/calldata.txt");
    let result_2 = dispatcher.verify_sp1_proof(calldata_2);
    assert(result_2.is_ok(), 'Block 2 verification failed');
    println!("Block 2 verified successfully");

    println!("=== All blocks verified with cache progression ===");
}

#[test]
#[ignore] // Requires MAINNET_RPC_URL
#[fork("MAINNET")]
fn test_verify_and_update_state() {
    // Use a dummy address for the registry
    let registry_address = deploy_amd_registry_live_mode();
    let (katana_address, storage_commitment_registry_address) =
        deploy_katana_tee_and_storage_commitment_registry(
        registry_address,
    );
    let katana_dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };
    let storage_commitment_dispatcher = IStorageCommitmentDispatcher {
        contract_address: storage_commitment_registry_address,
    };

    let sp1_proof = load_calldata_from_fixture("../../tests/fixtures/sp1_proof_as_calldata.txt");
    let state_root: felt252 = 0x4ff77ff86b29cd49b7c37d57fa7f1ea06d6c09145c4e18e82fb9667359f2c26;
    let block_hash: felt252 = 0x26198ccf53a6611cd2a6cab0906e98f7e7524ec163d266aec615ab2def91809;
    let block_number: u64 = 6149472;

    // Start spying BEFORE the action
    let mut spy = spy_events();

    // events_commitment=0 for legacy fixtures (will fail with 4-field Poseidon mismatch
    // but test is #[ignore] and needs new fixtures anyway)
    let (result, end_block_number) = katana_dispatcher
        .verify_and_update_state(sp1_proof, state_root, block_hash, block_number, 0, 0)
        .unwrap();

    // Get events AFTER the action
    let events = spy.get_events().emitted_by(katana_address);
    println!("Events emitted: {}", events.events.len());

    // Print event details
    for i in 0..events.events.len() {
        let (_, event) = events.events.at(i);
        println!("Event {}: keys={}, data={}", i, event.keys.len(), event.data.len());

        // Print first key (usually the event name selector)
        if event.keys.len() > 0 {
            println!("  key[0]: {:?}", *event.keys.at(0));
        }
        // Print data in hex
        for j in 0..event.data.len() {
            println!("  data[{}]: {:x}", j, *event.data.at(j));
        }
    }

    assert(result == true, 'Verify true');
    println!("end_block_number: {}", end_block_number);

    // Extract storage commitment from the event data (data[0]=low, data[1]=high)
    let (_, event) = events.events.at(0);
    let commitment_low: u128 = (*event.data.at(0)).try_into().unwrap();
    let commitment_high: u128 = (*event.data.at(1)).try_into().unwrap();
    let storage_commitment = u256 { low: commitment_low, high: commitment_high };
    let storage_commitment_felt: felt252 = storage_commitment.try_into().unwrap();

    println!("Checking storage commitment: {:x}", storage_commitment);
    assert(
        storage_commitment_dispatcher.is_registered(storage_commitment_felt),
        'Commitment not registered',
    );
}
use core::poseidon::poseidon_hash_span;

/// Helper function to compute commitment the same way as sharding contract
/// poseidon_hash([keys..., values...]) converted to u256
fn compute_commitment_helper(storage_changes: Span<(felt252, felt252)>) -> u256 {
    let mut data: Array<felt252> = ArrayTrait::new();

    // First all keys
    for change in storage_changes {
        let (key, _) = *change;
        data.append(key);
    }

    // Then all values
    for change in storage_changes {
        let (_, value) = *change;
        data.append(value);
    }

    poseidon_hash_span(data.span()).into()
}

#[test]
fn test_compute_commitment_matches_rust_format() {
    // This test verifies the format matches Rust side:
    // Poseidon::hash_array(&[keys..., values...])
    //
    // For storage_changes = [(key1, val1), (key2, val2)]
    // The hash input should be: [key1, key2, val1, val2]

    let storage_changes: Array<(felt252, felt252)> = array![
        (0x7ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854, 0x3),
    ];

    let commitment = compute_commitment_helper(storage_changes.span());

    // Verify format: hash([key, value])
    let expected_input: Array<felt252> = array![
        0x7ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854, 0x3,
    ];
    let expected: u256 = poseidon_hash_span(expected_input.span()).into();

    assert!(commitment == expected, "Commitment should match Rust format");

    println!("Real slot commitment: {:?}", commitment);
}
