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

/// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet and sepolia)
const GARAGA_CLASS_HASH: felt252 =
    0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

/// SP1 program ID for the AMD attestation verifier
const SP1_PROGRAM_ID_LOW: felt252 = 0xea077510823adf4b1255ada5d2977402;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x00613d956661ba71ff3d4d75fba28b79;

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
    let file = FileTrait::new("../../tests/fixtures/root_certs.json");
    FileParser::<RootCerts>::parse_json(@file).expect('Failed to parse root_certs.json')
}

/// Deploy the AMDTEERegistry contract in LIVE MODE (no pre-cached intermediates)
fn deploy_amd_registry_live_mode() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();
    let certs = load_root_certs();

    // Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    //              trusted_certs (array), processor_models (array), root_certs (array)
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
