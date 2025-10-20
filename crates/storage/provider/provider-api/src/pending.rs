use katana_primitives::block::PartialHeader;
use katana_primitives::env::BlockEnv;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdates;
use katana_primitives::transaction::{TxHash, TxWithHash};

use crate::state::StateProvider;
use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait PendingBlockProvider: Send + Sync {
    fn block_header(&self) -> ProviderResult<Option<PartialHeader>>;

    fn block_env(&self) -> ProviderResult<Option<BlockEnv>>;

    fn transaction(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>>;

    fn receipt(&self, hash: TxHash) -> ProviderResult<Option<Receipt>>;

    fn state_update(&self) -> ProviderResult<Option<StateUpdates>>;
}
