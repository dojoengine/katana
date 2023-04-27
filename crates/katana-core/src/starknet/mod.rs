use std::{collections::HashMap, sync::Mutex, time::SystemTime};

use anyhow::Result;
use blockifier::{
    block_context::BlockContext,
    state::cached_state::CachedState,
    transaction::{
        objects::TransactionExecutionInfo, transaction_execution::Transaction,
        transactions::ExecutableTransaction,
    },
};
use starknet::providers::jsonrpc::models::TransactionStatus;
use starknet_api::{
    block::{BlockHash, BlockNumber, BlockTimestamp, GasPrice},
    core::GlobalRoot,
    hash::StarkFelt,
    stark_felt,
};

pub mod block;
pub mod transaction;

use crate::{block_context::Base, state::DictStateReader};
use block::{StarknetBlock, StarknetBlocks};
use transaction::{StarknetTransaction, StarknetTransactions};

pub struct Config;

pub struct StarknetWrapper {
    pub config: Config,
    pub blocks: StarknetBlocks,
    pub block_context: BlockContext,
    pub transactions: StarknetTransactions,
    pub state: Mutex<CachedState<DictStateReader>>,
}

impl StarknetWrapper {
    pub fn new(config: Config) -> Self {
        let hash_to_num = HashMap::new();
        let num_to_blocks = HashMap::new();
        let block_context = BlockContext::base();
        let state = Mutex::new(CachedState::new(
            DictStateReader::new_with_sensible_defaults(),
        ));

        let blocks = StarknetBlocks {
            hash_to_num,
            num_to_blocks,
            pending_block: None,
            current_height: BlockNumber(0),
        };

        let transactions = StarknetTransactions {
            transactions: HashMap::new(),
        };

        Self {
            state,
            config,
            blocks,
            transactions,
            block_context,
        }
    }

    // execute the tx
    pub fn handle_transaction(&self, transaction: Transaction) -> Result<()> {
        let res = match transaction {
            Transaction::AccountTransaction(tx) => {
                tx.execute(&mut self.state.lock().unwrap(), &self.block_context)
            }
            Transaction::L1HandlerTransaction(tx) => {
                tx.execute(&mut self.state.lock().unwrap(), &self.block_context)
            }
        };

        let status: TransactionStatus;
        let execution_info: Option<TransactionExecutionInfo> = None;

        if let Ok(tx_info) = res {
            status = TransactionStatus::Pending;
            execution_info = Some(tx_info);

            //  append successful tx to pending block
            self.blocks
                .pending_block
                .unwrap()
                // convert blockifier tx -> starknet api tx
                .insert_transaction(transaction);
        } else {
            status = TransactionStatus::Rejected;
        }

        // convert blockifier tx -> starknet tx
        let tx = StarknetTransaction::new(transaction, status, execution_info);

        // store tx anyway even if it failed
        self.store_transaction(tx);

        self.blocks.append_block(self.generate_latest_block());

        self.update_block_context();

        self.generate_pending_block();

        Ok(())
    }

    // creates a new block that contains all the pending txs
    pub fn generate_latest_block(&self) -> StarknetBlock {
        if let Some(mut pending) = self.blocks.pending_block {
            for pending_tx in pending.transactions() {
                if let Some(tx) = self
                    .transactions
                    .transactions
                    .get_mut(&pending_tx.transaction_hash())
                {
                    let block_hash = pending.compute_block_hash();
                    pending.0.header.block_hash = block_hash;

                    tx.block_hash = Some(block_hash);
                    tx.status = TransactionStatus::AcceptedOnL2;
                    tx.block_number = Some(pending.block_number());
                }
            }

            pending
        } else {
            self.create_empty_block()
        }
    }

    pub fn generate_pending_block(&self) {
        self.blocks.pending_block = Some(self.create_empty_block());
    }

    pub fn create_empty_block(&self) -> StarknetBlock {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|t| BlockTimestamp(t.as_secs()))
            .expect("should get unix timestamp");

        let block_number = self.blocks.current_height;
        let parent_hash = if block_number.0 == 0 {
            BlockHash(stark_felt!(0))
        } else {
            self.blocks
                .get_last_block()
                .map(|last_block| last_block.block_hash())
                .unwrap()
        };

        StarknetBlock::new(
            BlockHash(stark_felt!(0)),
            parent_hash,
            block_number,
            GasPrice(self.block_context.gas_price),
            GlobalRoot(stark_felt!(0)),
            self.block_context.sequencer_address,
            timestamp,
            vec![],
            vec![],
        )
    }

    // store the tx doesnt matter if its successfully executed or not
    fn store_transaction(&self, transaction: StarknetTransaction) -> Option<StarknetTransaction> {
        self.transactions
            .transactions
            .insert(transaction.inner.transaction_hash(), transaction)
    }

    fn update_block_context(&self) {
        let current_height = self.blocks.current_height;
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.block_context.block_number = current_height;
        self.block_context.block_timestamp = BlockTimestamp(timestamp);
        self.blocks.current_height = BlockNumber(self.blocks.current_height.0 + 1);
    }
}
