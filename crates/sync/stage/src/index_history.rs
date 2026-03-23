use std::collections::BTreeSet;

use futures::future::BoxFuture;
use katana_db::abstraction::{Database, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::list::BlockChangeList;
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey};
use katana_db::tables;
use katana_primitives::block::BlockNumber;
use katana_primitives::contract::ContractAddress;
use katana_provider::api::state::HistoricalStateRetentionProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_tasks::TaskSpawner;
use tracing::info_span;

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

pub const INDEX_HISTORY_STAGE_ID: &str = "IndexHistory";

/// A stage for building historical state indices.
///
/// This stage processes blocks that have been stored by the [`Blocks`](crate::blocks::Blocks)
/// stage and builds the historical state indices (storage change sets, nonce/class change
/// histories, contract info) for each block.
#[derive(Debug)]
pub struct IndexHistory {
    provider: DbProviderFactory,
    task_spawner: TaskSpawner,
}

impl IndexHistory {
    /// Create a new [`IndexHistory`] stage.
    pub fn new(provider: DbProviderFactory, task_spawner: TaskSpawner) -> Self {
        Self { provider, task_spawner }
    }
}

impl Stage for IndexHistory {
    fn id(&self) -> &'static str {
        INDEX_HISTORY_STAGE_ID
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let span = info_span!(target: "stage", "index_history", from = %input.from(), to = %input.to());
            let _enter = span.enter();

            let provider = self.provider.clone();
            let from = input.from();
            let to = input.to();

            // First sync: history tables are empty, so we can skip all read-modify-write
            // overhead and use bulk append writes instead.
            let first_sync = from == 0;

            self.task_spawner
                .spawn_blocking(move || {
                    let provider_mut = provider.provider_mut();

                    if first_sync {
                        let blocks = (from..=to)
                            .map(|block_number| {
                                let state_updates = provider_mut
                                    .state_update(block_number.into())?
                                    .ok_or(Error::MissingStateUpdate(block_number));
                                state_updates.map(|su| (block_number, su))
                            })
                            .collect::<Result<Vec<_>, _>>()?;

                        provider_mut.insert_state_history_bulk(blocks)?;
                    } else {
                        for block_number in from..=to {
                            let state_updates = provider_mut
                                .state_update(block_number.into())?
                                .ok_or(Error::MissingStateUpdate(block_number))?;

                            provider_mut.insert_state_history(block_number, &state_updates)?;
                        }
                    }

                    provider_mut.commit()?;
                    Result::<(), Error>::Ok(())
                })
                .await
                .map_err(Error::TaskJoinError)??;

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async move {
            let Some(range) = input.prune_range() else {
                return Ok(PruneOutput::default());
            };

            let pruned_count = prune_state_history(&self.provider, range.start, range.end)?;
            update_historical_state_retention(&self.provider, range.end)?;

            Ok(PruneOutput { pruned_count })
        })
    }
}

fn prune_state_history(
    provider: &DbProviderFactory,
    start: BlockNumber,
    keep_from: BlockNumber,
) -> Result<u64, Error> {
    let tx = provider.db().tx_mut().map_err(Error::Database)?;

    let (storage_keys, nonce_addrs, class_addrs) =
        collect_touched_history_keys(&tx, start, keep_from)?;

    for storage_key in storage_keys {
        compact_storage_changeset(&tx, storage_key, keep_from)?;
    }

    for contract_address in nonce_addrs {
        compact_contract_info_changeset(&tx, contract_address, keep_from, true, false)?;
    }

    for contract_address in class_addrs {
        compact_contract_info_changeset(&tx, contract_address, keep_from, false, true)?;
    }

    let mut pruned_count = 0u64;
    for block_number in start..keep_from {
        pruned_count +=
            delete_block_history_entries::<tables::StorageChangeHistory, _>(&tx, block_number)?;

        pruned_count +=
            delete_block_history_entries::<tables::NonceChangeHistory, _>(&tx, block_number)?;

        pruned_count +=
            delete_block_history_entries::<tables::ClassChangeHistory, _>(&tx, block_number)?;

        pruned_count +=
            delete_block_history_entries::<tables::ClassDeclarations, _>(&tx, block_number)?;

        pruned_count += delete_block_history_entries::<tables::MigratedCompiledClassHashes, _>(
            &tx,
            block_number,
        )?;
    }

    tx.commit().map_err(Error::Database)?;
    Ok(pruned_count)
}

fn update_historical_state_retention(
    provider: &DbProviderFactory,
    keep_from: BlockNumber,
) -> Result<(), Error> {
    let provider_mut = provider.provider_mut();
    let current = provider_mut.earliest_available_state_block()?;

    let next = current.map_or(keep_from, |current| current.max(keep_from));
    if current != Some(next) {
        provider_mut.set_earliest_available_state_block(next)?;
        provider_mut.commit()?;
    }

    Ok(())
}

type TouchedHistoryKeys =
    (BTreeSet<ContractStorageKey>, BTreeSet<ContractAddress>, BTreeSet<ContractAddress>);

// Collects all the keys that were updated in the historical state between `start` and `keep_from`
// blocks.
fn collect_touched_history_keys<Tx: DbTx>(
    tx: &Tx,
    start: BlockNumber,
    keep_from: BlockNumber,
) -> Result<TouchedHistoryKeys, Error> {
    let mut storage_keys = BTreeSet::new();
    let mut nonce_addrs = BTreeSet::new();
    let mut class_addrs = BTreeSet::new();

    for block in start..keep_from {
        let get_storage_keys = || -> Result<Vec<ContractStorageKey>, Error> {
            let mut keys = Vec::new();
            let mut cursor = tx.cursor_dup::<tables::StorageChangeHistory>()?;

            if let Some(walker) = cursor.walk_dup(Some(block), None)? {
                for entry in walker {
                    let (_, entry) = entry?;
                    keys.push(entry.key.clone());
                }
            }

            Ok(keys)
        };

        let get_nonce_addrs = || -> Result<Vec<ContractAddress>, Error> {
            let mut addrs = Vec::new();
            let mut cursor = tx.cursor_dup::<tables::NonceChangeHistory>()?;

            if let Some(walker) = cursor.walk_dup(Some(block), None)? {
                for entry in walker {
                    let (_, entry) = entry?;
                    addrs.push(entry.contract_address);
                }
            }

            Ok(addrs)
        };

        let get_class_addrs = || -> Result<Vec<ContractAddress>, Error> {
            let mut addrs = Vec::new();
            let mut cursor = tx.cursor_dup::<tables::ClassChangeHistory>()?;

            if let Some(walker) = cursor.walk_dup(Some(block), None)? {
                for entry in walker {
                    let (_, entry) = entry?;
                    addrs.push(entry.contract_address);
                }
            }

            Ok(addrs)
        };

        // Run the three independent table scans in parallel since they access
        // different tables and DbTx is Send + Sync.
        let (storage_res, (nonce_res, class_res)) =
            rayon::join(get_storage_keys, || rayon::join(get_nonce_addrs, get_class_addrs));

        storage_keys.extend(storage_res?);
        nonce_addrs.extend(nonce_res?);
        class_addrs.extend(class_res?);
    }

    Ok((storage_keys, nonce_addrs, class_addrs))
}

fn compact_storage_changeset<Tx: DbTxMut>(
    tx: &Tx,
    key: ContractStorageKey,
    keep_from: BlockNumber,
) -> Result<(), Error> {
    let Some(mut block_list) = tx.get::<tables::StorageChangeSet>(key.clone())? else {
        return Ok(());
    };

    if let Some(anchor_block) = block_list.last_change_before(keep_from) {
        if !block_list.contains(keep_from) {
            let entry = storage_history_entry(tx, anchor_block, &key)?;
            tx.put::<tables::StorageChangeHistory>(keep_from, entry)?;
            block_list.insert(keep_from);
        }
    }

    block_list.remove_range(..keep_from);

    if block_list.is_empty() {
        tx.delete::<tables::StorageChangeSet>(key, None)?;
    } else {
        tx.put::<tables::StorageChangeSet>(key, block_list)?;
    }

    Ok(())
}

fn compact_contract_info_changeset<Tx: DbTxMut>(
    tx: &Tx,
    address: ContractAddress,
    keep_from: BlockNumber,
    compact_nonce_history: bool,
    compact_class_history: bool,
) -> Result<(), Error> {
    let Some(mut changes) = tx.get::<tables::ContractInfoChangeSet>(address)? else {
        return Ok(());
    };

    if compact_nonce_history {
        compact_nonce_history_list(tx, address, &mut changes.nonce_change_list, keep_from)?;
    }

    if compact_class_history {
        compact_class_history_list(tx, address, &mut changes.class_change_list, keep_from)?;
    }

    if changes.class_change_list.is_empty() && changes.nonce_change_list.is_empty() {
        tx.delete::<tables::ContractInfoChangeSet>(address, None)?;
    } else {
        tx.put::<tables::ContractInfoChangeSet>(address, changes)?;
    }

    Ok(())
}

fn compact_nonce_history_list<Tx: DbTxMut>(
    tx: &Tx,
    contract_address: ContractAddress,
    block_list: &mut BlockChangeList,
    keep_from: BlockNumber,
) -> Result<(), Error> {
    if let Some(anchor_block) = block_list.last_change_before(keep_from) {
        if !block_list.contains(keep_from) {
            let entry = nonce_history_entry(tx, anchor_block, contract_address)?;
            tx.put::<tables::NonceChangeHistory>(keep_from, entry)?;
            block_list.insert(keep_from);
        }
    }

    block_list.remove_range(..keep_from);
    Ok(())
}

fn compact_class_history_list<Tx: DbTxMut>(
    tx: &Tx,
    contract_address: ContractAddress,
    block_list: &mut BlockChangeList,
    keep_from: BlockNumber,
) -> Result<(), Error> {
    if let Some(anchor_block) = block_list.last_change_before(keep_from) {
        if !block_list.contains(keep_from) {
            let entry = class_history_entry(tx, anchor_block, contract_address)?;
            tx.put::<tables::ClassChangeHistory>(keep_from, entry)?;
            block_list.insert(keep_from);
        }
    }

    block_list.remove_range(..keep_from);
    Ok(())
}

fn storage_history_entry<Tx: DbTx>(
    tx: &Tx,
    block_number: BlockNumber,
    key: &ContractStorageKey,
) -> Result<ContractStorageEntry, Error> {
    let mut cursor = tx.cursor_dup::<tables::StorageChangeHistory>().map_err(Error::Database)?;
    let entry = cursor
        .seek_by_key_subkey(block_number, key.clone())
        .map_err(Error::Database)?
        .ok_or(ProviderError::MissingStorageChangeEntry {
            block: block_number,
            contract_address: key.contract_address,
            storage_key: key.key,
        })?;

    // cursor.seek_by_key_subkey(block_number, key) will return the first item whose `key` is >= or
    // equal to the specified `key`. so we have to check if the returned entry matches the key we're
    // looking for.
    if entry.key.contract_address == key.contract_address && entry.key.key == key.key {
        Ok(entry)
    } else {
        Err(ProviderError::MissingStorageChangeEntry {
            block: block_number,
            contract_address: key.contract_address,
            storage_key: key.key,
        }
        .into())
    }
}

fn nonce_history_entry<Tx: DbTx>(
    tx: &Tx,
    block_number: BlockNumber,
    contract_address: ContractAddress,
) -> Result<ContractNonceChange, Error> {
    let mut cursor = tx.cursor_dup::<tables::NonceChangeHistory>().map_err(Error::Database)?;
    let entry = cursor
        .seek_by_key_subkey(block_number, contract_address)
        .map_err(Error::Database)?
        .ok_or(ProviderError::MissingContractNonceChangeEntry {
            block: block_number,
            contract_address,
        })?;

    // cursor.seek_by_key_subkey(block_number, key) will return the first item whose `key` is >= or
    // equal to the specified `key`. so we have to check if the returned entry matches the key we're
    // looking for.
    if entry.contract_address == contract_address {
        Ok(entry)
    } else {
        Err(ProviderError::MissingContractNonceChangeEntry {
            block: block_number,
            contract_address,
        }
        .into())
    }
}

fn class_history_entry<Tx: DbTx>(
    tx: &Tx,
    block_number: BlockNumber,
    contract_address: ContractAddress,
) -> Result<ContractClassChange, Error> {
    let mut cursor = tx.cursor_dup::<tables::ClassChangeHistory>().map_err(Error::Database)?;
    let entry = cursor
        .seek_by_key_subkey(block_number, contract_address)
        .map_err(Error::Database)?
        .ok_or(ProviderError::MissingContractClassChangeEntry {
            block: block_number,
            contract_address,
        })?;

    // cursor.seek_by_key_subkey(block_number, key) will return the first item whose `key` is >= or
    // equal to the specified `key`. so we have to check if the returned entry matches the key we're
    // looking for.
    if entry.contract_address == contract_address {
        Ok(entry)
    } else {
        Err(ProviderError::MissingContractClassChangeEntry {
            block: block_number,
            contract_address,
        }
        .into())
    }
}

fn delete_block_history_entries<Tb, Tx>(tx: &Tx, block_number: BlockNumber) -> Result<u64, Error>
where
    Tb: tables::DupSort<Key = BlockNumber>,
    Tx: DbTxMut,
{
    let mut deleted = 0u64;
    let mut cursor = tx.cursor_dup_mut::<Tb>()?;
    let Some(mut walker) = cursor.walk_dup(Some(block_number), None)? else {
        return Ok(0);
    };

    while let Some(entry) = walker.next() {
        let _ = entry?;
        walker.delete_current()?;
        deleted += 1;
    }

    Ok(deleted)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Database(#[from] katana_db::error::DatabaseError),

    #[error("Missing state update for block {0}")]
    MissingStateUpdate(BlockNumber),

    #[error("task join error: {0}")]
    TaskJoinError(katana_tasks::JoinError),
}
