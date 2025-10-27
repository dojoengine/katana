use std::fmt::Debug;

use katana_primitives::block::GasPrices;
use num_traits::ToPrimitive;
use starknet::core::types::{BlockId, BlockTag, MaybePreConfirmedBlockWithTxHashes};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;

use super::SampledPrices;

#[derive(Debug, Clone)]
pub struct StarknetSampler<P = JsonRpcClient<HttpTransport>> {
    provider: P,
}

impl<P> StarknetSampler<P> {
    /// Creates a new [`StarknetGasPriceSampler`] using the given provider.
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P: starknet::providers::Provider> StarknetSampler<P> {
    async fn sample_prices(&self) -> anyhow::Result<SampledPrices> {
        let block_id = BlockId::Tag(BlockTag::Latest);
        let block = self.provider.get_block_with_tx_hashes(block_id).await?;

        let (l1_gas_price, l2_gas_price, l1_data_gas_price) = match block {
            MaybePreConfirmedBlockWithTxHashes::Block(block) => {
                (block.l1_gas_price, block.l2_gas_price, block.l1_data_gas_price)
            }
            MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(pending) => {
                (pending.l1_gas_price, pending.l2_gas_price, pending.l1_data_gas_price)
            }
        };

        let l2_gas_prices = GasPrices::new(
            l2_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l2_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        let l1_gas_prices = GasPrices::new(
            l1_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l1_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        let l1_data_gas_prices = GasPrices::new(
            l1_data_gas_price.price_in_wei.to_u128().unwrap().try_into()?,
            l1_data_gas_price.price_in_fri.to_u128().unwrap().try_into()?,
        );

        Ok(SampledPrices { l2_gas_prices, l1_gas_prices, l1_data_gas_prices })
    }
}

impl<P> super::Sampler for StarknetSampler<P>
where
    P: starknet::providers::Provider + Send + Sync,
{
    async fn sample(&self) -> anyhow::Result<SampledPrices> {
        self.sample_prices().await
    }
}

#[cfg(test)]
mod tests {
    use katana_utils::mock_provider;
    use starknet::core::types::{
        BlockStatus, BlockWithTxHashes, Felt, L1DataAvailabilityMode,
        MaybePreConfirmedBlockWithTxHashes, ResourcePrice,
    };

    use crate::sampled::starknet::StarknetSampler;

    mock_provider! {
        MockGasProvider,

       fn get_block_with_tx_hashes: (_) => {
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(BlockWithTxHashes {
                block_number: 100,
                transactions: vec![],
                timestamp: 1234567890,
                block_hash: Felt::from(1u32),
                new_root: Felt::from(123u32),
                parent_hash: Felt::from(0u32),
                status: BlockStatus::AcceptedOnL2,
                sequencer_address: Felt::from(999u32),
                starknet_version: "0.13.0".to_string(),
                l1_da_mode: L1DataAvailabilityMode::Blob,
                l1_gas_price: ResourcePrice {
                    price_in_wei: Felt::from(1000u32),
                    price_in_fri: Felt::from(2000u32),
                },
                l2_gas_price: ResourcePrice {
                    price_in_wei: Felt::from(500u32),
                    price_in_fri: Felt::from(750u32),
                },
                l1_data_gas_price: ResourcePrice {
                    price_in_wei: Felt::from(100u32),
                    price_in_fri: Felt::from(150u32),
                },
            }))
        }
    }

    #[tokio::test]
    async fn sample_gas_prices() {
        let sampler = StarknetSampler::new(MockGasProvider::new());
        let sampled_prices = sampler.sample_prices().await.unwrap();

        assert_eq!(sampled_prices.l2_gas_prices.eth.get(), 500);
        assert_eq!(sampled_prices.l2_gas_prices.strk.get(), 750);

        assert_eq!(sampled_prices.l1_gas_prices.eth.get(), 1000);
        assert_eq!(sampled_prices.l1_gas_prices.strk.get(), 2000);

        assert_eq!(sampled_prices.l1_data_gas_prices.eth.get(), 100);
        assert_eq!(sampled_prices.l1_data_gas_prices.strk.get(), 150);
    }
}
