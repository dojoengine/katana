use anyhow::Result;
use rand::Rng;
use serde_json::{json, Value};
use starknet::core::types::Felt;
use starknet_crypto::{pedersen_hash, PrivateKey, PoseidonHasher};
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::client::Account;

/// ERC20 transfer transaction builder
pub struct ERC20Transfer {
    pub contract_address: Felt,
    pub recipient: Felt,
    pub amount_low: Felt,
    pub amount_high: Felt,
}

impl ERC20Transfer {
    pub fn new_random() -> Self {
        let mut rng = rand::thread_rng();
        
        // Use default ERC20 contract from Katana genesis
        let contract_address = Felt::from_hex(
            "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
        ).unwrap();

        // Random recipient (keep it simple, use a few predefined addresses)
        let recipients = [
            Felt::from_hex("0x1").unwrap(),
            Felt::from_hex("0x2").unwrap(),
            Felt::from_hex("0x3").unwrap(),
            Felt::from_hex("0x517ececd29116499f4a1b64b094da79ba08dfd54a3edaa316134c41f8160973").unwrap(),
        ];
        let recipient = recipients[rng.gen_range(0..recipients.len())];

        // Random amount between 1 and 1000
        let amount = rng.gen_range(1..=1000);

        Self {
            contract_address,
            recipient,
            amount_low: Felt::from(amount),
            amount_high: Felt::ZERO,
        }
    }

    pub fn build_invoke_transaction(&self, account: &Account, nonce: Felt) -> Result<Value> {
        // transfer(recipient, amount)
        let calldata = vec![
            self.recipient,
            self.amount_low,
            self.amount_high,
        ];

        // Prepare transaction for signing
        let tx_hash = self.calculate_transaction_hash(account, &calldata, nonce)?;
        let (r, s) = account.sign_transaction_hash(tx_hash)?;

        let transaction = json!({
            "type": "INVOKE",
            "version": "0x1",
            "max_fee": "0x4f3878200000", // 1 ETH in wei (high fee for speed)
            "signature": [
                format!("{:#x}", r),
                format!("{:#x}", s)
            ],
            "nonce": format!("{:#x}", nonce),
            "sender_address": format!("{:#x}", account.address),
            "calldata": calldata.iter().map(|x| format!("{:#x}", x)).collect::<Vec<_>>()
        });

        Ok(transaction)
    }

    fn calculate_transaction_hash(&self, account: &Account, calldata: &[Felt], nonce: Felt) -> Result<Felt> {
        // This is a simplified transaction hash calculation
        // In production, this should follow the exact StarkNet transaction hash spec
        
        let mut hasher = PoseidonHasher::new();
        
        // Add transaction fields to hash
        hasher.update(Felt::from_hex("invoke").unwrap_or(Felt::ZERO)); // tx type
        hasher.update(Felt::ONE); // version
        hasher.update(account.address); // sender
        hasher.update(Felt::ZERO); // entry point selector (for invoke v1)
        
        // Add calldata hash
        let mut calldata_hasher = PoseidonHasher::new();
        for data in calldata {
            calldata_hasher.update(*data);
        }
        hasher.update(calldata_hasher.finalize());
        
        hasher.update(Felt::from_hex("0x4f3878200000").unwrap()); // max_fee
        hasher.update(nonce);

        Ok(hasher.finalize())
    }
}

/// Account deployment transaction
pub struct AccountDeployment {
    pub class_hash: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub salt: Felt,
}

impl AccountDeployment {
    pub fn new_random() -> Self {
        let mut rng = rand::thread_rng();
        
        // Standard account class hash (Katana default)
        let class_hash = Felt::from_hex(
            "0x025ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918"
        ).unwrap();

        // Random salt
        let salt = Felt::from(rng.gen::<u64>());

        // Constructor calldata for account (public key)
        let constructor_calldata = vec![
            Felt::from(rng.gen::<u64>()), // public key (mock)
        ];

        Self {
            class_hash,
            constructor_calldata,
            salt,
        }
    }

    pub fn build_deploy_account_transaction(&self, account: &Account, nonce: Felt) -> Result<Value> {
        let tx_hash = self.calculate_deploy_hash(account, nonce)?;
        let (r, s) = account.sign_transaction_hash(tx_hash)?;

        let transaction = json!({
            "type": "DEPLOY_ACCOUNT",
            "version": "0x1",
            "max_fee": "0x4f3878200000",
            "signature": [
                format!("{:#x}", r),
                format!("{:#x}", s)
            ],
            "nonce": format!("{:#x}", nonce),
            "class_hash": format!("{:#x}", self.class_hash),
            "contract_address_salt": format!("{:#x}", self.salt),
            "constructor_calldata": self.constructor_calldata
                .iter()
                .map(|x| format!("{:#x}", x))
                .collect::<Vec<_>>()
        });

        Ok(transaction)
    }

    fn calculate_deploy_hash(&self, account: &Account, nonce: Felt) -> Result<Felt> {
        let mut hasher = PoseidonHasher::new();
        
        hasher.update(Felt::from_hex("deploy_account").unwrap_or(Felt::ZERO));
        hasher.update(Felt::ONE); // version
        hasher.update(account.address);
        hasher.update(self.class_hash);
        hasher.update(self.salt);
        
        // Constructor calldata hash
        let mut calldata_hasher = PoseidonHasher::new();
        for data in &self.constructor_calldata {
            calldata_hasher.update(*data);
        }
        hasher.update(calldata_hasher.finalize());
        
        hasher.update(Felt::from_hex("0x4f3878200000").unwrap()); // max_fee
        hasher.update(nonce);

        Ok(hasher.finalize())
    }
}

/// Transaction types for load testing
#[derive(Debug, Clone)]
pub enum TransactionType {
    ERC20Transfer,
    AccountDeploy,
    ContractCall,
}

impl TransactionType {
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        match rng.gen_range(0..3) {
            0 => Self::ERC20Transfer,
            1 => Self::AccountDeploy,
            _ => Self::ContractCall,
        }
    }

    pub fn build_transaction(&self, account: &Account, nonce: Felt) -> Result<Value> {
        match self {
            Self::ERC20Transfer => {
                let transfer = ERC20Transfer::new_random();
                transfer.build_invoke_transaction(account, nonce)
            }
            Self::AccountDeploy => {
                let deploy = AccountDeployment::new_random();
                deploy.build_deploy_account_transaction(account, nonce)
            }
            Self::ContractCall => {
                // For now, use ERC20 transfer as a contract call
                let transfer = ERC20Transfer::new_random();
                transfer.build_invoke_transaction(account, nonce)
            }
        }
    }
}
