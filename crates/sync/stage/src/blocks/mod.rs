use anyhow::Result;
use futures::future::BoxFuture;
use katana_db::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use katana_db::tables;
use katana_gateway_types::{BlockStatus, StateUpdate as GatewayStateUpdate, StateUpdateWithBlock};
use katana_primitives::block::{
    BlockNumber, FinalityStatus, GasPrices, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::fee::{FeeInfo, PriceUnit};
use katana_primitives::receipt::{
    DeclareTxReceipt, DeployAccountTxReceipt, InvokeTxReceipt, L1HandlerTxReceipt, Receipt,
};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{Tx, TxWithHash};
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use num_traits::ToPrimitive;
use starknet::core::types::ResourcePrice;
use tracing::{debug, error, info_span, Instrument};

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

mod downloader;

pub use downloader::{BatchBlockDownloader, BlockDownloader};

/// A stage for syncing blocks.
#[derive(Debug)]
pub struct Blocks<B> {
    provider: DbProviderFactory,
    downloader: B,
}

impl<B> Blocks<B> {
    /// Create a new [`Blocks`] stage.
    pub fn new(provider: DbProviderFactory, downloader: B) -> Self {
        Self { provider, downloader }
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

    /// Unwinds block data by removing all blocks after the specified block number.
    ///
    /// This removes entries from the following tables:
    /// - Headers, BlockHashes, BlockNumbers, BlockBodyIndices, BlockStatusses
    /// - TxNumbers, TxBlocks, TxHashes, TxTraces, Transactions, Receipts
    fn unwind_blocks<Db: Database>(db: &Db, unwind_to: BlockNumber) -> Result<(), crate::Error> {
        db.update(|db_tx| -> Result<(), katana_provider::api::ProviderError> {
            // Get the tx_offset for the unwind_to block to know where to start deleting txs
            let mut last_tx_num = None;
            if let Some(indices) = db_tx.get::<tables::BlockBodyIndices>(unwind_to)? {
                last_tx_num = Some(indices.tx_offset + indices.tx_count);
            }

            // Remove all blocks after unwind_to
            let mut blocks_to_remove = Vec::new();
            let mut cursor = db_tx.cursor_mut::<tables::Headers>()?;

            // Find all blocks after unwind_to
            if let Some((block_num, _)) = cursor.seek(unwind_to + 1)? {
                blocks_to_remove.push(block_num);
                while let Some((block_num, _)) = cursor.next()? {
                    blocks_to_remove.push(block_num);
                }
            }
            drop(cursor);

            // Remove block data
            for block_num in blocks_to_remove {
                // Get block hash before deleting
                let block_hash = db_tx.get::<tables::BlockHashes>(block_num)?;

                db_tx.delete::<tables::Headers>(block_num, None)?;
                db_tx.delete::<tables::BlockHashes>(block_num, None)?;
                db_tx.delete::<tables::BlockBodyIndices>(block_num, None)?;
                db_tx.delete::<tables::BlockStatusses>(block_num, None)?;

                if let Some(hash) = block_hash {
                    db_tx.delete::<tables::BlockNumbers>(hash, None)?;
                }
            }

            // Remove transaction data if we have a last_tx_num
            if let Some(start_tx_num) = last_tx_num {
                let mut txs_to_remove = Vec::new();
                let mut cursor = db_tx.cursor_mut::<tables::Transactions>()?;

                if let Some((tx_num, _)) = cursor.seek(start_tx_num)? {
                    txs_to_remove.push(tx_num);
                    while let Some((tx_num, _)) = cursor.next()? {
                        txs_to_remove.push(tx_num);
                    }
                }
                drop(cursor);

                for tx_num in txs_to_remove {
                    // Get tx hash before deleting
                    let tx_hash = db_tx.get::<tables::TxHashes>(tx_num)?;

                    db_tx.delete::<tables::Transactions>(tx_num, None)?;
                    db_tx.delete::<tables::TxHashes>(tx_num, None)?;
                    db_tx.delete::<tables::TxBlocks>(tx_num, None)?;
                    db_tx.delete::<tables::Receipts>(tx_num, None)?;
                    db_tx.delete::<tables::TxTraces>(tx_num, None)?;

                    if let Some(hash) = tx_hash {
                        db_tx.delete::<tables::TxNumbers>(hash, None)?;
                    }
                }
            }

            Ok(())
        })
        .map_err(katana_provider::api::ProviderError::from)??;

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
                let (block, receipts, state_updates) = extract_block_data(block)?;
                let block_number = block.block.header.number;

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

    fn unwind(&mut self, unwind_to: BlockNumber) -> BoxFuture<'_, StageResult> {
        Box::pin(async move {
            debug!(target: "stage", id = %self.id(), unwind_to = %unwind_to, "Unwinding blocks.");

            // Unwind blocks using the database directly
            Self::unwind_blocks(self.provider.db(), unwind_to)?;

            // Update checkpoint
            let provider_mut = self.provider.provider_mut();
            provider_mut.set_execution_checkpoint(self.id(), unwind_to)?;
            provider_mut.commit()?;

            Ok(StageExecutionOutput { last_block_processed: unwind_to })
        })
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

            let unit = if tx.transaction.version() >= Felt::THREE {
                PriceUnit::Fri
            } else {
                PriceUnit::Wei
            };

            let fee = FeeInfo { unit, overall_fee, ..Default::default() };

            match tx.transaction {
                Tx::Invoke(_) => Receipt::Invoke(InvokeTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    execution_resources: Default::default(),
                }),
                Tx::Declare(_) => Receipt::Declare(DeclareTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    execution_resources: Default::default(),
                }),
                Tx::L1Handler(_) => Receipt::L1Handler(L1HandlerTxReceipt {
                    fee,
                    events,
                    messages_sent,
                    revert_error,
                    message_hash: Default::default(),
                    execution_resources: Default::default(),
                }),
                Tx::DeployAccount(_) => Receipt::DeployAccount(DeployAccountTxReceipt {
                    fee,
                    events,
                    revert_error,
                    messages_sent,
                    contract_address: Default::default(),
                    execution_resources: Default::default(),
                }),
                Tx::Deploy(_) => unreachable!("Deploy transactions are not supported"),
            }
        })
        .collect::<Vec<Receipt>>();

    let transaction_count = transactions.len() as u32;
    let block = SealedBlock {
        body: transactions,
        hash: data.block.block_hash.unwrap_or_default(),
        header: Header {
            transaction_count,
            timestamp: data.block.timestamp,
            l1_da_mode: data.block.l1_da_mode,
            events_count: Default::default(),
            parent_hash: data.block.parent_block_hash,
            state_diff_length: Default::default(),
            receipts_commitment: Default::default(),
            state_diff_commitment: Default::default(),
            number: data.block.block_number.unwrap_or_default(),
            l1_gas_prices: to_gas_prices(data.block.l1_gas_price),
            l2_gas_prices: to_gas_prices(data.block.l2_gas_price),
            state_root: data.block.state_root.unwrap_or_default(),
            l1_data_gas_prices: to_gas_prices(data.block.l1_data_gas_price),
            starknet_version: data.block.starknet_version.unwrap_or_default().try_into().unwrap(),
            events_commitment: data.block.event_commitment.unwrap_or_default(),
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
