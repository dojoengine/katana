use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use katana_chain_spec::ChainSpec;
use katana_executor::{ExecutionOutput, ExecutionResult, ExecutorFactory};
use katana_gas_price_oracle::GasPriceOracle;
use katana_primitives::block::{
    BlockHash, BlockNumber, FinalityStatus, Header, PartialHeader, SealedBlock,
    SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::env::BlockEnv;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::{Event, Receipt, ReceiptWithTxHash};
use katana_primitives::state::{compute_state_diff_hash, StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{TxHash, TxWithHash};
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use katana_primitives::{address, ContractAddress, Felt};
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::api::trie::TrieWriter;
use katana_provider::providers::EmptyStateProvider;
use katana_provider::{MutableProvider, ProviderFactory};
use katana_trie::bonsai::databases::HashMapDb;
use katana_trie::{
    compute_contract_state_hash, compute_merkle_root, ClassesTrie, CommitId, ContractLeaf,
    ContractsTrie, StoragesTrie,
};
use parking_lot::RwLock;
use rayon::prelude::*;
use starknet::macros::short_string;
use starknet_types_core::hash::{self, StarkHash};
use tracing::info;

pub mod storage;

use crate::backend::storage::{ProviderRO, ProviderRW};
use crate::env::BlockContextGenerator;
use crate::service::block_producer::{BlockProductionError, MinedBlockOutcome};
use crate::utils::get_current_timestamp;

pub(crate) const LOG_TARGET: &str = "katana::core::backend";

pub struct Backend<EF, PF> {
    pub chain_spec: Arc<ChainSpec>,
    pub storage: PF,
    pub block_context_generator: RwLock<BlockContextGenerator>,
    pub executor_factory: Arc<EF>,
    pub gas_oracle: GasPriceOracle,
}

impl<EF, PF> std::fmt::Debug for Backend<EF, PF> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("chain_spec", &self.chain_spec)
            .field("storage", &"..")
            .field("block_context_generator", &self.block_context_generator)
            .field("executor_factory", &"Arc<EF>")
            .field("gas_oracle", &self.gas_oracle)
            .finish()
    }
}

impl<EF, PF> Backend<EF, PF> {
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        storage: PF,
        gas_oracle: GasPriceOracle,
        executor_factory: EF,
    ) -> Self {
        Self {
            storage,
            chain_spec,
            gas_oracle,
            executor_factory: Arc::new(executor_factory),
            block_context_generator: RwLock::new(BlockContextGenerator::default()),
        }
    }
}

impl<EF, PF> Backend<EF, PF>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub fn update_block_env(&self, block_env: &mut BlockEnv) {
        let mut context_gen = self.block_context_generator.write();
        let current_timestamp_secs = get_current_timestamp().as_secs() as i64;

        let timestamp = if context_gen.next_block_start_time == 0 {
            (current_timestamp_secs + context_gen.block_timestamp_offset) as u64
        } else {
            let timestamp = context_gen.next_block_start_time;
            context_gen.block_timestamp_offset = timestamp as i64 - current_timestamp_secs;
            context_gen.next_block_start_time = 0;
            timestamp
        };

        block_env.number += 1;
        block_env.timestamp = timestamp;
        block_env.starknet_version = CURRENT_STARKNET_VERSION;

        // update the gas prices
        self.update_block_gas_prices(block_env);
    }

    /// Updates the gas prices in the block environment.
    pub fn update_block_gas_prices(&self, block_env: &mut BlockEnv) {
        block_env.l2_gas_prices = self.gas_oracle.l2_gas_prices();
        block_env.l1_gas_prices = self.gas_oracle.l1_gas_prices();
        block_env.l1_data_gas_prices = self.gas_oracle.l1_data_gas_prices();
    }
}

impl<EF, PF> Backend<EF, PF>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub fn init_genesis(&self, is_forking: bool) -> anyhow::Result<()> {
        match self.chain_spec.as_ref() {
            ChainSpec::Dev(cs) => self.init_dev_genesis(cs, is_forking),
            ChainSpec::Rollup(cs) => self.init_rollup_genesis(cs),
            ChainSpec::FullNode(_) => {
                // Full nodes sync from the network, so we skip genesis initialization
                info!("Full node mode: genesis initialization skipped, will sync from network");
                Ok(())
            }
        }
    }

    // TODO: add test for this function
    pub fn do_mine_block(
        &self,
        block_env: &BlockEnv,
        mut execution_output: ExecutionOutput,
    ) -> Result<MinedBlockOutcome, BlockProductionError> {
        let mut traces = Vec::with_capacity(execution_output.transactions.len());
        let mut receipts = Vec::with_capacity(execution_output.transactions.len());
        let mut transactions = Vec::with_capacity(execution_output.transactions.len());

        // only include successful transactions in the block
        for (tx, res) in execution_output.transactions {
            if let ExecutionResult::Success { receipt, trace } = res {
                traces.push(TypedTransactionExecutionInfo::new(receipt.r#type(), trace));
                receipts.push(ReceiptWithTxHash::new(tx.hash, receipt));
                transactions.push(tx);
            }
        }

        let tx_count = transactions.len();
        let tx_hashes = transactions.iter().map(|tx| tx.hash).collect::<Vec<_>>();

        let parent_hash = if block_env.number == 0 {
            BlockHash::ZERO
        } else {
            let parent_block_num = block_env.number - 1;
            self.storage
                .provider()
                .block_hash_by_num(parent_block_num)?
                .expect("qed; missing block hash for parent block")
        };

        // create a new block and compute its commitment
        let partial_header = PartialHeader {
            parent_hash,
            number: block_env.number,
            timestamp: block_env.timestamp,
            starknet_version: block_env.starknet_version,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            sequencer_address: block_env.sequencer_address,
            l2_gas_prices: block_env.l2_gas_prices.clone(),
            l1_gas_prices: block_env.l1_gas_prices.clone(),
            l1_data_gas_prices: block_env.l1_data_gas_prices.clone(),
        };

        let provider = self.storage.provider_mut();
        let block = commit_block(
            provider,
            partial_header,
            transactions,
            &receipts,
            &mut execution_output.states.state_updates,
        )?;

        let block = SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 };
        let block_hash = block.block.hash;
        let block_number = block.block.header.number;

        // TODO: maybe should change the arguments for insert_block_with_states_and_receipts to
        // accept ReceiptWithTxHash instead to avoid this conversion.
        let receipts = receipts.into_iter().map(|r| r.receipt).collect::<Vec<_>>();
        self.store_block(block, execution_output.states, receipts, traces)?;

        info!(target: LOG_TARGET, %block_number, %tx_count, "Block mined.");

        Ok(MinedBlockOutcome {
            block_hash,
            block_number,
            txs: tx_hashes,
            stats: execution_output.stats,
        })
    }

    fn store_block(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        traces: Vec<TypedTransactionExecutionInfo>,
    ) -> Result<(), BlockProductionError> {
        // Validate that all declared classes have their corresponding class artifacts
        if let Err(missing) = states.validate_classes() {
            return Err(BlockProductionError::InconsistentState(format!(
                "missing class artifacts for declared classes: {:#?}",
                missing,
            )));
        }

        let provider_mut = self.storage.provider_mut();
        provider_mut.insert_block_with_states_and_receipts(block, states, receipts, traces)?;
        provider_mut.commit()?;

        Ok(())
    }

    pub fn mine_empty_block(
        &self,
        block_env: &BlockEnv,
    ) -> Result<MinedBlockOutcome, BlockProductionError> {
        self.do_mine_block(block_env, Default::default())
    }

    fn init_dev_genesis(
        &self,
        chain_spec: &katana_chain_spec::dev::ChainSpec,
        is_forking: bool,
    ) -> anyhow::Result<()> {
        let provider = self.storage.provider();

        // check whether the genesis block has been initialized
        let local_hash = provider.block_hash_by_num(chain_spec.genesis.number)?;

        // NOTE: right now forking mode is not persistent, both `local_hash.is_some()` and
        // `is_forking` can never be true at the same time.
        if local_hash.is_some() && !is_forking {
            let local_hash = local_hash.unwrap();

            let genesis_block = chain_spec.block();
            let mut genesis_state_updates = chain_spec.state_updates();

            // commit the block but compute the trie using volatile storage so that it won't
            // overwrite the existing trie this is very hacky and we should find for a
            // much elegant solution.
            let committed_block = commit_genesis_block(
                GenesisTrieWriter,
                genesis_block.header.clone(),
                Vec::new(),
                &[],
                &mut genesis_state_updates.state_updates,
            )?;

            // check genesis should be the same
            if local_hash != committed_block.hash {
                return Err(anyhow!(
                    "Genesis block hash mismatch: expected {:#x}, got {local_hash:#x}",
                    committed_block.hash
                ));
            }

            info!(genesis_hash = %local_hash, "Genesis has already been initialized");
        } else {
            // Initialize the dev genesis block

            let block = chain_spec.block();
            let states = chain_spec.state_updates();

            let outcome = self.do_mine_block(
                &BlockEnv {
                    number: block.header.number,
                    timestamp: block.header.timestamp,
                    l2_gas_prices: block.header.l2_gas_prices,
                    l1_gas_prices: block.header.l1_gas_prices,
                    l1_data_gas_prices: block.header.l1_data_gas_prices,
                    sequencer_address: block.header.sequencer_address,
                    starknet_version: block.header.starknet_version,
                },
                ExecutionOutput { states, ..Default::default() },
            )?;

            info!(genesis_hash = %outcome.block_hash, "Genesis initialized");
        }

        Ok(())
    }

    fn init_rollup_genesis(
        &self,
        chain_spec: &katana_chain_spec::rollup::ChainSpec,
    ) -> anyhow::Result<()> {
        let provider = self.storage.provider();

        let block = chain_spec.block();
        let header = block.header.clone();

        let mut executor = self.executor_factory.with_state(EmptyStateProvider);
        executor.execute_block(block).context("failed to execute genesis block")?;

        let mut output =
            executor.take_execution_output().context("failed to get execution output")?;

        let mut traces = Vec::with_capacity(output.transactions.len());
        let mut receipts = Vec::with_capacity(output.transactions.len());
        let mut transactions = Vec::with_capacity(output.transactions.len());

        // only include successful transactions in the block
        for (tx, res) in output.transactions {
            if let ExecutionResult::Success { receipt, trace } = res {
                traces.push(TypedTransactionExecutionInfo::new(receipt.r#type(), trace));
                receipts.push(ReceiptWithTxHash::new(tx.hash, receipt));
                transactions.push(tx);
            }
        }

        // Check whether the genesis block has been initialized or not.
        if let Some(local_hash) = provider.block_hash_by_num(header.number)? {
            // commit the block but compute the trie using volatile storage so that it won't
            // overwrite the existing trie this is very hacky and we should find for a
            // much elegant solution.
            let block = commit_genesis_block(
                GenesisTrieWriter,
                header.clone(),
                transactions.clone(),
                &receipts,
                &mut output.states.state_updates,
            )?;

            let provided_genesis_hash = block.hash;
            if provided_genesis_hash != local_hash {
                return Err(anyhow!(
                    "Genesis block hash mismatch: local hash {local_hash:#x} is different than \
                     the provided genesis hash {provided_genesis_hash:#x}",
                ));
            }

            info!("Genesis has already been initialized");
        } else {
            let block = commit_genesis_block(
                self.storage.provider_mut(),
                header,
                transactions,
                &receipts,
                &mut output.states.state_updates,
            )?;

            let block = SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 };

            // TODO: maybe should change the arguments for insert_block_with_states_and_receipts to
            // accept ReceiptWithTxHash instead to avoid this conversion.
            let receipts = receipts.into_iter().map(|r| r.receipt).collect::<Vec<_>>();
            self.store_block(block, output.states, receipts, traces)?;

            info!("Genesis initialized");
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UncommittedBlock<'a, P: TrieWriter> {
    header: PartialHeader,
    transactions: Vec<TxWithHash>,
    receipts: &'a [ReceiptWithTxHash],
    state_updates: &'a StateUpdates,
    provider: P,
}

impl<'a, P: TrieWriter> UncommittedBlock<'a, P> {
    pub fn new(
        header: PartialHeader,
        transactions: Vec<TxWithHash>,
        receipts: &'a [ReceiptWithTxHash],
        state_updates: &'a StateUpdates,
        trie_provider: P,
    ) -> Self {
        Self { header, transactions, receipts, state_updates, provider: trie_provider }
    }

    pub fn commit(self) -> SealedBlock {
        // get the hash of the latest committed block
        let parent_hash = self.header.parent_hash;
        let events_count = self.receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();
        let transaction_count = self.transactions.len() as u32;
        let state_diff_length = self.state_updates.len() as u32;

        // optimisation 1
        let state_root = self.compute_new_state_root();
        let transactions_commitment = self.compute_transaction_commitment();
        let events_commitment = self.compute_event_commitment();
        let receipts_commitment = self.compute_receipt_commitment();
        let state_diff_commitment = self.compute_state_diff_commitment();

        let header = Header {
            state_root,
            parent_hash,
            events_count,
            state_diff_length,
            transaction_count,
            events_commitment,
            receipts_commitment,
            state_diff_commitment,
            transactions_commitment,
            number: self.header.number,
            timestamp: self.header.timestamp,
            l1_da_mode: self.header.l1_da_mode,
            l2_gas_prices: self.header.l2_gas_prices,
            l1_gas_prices: self.header.l1_gas_prices,
            l1_data_gas_prices: self.header.l1_data_gas_prices,
            sequencer_address: self.header.sequencer_address,
            starknet_version: self.header.starknet_version,
        };

        let hash = header.compute_hash();

        SealedBlock { hash, header, body: self.transactions }
    }

    pub fn commit_parallel(self) -> SealedBlock {
        // get the hash of the latest committed block
        let parent_hash = self.header.parent_hash;
        let events_count = self.receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();
        let transaction_count = self.transactions.len() as u32;
        let state_diff_length = self.state_updates.len() as u32;

        let mut state_root = Felt::default();
        let mut transactions_commitment = Felt::default();
        let mut events_commitment = Felt::default();
        let mut receipts_commitment = Felt::default();
        let mut state_diff_commitment = Felt::default();

        rayon::scope(|s| {
            s.spawn(|_| state_root = self.compute_new_state_root());
            s.spawn(|_| transactions_commitment = self.compute_transaction_commitment());
            s.spawn(|_| events_commitment = self.compute_event_commitment_parallel());
            s.spawn(|_| receipts_commitment = self.compute_receipt_commitment_parallel());
            s.spawn(|_| state_diff_commitment = self.compute_state_diff_commitment());
        });

        let header = Header {
            state_root,
            parent_hash,
            events_count,
            state_diff_length,
            transaction_count,
            events_commitment,
            receipts_commitment,
            state_diff_commitment,
            transactions_commitment,
            number: self.header.number,
            timestamp: self.header.timestamp,
            l1_da_mode: self.header.l1_da_mode,
            l2_gas_prices: self.header.l2_gas_prices,
            l1_gas_prices: self.header.l1_gas_prices,
            l1_data_gas_prices: self.header.l1_data_gas_prices,
            sequencer_address: self.header.sequencer_address,
            starknet_version: self.header.starknet_version,
        };

        let hash = header.compute_hash();

        SealedBlock { hash, header, body: self.transactions }
    }

    fn compute_transaction_commitment(&self) -> Felt {
        let tx_hashes = self.transactions.iter().map(|t| t.hash).collect::<Vec<TxHash>>();
        compute_merkle_root::<hash::Poseidon>(&tx_hashes).unwrap()
    }

    fn compute_receipt_commitment(&self) -> Felt {
        let receipt_hashes = self.receipts.iter().map(|r| r.compute_hash()).collect::<Vec<Felt>>();
        compute_merkle_root::<hash::Poseidon>(&receipt_hashes).unwrap()
    }

    fn compute_receipt_commitment_parallel(&self) -> Felt {
        let receipt_hashes =
            self.receipts.par_iter().map(|r| r.compute_hash()).collect::<Vec<Felt>>();
        compute_merkle_root::<hash::Poseidon>(&receipt_hashes).unwrap()
    }

    fn compute_state_diff_commitment(&self) -> Felt {
        compute_state_diff_hash(self.state_updates.clone())
    }

    fn compute_event_commitment(&self) -> Felt {
        // h(emitter_address, tx_hash, h(keys), h(data))
        fn event_hash(tx: TxHash, event: &Event) -> Felt {
            let keys_hash = hash::Poseidon::hash_array(&event.keys);
            let data_hash = hash::Poseidon::hash_array(&event.data);
            hash::Poseidon::hash_array(&[tx, event.from_address.into(), keys_hash, data_hash])
        }

        // the iterator will yield all events from all the receipts, each one paired with the
        // transaction hash that emitted it: (tx hash, event).
        let events = self.receipts.iter().flat_map(|r| r.events().iter().map(|e| (r.tx_hash, e)));
        let mut hashes = Vec::new();
        for (tx, event) in events {
            let event_hash = event_hash(tx, event);
            hashes.push(event_hash);
        }

        // compute events commitment
        compute_merkle_root::<hash::Poseidon>(&hashes).unwrap()
    }

    fn compute_event_commitment_parallel(&self) -> Felt {
        // h(emitter_address, tx_hash, h(keys), h(data))
        fn event_hash(tx: TxHash, event: &Event) -> Felt {
            let keys_hash = hash::Poseidon::hash_array(&event.keys);
            let data_hash = hash::Poseidon::hash_array(&event.data);
            hash::Poseidon::hash_array(&[tx, event.from_address.into(), keys_hash, data_hash])
        }

        // the iterator will yield all events from all the receipts, each one paired with the
        // transaction hash that emitted it: (tx hash, event).
        let events = self.receipts.iter().flat_map(|r| r.events().iter().map(|e| (r.tx_hash, e)));
        let hashes = events
            .par_bridge()
            .into_par_iter()
            .map(|(tx, event)| event_hash(tx, event))
            .collect::<Vec<_>>();

        // compute events commitment
        compute_merkle_root::<hash::Poseidon>(&hashes).unwrap()
    }

    // state_commitment = hPos("STARKNET_STATE_V0", contract_trie_root, class_trie_root)
    fn compute_new_state_root(&self) -> Felt {
        let class_trie_root = self
            .provider
            .trie_insert_declared_classes(self.header.number, &self.state_updates.declared_classes)
            .expect("failed to update class trie");

        let contract_trie_root = self
            .provider
            .trie_insert_contract_updates(self.header.number, self.state_updates)
            .expect("failed to update contract trie");

        hash::Poseidon::hash_array(&[
            short_string!("STARKNET_STATE_V0"),
            contract_trie_root,
            class_trie_root,
        ])
    }
}

// TODO: create a dedicated struct for this contract.
// https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1
fn update_block_hash_registry_contract(
    provider: impl BlockHashProvider,
    state_updates: &mut StateUpdates,
    block_number: BlockNumber,
) -> Result<(), BlockProductionError> {
    const STORED_BLOCK_HASH_BUFFER: u64 = 10;

    if block_number >= STORED_BLOCK_HASH_BUFFER {
        let block_number = block_number - STORED_BLOCK_HASH_BUFFER;
        let block_hash = provider.block_hash_by_num(block_number)?;

        // When in forked mode, we might not have the older block hash in the database. This
        // could be the case where the `block_number - STORED_BLOCK_HASH_BUFFER` is
        // earlier than the forked block, which right now, Katana doesn't
        // yet have the ability to fetch older blocks on the database level. So, we default to
        // `BlockHash::ZERO` in this case.
        //
        // TODO: Fix quick!
        let block_hash = block_hash.unwrap_or(BlockHash::ZERO);

        let storages = state_updates.storage_updates.entry(address!("0x1")).or_default();
        storages.insert(block_number.into(), block_hash);
    }

    Ok(())
}

fn commit_block(
    provider: impl ProviderRW,
    header: PartialHeader,
    transactions: Vec<TxWithHash>,
    receipts: &[ReceiptWithTxHash],
    state_updates: &mut StateUpdates,
) -> Result<SealedBlock, BlockProductionError> {
    // Update special contract address 0x1
    update_block_hash_registry_contract(&provider, state_updates, header.number)?;
    Ok(UncommittedBlock::new(header, transactions, receipts, state_updates, provider).commit())
}

fn commit_genesis_block(
    provider: impl TrieWriter,
    header: PartialHeader,
    transactions: Vec<TxWithHash>,
    receipts: &[ReceiptWithTxHash],
    state_updates: &mut StateUpdates,
) -> Result<SealedBlock, BlockProductionError> {
    Ok(UncommittedBlock::new(header, transactions, receipts, state_updates, provider).commit())
}

#[derive(Debug)]
struct GenesisTrieWriter;

impl TrieWriter for GenesisTrieWriter {
    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> katana_provider::ProviderResult<Felt> {
        let mut contract_trie_db = ContractsTrie::new(HashMapDb::<CommitId>::default());
        let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

        let leaf_hashes = {
            for (address, nonce) in &state_updates.nonce_updates {
                contract_leafs.entry(*address).or_default().nonce = Some(*nonce);
            }

            for (address, class_hash) in &state_updates.deployed_contracts {
                contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
            }

            for (address, class_hash) in &state_updates.replaced_classes {
                contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
            }

            for (address, storage_entries) in &state_updates.storage_updates {
                let mut storage_trie_db =
                    StoragesTrie::new(HashMapDb::<CommitId>::default(), *address);

                for (key, value) in storage_entries {
                    storage_trie_db.insert(*key, *value);
                }

                // Then we commit them
                storage_trie_db.commit(block_number);
                let storage_root = storage_trie_db.root();

                // insert the contract address in the contract_leafs to put the storage root
                // later
                contract_leafs.entry(*address).or_default().storage_root = Some(storage_root);
            }

            contract_leafs
                .into_iter()
                .map(|(address, leaf)| {
                    let class_hash = leaf.class_hash.unwrap();
                    let nonce = leaf.nonce.unwrap_or_default();
                    let storage_root = leaf.storage_root.unwrap_or_default();
                    let leaf_hash = compute_contract_state_hash(&class_hash, &storage_root, &nonce);
                    (address, leaf_hash)
                })
                .collect::<Vec<_>>()
        };

        for (k, v) in leaf_hashes {
            contract_trie_db.insert(k, v);
        }

        contract_trie_db.commit(block_number);
        Ok(contract_trie_db.root())
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> katana_provider::ProviderResult<Felt> {
        let mut trie = ClassesTrie::new(HashMapDb::default());

        for (class_hash, compiled_hash) in updates {
            trie.insert(*class_hash, *compiled_hash);
        }

        trie.commit(block_number);
        Ok(trie.root())
    }
}
