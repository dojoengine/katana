use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::U256;
use katana_contracts::contracts;
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::genesis::allocation::{DevGenesisAccount, GenesisAccountAlloc};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV0, DeclareTxV2, DeclareTxWithClass, DeployAccountTx, DeployAccountTxV1,
    ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV1,
};
use katana_primitives::utils::split_u256;
use katana_primitives::utils::transaction::compute_deploy_account_v1_tx_hash;
use katana_primitives::{felt, Felt};
use num_traits::FromPrimitive;
use starknet::core::utils::{get_contract_address, get_selector_from_name};
use starknet::macros::short_string;
use starknet::signers::SigningKey;

use crate::rollup::ChainSpec;

/// The contract address of the fee token generated by [`utils::GenesisTransactionsBuilder`].
pub const DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS: ContractAddress =
    ContractAddress(felt!("0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea"));

/// A convenience builder for creating valid and executable transactions for the genesis block based
/// on the [`Genesis`].
///
/// The transactions are crafted in a way that can be executed by the StarknetOS Cairo program and
/// thus `blockifier`.
#[derive(Debug)]
pub struct GenesisTransactionsBuilder<'c> {
    chain_spec: &'c ChainSpec,
    fee_token: OnceCell<ContractAddress>,
    master_address: OnceCell<ContractAddress>,
    master_signer: SigningKey,
    master_nonce: RefCell<Nonce>,
    transactions: RefCell<Vec<ExecutableTxWithHash>>,
    declared_classes: RefCell<HashSet<ClassHash>>,
}

impl<'c> GenesisTransactionsBuilder<'c> {
    /// Creates a new [`GenesisTransactionsBuilder`] for the given [`ChainSpec`].
    pub fn new(chain_spec: &'c ChainSpec) -> Self {
        Self {
            chain_spec,
            fee_token: OnceCell::new(),
            master_address: OnceCell::new(),
            transactions: RefCell::new(Vec::new()),
            master_nonce: RefCell::new(Nonce::ZERO),
            declared_classes: RefCell::new(HashSet::new()),
            master_signer: SigningKey::from_secret_scalar(felt!("0xa55")),
        }
    }

    fn legacy_declare(&self, class: ContractClass) -> ClassHash {
        if matches!(class, ContractClass::Class(..)) {
            panic!("legacy_declare must be called only with legacy class")
        }

        let class = Arc::new(class);
        let class_hash = class.class_hash().unwrap();

        // No need to declare the same class if it was already declared.
        if self.declared_classes.borrow_mut().contains(&class_hash) {
            return class_hash;
        }

        let transaction = ExecutableTx::Declare(DeclareTxWithClass {
            transaction: DeclareTx::V0(DeclareTxV0 {
                sender_address: Felt::ONE.into(),
                chain_id: self.chain_spec.id,
                signature: Vec::new(),
                class_hash,
                max_fee: 0,
            }),
            class,
        });

        let tx_hash = transaction.calculate_hash(false);
        self.declared_classes.borrow_mut().insert(class_hash);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });

        class_hash
    }

    fn declare(&self, class: ContractClass) -> ClassHash {
        let nonce = self.master_nonce.replace_with(|&mut n| n + Felt::ONE);
        let sender_address = *self.master_address.get().expect("must be initialized first");

        let class_hash = class.class_hash().unwrap();

        // No need to declare the same class if it was already declared.
        if self.declared_classes.borrow_mut().contains(&class_hash) {
            return class_hash;
        }

        let compiled_class_hash = class.clone().compile().unwrap().class_hash().unwrap();

        let mut transaction = DeclareTxV2 {
            chain_id: self.chain_spec.id,
            signature: Vec::new(),
            compiled_class_hash,
            sender_address,
            class_hash,
            max_fee: 0,
            nonce,
        };

        let hash = DeclareTx::V2(transaction.clone()).calculate_hash(false);
        let signature = self.master_signer.sign(&hash).unwrap();
        transaction.signature = vec![signature.r, signature.s];

        self.transactions.borrow_mut().push(ExecutableTxWithHash {
            transaction: ExecutableTx::Declare(DeclareTxWithClass {
                transaction: DeclareTx::V2(transaction),
                class: class.into(),
            }),
            hash,
        });

        self.declared_classes.borrow_mut().insert(class_hash);

        class_hash
    }

    fn deploy(&self, class: ClassHash, ctor_args: Vec<Felt>, salt: Felt) -> ContractAddress {
        use std::iter;

        const DEPLOY_CONTRACT_SELECTOR: &str = "deploy_contract";
        let master_address = *self.master_address.get().expect("must be initialized first");

        let contract_address = get_contract_address(salt, class, &ctor_args, Felt::ZERO);

        let ctor_args_len = Felt::from_usize(ctor_args.len()).unwrap();
        let args: Vec<Felt> = iter::once(class) // class_hash
			.chain(iter::once(salt)) // contract_address_salt
			.chain(iter::once(ctor_args_len)) // constructor_calldata_len
			.chain(ctor_args) // constructor_calldata
			.chain(iter::once(Felt::ONE)) // deploy_from_zero
			.collect();

        self.invoke(master_address, DEPLOY_CONTRACT_SELECTOR, args);

        contract_address.into()
    }

    fn invoke(&self, contract: ContractAddress, function: &str, args: Vec<Felt>) {
        use std::iter;

        let nonce = self.master_nonce.replace_with(|&mut n| n + Felt::ONE);
        let sender_address = *self.master_address.get().expect("must be initialized first");
        let selector = get_selector_from_name(function).unwrap();

        let args_len = Felt::from_usize(args.len()).unwrap();
        let calldata: Vec<Felt> = iter::once(Felt::ONE)
            .chain(iter::once(contract.into()))
            .chain(iter::once(selector))
            .chain(iter::once(Felt::ZERO))
            .chain(iter::once(args_len))
            .chain(iter::once(args_len))
            .chain(args)
            .collect();

        let mut transaction = InvokeTxV1 {
            chain_id: self.chain_spec.id,
            signature: Vec::new(),
            sender_address,
            max_fee: 0,
            calldata,
            nonce,
        };

        let tx_hash = InvokeTx::V1(transaction.clone()).calculate_hash(false);
        let signature = self.master_signer.sign(&tx_hash).unwrap();
        transaction.signature = vec![signature.r, signature.s];

        self.transactions.borrow_mut().push(ExecutableTxWithHash {
            transaction: ExecutableTx::Invoke(InvokeTx::V1(transaction)),
            hash: tx_hash,
        });
    }

    fn deploy_predeployed_dev_account(&self, account: &DevGenesisAccount) -> ContractAddress {
        let signer = SigningKey::from_secret_scalar(account.private_key);
        let pubkey = signer.verifying_key().scalar();

        let class_hash = account.class_hash;
        let calldata = vec![pubkey];
        let account_address = get_contract_address(account.salt, class_hash, &calldata, Felt::ZERO);

        let tx_hash = compute_deploy_account_v1_tx_hash(
            account_address,
            &calldata,
            class_hash,
            account.salt,
            0,
            self.chain_spec.id.into(),
            Felt::ZERO,
            false,
        );

        let signature = signer.sign(&tx_hash).unwrap();

        let transaction = ExecutableTx::DeployAccount(DeployAccountTx::V1(DeployAccountTxV1 {
            signature: vec![signature.r, signature.s],
            contract_address: account_address.into(),
            constructor_calldata: calldata,
            chain_id: self.chain_spec.id,
            contract_address_salt: account.salt,
            nonce: Felt::ZERO,
            max_fee: 0,
            class_hash,
        }));

        let tx_hash = transaction.calculate_hash(false);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });

        account_address.into()
    }

    fn deploy_predeployed_account(&self, salt: Felt, public_key: Felt) -> ContractAddress {
        self.deploy(contracts::Account::HASH, vec![public_key], salt)
    }

    fn build_master_account(&self) {
        let account_class_hash = self.legacy_declare(contracts::GenesisAccount::CLASS.clone());

        let master_pubkey = self.master_signer.verifying_key().scalar();
        let calldata = vec![master_pubkey];
        let salt = Felt::ONE;
        let master_address = get_contract_address(salt, account_class_hash, &calldata, Felt::ZERO);

        self.master_address.set(master_address.into()).expect("must be uninitialized");

        let deploy_account_tx_hash = compute_deploy_account_v1_tx_hash(
            master_address,
            &calldata,
            account_class_hash,
            salt,
            0,
            self.chain_spec.id.into(),
            Felt::ZERO,
            false,
        );

        let signature = self.master_signer.sign(&deploy_account_tx_hash).unwrap();

        let transaction = ExecutableTx::DeployAccount(DeployAccountTx::V1(DeployAccountTxV1 {
            signature: vec![signature.r, signature.s],
            nonce: Felt::ZERO,
            max_fee: 0,
            contract_address_salt: salt,
            contract_address: master_address.into(),
            constructor_calldata: calldata,
            class_hash: account_class_hash,
            chain_id: self.chain_spec.id,
        }));

        let tx_hash = transaction.calculate_hash(false);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });
        self.master_nonce.replace(Nonce::ONE);
    }

    fn build_core_contracts(&mut self) {
        let udc_class_hash = self.legacy_declare(contracts::UniversalDeployer::CLASS.clone());
        self.deploy(udc_class_hash, Vec::new(), Felt::ZERO);

        let master_address = *self.master_address.get().expect("must be initialized first");

        let ctor_args = vec![
            short_string!("Starknet Token"),
            short_string!("STRK"),
            felt!("0x12"),
            Felt::from_u128(u128::MAX).unwrap(),
            Felt::from_u128(u128::MAX).unwrap(),
            master_address.into(),
        ];

        let erc20_class_hash = self.legacy_declare(contracts::LegacyERC20::CLASS.clone());
        let fee_token_address = self.deploy(erc20_class_hash, ctor_args, Felt::ZERO);

        self.fee_token.set(fee_token_address).expect("must be uninitialized");
    }

    fn build_allocated_accounts(&mut self) {
        let default_account_class_hash = self.declare(contracts::Account::CLASS.clone());

        for (expected_addr, account) in self.chain_spec.genesis.accounts() {
            if account.class_hash() != default_account_class_hash {
                panic!(
                    "unexpected account class hash; expected {default_account_class_hash:#x}, got \
                     {:#x}",
                    account.class_hash()
                )
            }

            let address = match account {
                GenesisAccountAlloc::DevAccount(account) => {
                    self.deploy_predeployed_dev_account(account)
                }
                GenesisAccountAlloc::Account(account) => {
                    self.deploy_predeployed_account(account.salt, account.public_key)
                }
            };

            debug_assert_eq!(&address, expected_addr);

            if let Some(amount) = account.balance() {
                self.transfer_balance(address, amount)
            }
        }
    }

    fn transfer_balance(&self, recipient: ContractAddress, balance: U256) {
        let fee_token = *self.fee_token.get().expect("must be initialized first");

        let (low_amount, high_amount) = split_u256(balance);
        let args = vec![recipient.into(), low_amount, high_amount];

        const TRANSFER: &str = "transfer";
        self.invoke(fee_token, TRANSFER, args);
    }

    pub fn build(mut self) -> Vec<ExecutableTxWithHash> {
        self.build_master_account();
        self.build_core_contracts();
        self.build_allocated_accounts();
        self.transactions.into_inner()
    }
}

#[cfg(test)]
mod tests {

    use alloy_primitives::U256;
    use katana_contracts::contracts;
    use katana_executor::implementation::blockifier::cache::ClassCache;
    use katana_executor::implementation::blockifier::BlockifierFactory;
    use katana_executor::{BlockLimits, ExecutorFactory};
    use katana_primitives::chain::ChainId;
    use katana_primitives::class::ClassHash;
    use katana_primitives::contract::Nonce;
    use katana_primitives::env::CfgEnv;
    use katana_primitives::genesis::allocation::{
        DevAllocationsGenerator, GenesisAccount, GenesisAccountAlloc, GenesisAllocation,
    };
    use katana_primitives::genesis::constant::{
        DEFAULT_PREFUNDED_ACCOUNT_BALANCE, DEFAULT_UDC_ADDRESS,
    };
    use katana_primitives::genesis::Genesis;
    use katana_primitives::transaction::TxType;
    use katana_primitives::Felt;
    use katana_provider::providers::db::DbProvider;
    use katana_provider::traits::state::StateFactoryProvider;
    use url::Url;

    use super::GenesisTransactionsBuilder;
    use crate::rollup::{ChainSpec, FeeContract, DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS};
    use crate::SettlementLayer;

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
        let fee_contract = FeeContract::default();

        let settlement = SettlementLayer::Starknet {
            block: 0,
            id: ChainId::default(),
            account: Default::default(),
            core_contract: Default::default(),
            rpc_url: Url::parse("http://localhost:5050").unwrap(),
        };

        ChainSpec { id, genesis, settlement, fee_contract }
    }

    fn executor(chain_spec: &ChainSpec) -> BlockifierFactory {
        BlockifierFactory::new(
            CfgEnv {
                chain_id: chain_spec.id,
                validate_max_n_steps: u32::MAX,
                invoke_tx_max_n_steps: u32::MAX,
                max_recursion_depth: usize::MAX,
                ..Default::default()
            },
            Default::default(),
            BlockLimits::default(),
            ClassCache::new().unwrap(),
        )
    }

    #[test]
    fn valid_transactions() {
        let chain_spec = chain_spec(1, true);

        let provider = DbProvider::new_in_memory();
        let ef = executor(&chain_spec);

        let mut executor = ef.with_state(provider.latest().unwrap());
        executor.execute_block(chain_spec.block()).expect("failed to execute genesis block");

        let output = executor.take_execution_output().unwrap();

        for (i, (.., result)) in output.transactions.iter().enumerate() {
            assert!(result.is_success(), "tx {i} failed; {result:?}");
        }
    }

    #[test]
    fn genesis_states() {
        let chain_spec = chain_spec(1, true);

        let provider = DbProvider::new_in_memory();
        let ef = executor(&chain_spec);

        let mut executor = ef.with_state(provider.latest().unwrap());
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
}
