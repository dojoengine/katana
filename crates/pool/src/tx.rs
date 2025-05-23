use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Instant;

use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::transaction::{
    DeclareTx, DeployAccountTx, ExecutableTx, ExecutableTxWithHash, InvokeTx, TxHash,
};

use crate::ordering::PoolOrd;

// the transaction type is recommended to implement a cheap clone (eg ref-counting) so that it
// can be cloned around to different pools as necessary.
pub trait PoolTransaction: Clone {
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

/// the tx id in the pool. identified by its sender and nonce.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxId {
    sender: ContractAddress,
    nonce: Nonce,
}

impl TxId {
    pub fn new(sender: ContractAddress, nonce: Nonce) -> Self {
        Self { sender, nonce }
    }

    pub fn parent(&self) -> Option<Self> {
        if self.nonce == Nonce::ZERO {
            None
        } else {
            Some(Self { sender: self.sender, nonce: self.nonce - 1 })
        }
    }

    pub fn descendent(&self) -> Self {
        Self { sender: self.sender, nonce: self.nonce + 1 }
    }
}

#[derive(Debug)]
pub struct PendingTx<T, O: PoolOrd> {
    pub id: TxId,
    pub tx: Arc<T>,
    pub priority: O::PriorityValue,
    pub added_at: std::time::Instant,
}

impl<T, O: PoolOrd> PendingTx<T, O> {
    pub fn new(id: TxId, tx: T, priority: O::PriorityValue) -> Self {
        Self { id, tx: Arc::new(tx), priority, added_at: Instant::now() }
    }
}

// We can't just derive these traits because the derive implementation would require that
// the generics also implement these traits, which is not necessary.

impl<T, O: PoolOrd> Clone for PendingTx<T, O> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            added_at: self.added_at,
            tx: Arc::clone(&self.tx),
            priority: self.priority.clone(),
        }
    }
}

impl<T, O: PoolOrd> PartialEq for PendingTx<T, O> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<T, O: PoolOrd> Eq for PendingTx<T, O> {}

impl<T, O: PoolOrd> PartialOrd for PendingTx<T, O> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// When two transactions have the same priority, we want to prioritize the one that was added
// first. So, when an incoming transaction with similar priority value is added to the
// [BTreeSet](std::collections::BTreeSet), the transaction is assigned a 'greater'
// [Ordering](std::cmp::Ordering) so that it will be placed after the existing ones. This is
// because items in a BTree is ordered from lowest to highest.
impl<T, O: PoolOrd> Ord for PendingTx<T, O> {
    fn cmp(&self, other: &Self) -> Ordering {
        // If the txs are coming from the same account, then we completely ignore their assigned
        // priority value and the ordering will be relative to the sender's nonce.
        if self.id.sender == other.id.sender {
            return match self.id.nonce.cmp(&other.id.nonce) {
                Ordering::Equal => Ordering::Greater,
                other => other,
            };
        }

        match self.priority.cmp(&other.priority) {
            Ordering::Equal => Ordering::Greater,
            other => other,
        }
    }
}

impl PoolTransaction for ExecutableTxWithHash {
    fn hash(&self) -> TxHash {
        self.hash
    }

    fn nonce(&self) -> Nonce {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(..) => unimplemented!("v0 transaction not supported"),
                InvokeTx::V1(v1) => v1.nonce,
                InvokeTx::V3(v3) => v3.nonce,
            },
            ExecutableTx::L1Handler(tx) => tx.nonce,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(..) => unimplemented!("v0 transaction not supported"),
                DeclareTx::V1(v1) => v1.nonce,
                DeclareTx::V2(v2) => v2.nonce,
                DeclareTx::V3(v3) => v3.nonce,
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(v1) => v1.nonce,
                DeployAccountTx::V3(v3) => v3.nonce,
            },
        }
    }

    fn sender(&self) -> ContractAddress {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(v0) => v0.contract_address,
                InvokeTx::V1(v1) => v1.sender_address,
                InvokeTx::V3(v3) => v3.sender_address,
            },
            ExecutableTx::L1Handler(tx) => tx.contract_address,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(v0) => v0.sender_address,
                DeclareTx::V1(v1) => v1.sender_address,
                DeclareTx::V2(v2) => v2.sender_address,
                DeclareTx::V3(v3) => v3.sender_address,
            },
            ExecutableTx::DeployAccount(tx) => tx.contract_address(),
        }
    }

    fn max_fee(&self) -> u128 {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(v0) => v0.max_fee,
                InvokeTx::V1(v1) => v1.max_fee,
                InvokeTx::V3(_) => 0, // V3 doesn't have max_fee
            },
            ExecutableTx::L1Handler(tx) => tx.paid_fee_on_l1,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(v0) => v0.max_fee,
                DeclareTx::V1(v1) => v1.max_fee,
                DeclareTx::V2(v2) => v2.max_fee,
                DeclareTx::V3(_) => 0, // V3 doesn't have max_fee
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(v1) => v1.max_fee,
                DeployAccountTx::V3(_) => 0, // V3 doesn't have max_fee
            },
        }
    }

    fn tip(&self) -> u64 {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V3(v3) => v3.tip,
                _ => 0,
            },
            ExecutableTx::L1Handler(_) => 0,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V3(v3) => v3.tip,
                _ => 0,
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V3(v3) => v3.tip,
                _ => 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use katana_primitives::{address, ContractAddress, Felt};

    use super::{PendingTx, TxId};
    use crate::ordering::{FiFo, TxSubmissionNonce};
    use crate::pool::test_utils::PoolTx;

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
        let tx_b2_p40_later = create_pending_tx(sender_b, 2, 40); // Sender B, nonce 0, prio 40
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
}
