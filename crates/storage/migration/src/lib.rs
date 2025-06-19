//! Database migration crate for historical block re-execution.
//!
//! This crate provides functionality to re-execute historical blocks stored in the database
//! to derive new fields and data that may not be present in older database entries.

pub mod example;

use anyhow::{Context, Result};
use katana_executor::{ExecutionOutput, ExecutionResult, ExecutorFactory};
use katana_primitives::block::{
    BlockNumber, ExecutableBlock, FinalityStatus, PartialHeader, SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{
    DeclareTxWithClass, ExecutableTx, ExecutableTxWithHash, Tx, TxWithHash,
};
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::{
    BlockNumberProvider, BlockProvider, BlockWriter, HeaderProvider,
};
use katana_provider::traits::state::{StateFactoryProvider, StateProvider};
use katana_provider::traits::transaction::TransactionProvider;
use tracing::{debug, info, warn};

/// Migration manager for historical block re-execution.
#[derive(Debug)]
pub struct MigrationManager {
    database: DbProvider,
}

impl MigrationManager {
    /// Create a new migration manager with the given database.
    pub fn new(database: DbProvider) -> Self {
        Self { database }
    }

    /// Re-execute all historical blocks in the database.
    ///
    /// This method reads all blocks from the database, converts them to executable format,
    /// and re-executes them to derive new fields and data.
    pub fn migrate_all_blocks<EF: ExecutorFactory>(&self, executor_factory: EF) -> Result<()> {
        info!("Starting historical block re-execution migration");

        // Get the latest block number to determine migration range
        let latest_block =
            self.database.latest_number().context("Failed to get latest block number")?;

        if latest_block == 0 {
            info!("No blocks found in database, migration complete");
            return Ok(());
        }

        info!("Found {} blocks to migrate", latest_block + 1);

        // Re-execute blocks sequentially
        for block_number in 0..=latest_block {
            self.migrate_block(block_number, &executor_factory)
                .with_context(|| format!("Failed to migrate block {}", block_number))?;

            if block_number % 100 == 0 {
                info!("Migrated {} / {} blocks", block_number + 1, latest_block + 1);
            }
        }

        info!("Historical block re-execution migration completed successfully");
        Ok(())
    }

    /// Re-execute a specific block by its number.
    pub fn migrate_block<EF: ExecutorFactory>(
        &self,
        block: BlockNumber,
        executor_factory: &EF,
    ) -> Result<()> {
        debug!("Migrating block {}", block);

        // Read block data from database using provider traits
        let executable_block = self
            .load_executable_block(block)
            .with_context(|| format!("Failed to load block {}", block))?;

        // Create state provider for this block's parent state
        let state_provider = self.create_state_provider(block.saturating_sub(1))?;

        // Execute the block
        let mut executor = executor_factory.with_state(state_provider);
        executor
            .execute_block(executable_block)
            .with_context(|| format!("Failed to execute block {}", block))?;

        // Get execution output
        let execution_output = executor
            .take_execution_output()
            .with_context(|| format!("Failed to get execution output for block {}", block))?;

        // Store updated block data using provider traits
        self.store_block_execution_output(block, execution_output)
            .with_context(|| format!("Failed to store execution output for block {}", block))?;

        debug!("Successfully migrated block {}", block);
        Ok(())
    }

    /// Load an executable block from the database using provider traits.
    fn load_executable_block(&self, block_number: BlockNumber) -> Result<ExecutableBlock> {
        // Load header using HeaderProvider
        let header = self
            .database
            .header_by_number(block_number)
            .with_context(|| format!("Failed to get header for block {block_number}"))?
            .context("Block header not found")?;

        // Load transactions using TransactionProvider
        let tx_with_hashes = self
            .database
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
            parent_hash: header.parent_hash,
            number: header.number,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address,
            l1_gas_prices: header.l1_gas_prices,
            l1_data_gas_prices: header.l1_data_gas_prices,
            l2_gas_prices: header.l2_gas_prices,
            l1_da_mode: header.l1_da_mode,
            protocol_version: header.protocol_version,
        };

        Ok(ExecutableBlock { header: partial_header, body })
    }

    /// Convert a transaction with hash to an executable transaction.
    fn convert_to_executable_tx(&self, tx: TxWithHash) -> Result<ExecutableTxWithHash> {
        let executable_tx = match tx.transaction {
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
                    "Legacy deploy transactions are not supported in ExecutableTx for transaction {}",
                    tx.hash
                ));
            }
        };

        Ok(ExecutableTxWithHash::new(executable_tx))
    }

    /// Helper method to retrieve contract class using provider.
    fn get_contract_class(&self, class_hash: ClassHash) -> Result<ContractClass> {
        self.database
            .latest()?
            .class(class_hash)
            .context("Failed to query contract class")?
            .context("Contract class not found")
    }

    /// Create a state provider for a given block number.
    fn create_state_provider(&self, block_number: BlockNumber) -> Result<Box<dyn StateProvider>> {
        // For historical state, we need to create a provider that provides the state
        // at the specified block number.
        if block_number == 0 {
            // For genesis block, use the latest state (which should be empty/initial state)
            self.database.latest().context("Failed to create latest state provider")
        } else {
            // For historical blocks, create a historical state provider
            // that reflects the state at the specified block number
            self.database
                .historical(block_number.into())
                .context("Failed to create historical state provider")?
                .context("Historical state not available for specified block")
        }
    }

    /// Store the execution output for a block using the provider pattern.
    /// This method follows the same pattern as the production flow in do_mine_block.
    fn store_block_execution_output(
        &self,
        block_number: BlockNumber,
        execution_output: ExecutionOutput,
    ) -> Result<()> {
        debug!(
            "Storing execution output for block {} with {} transactions",
            block_number,
            execution_output.transactions.len()
        );

        // Get the existing block header
        let header = self
            .database
            .header_by_number(block_number)
            .context("Failed to get block header")?
            .context("Block header not found")?;

        // Process execution results similar to do_mine_block
        let mut traces = Vec::with_capacity(execution_output.transactions.len());
        let mut receipts = Vec::with_capacity(execution_output.transactions.len());
        let mut transactions = Vec::with_capacity(execution_output.transactions.len());

        // Only include successful transactions in the update
        for (tx, res) in execution_output.transactions {
            match res {
                ExecutionResult::Success { receipt, trace } => {
                    traces.push(TypedTransactionExecutionInfo::new(receipt.r#type(), trace));
                    receipts.push(receipt);
                    transactions.push(tx);
                }
                ExecutionResult::Failed { error } => {
                    warn!(
                        "Transaction {} in block {} failed during execution: {}",
                        tx.hash, block_number, error
                    );
                    // Skip failed transactions as in production flow
                }
            }
        }

        // Create a SealedBlockWithStatus for the existing block
        let block =
            katana_primitives::block::Block { header: header.clone(), body: transactions }.seal();

        let sealed_block = SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 };

        // Store the block with updated execution data using BlockWriter trait
        self.database
            .insert_block_with_states_and_receipts(
                sealed_block,
                execution_output.states,
                receipts,
                traces,
            )
            .context("Failed to store block execution output")?;

        debug!("Successfully stored execution output for block {}", block_number);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_manager_creation() {
        // This is a placeholder test since we don't have a concrete database implementation here
        // Real tests would use a test database implementation from katana-db

        // Example of how the migration manager would be used:
        // let db = create_test_database();
        // let migration_manager = MigrationManager::new(Arc::new(db));
        // migration_manager.migrate_all_blocks(executor_factory).unwrap();
    }

    #[test]
    fn test_conversion_functions() {
        // Test that our conversion logic handles different transaction types correctly
        // This would require setting up a proper test environment with sample data
    }
}
