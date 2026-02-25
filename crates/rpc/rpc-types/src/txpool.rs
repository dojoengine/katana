use std::collections::HashMap;

use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};

/// Response for `txpool_status`.
///
/// Contains the count of transactions in the node's local pool (not a network-wide mempool).
/// `queued` is currently always 0 (no queued pool yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolStatus {
    /// Number of transactions ready for execution.
    pub pending: u64,
    /// Number of transactions waiting on a nonce gap. Always 0 for now.
    pub queued: u64,
}

/// A lightweight representation of a pooled transaction.
///
/// Contains only the fields needed for pool inspection. The full transaction
/// body can be fetched via `starknet_getTransactionByHash` using the `hash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolTransaction {
    pub hash: TxHash,
    pub nonce: Nonce,
    pub sender: ContractAddress,
    pub max_fee: u128,
    pub tip: u64,
}

/// Response for `txpool_content` and `txpool_contentFrom`.
///
/// Transactions are grouped first by sender address, then by nonce.
/// `queued` is currently always empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolContent {
    /// Transactions ready for execution, keyed by sender then nonce.
    pub pending: HashMap<ContractAddress, HashMap<Nonce, TxPoolTransaction>>,
    /// Transactions waiting on a nonce gap. Always empty for now.
    pub queued: HashMap<ContractAddress, HashMap<Nonce, TxPoolTransaction>>,
}

/// Response for `txpool_inspect`.
///
/// Same structure as [`TxPoolContent`] but each transaction is a human-readable
/// summary string instead of a structured object.
/// `queued` is currently always empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPoolInspect {
    /// Textual summaries of pending transactions, keyed by sender then nonce.
    pub pending: HashMap<ContractAddress, HashMap<Nonce, String>>,
    /// Textual summaries of queued transactions. Always empty for now.
    pub queued: HashMap<ContractAddress, HashMap<Nonce, String>>,
}
