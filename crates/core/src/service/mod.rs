use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use block_producer::BlockProductionError;
use futures::stream::StreamExt;
use katana_pool::api::{PendingTransactions, PoolOrd, TransactionPool};
use katana_pool::TxPool;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use tracing::{error, info};

use self::block_producer::BlockProducer;
use self::metrics::BlockProducerMetrics;

pub mod block_producer;
mod metrics;

pub(crate) const LOG_TARGET: &str = "node";

/// The type that drives the blockchain's state
///
/// This task is basically an endless future that continuously polls the miner which returns
/// transactions for the next block, then those transactions are handed off to the [BlockProducer]
/// to construct a new block.
#[must_use = "BlockProductionTask does nothing unless polled"]
#[allow(missing_debug_implementations)]
pub struct BlockProductionTask<O, PF>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
    PF: ProviderFactory,
{
    /// creates new blocks
    pub(crate) block_producer: BlockProducer<PF>,
    /// the miner responsible to select transactions from the `pool´
    pub(crate) miner: TransactionMiner<O>,
    /// the pool that holds all transactions
    pub(crate) pool: TxPool,
    /// Metrics for recording the service operations
    metrics: BlockProducerMetrics,
}

impl<O, PF> BlockProductionTask<O, PF>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
    PF: ProviderFactory,
{
    pub fn new(
        pool: TxPool,
        miner: TransactionMiner<O>,
        block_producer: BlockProducer<PF>,
    ) -> Self {
        Self { block_producer, miner, pool, metrics: BlockProducerMetrics::default() }
    }
}

impl<O, PF> Future for BlockProductionTask<O, PF>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    type Output = Result<(), BlockProductionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // this drives block production and feeds new sets of ready transactions to the block
        // producer
        loop {
            while let Poll::Ready(Some(res)) = this.block_producer.poll_next(cx) {
                match res {
                    Ok(outcome) => {
                        info!(target: LOG_TARGET, block_number = %outcome.block_number, "Mined block.");

                        let gas_used = outcome.stats.l1_gas_used;
                        let steps_used = outcome.stats.cairo_steps_used;
                        this.metrics.l1_gas_processed_total.increment(gas_used as u64);
                        this.metrics.cairo_steps_processed_total.increment(steps_used as u64);

                        // remove mined transactions from the pool
                        this.pool.remove_transactions(&outcome.txs);
                    }

                    Err(error) => {
                        error!(target: LOG_TARGET, %error, "Mining block.");
                        return Poll::Ready(Err(error));
                    }
                }
            }

            if let Poll::Ready(pool_txs) = this.miner.poll(cx) {
                // miner returned a set of transaction that we feed to the producer
                this.block_producer.queue(pool_txs);
            } else {
                // no progress made
                break;
            }
        }

        Poll::Pending
    }
}

/// The type which takes the transaction from the pool and feeds them to the block producer.
#[derive(Debug)]
pub struct TransactionMiner<O>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
{
    pending_txs: PendingTransactions<ExecutableTxWithHash, O>,
}

impl<O> TransactionMiner<O>
where
    O: PoolOrd<Transaction = ExecutableTxWithHash>,
{
    pub fn new(pending_txs: PendingTransactions<ExecutableTxWithHash, O>) -> Self {
        Self { pending_txs }
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Vec<ExecutableTxWithHash>> {
        let mut transactions = Vec::new();

        while let Poll::Ready(Some(tx)) = self.pending_txs.poll_next_unpin(cx) {
            transactions.push(tx.tx.as_ref().clone());
        }

        if transactions.is_empty() {
            return Poll::Pending;
        }

        Poll::Ready(transactions)
    }
}
