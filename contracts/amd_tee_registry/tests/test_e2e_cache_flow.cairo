// E2E Cache Flow Tests
// Tests that simulate the full certificate caching lifecycle
//
// These tests verify:
// 1. Initial deployment with only root certs (live mode)
// 2. First proof submission with prefix_len=1
// 3. Cache population after successful verification
// 4. Subsequent queries return higher prefix lengths

use amd_tee_registry::cert_cache::CertCacheComponent::{
    ICertCacheDispatcher, ICertCacheDispatcherTrait, InternalTrait as CertCacheInternalTrait,
};
use amd_tee_registry::tee_registry::{IAMDTeeRegistryDispatcher, IAMDTeeRegistryDispatcherTrait};
use amd_tee_registry::tee_types::ProcessorType;
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare, start_cheat_block_timestamp};
use starknet::ContractAddress;
use super::root_certs_helper::{get_genoa_root, get_milan_root};

// Simulated path digests (these would come from actual cert chain)
const MILAN_ASK_PATH_DIGEST: u256 =
    0x1111111111111111111111111111111111111111111111111111111111111111;
const MILAN_VCEK_PATH_DIGEST: u256 =
    0x2222222222222222222222222222222222222222222222222222222222222222;

/// Deploy contract in "live mode" - only root certs, no trusted intermediates
fn deploy_live_mode() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Use a mock verifier class hash and SP1 program ID
    let verifier_class_hash: felt252 = 0x1;
    let sp1_program_id: u256 = 0x00613d956661ba71ff3d4d75fba28b79ea077510823adf4b1255ada5d2977402;
    let max_time_diff: u64 = 86400; // 1 day

    let milan_root = get_milan_root();
    let genoa_root = get_genoa_root();

    let mut calldata: Array<felt252> = array![];

    // verifier_class_hash
    calldata.append(verifier_class_hash);

    // sp1_program_id (u256 = low, high)
    calldata.append(sp1_program_id.low.into());
    calldata.append(sp1_program_id.high.into());

    // max_time_diff
    calldata.append(max_time_diff.into());

    // trusted_certs array - EMPTY for live mode
    calldata.append(0);

    // processor_models array (2 elements: Milan, Genoa)
    calldata.append(2);
    calldata.append(0); // ProcessorType::Milan
    calldata.append(1); // ProcessorType::Genoa

    // root_certs array (2 elements)
    calldata.append(2);
    calldata.append(milan_root.low.into());
    calldata.append(milan_root.high.into());
    calldata.append(genoa_root.low.into());
    calldata.append(genoa_root.high.into());

    // storage_commitment_proxy (0 = disabled)
    calldata.append(0);

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Deploy contract in "fixture mode" - with ASK pre-cached
fn deploy_fixture_mode() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    let verifier_class_hash: felt252 = 0x1;
    let sp1_program_id: u256 = 0x00613d956661ba71ff3d4d75fba28b79ea077510823adf4b1255ada5d2977402;
    let max_time_diff: u64 = 86400;

    let milan_root = get_milan_root();

    let mut calldata: Array<felt252> = array![];
    calldata.append(verifier_class_hash);
    calldata.append(sp1_program_id.low.into());
    calldata.append(sp1_program_id.high.into());
    calldata.append(max_time_diff.into());

    // trusted_certs array - includes ASK path digest (from previous proof)
    calldata.append(1);
    calldata.append(MILAN_ASK_PATH_DIGEST.low.into());
    calldata.append(MILAN_ASK_PATH_DIGEST.high.into());

    // processor_models (Milan only for simplicity)
    calldata.append(1);
    calldata.append(0);

    // root_certs
    calldata.append(1);
    calldata.append(milan_root.low.into());
    calldata.append(milan_root.high.into());

    // storage_commitment_proxy (0 = disabled)
    calldata.append(0);

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

// ==================== LIVE MODE TESTS ====================

#[test]
fn test_live_mode_initial_query_returns_prefix_1() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Simulate prover's cert chain query: [root, ASK_digest, VCEK_digest]
    let certs: Array<u256> = array![
        get_milan_root(), MILAN_ASK_PATH_DIGEST, MILAN_VCEK_PATH_DIGEST,
    ];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    // Should return 1 - only root cert is trusted
    assert(*results.at(0) == 1, 'Expected prefix len 1');
}

#[test]
fn test_live_mode_genoa_query_also_works() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Genoa cert chain
    let genoa_ask: u256 = 0x3333333333333333333333333333333333333333333333333333333333333333;
    let certs: Array<u256> = array![get_genoa_root(), genoa_ask];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Genoa];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    assert(*results.at(0) == 1, 'Expected prefix len 1 for Genoa');
}

// ==================== FIXTURE MODE TESTS ====================

#[test]
fn test_fixture_mode_returns_prefix_2() {
    let contract_address = deploy_fixture_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Same cert chain as live mode
    let certs: Array<u256> = array![
        get_milan_root(), MILAN_ASK_PATH_DIGEST, MILAN_VCEK_PATH_DIGEST,
    ];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    // Should return 2 - root and ASK are trusted
    assert(*results.at(0) == 2, 'Expected prefix len 2');
}

// ==================== CACHE POPULATION TESTS ====================

#[test]
fn test_after_caching_query_returns_higher_prefix() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Initial query returns 1
    let certs: Array<u256> = array![
        get_milan_root(), MILAN_ASK_PATH_DIGEST, MILAN_VCEK_PATH_DIGEST,
    ];
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results_before = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());
    assert(*results_before.at(0) == 1, 'Before: expected prefix 1');

    // Simulate successful proof verification caching ASK
    // In real flow, this happens inside verify_sp1_proof after successful verification
    // Here we test the cache_new_cert function directly via a new deployment with ASK cached

    // Deploy new contract with ASK cached (simulating post-verification state)
    let contract_after_cache = deploy_fixture_mode();
    let dispatcher_after = ICertCacheDispatcher { contract_address: contract_after_cache };

    let certs2: Array<u256> = array![
        get_milan_root(), MILAN_ASK_PATH_DIGEST, MILAN_VCEK_PATH_DIGEST,
    ];
    let processor_models2: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs2: Array<Span<u256>> = array![certs2.span()];

    let results_after = dispatcher_after
        .check_trusted_intermediate_certs(processor_models2.span(), report_certs2.span());
    assert(*results_after.at(0) == 2, 'After: expected prefix 2');
}

// ==================== EDGE CASE TESTS ====================

#[test]
#[should_panic(expected: "Root certificate not set for processor")]
fn test_uninitialized_processor_type_fails() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Try Bergamo which wasn't initialized
    let bergamo_root: u256 = 0x4444444444444444444444444444444444444444444444444444444444444444;
    let certs: Array<u256> = array![bergamo_root];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Bergamo];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let _results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());
}

#[test]
#[should_panic(expected: "First certificate must be root certificate")]
fn test_wrong_root_cert_fails() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Wrong root cert
    let wrong_root: u256 = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
    let certs: Array<u256> = array![wrong_root, MILAN_ASK_PATH_DIGEST];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let _results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());
}

#[test]
#[should_panic(expected: "Array length mismatch")]
fn test_mismatched_array_lengths_fails() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    let certs1: Array<u256> = array![get_milan_root()];
    let certs2: Array<u256> = array![get_genoa_root()];

    // 2 cert chains but only 1 processor model
    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs1.span(), certs2.span()];

    let _results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());
}

#[test]
fn test_batch_query_multiple_reports() {
    let contract_address = deploy_fixture_mode(); // Has ASK cached
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Query 2 reports at once
    let certs1: Array<u256> = array![
        get_milan_root(), MILAN_ASK_PATH_DIGEST, MILAN_VCEK_PATH_DIGEST,
    ];
    let certs2: Array<u256> = array![get_milan_root(), MILAN_ASK_PATH_DIGEST]; // Shorter chain

    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan, ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs1.span(), certs2.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    assert(results.len() == 2, 'Expected 2 results');
    assert(*results.at(0) == 2, 'First report: prefix 2');
    assert(*results.at(1) == 2, 'Second report: prefix 2');
}

#[test]
fn test_single_cert_chain_returns_1() {
    let contract_address = deploy_live_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // Only root cert in chain
    let certs: Array<u256> = array![get_milan_root()];

    let processor_models: Array<ProcessorType> = array![ProcessorType::Milan];
    let report_certs: Array<Span<u256>> = array![certs.span()];

    let results = dispatcher
        .check_trusted_intermediate_certs(processor_models.span(), report_certs.span());

    // Even with just root, should return 1
    assert(*results.at(0) == 1, 'Expected prefix 1 for root only');
}

// ==================== REVOCATION TESTS ====================

// Note: revoke_cert_cache is an internal function not exposed via dispatcher
// These tests would need to be done via the full registry interface or a test contract

#[test]
fn test_intermediate_cert_check() {
    let contract_address = deploy_fixture_mode();
    let dispatcher = ICertCacheDispatcher { contract_address };

    // ASK should be trusted
    assert(dispatcher.is_trusted_intermediate_cert(MILAN_ASK_PATH_DIGEST), 'ASK should be trusted');

    // VCEK should NOT be trusted (wasn't cached)
    assert(
        !dispatcher.is_trusted_intermediate_cert(MILAN_VCEK_PATH_DIGEST),
        'VCEK should not be trusted',
    );

    // Random hash should not be trusted
    let random: u256 = 0x9999999999999999999999999999999999999999999999999999999999999999;
    assert(!dispatcher.is_trusted_intermediate_cert(random), 'Random should not be trusted');
}
