use std::future::IntoFuture;
use std::time::Duration;

use alloy_provider::Provider;
use anyhow::Result;
use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_starknet::StarknetCore;
use tokio::sync::watch;
use tracing::{error, info};

pub type TipWatcherFut = BoxFuture<'static, Result<()>>;

pub struct ChainTipWatcher<P> {
    /// The Starknet Core Contract client for fetching the latest block.
    core_contract: StarknetCore<P>,
    /// Interval for checking the new tip.
    watch_interval: Duration,
    /// Watch channel for notifying subscribers of the latest tip.
    tip_sender: watch::Sender<BlockNumber>,
}

impl<P: alloy_provider::Provider> ChainTipWatcher<P> {
    pub fn new(core_contract: StarknetCore<P>) -> Self {
        let (tip_tx, _) = watch::channel(0);
        let watch_interval = Duration::from_secs(30);
        Self { core_contract, watch_interval, tip_sender: tip_tx }
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
            let block_number = self.core_contract.state_block_number().await? as BlockNumber;

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

impl<P: Provider + 'static> IntoFuture for ChainTipWatcher<P> {
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

impl<P: std::fmt::Debug> std::fmt::Debug for ChainTipWatcher<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainTipWatcher")
            .field("core_contract", &self.core_contract)
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
