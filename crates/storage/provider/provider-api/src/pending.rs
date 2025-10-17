use katana_primitives::block::PartialHeader;
use katana_primitives::env::BlockEnv;
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::{TxHash, TxWithHash};

use crate::state::StateProvider;
use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait PendingDataProvider: Send + Sync {
    fn state(&self) -> ProviderResult<Option<Box<dyn StateProvider>>>;

    // returns block header, transactions, and receipts
    fn block_header(&self) -> ProviderResult<Option<PartialHeader>>;

    fn block_env(&self) -> ProviderResult<Option<BlockEnv>>;

    fn block_transaction_count(&self) -> ProviderResult<Option<u64>>;

    fn transaction_by_block_id_and_index(&self) -> ProviderResult<Option<TxWithHash>>;

    fn transaction(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>>;

    fn receipt(&self, hash: TxHash) -> ProviderResult<Option<Receipt>>;

    fn state_update(&self) -> ProviderResult<Option<()>>;
}
