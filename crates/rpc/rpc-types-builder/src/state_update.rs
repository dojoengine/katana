use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::ProviderResult;
use katana_rpc_types::state_update::{ConfirmedStateUpdate, StateDiff};

/// A builder for building RPC state update type.
#[derive(Debug)]
pub struct StateUpdateBuilder<P> {
    provider: P,
    block_id: BlockHashOrNumber,
}

impl<P> StateUpdateBuilder<P> {
    pub fn new(block_id: BlockHashOrNumber, provider: P) -> Self {
        Self { provider, block_id }
    }
}

impl<P> StateUpdateBuilder<P>
where
    P: BlockHashProvider + BlockNumberProvider + HeaderProvider + StateUpdateProvider,
{
    /// Builds a state update for the given block.
    pub fn build(self) -> ProviderResult<Option<ConfirmedStateUpdate>> {
        let Some(block_hash) = self.provider.block_hash_by_id(self.block_id)? else {
            return Ok(None);
        };

        let Some(block_num) = self.provider.block_number_by_hash(block_hash)? else {
            return Ok(None);
        };

        let Some(new_root) = self.provider.header_by_number(block_num)?.map(|h| h.state_root)
        else {
            return Ok(None);
        };

        let old_root = {
            match block_num {
                0 => Felt::ZERO,
                _ => match self.provider.header_by_number(block_num - 1)? {
                    Some(header) => header.state_root,
                    None => return Ok(None),
                },
            }
        };

        let state_diff: StateDiff = match self.provider.state_update(self.block_id)? {
            Some(diff) => diff.into(),
            None => return Ok(None),
        };

        Ok(Some(ConfirmedStateUpdate { block_hash, new_root, old_root, state_diff }))
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::block::{
        BlockNumber, FinalityStatus, Header, SealedBlock, SealedBlockWithStatus,
    };
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::state::StateUpdatesWithClasses;
    use katana_primitives::{ContractAddress, Felt};
    use katana_provider::api::block::BlockWriter;
    use katana_provider::api::state::HistoricalStateRetentionProvider;
    use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};

    use super::StateUpdateBuilder;

    fn create_stored_block(block_number: u64) -> SealedBlockWithStatus {
        let header = Header {
            number: block_number,
            parent_hash: Felt::from(block_number.saturating_sub(1)),
            timestamp: block_number,
            sequencer_address: ContractAddress::default(),
            l1_gas_prices: Default::default(),
            l1_data_gas_prices: Default::default(),
            l2_gas_prices: Default::default(),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: Default::default(),
            state_root: Felt::from(block_number + 1000),
            state_diff_commitment: Felt::ZERO,
            transactions_commitment: Felt::ZERO,
            receipts_commitment: Felt::ZERO,
            events_commitment: Felt::ZERO,
            transaction_count: 0,
            events_count: 0,
            state_diff_length: 0,
        };

        SealedBlockWithStatus {
            block: SealedBlock { hash: Felt::from(block_number), header, body: Vec::new() },
            status: FinalityStatus::AcceptedOnL2,
        }
    }

    fn create_provider_with_blocks(max_block: u64) -> DbProviderFactory {
        let provider_factory = DbProviderFactory::new_in_memory();
        let provider_mut = provider_factory.provider_mut();

        for block_number in 0..=max_block {
            provider_mut
                .insert_block_with_states_and_receipts(
                    create_stored_block(block_number),
                    StateUpdatesWithClasses::default(),
                    Vec::new(),
                    Vec::new(),
                )
                .expect("failed to insert block");
        }

        provider_mut.commit().expect("failed to commit");
        provider_factory
    }

    #[test]
    fn state_update_builder_should_exists_for_pruned_block_state() {
        let provider_factory = create_provider_with_blocks(3);

        let provider_mut = provider_factory.provider_mut();
        provider_mut.set_earliest_available_state_block(2).unwrap();
        provider_mut.commit().unwrap();

        let provider = provider_factory.provider();
        let block_id = 1u64.into(); // note: the earliest available block is 2

        let state_update = StateUpdateBuilder::new(block_id, provider).build().unwrap();
        let state_update = state_update.expect("state update should be available");

        assert_eq!(state_update.new_root, Felt::from(1001u64));
        assert_eq!(state_update.old_root, Felt::from(1000u64));
    }

    #[test]
    fn state_update_builder_still_builds_at_prune_boundary() {
        let provider_factory = create_provider_with_blocks(5);

        let block_id: BlockNumber = 2;

        let provider_mut = provider_factory.provider_mut();
        provider_mut.set_earliest_available_state_block(block_id).unwrap();
        provider_mut.commit().unwrap();

        let provider = provider_factory.provider();
        let state_update = StateUpdateBuilder::new(block_id.into(), provider).build().unwrap();

        assert!(state_update.is_some(), "state update at first retained block should be available");
    }
}
