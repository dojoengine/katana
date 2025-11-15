use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use katana_db::abstraction::Database;
use katana_db::models::block::StoredBlockBodyIndices;
use katana_fork::Backend;
use katana_primitives::block::{
    Block, BlockHash, BlockHashOrNumber, BlockNumber, BlockWithTxHashes, FinalityStatus, Header,
    SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::ContractAddress;
use katana_primitives::env::BlockEnv;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{TxHash, TxNumber, TxWithHash};
use katana_provider_api::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
    HeaderProvider,
};
use katana_provider_api::env::BlockEnvProvider;
use katana_provider_api::stage::StageCheckpointProvider;
use katana_provider_api::state_update::StateUpdateProvider;
use katana_provider_api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::{GetBlockWithReceiptsResponse, RpcTxWithReceipt, StateUpdate};

use super::db::{self, DbProvider};
use crate::ProviderResult;

mod state;
mod trie;

#[derive(Debug)]
pub struct ForkedProvider<Db: Database = katana_db::Db> {
    local_db: Arc<DbProvider<Db>>,
    fork_db: Arc<ForkedDb>,
}

#[derive(Debug, Clone)]
struct ForkedDb {
    backend: Backend,
    block_id: BlockNumber,
    db: DbProvider<katana_db::Db>,
}

impl ForkedDb {
    /// Checks if a block number is before the fork point (and thus should be fetched externally)
    fn should_fetch_externally(&self, block_num: BlockNumber) -> bool {
        block_num <= self.block_id
    }

    /// Fetches historical blocks before the fork point.
    fn fetch_historical_blocks(&self, block_id: BlockHashOrNumber) -> ProviderResult<bool> {
        let block = match block_id {
            BlockHashOrNumber::Num(number) => {
                if !self.should_fetch_externally(number) {
                    return Ok(false);
                }

                // should exist if block id is older than fork point
                let block = self.backend.get_block(number.into())?.unwrap();
                let GetBlockWithReceiptsResponse::Block(block) = block else { unreachable!() };

                block
            }

            BlockHashOrNumber::Hash(hash) => {
                let Some(block) = self.backend.get_block(hash.into())? else {
                    return Ok(false);
                };

                let GetBlockWithReceiptsResponse::Block(block) = block else { unreachable!() };

                if !self.should_fetch_externally(block.block_number) {
                    return Ok(false);
                }

                block
            }
        };

        let state_update = self.backend.get_state_update(block_id)?.unwrap(); // should exist if block exist
        let StateUpdate::Update(state_update) = state_update else { unreachable!() };

        let header = Header {
            parent_hash: block.parent_hash,
            l1_da_mode: block.l1_da_mode,
            timestamp: block.timestamp,
            state_root: block.new_root,
            number: block.block_number,
            sequencer_address: block.sequencer_address,
            transaction_count: block.transactions.len() as u32,
            starknet_version: block.starknet_version.try_into().unwrap(),
            l1_data_gas_prices: block.l1_data_gas_price.into(),
            l1_gas_prices: block.l1_gas_price.into(),
            l2_gas_prices: block.l2_gas_price.into(),
            events_commitment: Default::default(),
            events_count: Default::default(),
            receipts_commitment: Default::default(),
            state_diff_commitment: Default::default(),
            state_diff_length: Default::default(),
            transactions_commitment: Default::default(),
        };

        let transactions_count = block.transactions.len();
        let mut body: Vec<TxWithHash> = Vec::with_capacity(transactions_count);
        let mut receipts: Vec<Receipt> = Vec::with_capacity(transactions_count);

        for RpcTxWithReceipt { transaction, receipt } in block.transactions {
            let hash = receipt.transaction_hash;

            let transaction = TxWithHash { hash, transaction: transaction.into() };
            let receipt = Receipt::from(receipt.receipt);

            body.push(transaction);
            receipts.push(receipt);
        }

        let status = block.status;
        let block = Block { header, body }.seal_with_hash_and_status(block.block_hash, status);
        let state_updates = StateUpdatesWithClasses {
            state_updates: state_update.state_diff.into(),
            classes: Default::default(),
        };

        self.db.insert_block_with_states_and_receipts(
            block,
            state_updates,
            receipts,
            Vec::new(),
        )?;

        Ok(true)
    }

    fn fetch_block_by_transaction_hash(&self, hash: TxHash) -> ProviderResult<bool> {
        if let Some(receipt) = self.backend.get_receipt(hash)? {
            let block_number = receipt.block.block_number();
            self.fetch_historical_blocks(block_number.into())
        } else {
            Ok(false)
        }
    }
}

impl<Db: Database> ForkedProvider<Db> {
    /// ## Arguments
    ///
    /// - `db`: The database to use for the provider.
    /// - `block_id`: The block number or hash to use as the fork point.
    /// - `starknet_client`: The Starknet JSON-RPC client to use for the provider.
    pub fn new(db: Db, block_id: BlockNumber, starknet_client: StarknetClient) -> Self {
        let local_db = Arc::new(DbProvider::new(db));

        let backend = Backend::new(starknet_client).expect("failed to create backend");
        let fork_db = Arc::new(ForkedDb { block_id, db: DbProvider::new_in_memory(), backend });

        Self { local_db, fork_db }
    }

    pub fn block_id(&self) -> BlockNumber {
        self.fork_db.block_id
    }

    pub fn forked_db(&self) -> &DbProvider {
        &self.fork_db.db
    }
}

impl ForkedProvider<katana_db::Db> {
    /// Creates a new [`ForkedProvider`] using an ephemeral database.
    pub fn new_ephemeral(block_id: BlockNumber, starknet_client: StarknetClient) -> Self {
        let local_db = Arc::new(DbProvider::new_in_memory());

        let backend = Backend::new(starknet_client).expect("failed to create backend");
        let fork_db = Arc::new(ForkedDb { block_id, db: DbProvider::new_in_memory(), backend });

        Self { local_db, fork_db }
    }
}

impl<Db: Database> BlockNumberProvider for ForkedProvider<Db> {
    fn block_number_by_hash(&self, hash: BlockHash) -> ProviderResult<Option<BlockNumber>> {
        if let Some(num) = self.local_db.block_number_by_hash(hash)? {
            return Ok(Some(num));
        }

        if let Some(num) = self.fork_db.db.block_number_by_hash(hash)? {
            return Ok(Some(num));
        }

        if self.fork_db.fetch_historical_blocks(hash.into())? {
            let num = self.fork_db.db.block_number_by_hash(hash)?.unwrap();
            Ok(Some(num))
        } else {
            Ok(None)
        }
    }

    fn latest_number(&self) -> ProviderResult<BlockNumber> {
        self.local_db.latest_number()
    }
}

impl<Db: Database> BlockHashProvider for ForkedProvider<Db> {
    fn latest_hash(&self) -> ProviderResult<BlockHash> {
        self.local_db.latest_hash()
    }

    fn block_hash_by_num(&self, num: BlockNumber) -> ProviderResult<Option<BlockHash>> {
        if let Some(hash) = self.local_db.block_hash_by_num(num)? {
            return Ok(Some(hash));
        }

        if num > self.block_id() {
            return Ok(None);
        }

        if let Some(hash) = self.fork_db.db.block_hash_by_num(num)? {
            return Ok(Some(hash));
        }

        if self.fork_db.fetch_historical_blocks(num.into())? {
            let num = self.fork_db.db.block_hash_by_num(num)?.unwrap();
            Ok(Some(num))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> HeaderProvider for ForkedProvider<Db> {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        if let Some(header) = self.local_db.header(id)? {
            return Ok(Some(header));
        }

        if let Some(header) = self.fork_db.db.header(id)? {
            return Ok(Some(header));
        }

        if self.fork_db.fetch_historical_blocks(id)? {
            let header = self.fork_db.db.header(id)?.unwrap();
            Ok(Some(header))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> BlockProvider for ForkedProvider<Db> {
    fn block_body_indices(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        if let Some(value) = self.local_db.block_body_indices(id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.block_body_indices(id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(id)? {
            let value = self.fork_db.db.block_body_indices(id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Block>> {
        if let Some(value) = self.local_db.block(id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.block(id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(id)? {
            let value = self.fork_db.db.block(id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn block_with_tx_hashes(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BlockWithTxHashes>> {
        if let Some(value) = self.local_db.block_with_tx_hashes(id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.block_with_tx_hashes(id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(id)? {
            let value = self.fork_db.db.block_with_tx_hashes(id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn blocks_in_range(&self, range: RangeInclusive<u64>) -> ProviderResult<Vec<Block>> {
        let mut blocks = Vec::new();

        for num in range {
            if let Some(block) = self.block(num.into())? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }
}

impl<Db: Database> BlockStatusProvider for ForkedProvider<Db> {
    fn block_status(&self, id: BlockHashOrNumber) -> ProviderResult<Option<FinalityStatus>> {
        if let Some(value) = self.local_db.block_status(id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.block_status(id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(id)? {
            let value = self.fork_db.db.block_status(id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> StateUpdateProvider for ForkedProvider<Db> {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        if let Some(value) = self.local_db.state_update(block_id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.state_update(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.state_update(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn declared_classes(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>> {
        if let Some(classes) = self.local_db.declared_classes(block_id)? {
            return Ok(Some(classes));
        }

        if let Some(value) = self.fork_db.db.declared_classes(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.declared_classes(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn deployed_contracts(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ContractAddress, ClassHash>>> {
        if let Some(value) = self.local_db.deployed_contracts(block_id)? {
            return Ok(Some(value));
        }

        if let Some(value) = self.fork_db.db.deployed_contracts(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.deployed_contracts(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> TransactionProvider for ForkedProvider<Db> {
    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>> {
        if let Some(tx) = self.local_db.transaction_by_hash(hash)? {
            return Ok(Some(tx));
        }

        if let Some(value) = self.fork_db.db.transaction_by_hash(hash)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_block_by_transaction_hash(hash)? {
            let value = self.fork_db.db.transaction_by_hash(hash)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn transactions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TxWithHash>>> {
        if let Some(txs) = self.local_db.transactions_by_block(block_id)? {
            return Ok(Some(txs));
        }

        if let Some(value) = self.fork_db.db.transactions_by_block(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.transactions_by_block(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn transaction_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxWithHash>> {
        let _ = range;
        unimplemented!()
    }

    fn transaction_block_num_and_hash(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
        if let Some(result) = self.local_db.transaction_block_num_and_hash(hash)? {
            return Ok(Some(result));
        }

        if let Some(value) = self.fork_db.db.transaction_block_num_and_hash(hash)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_block_by_transaction_hash(hash)? {
            let value = self.fork_db.db.transaction_block_num_and_hash(hash)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn transaction_by_block_and_idx(
        &self,
        block_id: BlockHashOrNumber,
        idx: u64,
    ) -> ProviderResult<Option<TxWithHash>> {
        if let Some(tx) = self.local_db.transaction_by_block_and_idx(block_id, idx)? {
            return Ok(Some(tx));
        }

        if let Some(value) = self.fork_db.db.transaction_by_block_and_idx(block_id, idx)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.transaction_by_block_and_idx(block_id, idx)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn transaction_count_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<u64>> {
        if let Some(count) = self.local_db.transaction_count_by_block(block_id)? {
            return Ok(Some(count));
        }

        if let Some(value) = self.fork_db.db.transaction_count_by_block(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.transaction_count_by_block(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> TransactionsProviderExt for ForkedProvider<Db> {
    fn transaction_hashes_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxHash>> {
        let _ = range;
        unimplemented!()
    }

    fn total_transactions(&self) -> ProviderResult<usize> {
        // NOTE: this only returns the total number of transactions in the local database
        self.local_db.total_transactions()
    }
}

impl<Db: Database> TransactionStatusProvider for ForkedProvider<Db> {
    fn transaction_status(&self, hash: TxHash) -> ProviderResult<Option<FinalityStatus>> {
        if let Some(result) = self.local_db.transaction_status(hash)? {
            return Ok(Some(result));
        }

        if let Some(value) = self.fork_db.db.transaction_status(hash)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_block_by_transaction_hash(hash)? {
            let value = self.fork_db.db.transaction_status(hash)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> TransactionTraceProvider for ForkedProvider<Db> {
    fn transaction_execution(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<TypedTransactionExecutionInfo>> {
        if let Some(result) = self.local_db.transaction_execution(hash)? {
            return Ok(Some(result));
        }

        // TODO: fetch from remote

        Ok(None)
    }

    fn transaction_executions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TypedTransactionExecutionInfo>>> {
        if let Some(result) = self.local_db.transaction_executions_by_block(block_id)? {
            return Ok(Some(result));
        }

        // TODO: fetch from remote

        Ok(None)
    }

    fn transaction_executions_in_range(
        &self,
        range: Range<TxNumber>,
    ) -> ProviderResult<Vec<TypedTransactionExecutionInfo>> {
        let _ = range;
        unimplemented!()
    }
}

impl<Db: Database> ReceiptProvider for ForkedProvider<Db> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        if let Some(result) = self.local_db.receipt_by_hash(hash)? {
            return Ok(Some(result));
        }

        if let Some(value) = self.fork_db.db.receipt_by_hash(hash)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_block_by_transaction_hash(hash)? {
            let value = self.fork_db.db.receipt_by_hash(hash)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn receipts_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Receipt>>> {
        if let Some(result) = self.local_db.receipts_by_block(block_id)? {
            return Ok(Some(result));
        }

        if let Some(value) = self.fork_db.db.receipts_by_block(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.receipts_by_block(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> BlockEnvProvider for ForkedProvider<Db> {
    fn block_env_at(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<BlockEnv>> {
        if let Some(result) = self.local_db.block_env_at(block_id)? {
            return Ok(Some(result));
        }

        if let Some(value) = self.fork_db.db.block_env_at(block_id)? {
            return Ok(Some(value));
        }

        if self.fork_db.fetch_historical_blocks(block_id)? {
            let value = self.fork_db.db.block_env_at(block_id)?.unwrap();
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> BlockWriter for ForkedProvider<Db> {
    fn insert_block_with_states_and_receipts(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        self.local_db.insert_block_with_states_and_receipts(block, states, receipts, executions)
    }
}

impl<Db: Database> StageCheckpointProvider for ForkedProvider<Db> {
    fn checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        self.local_db.checkpoint(id)
    }

    fn set_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        self.local_db.set_checkpoint(id, block_number)
    }
}
