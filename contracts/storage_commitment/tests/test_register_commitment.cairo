use core::poseidon::poseidon_hash_span;
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::{ContractAddress, contract_address_const};
use storage_commitment::{IStorageCommitmentDispatcher, IStorageCommitmentDispatcherTrait};

fn deploy_storage_commitment() -> ContractAddress {
    let contract_class = declare("StorageCommitment").unwrap().contract_class();
    let calldata: Array<felt252> = array![];
    let (contract_address, _) = contract_class.deploy(@calldata).unwrap();
    contract_address
}

fn test_contract_address() -> ContractAddress {
    contract_address_const::<0x123>()
}

/// Compute commitment hash the same way as the contract does
/// This simulates what SP1 verifier produces
fn compute_commitment(
    storage_commitment: felt252,
    contract_address: ContractAddress,
    nonce: u64,
    storage_state_root: felt252,
    end_block_number: u64,
) -> felt252 {
    let mut data: Array<felt252> = ArrayTrait::new();

    data.append(storage_commitment);
    data.append(contract_address.into());
    data.append(nonce.into());
    data.append(storage_state_root);
    data.append(end_block_number.into());

    poseidon_hash_span(data.span())
}

#[test]
fn test_register_commitment() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let commitment: felt252 = 0x111222;

    // Before registration
    assert(!dispatcher.is_registered(commitment), 'Should not be registered');

    // Register
    dispatcher.register_verified_commitment(commitment);

    // After registration
    assert(dispatcher.is_registered(commitment), 'Should be registered');
}

#[test]
fn test_register_idempotent() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let commitment: felt252 = 0x111222;

    // Register twice - should not fail
    dispatcher.register_verified_commitment(commitment);
    dispatcher.register_verified_commitment(commitment);

    assert(dispatcher.is_registered(commitment), 'Should still be registered');
}

#[test]
fn test_verify_flow() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let storage_contract = test_contract_address();
    let storage_commitment: felt252 = 0xaaabbb;
    let storage_state_root: felt252 = 0x100200;

    // Initial nonce should be 0
    assert(dispatcher.get_nonce(storage_contract) == 0, 'Initial nonce should be 0');

    // Compute the commitment hash (simulating SP1 verifier output)
    let commitment_hash = compute_commitment(
        storage_commitment, storage_contract, 0, // nonce = 0
        storage_state_root, 0,
    );

    // Register the commitment (simulating what KatanaTee does)
    dispatcher.register_verified_commitment(commitment_hash);
    assert(dispatcher.is_registered(commitment_hash), 'Should be registered');

    // Verify (simulating what sharding contract does)
    let verified = dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);
    assert(verified, 'Should verify successfully');

    // After verification:
    // - Nonce should be incremented
    assert(dispatcher.get_nonce(storage_contract) == 1, 'Nonce should be 1');
    // - Latest state root should be updated
    assert(
        dispatcher.get_latest_global_state_root(storage_contract) == storage_state_root,
        'State root should be updated',
    );
    // - Commitment should be deleted (not registered anymore)
    assert(!dispatcher.is_registered(commitment_hash), 'Should not be registered');
}

#[test]
#[should_panic(expected: 'Commitment not registered')]
fn test_verify_fails_if_not_registered() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let storage_contract = test_contract_address();
    let storage_commitment: felt252 = 0xaaabbb;
    let storage_state_root: felt252 = 0x100200;

    // Try to verify without registering first - should panic
    dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);
}

#[test]
#[should_panic(expected: 'Commitment not registered')]
fn test_replay_protection_nonce() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let storage_contract = test_contract_address();
    let storage_commitment: felt252 = 0xaaabbb;
    let storage_state_root: felt252 = 0x100200;

    // First commitment with nonce=0
    let commitment_hash_0 = compute_commitment(
        storage_commitment, storage_contract, 0, storage_state_root, 0,
    );
    dispatcher.register_verified_commitment(commitment_hash_0);

    // Verify first time - should succeed
    let verified1 = dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);
    assert(verified1, 'First verify should succeed');
    assert(dispatcher.get_nonce(storage_contract) == 1, 'Nonce should be 1');

    // Try to verify again with same inputs - should panic because:
    // 1. The contract now computes hash with nonce=1 (different hash)
    // 2. That hash is not registered
    dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);
}

#[test]
#[should_panic(expected: 'State root unchanged')]
fn test_state_root_must_change() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let storage_contract = test_contract_address();
    let storage_commitment1: felt252 = 0xaaabbb;
    let storage_commitment2: felt252 = 0xcccddd;
    let storage_state_root: felt252 = 0x100200;

    // First commitment with nonce=0
    let commitment_hash_0 = compute_commitment(
        storage_commitment1, storage_contract, 0, storage_state_root, 0,
    );
    dispatcher.register_verified_commitment(commitment_hash_0);
    let verified1 = dispatcher.verify(storage_commitment1, storage_contract, storage_state_root, 0);
    assert(verified1, 'First verify should succeed');

    // Second commitment with nonce=1 but SAME state_root
    let commitment_hash_1 = compute_commitment(
        storage_commitment2, storage_contract, 1, storage_state_root, 0 // same state_root!
    );
    dispatcher.register_verified_commitment(commitment_hash_1);

    // Should panic because state_root unchanged
    dispatcher.verify(storage_commitment2, storage_contract, storage_state_root, 0);
}

#[test]
fn test_multiple_contracts_independent() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let contract_a = contract_address_const::<0xA>();
    let contract_b = contract_address_const::<0xB>();
    let storage_commitment: felt252 = 0xaaabbb;
    let state_root_a: felt252 = 0x100100;
    let state_root_b: felt252 = 0x200200;

    // Both start with nonce 0
    assert(dispatcher.get_nonce(contract_a) == 0, 'A nonce should be 0');
    assert(dispatcher.get_nonce(contract_b) == 0, 'B nonce should be 0');

    // Register and verify for contract A
    let commitment_a = compute_commitment(storage_commitment, contract_a, 0, state_root_a, 0);
    dispatcher.register_verified_commitment(commitment_a);
    let verified_a = dispatcher.verify(storage_commitment, contract_a, state_root_a, 0);
    assert(verified_a, 'A should verify');

    // A's nonce should be 1, B's still 0
    assert(dispatcher.get_nonce(contract_a) == 1, 'A nonce should be 1');
    assert(dispatcher.get_nonce(contract_b) == 0, 'B nonce should still be 0');

    // Register and verify for contract B (uses nonce 0)
    let commitment_b = compute_commitment(storage_commitment, contract_b, 0, state_root_b, 0);
    dispatcher.register_verified_commitment(commitment_b);
    let verified_b = dispatcher.verify(storage_commitment, contract_b, state_root_b, 0);
    assert(verified_b, 'B should verify');

    // Both should have their own nonces
    assert(dispatcher.get_nonce(contract_a) == 1, 'A nonce should still be 1');
    assert(dispatcher.get_nonce(contract_b) == 1, 'B nonce should be 1');
}

#[test]
#[should_panic(expected: 'Commitment not registered')]
fn test_cannot_reuse_deleted_commitment() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let storage_contract = test_contract_address();
    let storage_commitment: felt252 = 0xaaabbb;
    let storage_state_root: felt252 = 0x100200;

    // Register and verify
    let commitment_hash = compute_commitment(
        storage_commitment, storage_contract, 0, storage_state_root, 0,
    );
    dispatcher.register_verified_commitment(commitment_hash);
    dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);

    // Commitment should be deleted
    assert(!dispatcher.is_registered(commitment_hash), 'Should be deleted');

    // Try to re-register the same commitment hash (attacker re-verifies SP1 proof)
    dispatcher.register_verified_commitment(commitment_hash);

    // Commitment is registered again
    assert(dispatcher.is_registered(commitment_hash), 'Should be re-registered');

    // BUT verify will panic because nonce is now 1
    // Contract computes hash with nonce=1, which is different from commitment_hash
    dispatcher.verify(storage_commitment, storage_contract, storage_state_root, 0);
}
