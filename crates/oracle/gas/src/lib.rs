use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::Arc;

use ::starknet::core::types::{BlockId, BlockTag, MaybePendingBlockWithTxHashes, ResourcePrice};
use ::starknet::providers::jsonrpc::HttpTransport;
use ::starknet::providers::{JsonRpcClient, Provider as StarknetProvider};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::{BlockNumberOrTag, FeeHistory};
use anyhow::{Context, Ok};
use katana_primitives::block::GasPrice;
use katana_tasks::TaskSpawner;
use num_traits::ToPrimitive;
use parking_lot::Mutex;
use tokio::time::Duration;
use tracing::error;
use url::Url;

pub mod ethereum;
pub mod fixed;
pub mod starknet;

const BUFFER_SIZE: usize = 60;
const INTERVAL: Duration = Duration::from_secs(60);
const ONE_GWEI: u128 = 1_000_000_000;

#[derive(Debug)]
pub enum GasOracle {
    Fixed(FixedGasOracle),
    Sampled(SampledGasOracle),
}

/// Gas oracle that samples prices from different blockchain networks.
#[derive(Debug)]
pub enum SampledGasOracle {
    /// Samples gas prices from an Ethereum-based network.
    Ethereum(ethereum::GasOracle),
    /// Samples gas prices from a Starknet-based network.
    Starknet(starknet::GasOracle),
}

#[derive(Debug, Default)]
pub struct SampledPrices {
    l2_gas_prices: GasPrice,
    l1_gas_prices: GasPrice,
    l1_data_gas_prices: GasPrice,
}

impl GasOracle {
    /// Creates a gas oracle that uses fixed gas prices.
    pub fn fixed(
        l2_gas_prices: GasPrice,
        l1_gas_prices: GasPrice,
        l1_data_gas_prices: GasPrice,
    ) -> Self {
        GasOracle::Fixed(FixedGasOracle { l2_gas_prices, l1_gas_prices, l1_data_gas_prices })
    }

    /// Creates a new gas oracle that samples the gas prices from an Ethereum chain.
    pub fn sampled_ethereum(eth_provider: Url) -> Self {
        GasOracle::Sampled(SampledGasOracle::Ethereum(ethereum::GasOracle::new(eth_provider)))
    }

    /// Creates a new gas oracle that samples the gas prices from a Starknet chain.
    pub fn sampled_starknet(starknet_provider: Url) -> Self {
        GasOracle::Sampled(SampledGasOracle::Starknet(starknet::GasOracle::new(starknet_provider)))
    }

    /// Returns the current L1 gas prices.
    pub fn current_l1_gas_prices(&self) -> GasPrice {
        match self {
            GasOracle::Fixed(fixed) => fixed.current_l1_gas_prices(),
            GasOracle::Sampled(sampled) => sampled.current_l1_gas_prices(),
        }
    }

    /// Returns the current data gas prices.
    pub fn current_l1_data_gas_prices(&self) -> GasPrice {
        match self {
            GasOracle::Fixed(fixed) => fixed.current_l1_data_gas_prices(),
            GasOracle::Sampled(sampled) => sampled.current_l1_data_gas_prices(),
        }
    }

    /// Returns the current L2 gas prices.
    pub fn current_l2_gas_prices(&self) -> GasPrice {
        match self {
            GasOracle::Fixed(fixed) => fixed.current_l1_gas_prices(),
            GasOracle::Sampled(sampled) => sampled.current_l2_gas_prices(),
        }
    }

    pub fn run_worker(&self, task_spawner: TaskSpawner) {
        match self {
            Self::Fixed(..) => {}
            Self::Sampled(sampled) => match sampled {
                SampledGasOracle::Ethereum(oracle) => {
                    let prices = oracle.prices.clone();
                    let provider = oracle.provider.clone();

                    task_spawner.build_task().critical().name("Gas Oracle Worker").spawn(
                        async move {
                            EthereumGasOracleWorker::new(prices, provider)
                                .run()
                                .await
                                .inspect_err(|error| error!(target: "gas_oracle", %error, "Gas oracle worker failed."))
                        },
                    );
                }
                SampledGasOracle::Starknet(oracle) => {
                    let prices = oracle.prices.clone();
                    let provider = oracle.provider.clone();

                    task_spawner.build_task().critical().name("Gas Oracle Worker").spawn(
                        async move {
                            StarknetGasOracleWorker::new(prices, provider)
                                .run()
                                .await
                                .inspect_err(|error| )
                        },
                    );
                }
            },
        }
    }
}

impl SampledGasOracle {
    /// Returns the current l2 gas prices.
    pub fn current_l2_gas_prices(&self) -> GasPrice {
        match self {
            SampledGasOracle::Ethereum(ethereum) => ethereum.current_l2_gas_prices(),
            SampledGasOracle::Starknet(starknet) => starknet.current_l2_gas_prices(),
        }
    }

    /// Returns the current l1 gas prices.
    pub fn current_l1_gas_prices(&self) -> GasPrice {
        match self {
            SampledGasOracle::Ethereum(ethereum) => ethereum.current_l1_gas_prices(),
            SampledGasOracle::Starknet(starknet) => starknet.current_l1_gas_prices(),
        }
    }

    /// Returns the current l1 data gas prices.
    pub fn current_l1_data_gas_prices(&self) -> GasPrice {
        match self {
            SampledGasOracle::Ethereum(ethereum) => ethereum.current_l1_data_gas_prices(),
            SampledGasOracle::Starknet(starknet) => starknet.current_l1_data_gas_prices(),
        }
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

    l1_oracle.l1_gas_prices = avg_gas_price;
    l1_oracle.l1_data_gas_prices = avg_blob_price;
    Ok(())
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
            GasOracle::Sampled(SampledGasOracle::Ethereum(sampled)) => sampled.prices.clone(),
            _ => panic!("Expected Ethereum sampled oracle"),
        };

        let mut worker = EthereumGasOracleWorker::new(shared_prices.clone(), url);

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

    #[tokio::test]
    #[ignore = "Requires external assumption"]
    async fn test_starknet_gas_oracle() {
        let url =
            Url::parse("https://starknet-mainnet.g.alchemy.com/v2/demo").expect("Invalid URL");
        let oracle = GasOracle::sampled_starknet(url.clone());

        let shared_prices = match &oracle {
            GasOracle::Sampled(SampledGasOracle::Starknet(sampled)) => sampled.prices.clone(),
            _ => panic!("Expected Starknet sampled oracle"),
        };

        let mut worker = StarknetGasOracleWorker::new(shared_prices.clone(), url);

        // Test initial state
        let initial_gas_prices = oracle.current_gas_prices();
        assert_eq!(initial_gas_prices, GasPrice::MIN, "Should start with zero prices");

        // Update once and verify prices are sampled
        worker.update_once().await.expect("Failed to update prices");

        let updated_gas_prices = oracle.current_gas_prices();
        let updated_data_gas_prices = oracle.current_data_gas_prices();

        // Verify gas prices are now non-zero (sampled from Starknet)
        assert!(
            updated_gas_prices.eth.get() > 0 || updated_gas_prices.strk.get() > 0,
            "Gas prices should be non-zero after sampling"
        );

        // Data gas prices should always be valid (non-negative by type constraints)
        let _ = updated_data_gas_prices;
    }
}
