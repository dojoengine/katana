use std::ops::Deref;
use std::sync::{Arc, RwLock};

use ark_ec::short_weierstrass::Affine;
use cainome_cairo_serde::ContractAddress;
use stark_vrf::{generate_public_key, StarkCurve};
use starknet::core::types::Felt;
use starknet::signers::{LocalWallet, SigningKey};

use crate::utils::{felt_to_scalar, parse_felt};
use crate::Args;

#[derive(Clone)]
pub struct SharedState(pub Arc<RwLock<AppState>>);

impl Deref for SharedState {
    type Target = Arc<RwLock<AppState>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SharedState {
    pub async fn get(&self) -> AppState {
        self.0.read().unwrap().clone()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub secret_key: Felt,
    pub public_key: Affine<StarkCurve>,
    pub vrf_account_address: ContractAddress,
    pub vrf_signer: LocalWallet,
}

impl AppState {
    pub async fn from_args(args: &Args) -> AppState {
        let secret_key = parse_felt(&args.secret_key).expect("Invalid secret key");
        let public_key = generate_public_key(felt_to_scalar(secret_key));

        let vrf_account_address = ContractAddress::from(
            parse_felt(&args.account_address).expect("Invalid account address"),
        );
        let vrf_signer = LocalWallet::from(SigningKey::from_secret_scalar(
            parse_felt(&args.account_private_key).expect("Invalid account private key"),
        ));

        AppState { secret_key, public_key, vrf_account_address, vrf_signer }
    }
}
