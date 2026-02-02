/// Interface for the Storage Commitment contract.
#[starknet::interface]
pub trait IStorageCommitment<TContractState> {
    /// Register a storage commitment that was verified by the TEE (SP1 proof).
    fn register_verified_commitment(ref self: TContractState, commitment: u256);

    /// Check if a commitment has already been registered
    fn is_verified(ref self: TContractState, commitment: u256) -> bool;
}

/// Storage Commitment contract that stores verified storage commitments.
#[starknet::contract]
pub mod StorageCommitment {
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};

    #[storage]
    struct Storage {
        commitments: Map<u256, bool> // true = commitment is registered
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        StorageCommitmentRegistered: StorageCommitmentRegisteredEvent,
    }

    #[derive(Drop, starknet::Event)]
    struct StorageCommitmentRegisteredEvent {
        commitment: u256,
    }

    #[abi(embed_v0)]
    impl StorageCommitmentImpl of super::IStorageCommitment<ContractState> {
        fn register_verified_commitment(ref self: ContractState, commitment: u256) {
            let already_registered = self.commitments.read(commitment);
            if !already_registered {
                self.commitments.write(commitment, true);
                self.emit(StorageCommitmentRegisteredEvent { commitment });
            }
        }

        fn is_verified(ref self: ContractState, commitment: u256) -> bool {
            self.commitments.read(commitment)
        }
    }
}
