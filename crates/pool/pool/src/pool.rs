use core::fmt;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use futures::channel::mpsc::{channel, Receiver, Sender};
use katana_pool_api::validation::{InvalidTransactionError, ValidationOutcome, Validator};
use katana_pool_api::{
    PendingTransactions, PendingTx, PoolError, PoolOrd, PoolResult, PoolTransaction, Subscription,
    TransactionPool, TxId,
};
use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::{error, trace, warn, Instrument};

#[derive(Debug)]
pub struct Pool<T, V, O>
where
    T: PoolTransaction,
    V: Validator<Transaction = T>,
    O: PoolOrd<Transaction = T>,
{
    inner: Arc<Inner<T, V, O>>,
}

#[derive(Debug)]
struct Inner<T, V, O: PoolOrd> {
    /// List of all valid txs in the pool.
    transactions: RwLock<BTreeSet<PendingTx<T, O>>>,

    /// listeners for incoming txs
    listeners: RwLock<Vec<Sender<TxHash>>>,

    /// subscribers for incoming txs
    subscribers: RwLock<Vec<mpsc::UnboundedSender<PendingTx<T, O>>>>,

    /// the tx validator
    validator: V,

    /// the ordering mechanism used to order the txs in the pool
    ordering: O,
}

impl<T, V, O> Pool<T, V, O>
where
    T: PoolTransaction,
    V: Validator<Transaction = T>,
    O: PoolOrd<Transaction = T>,
{
    /// Creates a new [Pool] with the given [Validator] and [PoolOrd] mechanism.
    pub fn new(validator: V, ordering: O) -> Self {
        Self {
            inner: Arc::new(Inner {
                ordering,
                validator,
                transactions: Default::default(),
                subscribers: Default::default(),
                listeners: Default::default(),
            }),
        }
    }

    /// Notifies all listeners about the new incoming transaction.
    fn notify_listener(&self, hash: TxHash) {
        let mut listener = self.inner.listeners.write();
        // this is basically a retain but with mut reference
        for n in (0..listener.len()).rev() {
            let mut listener_tx = listener.swap_remove(n);
            let retain = match listener_tx.try_send(hash) {
                Ok(()) => true,
                Err(e) => {
                    if e.is_full() {
                        warn!(
                            hash = format!("{hash:#x}"),
                            "Unable to send tx notification because channel is full."
                        );
                        true
                    } else {
                        false
                    }
                }
            };

            if retain {
                listener.push(listener_tx)
            }
        }
    }

    fn notify_subscribers(&self, tx: PendingTx<T, O>) {
        let mut subscribers = self.inner.subscribers.write();
        // this is basically a retain but with mut reference
        for n in (0..subscribers.len()).rev() {
            let sender = subscribers.swap_remove(n);
            let retain = match sender.send(tx.clone()) {
                Ok(()) => true,
                Err(error) => {
                    warn!(%error, "Subscription channel closed");
                    false
                }
            };

            if retain {
                subscribers.push(sender)
            }
        }
    }

    // notify both listener and subscribers
    fn notify(&self, tx: PendingTx<T, O>) {
        self.notify_listener(tx.tx.hash());
        self.notify_subscribers(tx);
    }

    fn subscribe(&self) -> Subscription<T, O> {
        let (subscriber, tx) = Subscription::new();
        self.inner.subscribers.write().push(tx);
        subscriber
    }
}

impl<T, V, O> TransactionPool for Pool<T, V, O>
where
    T: PoolTransaction + fmt::Debug,
    V: Validator<Transaction = T> + Send + Sync,
    O: PoolOrd<Transaction = T> + Send + Sync,
    O::PriorityValue: Send + Sync,
{
    type Transaction = T;
    type Validator = V;
    type Ordering = O;

    fn add_transaction(&self, tx: T) -> impl Future<Output = PoolResult<TxHash>> + Send {
        let pool = self.clone();

        let hash = tx.hash();
        let id = TxId::new(tx.sender(), tx.nonce());

        async move {
            match pool.inner.validator.validate(tx).await {
	            Ok(outcome) => {
	                match outcome {
	                    ValidationOutcome::Valid(tx) => {
	                        // get the priority of the validated tx
	                        let priority = pool.inner.ordering.priority(&tx);
	                        let tx = PendingTx::new(id, tx, priority);

	                        // insert the tx in the pool
	                        pool.inner.transactions.write().insert(tx.clone());
	                        trace!(target: "pool", "Transaction added to the pool");

	                        pool.notify(tx);

	                        Ok(hash)
	                    }

	                    // TODO: create a small cache for rejected transactions to respect the rpc spec
	                    // `getTransactionStatus`
	                    ValidationOutcome::Invalid { error, .. } => {
	                        warn!(target: "pool", ?error, "Invalid transaction.");
	                        Err(PoolError::InvalidTransaction(Box::new(error)))
	                    }

	                    // return as error for now but ideally we should kept the tx in a separate
	                    // queue and revalidate it when the parent tx is added to the pool
	                    ValidationOutcome::Dependent { tx, tx_nonce, current_nonce } => {
	                        trace!(target: "pool", %tx_nonce, %current_nonce, "Dependent transaction.");
	                        let err = InvalidTransactionError::InvalidNonce {
	                            address: tx.sender(),
	                            current_nonce,
	                            tx_nonce,
	                        };
	                        Err(PoolError::InvalidTransaction(Box::new(err)))
	                    }
	                }
	            }

	            Err(error) => {
	                error!(target: "pool", %error, "Failed to validate transaction.");
	                Err(PoolError::Internal(error.error))
	            }
            }
        }
        .instrument(tracing::trace_span!(target: "pool", "pool_add", tx_hash = format!("{hash:#x}")))
    }

    fn pending_transactions(&self) -> PendingTransactions<Self::Transaction, Self::Ordering> {
        // take all the transactions
        PendingTransactions {
            subscription: self.subscribe(),
            all: self.inner.transactions.read().clone().into_iter(),
        }
    }

    // check if a tx is in the pool
    fn contains(&self, hash: TxHash) -> bool {
        self.get(hash).is_some()
    }

    fn get(&self, hash: TxHash) -> Option<Arc<T>> {
        self.inner
            .transactions
            .read()
            .iter()
            .find(|tx| tx.tx.hash() == hash)
            .map(|t| Arc::clone(&t.tx))
    }

    fn add_listener(&self) -> Receiver<TxHash> {
        const TX_LISTENER_BUFFER_SIZE: usize = 2048;
        let (tx, rx) = channel(TX_LISTENER_BUFFER_SIZE);
        self.inner.listeners.write().push(tx);
        rx
    }

    fn remove_transactions(&self, hashes: &[TxHash]) {
        // retain only transactions that aren't included in the list
        let mut txs = self.inner.transactions.write();
        txs.retain(|t| !hashes.contains(&t.tx.hash()))
    }

    fn size(&self) -> usize {
        self.inner.transactions.read().len()
    }

    fn validator(&self) -> &Self::Validator {
        &self.inner.validator
    }

    fn get_nonce(&self, address: ContractAddress) -> Option<Nonce> {
        self.inner
            .transactions
            .read()
            .iter()
            .filter(|tx| tx.tx.sender() == address)
            .map(|tx| tx.tx.nonce())
            .max()
            .map(|max_nonce| max_nonce + 1)
    }
}

impl<T, V, O> Clone for Pool<T, V, O>
where
    T: PoolTransaction,
    V: Validator<Transaction = T>,
    O: PoolOrd<Transaction = T>,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

#[cfg(test)]
pub(crate) mod test_utils {

    use katana_pool_api::PoolTransaction;
    use katana_primitives::contract::{ContractAddress, Nonce};
    use katana_primitives::Felt;
    use rand::Rng;

    use super::*;

    fn random_bytes<const SIZE: usize>() -> [u8; SIZE] {
        let mut bytes = [0u8; SIZE];
        rand::thread_rng().fill(&mut bytes[..]);
        bytes
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PoolTx {
        tip: u64,
        nonce: Nonce,
        hash: TxHash,
        max_fee: u128,
        sender: ContractAddress,
    }

    impl PoolTx {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self {
                tip: rand::thread_rng().gen(),
                max_fee: rand::thread_rng().gen(),
                hash: TxHash::from_bytes_be(&random_bytes::<32>()),
                nonce: Nonce::from_bytes_be(&random_bytes::<32>()),
                sender: {
                    let felt = Felt::from_bytes_be(&random_bytes::<32>());
                    ContractAddress::from(felt)
                },
            }
        }

        pub fn with_tip(mut self, tip: u64) -> Self {
            self.tip = tip;
            self
        }

        pub fn with_sender(mut self, sender: ContractAddress) -> Self {
            self.sender = sender;
            self
        }

        pub fn with_nonce(mut self, nonce: Nonce) -> Self {
            self.nonce = nonce;
            self
        }
    }

    impl PoolTransaction for PoolTx {
        fn hash(&self) -> TxHash {
            self.hash
        }

        fn max_fee(&self) -> u128 {
            self.max_fee
        }

        fn nonce(&self) -> Nonce {
            self.nonce
        }

        fn sender(&self) -> ContractAddress {
            self.sender
        }

        fn tip(&self) -> u64 {
            self.tip
        }
    }
}

#[cfg(test)]
mod tests {

    use futures::StreamExt;
    use katana_pool_api::{PoolTransaction, TransactionPool};
    use katana_primitives::contract::{ContractAddress, Nonce};
    use katana_primitives::transaction::TxHash;
    use katana_primitives::Felt;

    use super::test_utils::*;
    use super::Pool;
    use crate::ordering::FiFo;
    use crate::validation::NoopValidator;

    /// Tx pool that uses a noop validator and a first-come-first-serve ordering.
    type TestPool = Pool<PoolTx, NoopValidator<PoolTx>, FiFo<PoolTx>>;

    impl TestPool {
        fn test() -> Self {
            Pool::new(NoopValidator::new(), FiFo::new())
        }
    }

    #[tokio::test]
    async fn pool_operations() {
        let txs = [
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
        ];

        let pool = TestPool::test();

        // initially pool should be empty
        assert!(pool.size() == 0);
        assert!(pool.inner.transactions.read().is_empty());

        // add all the txs to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        // all the txs should be in the pool
        assert_eq!(pool.size(), txs.len());
        assert_eq!(pool.inner.transactions.read().len(), txs.len());
        assert!(txs.iter().all(|tx| pool.get(tx.hash()).is_some()));

        // noop validator should consider all txs as valid
        let mut pendings = pool.pending_transactions();

        // bcs we're using fcfs, the order should be the same as the order of the txs submission
        // (position in the array)
        for expected in &txs {
            let actual = pendings.next().await.unwrap();
            assert_eq!(actual.tx.tip(), expected.tip());
            assert_eq!(actual.tx.hash(), expected.hash());
            assert_eq!(actual.tx.nonce(), expected.nonce());
            assert_eq!(actual.tx.sender(), expected.sender());
            assert_eq!(actual.tx.max_fee(), expected.max_fee());
        }

        // remove all transactions
        let hashes = txs.iter().map(|t| t.hash()).collect::<Vec<TxHash>>();
        pool.remove_transactions(&hashes);

        // all txs should've been removed
        assert!(pool.size() == 0);
        assert!(pool.inner.transactions.read().is_empty());
        assert!(txs.iter().all(|tx| pool.get(tx.hash()).is_none()));
    }

    #[tokio::test]
    async fn tx_listeners() {
        let txs = [
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
        ];

        let pool = TestPool::test();
        // register a listener for incoming txs
        let mut listener = pool.add_listener();

        // start adding txs to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        // the channel should contain all the added txs
        let mut counter = 0;
        while let Ok(Some(hash)) = listener.try_next() {
            counter += 1;
            assert!(txs.iter().any(|tx| tx.hash() == hash));
        }

        // we should be notified exactly the same number of txs as we added
        assert_eq!(counter, txs.len());
    }

    #[tokio::test]
    async fn remove_transactions() {
        let pool = TestPool::test();

        let txs = [
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
        ];

        // start adding txs to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        // first check that the transaction are indeed in the pool
        txs.iter().for_each(|tx| {
            assert!(pool.contains(tx.hash()));
        });

        // remove the transactions
        let hashes = txs.iter().map(|t| t.hash()).collect::<Vec<TxHash>>();
        pool.remove_transactions(&hashes);

        // check that the transaction are no longer in the pool
        txs.iter().for_each(|tx| {
            assert!(!pool.contains(tx.hash()));
        });
    }

    #[tokio::test]
    #[ignore = "Txs dependency management not fully implemented yet"]
    async fn dependent_txs_linear_insertion() {
        let pool = TestPool::test();

        // Create 100 transactions with the same sender but increasing nonce
        let total = 100u128;
        let sender = ContractAddress::from(Felt::from_hex("0x1337").unwrap());
        let txs: Vec<PoolTx> = (0..total)
            .map(|i| PoolTx::new().with_sender(sender).with_nonce(Nonce::from(i)))
            .collect();

        // Add all transactions to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        // Get pending transactions
        let mut pendings = pool.pending_transactions();

        // Check that the pending transactions are in the same order as they were added
        for i in 0..total {
            let pending_tx = pendings.next().await.unwrap();
            assert_eq!(pending_tx.tx.nonce(), Nonce::from(i));
            assert_eq!(pending_tx.tx.sender(), sender);
        }
    }

    #[test]
    #[ignore = "Txs dependency management not fully implemented yet"]
    fn dependent_txs_random_insertion() {}

    #[tokio::test]
    async fn get_nonce_returns_none_for_unknown_address() {
        let pool = TestPool::test();
        let unknown_address = ContractAddress::from(Felt::from_hex("0xdead").unwrap());
        assert_eq!(pool.get_nonce(unknown_address), None);
    }

    #[tokio::test]
    async fn get_nonce_returns_next_nonce_for_single_pending_tx() {
        let pool = TestPool::test();
        let sender = ContractAddress::from(Felt::from_hex("0x1337").unwrap());
        let nonce = Nonce::from(5u128);

        let tx = PoolTx::new().with_sender(sender).with_nonce(nonce);
        pool.add_transaction(tx).await.unwrap();

        // Should return nonce + 1
        assert_eq!(pool.get_nonce(sender), Some(Nonce::from(6u128)));
    }

    #[tokio::test]
    async fn get_nonce_returns_highest_nonce_plus_one_for_multiple_txs() {
        let pool = TestPool::test();
        let sender = ContractAddress::from(Felt::from_hex("0x1337").unwrap());

        // Add transactions with nonces 1, 5, 3 (out of order)
        let tx1 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(1u128));
        let tx2 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(3u128));
        let tx3 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(2u128));

        pool.add_transaction(tx1).await.unwrap();
        pool.add_transaction(tx2).await.unwrap();
        pool.add_transaction(tx3).await.unwrap();

        // Should return max(1, 3, 2) + 1 = 4
        assert_eq!(pool.get_nonce(sender), Some(Nonce::from(4u128)));
    }

    #[tokio::test]
    async fn get_nonce_isolated_per_address() {
        let pool = TestPool::test();

        let sender1 = ContractAddress::from(Felt::from_hex("0x1").unwrap());
        let sender2 = ContractAddress::from(Felt::from_hex("0x2").unwrap());
        let sender3 = ContractAddress::from(Felt::from_hex("0x3").unwrap());

        // Add transactions from different senders
        let tx1 = PoolTx::new().with_sender(sender1).with_nonce(Nonce::from(10u128));
        let tx2 = PoolTx::new().with_sender(sender2).with_nonce(Nonce::from(20u128));
        let tx3 = PoolTx::new().with_sender(sender1).with_nonce(Nonce::from(15u128));

        pool.add_transaction(tx1).await.unwrap();
        pool.add_transaction(tx2).await.unwrap();
        pool.add_transaction(tx3).await.unwrap();

        // Each sender should have their own max nonce
        assert_eq!(pool.get_nonce(sender1), Some(Nonce::from(16u128))); // max(10, 15) + 1
        assert_eq!(pool.get_nonce(sender2), Some(Nonce::from(21u128))); // 20 + 1
        assert_eq!(pool.get_nonce(sender3), None); // No txs from sender3
    }

    #[tokio::test]
    async fn get_nonce_updates_after_transaction_removal() {
        let pool = TestPool::test();
        let sender = ContractAddress::from(Felt::from_hex("0x1337").unwrap());

        let tx1 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(1u128));
        let tx2 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(2u128));
        let tx3 = PoolTx::new().with_sender(sender).with_nonce(Nonce::from(3u128));

        let hash1 = tx1.hash();
        let hash3 = tx3.hash();

        pool.add_transaction(tx1).await.unwrap();
        pool.add_transaction(tx2.clone()).await.unwrap();
        pool.add_transaction(tx3).await.unwrap();

        // Should be 4 (max nonce 3 + 1)
        assert_eq!(pool.get_nonce(sender), Some(Nonce::from(4u128)));

        // Remove transactions with nonce 1 and 3
        pool.remove_transactions(&[hash1, hash3]);

        // Should now be 3 (only tx with nonce 2 remains)
        assert_eq!(pool.get_nonce(sender), Some(Nonce::from(3u128)));

        // Remove last transaction
        pool.remove_transactions(&[tx2.hash()]);

        // Should be None (no transactions left)
        assert_eq!(pool.get_nonce(sender), None);
    }
}
