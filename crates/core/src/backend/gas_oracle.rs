use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::Arc;

use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{BlockNumberOrTag, FeeHistory};
use anyhow::{Context, Ok};
use katana_primitives::block::GasPrice;
use katana_tasks::TaskSpawner;
use parking_lot::Mutex;
use tokio::time::Duration;
use tracing::error;
use url::Url;

const BUFFER_SIZE: usize = 60;
const INTERVAL: Duration = Duration::from_secs(60);
const ONE_GWEI: u128 = 1_000_000_000;

#[derive(Debug)]
pub enum GasOracle {
    Fixed(FixedGasOracle),
    Sampled(EthereumSampledGasOracle),
}

#[derive(Debug)]
pub struct FixedGasOracle {
    gas_prices: GasPrice,
    data_gas_prices: GasPrice,
}

#[derive(Debug, Clone)]
pub struct EthereumSampledGasOracle {
    prices: Arc<Mutex<SampledPrices>>,
    provider: Url,
}

#[derive(Debug, Default)]
pub struct SampledPrices {
    gas_prices: GasPrice,
    data_gas_prices: GasPrice,
}

#[derive(Debug, Clone)]
pub struct GasOracleWorker {
    pub prices: Arc<Mutex<SampledPrices>>,
    pub l1_provider_url: Url,
    pub gas_price_buffer: GasPriceBuffer,
    pub data_gas_price_buffer: GasPriceBuffer,
}

impl GasOracle {
    pub fn fixed(gas_prices: GasPrice, data_gas_prices: GasPrice) -> Self {
        GasOracle::Fixed(FixedGasOracle { gas_prices, data_gas_prices })
    }

    /// Creates a new gas oracle that samples the gas prices from an Ethereum chain.
    pub fn sampled_ethereum(eth_provider: Url) -> Self {
        let prices: Arc<Mutex<SampledPrices>> = Arc::new(Mutex::new(SampledPrices::default()));
        GasOracle::Sampled(EthereumSampledGasOracle { prices, provider: eth_provider })
    }

    /// This is just placeholder for now, as Starknet doesn't provide a way to get the L2 gas
    /// prices, we just return a fixed gas price values of 0. This is equivalent to calling
    /// [`GasOracle::fixed`] with 0 values for both gas and data prices.
    ///
    /// The result of this is the same as running the node with fee disabled.
    pub fn sampled_starknet() -> Self {
        Self::fixed(GasPrice::MIN, GasPrice::MIN)
    }

    /// Returns the current gas prices.
    pub fn current_gas_prices(&self) -> GasPrice {
        match self {
            GasOracle::Fixed(fixed) => fixed.current_gas_prices(),
            GasOracle::Sampled(sampled) => sampled.prices.lock().gas_prices.clone(),
        }
    }

    /// Returns the current data gas prices.
    pub fn current_data_gas_prices(&self) -> GasPrice {
        match self {
            GasOracle::Fixed(fixed) => fixed.current_data_gas_prices(),
            GasOracle::Sampled(sampled) => sampled.prices.lock().data_gas_prices.clone(),
        }
    }

    pub fn run_worker(&self, task_spawner: TaskSpawner) {
        match self {
            Self::Fixed(..) => {}
            Self::Sampled(oracle) => {
                let prices = oracle.prices.clone();
                let l1_provider = oracle.provider.clone();

                task_spawner.build_task().critical().name("L1 Gas Oracle Worker").spawn(
                    async move {
                        let mut worker = GasOracleWorker::new(prices, l1_provider);
                        worker
                            .run()
                            .await
                            .inspect_err(|error| error!(target: "gas_oracle", %error, "Gas oracle worker failed."))
                    },
                );
            }
        }
    }
}

impl EthereumSampledGasOracle {
    pub fn current_data_gas_prices(&self) -> GasPrice {
        self.prices.lock().data_gas_prices.clone()
    }

    pub fn current_gas_prices(&self) -> GasPrice {
        self.prices.lock().gas_prices.clone()
    }
}

impl FixedGasOracle {
    pub fn current_data_gas_prices(&self) -> GasPrice {
        self.data_gas_prices.clone()
    }

    pub fn current_gas_prices(&self) -> GasPrice {
        self.gas_prices.clone()
    }
}

pub fn update_gas_price(
    l1_oracle: &mut SampledPrices,
    gas_price_buffer: &mut GasPriceBuffer,
    data_gas_price_buffer: &mut GasPriceBuffer,
    fee_history: FeeHistory,
) -> anyhow::Result<()> {
    let latest_gas_price = fee_history.base_fee_per_gas.last().context("Getting eth gas price")?;

    gas_price_buffer.add_sample(*latest_gas_price);

    let blob_fee_history = fee_history.base_fee_per_blob_gas;
    let avg_blob_base_fee = blob_fee_history.iter().last().context("Getting blob gas price")?;

    data_gas_price_buffer.add_sample(*avg_blob_base_fee);
    // The price of gas on Starknet is set to the average of the last 60 gas price samples, plus 1
    // gwei.
    let avg_gas_price = unsafe {
        GasPrice::new_unchecked(
            gas_price_buffer.average() + ONE_GWEI,
            gas_price_buffer.average() + ONE_GWEI,
        )
    };
    // The price of data gas on Starknet is set to the average of the last 60 data gas price
    // samples.
    let avg_blob_price = unsafe {
        GasPrice::new_unchecked(data_gas_price_buffer.average(), data_gas_price_buffer.average())
    };

    l1_oracle.gas_prices = avg_gas_price;
    l1_oracle.data_gas_prices = avg_blob_price;
    Ok(())
}

impl GasOracleWorker {
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

// Buffer to store the last 60 gas price samples
#[derive(Debug, Clone)]
pub struct GasPriceBuffer {
    buffer: VecDeque<u128>,
}

impl GasPriceBuffer {
    fn new() -> Self {
        Self { buffer: VecDeque::with_capacity(BUFFER_SIZE) } // 60 size
    }

    fn add_sample(&mut self, sample: u128) {
        if self.buffer.len() == BUFFER_SIZE {
            // remove oldest sample if buffer is full
            self.buffer.pop_front();
        }
        self.buffer.push_back(sample);
    }

    fn average(&self) -> u128 {
        if self.buffer.is_empty() {
            return 0;
        }
        let sum: u128 = self.buffer.iter().sum();
        sum / self.buffer.len() as u128
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    // Test the buffer functionality separately
    #[test]
    fn test_buffer_size_limit() {
        let mut buffer = GasPriceBuffer::new();

        // Add more samples than the buffer size
        for i in 0..BUFFER_SIZE + 10 {
            buffer.add_sample(i as u128);
        }

        // Check if buffer size is maintained
        assert_eq!(buffer.buffer.len(), BUFFER_SIZE);

        // Check if oldest values were removed (should start from 10)
        assert_eq!(*buffer.buffer.front().unwrap(), 10);
    }

    #[test]
    fn test_empty_buffer_average() {
        let buffer = GasPriceBuffer::new();
        assert_eq!(buffer.average(), 0);
    }

    #[test]
    fn test_buffer_single_sample_average() {
        let mut buffer = GasPriceBuffer::new();
        buffer.add_sample(100);
        assert_eq!(buffer.average(), 100);
    }

    #[test]
    fn test_bufffer_multiple_samples_average() {
        let mut buffer = GasPriceBuffer::new();
        // Add some test values
        let test_values = [100, 200, 300];
        for value in test_values.iter() {
            buffer.add_sample(*value);
        }

        let expected_avg = 200; // (100 + 200 + 300) / 3
        assert_eq!(buffer.average(), expected_avg);
    }

    #[tokio::test]
    #[ignore = "Requires external assumption"]
    async fn test_gas_oracle() {
        let url = Url::parse("https://eth.merkle.io/").expect("Invalid URL");
        let oracle = GasOracle::sampled_ethereum(url.clone());

        let shared_prices = match &oracle {
            GasOracle::Sampled(sampled) => sampled.prices.clone(),
            _ => panic!("Expected sampled oracle"),
        };

        let mut worker = GasOracleWorker::new(shared_prices.clone(), url);

        for i in 0..3 {
            let initial_gas_prices = oracle.current_gas_prices();

            // Verify initial state for first iteration
            if i == 0 {
                assert_eq!(
                    initial_gas_prices,
                    GasPrice::MIN,
                    "First iteration should start with zero prices"
                );
            }

            worker.update_once().await.expect("Failed to update prices");

            let updated_gas_prices = oracle.current_gas_prices();
            let updated_data_gas_prices = oracle.current_data_gas_prices();

            // Verify gas prices
            assert!(updated_gas_prices.eth.get() > 1, "ETH gas price should be non-zero");

            assert!(updated_data_gas_prices.eth.get() > 1, "ETH data gas price should be non-zero");

            // For iterations after the first, verify that prices have been updated
            if i > 0 {
                // Give some flexibility for price changes
                if initial_gas_prices.eth.get() != 1 {
                    assert!(
                        initial_gas_prices.eth != updated_gas_prices.eth
                            || initial_gas_prices.strk != updated_gas_prices.strk,
                        "Prices should potentially change between updates"
                    );
                }
            }

            // ETH current avg blocktime is ~12 secs so we need a delay to wait for block creation
            tokio::time::sleep(std::time::Duration::from_secs(9)).await;
        }
    }
}
