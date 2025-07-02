//! Database migration crate for historical block re-execution.
//!
//! This crate provides functionality to re-execute historical blocks stored in the database
//! to derive new fields introduced in newer versions and data that may not be present in older
//! database entries.

use std::ops::RangeInclusive;
use std::sync::Arc;

use anyhow::{Context, Result};
use katana_chain_spec::ChainSpec;
use katana_core::backend::gas_oracle::GasOracle;
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_db::mdbx::DbEnv;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::ExecutorFactory;
use katana_primitives::block::{
    BlockHashOrNumber, BlockNumber, ExecutableBlock, PartialHeader, SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::env::BlockEnv;
use katana_primitives::genesis::constant::{
    DEFAULT_LEGACY_ERC20_CLASS, DEFAULT_LEGACY_ERC20_CLASS_HASH,
};
use katana_primitives::hash::{self, StarkHash};
use katana_primitives::transaction::{
    DeclareTxWithClass, ExecutableTx, ExecutableTxWithHash, Tx, TxWithHash,
};
use katana_primitives::Felt;
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
    HeaderProvider,
};
use katana_provider::traits::state::StateFactoryProvider;
use katana_provider::traits::state_update::StateUpdateProvider;
use katana_provider::traits::transaction::{ReceiptProvider, TransactionProvider};
use katana_provider::traits::trie::TrieWriter;
use starknet::macros::short_string;
use tracing::info;

/// Migration manager for historical block re-execution.
#[derive(Debug)]
pub struct MigrationManager {
    /// The new database to migrate to.
    new_database: DbProvider,
    old_database: DbProvider,
    backend: Backend<BlockifierFactory>,
}

impl MigrationManager {
    /// Create a new migration manager with the given database.
    pub fn new(
        new_database: DbEnv,
        old_database: DbProvider,
        chain_spec: Arc<ChainSpec>,
        gas_oracle: GasOracle,
        executor: BlockifierFactory,
    ) -> Result<Self> {
        let new_blockchain = Blockchain::new_with_db(new_database.clone());
        let backend = Backend::new(chain_spec, new_blockchain, gas_oracle, executor);
        // backend.init_genesis().context("failed to initialize new database")?;

        let new_database = DbProvider::new(new_database);
        let new = Self { old_database, new_database, backend };
        new.init_dev_db()?;

        let old_genesis_state_root =
            new.old_database.historical(0.into())?.unwrap().state_root()?;
        let new_genesis_state_root =
            new.new_database.historical(0.into())?.unwrap().state_root()?;

        similar_asserts::assert_eq!(old_genesis_state_root, new_genesis_state_root);

        Ok(new)
    }

    fn init_dev_db(&self) -> Result<()> {
        let old_db = &self.old_database;
        let genesis_block_id: BlockHashOrNumber = 0.into();

        // determine whether the genesis block is a dev or rollup chain spec
        let genesis_tx_count = old_db
            .transaction_count_by_block(genesis_block_id)?
            .context("missing genesis block")?;

        // if the genesis block is from the DEV chain spec, follow how the dev chain spec is
        // initialized in Backend::init_dev_genesis
        if genesis_tx_count == 0 {
            let mut state_updates = old_db.state_update_with_classes(genesis_block_id)?.unwrap();
            // dbg!(&state_updates.state_updates);

            // fix for a bug
            state_updates
                .classes
                .insert(DEFAULT_LEGACY_ERC20_CLASS_HASH, DEFAULT_LEGACY_ERC20_CLASS.clone());

            let old_block_hash = old_db.block_hash_by_id(genesis_block_id)?.unwrap();
            let status = old_db.block_status(genesis_block_id)?.unwrap();
            let mut block = old_db.block(genesis_block_id)?.unwrap();
            // tech debt in the initialization of dev chain spec
            block.header.state_root = Felt::ZERO;

            let block_number = block.header.number;
            let mut block = block.seal();
            assert_eq!(old_block_hash, block.hash);

            let class_trie_root = self
                .new_database
                .trie_insert_declared_classes(
                    block_number,
                    &state_updates.state_updates.declared_classes,
                )
                .context("failed to update class trie")?;

            let contract_trie_root = self
                .new_database
                .trie_insert_contract_updates(block_number, &state_updates.state_updates)
                .context("failed to update contract trie")?;

            let genesis_state_root = hash::Poseidon::hash_array(&[
                short_string!("STARKNET_STATE_V0"),
                contract_trie_root,
                class_trie_root,
            ]);

            block.header.state_root = genesis_state_root;
            self.new_database.insert_block_with_states_and_receipts(
                SealedBlockWithStatus { block, status },
                state_updates,
                vec![],
                vec![],
            )?;
        };

        Ok(())
    }

    /// Re-execute all historical blocks in the database.
    ///
    /// This method reads all blocks from the database, converts them to executable format,
    /// and re-executes them to derive new fields and data.
    pub fn migrate_all_blocks(&self) -> Result<()> {
        info!(target: "migration", "Starting historical block re-execution migration");

        // Get the latest block number to determine migration range
        let latest_block =
            self.old_database.latest_number().context("Failed to get latest block number")?;

        if latest_block == 0 {
            info!(target: "migration", "No blocks found in database, migration complete.");
            return Ok(());
        }

        self.migrate_block_range(1..=latest_block)?;

        info!(target: "migration", total_blocks=%latest_block, "Database migration completed");

        Ok(())
    }

    /// Re-execute a specific block by its number.
    pub fn migrate_block_range(&self, block_range: RangeInclusive<BlockNumber>) -> Result<()> {
        let block_start = *block_range.start();
        let block_end = *block_range.end();
        info!(target: "migration", %block_start, %block_end, "Migrating database");

        for block_num in block_range {
            info!(target: "migration", block=format!("{block_num}/{block_end}"), "Migrating block.");
            self.migrate_block(block_num)?;
        }

        Ok(())
    }

    fn migrate_block(&self, block_num: BlockNumber) -> Result<()> {
        let state = self.new_database.historical(block_num.saturating_sub(1).into())?.unwrap();
        let block = self.load_executable_block(block_num)?;

        let block_env = BlockEnv {
            number: block.header.number,
            timestamp: block.header.timestamp,
            l2_gas_prices: block.header.l2_gas_prices.clone(),
            l1_gas_prices: block.header.l1_gas_prices.clone(),
            l1_data_gas_prices: block.header.l1_data_gas_prices.clone(),
            sequencer_address: block.header.sequencer_address,
            starknet_version: block.header.protocol_version,
        };

        let mut executor =
            self.backend.executor_factory.with_state_and_block_env(state, block_env.clone());

        executor.execute_block(block)?;
        let execution_output = executor.take_execution_output()?;

        self.backend.do_mine_block(&block_env, execution_output)?;

        // self.verify_block_migration(block_num)?;

        Ok(())
    }

    /// Load an executable block from the database using provider traits.
    fn load_executable_block(&self, block_number: BlockNumber) -> Result<ExecutableBlock> {
        // Load header using HeaderProvider
        let old_header = self
            .old_database
            .header_by_number(block_number)
            .with_context(|| format!("Failed to get header for block {block_number}"))?
            .context("Block header not found")?;

        // Load transactions using TransactionProvider
        let tx_with_hashes = self
            .old_database
            .transactions_by_block(block_number.into())
            .with_context(|| format!("Failed to get transactions for block {block_number}"))?
            .context("Block transactions not found")?;

        // Convert to executable transactions
        let mut body = Vec::with_capacity(tx_with_hashes.len());
        for tx in tx_with_hashes {
            body.push(self.convert_to_executable_tx(tx)?);
        }

        // Create partial header for ExecutableBlock
        let partial_header = PartialHeader {
            parent_hash: old_header.parent_hash,
            number: old_header.number,
            timestamp: old_header.timestamp,
            sequencer_address: old_header.sequencer_address,
            l1_gas_prices: old_header.l1_gas_prices,
            l1_data_gas_prices: old_header.l1_data_gas_prices,
            l2_gas_prices: old_header.l2_gas_prices,
            l1_da_mode: old_header.l1_da_mode,
            protocol_version: old_header.protocol_version,
        };

        Ok(ExecutableBlock { header: partial_header, body })
    }

    /// Convert a transaction to an executable transaction.
    fn convert_to_executable_tx(&self, tx: TxWithHash) -> Result<ExecutableTxWithHash> {
        let hash = tx.hash;
        let transaction = match tx.transaction {
            Tx::Invoke(invoke_tx) => ExecutableTx::Invoke(invoke_tx),
            Tx::L1Handler(l1_handler_tx) => ExecutableTx::L1Handler(l1_handler_tx),
            Tx::DeployAccount(deploy_account_tx) => ExecutableTx::DeployAccount(deploy_account_tx),

            Tx::Declare(declare_tx) => {
                // For declare transactions, we need to fetch the contract class from storage
                let class_hash = declare_tx.class_hash();

                // Retrieve the contract class from the database using provider
                let contract_class = self.get_contract_class(class_hash).with_context(|| {
                    format!("Failed to get contract class for hash {class_hash:#x}")
                })?;

                ExecutableTx::Declare(DeclareTxWithClass::new(declare_tx, contract_class))
            }

            Tx::Deploy(_deploy_tx) => {
                // Legacy deploy transactions are not supported in ExecutableTx
                // We'll skip these transactions for now
                return Err(anyhow::anyhow!(
                    "Legacy deploy transactions are not supported in ExecutableTx for transaction \
                     {}",
                    tx.hash
                ));
            }
        };

        Ok(ExecutableTxWithHash { transaction, hash })
    }

    fn get_contract_class(&self, class_hash: ClassHash) -> Result<ContractClass> {
        self.old_database
            .latest()?
            .class(class_hash)
            .context("Failed to query contract class")?
            .context("Contract class not found")
    }

    /// Will return an Err if the block migration verification fails.
    fn verify_block_migration(&self, block: BlockNumber) -> Result<()> {
        let old_db = &self.old_database;
        let new_db = &self.new_database;
        let block_id: BlockHashOrNumber = block.into();

        let old_receipts = old_db.receipts_by_block(block_id)?.unwrap_or_default().into_iter();
        let new_receipts = new_db.receipts_by_block(block_id)?.unwrap_or_default().into_iter();

        // make sure both transactions have the same execution status (ie succeeded / reverted )
        for (idx, (new, old)) in new_receipts.zip(old_receipts).enumerate() {
            assert_eq!(
                new.is_reverted(),
                old.is_reverted(),
                "mismatch receipt status for tx {idx} in block {block}",
            )
        }

        // State root
        let old_state_root = old_db.historical(block_id)?.unwrap().state_root()?;
        let new_state_root = new_db.historical(block_id)?.unwrap().state_root()?;
        assert_eq!(old_state_root, new_state_root, "mismatch state root for block {block}");

        // Block
        let old_block_hash = old_db.block_hash_by_num(block)?.unwrap();
        let old_block = old_db.block(block.into())?.unwrap();

        let new_block_hash = new_db.block_hash_by_num(block)?.unwrap();
        let new_block = new_db.block(block.into())?.unwrap();

        similar_asserts::assert_eq!(old_block, new_block, "mismatch block for block {block}");
        assert_eq!(old_block_hash, new_block_hash, "mismatch block hashes for block {block}");

        Ok(())
    }
}
