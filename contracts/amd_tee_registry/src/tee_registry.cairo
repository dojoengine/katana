#[starknet::interface]
pub trait IAMDTeeRegistry<TContractState> {
    /// Verify a SP1 proof.
    fn verify_sp1_proof(ref self: TContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>>;
}

#[starknet::contract]
pub mod AMDTEERegistry {
    use starknet::ClassHash;
    use starknet::SyscallResultTrait;
    use starknet::get_block_timestamp;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::library_call_syscall;
    use crate::cert_cache::CertCacheComponent;
    use crate::journal_decode::decode_verifier_journal;
    use crate::tee_types::ProcessorType;
    use crate::tee_types::VerificationResult;

    // Embed the certificate cache component
    component!(path: CertCacheComponent, storage: cert_cache, event: CertCacheEvent);

    // Expose CertCache external functions
    #[abi(embed_v0)]
    impl CertCacheImpl = CertCacheComponent::CertCacheImpl<ContractState>;

    // Access to internal functions
    impl CertCacheInternalImpl = CertCacheComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        balance: felt252,
        verifier_class_hash: ClassHash,
        sp1_program_id: u256,
        max_time_diff: u64,
        #[substorage(v0)]
        cert_cache: CertCacheComponent::Storage,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        CertCacheEvent: CertCacheComponent::Event,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState,
        verifier_class_hash: ClassHash,
        sp1_program_id: u256,
        max_time_diff: u64,
        trusted_certs: Span<u256>,
        processor_models: Span<ProcessorType>,
        root_certs: Span<u256>,
    ) {
        self.verifier_class_hash.write(verifier_class_hash);
        self.sp1_program_id.write(sp1_program_id);
        self.max_time_diff.write(max_time_diff);

        // Initialize trusted intermediate certificates
        self.cert_cache.initialize_trusted_certs(trusted_certs);

        // Initialize root certificates for each processor model
        assert!(processor_models.len() == root_certs.len(), "Array length mismatch");

        for (root_cert, processor_model) in core::iter::zip(root_certs, processor_models) {
            self.cert_cache.set_root_cert(*processor_model, *root_cert);
        }
    }

    #[abi(embed_v0)]
    impl AMDTEERegistryImpl of super::IAMDTeeRegistry<ContractState> {
        fn verify_sp1_proof(ref self: ContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>> {
            // Step 1: Call the Garaga SP1 Verifier to validate the proof cryptographically
            // This verifies the Groth16 proof structure and cryptographic validity
            let mut result_serialized = library_call_syscall(
                self.verifier_class_hash.read(),
                selector!("verify_sp1_groth16_proof_bn254"),
                sp1_proof.span(),
            )
                .unwrap_syscall();

            // Step 2: Deserialize the verification result
            // The verifier returns Result<(verification_key, public_inputs), error>
            let result = Serde::<Result<(u256, Span<u256>), felt252>>::deserialize(
                ref result_serialized
            )
                .unwrap();

            // Step 3: Check if cryptographic verification succeeded
            match result {
                Result::Ok((vk, public_inputs)) => {
                    // Step 4: Verify this proof corresponds to our expected SP1 program
                    // This ensures we only accept proofs for the specific computation we expect
                    assert(vk == self.sp1_program_id.read(), 'Wrong program');

                    let journal = decode_verifier_journal(public_inputs);
                    if journal.result != VerificationResult::Success {
                        return None;
                    }
                    if journal.trusted_certs_prefix_len == 0 {
                        return None;
                    }

                    let mut processor_model: ProcessorType = ProcessorType::Milan;
                    match processor_type_from_u8(journal.processor_model) {
                        Option::Some(value) => {
                            processor_model = value;
                        },
                        Option::None => {
                            return None;
                        },
                    }

                    let expected_root_cert = self.cert_cache.get_root_cert(processor_model);
                    if expected_root_cert == 0 {
                        return None;
                    }

                    let mut certs: Array<u256> = journal.certs;
                    let trusted_len: usize = journal.trusted_certs_prefix_len.into();
                    if certs.len() < trusted_len {
                        return None;
                    }
                    if *certs.at(0) != expected_root_cert {
                        return None;
                    }

                    let mut i: usize = 1;
                    while i < trusted_len {
                        if !self.cert_cache.is_trusted_intermediate_cert(*certs.at(i)) {
                            return None;
                        }
                        i += 1;
                    }

                    let current_timestamp = get_block_timestamp();
                    if journal.timestamp > current_timestamp {
                        return None;
                    }
                    if current_timestamp - journal.timestamp > self.max_time_diff.read() {
                        return None;
                    }

                    let trusted_len_u32: u32 = journal.trusted_certs_prefix_len.into();
                    self.cert_cache.cache_new_cert(certs, trusted_len_u32);

                    // Return the public inputs for the verified computation
                    Some(public_inputs)
                },
                Result::Err(_) => None,
            }
        }
    }

    fn processor_type_from_u8(value: u8) -> Option<ProcessorType> {
        match value {
            0 => Option::Some(ProcessorType::Milan),
            1 => Option::Some(ProcessorType::Genoa),
            2 => Option::Some(ProcessorType::Bergamo),
            3 => Option::Some(ProcessorType::Siena),
            _ => Option::None,
        }
    }
}
