use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ::starknet::providers::jsonrpc::HttpTransport;
use alloy_provider::network;
use backon::{ExponentialBuilder, Retryable};
use buffer::GasPricesBuffer;
use katana_primitives::block::GasPrices;
use parking_lot::Mutex;
use tracing::{error, warn};
use url::Url;

mod buffer;
pub mod ethereum;
pub mod starknet;

const DEFAULT_SAMPLING_INTERVAL: Duration = Duration::from_secs(60);

/// Trait for sampling gas prices from different blockchain networks.
#[auto_impl::auto_impl(Box)]
pub trait Sampler: Send + Sync {
    /// Sample gas prices from the underlying network.
    fn sample(&self) -> impl Future<Output = anyhow::Result<SampledPrices>> + Send;
}

#[derive(Debug, Clone)]
pub struct SampledPriceOracle {
    inner: Arc<SampledPriceOracleInner>,
}

struct SampledPriceOracleInner {
    samples: Mutex<Samples>,
    sampler: Box<dyn Sampler>,
}

impl SampledPriceOracle {
    pub fn new(sampler: impl Sampler + 'static) -> Self {
        let samples = Mutex::new(Samples::new());
        let sampler = Box::new(sampler);
        let inner = Arc::new(SampledPriceOracleInner { samples, sampler });
        Self { inner }
    }

    /// Returns the average l2 gas prices of the samples.
    pub fn avg_l2_gas_prices(&self) -> GasPrices {
        self.inner.samples.lock().l2_gas_prices.average()
    }

    /// Returns the average l1 gas prices of the samples.
    pub fn avg_l1_gas_prices(&self) -> GasPrices {
        self.inner.samples.lock().l1_gas_prices.average()
    }

    /// Returns the average l1 data gas prices of the samples.
    pub fn avg_l1_data_gas_prices(&self) -> GasPrices {
        self.inner.samples.lock().l1_data_gas_prices.average()
    }

    /// Runs the worker that samples gas prices from the configured network.
    pub fn run_worker(&self) -> impl Future<Output = ()> + 'static {
        let inner = self.inner.clone();

        // every 60 seconds, Starknet samples the base price of gas and data gas on L1
        let mut interval = tokio::time::interval(DEFAULT_SAMPLING_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        async move {
            loop {
                interval.tick().await;

                let request = || async { inner.sampler.clone().sample().await };
                let backoff = ExponentialBuilder::default().with_min_delay(Duration::from_secs(3));
                let future = request.retry(backoff).notify(|error, _| {
                    warn!(target: "gas_oracle", %error, "Retrying gas prices sampling.");
                });

                match future.await {
                    Ok(prices) => {
                        let mut buffers = inner.samples.lock();
                        buffers.l2_gas_prices.push(prices.l2_gas_prices);
                        buffers.l1_gas_prices.push(prices.l1_gas_prices);
                        buffers.l1_data_gas_prices.push(prices.l1_data_gas_prices);
                    }
                    Err(error) => {
                        error!(target: "gas_oracle", %error, "Failed to sample gas prices.")
                    }
                }
            }
        }
    }
}

/// The default sample size for the gas prices buffer.
const SAMPLE_SIZE: usize = 60;

#[derive(Debug, Clone)]
struct Samples {
    l2_gas_prices: GasPricesBuffer<SAMPLE_SIZE>,
    l1_gas_prices: GasPricesBuffer<SAMPLE_SIZE>,
    l1_data_gas_prices: GasPricesBuffer<SAMPLE_SIZE>,
}

impl Samples {
    fn new() -> Self {
        Self {
            l2_gas_prices: GasPricesBuffer::new(),
            l1_gas_prices: GasPricesBuffer::new(),
            l1_data_gas_prices: GasPricesBuffer::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SampledPrices {
    pub l2_gas_prices: GasPrices,
    pub l1_gas_prices: GasPrices,
    pub l1_data_gas_prices: GasPrices,
}
