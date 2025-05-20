use std::sync::Arc;

use futures::channel::mpsc::Receiver;
use katana_pool::{pending::PendingTransactions, PoolResult, TransactionPool};
use katana_primitives::transaction::TxHash;
use katana_tasks::TaskSpawner;
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};

pub struct TxPool {
    inner: katana_pool::TxPool,
    task_spawner: TaskSpawner,
    client: Arc<JsonRpcClient<HttpTransport>>,
}

// NOTE: The actual methods and associated types required by the TransactionPool trait
// need to be filled in based on the trait definition. This implementation
// delegates all required trait methods to the inner katana_pool::TxPool instance.
impl TransactionPool for TxPool {
    type Ordering = <katana_pool::TxPool as TransactionPool>::Ordering;
    type Validator = <katana_pool::TxPool as TransactionPool>::Validator;
    type Transaction = <katana_pool::TxPool as TransactionPool>::Transaction;

    fn add_transaction(&self, tx: Self::Transaction) -> PoolResult<TxHash> {
        self.inner.add_transaction(tx)
        // self.client.add_invoke_transaction(tx)
    }

    fn contains(&self, hash: TxHash) -> bool {
        self.inner.contains(hash)
    }

    fn get(&self, hash: TxHash) -> Option<Arc<Self::Transaction>> {
        self.inner.get(hash)
    }

    fn pending_transactions(&self) -> PendingTransactions<Self::Transaction, Self::Ordering> {
        self.inner.pending_transactions()
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

    fn add_listener(&self) -> Receiver<TxHash> {
        self.inner.add_listener()
    }
}
