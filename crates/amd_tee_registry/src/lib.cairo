#[starknet::interface]
pub trait IAMDTeeRegistry<TContractState> {
    /// Verify a SP1 proof.
    fn verify_sp1_proof(self: @TContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>>;
}

/// Simple contract for managing AMD TEE registry.
#[starknet::contract]
mod AMDTEERegistry {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    #[storage]
    struct Storage {
        balance: felt252,
    }

    const SP1_VERIFIER_CLASS_HASH: u256 = 0x1234567890123456789012345678901234567890123456789012345678901234;
    const SP1_PROGRAM: u256 = 0x1234567890123456789012345678901234567890123456789012345678901234;

    #[abi(embed_v0)]
    impl AMDTEERegistryImpl of super::IAMDTeeRegistry<ContractState> {
        fn verify_sp1_proof(ref self: ContractState, sp1_proof: Array<felt252>) -> Option<Span<u256>> {
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
}
