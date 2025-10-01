use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use katana_pool_api::{PoolOrd, PoolTransaction};

/// First-come-first-serve ordering implementation.
///
/// This ordering implementation can be generic over any transaction type as it doesn't require any
/// context on the tx data itself.
#[derive(Debug)]
pub struct FiFo<T> {
    nonce: AtomicU64,
    _tx: PhantomData<T>,
}

impl<T> FiFo<T> {
    pub fn new() -> Self {
        Self { nonce: AtomicU64::new(0), _tx: PhantomData }
    }

    pub fn assign(&self) -> u64 {
        self.nonce.fetch_add(1, AtomicOrdering::SeqCst)
    }
}

impl<T: PoolTransaction> PoolOrd for FiFo<T> {
    type Transaction = T;
    type PriorityValue = TxSubmissionNonce;

    fn priority(&self, _: &Self::Transaction) -> Self::PriorityValue {
        TxSubmissionNonce(self.assign())
    }
}

impl<T> Default for FiFo<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TxSubmissionNonce(u64);

impl From<u64> for TxSubmissionNonce {
    fn from(value: u64) -> Self {
        TxSubmissionNonce(value)
    }
}

impl Ord for TxSubmissionNonce {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse the ordering so lower values have higher priority
        self.0.cmp(&other.0)
    }
}

impl Eq for TxSubmissionNonce {}

impl PartialOrd for TxSubmissionNonce {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TxSubmissionNonce {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// Tip-based ordering implementation.
///
/// This ordering implementation uses the transaction's tip as the priority value. We don't have a
/// use case for this ordering implementation yet, but it's mostly used for testing.
#[derive(Debug)]
pub struct TipOrdering<T>(PhantomData<T>);

impl<T> TipOrdering<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Debug, Clone)]
pub struct Tip(u64);

impl Ord for Tip {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.cmp(&self.0)
    }
}

impl PartialOrd for Tip {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Tip {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Tip {}

impl<T: PoolTransaction> PoolOrd for TipOrdering<T> {
    type Transaction = T;
    type PriorityValue = Tip;

    fn priority(&self, tx: &Self::Transaction) -> Self::PriorityValue {
        Tip(tx.tip())
    }
}

impl<T> Default for TipOrdering<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {

    use std::collections::BTreeSet;

    use futures::StreamExt;
    use katana_pool_api::{PendingTx, PoolTransaction, TransactionPool, TxId};
    use katana_primitives::{address, ContractAddress, Felt};

    use crate::ordering::{self, FiFo, TxSubmissionNonce};
    use crate::pool::test_utils::PoolTx;
    use crate::validation::NoopValidator;
    use crate::Pool;

    fn create_pending_tx(
        sender: ContractAddress,
        nonce: u64,
        priority_value: u64,
    ) -> PendingTx<PoolTx, FiFo<PoolTx>> {
        let tx = PoolTx::new();
        let tx_id = TxId::new(sender, Felt::from(nonce));
        let priority = TxSubmissionNonce::from(priority_value);
        PendingTx::new(tx_id, tx, priority)
    }

    #[test]
    fn ordering_same_sender_is_by_nonce_only() {
        let sender = address!("0x1");

        let tx1 = create_pending_tx(sender, 2, 10); // nonce 2, prio 10
        let tx2 = create_pending_tx(sender, 0, 20); // nonce 0, prio 20
        let tx3 = create_pending_tx(sender, 1, 5); // nonce 1, prio 5

        let mut tx_set = BTreeSet::new();
        tx_set.insert(tx1.clone());
        tx_set.insert(tx2.clone());
        tx_set.insert(tx3.clone());

        let ordered_tx_ids: Vec<TxId> = tx_set.iter().map(|ptx| ptx.id.clone()).collect();

        // The order should be purely by nonce regardless of priority and insertion order.
        let expected_order = vec![tx2.id, tx3.id, tx1.id];

        assert_eq!(ordered_tx_ids, expected_order);
    }

    #[test]
    fn ordering_different_senders_is_by_priority_then_nonce_within_sender() {
        let sender_a = address!("0xA");
        let sender_b = address!("0xB");

        let tx_a2_p20 = create_pending_tx(sender_a, 2, 20); // Sender A, nonce 2, prio 20
        let tx_b1_p15 = create_pending_tx(sender_b, 1, 15); // Sender B, nonce 1, prio 15
        let tx_a1_p30 = create_pending_tx(sender_a, 1, 30); // Sender A, nonce 1, prio 30
        let tx_b0_p10_later = create_pending_tx(sender_b, 0, 10); // Sender B, nonce 0, prio 10
        let tx_b2_p40_later = create_pending_tx(sender_b, 2, 40); // Sender B, nonce 2, prio 40
        let tx_a0_p12_later = create_pending_tx(sender_a, 0, 12); // Sender A, nonce 0, prio 12

        let mut tx_set = BTreeSet::new();
        tx_set.insert(tx_a2_p20.clone());
        tx_set.insert(tx_b1_p15.clone());
        tx_set.insert(tx_a1_p30.clone());
        tx_set.insert(tx_b0_p10_later.clone());
        tx_set.insert(tx_b2_p40_later.clone());
        tx_set.insert(tx_a0_p12_later.clone());

        let ordered_tx_ids: Vec<TxId> = tx_set.iter().map(|ptx| ptx.id.clone()).collect();

        // If we only consider the order based on priority value, then the order would be:
        // [tx_b0_p10_later, tx_a0_p12_later, tx_b1_p15, tx_a2_p20, tx_a0_p30, tx_b2_p40_later]
        //
        // This is the expected order if tx with the same sender are ordered according to their
        // nonces:
        let expected_order = vec![
            tx_b0_p10_later.id,
            tx_a0_p12_later.id,
            tx_b1_p15.id,
            tx_a1_p30.id,
            tx_a2_p20.id,
            tx_b2_p40_later.id,
        ];

        assert_eq!(ordered_tx_ids, expected_order);
    }

    #[tokio::test]
    async fn fifo_ordering() {
        // Create mock transactions
        let txs = [PoolTx::new(), PoolTx::new(), PoolTx::new(), PoolTx::new(), PoolTx::new()];

        // Create a pool with FiFo ordering
        let pool = Pool::new(NoopValidator::new(), FiFo::new());

        // Add transactions to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        // Get pending transactions
        let mut pendings = pool.pending_transactions();

        // Assert that the transactions are in the order they were added (first to last)
        for tx in txs {
            let pending = pendings.next().await.unwrap();
            assert_eq!(pending.tx.as_ref(), &tx);
        }
    }

    #[tokio::test]
    async fn tip_based_ordering() {
        // Create mock transactions with different tips and in random order
        let txs = [
            PoolTx::new().with_tip(2),
            PoolTx::new().with_tip(1),
            PoolTx::new().with_tip(6),
            PoolTx::new().with_tip(3),
            PoolTx::new().with_tip(2),
            PoolTx::new().with_tip(2),
            PoolTx::new().with_tip(5),
            PoolTx::new().with_tip(4),
            PoolTx::new().with_tip(7),
        ];

        // Create a pool with tip-based ordering
        let pool = Pool::new(NoopValidator::new(), ordering::TipOrdering::new());

        // Add transactions to the pool
        for tx in &txs {
            let _ = pool.add_transaction(tx.clone()).await;
        }

        let mut pending = pool.pending_transactions();

        // Assert that the transactions are ordered by tip (highest to lowest)
        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 7);
        assert_eq!(tx.tx.hash(), txs[8].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 6);
        assert_eq!(tx.tx.hash(), txs[2].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 5);
        assert_eq!(tx.tx.hash(), txs[6].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 4);
        assert_eq!(tx.tx.hash(), txs[7].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 3);
        assert_eq!(tx.tx.hash(), txs[3].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 2);
        assert_eq!(tx.tx.hash(), txs[0].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 2);
        assert_eq!(tx.tx.hash(), txs[4].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 2);
        assert_eq!(tx.tx.hash(), txs[5].hash());

        let tx = pending.next().await.unwrap();
        assert_eq!(tx.tx.tip(), 1);
        assert_eq!(tx.tx.hash(), txs[1].hash());
    }
}
