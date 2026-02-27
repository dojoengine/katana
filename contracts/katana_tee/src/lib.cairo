pub mod katana_report_utils;
use amd_tee_registry::tee_types::VerifierJournal;
use starknet::ContractAddress;

/// Interface for the Katana TEE contract.
#[starknet::interface]
pub trait IKatanaTee<TContractState> {
    /// Verify an SP1 proof by calling the AMD TEE Registry contract.
    /// Returns the public inputs if verification succeeds.
    fn verify_sp1_proof(
        self: @TContractState, sp1_proof: Array<felt252>,
    ) -> Result<VerifierJournal, felt252>;

    /// Verify proof and update the latest verified sequencer state.
    /// Also registers the storage commitment from the SP1 journal.
    /// Returns (success, end_block_number) where end_block_number comes from SP1 journal.
    fn verify_and_update_state(
        ref self: TContractState,
        sp1_proof: Array<felt252>,
        state_root: felt252,
        block_hash: felt252,
        block_number: u64,
        fork_block_number: u64,
        events_commitment: felt252,
    ) -> Result<(bool, u64), felt252>;

    /// Get the AMD TEE Registry contract address.
    fn get_registry_address(self: @TContractState) -> ContractAddress;

    /// Get the latest verified sequencer state.
    fn get_latest_state(self: @TContractState) -> (u64, felt252, felt252);
}

/// Katana TEE contract that delegates SP1 proof verification to the AMD TEE Registry.
#[starknet::contract]
pub mod KatanaTee {
    use amd_tee_registry::journal_decode::decode_verifier_journal;
    use amd_tee_registry::tee_registry::{IAMDTeeRegistryDispatcher, IAMDTeeRegistryDispatcherTrait};
    use amd_tee_registry::tee_types::{
        RawAttestationReport, RawAttestationReportTrait, VerifierJournal,
    };
    use starknet::ContractAddress;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use storage_commitment::{IStorageCommitmentDispatcher, IStorageCommitmentDispatcherTrait};
    use crate::katana_report_utils::verify_katana_report_data;

    #[storage]
    struct Storage {
        /// Address of the AMD TEE Registry contract
        registry_address: ContractAddress,
        /// Latest verified state root
        latest_state_root: felt252,
        /// Latest verified block hash
        latest_block_hash: felt252,
        /// Latest verified block number
        latest_block_number: u64,
        /// Storage commitment registry contract address
        storage_commitment_registry: ContractAddress,
    }


    #[constructor]
    fn constructor(
        ref self: ContractState,
        registry_address: ContractAddress,
        storage_commitment_registry: ContractAddress,
    ) {
        self.registry_address.write(registry_address);
        self.storage_commitment_registry.write(storage_commitment_registry);
    }

    #[abi(embed_v0)]
    impl KatanaTeeImpl of super::IKatanaTee<ContractState> {
        /// Verify an SP1 proof by forwarding to the AMD TEE Registry.
        fn verify_sp1_proof(
            self: @ContractState, sp1_proof: Array<felt252>,
        ) -> Result<VerifierJournal, felt252> {
            let registry = IAMDTeeRegistryDispatcher {
                contract_address: self.registry_address.read(),
            };
            registry.verify_sp1_proof(sp1_proof)
        }

        /// Verify proof, validate report data, and update the latest state.
        fn verify_and_update_state(
            ref self: ContractState,
            sp1_proof: Array<felt252>,
            state_root: felt252,
            block_hash: felt252,
            block_number: u64,
            fork_block_number: u64,
            events_commitment: felt252,
        ) -> Result<(bool, u64), felt252> {
            let registry = IAMDTeeRegistryDispatcher {
                contract_address: self.registry_address.read(),
            };
            match registry.verify_sp1_proof(sp1_proof) {
                Result::Ok(journal) => {
                    let raw_report = RawAttestationReport { raw: journal.raw_report };
                    let report_data = raw_report.report_data();
                    verify_katana_report_data(
                        report_data, state_root, block_hash, fork_block_number, events_commitment,
                    );

                    self.latest_state_root.write(state_root);
                    self.latest_block_hash.write(block_hash);
                    self.latest_block_number.write(block_number);

                    IStorageCommitmentDispatcher {
                        contract_address: self.storage_commitment_registry.read(),
                    }
                        .register_verified_commitment(journal.storage_commitment);

                    Result::Ok((true, journal.end_block_number))
                },
                Result::Err(error) => Result::Err(error),
            }
        }

        /// Get the AMD TEE Registry contract address.
        fn get_registry_address(self: @ContractState) -> ContractAddress {
            self.registry_address.read()
        }

        /// Get the latest verified sequencer state.
        fn get_latest_state(self: @ContractState) -> (u64, felt252, felt252) {
            (
                self.latest_block_number.read(),
                self.latest_state_root.read(),
                self.latest_block_hash.read(),
            )
        }
    }
}
