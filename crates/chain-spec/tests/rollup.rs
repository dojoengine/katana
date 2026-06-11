use std::sync::Arc;

use alloy_primitives::U256;
use katana_chain_spec::rollup::utils::GenesisTransactionsBuilder;
use katana_chain_spec::rollup::ChainSpec;
use katana_chain_spec::{FeeContracts, SettlementLayer};
use katana_contracts::contracts;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutorFactory};
use katana_genesis::allocation::{
    DevAllocationsGenerator, GenesisAccount, GenesisAccountAlloc, GenesisAllocation,
};
use katana_genesis::constant::{DEFAULT_PREFUNDED_ACCOUNT_BALANCE, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::transaction::TxType;
use katana_primitives::utils::get_contract_address;
use katana_primitives::Felt;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::providers::PreloadedStateProvider;
use katana_provider::{DbProviderFactory, ProviderFactory};
use url::Url;

fn chain_spec(n_dev_accounts: u16, with_balance: bool) -> ChainSpec {
    let accounts = if with_balance {
        DevAllocationsGenerator::new(n_dev_accounts)
            .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
            .generate()
    } else {
        DevAllocationsGenerator::new(n_dev_accounts).generate()
    };

    let mut genesis = Genesis::default();
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    let id = ChainId::parse("KATANA").unwrap();
    let fee_contracts =
        FeeContracts { eth: DEFAULT_STRK_FEE_TOKEN_ADDRESS, strk: DEFAULT_STRK_FEE_TOKEN_ADDRESS };

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: ChainId::default(),
        core_contract: Default::default(),
        rpc_url: Url::parse("http://localhost:5050").unwrap(),
        proof_kind: Default::default(),
    };

    ChainSpec { id, genesis, settlement, fee_contracts }
}

fn executor(chain_spec: ChainSpec) -> BlockifierFactory {
    BlockifierFactory::new(
        None,
        Default::default(),
        BlockLimits::default(),
        ClassCache::new().unwrap(),
        Arc::new(katana_chain_spec::ChainSpec::Rollup(chain_spec)),
    )
}

#[test]
fn valid_transactions() {
    let chain_spec = chain_spec(1, true);

    let provider = DbProviderFactory::new_in_memory();
    let provider = provider.provider();
    let ef = executor(chain_spec.clone());

    // Mirror `init_rollup_genesis`: wrap the executor's source state with the rollup's
    // pre-allocated overlay so the genesis `transfer` invokes find the STRK contract.
    let state = PreloadedStateProvider::new(provider.latest().unwrap(), chain_spec.state_updates());
    let mut executor = ef.executor(Box::new(state), katana_primitives::env::BlockEnv::default());
    executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

    let output = executor.take_execution_output().unwrap();

    for (i, (.., result)) in output.transactions.iter().enumerate() {
        assert!(result.is_success(), "tx {i} failed; {result:?}");
    }
}

#[test]
fn controller_class_queryable_at_canonical_hash_after_genesis() {
    use katana_contracts::controller::{ControllerLatest, ControllerV108};
    use katana_genesis::json::GenesisJson;

    const CANONICAL_LATEST: Felt = Felt::from_hex_unchecked(
        "0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf",
    );
    const CANONICAL_V108: Felt = Felt::from_hex_unchecked(
        "0x511dd75da368f5311134dee2356356ac4da1538d2ad18aa66d57c47e3757d59",
    );

    let mut chain_spec = chain_spec(1, true);
    chain_spec
        .genesis
        .classes
        .insert(ControllerLatest::HASH, ControllerLatest::CLASS.clone().into());
    chain_spec.genesis.classes.insert(ControllerV108::HASH, ControllerV108::CLASS.clone().into());

    // Simulate `katana init rollup` -> genesis.json -> node reload.
    let json = GenesisJson::try_from(chain_spec.genesis.clone()).unwrap();
    chain_spec.genesis = Genesis::try_from(json).unwrap();

    let provider = DbProviderFactory::new_in_memory();
    let provider = provider.provider();
    let ef = executor(chain_spec.clone());

    let state = PreloadedStateProvider::new(provider.latest().unwrap(), chain_spec.state_updates());
    let mut executor = ef.executor(Box::new(state), katana_primitives::env::BlockEnv::default());
    executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

    let output = executor.take_execution_output().unwrap();
    for (i, (.., result)) in output.transactions.iter().enumerate() {
        assert!(result.is_success(), "genesis tx {i} failed: {result:?}");
    }

    let genesis_state = executor.state();
    assert!(
        genesis_state.class(CANONICAL_LATEST).unwrap().is_some(),
        "controller.latest must be queryable at canonical hash {CANONICAL_LATEST:#x}"
    );
    assert!(
        genesis_state.class(CANONICAL_V108).unwrap().is_some(),
        "controller.v1.0.8 must be queryable at canonical hash {CANONICAL_V108:#x}"
    );
}

#[test]
fn genesis_states() {
    let chain_spec = chain_spec(1, true);

    let provider = DbProviderFactory::new_in_memory();
    let provider = provider.provider();
    let ef = executor(chain_spec.clone());

    let state = PreloadedStateProvider::new(provider.latest().unwrap(), chain_spec.state_updates());
    let mut executor = ef.executor(Box::new(state), katana_primitives::env::BlockEnv::default());
    executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

    let genesis_state = executor.state();

    // -----------------------------------------------------------------------
    // Classes

    // check that the default erc20 class is declared
    let erc20_class_hash = contracts::LegacyERC20::HASH;
    assert!(genesis_state.class(erc20_class_hash).unwrap().is_some());

    // check that both UDC classes are declared
    let udc_class_hash = contracts::OpenZeppelinUniversalDeployer::HASH;
    assert!(genesis_state.class(udc_class_hash).unwrap().is_some());
    let legacy_udc_class_hash = contracts::UniversalDeployer::HASH;
    assert!(genesis_state.class(legacy_udc_class_hash).unwrap().is_some());

    // -----------------------------------------------------------------------
    // Contracts

    // STRK fee token is pre-allocated to the canonical Starknet mainnet address rather than
    // deployed via UDC — see `rollup::ChainSpec::state_updates`.
    let res = genesis_state.class_hash_of_contract(DEFAULT_STRK_FEE_TOKEN_ADDRESS).unwrap();
    assert_eq!(res, Some(erc20_class_hash));

    // Rollup mode deploys each UDC via the master account with salt=0 and no ctor args; the
    // resulting address is derived from the UDC class hash (not the fixed mainnet address).
    let udc_address: ContractAddress =
        get_contract_address(Felt::ZERO, udc_class_hash, &[], ContractAddress::ZERO).into();
    let res = genesis_state.class_hash_of_contract(udc_address).unwrap();
    assert_eq!(res, Some(udc_class_hash));

    let legacy_udc_address: ContractAddress =
        get_contract_address(Felt::ZERO, legacy_udc_class_hash, &[], ContractAddress::ZERO).into();
    let res = genesis_state.class_hash_of_contract(legacy_udc_address).unwrap();
    assert_eq!(res, Some(legacy_udc_class_hash));

    for (address, account) in chain_spec.genesis.accounts() {
        let nonce = genesis_state.nonce(*address).unwrap();
        let class_hash = genesis_state.class_hash_of_contract(*address).unwrap();

        assert_eq!(nonce, Some(Nonce::ONE));
        assert_eq!(class_hash, Some(account.class_hash()));
    }
}

#[test]
fn transaction_order() {
    let chain_spec = chain_spec(1, true);
    let transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

    let expected_order = vec![
        TxType::Declare,       // Master account class declare
        TxType::DeployAccount, // Master account
        TxType::Declare,       // UDC declare
        TxType::Invoke,        // UDC deploy
        TxType::Declare,       // Legacy UDC declare
        TxType::Invoke,        // Legacy UDC deploy
        // ERC20 declare/deploy intentionally absent — the STRK fee token is
        // pre-allocated to genesis state by ChainSpec::state_updates instead.
        TxType::Declare,       // Account class declare (V2)
        TxType::DeployAccount, // Dev account
        TxType::Invoke,        // Balance transfer
    ];

    assert_eq!(transactions.len(), expected_order.len());
    for (tx, expected) in transactions.iter().zip(expected_order) {
        assert_eq!(tx.transaction.r#type(), expected);
    }
}

#[rstest::rstest]
#[case::with_balance(true)]
#[case::no_balance(false)]
fn predeployed_acccounts(#[case] with_balance: bool) {
    fn inner(n_accounts: usize, with_balance: bool) {
        let mut chain_spec = chain_spec(0, with_balance);

        // add non-dev allocations
        for i in 0..n_accounts {
            const CLASS_HASH: ClassHash = contracts::Account::HASH;
            let salt = Felt::from(i);
            let pk = Felt::from(1337);

            let mut account = GenesisAccount::new_with_salt(pk, CLASS_HASH, salt);

            if with_balance {
                account.balance = Some(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE));
            }

            chain_spec.genesis.extend_allocations([(
                account.address(),
                GenesisAllocation::Account(GenesisAccountAlloc::Account(account)),
            )]);
        }

        let mut transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

        // Skip the prefix txs (master account declare/deploy, UDC + legacy UDC declare/deploy,
        // Account class declare) so we're left with just the per-account work. STRK is
        // pre-allocated to state rather than declared/deployed here, so the prefix is 7 txs.
        let account_transactions = &transactions.split_off(7);

        if with_balance {
            assert_eq!(account_transactions.len(), n_accounts * 2);
            for txs in account_transactions.chunks(2) {
                assert_eq!(txs[0].transaction.r#type(), TxType::Invoke); // deploy
                assert_eq!(txs[1].transaction.r#type(), TxType::Invoke); // transfer
            }
        } else {
            assert_eq!(account_transactions.len(), n_accounts);
            for txs in account_transactions.chunks(2) {
                assert_eq!(txs[0].transaction.r#type(), TxType::Invoke); // deploy
            }
        }
    }

    for i in 0..10 {
        inner(i, with_balance);
    }
}

#[rstest::rstest]
#[case::with_balance(true)]
#[case::no_balance(false)]
fn dev_predeployed_acccounts(#[case] with_balance: bool) {
    fn inner(n_accounts: u16, with_balance: bool) {
        let chain_spec = chain_spec(n_accounts, with_balance);
        let mut transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

        // Skip the prefix txs (master account declare/deploy, UDC + legacy UDC declare/deploy,
        // Account class declare) so we're left with just the per-account work. STRK is
        // pre-allocated to state rather than declared/deployed here, so the prefix is 7 txs.
        let account_transactions = &transactions.split_off(7);

        if with_balance {
            assert_eq!(account_transactions.len(), n_accounts as usize * 2);
            for txs in account_transactions.chunks(2) {
                assert_eq!(txs[0].transaction.r#type(), TxType::DeployAccount);
                assert_eq!(txs[1].transaction.r#type(), TxType::Invoke); // transfer
            }
        } else {
            assert_eq!(account_transactions.len(), n_accounts as usize);
            for txs in account_transactions.chunks(2) {
                assert_eq!(txs[0].transaction.r#type(), TxType::DeployAccount);
            }
        }
    }

    for i in 0..10 {
        inner(i, with_balance);
    }
}
