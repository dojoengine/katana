use std::sync::Arc;

use blockifier::abi::abi_utils::get_storage_var_address;
use rand::{rngs::SmallRng, RngCore, SeedableRng};
use starknet::{core::types::FieldElement, signers::SigningKey};
use starknet_api::{
    core::{calculate_contract_address, ClassHash, ContractAddress, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
    transaction::{Calldata, ContractAddressSalt},
};

use crate::{
    constants::{
        ACCOUNT_CONTRACT_CLASS_HASH, DEFAULT_PREFUNDED_ACCOUNT_BALANCE, FEE_ERC20_CONTRACT_ADDRESS,
    },
    state::DictStateReader,
};

#[derive(Debug)]
pub struct Account {
    pub balance: StarkFelt,
    pub class_hash: ClassHash,
    pub public_key: StarkFelt,
    pub private_key: StarkFelt,
    pub account_address: ContractAddress,
}

impl Account {
    pub fn new(
        balance: StarkFelt,
        public_key: StarkFelt,
        private_key: StarkFelt,
        class_hash: ClassHash,
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
            account_address,
        }
    }

    pub fn deploy(&self, state: &mut DictStateReader) {
        // set the contract
        state
            .address_to_class_hash
            .insert(self.account_address, self.class_hash);
        // set the balance in the FEE CONTRACT
        state.storage_view.insert(
            (
                ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
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
}

pub struct PredeployedAccounts {
    pub seed: [u8; 32],
    pub accounts: Vec<Account>,
    pub initial_balance: StarkFelt,
}

impl PredeployedAccounts {
    pub fn new(total: usize, seed: [u8; 32], initial_balance: StarkFelt) -> Self {
        let accounts = Self::generate_accounts(total, seed, initial_balance);

        Self {
            seed,
            accounts,
            initial_balance,
        }
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
Account address | {} 
Private key     | {}
Public key      | {}",
                account.account_address.0.key(),
                account.private_key,
                account.public_key
            )
        }

        format!(
            "{}",
            self.accounts
                .iter()
                .map(print_account)
                .collect::<Vec<String>>()
                .join("")
        )
    }

    fn generate_accounts(total: usize, seed: [u8; 32], balance: StarkFelt) -> Vec<Account> {
        let mut seed = seed;
        let mut accounts = vec![];

        for i in 0..total {
            let mut rng = SmallRng::from_seed(seed);
            let mut private_key_bytes = [0u8; 32];

            rng.fill_bytes(&mut private_key_bytes);
            private_key_bytes[0] = 0;
            seed = private_key_bytes;

            let private_key =
                StarkFelt::new(private_key_bytes).expect("should create StarkFelt from bytes");

            accounts.push(Account::new(
                balance,
                compute_public_key_from_private_key(&private_key),
                private_key,
                ClassHash(stark_felt!(ACCOUNT_CONTRACT_CLASS_HASH)),
            ));
        }

        accounts
    }
}

impl Default for PredeployedAccounts {
    fn default() -> Self {
        Self::new(
            10,
            [0u8; 32],
            stark_felt!(DEFAULT_PREFUNDED_ACCOUNT_BALANCE),
        )
    }
}

// TODO: remove starknet-rs dependency
fn compute_public_key_from_private_key(private_key: &StarkFelt) -> StarkFelt {
    StarkFelt::from(
        SigningKey::from_secret_scalar(
            FieldElement::from_byte_slice_be(private_key.bytes()).unwrap(),
        )
        .verifying_key()
        .scalar(),
    )
}
