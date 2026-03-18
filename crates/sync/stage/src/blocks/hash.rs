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

use katana_primitives::block::{BlockNumber, Header, SealedBlock};
use katana_primitives::cairo::ShortString;
use katana_primitives::chain::ChainId;
use katana_primitives::receipt::{Event, Receipt};
use katana_primitives::state::{compute_state_diff_hash, StateUpdates};
use katana_primitives::transaction::{DeclareTx, DeployAccountTx, InvokeTx, Tx, TxWithHash};
use katana_primitives::utils::starknet_keccak;
use katana_primitives::version::StarknetVersion;
use katana_primitives::{address, ContractAddress, Felt};
use katana_trie::compute_merkle_root;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

/// Fallback sequencer address used for mainnet blocks in the 0.7–0.8.2 range.
///
/// During this period, the sequencer address was not stored in the block header. The actual
/// address must be injected when computing the block hash for these blocks.
const MAINNET_FALLBACK_SEQUENCER_ADDRESS: ContractAddress =
    address!("0x021f4b90b0377c82bf330b7b5295820769e72d79d8acd0effa0ebde6e9988bc5");

/// Patches the header for missing fields in the block header (earlier Starknet blocks omit some
/// fields) and computes the block hash.
pub fn patch_and_verify_block_hash(
    block: &mut SealedBlock,
    receipts: &[Receipt],
    state_updates: &StateUpdates,
    chain_id: &ChainId,
) -> bool {
    let expected_hash = block.hash;

    patch_missing_fields(block, receipts, state_updates, chain_id);
    let computed_hash = compute_hash(&block.header, chain_id);

    if expected_hash != computed_hash {
        block.header.sequencer_address = MAINNET_FALLBACK_SEQUENCER_ADDRESS;
        let updated_computed_hash = compute_hash(&block.header, chain_id);
        updated_computed_hash == expected_hash
    } else {
        true
    }
}

/// Computes the block hash for a header, dispatching to the correct algorithm based
/// on the block's protocol version.
///
/// The `chain_id` is required for pre-0.7 blocks which include the chain ID in the hash.
///
/// The `expected_hash` is used to resolve ambiguity for early mainnet blocks where the
/// sequencer address was not stored in the header. When the initial hash doesn't match,
/// the known fallback sequencer address from the 0.7–0.8.2 era is tried.
pub fn compute_hash(header: &Header, chain_id: &ChainId) -> Felt {
    let version_str = header.starknet_version.to_string();

    // [0, 0.7.0)
    if header.starknet_version < StarknetVersion::V0_7_0 {
        compute_hash_pre_0_7(header, chain_id)
    }
    // [0.7.0, 0.13.2)
    else if header.starknet_version < StarknetVersion::V0_13_2 {
        compute_hash_pre_0_13_2(header)
    }
    // [0.13.2, 0.13.4)
    else if header.starknet_version < StarknetVersion::V0_13_4 {
        compute_hash_post_0_13_2(header, &version_str)
    }
    // [0.13.4, ..)
    else {
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

/// Fills in header fields and commitments that some synced block sources omit.
///
/// This handles two categories of missing data:
///
/// **Header fields** — Early blocks don't populate `starknet_version` and `sequencer_address`.
/// For blocks on or after the 0.7 cutoff on mainnet, the version is set to `V0_7_0`. Pre-0.7
/// blocks remain `UNVERSIONED`.
///
/// **Commitments** — Version gates:
/// - Transaction commitment: reconstructed for any version when the header field is zero.
/// - Event commitment: reconstructed for any version when the header field is zero.
/// - State diff commitment and length: only meaningful from `0.13.2` onward.
/// - Receipt commitment: only meaningful from `0.13.2` onward.
pub(crate) fn patch_missing_fields(
    block: &mut SealedBlock,
    receipts: &[Receipt],
    state_updates: &StateUpdates,
    chain_id: &ChainId,
) {
    // Patch starknet_version for early blocks that don't populate it.
    if block.header.starknet_version == StarknetVersion::UNVERSIONED {
        block.header.starknet_version = effective_version(&block.header, chain_id);
    }

    let version = block.header.starknet_version;

    if block.header.transactions_commitment == Felt::ZERO {
        block.header.transactions_commitment = compute_transaction_commitment(&block.body, version);
    }

    if block.header.events_commitment == Felt::ZERO {
        block.header.events_commitment = compute_event_commitment(&block.body, receipts, version);
    }

    // The rest of the commitments are only meaningful from 0.13.2 onward.
    if version < StarknetVersion::V0_13_2 {
        return;
    }

    // For early mainnet blocks, the sequencer address was not stored in the header.
    //
    // Early sepolia blocks (>= block 0) already have a non-zero sequencer address.
    if block.header.sequencer_address == ContractAddress::ZERO && chain_id == &ChainId::MAINNET {
        block.header.sequencer_address = MAINNET_FALLBACK_SEQUENCER_ADDRESS;
    }

    // Post-0.13.2 block hashes include `state_diff_length` and `state_diff_commitment`.
    // The commitment itself is the canonical Poseidon hash implemented in
    // `katana_primitives::state::compute_state_diff_hash`.
    if block.header.state_diff_length == 0 || block.header.state_diff_commitment == Felt::ZERO {
        let state_diff_length = state_updates.len();
        block.header.state_diff_length = u32::try_from(state_diff_length).unwrap();
        block.header.state_diff_commitment = compute_state_diff_hash(state_updates.clone());
    }

    // event commitment is not computed in <= 0.13.2 blocks, so we can safely skip this
    if block.header.receipts_commitment == Felt::ZERO {
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
    if version < StarknetVersion::V0_13_2 {
        let tx_signature = transaction_signature(&tx.transaction);
        let signature_hash = Pedersen::hash_array(tx_signature);

        Pedersen::hash(&tx.hash, &signature_hash)
    } else {
        let signature = transaction_signature(&tx.transaction);
        let mut elements = Vec::with_capacity(signature.len() + 1);
        elements.push(tx.hash);

        if version < StarknetVersion::V0_13_4 && signature.is_empty() {
            elements.push(Felt::ZERO);
        } else {
            elements.extend(signature.iter().copied());
        }

        Poseidon::hash_array(&elements)
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
    let revert_reason = receipt
        .revert_reason()
        .map(|reason| starknet_keccak(reason.as_bytes()))
        .unwrap_or(Felt::ZERO);

    Poseidon::hash_array(&[
        tx_hash,
        receipt.fee().overall_fee.into(),
        calculate_messages_to_l1_hash(receipt),
        revert_reason,
        Felt::ZERO,
        resources.total_gas_consumed.l1_gas.into(),
        resources.total_gas_consumed.l1_data_gas.into(),
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

/// Resolves the effective starknet version for hash computation.
///
/// Early mainnet blocks lack the `starknet_version` header field. For these,
/// we infer the version from the block number using known mainnet cutoffs.
fn effective_version(header: &Header, chain_id: &ChainId) -> StarknetVersion {
    /// The first block on mainnet that uses the 0.7 hash algorithm.
    ///
    /// Blocks before this number use the pre-0.7 algorithm which includes chain_id and zeros
    /// for sequencer/timestamp. The `starknet_version` header field was not populated for
    /// early blocks, so we cannot rely on it alone for the algorithm selection.
    ///
    /// Reference: <https://github.com/eqlabs/pathfinder/blob/main/crates/pathfinder/src/state/block_hash.rs>
    const MAINNET_FIRST_0_7_BLOCK: BlockNumber = 833;

    if header.starknet_version != StarknetVersion::UNVERSIONED {
        return header.starknet_version;
    }

    // Only mainnet has pre-0.7 blocks; other networks start at 0.7+.
    if *chain_id == ChainId::MAINNET && header.number < MAINNET_FIRST_0_7_BLOCK {
        StarknetVersion::UNVERSIONED
    }
    // Treat unversioned blocks on or after the cutoff as 0.7.
    else {
        StarknetVersion::V0_7_0
    }
}

#[cfg(test)]
mod tests {
    use katana_gateway_types::{Block, ConfirmedStateUpdate, StateUpdate, StateUpdateWithBlock};
    use katana_primitives::block::{GasPrices, Header};
    use katana_primitives::chain::ChainId;
    use katana_primitives::version::StarknetVersion;
    use katana_primitives::{felt, Felt};
    use num_traits::ToPrimitive;

    use super::{compute_hash, patch_missing_fields};
    use crate::blocks::hash::patch_and_verify_block_hash;
    use crate::blocks::BlockData;

    /// Shorthand for including a gateway test fixture file at compile time.
    macro_rules! fixture {
        ($path:expr) => {
            include_str!(concat!("../../../../gateway/gateway-client/tests/fixtures/", $path))
        };
    }

    /// Parses a gateway block fixture JSON and returns a (Header, expected_block_hash) pair.
    ///
    /// `version_override` is used for blocks that predate the `starknet_version` field.
    fn header_from_fixture(
        json: &str,
        version_override: Option<StarknetVersion>,
    ) -> (Header, Felt) {
        let mut value = serde_json::from_str::<serde_json::Value>(json).unwrap();
        normalize_legacy_block_fixture(&mut value);
        let block = serde_json::from_value::<Block>(value).unwrap();

        let block_hash = block.block_hash.unwrap();

        let transaction_count = block.transactions.len() as u32;
        let events_count: u32 =
            block.transaction_receipts.iter().map(|r| r.body.events.len() as u32).sum();

        let starknet_version = version_override.unwrap_or_else(|| {
            block
                .starknet_version
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| StarknetVersion::parse(s).unwrap())
                .unwrap_or(StarknetVersion::UNVERSIONED)
        });

        let header = Header {
            transaction_count,
            events_count,
            starknet_version,
            timestamp: block.timestamp,
            l1_da_mode: block.l1_da_mode,
            parent_hash: block.parent_block_hash,
            number: block.block_number.unwrap_or_default(),
            state_root: block.state_root.unwrap_or_default(),
            sequencer_address: block.sequencer_address.unwrap_or_default(),
            transactions_commitment: block.transaction_commitment.unwrap_or_default(),
            events_commitment: block.event_commitment.unwrap_or_default(),
            state_diff_commitment: block.state_diff_commitment.unwrap_or_default(),
            receipts_commitment: block.receipt_commitment.unwrap_or_default(),
            state_diff_length: block.state_diff_length.unwrap_or_default(),
            l1_gas_prices: to_gas_prices(block.l1_gas_price),
            l1_data_gas_prices: to_gas_prices(block.l1_data_gas_price),
            l2_gas_prices: to_gas_prices(block.l2_gas_price),
        };

        (header, block_hash)
    }

    fn to_gas_prices(prices: starknet::core::types::ResourcePrice) -> GasPrices {
        let eth = prices.price_in_wei.to_u128().expect("valid u128");
        let strk = prices.price_in_fri.to_u128().expect("valid u128");
        let eth = if eth == 0 { 1 } else { eth };
        let strk = if strk == 0 { 1 } else { strk };
        unsafe { GasPrices::new_unchecked(eth, strk) }
    }

    // NOTE: Pre-0.7 (block 0) and 0.7.0 (block 2240) are not tested because
    // the gateway returns 0x0 for transaction_commitment and event_commitment
    // on these very old blocks — the actual values needed for hash verification
    // were never stored.

    /// Block 65000 — version 0.11.1, Pedersen with real event commitments.
    #[test]
    fn block_hash_mainnet_65000_v0_11_1() {
        let json = fixture!("0.11.1/block/mainnet_65000.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected_hash, "block 65000 (v0.11.1) hash mismatch");
    }

    /// Block 550000 — version 0.13.0, last Pedersen-era version before Poseidon switch.
    #[test]
    fn block_hash_mainnet_550000_v0_13_0() {
        let json = fixture!("0.13.0/block/mainnet_550000.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected_hash, "block 550000 (v0.13.0) hash mismatch");
    }

    /// Sepolia integration block 35748 — version 0.13.2, Poseidon v0.
    #[test]
    fn block_hash_sepolia_35748_v0_13_2() {
        let json = fixture!("0.13.2/block/sepolia_integration_35748.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected_hash, "sepolia block 35748 (v0.13.2) hash mismatch");
    }

    /// Sepolia integration block 63881 — version 0.13.4, Poseidon v1.
    #[test]
    fn block_hash_sepolia_63881_v0_13_4() {
        let json = fixture!("0.13.4/block/sepolia_integration_63881.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected_hash, "sepolia block 63881 (v0.13.4) hash mismatch");
    }

    /// Sepolia block 2473486 — version 0.14.0, Poseidon v1 (sepolia).
    #[test]
    fn block_hash_sepolia_2473486_v0_14_0() {
        let json = fixture!("0.14.0/block/sepolia_2473486.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::SEPOLIA);
        assert_eq!(hash, expected_hash, "sepolia block 2473486 (v0.14.0) hash mismatch");
    }

    /// Block 2238855 — version 0.14.0, Poseidon v1 with consolidated gas prices.
    #[test]
    fn block_hash_mainnet_2238855_v0_14_0() {
        let json = fixture!("0.14.0/block/mainnet_2238855.json");
        let (header, expected_hash) = header_from_fixture(json, None);
        let hash = compute_hash(&header, &ChainId::MAINNET);
        assert_eq!(hash, expected_hash, "block 2238855 (v0.14.0) hash mismatch");
    }

    #[test]
    fn mainnet_0_7_0_cutoff_block_833() {
        let (mut block_data, _) = block_data_from_split_fixtures(
            fixture!("0.7.0/block/mainnet_833.json"),
            fixture!("0.7.0/state_update/mainnet_833.json"),
        );

        assert!(patch_and_verify_block_hash(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
            &ChainId::MAINNET,
        ));
    }

    #[test]
    fn reconstructs_missing_commitments_for_unversioned_mainnet_770() {
        let (mut block_data, _) = block_data_from_split_fixtures(
            fixture!("pre_0.7.0/block/mainnet_770.json"),
            fixture!("pre_0.7.0/state_update/mainnet_770.json"),
        );

        // reset commitments that may be ommitted by the download source
        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;

        patch_missing_fields(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
            &ChainId::MAINNET,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            felt!("0x51aad3267df44940cbdf4054b5a4e32ed0ba5e9ef02d9f15010374e3649dcc4")
        );
        assert_eq!(block_data.block.block.header.events_commitment, Felt::ZERO);
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_0_block() {
        let (mut block_data, expected) = block_data_from_split_fixtures(
            fixture!("0.13.0/block/mainnet_550000.json"),
            fixture!("0.13.0/state_update/mainnet_550000.json"),
        );

        // reset commitments that may be ommitted by the download source
        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;

        patch_missing_fields(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
            &ChainId::MAINNET,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            expected.transactions_commitment
        );
        assert_eq!(block_data.block.block.header.events_commitment, expected.events_commitment);
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_2_block() {
        let (mut block_data, expected) = block_data_from_split_fixtures(
            fixture!("0.13.2/block/sepolia_integration_35748.json"),
            fixture!("0.13.2/state_update/sepolia_integration_35748.json"),
        );

        // reset all the commitments that may be ommitted by the sync source
        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;
        block_data.block.block.header.receipts_commitment = Felt::ZERO;
        block_data.block.block.header.state_diff_length = 0;
        block_data.block.block.header.state_diff_commitment = Felt::ZERO;

        patch_missing_fields(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
            &ChainId::SEPOLIA,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            expected.transactions_commitment
        );
        assert_eq!(block_data.block.block.header.events_commitment, expected.events_commitment);
        assert_eq!(block_data.block.block.header.receipts_commitment, expected.receipts_commitment);
        assert_eq!(
            block_data.block.block.header.state_diff_commitment,
            expected.state_diff_commitment
        );
        assert_eq!(block_data.block.block.header.state_diff_length, expected.state_diff_length);
    }

    #[test]
    fn reconstructs_missing_commitments_for_0_13_4_block() {
        let (mut block_data, expected) = block_data_from_split_fixtures(
            fixture!("0.13.4/block/sepolia_integration_63881.json"),
            fixture!("0.13.4/state_update/sepolia_integration_63881.json"),
        );

        // reset commitments that may be ommitted by the download source
        block_data.block.block.header.transactions_commitment = Felt::ZERO;
        block_data.block.block.header.events_commitment = Felt::ZERO;
        block_data.block.block.header.receipts_commitment = Felt::ZERO;
        block_data.block.block.header.state_diff_length = 0;
        block_data.block.block.header.state_diff_commitment = Felt::ZERO;

        patch_missing_fields(
            &mut block_data.block.block,
            &block_data.receipts,
            &block_data.state_updates.state_updates,
            &ChainId::SEPOLIA,
        );

        assert_eq!(
            block_data.block.block.header.transactions_commitment,
            expected.transactions_commitment
        );
        assert_eq!(block_data.block.block.header.events_commitment, expected.events_commitment);
        assert_eq!(block_data.block.block.header.receipts_commitment, expected.receipts_commitment);
        assert_eq!(
            block_data.block.block.header.state_diff_commitment,
            expected.state_diff_commitment
        );
        assert_eq!(block_data.block.block.header.state_diff_length, expected.state_diff_length);
    }

    /// Parses block and state update fixture JSON into `BlockData` and the expected `Header`
    /// (with all commitment fields populated from the fixture).
    fn block_data_from_split_fixtures(
        block_json: &str,
        state_update_json: &str,
    ) -> (BlockData, Header) {
        let (expected_header, _) = header_from_fixture(block_json, None);

        let mut value = serde_json::from_str::<serde_json::Value>(block_json).unwrap();
        normalize_legacy_block_fixture(&mut value);
        let block = serde_json::from_value::<Block>(value).unwrap();
        let state_update = serde_json::from_str::<ConfirmedStateUpdate>(state_update_json).unwrap();
        let block_data = BlockData::from(StateUpdateWithBlock {
            block,
            state_update: StateUpdate::Confirmed(state_update),
        });

        (block_data, expected_header)
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
}
