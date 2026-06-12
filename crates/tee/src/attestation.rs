//! Block attestation building.
//!
//! This is the single implementation behind both the `tee_generateQuote` RPC
//! method and the embedded settlement service: it aggregates the block range's
//! L1<->L2 messages, computes the v1 `report_data` commitment, and asks the
//! [`Attester`] for a quote over it. Keeping it here (rather than in the RPC
//! server) guarantees the attestation served to external verifiers is
//! byte-identical to the one the node settles with.

use katana_chain_spec::tee::{
    KATANA_TEE_APPCHAIN_MODE, KATANA_TEE_REPORT_VERSION, KATANA_TEE_SHARDING_MODE,
};
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::Tx;
use katana_primitives::Felt;
use katana_provider_api::block::{BlockHashProvider, HeaderProvider};
use katana_provider_api::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider_api::ProviderError;
use katana_rpc_types::tee::BlockAttestation;
use katana_rpc_types::{L1ToL2Message, L2ToL1Message};
use tracing::{debug, info};

use crate::{Attester, TeeError};

const LOG_TARGET: &str = "tee";

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error("Block hash not found for block {0}")]
    BlockHashNotFound(BlockNumber),

    #[error("Header not found for block {0}")]
    HeaderNotFound(BlockNumber),

    #[error("quote generation failed: {0}")]
    Quote(#[from] TeeError),
}

/// Builds the full [`BlockAttestation`] for the state transition `(prev_block, block]`.
///
/// In appchain mode (`fork_block_number = None`) this aggregates all L1<->L2
/// messages in the range and binds their commitment into the attestation's
/// `report_data`; in fork/sharding mode message aggregation is skipped and the
/// `events_commitment` is bound instead.
///
/// `prev_block = None` is the genesis case: the previous block fields fold to
/// the `(Felt::MAX, ZERO, ZERO)` sentinel that matches Piltover's
/// `AppchainState` initial value.
pub fn build_block_attestation<P>(
    provider: &P,
    attester: &dyn Attester,
    prev_block: Option<BlockNumber>,
    block: BlockNumber,
    fork_block_number: Option<u64>,
    katana_tee_config_hash: Felt,
) -> Result<BlockAttestation, AttestationError>
where
    P: BlockHashProvider + HeaderProvider + ReceiptProvider + TransactionProvider + ?Sized,
{
    debug!(
        target: LOG_TARGET,
        ?prev_block,
        block,
        %katana_tee_config_hash,
        "Generating TEE attestation quote"
    );

    // Get prev block info
    let (prev_block_id, prev_block_hash, prev_state_root) = match prev_block {
        None => (Felt::MAX, Felt::ZERO, Felt::ZERO),

        Some(num) => {
            let hash =
                provider.block_hash_by_num(num)?.ok_or(AttestationError::BlockHashNotFound(num))?;

            let header =
                provider.header_by_number(num)?.ok_or(AttestationError::HeaderNotFound(num))?;

            (Felt::from(num), hash, header.state_root)
        }
    };

    let block_hash =
        provider.block_hash_by_num(block)?.ok_or(AttestationError::BlockHashNotFound(block))?;

    let header =
        provider.header_by_number(block)?.ok_or(AttestationError::HeaderNotFound(block))?;

    let state_root = header.state_root;
    let events_commitment = header.events_commitment;

    if let Some(fork_block) = fork_block_number {
        let report_data = compute_report_data_sharding(
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_id,
            block.into(),
            fork_block.into(),
            events_commitment,
            katana_tee_config_hash,
        );

        let quote = attester.generate_quote(&report_data)?;

        info!(
            target: LOG_TARGET,
            ?prev_block_id,
            block_number = block,
            %prev_block_hash,
            %block_hash,
            %katana_tee_config_hash,
            quote_size = quote.len(),
            "Generated TEE attestation quote"
        );

        Ok(BlockAttestation {
            quote: format!("0x{}", hex::encode(&quote)),
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: prev_block.map(Felt::from).unwrap_or(Felt::MAX),
            block_number: block.into(),
            fork_block_number: fork_block_number.map(Felt::from),
            events_commitment,
            katana_tee_config_hash,
            l1_to_l2_messages: Vec::new(),
            l2_to_l1_messages: Vec::new(),
            messages_commitment: Felt::ZERO,
        })
    } else {
        // Gather all L1<->L2 messages from prev_block+1 to block_id (inclusive)
        let start_block = prev_block.map(|n| n + 1).unwrap_or(0);

        let mut l2_to_l1_messages: Vec<L2ToL1Message> = Vec::new();
        let mut l1_to_l2_messages: Vec<L1ToL2Message> = Vec::new();

        let mut l2_to_l1_msg_hashes: Vec<Felt> = Vec::new();
        let mut l1_to_l2_msg_hashes: Vec<Felt> = Vec::new();

        for block_num in start_block..=block {
            let block_id_or_num = BlockHashOrNumber::Num(block_num);

            let receipts = provider.receipts_by_block(block_id_or_num)?.unwrap_or_default();
            let txs = provider.transactions_by_block(block_id_or_num)?.unwrap_or_default();

            for receipt in &receipts {
                // L2->L1: each message in messages_sent
                for msg in receipt.messages_sent() {
                    let len = Felt::from(msg.payload.len());
                    let payload_hash = Poseidon::hash_array(
                        &std::iter::once(len)
                            .chain(msg.payload.iter().copied())
                            .collect::<Vec<_>>(),
                    );
                    let msg_hash = Poseidon::hash_array(&[
                        msg.from_address.into(),
                        msg.to_address,
                        payload_hash,
                    ]);
                    l2_to_l1_msg_hashes.push(msg_hash);
                    l2_to_l1_messages.push(L2ToL1Message {
                        from_address: msg.from_address.into(),
                        to_address: msg.to_address,
                        payload: msg.payload.clone(),
                    });
                }

                // L1->L2: message_hash from L1Handler receipt; full fields from the tx
                if let Receipt::L1Handler(l1h) = receipt {
                    let felt = Felt::from_bytes_be_slice(&l1h.message_hash.0);
                    l1_to_l2_msg_hashes.push(felt);
                }
            }

            // Collect full L1→L2 message fields from L1Handler transactions
            for tx in &txs {
                if let Tx::L1Handler(l1h) = &tx.transaction {
                    // calldata[0] is always the settlement-chain sender address as a Felt
                    let from_address =
                        l1h.calldata.first().copied().expect("qed; missing message sender address");

                    let payload = l1h.calldata.get(1..).unwrap_or_default().to_vec();
                    l1_to_l2_messages.push(L1ToL2Message {
                        payload,
                        from_address,
                        nonce: l1h.nonce,
                        to_address: l1h.contract_address.into(),
                        entry_point_selector: l1h.entry_point_selector,
                    });
                }
            }
        }

        // Combine both directions into a single messages commitment
        let l2_to_l1_commitment = Poseidon::hash_array(&l2_to_l1_msg_hashes);
        let l1_to_l2_commitment = Poseidon::hash_array(&l1_to_l2_msg_hashes);
        let messages_commitment = Poseidon::hash_array(&[l2_to_l1_commitment, l1_to_l2_commitment]);

        let commitment = compute_appchain_commitment(
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_id,
            block.into(),
            messages_commitment,
            katana_tee_config_hash,
        );
        let report_data = encode_report_data(commitment, katana_tee_config_hash);

        debug!(
            target: LOG_TARGET,
            %state_root,
            %block_hash,
            %katana_tee_config_hash,
            %commitment,
            "Computed report data for attestation"
        );

        let quote = attester.generate_quote(&report_data)?;

        info!(
            target: LOG_TARGET,
            ?prev_block_id,
            block_number = block,
            %prev_block_hash,
            %block_hash,
            %katana_tee_config_hash,
            quote_size = quote.len(),
            l2_to_l1_count = l2_to_l1_messages.len(),
            l1_to_l2_count = l1_to_l2_messages.len(),
            "Generated TEE attestation quote"
        );

        Ok(BlockAttestation {
            quote: format!("0x{}", hex::encode(&quote)),
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: prev_block.map(Felt::from).unwrap_or(Felt::MAX),
            block_number: block.into(),
            fork_block_number: None,
            events_commitment,
            katana_tee_config_hash,
            l1_to_l2_messages,
            l2_to_l1_messages,
            messages_commitment,
        })
    }
}

/// The v1 appchain commitment — the first 32 bytes of the attestation's
/// `report_data`, recomputed and asserted by the Piltover core contract's
/// `TeeInput` validation. The mock prover reuses this exact formula to
/// synthesize verifiable journals.
///
/// ```text
/// commitment = Poseidon(
///     KATANA_TEE_REPORT_VERSION,
///     KATANA_TEE_APPCHAIN_MODE,
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     messages_commitment,
///     katana_tee_config_hash,
/// )
/// ```
#[allow(clippy::too_many_arguments)]
pub fn compute_appchain_commitment(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    messages_commitment: Felt,
    katana_tee_config_hash: Felt,
) -> Felt {
    Poseidon::hash_array(&[
        KATANA_TEE_REPORT_VERSION.into(),
        KATANA_TEE_APPCHAIN_MODE.into(),
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        messages_commitment,
        katana_tee_config_hash,
    ])
}

/// Compute the 64-byte report data for fork/sharding attestation (v1 schema).
///
/// ```text
/// commitment = Poseidon(
///     KATANA_TEE_REPORT_VERSION,
///     KATANA_TEE_SHARDING_MODE,
///     prev_state_root,
///     state_root,
///     prev_block_hash,
///     block_hash,
///     prev_block_number,
///     block_number,
///     fork_block_number,
///     events_commitment,
///     katana_tee_config_hash,
/// )
/// report_data = commitment_bytes_be ++ katana_tee_config_hash_bytes_be
/// ```
#[allow(clippy::too_many_arguments)]
fn compute_report_data_sharding(
    prev_state_root: Felt,
    state_root: Felt,
    prev_block_hash: Felt,
    block_hash: Felt,
    prev_block_number: Felt,
    block_number: Felt,
    fork_block_number: Felt,
    events_commitment: Felt,
    katana_tee_config_hash: Felt,
) -> [u8; 64] {
    let commitment = Poseidon::hash_array(&[
        KATANA_TEE_REPORT_VERSION.into(),
        KATANA_TEE_SHARDING_MODE.into(),
        prev_state_root,
        state_root,
        prev_block_hash,
        block_hash,
        prev_block_number,
        block_number,
        fork_block_number,
        events_commitment,
        katana_tee_config_hash,
    ]);

    let report_data = encode_report_data(commitment, katana_tee_config_hash);

    debug!(
        target: LOG_TARGET,
        %state_root,
        %block_hash,
        ?fork_block_number,
        %events_commitment,
        %katana_tee_config_hash,
        %commitment,
        "Computed report data for attestation"
    );

    report_data
}

/// `report_data = commitment_bytes_be ++ katana_tee_config_hash_bytes_be`
///
/// The second half exposes the config hash directly so verifiers can check the
/// environment binding without recomputing the commitment.
pub fn encode_report_data(commitment: Felt, katana_tee_config_hash: Felt) -> [u8; 64] {
    let mut report_data = [0u8; 64];
    report_data[..32].copy_from_slice(&commitment.to_bytes_be());
    report_data[32..].copy_from_slice(&katana_tee_config_hash.to_bytes_be());
    report_data
}
