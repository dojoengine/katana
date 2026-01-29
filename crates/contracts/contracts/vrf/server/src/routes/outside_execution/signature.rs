// https://github.com/neotheprogramist/starknet-hive/blob/48d4446b2e8ccf4194242cbe0102107f9df8e26d/openrpc-testgen/src/utils/outside_execution.rs#L20

use cainome::cairo_serde_derive::CairoSerde;
use starknet::signers::{LocalWallet, Signer};
use starknet_crypto::{poseidon_hash_many, Felt, PoseidonHasher};

use crate::routes::outside_execution::types::{Call, OutsideExecution};

pub const STARKNET_DOMAIN_TYPE_HASH: Felt = starknet_crypto::Felt::from_hex_unchecked(
    "0x1ff2f602e42168014d405a94f75e8a93d640751d71d16311266e140d8b0a210",
);
pub const CALL_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x3635c7f2a7ba93844c0d064e18e487f35ab90f7c39d00f186a781fc3f0c2ca9");
pub const OUTSIDE_EXECUTION_TYPE_HASH: Felt =
    Felt::from_hex_unchecked("0x312b56c05a7965066ddbda31c016d8d05afc305071c0ca3cdc2192c3c2f1f0f");

#[derive(Debug, CairoSerde)]
pub struct StarknetDomain {
    pub name: Felt,
    pub version: Felt,
    pub chain_id: Felt,
    pub revision: Felt,
}

pub fn get_starknet_domain_hash(chain_id: Felt) -> Felt {
    let domain = StarknetDomain {
        name: Felt::from_bytes_be_slice(b"Account.execute_from_outside"),
        version: Felt::TWO,
        chain_id,
        revision: Felt::ONE,
    };

    let domain_vec = vec![
        STARKNET_DOMAIN_TYPE_HASH,
        domain.name,
        domain.version,
        domain.chain_id,
        domain.revision,
    ];
    poseidon_hash_many(&domain_vec)
}

pub fn get_outside_execution_hash(outside_execution: &OutsideExecution) -> Felt {
    let calls_vec = outside_execution.calls().clone();
    let mut hashed_calls = Vec::<Felt>::new();

    for call in calls_vec {
        hashed_calls.push(get_call_hash(call));
    }

    let mut hasher_outside_execution = PoseidonHasher::new();
    hasher_outside_execution.update(OUTSIDE_EXECUTION_TYPE_HASH);
    hasher_outside_execution.update(outside_execution.caller());
    hasher_outside_execution.update(outside_execution.nonce());
    hasher_outside_execution.update(Felt::from(outside_execution.execute_after()));
    hasher_outside_execution.update(Felt::from(outside_execution.execute_before()));
    hasher_outside_execution.update(poseidon_hash_many(&hashed_calls));

    hasher_outside_execution.finalize()
}

pub fn get_call_hash(call: Call) -> Felt {
    let mut hasher_call = PoseidonHasher::new();
    hasher_call.update(CALL_TYPE_HASH);
    hasher_call.update(call.to);
    hasher_call.update(call.selector);
    hasher_call.update(poseidon_hash_many(&call.calldata));
    hasher_call.finalize()
}

pub async fn sign_outside_execution(
    outside_execution: &OutsideExecution,
    chain_id: Felt,
    signer_address: Felt,
    signer: LocalWallet,
) -> Vec<Felt> {
    let mut final_hasher = PoseidonHasher::new();
    final_hasher.update(Felt::from_bytes_be_slice(b"StarkNet Message"));
    final_hasher.update(get_starknet_domain_hash(chain_id));
    final_hasher.update(signer_address);
    final_hasher.update(get_outside_execution_hash(outside_execution));

    let hash = final_hasher.finalize();

    let signature = signer.sign_hash(&hash).await.unwrap();

    vec![signature.r, signature.s]
}
