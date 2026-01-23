// VRF

use cainome::cairo_serde_derive::CairoSerde;
use cainome_cairo_serde::ContractAddress;
use num_bigint::BigInt;
use num_traits::Num;
use serde::{Deserialize, Serialize};
use stark_vrf::{BaseField, StarkVRF};
use starknet::{
    core::types::{BlockId, BlockTag, Felt},
    macros::selector,
    providers::Provider,
};
use starknet_crypto::{pedersen_hash, poseidon_hash_many};
use std::str::FromStr;

use crate::{
    routes::outside_execution::{
        context::VrfContext,
        types::{Call, OutsideExecution},
        Errors,
    },
    utils::format_felt,
    utils::felt_to_scalar,
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
                        BlockId::Tag(BlockTag::PreConfirmed),
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
        .prove(&felt_to_scalar(vrf_context.secret_key), seed_vec.as_slice())
        .unwrap();
    let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed_vec.as_slice());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::outside_execution::context::VrfContext;
    use crate::utils::felt_to_scalar;
    use cainome_cairo_serde::ContractAddress;
    use stark_vrf::{generate_public_key, StarkVRF};
    use starknet::{
        core::types::Felt,
        providers::{jsonrpc::HttpTransport, JsonRpcClient, Url},
        signers::{LocalWallet, SigningKey},
    };

    fn felt_from_display<T: std::fmt::Display>(value: T) -> Felt {
        Felt::from_dec_str(&value.to_string()).expect("valid felt")
    }

    #[test]
    fn submit_random_call_matches_proof() {
        let secret_key = Felt::from(0x123_u128);
        let public_key = generate_public_key(felt_to_scalar(secret_key));
        let vrf_account_address = ContractAddress::from(Felt::from(0x456_u128));

        let provider =
            JsonRpcClient::new(HttpTransport::new(Url::parse("http://localhost:0").unwrap()));
        let vrf_signer =
            LocalWallet::from(SigningKey::from_secret_scalar(Felt::from(0x789_u128)));

        let vrf_context = VrfContext {
            chain_id: Felt::from(1_u8),
            provider,
            secret_key,
            public_key,
            vrf_account_address,
            vrf_signer,
        };

        let seed = Felt::from(0xabc_u128);
        let call = build_submit_random_call(&vrf_context, seed);

        let seed_vec = vec![BaseField::from_str(&seed.to_biguint().to_str_radix(10)).unwrap()];
        let ecvrf = StarkVRF::new(public_key).unwrap();
        let proof = ecvrf
            .prove(&felt_to_scalar(secret_key), seed_vec.as_slice())
            .unwrap();
        let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed_vec.as_slice());

        let expected = vec![
            seed,
            felt_from_display(proof.0.x),
            felt_from_display(proof.0.y),
            felt_from_display(proof.1),
            felt_from_display(proof.2),
            felt_from_display(sqrt_ratio_hint),
        ];

        assert_eq!(call.selector, selector!("submit_random"));
        assert_eq!(call.to, vrf_account_address.0);
        assert_eq!(call.calldata, expected);
    }
}
