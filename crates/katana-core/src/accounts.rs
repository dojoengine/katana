use std::sync::Arc;

use anyhow::Result;
use blockifier::{abi::abi_utils::get_storage_var_address, state::state_api::State};
use rand::{rngs::SmallRng, RngCore, SeedableRng};
use starknet::{core::types::FieldElement, signers::SigningKey};
use starknet_api::{
    core::{calculate_contract_address, ClassHash, ContractAddress, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
    transaction::{Calldata, ContractAddressSalt},
};

use crate::constants::{ACCOUNT_CONTRACT_CLASS_HASH, FEE_ERC20_CONTRACT_ADDRESS};

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

    pub fn deploy(&self, state: &mut dyn State) -> Result<()> {
        // set the contract
        state.set_class_hash_at(self.account_address, self.class_hash)?;
        // set the balance in the FEE CONTRACT
        state.set_storage_at(
            ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
            get_storage_var_address("ERC20_balances", &[*self.account_address.0.key()])?,
            self.balance,
        );
        // set the public key in the account contract
        state.set_storage_at(
            self.account_address,
            get_storage_var_address("Account_public_key", &[])?,
            self.public_key,
        );
        Ok(())
    }
}

pub struct PredeployedAccounts {
    seed: u64,
    total: usize,
    accounts: Vec<Account>,
    initial_balance: StarkFelt,
}

impl PredeployedAccounts {
    pub fn new(total: usize, seed: Option<u64>, initial_balance: StarkFelt) -> Self {
        let seed = seed.unwrap_or(0);
        let accounts = Self::generate_accounts(total, seed, initial_balance);

        Self {
            seed,
            total,
            accounts,
            initial_balance,
        }
    }

    pub fn deploy_accounts(&self, state: &mut dyn State) -> Result<()> {
        for account in &self.accounts {
            account.deploy(state)?;
        }
        Ok(())
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

    fn generate_accounts(total: usize, seed: u64, balance: StarkFelt) -> Vec<Account> {
        let mut accounts = vec![];

        for _ in 0..total {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut private_key_bytes = [0u8; 32];

            rng.fill_bytes(&mut private_key_bytes);
            private_key_bytes[0] = private_key_bytes[0] % 0x10;

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
