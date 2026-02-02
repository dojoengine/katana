//! VRF (Verifiable Random Function) service for Cartridge.

use std::time::{SystemTime, UNIX_EPOCH};

use cainome::cairo_serde_derive::CairoSerde as CairoSerdeDerive;
use cainome_cairo_serde::ContractAddress as CairoContractAddress;
use cartridge::vrf::StarkVrfProof;
use cartridge::VrfClient;
use katana_primitives::chain::ChainId;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_provider::api::state::StateProvider;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::outside_execution::{OutsideExecution, OutsideExecutionV2};
use serde::{Deserialize, Serialize};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_crypto::{pedersen_hash, poseidon_hash_many, PoseidonHasher};

use super::CartridgeVrfConfig;

const STARKNET_DOMAIN_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x1ff2f602e42168014d405a94f75e8a93d640751d71d16311266e140d8b0a210");
const CALL_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x3635c7f2a7ba93844c0d064e18e487f35ab90f7c39d00f186a781fc3f0c2ca9");
const OUTSIDE_EXECUTION_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x312b56c05a7965066ddbda31c016d8d05afc305071c0ca3cdc2192c3c2f1f0f");
const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552");

#[derive(Clone)]
pub struct VrfService {
    client: VrfClient,
    account_address: ContractAddress,
    account_private_key: Felt,
}

impl VrfService {
    pub fn new(config: CartridgeVrfConfig) -> Self {
        Self {
            client: VrfClient::new(config.url),
            account_address: config.account_address,
            account_private_key: config.account_private_key,
        }
    }

    pub fn account_address(&self) -> ContractAddress {
        self.account_address
    }

    pub fn account_private_key(&self) -> Felt {
        self.account_private_key
    }

    pub async fn prove(&self, seed: Felt) -> Result<StarkVrfProof, StarknetApiError> {
        self.client
            .proof(vec![seed.to_hex_string()])
            .await
            .map_err(|err| StarknetApiError::unexpected(format!("vrf proof request failed: {err}")))
    }
}

#[derive(Clone, CairoSerdeDerive, Serialize, Deserialize, Debug)]
pub(super) enum VrfSource {
    Nonce(CairoContractAddress),
    Salt(Felt),
}

#[derive(Clone, CairoSerdeDerive, Serialize, Deserialize, Debug)]
pub(super) struct VrfRequestRandom {
    pub caller: CairoContractAddress,
    pub source: VrfSource,
}

pub(super) fn request_random_call(
    outside_execution: &OutsideExecution,
) -> Option<(katana_rpc_types::outside_execution::Call, usize)> {
    let calls = match outside_execution {
        OutsideExecution::V2(v2) => &v2.calls,
        OutsideExecution::V3(v3) => &v3.calls,
    };

    calls
        .iter()
        .position(|call| call.selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}

pub(super) fn outside_execution_calls_len(outside_execution: &OutsideExecution) -> usize {
    match outside_execution {
        OutsideExecution::V2(v2) => v2.calls.len(),
        OutsideExecution::V3(v3) => v3.calls.len(),
    }
}

pub(super) fn compute_vrf_seed(
    state: &dyn StateProvider,
    vrf_account_address: ContractAddress,
    request_random: &VrfRequestRandom,
    chain_id: Felt,
) -> Result<Felt, StarknetApiError> {
    let caller = request_random.caller.0;

    match &request_random.source {
        VrfSource::Nonce(contract_address) => {
            let storage_key = pedersen_hash(&selector!("VrfProvider_nonces"), &contract_address.0);
            let nonce = state.storage(vrf_account_address, storage_key)?.unwrap_or_default();
            Ok(poseidon_hash_many(&[nonce, contract_address.0, caller, chain_id]))
        }
        VrfSource::Salt(salt) => Ok(poseidon_hash_many(&[*salt, caller, chain_id])),
    }
}

pub(super) fn build_submit_random_call(
    vrf_account_address: ContractAddress,
    seed: Felt,
    proof: &StarkVrfProof,
) -> katana_rpc_types::outside_execution::Call {
    katana_rpc_types::outside_execution::Call {
        to: vrf_account_address,
        selector: selector!("submit_random"),
        calldata: vec![seed, proof.gamma_x, proof.gamma_y, proof.c, proof.s, proof.sqrt_ratio],
    }
}

pub(super) async fn build_vrf_outside_execution(
    account_address: ContractAddress,
    account_private_key: Felt,
    chain_id: ChainId,
    calls: Vec<katana_rpc_types::outside_execution::Call>,
) -> Result<(OutsideExecutionV2, Vec<Felt>), StarknetApiError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| StarknetApiError::unexpected(format!("clock error: {err}")))?
        .as_secs();
    let outside_execution = OutsideExecutionV2 {
        caller: ContractAddress::from(ANY_CALLER),
        execute_after: 0,
        execute_before: now + 600,
        calls,
        nonce: SigningKey::from_random().secret_scalar(),
    };

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(account_private_key));
    let signature =
        sign_outside_execution_v2(&outside_execution, chain_id.id(), account_address, signer)
            .await?;

    Ok((outside_execution, signature))
}

async fn sign_outside_execution_v2(
    outside_execution: &OutsideExecutionV2,
    chain_id: Felt,
    signer_address: ContractAddress,
    signer: LocalWallet,
) -> Result<Vec<Felt>, StarknetApiError> {
    let mut final_hasher = PoseidonHasher::new();
    final_hasher.update(Felt::from_bytes_be_slice(b"StarkNet Message"));
    final_hasher.update(starknet_domain_hash(chain_id));
    final_hasher.update(signer_address.into());
    final_hasher.update(outside_execution_hash(outside_execution));

    let hash = final_hasher.finalize();
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| StarknetApiError::unexpected(format!("failed to sign vrf execution: {e}")))?;

    Ok(vec![signature.r, signature.s])
}

fn starknet_domain_hash(chain_id: Felt) -> Felt {
    let domain = [
        STARKNET_DOMAIN_TYPE_HASH,
        Felt::from_bytes_be_slice(b"Account.execute_from_outside"),
        Felt::TWO,
        chain_id,
        Felt::ONE,
    ];
    poseidon_hash_many(&domain)
}

fn outside_execution_hash(outside_execution: &OutsideExecutionV2) -> Felt {
    let hashed_calls: Vec<Felt> = outside_execution.calls.iter().map(call_hash).collect();

    let mut hasher = PoseidonHasher::new();
    hasher.update(OUTSIDE_EXECUTION_TYPE_HASH);
    hasher.update(outside_execution.caller.into());
    hasher.update(outside_execution.nonce);
    hasher.update(Felt::from(outside_execution.execute_after));
    hasher.update(Felt::from(outside_execution.execute_before));
    hasher.update(poseidon_hash_many(&hashed_calls));
    hasher.finalize()
}

fn call_hash(call: &katana_rpc_types::outside_execution::Call) -> Felt {
    let mut hasher = PoseidonHasher::new();
    hasher.update(CALL_TYPE_HASH);
    hasher.update(call.to.into());
    hasher.update(call.selector);
    hasher.update(poseidon_hash_many(&call.calldata));
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;

    use katana_primitives::contract::{StorageKey, StorageValue};
    use katana_provider::api::contract::ContractClassProvider;
    use katana_provider::api::state::{StateProofProvider, StateProvider, StateRootProvider};
    use katana_provider::ProviderResult;
    use stark_vrf::{generate_public_key, BaseField, ScalarField, StarkVRF};
    use starknet::macros::selector;
    use starknet_crypto::{pedersen_hash, poseidon_hash_many};

    use super::*;

    fn felt_from_display<T: std::fmt::Display>(value: T) -> Felt {
        Felt::from_dec_str(&value.to_string()).expect("valid felt")
    }

    #[test]
    fn request_random_call_finds_position() {
        let vrf_address = ContractAddress::from(felt!("0x123"));
        let other_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("transfer"),
            calldata: vec![Felt::ONE],
        };
        let vrf_call = katana_rpc_types::outside_execution::Call {
            to: vrf_address,
            selector: selector!("request_random"),
            calldata: vec![Felt::TWO],
        };

        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            execute_after: 0,
            execute_before: 100,
            calls: vec![other_call.clone(), vrf_call.clone()],
            nonce: Felt::THREE,
        });

        let (call, position) =
            request_random_call(&outside_execution).expect("request_random found");
        assert_eq!(position, 1);
        assert_eq!(call.selector, vrf_call.selector);
        assert_eq!(call.calldata, vrf_call.calldata);
    }

    #[test]
    fn submit_random_call_matches_proof() {
        let secret_key = Felt::from(0x123_u128);
        let secret_key_scalar =
            ScalarField::from_str(&secret_key.to_biguint().to_str_radix(10)).unwrap();
        let public_key = generate_public_key(secret_key_scalar);
        let vrf_account_address = ContractAddress::from(Felt::from(0x456_u128));

        let seed = Felt::from(0xabc_u128);
        let seed_vec = vec![BaseField::from_str(&seed.to_biguint().to_str_radix(10)).unwrap()];
        let ecvrf = StarkVRF::new(public_key).unwrap();
        let proof = ecvrf.prove(&secret_key_scalar, seed_vec.as_slice()).unwrap();
        let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed_vec.as_slice());
        let rnd = ecvrf.proof_to_hash(&proof).unwrap();

        let vrf_proof = StarkVrfProof {
            gamma_x: felt_from_display(proof.0.x),
            gamma_y: felt_from_display(proof.0.y),
            c: felt_from_display(proof.1),
            s: felt_from_display(proof.2),
            sqrt_ratio: felt_from_display(sqrt_ratio_hint),
            rnd: felt_from_display(rnd),
        };

        let call = build_submit_random_call(vrf_account_address, seed, &vrf_proof);

        let expected = vec![
            seed,
            felt_from_display(proof.0.x),
            felt_from_display(proof.0.y),
            felt_from_display(proof.1),
            felt_from_display(proof.2),
            felt_from_display(sqrt_ratio_hint),
        ];

        assert_eq!(call.selector, selector!("submit_random"));
        assert_eq!(call.to, vrf_account_address);
        assert_eq!(call.calldata, expected);
    }

    #[derive(Default)]
    struct StubState {
        storage: HashMap<(ContractAddress, StorageKey), StorageValue>,
    }

    impl ContractClassProvider for StubState {
        fn class(
            &self,
            _hash: katana_primitives::class::ClassHash,
        ) -> ProviderResult<Option<katana_primitives::class::ContractClass>> {
            Ok(None)
        }

        fn compiled_class_hash_of_class_hash(
            &self,
            _hash: katana_primitives::class::ClassHash,
        ) -> ProviderResult<Option<katana_primitives::class::CompiledClassHash>> {
            Ok(None)
        }
    }

    impl StateRootProvider for StubState {}
    impl StateProofProvider for StubState {}

    impl StateProvider for StubState {
        fn nonce(&self, _address: ContractAddress) -> ProviderResult<Option<Felt>> {
            Ok(None)
        }

        fn storage(
            &self,
            address: ContractAddress,
            storage_key: StorageKey,
        ) -> ProviderResult<Option<StorageValue>> {
            Ok(self.storage.get(&(address, storage_key)).copied())
        }

        fn class_hash_of_contract(
            &self,
            _address: ContractAddress,
        ) -> ProviderResult<Option<katana_primitives::class::ClassHash>> {
            Ok(None)
        }
    }

    #[test]
    fn compute_vrf_seed_uses_nonce_storage() {
        let vrf_account_address = ContractAddress::from(Felt::from(0x100_u128));
        let caller = CairoContractAddress(Felt::from(0x200_u128));
        let source = CairoContractAddress(Felt::from(0x300_u128));
        let request = VrfRequestRandom { caller, source: VrfSource::Nonce(source) };

        let storage_key = pedersen_hash(&selector!("VrfProvider_nonces"), &source.0);
        let nonce = Felt::from(0x1234_u128);

        let mut state = StubState::default();
        state.storage.insert((vrf_account_address, storage_key), nonce);

        let chain_id = Felt::from(0x534e5f4d41494e_u128);
        let seed = compute_vrf_seed(&state, vrf_account_address, &request, chain_id).expect("seed");

        let expected = poseidon_hash_many(&[nonce, source.0, caller.0, chain_id]);
        assert_eq!(seed, expected);
    }

    #[test]
    fn compute_vrf_seed_uses_salt() {
        let vrf_account_address = ContractAddress::from(Felt::from(0x100_u128));
        let caller = CairoContractAddress(Felt::from(0x200_u128));
        let salt = Felt::from(0x999_u128);
        let request = VrfRequestRandom { caller, source: VrfSource::Salt(salt) };

        let state = StubState::default();
        let chain_id = Felt::from(0x534e5f4d41494e_u128);
        let seed = compute_vrf_seed(&state, vrf_account_address, &request, chain_id).expect("seed");

        let expected = poseidon_hash_many(&[salt, caller.0, chain_id]);
        assert_eq!(seed, expected);
    }
}
