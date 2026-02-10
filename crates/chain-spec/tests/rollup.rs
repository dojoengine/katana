use std::sync::Arc;

use alloy_primitives::U256;
use katana_chain_spec::rollup::utils::GenesisTransactionsBuilder;
use katana_chain_spec::rollup::{ChainSpec, DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS};
use katana_chain_spec::{FeeContracts, SettlementLayer};
use katana_contracts::contracts;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutorFactory};
use katana_genesis::allocation::{
    DevAllocationsGenerator, GenesisAccount, GenesisAccountAlloc, GenesisAllocation,
};
use katana_genesis::constant::{DEFAULT_PREFUNDED_ACCOUNT_BALANCE, DEFAULT_UDC_ADDRESS};
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxType;
use katana_primitives::Felt;
use katana_provider::api::state::StateFactoryProvider;
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
    let fee_contracts = FeeContracts {
        eth: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
        strk: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
    };

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: ChainId::default(),
        account: Default::default(),
        core_contract: Default::default(),
        rpc_url: Url::parse("http://localhost:5050").unwrap(),
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

    let mut executor =
        ef.block_executor(provider.latest().unwrap(), katana_primitives::env::BlockEnv::default());
    executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

    let output = executor.take_execution_output().unwrap();

    for (i, (.., result)) in output.transactions.iter().enumerate() {
        assert!(result.is_success(), "tx {i} failed; {result:?}");
    }
}

#[test]
fn genesis_states() {
    let chain_spec = chain_spec(1, true);

    let provider = DbProviderFactory::new_in_memory();
    let provider = provider.provider();
    let ef = executor(chain_spec.clone());

    let mut executor =
        ef.block_executor(provider.latest().unwrap(), katana_primitives::env::BlockEnv::default());
    executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

    let genesis_state = executor.state();

    // -----------------------------------------------------------------------
    // Classes

    // check that the default erc20 class is declared
    let erc20_class_hash = contracts::LegacyERC20::HASH;
    assert!(genesis_state.class(erc20_class_hash).unwrap().is_some());

    // check that the default udc class is declared
    let udc_class_hash = contracts::UniversalDeployer::HASH;
    assert!(genesis_state.class(udc_class_hash).unwrap().is_some());

    // -----------------------------------------------------------------------
    // Contracts

    // check that the default fee token is deployed
    let res = genesis_state.class_hash_of_contract(DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS).unwrap();
    assert_eq!(res, Some(erc20_class_hash));

    // check that the default udc is deployed
    let res = genesis_state.class_hash_of_contract(DEFAULT_UDC_ADDRESS).unwrap();
    assert_eq!(res, Some(udc_class_hash));

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
        TxType::Declare,       // ERC20 declare
        TxType::Invoke,        // ERC20 deploy
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

        // We only want to check that for each predeployed accounts, there should be a deploy
        // account and transfer balance (invoke) transactions. So we skip the first 7
        // transactions (master account, UDC, ERC20, etc).
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

        // We only want to check that for each predeployed accounts, there should be a deploy
        // account and transfer balance (invoke) transactions. So we skip the first 7
        // transactions (master account, UDC, ERC20, etc).
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
