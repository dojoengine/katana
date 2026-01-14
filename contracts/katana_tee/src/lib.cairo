mod katana_report_utils;
use starknet::ContractAddress;

/// Interface for the Katana TEE contract.
#[starknet::interface]
pub trait IKatanaTee<TContractState> {
    /// Verify an SP1 proof by calling the AMD TEE Registry contract.
    /// Returns the public inputs if verification succeeds.
    fn verify_sp1_proof(self: @TContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>>;

    /// Get the AMD TEE Registry contract address.
    fn get_registry_address(self: @TContractState) -> ContractAddress;
}

/// Katana TEE contract that delegates SP1 proof verification to the AMD TEE Registry.
#[starknet::contract]
pub mod KatanaTee {
    use amd_tee_registry::tee_registry::{IAMDTeeRegistryDispatcher, IAMDTeeRegistryDispatcherTrait};
    use starknet::ContractAddress;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        /// Address of the AMD TEE Registry contract
        registry_address: ContractAddress,
    }

    #[constructor]
    fn constructor(ref self: ContractState, registry_address: ContractAddress) {
        self.registry_address.write(registry_address);
    }

    #[abi(embed_v0)]
    impl KatanaTeeImpl of super::IKatanaTee<ContractState> {
        /// Verify an SP1 proof by forwarding to the AMD TEE Registry.
        fn verify_sp1_proof(self: @ContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>> {
            let registry = IAMDTeeRegistryDispatcher {
                contract_address: self.registry_address.read(),
            };
            registry.verify_sp1_proof(sp1_proof)
        }

        /// Get the AMD TEE Registry contract address.
        fn get_registry_address(self: @ContractState) -> ContractAddress {
            self.registry_address.read()
        }
    }
}
