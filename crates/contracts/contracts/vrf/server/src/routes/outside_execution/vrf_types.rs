// VRF

use cainome::cairo_serde_derive::CairoSerde;
use cainome_cairo_serde::ContractAddress;
use num::{BigInt, Num};
use serde::{Deserialize, Serialize};
use stark_vrf::{BaseField, StarkVRF};
use starknet::{core::types::BlockId, macros::selector, providers::Provider};
use starknet_crypto::{pedersen_hash, poseidon_hash_many, Felt};
use std::str::FromStr;

use crate::{
    routes::outside_execution::{
        context::VrfContext,
        types::{Call, OutsideExecution},
        Errors,
    },
    utils::format_felt,
};

#[derive(Clone, CairoSerde, Serialize, Deserialize, Debug)]
pub enum Source {
    Nonce(ContractAddress),
    Salt(Felt),
}

#[derive(Clone, CairoSerde, Serialize, Deserialize, Debug)]
pub struct RequestRandom {
    pub caller: ContractAddress,
    pub source: Source,
}

impl RequestRandom {
    pub fn get_request_random_call(outside_execution: &OutsideExecution) -> (Option<Call>, usize) {
        let calls = outside_execution.calls();

        let position = calls
            .iter()
            .position(|call| call.selector == selector!("request_random"));

        match position {
            Some(position) => (Option::Some(calls.get(position).unwrap().clone()), position),
            None => (Option::None, 0),
        }
    }

    pub async fn compute_seed(
        self: &RequestRandom,
        vrf_context: &VrfContext,
    ) -> Result<Felt, Errors> {
        let caller = self.caller.0;

        let seed = match self.source {
            Source::Nonce(contract_address) => {
                let provider = vrf_context.provider.clone();
                let vrf_account_address = vrf_context.vrf_account_address;

                let key = pedersen_hash(&selector!("VrfProvider_nonces"), &contract_address.0);
                let nonce = provider
                    .get_storage_at(
                        vrf_account_address.0,
                        key,
                        BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed),
                    )
                    .await?;

                poseidon_hash_many(&[nonce, contract_address.0, caller, vrf_context.chain_id])
            }
            Source::Salt(felt) => poseidon_hash_many(&[felt, caller, vrf_context.chain_id]),
        };

        Ok(seed)
    }
}

pub fn build_submit_random_call(vrf_context: &VrfContext, seed: Felt) -> Call {
    let seed_vec: Vec<_> = [seed]
        .iter()
        .map(|x| {
            let x = x.to_hex_string();
            let dec_string = BigInt::from_str_radix(&x[2..], 16).unwrap().to_string();
            BaseField::from_str(&dec_string).unwrap()
        })
        .collect();

    let ecvrf = StarkVRF::new(vrf_context.public_key).unwrap();
    let proof = ecvrf
        .prove(
            &vrf_context.secret_key.parse().unwrap(),
            seed_vec.as_slice(),
        )
        .unwrap();
    let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed_vec.as_slice());
    // let rnd = ecvrf.proof_to_hash(&proof).unwrap();

    Call {
        to: vrf_context.vrf_account_address.0,
        selector: selector!("submit_random"),
        calldata: vec![
            seed,
            format_felt(proof.0.x),
            format_felt(proof.0.y),
            format_felt(proof.1),
            format_felt(proof.2),
            format_felt(sqrt_ratio_hint),
        ],
    }
}
