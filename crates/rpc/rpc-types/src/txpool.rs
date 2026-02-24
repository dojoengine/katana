use std::collections::HashMap;

use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};

/// Response for `txpool_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolStatus {
    pub pending: u64,
    pub queued: u64,
}

/// A lightweight representation of a pooled transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolTransaction {
    pub hash: TxHash,
    pub nonce: Nonce,
    pub sender: ContractAddress,
    pub max_fee: u128,
    pub tip: u64,
}

/// Response for `txpool_content` and `txpool_contentFrom`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolContent {
    pub pending: HashMap<ContractAddress, HashMap<Nonce, TxPoolTransaction>>,
    pub queued: HashMap<ContractAddress, HashMap<Nonce, TxPoolTransaction>>,
}

/// Response for `txpool_inspect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolInspect {
    pub pending: HashMap<ContractAddress, HashMap<Nonce, String>>,
    pub queued: HashMap<ContractAddress, HashMap<Nonce, String>>,
}
