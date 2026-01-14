#[starknet::interface]
pub trait IAMDTeeRegistry<TContractState> {
    /// Verify a SP1 proof.
    fn verify_sp1_proof(self: @TContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>>;
}

#[starknet::contract]
pub mod AMDTEERegistry {
    use starknet::SyscallResultTrait;
    use starknet::syscalls::library_call_syscall;
    use crate::cert_cache::CertCacheComponent;
    use crate::tee_types::ProcessorType;

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
        #[substorage(v0)]
        cert_cache: CertCacheComponent::Storage,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        CertCacheEvent: CertCacheComponent::Event,
    }

    const SP1_VERIFIER_CLASS_HASH: felt252 =
        0x1234567890123456789012345678901234567890123456789012345678934;
    const SP1_PROGRAM: u256 = 0x1234567890123456789012345678901234567890123456789012345678901234;

    #[constructor]
    fn constructor(
        ref self: ContractState,
        trusted_certs: Array<u256>,
        processor_models: Array<ProcessorType>,
        root_certs: Array<u256>,
    ) {
        // Initialize trusted intermediate certificates
        self.cert_cache.initialize_trusted_certs(trusted_certs);

        // Initialize root certificates for each processor model
        assert!(processor_models.len() == root_certs.len(), "Array length mismatch");
        let mut i: u32 = 0;
        while i < processor_models.len() {
            self.cert_cache.set_root_cert(*processor_models.at(i), *root_certs.at(i));
            i += 1;
        };
    }

    #[abi(embed_v0)]
    impl AMDTEERegistryImpl of super::IAMDTeeRegistry<ContractState> {
        fn verify_sp1_proof(self: @ContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>> {
            // Step 1: Call the Garaga SP1 Verifier to validate the proof cryptographically
            // This verifies the Groth16 proof structure and cryptographic validity
            let mut result_serialized = library_call_syscall(
                SP1_VERIFIER_CLASS_HASH.try_into().unwrap(),
                selector!("verify_sp1_groth16_proof_bn254"),
                sp1_proof.span(),
            )
                .unwrap_syscall();

            // Step 2: Deserialize the verification result
            // The verifier returns Option<(verification_key, public_inputs)>
            let result = Serde::<Option<(u256, Span<u256>)>>::deserialize(ref result_serialized)
                .unwrap();

            // Step 3: Check if cryptographic verification succeeded
            if result.is_none() {
                return None;
            }

            // Step 4: Extract verification key and public inputs
            let (vk, public_inputs) = result.unwrap();

            // Step 5: Verify this proof corresponds to our expected SP1 program
            // This ensures we only accept proofs for the specific computation we expect
            assert(vk == SP1_PROGRAM, 'Wrong program');

            // Step 6: Return the public inputs for the verified computation
            // These inputs represent the publicly committed values from the SP1 program
            Some(public_inputs)
        }
    }
}
