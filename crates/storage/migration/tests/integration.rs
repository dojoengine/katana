use katana_chain_spec::ChainSpec;
use katana_db::open_db;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutionFlags};
use katana_migration::MigrationManager;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
use katana_provider::providers::db::DbProvider;

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
    let db = DbProvider::new(open_db("tests/fixtures/v1_2_2").unwrap());
    let migration = MigrationManager::new(db, executor());
    migration.migrate_all_blocks().unwrap();
}
