use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::Arc;

use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{BlockNumberOrTag, FeeHistory};
use anyhow::{Context, Ok};
use katana_primitives::block::GasPrice;
use katana_tasks::TaskSpawner;
use num_traits::ToPrimitive;
use parking_lot::Mutex;
use starknet::core::types::{BlockId, BlockTag, MaybePendingBlockWithTxHashes, ResourcePrice};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider as StarknetProvider};
use tokio::time::Duration;
use tracing::error;
use url::Url;

#[derive(Debug, Clone)]
pub struct GasOracle {
    prices: Arc<Mutex<SampledPrices>>,
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
        GasPrice::MIN
    }
}

#[derive(Debug, Clone)]
pub struct EthereumGasOracleWorker {
    pub prices: Arc<Mutex<SampledPrices>>,
    pub l1_provider_url: Url,
    pub gas_price_buffer: GasPriceBuffer,
    pub data_gas_price_buffer: GasPriceBuffer,
}

impl EthereumGasOracleWorker {
    pub fn new(prices: Arc<Mutex<SampledPrices>>, l1_provider_url: Url) -> Self {
        Self {
            prices,
            l1_provider_url,
            gas_price_buffer: GasPriceBuffer::new(),
            data_gas_price_buffer: GasPriceBuffer::new(),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let provider = ProviderBuilder::new().on_http(self.l1_provider_url.clone());
        // every 60 seconds, Starknet samples the base price of gas and data gas on L1
        let mut interval = tokio::time::interval(INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            // Wait for the interval to tick
            interval.tick().await;

            {
                // Attempt to get the gas price from L1
                let last_block_number = provider.get_block_number().await?;

                let fee_history = provider
                    .get_fee_history(1, BlockNumberOrTag::Number(last_block_number), &[])
                    .await?;

                let mut prices = self.prices.lock();

                if let Err(error) = update_gas_price(
                    &mut prices,
                    &mut self.gas_price_buffer,
                    &mut self.data_gas_price_buffer,
                    fee_history,
                ) {
                    error!(target: "gas_oracle", %error, "Error updating gas prices.");
                }
            }
        }
    }

    pub async fn update_once(&mut self) -> anyhow::Result<()> {
        let provider = ProviderBuilder::new().on_http(self.l1_provider_url.clone());

        // Attempt to get the gas price from L1
        let last_block_number = provider.get_block_number().await?;

        let fee_history =
            provider.get_fee_history(1, BlockNumberOrTag::Number(last_block_number), &[]).await?;

        let mut prices = self.prices.lock();

        update_gas_price(
            &mut prices,
            &mut self.gas_price_buffer,
            &mut self.data_gas_price_buffer,
            fee_history,
        )
    }
}
