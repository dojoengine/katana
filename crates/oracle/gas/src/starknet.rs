use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::Arc;

use katana_primitives::block::GasPrice;
use num_traits::ToPrimitive;
use parking_lot::Mutex;
use starknet::core::types::{BlockId, BlockTag, MaybePendingBlockWithTxHashes, ResourcePrice};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use tokio::time::{Duration, Interval, MissedTickBehavior};
use tracing::error;
use url::Url;

#[derive(Debug, Clone)]
pub struct GasOracle {
    prices: Arc<Mutex<()>>,
    // prices: Arc<Mutex<SampledPrices>>,
    provider: Url,
}

impl GasOracle {
    pub fn new(provider: Url) -> Self {
        Self { prices: Default::default(), provider }
    }

    pub fn current_l1_data_gas_prices(&self) -> GasPrice {
        self.prices.lock().l1_data_gas_prices.clone()
    }

    pub fn current_l1_gas_prices(&self) -> GasPrice {
        self.prices.lock().l1_gas_prices.clone()
    }

    pub fn current_l2_gas_prices(&self) -> GasPrice {
        self.prices.lock().l2_gas_prices.clone()
    }

    pub fn worker(&self) -> GasOracleWorker {
        todo!()
    }

    pub async fn worker_builder(&self) -> GasOracleWorkerBuilder {
        GasOracleWorkerBuilder::default()
    }
}

#[derive(Debug, Clone)]
pub struct GasOracleWorkerBuilder {
    interval: Duration,
}

impl GasOracleWorkerBuilder {
    pub fn new() -> Self {
        Self { interval: Duration::from_secs(60) }
    }

    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    pub fn bulid<P: Provider>(self, provider: P) -> GasOracleWorker {
        todo!()
    }
}

impl Default for GasOracleWorkerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct GasOracleWorker<P> {
    provider: P,
    interval: Duration,
    prices: Arc<Mutex<SampledPrices>>,
    l2_gas_price_buffer: GasPriceBuffer,
    l1_gas_price_buffer: GasPriceBuffer,
    l1_data_gas_price_buffer: GasPriceBuffer,
}

impl<P: Provider> GasOracleWorker<P> {
    pub async fn run(&self) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let block_id = BlockId::Tag(BlockTag::Latest);
            let block = self.provider.get_block_with_tx_hashes(block_id).await?;

            let mut prices = self.prices.lock();

            if let Err(error) = update_starknet_gas_price(
                &mut prices,
                &mut self.l2_gas_price_buffer,
                &mut self.l1_gas_price_buffer,
                &mut self.l1_data_gas_price_buffer,
                block,
            ) {
                error!(target: "gas_oracle", %error, "Error updating Starknet gas prices.");
            }

            interval.tick().await;
        }
    }

    pub async fn update_once(&mut self) -> anyhow::Result<()> {
        let client = JsonRpcClient::new(HttpTransport::new(Url::parse(
            self.starknet_provider_url.as_ref(),
        )?));

        // Get the latest block from Starknet
        let block = client.get_block_with_tx_hashes(BlockId::Tag(BlockTag::Latest)).await?;

        let mut prices = self.prices.lock();

        update_starknet_gas_price(
            &mut prices,
            &mut self.gas_price_buffer,
            &mut self.data_gas_price_buffer,
            block,
        )
    }
}

pub fn update_starknet_gas_price(
    prices: &mut SampledPrices,
    l2_gas_price_buffer: &mut GasPriceBuffer,
    l1_gas_price_buffer: &mut GasPriceBuffer,
    l1_data_gas_price_buffer: &mut GasPriceBuffer,
    block: MaybePendingBlockWithTxHashes,
) -> anyhow::Result<()> {
    let (l1_gas_price, _l2_gas_price, l1_data_gas_price) = match block {
        MaybePendingBlockWithTxHashes::Block(block) => {
            (block.l1_gas_price, block.l2_gas_price, block.l1_data_gas_price)
        }
        MaybePendingBlockWithTxHashes::PendingBlock(pending) => {
            (pending.l1_gas_price, pending.l2_gas_price, pending.l1_data_gas_price)
        }
    };

    let l2_gas_price_wei = l1_gas_price.price_in_wei.to_u128().unwrap();
    l2_gas_price_buffer.add_sample(l2_gas_price_wei);
    let l2_gas_price_fri = l1_gas_price.price_in_fri.to_u128().unwrap();
    l2_gas_price_buffer.add_sample(l2_gas_price_fri);

    let l1_gas_price_wei = l1_gas_price.price_in_wei.to_u128().unwrap();
    l1_gas_price_buffer.add_sample(l1_gas_price_wei);
    let l1_gas_price_fri = l1_gas_price.price_in_fri.to_u128().unwrap();
    l1_gas_price_buffer.add_sample(l1_gas_price_fri);

    let l1_data_gas_price_wei = l1_data_gas_price.price_in_wei.to_u128().unwrap();
    l1_data_gas_price_buffer.add_sample(l1_data_gas_price_wei);
    let l1_data_gas_price_fri = l1_data_gas_price.price_in_fri.to_u128().unwrap();
    l1_data_gas_price_buffer.add_sample(l1_data_gas_price_fri);

    // Calculate average gas prices
    let avg_l1_gas_price = unsafe {
        GasPrice::new_unchecked(l1_gas_price_buffer.average(), l1_gas_price_buffer.average())
    };

    // let avg_l1_data_gas_price = unsafe {
    //     GasPrice::new_unchecked(
    //         l1_data_gas_price_buffer.average(),
    //         l1_data_gas_price_buffer.average(),
    //     )
    // };

    // prices.l2_gas_prices = avg_l1_gas_price;
    // prices.l1_gas_prices = avg_l1_gas_price;
    // prices.l1_data_gas_prices = avg_l1_data_gas_price;
    // Ok(())

    todo!()
}
