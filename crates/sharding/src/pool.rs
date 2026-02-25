use std::fmt;
use std::future::Future;
use std::sync::Arc;

use futures::channel::mpsc::Receiver;
use katana_pool::{PendingTransactions, TransactionPool, TxPool};
use katana_pool_api::PoolResult;
use katana_primitives::contract::Nonce;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;

use crate::scheduler::Scheduler;
use crate::shard::ShardId;

/// Transaction pool wrapper that triggers shard scheduling when a tx is accepted.
#[derive(Clone)]
pub struct ShardPool {
    inner: TxPool,
    scheduler: Scheduler,
    shard_id: ShardId,
}

impl ShardPool {
    pub fn new(inner: TxPool, scheduler: Scheduler, shard_id: ShardId) -> Self {
        Self { inner, scheduler, shard_id }
    }
}

impl TransactionPool for ShardPool {
    type Transaction = <TxPool as TransactionPool>::Transaction;
    type Ordering = <TxPool as TransactionPool>::Ordering;
    type Validator = <TxPool as TransactionPool>::Validator;

    fn add_transaction(
        &self,
        tx: Self::Transaction,
    ) -> impl Future<Output = PoolResult<TxHash>> + Send {
        let inner = self.inner.clone();
        let scheduler = self.scheduler.clone();
        let shard_id = self.shard_id;

        async move {
            let result = inner.add_transaction(tx).await;

            if result.is_ok() {
                scheduler.schedule(shard_id);
            }

            result
        }
    }

    fn pending_transactions(&self) -> PendingTransactions<Self::Transaction, Self::Ordering> {
        self.inner.pending_transactions()
    }

    fn contains(&self, hash: TxHash) -> bool {
        self.inner.contains(hash)
    }

    fn get(&self, hash: TxHash) -> Option<Arc<Self::Transaction>> {
        self.inner.get(hash)
    }

    fn add_listener(&self) -> Receiver<TxHash> {
        self.inner.add_listener()
    }

    fn remove_transactions(&self, hashes: &[TxHash]) {
        self.inner.remove_transactions(hashes);
    }

    fn size(&self) -> usize {
        self.inner.size()
    }

    fn validator(&self) -> &Self::Validator {
        self.inner.validator()
    }

    fn get_nonce(&self, address: ContractAddress) -> Option<Nonce> {
        self.inner.get_nonce(address)
    }
}

impl fmt::Debug for ShardPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardPool")
            .field("size", &self.inner.size())
            .field("shard_id", &self.shard_id)
            .finish()
    }
}
