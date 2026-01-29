use crate::Args;
use ark_ec::short_weierstrass::Affine;
use cainome_cairo_serde::ContractAddress;
use clap::Parser;
use stark_vrf::{generate_public_key, StarkCurve};
use starknet::core::types::Felt;
use starknet::signers::{LocalWallet, SigningKey};
use std::ops::Deref;
use std::sync::{Arc, RwLock};

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
    pub secret_key: String,
    pub public_key: Affine<StarkCurve>,
    pub vrf_account_address: ContractAddress,
    pub vrf_signer: LocalWallet,
}

impl AppState {
    pub async fn new() -> AppState {
        let args = Args::parse();
        AppState::from_args(&args).await
    }

    pub async fn from_args(args: &Args) -> AppState {
        let secret_key = args.secret_key.to_string();
        let public_key = generate_public_key(secret_key.parse().unwrap());

        let vrf_account_address =
            ContractAddress::from(Felt::from_hex(&args.account_address).unwrap());
        let vrf_signer = LocalWallet::from(SigningKey::from_secret_scalar(
            Felt::from_hex(&args.account_private_key).unwrap(),
        ));

        AppState {
            secret_key,
            public_key,
            vrf_account_address,
            vrf_signer,
        }
    }
}
