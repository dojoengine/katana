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

#[derive(Debug)]
pub struct FixedGasOracle {
    l2_gas_prices: GasPrice,
    l1_gas_prices: GasPrice,
    l1_data_gas_prices: GasPrice,
}

impl FixedGasOracle {
    pub fn current_l1_data_gas_prices(&self) -> GasPrice {
        self.l1_data_gas_prices.clone()
    }

    pub fn current_l1_gas_prices(&self) -> GasPrice {
        self.l1_gas_prices.clone()
    }

    pub fn current_l2_gas_prices(&self) -> GasPrice {
        self.l2_gas_prices.clone()
    }
}
