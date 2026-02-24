use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use block_producer::BlockProductionError;
use katana_pool::TxPool;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use tracing::{error, info};

use self::block_producer::BlockProducer;
use self::metrics::BlockProducerMetrics;

pub mod block_producer;
mod metrics;

pub(crate) const LOG_TARGET: &str = "node";

/// The type that drives block production and chain progression.
#[must_use = "BlockProductionTask does nothing unless polled"]
#[allow(missing_debug_implementations)]
pub struct BlockProductionTask<PF>
where
    PF: ProviderFactory,
{
    /// Creates and seals blocks.
    pub(crate) block_producer: BlockProducer<PF>,
    /// Metrics for recording the service operations.
    metrics: BlockProducerMetrics,
}

impl<PF> BlockProductionTask<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub fn new(pool: TxPool, block_producer: BlockProducer<PF>) -> Self {
        block_producer.attach_pool(pool);
        Self { block_producer, metrics: BlockProducerMetrics::default() }
    }
}

impl<PF> Future for BlockProductionTask<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    type Output = Result<(), BlockProductionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        while let Poll::Ready(Some(res)) = this.block_producer.poll_next(cx) {
            match res {
                Ok(outcome) => {
                    info!(target: LOG_TARGET, block_number = %outcome.block_number, "Mined block.");

                    let gas_used = outcome.stats.l1_gas_used;
                    let steps_used = outcome.stats.cairo_steps_used;
                    this.metrics.l1_gas_processed_total.increment(gas_used as u64);
                    this.metrics.cairo_steps_processed_total.increment(steps_used as u64);
                }

                Err(error) => {
                    error!(target: LOG_TARGET, %error, "Mining block.");
                    return Poll::Ready(Err(error));
                }
            }
        }

        Poll::Pending
    }
}
