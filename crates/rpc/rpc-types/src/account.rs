use katana_genesis::allocation::GenesisAccountAlloc;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::{Felt, U256};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub address: ContractAddress,
    pub public_key: Felt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<Felt>,
    pub class_hash: ClassHash,
    pub balance: U256,
}

impl Account {
    pub fn new(address: ContractAddress, account: &GenesisAccountAlloc) -> Self {
        Self {
            address,
            public_key: account.public_key(),
            private_key: account.private_key(),
            class_hash: account.class_hash(),
            balance: account.balance().unwrap_or_default(),
        }
    }
}
