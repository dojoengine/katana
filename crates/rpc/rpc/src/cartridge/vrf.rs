use std::collections::HashMap;
use std::sync::Arc;

use ark_ec::short_weierstrass::Affine;
use katana_primitives::contract::Nonce;
use katana_primitives::{ContractAddress, Felt};
use num_bigint::BigUint;
use parking_lot::Mutex;
use stark_vrf::{generate_public_key, StarkCurve};
use starknet::core::utils::get_contract_address;
use starknet::macros::{felt, short_string};

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
    pub cache: Arc<Mutex<HashMap<ContractAddress, Nonce>>>,
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

/// Computes the deterministic VRF contract address from the paymaster address and the public
/// key coordinates.
fn compute_vrf_address(
    pm_address: ContractAddress,
    public_key_x: Felt,
    public_key_y: Felt,
) -> ContractAddress {
    get_contract_address(
        CARTRIDGE_VRF_SALT,
        CARTRIDGE_VRF_CLASS_HASH,
        &[*pm_address, public_key_x, public_key_y],
        Felt::ZERO,
    )
    .into()
}
