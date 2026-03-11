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

use katana_primitives::block::{Header, SealedBlock};
use katana_primitives::cairo::ShortString;
use katana_primitives::chain::ChainId;
use katana_primitives::receipt::{Event, Receipt};
use katana_primitives::state::{compute_state_diff_hash, StateUpdates};
use katana_primitives::transaction::{DeclareTx, DeployAccountTx, InvokeTx, Tx, TxWithHash};
use katana_primitives::utils::starknet_keccak;
use katana_primitives::version::StarknetVersion;
use katana_primitives::Felt;
use katana_trie::compute_merkle_root;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

const STARKNET_VERSION_0_11_1: StarknetVersion = StarknetVersion::new([0, 11, 1, 0]);
const STARKNET_VERSION_0_13_4: StarknetVersion = StarknetVersion::new([0, 13, 4, 0]);

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

/// Computes block commitments that some synced block sources omit from the header.
///
/// Version gates:
/// - Transaction commitment: reconstructed for any version when the header field is zero.
/// - Event commitment: reconstructed for any version when the header field is zero.
/// - State diff commitment and length: only meaningful from `0.13.2` onward.
/// - Receipt commitment: only meaningful from `0.13.2` onward.
pub(crate) fn compute_missing_commitments(
    block: &mut SealedBlock,
    receipts: &[Receipt],
    state_updates: &StateUpdates,
) {
    let version = block.header.starknet_version;

    if block.header.transactions_commitment == Felt::ZERO {
        block.header.transactions_commitment = compute_transaction_commitment(&block.body, version);
    }

    if block.header.events_commitment == Felt::ZERO {
        block.header.events_commitment = compute_event_commitment(&block.body, receipts, version);
    }

    // Post-0.13.2 block hashes include `state_diff_length` and `state_diff_commitment`.
    // The commitment itself is the canonical Poseidon hash implemented in
    // `katana_primitives::state::compute_state_diff_hash`.
    if version >= StarknetVersion::V0_13_2
        && (block.header.state_diff_length == 0 || block.header.state_diff_commitment == Felt::ZERO)
    {
        block.header.state_diff_length =
            u32::try_from(state_updates.len()).expect("state diff length overflow");
        block.header.state_diff_commitment = compute_state_diff_hash(state_updates.clone());
    }

    if version >= StarknetVersion::V0_13_2 && block.header.receipts_commitment == Felt::ZERO {
        block.header.receipts_commitment = compute_receipt_commitment(&block.body, receipts);
    }
}

/// Computes the transaction commitment root for a block.
///
/// The root is always a height-64 Patricia Merkle tree keyed by transaction index.
/// The leaf algorithm depends on Starknet version:
/// - `< 0.11.1`: Pedersen root and Pedersen leaf; only invoke transactions carry signatures.
/// - `0.11.1 .. 0.13.2`: Pedersen root and Pedersen leaf; declare and deploy-account signatures are
///   also part of the leaf.
/// - `0.13.2 .. 0.13.4`: Poseidon root and Poseidon leaf; empty signatures are encoded as `[0]`.
/// - `>= 0.13.4`: Poseidon root and Poseidon leaf; empty signatures remain empty.
fn compute_transaction_commitment(transactions: &[TxWithHash], version: StarknetVersion) -> Felt {
    let leaves = transactions
        .iter()
        .map(|tx| calculate_transaction_commitment_leaf(tx, version))
        .collect::<Vec<_>>();

    if version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(&leaves).unwrap()
    } else {
        compute_merkle_root::<Poseidon>(&leaves).unwrap()
    }
}

/// Computes the versioned transaction-commitment leaf for a single transaction.
fn calculate_transaction_commitment_leaf(tx: &TxWithHash, version: StarknetVersion) -> Felt {
    if version < STARKNET_VERSION_0_11_1 {
        let tx_signature = transaction_signature_pre_0_11_1(&tx.transaction);
        let signature_hash = Pedersen::hash_array(tx_signature);

        Pedersen::hash(&tx.hash, &signature_hash)
    } else if version < StarknetVersion::V0_13_2 {
        let tx_signature = transaction_signature(&tx.transaction);
        let signature_hash = Pedersen::hash_array(tx_signature);

        Pedersen::hash(&tx.hash, &signature_hash)
    } else {
        let signature = transaction_signature(&tx.transaction);
        let mut elements = Vec::with_capacity(signature.len() + 1);
        elements.push(tx.hash);

        if version < STARKNET_VERSION_0_13_4 && signature.is_empty() {
            elements.push(Felt::ZERO);
        } else {
            elements.extend(signature.iter().copied());
        }

        Poseidon::hash_array(&elements)
    }
}

fn transaction_signature_pre_0_11_1(transaction: &Tx) -> &[Felt] {
    match transaction {
        Tx::Invoke(InvokeTx::V0(tx)) => &tx.signature,
        Tx::Invoke(InvokeTx::V1(tx)) => &tx.signature,
        Tx::Invoke(InvokeTx::V3(tx)) => &tx.signature,
        Tx::Declare(_) | Tx::Deploy(_) | Tx::DeployAccount(_) | Tx::L1Handler(_) => &[],
    }
}

fn transaction_signature(transaction: &Tx) -> &[Felt] {
    match transaction {
        Tx::Invoke(InvokeTx::V0(tx)) => &tx.signature,
        Tx::Invoke(InvokeTx::V1(tx)) => &tx.signature,
        Tx::Invoke(InvokeTx::V3(tx)) => &tx.signature,
        Tx::Declare(DeclareTx::V0(tx)) => &tx.signature,
        Tx::Declare(DeclareTx::V1(tx)) => &tx.signature,
        Tx::Declare(DeclareTx::V2(tx)) => &tx.signature,
        Tx::Declare(DeclareTx::V3(tx)) => &tx.signature,
        Tx::DeployAccount(DeployAccountTx::V1(tx)) => &tx.signature,
        Tx::DeployAccount(DeployAccountTx::V3(tx)) => &tx.signature,
        Tx::Deploy(_) | Tx::L1Handler(_) => &[],
    }
}

/// Computes the receipt commitment root for post-`0.13.2` blocks.
///
/// The root is a height-64 Poseidon Patricia tree keyed by transaction index.
/// Each leaf is:
///
/// ```text
/// Poseidon(
///     tx_hash,
///     actual_fee,
///     messages_hash,
///     starknet_keccak(revert_reason) | 0,
///     0,              // l2 gas, kept zero for the historical block hash formula
///     l1_gas,
///     l1_data_gas,
/// )
/// ```
fn compute_receipt_commitment(transactions: &[TxWithHash], receipts: &[Receipt]) -> Felt {
    let leaves = transactions
        .iter()
        .zip(receipts.iter())
        .map(|(tx, receipt)| calculate_receipt_commitment_leaf(receipt, tx.hash))
        .collect::<Vec<_>>();

    compute_merkle_root::<Poseidon>(&leaves).unwrap()
}

/// Computes the receipt-commitment leaf used from `0.13.2` onward.
fn calculate_receipt_commitment_leaf(receipt: &Receipt, tx_hash: Felt) -> Felt {
    let resources = receipt.resources_used();

    Poseidon::hash_array(&[
        tx_hash,
        receipt.fee().overall_fee.into(),
        calculate_messages_to_l1_hash(receipt),
        receipt
            .revert_reason()
            .map(|reason| starknet_keccak(reason.as_bytes()))
            .unwrap_or(Felt::ZERO),
        Felt::ZERO,
        Felt::from(resources.total_gas_consumed.l1_gas),
        Felt::from(resources.total_gas_consumed.l1_data_gas),
    ])
}

/// Computes the flattened Poseidon hash of all L2-to-L1 messages in a receipt.
///
/// The sequence is:
/// `[message_count, from, to, payload_len, payload..., from, to, payload_len, payload..., ...]`.
fn calculate_messages_to_l1_hash(receipt: &Receipt) -> Felt {
    let mut elements: Vec<Felt> = Vec::new();

    elements.push(receipt.messages_sent().len().into());

    for message in receipt.messages_sent() {
        elements.push(message.from_address.into());
        elements.push(message.to_address);
        elements.push(message.payload.len().into());

        for payload in &message.payload {
            elements.push(*payload);
        }
    }

    Poseidon::hash_array(&elements)
}

/// Computes the event commitment root for a block.
///
/// The root is a height-64 Patricia tree keyed by event index:
/// - `< 0.13.2`: Pedersen root over Pedersen event leaves.
/// - `>= 0.13.2`: Poseidon root over Poseidon event leaves that also include the transaction hash.
fn compute_event_commitment(
    transactions: &[TxWithHash],
    receipts: &[Receipt],
    version: StarknetVersion,
) -> Felt {
    let leaves = transactions
        .iter()
        .zip(receipts.iter())
        .flat_map(|(tx, receipt)| {
            receipt
                .events()
                .iter()
                .map(move |event| calculate_event_commitment_leaf(event, tx.hash, version))
        })
        .collect::<Vec<_>>();

    if version < StarknetVersion::V0_13_2 {
        compute_merkle_root::<Pedersen>(&leaves).unwrap()
    } else {
        compute_merkle_root::<Poseidon>(&leaves).unwrap()
    }
}

/// Computes the versioned event-commitment leaf for a single event.
///
/// Versions:
/// - `< 0.13.2`: `Pedersen(from_address, Pedersen(keys), Pedersen(data))`
/// - `>= 0.13.2`: Poseidon chain hash of `[from_address, tx_hash, len(keys), keys..., len(data),
///   data...]`
fn calculate_event_commitment_leaf(event: &Event, tx_hash: Felt, version: StarknetVersion) -> Felt {
    if version < StarknetVersion::V0_13_2 {
        let keys_hash = Pedersen::hash_array(&event.keys);
        let data_hash = Pedersen::hash_array(&event.data);

        Pedersen::hash_array(&[event.from_address.into(), keys_hash, data_hash])
    } else {
        let mut elements: Vec<Felt> = Vec::new();
        elements.push(event.from_address.into());
        elements.push(tx_hash);
        elements.push(event.keys.len().into());

        for key in &event.keys {
            elements.push(*key);
        }

        elements.push(event.data.len().into());

        for data in &event.data {
            elements.push(*data);
        }

        Poseidon::hash_array(&elements)
    }
}

#[cfg(test)]
mod tests {
    use katana_gateway_types::{Block, ConfirmedStateUpdate, StateUpdate, StateUpdateWithBlock};
    use katana_primitives::block::{GasPrices, Header};
    use katana_primitives::chain::ChainId;
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::transaction::{DeployTx, InvokeTxV0, Tx, TxWithHash};
    use katana_primitives::version::StarknetVersion;
    use katana_primitives::Felt;
    use starknet_types_core::hash::{Pedersen, StarkHash};

    use super::{calculate_transaction_commitment_leaf, compute_hash, compute_missing_commitments};
    use crate::blocks::BlockData;

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

    #[test]
    fn pre_0_11_1_non_invoke_transactions_use_the_empty_signature_hash() {
        let tx_hash = Felt::from_hex("0x1234").unwrap();
        let tx = TxWithHash {
            hash: tx_hash,
            transaction: Tx::Deploy(DeployTx {
                contract_address: Felt::ZERO,
                contract_address_salt: Felt::ZERO,
                constructor_calldata: Vec::new(),
                class_hash: Felt::ZERO,
                version: Felt::ZERO,
            }),
        };

        let expected = Pedersen::hash(&tx_hash, &Pedersen::hash_array(&[]));
        let actual = calculate_transaction_commitment_leaf(&tx, StarknetVersion::UNVERSIONED);

        assert_eq!(actual, expected);
    }

    #[test]
    fn pre_0_11_1_invoke_transactions_hash_the_signature() {
        let tx_hash = Felt::from_hex("0x5678").unwrap();
        let signature = vec![Felt::ONE, Felt::TWO, Felt::THREE];
        let tx = TxWithHash {
            hash: tx_hash,
            transaction: Tx::Invoke(katana_primitives::transaction::InvokeTx::V0(InvokeTxV0 {
                signature: signature.clone(),
                ..Default::default()
            })),
        };

        let expected = Pedersen::hash(&tx_hash, &Pedersen::hash_array(&signature));
        let actual = calculate_transaction_commitment_leaf(&tx, StarknetVersion::UNVERSIONED);

        assert_eq!(actual, expected);
    }

    #[test]
    fn reconstructs_missing_commitments_for_unversioned_mainnet_770() {
        let (mut block_data, ..) = block_data_from_split_fixtures(
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/pre_0.7.0/block/mainnet_770.json"
            )),
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/pre_0.7.0/state_update/mainnet_770.json"
            )),
        );

        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;

        compute_missing_commitments(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            Felt::from_hex("0x51aad3267df44940cbdf4054b5a4e32ed0ba5e9ef02d9f15010374e3649dcc4",)
                .unwrap()
        );
        assert_eq!(block_data.block.block.header.events_commitment, Felt::ZERO);
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_0_block() {
        let (mut block_data, block_fixture) = block_data_from_split_fixtures(
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.0/block/mainnet_550000.json"
            )),
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.0/state_update/mainnet_550000.json"
            )),
        );

        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;

        compute_missing_commitments(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            felt_field_from_block_fixture(&block_fixture, "transaction_commitment")
        );
        assert_eq!(
            block_data.block.block.header.events_commitment,
            felt_field_from_block_fixture(&block_fixture, "event_commitment")
        );
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_2_block() {
        let (mut block_data, block_fixture) = block_data_from_split_fixtures(
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.2/block/sepolia_integration_35748.json"
            )),
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.2/state_update/sepolia_integration_35748.json"
            )),
        );

        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;
        block_data.block.block.header.receipts_commitment = Felt::ZERO;
        block_data.block.block.header.state_diff_length = 0;
        block_data.block.block.header.state_diff_commitment = Felt::ZERO;

        compute_missing_commitments(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            felt_field_from_block_fixture(&block_fixture, "transaction_commitment")
        );
        assert_eq!(
            block_data.block.block.header.events_commitment,
            felt_field_from_block_fixture(&block_fixture, "event_commitment")
        );
        assert_eq!(
            block_data.block.block.header.receipts_commitment,
            felt_field_from_block_fixture(&block_fixture, "receipt_commitment")
        );
        assert_eq!(
            block_data.block.block.header.state_diff_commitment,
            felt_field_from_block_fixture(&block_fixture, "state_diff_commitment")
        );
        assert_eq!(
            block_data.block.block.header.state_diff_length,
            u32_field_from_block_fixture(&block_fixture, "state_diff_length")
        );
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_4_block() {
        let (mut block_data, block_fixture) = block_data_from_split_fixtures(
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.4/block/sepolia_integration_63881.json"
            )),
            include_str!(concat!(
                "../../../../gateway/gateway-client/tests/fixtures",
                "/0.13.4/state_update/sepolia_integration_63881.json"
            )),
        );

        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;
        block_data.block.block.header.receipts_commitment = Felt::ZERO;
        block_data.block.block.header.state_diff_length = 0;
        block_data.block.block.header.state_diff_commitment = Felt::ZERO;

        compute_missing_commitments(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            felt_field_from_block_fixture(&block_fixture, "transaction_commitment")
        );
        assert_eq!(
            block_data.block.block.header.events_commitment,
            felt_field_from_block_fixture(&block_fixture, "event_commitment")
        );
        assert_eq!(
            block_data.block.block.header.receipts_commitment,
            felt_field_from_block_fixture(&block_fixture, "receipt_commitment")
        );
        assert_eq!(
            block_data.block.block.header.state_diff_commitment,
            felt_field_from_block_fixture(&block_fixture, "state_diff_commitment")
        );
        assert_eq!(
            block_data.block.block.header.state_diff_length,
            u32_field_from_block_fixture(&block_fixture, "state_diff_length")
        );
    }

    fn block_data_from_split_fixtures(
        block_json: &str,
        state_update_json: &str,
    ) -> (BlockData, serde_json::Value) {
        let block_fixture = parse_block_fixture(block_json);
        let mut block_for_parse = block_fixture.clone();
        normalize_legacy_block_fixture(&mut block_for_parse);
        let block = serde_json::from_value::<Block>(block_for_parse).unwrap();
        let state_update = serde_json::from_str::<ConfirmedStateUpdate>(state_update_json).unwrap();
        let block_data = BlockData::from(StateUpdateWithBlock {
            block,
            state_update: StateUpdate::Confirmed(state_update),
        });

        (block_data, block_fixture)
    }

    fn parse_block_fixture(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    fn normalize_legacy_block_fixture(block: &mut serde_json::Value) {
        let Some(receipts) =
            block.get_mut("transaction_receipts").and_then(serde_json::Value::as_array_mut)
        else {
            return;
        };

        for receipt in receipts {
            let Some(total_gas_consumed) = receipt
                .get_mut("execution_resources")
                .and_then(|resources| resources.get_mut("total_gas_consumed"))
                .and_then(serde_json::Value::as_object_mut)
            else {
                continue;
            };

            total_gas_consumed
                .entry("l2_gas".to_owned())
                .or_insert_with(|| serde_json::Value::from(0));
        }
    }

    fn felt_field_from_block_fixture(block: &serde_json::Value, field: &str) -> Felt {
        Felt::from_hex(block[field].as_str().unwrap()).unwrap()
    }

    fn u32_field_from_block_fixture(block: &serde_json::Value, field: &str) -> u32 {
        block[field].as_u64().unwrap().try_into().unwrap()
    }
}
