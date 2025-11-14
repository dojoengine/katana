use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};

use katana_db::abstraction::{DbTx, DbTxMut};
use katana_db::models::block::StoredBlockBodyIndices;
use katana_fork::BackendClient;
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

use super::db::{self, DbProvider};
use crate::ProviderResult;

mod state;
mod trie;

#[derive(Debug)]
pub struct ForkedProvider<Tx: DbTx> {
    block_id: BlockNumber,
    provider: DbProvider<Tx>,
    backend_client: BackendClient,
}

impl<Tx: DbTx> ForkedProvider<Tx> {
    pub fn new(
        block_id: BlockNumber,
        provider: DbProvider<Tx>,
        backend_client: BackendClient,
    ) -> Self {
        Self { block_id, provider, backend_client }
    }

    /// Returns the block id this fork provider is based on.
    pub fn block_id(&self) -> BlockNumber {
        self.block_id
    }
}

impl<Tx: DbTx> BlockNumberProvider for ForkedProvider<Tx> {
    fn block_number_by_hash(&self, hash: BlockHash) -> ProviderResult<Option<BlockNumber>> {
        self.provider.block_number_by_hash(hash)
    }

    fn latest_number(&self) -> ProviderResult<BlockNumber> {
        self.provider.latest_number()
    }
}

impl<Tx: DbTx> BlockHashProvider for ForkedProvider<Tx> {
    fn latest_hash(&self) -> ProviderResult<BlockHash> {
        self.provider.latest_hash()
    }

    fn block_hash_by_num(&self, num: BlockNumber) -> ProviderResult<Option<BlockHash>> {
        self.provider.block_hash_by_num(num)
    }
}

impl<Tx: DbTx> HeaderProvider for ForkedProvider<Tx> {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        self.provider.header(id)
    }
}

impl<Tx: DbTx> BlockProvider for ForkedProvider<Tx> {
    fn block_body_indices(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        self.provider.block_body_indices(id)
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Block>> {
        self.provider.block(id)
    }

    fn block_with_tx_hashes(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BlockWithTxHashes>> {
        self.provider.block_with_tx_hashes(id)
    }

    fn blocks_in_range(&self, range: RangeInclusive<u64>) -> ProviderResult<Vec<Block>> {
        self.provider.blocks_in_range(range)
    }
}

impl<Tx: DbTx> BlockStatusProvider for ForkedProvider<Tx> {
    fn block_status(&self, id: BlockHashOrNumber) -> ProviderResult<Option<FinalityStatus>> {
        self.provider.block_status(id)
    }
}

impl<Tx: DbTx> StateUpdateProvider for ForkedProvider<Tx> {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        self.provider.state_update(block_id)
    }

    fn declared_classes(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>> {
        self.provider.declared_classes(block_id)
    }

    fn deployed_contracts(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ContractAddress, ClassHash>>> {
        self.provider.deployed_contracts(block_id)
    }
}

impl<Tx: DbTx> TransactionProvider for ForkedProvider<Tx> {
    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>> {
        self.provider.transaction_by_hash(hash)
    }

    fn transactions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TxWithHash>>> {
        self.provider.transactions_by_block(block_id)
    }

    fn transaction_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxWithHash>> {
        self.provider.transaction_in_range(range)
    }

    fn transaction_block_num_and_hash(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
        self.provider.transaction_block_num_and_hash(hash)
    }

    fn transaction_by_block_and_idx(
        &self,
        block_id: BlockHashOrNumber,
        idx: u64,
    ) -> ProviderResult<Option<TxWithHash>> {
        self.provider.transaction_by_block_and_idx(block_id, idx)
    }

    fn transaction_count_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<u64>> {
        self.provider.transaction_count_by_block(block_id)
    }
}

impl<Tx: DbTx> TransactionsProviderExt for ForkedProvider<Tx> {
    fn transaction_hashes_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxHash>> {
        self.provider.transaction_hashes_in_range(range)
    }

    fn total_transactions(&self) -> ProviderResult<usize> {
        self.provider.total_transactions()
    }
}

impl<Tx: DbTx> TransactionStatusProvider for ForkedProvider<Tx> {
    fn transaction_status(&self, hash: TxHash) -> ProviderResult<Option<FinalityStatus>> {
        self.provider.transaction_status(hash)
    }
}

impl<Tx: DbTx> TransactionTraceProvider for ForkedProvider<Tx> {
    fn transaction_execution(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<TypedTransactionExecutionInfo>> {
        self.provider.transaction_execution(hash)
    }

    fn transaction_executions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TypedTransactionExecutionInfo>>> {
        self.provider.transaction_executions_by_block(block_id)
    }

    fn transaction_executions_in_range(
        &self,
        range: Range<TxNumber>,
    ) -> ProviderResult<Vec<TypedTransactionExecutionInfo>> {
        self.provider.transaction_executions_in_range(range)
    }
}

impl<Tx: DbTx> ReceiptProvider for ForkedProvider<Tx> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        self.provider.receipt_by_hash(hash)
    }

    fn receipts_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Receipt>>> {
        self.provider.receipts_by_block(block_id)
    }
}

impl<Tx: DbTx> BlockEnvProvider for ForkedProvider<Tx> {
    fn block_env_at(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<BlockEnv>> {
        self.provider.block_env_at(block_id)
    }
}

impl<Tx: DbTxMut> BlockWriter for ForkedProvider<Tx> {
    fn insert_block_with_states_and_receipts(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        self.provider.insert_block_with_states_and_receipts(block, states, receipts, executions)
    }
}

impl<Tx: DbTxMut> StageCheckpointProvider for ForkedProvider<Tx> {
    fn checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        self.provider.checkpoint(id)
    }

    fn set_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        self.provider.set_checkpoint(id, block_number)
    }
}
