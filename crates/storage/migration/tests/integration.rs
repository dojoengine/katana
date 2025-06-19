use std::fs;

use katana_chain_spec::ChainSpec;
use katana_db::abstraction::{Database, DbTx};
use katana_db::{open_db, tables};
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutionFlags};
use katana_migration::MigrationManager;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::BlockNumberProvider;
use katana_provider::traits::transaction::{
    ReceiptProvider, TransactionProvider, TransactionTraceProvider,
};
use tempfile::tempdir;

fn executor() -> BlockifierFactory {
    let chain_spec = ChainSpec::dev();

    let fee_token_addresses = match &chain_spec {
        ChainSpec::Dev(cs) => {
            FeeTokenAddressses { eth: cs.fee_contracts.eth, strk: cs.fee_contracts.strk }
        }
        ChainSpec::Rollup(cs) => {
            FeeTokenAddressses { eth: cs.fee_contract.strk, strk: cs.fee_contract.strk }
        }
    };

    let cfg_env = CfgEnv {
        fee_token_addresses,
        chain_id: chain_spec.id(),
        invoke_tx_max_n_steps: 10_000_000,
        validate_max_n_steps: 1_000_000,
        max_recursion_depth: 1000,
    };

    let block_limits = BlockLimits { cairo_steps: 50_000_000 };

    BlockifierFactory::new(
        cfg_env,
        ExecutionFlags::new(),
        block_limits,
        ClassCache::builder().build().unwrap(),
    )
}

#[test]
fn db_migration() {
    // Copy the fixture database to the temporary location
    // let source_path = "tests/fixtures/v1_2_2";
    let copy_path = "tests/fixtures/v1_2_2-copy";
    // fs_extra::dir::copy(source_path, &copy_path, &fs_extra::dir::CopyOptions::new()).unwrap();

    let db = open_db(copy_path).unwrap();

    // {
    //     let total = db.tx().unwrap().entries::<tables::TxTraces>().unwrap();
    //     dbg!(total);
    // }

    let db = DbProvider::new(db);
    // let latest_block = db.latest_number().unwrap();

    // for i in 0..=latest_block {
    //     let txs = db.transactions_by_block(i.into()).unwrap().unwrap();
    //     dbg!(txs);
    // }

    // let traces = db.transaction_executions_by_block(11u64.into()).unwrap().unwrap();
    // dbg!(db.latest_number().unwrap());
    // dbg!(traces.len());

    let migration = MigrationManager::new(db, executor());
    migration.migrate_block_range(0..=1).unwrap();
}
