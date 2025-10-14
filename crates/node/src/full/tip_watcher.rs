use std::future::IntoFuture;
use std::time::Duration;

use alloy_provider::Provider;
use anyhow::Result;
use futures::future::BoxFuture;
use katana_pipeline::PipelineHandle;
use katana_starknet::StarknetCore;
use tracing::{error, info, trace};

type TipWatcherFut = BoxFuture<'static, Result<()>>;

#[derive(Debug)]
pub struct ChainTipWatcher<P> {
    /// The Starknet Core Contract client for fetching the latest block.
    core_contract: StarknetCore<P>,
    /// The pipeline handle for setting the tip.
    pipeline_handle: PipelineHandle,
    /// Interval for checking the new tip.
    watch_interval: Duration,
}

impl<P: alloy_provider::Provider> ChainTipWatcher<P> {
    pub fn new(core_contract: StarknetCore<P>, pipeline_handle: PipelineHandle) -> Self {
        let watch_interval = Duration::from_secs(30);
        Self { core_contract, pipeline_handle, watch_interval }
    }

    pub async fn run(&self) -> Result<()> {
        let interval_in_secs = self.watch_interval.as_secs();
        info!(interval = %interval_in_secs, "Chain tip watcher started.");

        let mut prev_tip = 0;

        loop {
            let block_number = self.core_contract.state_block_number().await?;

            if prev_tip != block_number {
                info!(block = %block_number, "New tip found.");
                self.pipeline_handle.set_tip(block_number as u64);
                prev_tip = block_number;
            }

            tokio::time::sleep(self.watch_interval).await;
        }
    }
}

impl<P: Provider + 'static> IntoFuture for ChainTipWatcher<P> {
    type Output = Result<()>;
    type IntoFuture = TipWatcherFut;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.run().await.inspect_err(|error| {
                error!(target: "pipeline", %error, "Tip watcher failed.");
            })
        })
    }
}
