use crate::{state::SharedState, utils::format};
use ark_ec::short_weierstrass::Affine;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use stark_vrf::StarkCurve;

#[derive(Debug, Serialize, Deserialize)]
pub struct InfoResult {
    pub public_key_x: String,
    pub public_key_y: String,
}

impl InfoResult {
    pub fn from_public_key(public_key: Affine<StarkCurve>) -> InfoResult {
        InfoResult {
            public_key_x: format(public_key.x),
            public_key_y: format(public_key.y),
        }
    }
}

// curl http://0.0.0.0:3000/info

pub async fn vrf_info(State(state): State<SharedState>) -> Json<InfoResult> {
    let public_key = state.read().unwrap().public_key;

    Json(InfoResult::from_public_key(public_key))
}
