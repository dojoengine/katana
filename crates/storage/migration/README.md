# Katana Migration

This crate provides functionality for migrating historical blockchain data stored in Katana's database. The primary use case is re-executing historical blocks to derive new fields and data that may not be present in older database entries.

## Overview

When new fields are added to types like `Receipt` or `TransactionTrace`, older database entries won't have these fields. Rather than versioning all database types (which would be a huge maintenance effort), this crate allows you to re-execute historical blocks to generate the missing derived data.

## Key Features

- **Historical Block Re-execution**: Re-execute all blocks stored in the database
- **Database Abstraction**: Uses database abstraction traits, not concrete database implementations
- **Versioned Type Support**: Handles conversion from versioned database types (V6 → V7)
- **Standalone Implementation**: Easy to test and verify without katana-node integration

## Usage

```rust
use std::sync::Arc;
use katana_migration::MigrationManager;

// Create a migration manager with your database
let migration_manager = MigrationManager::new(database);

// Re-execute all historical blocks
migration_manager.migrate_all_blocks(executor_factory)?;

// Or migrate a specific block
migration_manager.migrate_block(block_number, &executor_factory)?;
```

## Architecture

The migration process works as follows:

1. **Read Block Data**: Load versioned block headers and transactions from database
2. **Convert to Executable**: Convert database types to executable types required by the executor
3. **Re-execute Block**: Use katana-executor to re-execute the block
4. **Update Derived Data**: Store the newly generated receipts, traces, and other derived data

## Type Conversions

The migration handles conversion between different type versions:

- **VersionedHeader**: V6 → V7 header conversion
- **VersionedTx**: V6 → V7 transaction conversion  
- **Tx → ExecutableTx**: Convert stored transactions to executable format
- **Declare Transactions**: Fetch associated contract classes from storage

## Database Tables

The migration reads from and writes to these database tables:

- **Read**: `Headers`, `Transactions`, `BlockBodyIndices`, `Classes`, `TxNumbers`
- **Write**: `Receipts`, `TxTraces` (and potentially other derived data tables)

## Error Handling

The migration gracefully handles:

- Missing contract classes for declare transactions
- Legacy deploy transactions (not supported in ExecutableTx)
- Execution failures during re-execution
- Database transaction failures

## Testing

Run the tests with:

```bash
cargo test -p katana-migration
```

The crate includes both unit tests and integration tests that demonstrate the migration workflow.

## Future Enhancements

- Historical state providers for more accurate re-execution
- Parallel block processing for better performance
- Progress tracking and resumption for large migrations
- Integration with katana-node for automated migrations