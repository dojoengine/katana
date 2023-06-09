use std::{fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use blockifier::{
    abi::abi_utils::get_storage_var_address,
    execution::contract_class::{ContractClass, ContractClassV0},
};
use rand::{rngs::SmallRng, RngCore, SeedableRng};
use starknet::{core::types::FieldElement, signers::SigningKey};
use starknet_api::{
    core::{calculate_contract_address, ClassHash, ContractAddress, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
    transaction::{Calldata, ContractAddressSalt},
};

use crate::{
    constants::{DEFAULT_ACCOUNT_CONTRACT, DEFAULT_ACCOUNT_CONTRACT_CLASS_HASH, FEE_TOKEN_ADDRESS},
    state::DictStateReader,
    util::compute_legacy_class_hash,
};

#[derive(Debug, Clone)]
pub struct Account {
    pub balance: StarkFelt,
    pub class_hash: ClassHash,
    pub public_key: StarkFelt,
    pub private_key: StarkFelt,
    pub contract_class: ContractClass,
    pub account_address: ContractAddress,
}

impl Account {
    pub fn new(
        balance: StarkFelt,
        public_key: StarkFelt,
        private_key: StarkFelt,
        class_hash: ClassHash,
        contract_class: ContractClass,
    ) -> Self {
        let account_address = calculate_contract_address(
            ContractAddressSalt(stark_felt!(666)),
            class_hash,
            &Calldata(Arc::new(vec![public_key])),
            ContractAddress(patricia_key!(0)),
        )
        .expect("should calculate contract address");

        Self {
            balance,
            public_key,
            private_key,
            class_hash,
            contract_class,
            account_address,
        }
    }

    pub fn deploy(&self, state: &mut DictStateReader) {
        self.declare(state);

        // set the contract
        state
            .address_to_class_hash
            .insert(self.account_address, self.class_hash);
        // set the balance in the FEE CONTRACT
        state.storage_view.insert(
            (
                ContractAddress(patricia_key!(*FEE_TOKEN_ADDRESS)),
                get_storage_var_address("ERC20_balances", &[*self.account_address.0.key()])
                    .unwrap(),
            ),
            self.balance,
        );
        // set the public key in the account contract
        state.storage_view.insert(
            (
                self.account_address,
                get_storage_var_address("Account_public_key", &[]).unwrap(),
            ),
            self.public_key,
        );
    }

    fn declare(&self, state: &mut DictStateReader) {
        state
            .class_hash_to_class
            .insert(self.class_hash, self.contract_class.clone());
    }
}

#[derive(Debug, Clone)]
pub struct PredeployedAccounts {
    pub seed: [u8; 32],
    pub accounts: Vec<Account>,
    pub initial_balance: StarkFelt,
    pub contract_class: ContractClass,
}

impl PredeployedAccounts {
    pub fn initialize(
        total: u8,
        seed: [u8; 32],
        initial_balance: StarkFelt,
        contract_class_path: Option<PathBuf>,
    ) -> Result<Self> {
        let (class_hash, contract_class) = if let Some(path) = contract_class_path {
            let contract_class_str = fs::read_to_string(path)?;
            let contract_class = serde_json::from_str::<ContractClassV0>(&contract_class_str)
                .expect("can deserialize contract class");
            let class_hash = compute_legacy_class_hash(&contract_class_str)
                .expect("can compute legacy contract class hash");

            (class_hash, ContractClass::V0(contract_class))
        } else {
            Self::default_account_class()
        };

        let accounts = Self::generate_accounts(
            total,
            seed,
            initial_balance,
            class_hash,
            contract_class.clone(),
        );

        Ok(Self {
            seed,
            accounts,
            contract_class,
            initial_balance,
        })
    }

    pub fn deploy_accounts(&self, state: &mut DictStateReader) {
        for account in &self.accounts {
            account.deploy(state);
        }
    }

    pub fn display(&self) -> String {
        fn print_account(account: &Account) -> String {
            format!(
                r"
| Account address |  {} 
| Private key     |  {}
| Public key      |  {}",
                account.account_address.0.key(),
                account.private_key,
                account.public_key
            )
        }

        self.accounts
            .iter()
            .map(print_account)
            .collect::<Vec<String>>()
            .join("\n")
    }

    fn generate_accounts(
        total: u8,
        seed: [u8; 32],
        balance: StarkFelt,
        class_hash: ClassHash,
        contract_class: ContractClass,
    ) -> Vec<Account> {
        let mut seed = seed;
        let mut accounts = vec![];

        for _ in 0..total {
            let mut rng = SmallRng::from_seed(seed);
            let mut private_key_bytes = [0u8; 32];

            rng.fill_bytes(&mut private_key_bytes);
            private_key_bytes[0] = 0;
            seed = private_key_bytes;

            let private_key =
                StarkFelt::new(private_key_bytes).expect("should create StarkFelt from bytes");

            accounts.push(Account::new(
                balance,
                compute_public_key_from_private_key(private_key),
                private_key,
                class_hash,
                contract_class.clone(),
            ));
        }

        accounts
    }

    pub fn default_account_class() -> (ClassHash, ContractClass) {
        (
            ClassHash(*DEFAULT_ACCOUNT_CONTRACT_CLASS_HASH),
            (*DEFAULT_ACCOUNT_CONTRACT).clone(),
        )
    }
}

// TODO: remove starknet-rs dependency
fn compute_public_key_from_private_key(private_key: StarkFelt) -> StarkFelt {
    StarkFelt::from(
        SigningKey::from_secret_scalar(FieldElement::from(private_key))
            .verifying_key()
            .scalar(),
    )
}
