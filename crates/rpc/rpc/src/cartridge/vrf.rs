use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::anyhow;
use ark_ec::short_weierstrass::Affine;
use katana_primitives::chain::ChainId;
use katana_primitives::fee::ResourceBoundsMapping;
use katana_primitives::genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use num_bigint::BigUint;
use stark_vrf::{generate_public_key, StarkCurve};
use starknet::core::types::Call;
use starknet::core::utils::get_contract_address;
use starknet::macros::{felt, selector, short_string};
use starknet::signers::{LocalWallet, Signer, SigningKey};

use super::encode_calls;

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
    pub cache: Arc<RwLock<HashMap<Felt, Felt>>>,
}

impl VrfContext {
    /// Creates a new [`VrfContext`] with the given private key and provider address.
    pub fn new(private_key: Felt, provider: ContractAddress) -> Self {
        let cache = Arc::new(RwLock::new(HashMap::new()));
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
}

/// Crafts a deploy of the VRF provider contract transaction.
pub async fn craft_deploy_cartridge_vrf_tx(
    paymaster_address: ContractAddress,
    paymaster_private_key: Felt,
    chain_id: ChainId,
    paymaster_nonce: Felt,
    public_key_x: Felt,
    public_key_y: Felt,
) -> anyhow::Result<ExecutableTxWithHash> {
    let calldata = vec![
        CARTRIDGE_VRF_CLASS_HASH,
        CARTRIDGE_VRF_SALT,
        // from zero
        Felt::ZERO,
        // Calldata len
        Felt::THREE,
        // owner
        paymaster_address.into(),
        // public key
        public_key_x,
        public_key_y,
    ];

    let call =
        Call { to: DEFAULT_UDC_ADDRESS.into(), selector: selector!("deployContract"), calldata };

    let mut tx = InvokeTxV3 {
        chain_id,
        tip: 0_u64,
        signature: vec![],
        paymaster_data: vec![],
        account_deployment_data: vec![],
        sender_address: paymaster_address,
        calldata: encode_calls(vec![call]),
        nonce: paymaster_nonce,
        resource_bounds: ResourceBoundsMapping::default(),
        nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
        fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
    };

    let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(paymaster_private_key));
    let signature = signer
        .sign_hash(&tx_hash)
        .await
        .map_err(|e| anyhow!("failed to sign hash with paymaster: {e}"))?;
    tx.signature = vec![signature.r, signature.s];

    let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));

    Ok(tx)
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
