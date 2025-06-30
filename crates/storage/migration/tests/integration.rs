use std::fs;
use std::sync::Arc;

use katana_chain_spec::{dev, ChainSpec};
use katana_core::backend::gas_oracle::GasOracle;
use katana_core::constants::DEFAULT_SEQUENCER_ADDRESS;
use katana_db::abstraction::{Database, DbTx};
use katana_db::{open_db, tables};
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutionFlags};
use katana_migration::MigrationManager;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
use katana_primitives::genesis::allocation::DevAllocationsGenerator;
use katana_primitives::genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_primitives::U256;
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
    let copy_path = "tests/fixtures/v1_2_2";
    let old_db = DbProvider::new(open_db(copy_path).unwrap());

    let chain = Arc::new(cs());
    let gpo = GasOracle::sampled_starknet();
    let migration = MigrationManager::new(old_db, chain, gpo, executor()).unwrap();

    migration.migrate_all_blocks().unwrap();
}

fn cs() -> ChainSpec {
    let mut chain_spec = katana_chain_spec::dev::DEV_UNALLOCATED.clone();
    chain_spec.genesis.sequencer_address = *DEFAULT_SEQUENCER_ADDRESS;

    // Generate dev accounts.
    // If `cartridge` is enabled, the first account will be the paymaster.
    let accounts = DevAllocationsGenerator::new(DEFAULT_DEV_ACCOUNTS)
        .with_seed(parse_seed(DEFAULT_DEV_SEED))
        .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
        .generate();

    chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    ChainSpec::Dev(chain_spec)
}

const DEFAULT_DEV_SEED: &str = "0";
const DEFAULT_DEV_ACCOUNTS: u16 = 10;

pub fn parse_seed(seed: &str) -> [u8; 32] {
    let seed = seed.as_bytes();

    if seed.len() >= 32 {
        unsafe { *(seed[..32].as_ptr() as *const [u8; 32]) }
    } else {
        let mut actual_seed = [0u8; 32];
        seed.iter().enumerate().for_each(|(i, b)| actual_seed[i] = *b);
        actual_seed
    }
}
