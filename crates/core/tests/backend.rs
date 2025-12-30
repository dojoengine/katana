use std::sync::Arc;

use alloy_primitives::U256;
use katana_chain_spec::rollup::{self};
use katana_chain_spec::{dev, ChainSpecT, FeeContracts, SettlementLayer};
use katana_core::backend::{Backend, GenesisInitializer};
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::BlockLimits;
use katana_gas_price_oracle::GasPriceOracle;
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::env::VersionedConstantsOverrides;
use katana_primitives::felt;
use katana_provider::DbProviderFactory;
use url::Url;

fn executor<C: ChainSpecT>(chain_spec: Arc<C>) -> BlockifierFactory<C> {
    BlockifierFactory::new(
        Some(VersionedConstantsOverrides {
            validate_max_n_steps: Some(u32::MAX),
            invoke_tx_max_n_steps: Some(u32::MAX),
            max_recursion_depth: Some(usize::MAX),
        }),
        Default::default(),
        BlockLimits::default(),
        ClassCache::new().unwrap(),
        chain_spec,
    )
}

fn backend<C: ChainSpecT>(
    chain_spec: Arc<C>,
) -> Backend<BlockifierFactory<C>, DbProviderFactory, C> {
    backend_with_db(chain_spec, DbProviderFactory::new_in_memory())
}

fn backend_with_db<C: ChainSpecT>(
    chain_spec: Arc<C>,
    provider: DbProviderFactory,
) -> Backend<BlockifierFactory<C>, DbProviderFactory, C> {
    Backend::new(
        chain_spec.clone(),
        provider,
        GasPriceOracle::create_for_testing(),
        executor(chain_spec),
    )
}

fn dev_chain_spec() -> dev::ChainSpec {
    dev::ChainSpec::default()
}

fn rollup_chain_spec() -> rollup::ChainSpec {
    let accounts = DevAllocationsGenerator::new(10)
        .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
        .generate();

    let mut genesis = Genesis::default();
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    let id = ChainId::parse("KATANA").unwrap();
    let fee_contracts = FeeContracts::default();

    let settlement = Some(SettlementLayer::Starknet {
        block: 0,
        id: ChainId::default(),
        account: Default::default(),
        core_contract: Default::default(),
        rpc_url: Url::parse("http://localhost:5050").unwrap(),
    });

    rollup::ChainSpec { id, genesis, settlement, fee_contracts }
}

#[test]
fn can_initialize_genesis() {
    // dev chain spec

    let chain_spec = Arc::new(dev_chain_spec());
    backend(chain_spec).init_genesis(false).expect("failed to initialize genesis");

    // rollup chain spec

    let chain_spec = Arc::new(rollup_chain_spec());
    backend(chain_spec).init_genesis(false).expect("failed to initialize genesis");
}

#[test]
fn can_reinitialize_genesis() {
    let db = DbProviderFactory::new_in_memory();

    // dev chain spec

    let chain_spec = Arc::new(dev_chain_spec());

    let backend = backend_with_db(chain_spec.clone(), db.clone());
    backend.init_genesis(false).expect("failed to initialize genesis");

    let backend = backend_with_db(chain_spec.clone(), db.clone());
    backend.init_genesis(false).unwrap();

    // rollup chain spec

    let chain_spec = Arc::new(rollup_chain_spec());

    let backend = backend_with_db(chain_spec.clone(), db.clone());
    backend.init_genesis(false).expect("failed to initialize genesis");

    let backend = backend_with_db(chain_spec.clone(), db);
    backend.init_genesis(false).unwrap();
}

#[test]
fn reinitialize_with_different_rollup_chain_spec() {
    let db = DbProviderFactory::new_in_memory();

    let chain1 = rollup_chain_spec();
    let backend1 = backend_with_db(chain1.into(), db.clone());
    backend1.init_genesis(false).expect("failed to initialize genesis");

    // Modify the chain spec so that the resultant genesis block hash will be different.
    let mut chain2 = rollup_chain_spec();
    chain2.genesis.parent_hash = felt!("0x1337");

    let backend2 = backend_with_db(chain2.into(), db);
    let err = backend2.init_genesis(false).unwrap_err().to_string();
    assert!(err.as_str().contains("Genesis block hash mismatch"));
}
