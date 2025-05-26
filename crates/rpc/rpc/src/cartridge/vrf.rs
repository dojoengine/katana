use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use ark_ec::short_weierstrass::Affine;
use katana_primitives::contract::Nonce;
use katana_primitives::{ContractAddress, Felt};
use num_bigint::{BigInt, BigUint};
use parking_lot::Mutex;
use stark_vrf::{generate_public_key, BaseField, StarkCurve, StarkVRF};
use starknet::core::utils::get_contract_address;
use starknet::macros::{felt, short_string};
use tracing::trace;

// Class hash of the VRF provider contract (fee estimation code commented, since currently Katana
// returns 0 for the fees): <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L124>
// The contract is compiled in
// `crates/controller/artifacts/cartridge_vrf_VrfProvider.contract_class.json`
pub const CARTRIDGE_VRF_CLASS_HASH: Felt =
    felt!("0x07007ea60938ff539f1c0772a9e0f39b4314cfea276d2c22c29a8b64f2a87a58");
pub const CARTRIDGE_VRF_SALT: Felt = short_string!("cartridge_vrf");
pub const CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY: Felt = felt!("0x01");

#[derive(Debug, Default, Clone)]
pub struct StarkVrfProof {
    pub gamma_x: String,
    pub gamma_y: String,
    pub c: String,
    pub s: String,
    pub sqrt_ratio: String,
    pub rnd: String,
}

#[derive(Debug, Clone)]
pub struct VrfContext {
    private_key: Felt,
    public_key: Affine<StarkCurve>,
    contract_address: ContractAddress,
    cache: Arc<Mutex<HashMap<ContractAddress, Nonce>>>,
}

impl VrfContext {
    /// Creates a new [`VrfContext`] with the given private key and provider address.
    pub fn new(private_key: Felt, provider: ContractAddress) -> Self {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let public_key = generate_public_key(private_key.to_biguint().into());

        let contract_address = compute_vrf_address(
            provider,
            Felt::from(BigUint::from(public_key.x.0)),
            Felt::from(BigUint::from(public_key.y.0)),
        );

        Self { cache, private_key, public_key, contract_address }
    }

    /// Get the public key x and y coordinates as Felt values.
    pub fn get_public_key_xy_felts(&self) -> (Felt, Felt) {
        let x = Felt::from(BigUint::from(self.public_key.x.0));
        let y = Felt::from(BigUint::from(self.public_key.y.0));
        (x, y)
    }

    /// Retruns the address of the VRF contract.
    pub fn address(&self) -> ContractAddress {
        self.contract_address
    }

    /// Returns the private key of the VRF.
    pub fn private_key(&self) -> Felt {
        self.private_key
    }

    /// Returns the current internal nonce of the `address` and consume it. Consuming the nonce will
    /// increment it by one - ensuring the generated VRF seed will always be unique.
    ///
    /// This is when the VRF is requested with nonce as the source of randomness.
    ///
    /// Refer to <https://docs.cartridge.gg/vrf/overview#using-the-vrf-provider> for further information.
    pub fn consume_nonce(&self, address: ContractAddress) -> Nonce {
        let mut cache = self.cache.lock();
        let nonce = cache.get(&address).unwrap_or(&Felt::ZERO).to_owned();
        cache.insert(address, nonce + Felt::ONE);
        nonce
    }
}

/// Computes a VRF proof for the given seed.
pub fn stark_vrf(seed: Felt, vrf_private_key: Felt) -> anyhow::Result<StarkVrfProof> {
    let private_key = vrf_private_key.to_string();
    let public_key = generate_public_key(private_key.parse().unwrap());

    let seed = vec![BaseField::from_str(&format!("{seed}")).unwrap()];

    let ecvrf = StarkVRF::new(public_key).unwrap();
    let proof = ecvrf.prove(&private_key.parse().unwrap(), seed.as_slice()).unwrap();
    let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed.as_slice());
    let rnd = ecvrf.proof_to_hash(&proof).unwrap();

    let beta = ecvrf.proof_to_hash(&proof).unwrap();

    trace!(target: "rpc::cartridge", seed = ?seed[0], random_value = %format(beta), "Computing VRF proof.");

    Ok(StarkVrfProof {
        gamma_x: format(proof.0.x),
        gamma_y: format(proof.0.y),
        c: format(proof.1),
        s: format(proof.2),
        sqrt_ratio: format(sqrt_ratio_hint),
        rnd: format(rnd),
    })
}

/// Computes the deterministic VRF contract address from the provider address and the public
/// key coordinates.
fn compute_vrf_address(
    provider_addrss: ContractAddress,
    public_key_x: Felt,
    public_key_y: Felt,
) -> ContractAddress {
    get_contract_address(
        CARTRIDGE_VRF_SALT,
        CARTRIDGE_VRF_CLASS_HASH,
        &[*provider_addrss, public_key_x, public_key_y],
        Felt::ZERO,
    )
    .into()
}

/// Formats the given value as a hexadecimal string.
fn format<T: std::fmt::Display>(v: T) -> String {
    let int = BigInt::from_str(&format!("{v}")).unwrap();
    format!("0x{}", int.to_str_radix(16))
}
