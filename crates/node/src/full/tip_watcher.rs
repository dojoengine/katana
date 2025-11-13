use std::future::IntoFuture;
use std::time::Duration;

use anyhow::Result;
use futures::future::BoxFuture;
use katana_gateway_types::BlockId;
use katana_primitives::block::BlockNumber;
use tokio::sync::watch;
use tracing::{error, info};

pub type TipWatcherFut = BoxFuture<'static, Result<()>>;

/// A trait for abstracting the source of the latest block number.
///
/// This allows the chain tip watcher to work with different sources such as:
/// - Starknet Core Contract on L1 (Ethereum) - tracks the settled/proven tip
/// - Starknet RPC endpoints - tracks the latest L2 tip
/// - Feeder Gateway - tracks the latest L2 tip from the sequencer
pub trait ChainTipProvider: Send + Sync {
    /// Retrieves the latest block number from the source.
    ///
    /// # Returns
    ///
    /// Returns the latest block number.
    fn latest_number(&self) -> BoxFuture<'_, Result<BlockNumber>>;
}

pub struct ChainTipWatcher<P> {
    /// The block number provider for fetching the latest block.
    tip_provider: P,
    /// Interval for checking the new tip.
    watch_interval: Duration,
    /// Watch channel for notifying subscribers of the latest tip.
    tip_sender: watch::Sender<BlockNumber>,
}

impl<P: ChainTipProvider> ChainTipWatcher<P> {
    pub fn new(provider: P) -> Self {
        let (tip_tx, _) = watch::channel(0);
        let watch_interval = Duration::from_secs(30);
        Self { tip_provider: provider, watch_interval, tip_sender: tip_tx }
    }

    /// Set the watch interval for checking new tips.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.watch_interval = interval;
        self
    }

    /// Subscribe to tip updates.
    ///
    /// Returns a subscription that always reflects the latest tip block number.
    pub fn subscribe(&self) -> TipSubscription {
        TipSubscription(self.tip_sender.subscribe())
    }

    pub async fn run(&self) -> Result<()> {
        let interval_in_secs = self.watch_interval.as_secs();
        info!(interval = %interval_in_secs, "Chain tip watcher started.");

        let mut prev_tip: BlockNumber = 0;

        loop {
            let block_number = self.tip_provider.latest_number().await?;

            if prev_tip != block_number {
                info!(block = %block_number, "New tip found.");
                prev_tip = block_number;
                self.broadcast_tip(block_number);
            }

            tokio::time::sleep(self.watch_interval).await;
        }
    }

    fn broadcast_tip(&self, block_number: BlockNumber) {
        let _ = self.tip_sender.send(block_number);
    }
}

impl<P: ChainTipProvider + 'static> IntoFuture for ChainTipWatcher<P> {
    type Output = Result<()>;
    type IntoFuture = TipWatcherFut;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.run().await.inspect_err(|error| {
                error!(target: "node", %error, "Tip watcher failed.");
            })
        })
    }
}

impl<P> std::fmt::Debug for ChainTipWatcher<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainTipWatcher")
            .field("provider", &"ChainTipProvider")
            .field("subscribers", &self.tip_sender.receiver_count())
            .field("watch_interval", &self.watch_interval)
            .finish()
    }
}

/// A subscription to chain tip updates.
#[derive(Clone)]
pub struct TipSubscription(watch::Receiver<BlockNumber>);

impl TipSubscription {
    /// Get the current tip block number.
    pub fn tip(&self) -> BlockNumber {
        *self.0.borrow()
    }

    /// Wait for the tip to change and return the new value.
    pub async fn changed(&mut self) -> Result<BlockNumber> {
        self.0.changed().await?;
        Ok(*self.0.borrow_and_update())
    }
}

impl std::fmt::Debug for TipSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TipSubscription").field("current_tip", &self.tip()).finish()
    }
}

/// Implementation of [`ChainTipProvider`] for the feeder gateway client.
///
/// This fetches the latest L2 block number directly from the Starknet feeder gateway,
/// which may be ahead of the L1 settlement.
impl ChainTipProvider for katana_gateway_client::Client {
    fn latest_number(&self) -> BoxFuture<'_, Result<BlockNumber>> {
        Box::pin(async move {
            let block = self.get_block(BlockId::Latest).await?;
            block.block_number.ok_or_else(|| anyhow::anyhow!("Block number not available"))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use super::*;

    /// Mock provider that returns a sequence of block numbers from an atomic counter.
    #[derive(Clone)]
    struct MockProvider {
        counter: Arc<AtomicU64>,
    }

    impl MockProvider {
        fn new(initial: BlockNumber) -> Self {
            Self { counter: Arc::new(AtomicU64::new(initial)) }
        }

        fn set(&self, value: BlockNumber) {
            self.counter.store(value, Ordering::SeqCst);
        }

        fn increment(&self) {
            self.counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ChainTipProvider for MockProvider {
        fn latest_number(&self) -> BoxFuture<'_, Result<BlockNumber>> {
            let value = self.counter.load(Ordering::SeqCst);
            Box::pin(async move { Ok(value) })
        }
    }

    #[tokio::test]
    async fn tip_updates_are_broadcast_to_subscribers() {
        let provider = MockProvider::new(100);
        let watcher = ChainTipWatcher::new(provider.clone()).interval(Duration::from_millis(10));

        let mut sub1 = watcher.subscribe();
        let sub2 = watcher.subscribe();

        // Initial value should be 0 (default)
        assert_eq!(sub1.tip(), 0);
        assert_eq!(sub2.tip(), 0);

        // Spawn the watcher task
        let handle = tokio::spawn(async move { watcher.run().await });

        // Wait for first update (block 100)
        let tip = sub1.changed().await.unwrap();
        assert_eq!(tip, 100);
        assert_eq!(sub2.tip(), 100);

        // Update provider and wait for new tip
        provider.set(150);
        tokio::time::sleep(Duration::from_millis(20)).await;

        let tip = sub1.changed().await.unwrap();
        assert_eq!(tip, 150);
        assert_eq!(sub2.tip(), 150);

        handle.abort();
    }

    #[tokio::test]
    async fn duplicate_tips_are_not_rebroadcast() {
        let provider = MockProvider::new(100);
        let watcher = ChainTipWatcher::new(provider.clone()).interval(Duration::from_millis(10));

        let mut sub = watcher.subscribe();

        let handle = tokio::spawn(async move { watcher.run().await });

        // Wait for first update
        let tip = sub.changed().await.unwrap();
        assert_eq!(tip, 100);

        // Keep the same tip for multiple intervals
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should timeout as no new update is expected
        let result = tokio::time::timeout(Duration::from_millis(30), sub.changed()).await;
        assert!(result.is_err(), "Should not receive duplicate tip updates");

        handle.abort();
    }

    #[tokio::test]
    async fn monotonically_increasing_tips() {
        let provider = MockProvider::new(1);
        let watcher = ChainTipWatcher::new(provider.clone()).interval(Duration::from_millis(10));

        let mut sub = watcher.subscribe();

        let handle = tokio::spawn(async move { watcher.run().await });

        // Verify multiple sequential updates
        for expected in 1..=5 {
            let tip = sub.changed().await.unwrap();
            assert_eq!(tip, expected);

            provider.increment(); // Increment the chain tip
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        handle.abort();
    }
}
