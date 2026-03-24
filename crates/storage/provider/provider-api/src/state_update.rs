use std::collections::BTreeMap;

use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::ContractAddress;

use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait StateUpdateProvider: Send + Sync {
    /// Returns the canonical state update at the given block.
    ///
    /// This is the exact per-block diff and is independent from historical state retention.
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>>;

    /// Returns the declared Sierra class hashes at the given block.
    fn declared_classes(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>>;

    /// Returns all deployed contracts at the given block.
    fn deployed_contracts(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ContractAddress, ClassHash>>>;
}
