use std::future::Future;
use std::sync::Arc;

use futures::channel::mpsc::Receiver;
use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;

mod ordering;
mod pending;
mod subscription;
mod tx;
pub mod validation;

pub use ordering::*;
pub use pending::*;
pub use subscription::*;
pub use tx::*;

use crate::validation::{InvalidTransactionError, Validator};

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(Box<InvalidTransactionError>),
    #[error("Internal error: {0}")]
    Internal(Box<dyn core::error::Error + Send + Sync + 'static>),
}

pub type PoolResult<T> = Result<T, PoolError>;

/// Represents a complete transaction pool.
pub trait TransactionPool: Send + Sync {
    /// The pool's transaction type.
    type Transaction: PoolTransaction;

    /// The ordering mechanism to use. This is used to determine
    /// how transactions are being ordered within the pool.
    type Ordering: PoolOrd<Transaction = Self::Transaction>;

    /// Transaction validation before adding to the pool.
    type Validator: Validator<Transaction = Self::Transaction>;

    /// Add a new transaction to the pool.
    fn add_transaction(
        &self,
        tx: Self::Transaction,
    ) -> impl Future<Output = PoolResult<TxHash>> + Send;

    /// Returns a [`Stream`](futures::Stream) which yields pending transactions - transactions that
    /// can be executed - from the pool.
    fn pending_transactions(&self) -> PendingTransactions<Self::Transaction, Self::Ordering>;

    /// Check if the pool contains a transaction with the given hash.
    fn contains(&self, hash: TxHash) -> bool;

    /// Get a transaction from the pool by its hash.
    fn get(&self, hash: TxHash) -> Option<Arc<Self::Transaction>>;

    fn add_listener(&self) -> Receiver<TxHash>;

    /// Removes a list of transactions from the pool according to their hashes.
    fn remove_transactions(&self, hashes: &[TxHash]);

    /// Get the total number of transactions in the pool.
    fn size(&self) -> usize;

    /// Get a reference to the pool's validator.
    fn validator(&self) -> &Self::Validator;

    /// Get the next expected nonce for an account based on pending transactions in the pool.
    ///
    /// Returns `Some(nonce)` if there are pending transactions for this account,
    /// where nonce is the next expected nonce (highest pending nonce + 1).
    /// Returns `None` if no pending transactions exist for this account.
    fn get_nonce(&self, address: ContractAddress) -> Option<Nonce>;

    /// Returns a point-in-time snapshot of all transactions currently in the pool.
    fn take_transactions_snapshot(&self) -> Vec<Arc<Self::Transaction>>;
}

// the transaction type is recommended to implement a cheap clone (eg ref-counting) so that it
// can be cloned around to different pools as necessary.
pub trait PoolTransaction: Clone + Send + Sync {
    /// return the tx hash.
    fn hash(&self) -> TxHash;

    /// return the tx nonce.
    fn nonce(&self) -> Nonce;

    /// return the tx sender.
    fn sender(&self) -> ContractAddress;

    /// return the max fee that tx is willing to pay.
    fn max_fee(&self) -> u128;

    /// return the tx tip.
    fn tip(&self) -> u64;
}
