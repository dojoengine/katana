use std::sync::Arc;

use alloy_primitives::U256;
use katana_chain_spec::rollup::{self};
use katana_chain_spec::{dev, ChainSpec, FeeContracts, SettlementLayer};
use katana_core::backend::Backend;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::BlockifierFactory;
use katana_executor::BlockLimits;
use katana_gas_price_oracle::GasPriceOracle;
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::{felt, Felt};
use katana_provider::api::block::HeaderProvider;
use katana_provider::{DbProviderFactory, ProviderFactory};
use rstest::rstest;
use url::Url;

fn executor(chain_spec: Arc<ChainSpec>) -> Arc<dyn katana_executor::ExecutorFactory> {
    Arc::new(BlockifierFactory::new(
        Some(VersionedConstantsOverrides {
            validate_max_n_steps: Some(u32::MAX),
            invoke_tx_max_n_steps: Some(u32::MAX),
            max_recursion_depth: Some(usize::MAX),
            is_l3: false,
        }),
        Default::default(),
        BlockLimits::default(),
        ClassCache::new().unwrap(),
        chain_spec,
    ))
}

fn backend(chain_spec: Arc<ChainSpec>) -> Backend<DbProviderFactory> {
    backend_with_db(chain_spec, DbProviderFactory::new_in_memory())
}

fn backend_with_db(
    chain_spec: Arc<ChainSpec>,
    provider: DbProviderFactory,
) -> Backend<DbProviderFactory> {
    backend_with_db_and_flag(chain_spec, provider, false)
}

fn backend_with_db_and_flag(
    chain_spec: Arc<ChainSpec>,
    provider: DbProviderFactory,
    no_state_trie: bool,
) -> Backend<DbProviderFactory> {
    Backend::new(
        chain_spec.clone(),
        provider,
        GasPriceOracle::create_for_testing(),
        executor(chain_spec),
        no_state_trie,
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

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: ChainId::default(),
        core_contract: Default::default(),
        rpc_url: Url::parse("http://localhost:5050").unwrap(),
        proof_kind: Default::default(),
    };

    rollup::ChainSpec { id, genesis, settlement, fee_contracts }
}

#[rstest]
#[case::dev(ChainSpec::Dev(dev_chain_spec()))]
#[case::rollup(ChainSpec::Rollup(rollup_chain_spec()))]
fn can_initialize_genesis(#[case] chain: ChainSpec) {
    let backend = backend(chain.into());
    backend.init_genesis(false).expect("failed to initialize genesis");
}

#[rstest]
#[case::dev(ChainSpec::Dev(dev_chain_spec()))]
#[case::rollup(ChainSpec::Rollup(rollup_chain_spec()))]
fn can_reinitialize_genesis(#[case] chain: ChainSpec) {
    let db = DbProviderFactory::new_in_memory();

    let backend = backend_with_db(chain.clone().into(), db.clone());
    backend.init_genesis(false).expect("failed to initialize genesis");

    let backend = backend_with_db(chain.into(), db);
    backend.init_genesis(false).unwrap();
}

#[test]
fn reinitialize_with_different_rollup_chain_spec() {
    let db = DbProviderFactory::new_in_memory();

    let chain1 = ChainSpec::Rollup(rollup_chain_spec());
    let backend1 = backend_with_db(chain1.into(), db.clone());
    backend1.init_genesis(false).expect("failed to initialize genesis");

    // Modify the chain spec so that the resultant genesis block hash will be different.
    let chain2 = ChainSpec::Rollup({
        let mut chain = rollup_chain_spec();
        chain.genesis.parent_hash = felt!("0x1337");
        chain
    });

    let backend2 = backend_with_db(chain2.into(), db);
    let err = backend2.init_genesis(false).unwrap_err().to_string();
    assert!(err.as_str().contains("Genesis block hash mismatch"));
}

#[rstest]
#[case::trie_enabled(false)]
#[case::trie_disabled(true)]
fn no_state_trie_flag_controls_state_root(#[case] no_state_trie: bool) {
    let chain_spec: Arc<ChainSpec> = ChainSpec::Dev(dev_chain_spec()).into();
    let backend =
        backend_with_db_and_flag(chain_spec, DbProviderFactory::new_in_memory(), no_state_trie);
    backend.init_genesis(false).expect("failed to initialize genesis");

    let mut block_env = BlockEnv::default();
    backend.update_block_env(&mut block_env);
    let outcome = backend.mine_empty_block(&block_env).expect("failed to mine empty block");

    let header = backend
        .storage
        .provider()
        .header(outcome.block_number.into())
        .expect("failed to read header")
        .expect("header missing");

    if no_state_trie {
        assert_eq!(
            header.state_root,
            Felt::ZERO,
            "state_root must be zero when --no-state-trie is set",
        );
    } else {
        assert_ne!(
            header.state_root,
            Felt::ZERO,
            "state_root must reflect the genesis-populated trie when trie computation is enabled",
        );
    }
}
