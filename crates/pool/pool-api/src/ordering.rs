use crate::PoolTransaction;

// evaluates the priority of a transaction which would be used to determine how txs are ordered in
// the pool.
pub trait PoolOrd: Send + Sync {
    type Transaction: PoolTransaction;
    /// The priority value type whose [Ord] implementation is used to order the transaction in the
    /// pool.
    type PriorityValue: Ord + Clone + core::fmt::Debug;

    /// returns the priority value for the given transaction
    fn priority(&self, tx: &Self::Transaction) -> Self::PriorityValue;
}
