use std::sync::Arc;

use anyhow::{bail, Context, Result};
use katana_chain_spec::ChainSpec;
use katana_primitives::block::{BlockHashOrNumber, GasPrices};
use katana_primitives::cairo::ShortString;
use katana_provider::ForkProviderFactory;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::GetBlockWithTxHashesResponse;
use num_traits::ToPrimitive;
use tracing::info;

use crate::config::fork::ForkingConfig;
use crate::config::NodeConfig;
use crate::sequencer::Sequencer;

pub type ForkedNodeConfig = NodeConfig<katana_chain_spec::dev::ChainSpec, ForkingConfig>;

pub type ForkedSequencer =
    Sequencer<katana_chain_spec::dev::ChainSpec, ForkProviderFactory, ForkingConfig>;

impl ForkedSequencer {
    pub async fn build(mut config: ForkedNodeConfig) -> Result<Self> {
        // NOTE: because the chain spec will be cloned for the BlockifierFactory (see below),
        // this mutation must be performed before the chain spec is cloned. Otherwise
        // this will panic.
        let chain_spec = Arc::get_mut(&mut config.chain).expect("get mut Arc");

        let forking_cfg = &config.extension;

        let db = katana_db::Db::in_memory()?;

        let client = StarknetClient::new(forking_cfg.url.clone());
        let chain_id = client.chain_id().await.context("failed to fetch forked network id")?;

        // If the fork block number is not specified, we use the latest accepted block on the forked
        // network.
        let block_id = if let Some(id) = forking_cfg.block {
            id
        } else {
            let res = client.block_number().await?;
            BlockHashOrNumber::Num(res.block_number)
        };

        // if the id is not in ASCII encoding, we display the chain id as is in hex.
        match ShortString::try_from(chain_id) {
            Ok(id) => {
                info!(chain = %id, block = %block_id, "Forking chain.");
            }

            Err(_) => {
                let id = format!("{chain_id:#x}");
                info!(chain = %id, block = %block_id, "Forking chain.");
            }
        };

        let block = client
            .get_block_with_tx_hashes(block_id.into())
            .await
            .context("failed to fetch forked block")?;

        let GetBlockWithTxHashesResponse::Block(forked_block) = block else {
            bail!("forking a pending block is not allowed")
        };

        let block_num = forked_block.block_number;
        let genesis_block_num = block_num + 1;

        chain_spec.id = chain_id.into();

        // adjust the genesis to match the forked block
        chain_spec.genesis.timestamp = forked_block.timestamp;
        chain_spec.genesis.number = genesis_block_num;
        chain_spec.genesis.state_root = Default::default();
        chain_spec.genesis.parent_hash = forked_block.parent_hash;
        chain_spec.genesis.sequencer_address = forked_block.sequencer_address;

        // TODO: remove gas price from genesis
        let eth_l1_gas_price =
            forked_block.l1_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_gas_price =
            forked_block.l1_gas_price.price_in_fri.to_u128().expect("should fit in u128");
        chain_spec.genesis.gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_gas_price, strk_l1_gas_price) };

        // TODO: convert this to block number instead of BlockHashOrNumber so that it is easier to
        // check if the requested block is within the supported range or not.
        let provider_factory = ForkProviderFactory::new(db.clone(), block_num, client.clone());

        // update the genesis block with the forked block's data
        // we dont update the `l1_gas_price` bcs its already done when we set the `gas_prices` in
        // genesis. this flow is kinda flawed, we should probably refactor it out of the
        // genesis.
        let mut block = chain_spec.block();

        let eth_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_fri.to_u128().expect("should fit in u128");

        block.header.l1_data_gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_data_gas_price, strk_l1_data_gas_price) };

        block.header.l1_da_mode = forked_block.l1_da_mode;

        Sequencer::build_with_provider(db, provider_factory, config)
    }
}
