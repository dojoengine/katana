use crate::oracle::{StarkVrfProof, StarkVrfRequest};
use crate::state::SharedState;
use crate::utils::format;
use axum::extract::State;
use axum::Json;
use num::{BigInt, Num};
use serde::{Deserialize, Serialize};
use stark_vrf::{BaseField, StarkVRF};
use std::str::FromStr;
use tracing::debug;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonResult {
    result: StarkVrfProof,
}

// curl -X POST -H "Content-Type: application/json" -d '{"seed": ["0x5db4e1c9bd8b0898674bf96f79e8fbffa3fe6d70a4597683c4dba2f0930dc45"]}' http://0.0.0.0:3000/proof

pub async fn vrf_proof(
    State(state): State<SharedState>,
    Json(payload): Json<StarkVrfRequest>,
) -> Json<JsonResult> {
    debug!("received payload {payload:?}");
    let secret_key = &state.read().unwrap().secret_key;
    let public_key = state.read().unwrap().public_key;

    debug!("public key {public_key}");

    let seed: Vec<_> = payload
        .seed
        .iter()
        .map(|x| {
            let dec_string = BigInt::from_str_radix(&x[2..], 16).unwrap().to_string();
            debug!("seed string {dec_string}");
            BaseField::from_str(&dec_string).unwrap()
        })
        .collect();

    let ecvrf = StarkVRF::new(public_key).unwrap();
    let proof = ecvrf
        .prove(&secret_key.parse().unwrap(), seed.as_slice())
        .unwrap();
    let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed.as_slice());
    let rnd = ecvrf.proof_to_hash(&proof).unwrap();

    debug!("proof gamma: {}", proof.0);
    debug!("proof c: {}", proof.1);
    debug!("proof s: {}", proof.2);
    debug!("proof verify hint: {}", sqrt_ratio_hint);

    let result = StarkVrfProof {
        gamma_x: format(proof.0.x),
        gamma_y: format(proof.0.y),
        c: format(proof.1),
        s: format(proof.2),
        sqrt_ratio: format(sqrt_ratio_hint),
        rnd: format(rnd),
    };

    debug!("result {result:?}");

    //let n = (payload.n as f64).sqrt() as u64;
    Json(JsonResult { result })
}
