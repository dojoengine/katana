//! Version-aware block hash computation for synced blocks.
//!
//! The Starknet block hash algorithm changed across protocol versions:
//!
//! - **Pre-0.7**: Pedersen hash chain with chain ID, uses zero for timestamp and sequencer address.
//!
//! - **0.7 to pre-0.13.2**: Pedersen hash chain without chain ID, uses actual timestamp and
//!   sequencer address.
//!
//! - **Post-0.13.2 (v0)**: Poseidon hash with `STARKNET_BLOCK_HASH0` prefix, individual gas price
//!   fields, and the protocol version as a short string.
//!
//! - **Post-0.13.4 (v1)**: Poseidon hash with `STARKNET_BLOCK_HASH1` prefix, gas prices
//!   consolidated into a single Poseidon hash.

use katana_primitives::block::Header;
use katana_primitives::cairo::ShortString;
use katana_primitives::chain::ChainId;
use katana_primitives::version::StarknetVersion;
use katana_primitives::Felt;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

/// Computes the block hash for a header, dispatching to the correct algorithm based
/// on the block's `starknet_version`.
///
/// The `chain_id` is required for pre-0.7 blocks which include the chain ID in the hash.
pub fn compute_hash(header: &Header, chain_id: &ChainId) -> Felt {
    let version_str = header.starknet_version.to_string();

    if header.starknet_version < StarknetVersion::V0_7_0 {
        compute_hash_pre_0_7(header, chain_id)
    } else if header.starknet_version < StarknetVersion::V0_13_2 {
        compute_hash_pre_0_13_2(header)
    } else if header.starknet_version < StarknetVersion::new([0, 13, 4, 0]) {
        compute_hash_post_0_13_2(header, &version_str)
    } else {
        compute_hash_post_0_13_4(header, &version_str)
    }
}

/// Pre-0.7 block hash using Pedersen hash chain with chain ID.
///
/// Used for the earliest Starknet blocks (mainnet blocks before ~833). These blocks
/// use zero for both timestamp and sequencer address, and include the chain ID as
/// an extra field in the hash.
fn compute_hash_pre_0_7(header: &Header, chain_id: &ChainId) -> Felt {
    Pedersen::hash_array(&[
        header.number.into(), // block number
        header.state_root,    // global state root
        Felt::ZERO,           // sequencer address (zero for pre-0.7)
        Felt::ZERO,           // block timestamp (zero for pre-0.7)
        Felt::from(header.transaction_count),
        header.transactions_commitment,
        Felt::ZERO,    // number of events (zero for pre-0.7)
        Felt::ZERO,    // event commitment (zero for pre-0.7)
        Felt::ZERO,    // protocol version
        Felt::ZERO,    // extra data
        chain_id.id(), // chain id (extra field in pre-0.7)
        header.parent_hash,
    ])
}

/// Pre-0.13.2 block hash using Pedersen hash chain.
///
/// Used for blocks with `0.7 <= starknet_version < 0.13.2`.
/// Protocol version and extra data fields are set to `Felt::ZERO`.
fn compute_hash_pre_0_13_2(header: &Header) -> Felt {
    Pedersen::hash_array(&[
        header.number.into(),
        header.state_root,
        header.sequencer_address.into(),
        header.timestamp.into(),
        Felt::from(header.transaction_count),
        header.transactions_commitment,
        Felt::from(header.events_count),
        header.events_commitment,
        Felt::ZERO, // protocol version
        Felt::ZERO, // extra data
        header.parent_hash,
    ])
}

/// Post-0.13.2 (v0) block hash using Poseidon with `STARKNET_BLOCK_HASH0`.
///
/// Used for blocks with `0.13.2 <= starknet_version < 0.13.4`.
/// Gas prices are included as individual fields.
fn compute_hash_post_0_13_2(header: &Header, version_str: &str) -> Felt {
    const BLOCK_HASH_VERSION: ShortString = ShortString::from_ascii("STARKNET_BLOCK_HASH0");

    let concat = Header::concat_counts(
        header.transaction_count,
        header.events_count,
        header.state_diff_length,
        header.l1_da_mode,
    );

    Poseidon::hash_array(&[
        BLOCK_HASH_VERSION.into(),
        header.number.into(),
        header.state_root,
        header.sequencer_address.into(),
        header.timestamp.into(),
        concat,
        header.state_diff_commitment,
        header.transactions_commitment,
        header.events_commitment,
        header.receipts_commitment,
        header.l1_gas_prices.eth.get().into(),
        header.l1_gas_prices.strk.get().into(),
        header.l1_data_gas_prices.eth.get().into(),
        header.l1_data_gas_prices.strk.get().into(),
        cairo_short_string_to_felt(version_str).unwrap(),
        Felt::ZERO,
        header.parent_hash,
    ])
}

/// Post-0.13.4 (v1) block hash using Poseidon with `STARKNET_BLOCK_HASH1`.
///
/// Used for blocks with `starknet_version >= 0.13.4`.
/// Gas prices are consolidated into a single Poseidon hash.
fn compute_hash_post_0_13_4(header: &Header, version_str: &str) -> Felt {
    const BLOCK_HASH_VERSION: ShortString = ShortString::from_ascii("STARKNET_BLOCK_HASH1");

    let concat = Header::concat_counts(
        header.transaction_count,
        header.events_count,
        header.state_diff_length,
        header.l1_da_mode,
    );

    let gas_prices_hash = Poseidon::hash_array(&[
        header.l1_gas_prices.eth.get().into(),
        header.l1_gas_prices.strk.get().into(),
        header.l1_data_gas_prices.eth.get().into(),
        header.l1_data_gas_prices.strk.get().into(),
        header.l2_gas_prices.eth.get().into(),
        header.l2_gas_prices.strk.get().into(),
    ]);

    Poseidon::hash_array(&[
        BLOCK_HASH_VERSION.into(),
        header.number.into(),
        header.state_root,
        header.sequencer_address.into(),
        header.timestamp.into(),
        concat,
        header.state_diff_commitment,
        header.transactions_commitment,
        header.events_commitment,
        header.receipts_commitment,
        gas_prices_hash,
        cairo_short_string_to_felt(version_str).unwrap(),
        Felt::ZERO,
        header.parent_hash,
    ])
}
