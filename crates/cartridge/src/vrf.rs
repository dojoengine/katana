//! For the VRF, the integration works as follows (if an execution from outside is targetting the
//! VRF provider contract):
//!
//! 1. The VRF provider contract is deployed (if not already deployed).
//!
//! 2. The Stark VRF proof is generated using the `Source` provided in the call. The seed is
//!    precomputed to match smart contract behavior <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L112>.
//!
//! 3. The original execution from outside call is then sandwitched between two VRF calls, one for
//!    submitting the randomness, and one to assert the correct consumption of the randomness.
//!
//! 4. When using the VRF, the user has the responsability to add a first call to target the VRF
//!    provider contract `request_random` entrypoint. This call sets which `Source` will be used
//!    to generate the randomness.
//!    <https://docs.cartridge.gg/vrf/overview#executing-vrf-transactions>
//!
//! In the current implementation, the VRF contract is deployed with a default private key, or read
//! from environment variable `KATANA_VRF_PRIVATE_KEY`. It is important to note that changing the
//! private key will result in a different VRF provider contract address.

use std::str::FromStr;

use ark_ec::short_weierstrass::Affine;
use katana_primitives::cairo::ShortString;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{felt, ContractAddress, Felt};
use num_bigint::BigInt;
use stark_vrf::{generate_public_key, BaseField, StarkCurve, StarkVRF};
use tracing::trace;

// Class hash of the VRF provider contract (fee estimation code commented, since currently Katana
// returns 0 for the fees): <https://github.com/cartridge-gg/vrf/blob/38d71385f939a19829113c122f1ab12dbbe0f877/src/vrf_provider/vrf_provider_component.cairo#L124>
// The contract is compiled in
// `crates/controller/artifacts/cartridge_vrf_VrfProvider.contract_class.json`
pub const CARTRIDGE_VRF_CLASS_HASH: Felt =
    felt!("0x07007ea60938ff539f1c0772a9e0f39b4314cfea276d2c22c29a8b64f2a87a58");
pub const CARTRIDGE_VRF_SALT: ShortString = ShortString::from_ascii("cartridge_vrf");
pub const CARTRIDGE_VRF_DEFAULT_PRIVATE_KEY: Felt = felt!("0x1");

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
}

impl VrfContext {
    /// Creates a new [`VrfContext`] with the given private key and provider address.
    pub fn new(private_key: Felt, provider: ContractAddress) -> Self {
        let public_key = generate_public_key(private_key.to_biguint().into());

        let contract_address = compute_vrf_address(
            provider,
            Felt::from_hex(&format(public_key.x)).unwrap(),
            Felt::from_hex(&format(public_key.y)).unwrap(),
        );

        Self { private_key, public_key, contract_address }
    }

    /// Get the public key x and y coordinates as Felt values.
    pub fn get_public_key_xy_felts(&self) -> (Felt, Felt) {
        let x = Felt::from_hex(&format(self.public_key.x)).unwrap();
        let y = Felt::from_hex(&format(self.public_key.y)).unwrap();

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

    /// Returns the public key of the VRF.
    pub fn public_key(&self) -> &Affine<StarkCurve> {
        &self.public_key
    }

    /// Computes a VRF proof for the given seed.
    pub fn stark_vrf(&self, seed: Felt) -> anyhow::Result<StarkVrfProof> {
        let private_key = self.private_key.to_string();
        let public_key = self.public_key;

        let seed = vec![BaseField::from(seed.to_biguint())];
        let ecvrf = StarkVRF::new(public_key)?;
        let proof = ecvrf.prove(&private_key.parse().unwrap(), seed.as_slice())?;
        let sqrt_ratio_hint = ecvrf.hash_to_sqrt_ratio_hint(seed.as_slice());
        let rnd = ecvrf.proof_to_hash(&proof)?;

        let beta = ecvrf.proof_to_hash(&proof)?;

        trace!(target: "cartridge", seed = ?seed[0], random_value = %format(beta), "Computing VRF proof.");

        Ok(StarkVrfProof {
            gamma_x: format(proof.0.x),
            gamma_y: format(proof.0.y),
            c: format(proof.1),
            s: format(proof.2),
            sqrt_ratio: format(sqrt_ratio_hint),
            rnd: format(rnd),
        })
    }
}

/// Computes the deterministic VRF contract address from the provider address and the public
/// key coordinates.
fn compute_vrf_address(
    provider_addrss: ContractAddress,
    public_key_x: Felt,
    public_key_y: Felt,
) -> ContractAddress {
    get_contract_address(
        CARTRIDGE_VRF_SALT.into(),
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
