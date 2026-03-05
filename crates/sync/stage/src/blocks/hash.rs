//! Version-aware block hash computation for synced blocks.
//!
//! The Starknet block hash algorithm changed across protocol versions:
//!
//! ## Pre-0.7
//!
//! ```text
//! Pedersen(
//!     block_number,
//!     state_root,
//!     0,                          // sequencer_address (zero)
//!     0,                          // timestamp (zero)
//!     transaction_count,
//!     transaction_commitment,
//!     0,                          // event_count (zero)
//!     0,                          // event_commitment (zero)
//!     0,                          // protocol_version
//!     0,                          // extra_data
//!     chain_id,
//!     parent_hash,
//! )
//! ```
//!
//! ## 0.7 to pre-0.13.2
//!
//! ```text
//! Pedersen(
//!     block_number,
//!     state_root,
//!     sequencer_address,
//!     timestamp,
//!     transaction_count,
//!     transaction_commitment,
//!     event_count,
//!     event_commitment,
//!     0,                          // protocol_version
//!     0,                          // extra_data
//!     parent_hash,
//! )
//! ```
//!
//! ## Post-0.13.2 (v0)
//!
//! ```text
//! Poseidon(
//!     "STARKNET_BLOCK_HASH0",
//!     block_number,
//!     state_root,
//!     sequencer_address,
//!     timestamp,
//!     concat(tx_count, event_count, state_diff_length, l1_da_mode),
//!     state_diff_commitment,
//!     transaction_commitment,
//!     event_commitment,
//!     receipt_commitment,
//!     gas_price_wei,
//!     gas_price_fri,
//!     data_gas_price_wei,
//!     data_gas_price_fri,
//!     protocol_version,
//!     0,
//!     parent_hash,
//! )
//! ```
//!
//! ## Post-0.13.4 (v1)
//!
//! ```text
//! gas_prices_hash = Poseidon(
//!     "STARKNET_GAS_PRICES0",
//!     gas_price_wei, gas_price_fri,
//!     data_gas_price_wei, data_gas_price_fri,
//!     l2_gas_price_wei, l2_gas_price_fri,
//! )
//!
//! Poseidon(
//!     "STARKNET_BLOCK_HASH1",
//!     block_number,
//!     state_root,
//!     sequencer_address,
//!     timestamp,
//!     concat(tx_count, event_count, state_diff_length, l1_da_mode),
//!     state_diff_commitment,
//!     transaction_commitment,
//!     event_commitment,
//!     receipt_commitment,
//!     gas_prices_hash,
//!     protocol_version,
//!     0,
//!     parent_hash,
//! )
//! ```
//!
//! Reference implementation (v0 and v1):-
//! * <https://github.com/starkware-libs/sequencer/blob/e3be9f1a0f3514e989f5b6d753022f6ef7bf5b1d/crates/starknet_api/src/block_hash/block_hash_calculator.rs#L219-L256>
//! * <https://github.com/starkware-libs/sequencer/blob/e3be9f1a0f3514e989f5b6d753022f6ef7bf5b1d/crates/starknet_api/src/block_hash/block_hash_calculator.rs#L383-L409>

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

    // See module-level docs for the gas prices hash pseudocode.
    const GAS_PRICES_VERSION: ShortString = ShortString::from_ascii("STARKNET_GAS_PRICES0");
    let gas_prices_hash = Poseidon::hash_array(&[
        GAS_PRICES_VERSION.into(),
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

#[cfg(test)]
mod tests {
    use katana_primitives::block::{GasPrices, Header};
    use katana_primitives::chain::ChainId;
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::version::StarknetVersion;
    use katana_primitives::Felt;

    use super::compute_hash;

    /// Parses a gateway block fixture JSON and returns a (Header, expected_block_hash) pair.
    ///
    /// `version_override` is used for blocks that predate the `starknet_version` field.
    fn header_from_fixture(
        json: &str,
        version_override: Option<StarknetVersion>,
    ) -> (Header, Felt) {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();

        let block_hash = Felt::from_hex(v["block_hash"].as_str().unwrap()).unwrap();

        let parent_hash = Felt::from_hex(
            v.get("parent_block_hash").or_else(|| v.get("parent_hash")).unwrap().as_str().unwrap(),
        )
        .unwrap();

        let number = v["block_number"].as_u64().unwrap_or(0);
        let state_root = felt_or_zero(&v, "state_root");
        let timestamp = v["timestamp"].as_u64().unwrap();

        let sequencer_address = felt_or_zero(&v, "sequencer_address");
        let transaction_commitment = felt_or_zero(&v, "transaction_commitment");
        let event_commitment = felt_or_zero(&v, "event_commitment");
        let state_diff_commitment = felt_or_zero(&v, "state_diff_commitment");
        let receipts_commitment = felt_or_zero(&v, "receipt_commitment");

        let state_diff_length =
            v.get("state_diff_length").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let transaction_count = v["transactions"].as_array().unwrap().len() as u32;

        // Count total events from transaction_receipts.
        let events_count = v
            .get("transaction_receipts")
            .and_then(|r| r.as_array())
            .map(|receipts| {
                receipts
                    .iter()
                    .filter_map(|r| r.get("events"))
                    .filter_map(|e| e.as_array())
                    .map(|e| e.len() as u32)
                    .sum()
            })
            .unwrap_or(0);

        let l1_da_mode = match v["l1_da_mode"].as_str().unwrap_or("CALLDATA") {
            "BLOB" => L1DataAvailabilityMode::Blob,
            _ => L1DataAvailabilityMode::Calldata,
        };

        let starknet_version = version_override.unwrap_or_else(|| {
            v.get("starknet_version")
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| StarknetVersion::parse(s).unwrap())
                .unwrap_or(StarknetVersion::UNVERSIONED)
        });

        let l1_gas_prices = parse_gas_prices(&v["l1_gas_price"]);
        let l1_data_gas_prices = parse_gas_prices(&v["l1_data_gas_price"]);
        let l2_gas_prices =
            v.get("l2_gas_price").map(parse_gas_prices).unwrap_or(GasPrices::default());

        let header = Header {
            number,
            timestamp,
            state_root,
            l1_da_mode,
            events_count,
            transaction_count,
            state_diff_length,
            l1_gas_prices,
            l1_data_gas_prices,
            l2_gas_prices,
            starknet_version,
            parent_hash: parent_hash.into(),
            sequencer_address: sequencer_address.into(),
            transactions_commitment: transaction_commitment,
            events_commitment: event_commitment,
            state_diff_commitment,
            receipts_commitment,
        };

        (header, block_hash)
    }

    fn felt_or_zero(v: &serde_json::Value, key: &str) -> Felt {
        v.get(key)
            .and_then(|f| f.as_str())
            .map(|s| Felt::from_hex(s).unwrap())
            .unwrap_or(Felt::ZERO)
    }

    /// Parses a gas price object `{ "price_in_wei": "0x...", "price_in_fri": "0x..." }`
    /// into `GasPrices`, replacing zero values with 1 (matching `extract_block_data`).
    fn parse_gas_prices(v: &serde_json::Value) -> GasPrices {
        let wei = hex_to_u128(v["price_in_wei"].as_str().unwrap());
        let fri = hex_to_u128(v["price_in_fri"].as_str().unwrap());
        let wei = if wei == 0 { 1 } else { wei };
        let fri = if fri == 0 { 1 } else { fri };
        unsafe { GasPrices::new_unchecked(wei, fri) }
    }

    fn hex_to_u128(s: &str) -> u128 {
        u128::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
    }

    // NOTE: Pre-0.7 (block 0) and 0.7.0 (block 2240) are not tested because
    // the gateway returns 0x0 for transaction_commitment and event_commitment
    // on these very old blocks — the actual values needed for hash verification
    // were never stored.

    /// Block 65000 — version 0.11.1, Pedersen with real event commitments.
    #[test]
    fn block_hash_mainnet_65000_v0_11_1() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.11.1/block/mainnet_65000.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected, "block 65000 (v0.11.1) hash mismatch");
    }

    /// Block 550000 — version 0.13.0, last Pedersen-era version before Poseidon switch.
    #[test]
    fn block_hash_mainnet_550000_v0_13_0() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.13.0/block/mainnet_550000.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected, "block 550000 (v0.13.0) hash mismatch");
    }

    /// Sepolia integration block 35748 — version 0.13.2, Poseidon v0.
    #[test]
    fn block_hash_sepolia_35748_v0_13_2() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.13.2/block/sepolia_integration_35748.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected, "sepolia block 35748 (v0.13.2) hash mismatch");
    }

    /// Sepolia integration block 63881 — version 0.13.4, Poseidon v1.
    #[test]
    fn block_hash_sepolia_63881_v0_13_4() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.13.4/block/sepolia_integration_63881.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected, "sepolia block 63881 (v0.13.4) hash mismatch");
    }

    /// Sepolia block 2473486 — version 0.14.0, Poseidon v1 (sepolia).
    #[test]
    fn block_hash_sepolia_2473486_v0_14_0() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.14.0/block/sepolia_2473486.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected, "sepolia block 2473486 (v0.14.0) hash mismatch");
    }

    /// Block 2238855 — version 0.14.0, Poseidon v1 with consolidated gas prices.
    #[test]
    fn block_hash_mainnet_2238855_v0_14_0() {
        let json = include_str!(concat!(
            "../../../../gateway/gateway-client/tests/fixtures",
            "/0.14.0/block/mainnet_2238855.json"
        ));
        let (header, expected) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected, "block 2238855 (v0.14.0) hash mismatch");
    }
}
