use anyhow::Result;
use futures::future::BoxFuture;
use katana_gateway_types::{BlockStatus, StateUpdate as GatewayStateUpdate, StateUpdateWithBlock};
use katana_primitives::block::{
    FinalityStatus, GasPrices, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::chain::ChainId;
use katana_primitives::fee::{FeeInfo, PriceUnit};
use katana_primitives::receipt::{
    DeclareTxReceipt, DeployAccountTxReceipt, DeployTxReceipt, InvokeTxReceipt, L1HandlerTxReceipt,
    Receipt, ReceiptWithTxHash,
};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{Tx, TxWithHash};
use katana_primitives::version::StarknetVersion;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_trie::compute_merkle_root;
use num_traits::ToPrimitive;
use starknet::core::types::ResourcePrice;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use tracing::{error, info_span, warn, Instrument};

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

mod downloader;
pub mod hash;

pub use downloader::{BatchBlockDownloader, BlockDownloader};

/// A stage for syncing blocks.
#[derive(Debug)]
pub struct Blocks<B> {
    provider: DbProviderFactory,
    downloader: B,
    chain_id: ChainId,
}

impl<B> Blocks<B> {
    /// Create a new [`Blocks`] stage.
    pub fn new(provider: DbProviderFactory, downloader: B, chain_id: ChainId) -> Self {
        Self { provider, downloader, chain_id }
    }

    /// Validates that the downloaded blocks form a valid chain.
    ///
    /// This method checks the chain invariant: block N's parent hash must be block N-1's hash.
    /// For the first block in the list (if not block 0), it fetches the parent hash from storage.
    fn validate_chain_invariant(&self, blocks: &[StateUpdateWithBlock]) -> Result<(), Error> {
        if blocks.is_empty() {
            return Ok(());
        }

        // Validate the first block against its parent in storage (if not block 0)
        let first_block = &blocks[0].block;
        let first_block_num =
            first_block.block_number.expect("only confirmed blocks are synced atm");

        if first_block_num > 0 {
            let parent_block_num = first_block_num - 1;
            let expected_parent_hash = self
                .provider
                .provider()
                .block_hash_by_num(parent_block_num)?
                .ok_or(ProviderError::MissingBlockHash(parent_block_num))?;

            if first_block.parent_block_hash != expected_parent_hash {
                return Err(Error::ChainInvariantViolation {
                    block_num: first_block_num,
                    parent_hash: first_block.parent_block_hash,
                    expected_hash: expected_parent_hash,
                });
            }
        }

        // Validate the rest of the blocks in the list
        for window in blocks.windows(2) {
            let prev_block = &window[0].block;
            let curr_block = &window[1].block;

            let prev_hash = prev_block.block_hash.unwrap_or_default();
            let curr_block_num = curr_block.block_number.unwrap_or_default();

            if curr_block.parent_block_hash != prev_hash {
                return Err(Error::ChainInvariantViolation {
                    block_num: curr_block_num,
                    parent_hash: curr_block.parent_block_hash,
                    expected_hash: prev_hash,
                });
            }
        }

        Ok(())
    }
}

impl<D> Stage for Blocks<D>
where
    D: BlockDownloader,
{
    fn id(&self) -> &'static str {
        "Blocks"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let blocks = self
                .downloader
                .download_blocks(input.from(), input.to())
                .instrument(info_span!(target: "stage", "blocks.download", from = %input.from(), to = %input.to()))
                .await
                .map_err(Error::Gateway)?;

            let span = info_span!(target: "stage", "blocks.insert", from = %input.from(), to = %input.to());
            let _enter = span.enter();

            // TODO: spawn onto a blocking thread pool
            self.validate_chain_invariant(&blocks)?;

            let provider_mut = self.provider.provider_mut();

            for block in blocks {
                let (mut block, receipts, state_updates) = extract_block_data(block)?;
                let block_number = block.block.header.number;

                // Compute missing commitments for older blocks where the gateway
                // doesn't include them in the block header.
                compute_missing_commitments(&mut block.block, &receipts);

                // Verify the block hash matches what we compute locally.
                let computed_hash = hash::compute_hash(&block.block.header, &self.chain_id);
                if computed_hash != block.block.hash {
                    warn!(
                        block = %block_number,
                        expected = %block.block.hash,
                        computed = %computed_hash,
                        "Block hash mismatch"
                    );
                }

                provider_mut
                    .insert_block_with_states_and_receipts(
                        block,
                        state_updates,
                        receipts,
                        Vec::new(),
                    )
                    .inspect_err(
                        |e| error!(error = %e, block = %block_number, "Error storing block."),
                    )?;
            }

            provider_mut.commit()?;

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    // TODO: implement block pruning
    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error returnd by the client used to download the classes from.
    #[error(transparent)]
    Gateway(#[from] katana_gateway_client::Error),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(
        "chain invariant violation: block {block_num} parent hash {parent_hash:#x} does not match \
         previous block hash {expected_hash:#x}"
    )]
    ChainInvariantViolation { block_num: u64, parent_hash: Felt, expected_hash: Felt },

    #[error(
        "block hash mismatch: block {block_num} gateway hash {expected:#x} does not match \
         computed hash {computed:#x}"
    )]
    BlockHashMismatch { block_num: u64, expected: Felt, computed: Felt },
}

fn extract_block_data(
    data: StateUpdateWithBlock,
) -> Result<(SealedBlockWithStatus, Vec<Receipt>, StateUpdatesWithClasses)> {
    fn to_gas_prices(prices: ResourcePrice) -> GasPrices {
        let eth = prices.price_in_wei.to_u128().expect("valid u128");
        let strk = prices.price_in_fri.to_u128().expect("valid u128");
        // older blocks might have zero gas prices (recent Starknet upgrade has made the minimum gas
        // prices to 1) we may need to handle this case if we want to be able to compute the
        // block hash correctly
        let eth = if eth == 0 { 1 } else { eth };
        let strk = if strk == 0 { 1 } else { strk };
        unsafe { GasPrices::new_unchecked(eth, strk) }
    }

    let status = match data.block.status {
        BlockStatus::AcceptedOnL2 => FinalityStatus::AcceptedOnL2,
        BlockStatus::AcceptedOnL1 => FinalityStatus::AcceptedOnL1,
        status => panic!("unsupported block status: {status:?}"),
    };

    let transactions = data
        .block
        .transactions
        .into_iter()
        .map(|tx| tx.try_into())
        .collect::<Result<Vec<TxWithHash>, _>>()?;

    let receipts = data
        .block
        .transaction_receipts
        .into_iter()
        .zip(transactions.iter())
        .map(|(receipt, tx)| {
            let events = receipt.body.events;
            let revert_error = receipt.body.revert_error;
            let messages_sent = receipt.body.l2_to_l1_messages;
            let overall_fee = receipt.body.actual_fee.to_u128().expect("valid u128");
            let execution_resources = receipt.body.execution_resources.unwrap_or_default();

            let unit = if tx.transaction.version() >= Felt::THREE {
                PriceUnit::Fri
            } else {
                PriceUnit::Wei
            };

            let fee = FeeInfo { unit, overall_fee, ..Default::default() };

            match &tx.transaction {
                Tx::Invoke(_) => Receipt::Invoke(InvokeTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    execution_resources: execution_resources.into(),
                }),
                Tx::Declare(_) => Receipt::Declare(DeclareTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    execution_resources: execution_resources.into(),
                }),
                Tx::L1Handler(_) => Receipt::L1Handler(L1HandlerTxReceipt {
                    fee,
                    events,
                    messages_sent,
                    revert_error,
                    message_hash: Default::default(),
                    execution_resources: execution_resources.into(),
                }),
                Tx::DeployAccount(tx) => Receipt::DeployAccount(DeployAccountTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    contract_address: tx.contract_address(),
                    execution_resources: execution_resources.into(),
                }),
                Tx::Deploy(tx) => Receipt::Deploy(DeployTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    contract_address: tx.contract_address.into(),
                    execution_resources: execution_resources.into(),
                }),
            }
        })
        .collect::<Vec<Receipt>>();

    let transaction_count = transactions.len() as u32;
    let events_count = receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();

    let block = SealedBlock {
        body: transactions,
        hash: data.block.block_hash.unwrap_or_default(),
        header: Header {
            transaction_count,
            events_count,
            timestamp: data.block.timestamp,
            l1_da_mode: data.block.l1_da_mode,
            parent_hash: data.block.parent_block_hash,
            state_diff_length: Default::default(),
            state_diff_commitment: Default::default(),
            number: data.block.block_number.unwrap_or_default(),
            l1_gas_prices: to_gas_prices(data.block.l1_gas_price),
            l2_gas_prices: to_gas_prices(data.block.l2_gas_price),
            state_root: data.block.state_root.unwrap_or_default(),
            l1_data_gas_prices: to_gas_prices(data.block.l1_data_gas_price),
            starknet_version: data.block.starknet_version.unwrap_or_default().try_into().unwrap(),
            events_commitment: data.block.event_commitment.unwrap_or_default(),
            receipts_commitment: data.block.receipt_commitment.unwrap_or_default(),
            sequencer_address: data.block.sequencer_address.unwrap_or_default(),
            transactions_commitment: data.block.transaction_commitment.unwrap_or_default(),
        },
    };

    let state_updates: StateUpdates = match data.state_update {
        GatewayStateUpdate::Confirmed(update) => update.state_diff.into(),
        GatewayStateUpdate::PreConfirmed(update) => update.state_diff.into(),
    };

    let state_updates = StateUpdatesWithClasses { state_updates, ..Default::default() };

    Ok((SealedBlockWithStatus { block, status }, receipts, state_updates))
}

/// Computes missing block commitments that older gateway blocks don't include.
///
/// For blocks before 0.13.2, the gateway may return zero for transaction and event
/// commitments. For receipt commitment, the gateway may not include it at all for
/// older blocks. This function computes them locally so that block hash verification
/// can succeed.
fn compute_missing_commitments(block: &mut SealedBlock, receipts: &[Receipt]) {
    let version = block.header.starknet_version;

    // Transaction commitment: older blocks (pre-0.7 and 0.7.0) return zero from the gateway.
    // Pre-0.13.2 uses Pedersen, post-0.13.2 uses Poseidon.
    if block.header.transactions_commitment == Felt::ZERO {
        let tx_hashes: Vec<Felt> = block.body.iter().map(|t| t.hash).collect();
        block.header.transactions_commitment = if version < StarknetVersion::V0_13_2 {
            compute_merkle_root::<Pedersen>(&tx_hashes).unwrap()
        } else {
            compute_merkle_root::<Poseidon>(&tx_hashes).unwrap()
        };
    }

    // Event commitment: older blocks return zero from the gateway.
    // Pre-0.13.2 uses Pedersen, post-0.13.2 uses Poseidon (and includes tx_hash in leaf).
    if block.header.events_commitment == Felt::ZERO {
        let event_hashes: Vec<Felt> = if version < StarknetVersion::V0_13_2 {
            receipts
                .iter()
                .flat_map(|r| {
                    r.events().iter().map(|event| {
                        let keys_hash = Pedersen::hash_array(&event.keys);
                        let data_hash = Pedersen::hash_array(&event.data);
                        Pedersen::hash_array(&[event.from_address.into(), keys_hash, data_hash])
                    })
                })
                .collect()
        } else {
            receipts
                .iter()
                .zip(block.body.iter())
                .flat_map(|(receipt, tx)| {
                    receipt.events().iter().map(move |event| {
                        let keys_hash = Poseidon::hash_array(&event.keys);
                        let data_hash = Poseidon::hash_array(&event.data);
                        Poseidon::hash_array(&[
                            tx.hash,
                            event.from_address.into(),
                            keys_hash,
                            data_hash,
                        ])
                    })
                })
                .collect()
        };

        block.header.events_commitment = if version < StarknetVersion::V0_13_2 {
            compute_merkle_root::<Pedersen>(&event_hashes).unwrap()
        } else {
            compute_merkle_root::<Poseidon>(&event_hashes).unwrap()
        };
    }

    // Receipt commitment: only used in post-0.13.2 block hashes. The gateway may not
    // include it, so we compute it when missing.
    if block.header.receipts_commitment == Felt::ZERO && version >= StarknetVersion::V0_13_2 {
        let receipt_hashes: Vec<Felt> = receipts
            .iter()
            .zip(block.body.iter())
            .map(|(receipt, tx)| ReceiptWithTxHash::new(tx.hash, receipt.clone()).compute_hash())
            .collect();
        block.header.receipts_commitment =
            compute_merkle_root::<Poseidon>(&receipt_hashes).unwrap();
    }
}
