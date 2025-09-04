#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod ordering;
pub mod pool;
pub mod validation;

pub use katana_pool_api::{PendingTransactions, PoolOrd, PoolTransaction, TransactionPool};
use katana_primitives::transaction::ExecutableTxWithHash;
use ordering::FiFo;
use pool::Pool;
use validation::stateful::TxValidator;

/// Katana default transacstion pool type.
pub type TxPool = Pool<ExecutableTxWithHash, TxValidator, FiFo<ExecutableTxWithHash>>;

#[cfg(test)]
mod tests {

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use futures::StreamExt;
    use katana_pool_api::{PoolTransaction, TransactionPool};
    use tokio::task::yield_now;

    use crate::ordering;
    use crate::pool::test_utils::PoolTx;
    use crate::pool::Pool;
    use crate::validation::NoopValidator;

    #[tokio::test]
    async fn pending_transactions() {
        let pool = Pool::new(NoopValidator::<PoolTx>::new(), ordering::FiFo::new());

        let first_batch = [
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
        ];

        for tx in &first_batch {
            pool.add_transaction(tx.clone()).expect("failed to add tx");
        }

        let mut pendings = pool.pending_transactions();

        // exhaust all the first batch transactions
        for expected in &first_batch {
            let actual = pendings.next().await.map(|t| t.tx).unwrap();
            assert_eq!(expected, actual.as_ref());
        }

        let second_batch = [
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
            PoolTx::new(),
        ];

        for tx in &second_batch {
            pool.add_transaction(tx.clone()).expect("failed to add tx");
        }

        // exhaust all the first batch transactions
        for expected in &second_batch {
            let actual = pendings.next().await.map(|t| t.tx).unwrap();
            assert_eq!(expected, actual.as_ref());
        }

        // Check that all the added transaction is still in the pool because we haven't removed it
        // yet.
        let all = [first_batch, second_batch].concat();
        for tx in all {
            assert!(pool.contains(tx.hash()));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscription_stream_wakeup() {
        let pool = Pool::new(NoopValidator::<PoolTx>::new(), ordering::FiFo::new());
        let mut pending = pool.pending_transactions();

        // Spawn a task that will add a transaction after a delay
        let pool_clone = pool.clone();

        let txs = [PoolTx::new(), PoolTx::new(), PoolTx::new()];
        let txs_clone = txs.clone();

        let has_polled_once = Arc::new(AtomicBool::new(false));
        let has_polled_once_clone = has_polled_once.clone();

        tokio::spawn(async move {
            while !has_polled_once_clone.load(Ordering::SeqCst) {
                yield_now().await;
            }

            for tx in txs_clone {
                pool_clone.add_transaction(tx).expect("failed to add tx");
            }
        });

        // Check that first poll_next returns Pending because no pending transaction has been added
        // to the pool yet
        assert!(futures_util::poll!(pending.next()).is_pending());
        has_polled_once.store(true, Ordering::SeqCst);

        for tx in txs {
            let received = pending.next().await.unwrap();
            assert_eq!(&tx, received.tx.as_ref());
        }
    }
}
