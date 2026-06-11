//! Protocol-level constants and helpers for Katana's TEE settlement and attestation flow.
//!
//! Lives in chain-spec because both the chain-spec settlement validator and the runtime TEE RPC
//! attestation layer need them, and chain-spec is a fundamental crate that both can depend on
//! without cycles.

use katana_primitives::cairo::ShortString;
use katana_primitives::hash::{Pedersen, StarkHash};
use katana_primitives::Felt;

/// Version tag for the Katana TEE environment config hash.
pub const KATANA_TEE_CONFIG_VERSION: ShortString = ShortString::from_ascii("KatanaTeeConfig1");

/// Version tag for the v1 Katana TEE report-data schema.
pub const KATANA_TEE_REPORT_VERSION: ShortString = ShortString::from_ascii("KatanaTeeReport1");

/// Mode tag for appchain settlement attestations.
pub const KATANA_TEE_APPCHAIN_MODE: ShortString = ShortString::from_ascii("KatanaTeeAppchain");

/// Mode tag for fork/sharding attestations.
pub const KATANA_TEE_SHARDING_MODE: ShortString = ShortString::from_ascii("KatanaTeeSharding");

/// Computes the Starknet-OS-style config hash bound into v1 TEE report data.
///
/// Mirrors the Starknet OS pattern of hashing a version tag plus stable environment fields. The
/// non-legacy fee token address is the STRK fee token address for the appchain/Katana network.
pub fn compute_katana_tee_config_hash(
    appchain_chain_id_or_katana_chain_id: Felt,
    fee_token_address: Felt,
) -> Felt {
    Pedersen::hash_array(&[
        KATANA_TEE_CONFIG_VERSION.into(),
        appchain_chain_id_or_katana_chain_id,
        fee_token_address,
    ])
}
