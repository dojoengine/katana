/// Interface for the Storage Commitment contract.
///
/// Security model:
/// - Commitments are pre-computed hashes: hash(storage_commitment, contract_address, nonce, storage_state_root)
/// - Registration just stores the hash (from SP1 journal)
/// - Verification recomputes the hash using stored nonce and checks if it was registered
/// - After successful verification, commitment is deleted and nonce is incremented
/// - The nonce increment means the same proof can never produce a matching hash again
/// - Latest state_root is tracked to prevent using the same state twice
use starknet::ContractAddress;

/// Per-contract tracking info (nonce + latest state root)
#[derive(Drop, Copy, Serde, starknet::Store, Default)]
pub struct ContractInfo {
    /// Current nonce (incremented after each successful verification)
    pub nonce: u64,
    /// Latest state root (must change for new verification)
    pub latest_state_root: u256,
}

#[starknet::interface]
pub trait IStorageCommitment<TContractState> {
    /// Register a storage commitment hash that was verified by the TEE (SP1 proof).
    /// The commitment is already hash(storage_commitment, contract_address, nonce, storage_state_root).
    /// This function just stores it - no validation here.
    fn register_verified_commitment(ref self: TContractState, commitment: u256);

    /// Verify a commitment by recomputing the hash with the stored nonce.
    ///
    /// Flow:
    /// 1. Reads nonce from storage for contract_address
    /// 2. Computes: expected_hash = hash(storage_commitment, contract_address, nonce, storage_state_root)
    /// 3. Checks if expected_hash is registered
    /// 4. Checks if storage_state_root changed from last verification
    /// 5. If all checks pass: deletes commitment, increments nonce, updates latest_state_root
    ///
    /// Returns true if verified successfully.
    fn verify(
        ref self: TContractState,
        storage_commitment: u256,
        contract_address: ContractAddress,
        storage_state_root: u256,
    ) -> bool;

    fn is_registered(self: @TContractState, commitment: u256) -> bool;

    fn get_nonce(self: @TContractState, contract_address: ContractAddress) -> u64;

    fn get_latest_state_root(self: @TContractState, contract_address: ContractAddress) -> u256;
}

/// Storage Commitment contract that stores verified storage commitments.
#[starknet::contract]
pub mod StorageCommitment {
    use starknet::ContractAddress;
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};
    use core::poseidon::poseidon_hash_span;
    use super::ContractInfo;

    type Commitment = u256;
    type Registered = bool;

    #[storage]
    struct Storage {
        commitments: Map<Commitment, Registered>,
        contract_infos: Map<ContractAddress, ContractInfo>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        CommitmentRegistered: CommitmentRegisteredEvent,
        CommitmentVerified: CommitmentVerifiedEvent,
    }

    #[derive(Drop, starknet::Event)]
    struct CommitmentRegisteredEvent {
        #[key]
        commitment: u256,
    }

    #[derive(Drop, starknet::Event)]
    struct CommitmentVerifiedEvent {
        #[key]
        commitment: u256,
        #[key]
        contract_address: ContractAddress,
        nonce: u64,
        storage_state_root: u256,
    }

    #[abi(embed_v0)]
    impl StorageCommitmentImpl of super::IStorageCommitment<ContractState> {
        fn register_verified_commitment(ref self: ContractState, commitment: u256) {
            // Only register if not already registered
            if !self.commitments.read(commitment) {
                self.commitments.write(commitment, true);
                self.emit(CommitmentRegisteredEvent { commitment });
            }
        }

        fn verify(
            ref self: ContractState,
            storage_commitment: u256,
            contract_address: ContractAddress,
            storage_state_root: u256,
        ) -> bool {
            let info = self.contract_infos.read(contract_address);

            let expected_commitment = InternalImpl::compute_commitment(
                storage_commitment, contract_address, info.nonce, storage_state_root,
            );

            if !self.commitments.read(expected_commitment) {
                return false; // Not registered
            }

            if info.nonce > 0 && storage_state_root == info.latest_state_root {
                return false; // State root unchanged
            }

            self.commitments.write(expected_commitment, false);

            self.contract_infos.write(
                contract_address,
                ContractInfo { nonce: info.nonce + 1, latest_state_root: storage_state_root },
            );

            self.emit(CommitmentVerifiedEvent {
                commitment: expected_commitment,
                contract_address,
                nonce: info.nonce,
                storage_state_root,
            });

            true
        }

        fn is_registered(self: @ContractState, commitment: u256) -> bool {
            self.commitments.read(commitment)
        }

        fn get_nonce(self: @ContractState, contract_address: ContractAddress) -> u64 {
            self.contract_infos.read(contract_address).nonce
        }

        fn get_latest_state_root(self: @ContractState, contract_address: ContractAddress) -> u256 {
            self.contract_infos.read(contract_address).latest_state_root
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        fn compute_commitment(
            storage_commitment: u256,
            contract_address: ContractAddress,
            nonce: u64,
            storage_state_root: u256,
        ) -> u256 {
            let mut data: Array<felt252> = ArrayTrait::new();

            data.append(storage_commitment.low.into());
            data.append(storage_commitment.high.into());

            data.append(contract_address.into());
            data.append(nonce.into());

            data.append(storage_state_root.low.into());
            data.append(storage_state_root.high.into());

            let hash: felt252 = poseidon_hash_span(data.span());

            hash.into()
        }
    }
}
