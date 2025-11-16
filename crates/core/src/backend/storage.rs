use std::sync::Arc;

use anyhow::{bail, Context, Result};
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, GasPrices};
use katana_provider::api::block::{BlockIdReader, BlockProvider, BlockWriter};
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::api::state::{StateFactoryProvider, StateWriter};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_provider::api::trie::TrieWriter;
use katana_provider::{DbProviderFactory, ForkProviderFactory, ProviderFactory};
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_types::GetBlockWithTxHashesResponse;
use num_traits::ToPrimitive;
use starknet::core::utils::parse_cairo_short_string;
use tracing::info;

pub type GenericStorageProvider =
    Arc<dyn ProviderFactory<Provider = Box<dyn DatabaseRO>, ProviderMut = Box<dyn DatabaseRW>>>;

#[derive(Debug, Clone)]
pub struct StorageProvider<P> {
    provider_factory: P,
}

impl<P: ProviderFactory> StorageProvider<P> {
    pub fn new(provider_factory: P) -> Self {
        Self { provider_factory }
    }
}

impl StorageProvider<DbProviderFactory<katana_db::Db>> {
    pub fn new_with_db(db: katana_db::Db) -> Self {
        Self::new(DbProviderFactory::new(db))
    }

    pub fn new_in_memory() -> Self {
        Self::new(DbProviderFactory::new_in_memory())
    }
}

impl StorageProvider<ForkProviderFactory<katana_db::Db>> {
    /// Builds a new blockchain with a forked block.
    pub async fn new_forked(
        db: katana_db::Db,
        client: StarknetClient,
        fork_block: Option<BlockHashOrNumber>,
        chain: &mut katana_chain_spec::dev::ChainSpec,
    ) -> Result<Self> {
        let chain_id = client.chain_id().await.context("failed to fetch forked network id")?;

        // if the id is not in ASCII encoding, we display the chain id as is in hex.
        let parsed_id = match parse_cairo_short_string(&chain_id) {
            Ok(id) => id,
            Err(_) => format!("{chain_id:#x}"),
        };

        // If the fork block number is not specified, we use the latest accepted block on the forked
        // network.
        let block_id = if let Some(id) = fork_block {
            id
        } else {
            let res = client.block_number().await?;
            BlockHashOrNumber::Num(res.block_number)
        };

        info!(chain = %parsed_id, block = %block_id, "Forking chain.");

        let block = client
            .get_block_with_tx_hashes(BlockIdOrTag::from(block_id))
            .await
            .context("failed to fetch forked block")?;

        let GetBlockWithTxHashesResponse::Block(forked_block) = block else {
            bail!("forking a pending block is not allowed")
        };

        let block_num = forked_block.block_number;
        let genesis_block_num = block_num + 1;

        chain.id = chain_id.into();

        // adjust the genesis to match the forked block
        chain.genesis.timestamp = forked_block.timestamp;
        chain.genesis.number = genesis_block_num;
        chain.genesis.state_root = Default::default();
        chain.genesis.parent_hash = forked_block.parent_hash;
        chain.genesis.sequencer_address = forked_block.sequencer_address;

        // TODO: remove gas price from genesis
        let eth_l1_gas_price =
            forked_block.l1_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_gas_price =
            forked_block.l1_gas_price.price_in_fri.to_u128().expect("should fit in u128");
        chain.genesis.gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_gas_price, strk_l1_gas_price) };

        // TODO: convert this to block number instead of BlockHashOrNumber so that it is easier to
        // check if the requested block is within the supported range or not.
        let provider_factory = ForkProviderFactory::new(db, block_num, client.clone());

        // update the genesis block with the forked block's data
        // we dont update the `l1_gas_price` bcs its already done when we set the `gas_prices` in
        // genesis. this flow is kinda flawed, we should probably refactor it out of the
        // genesis.
        let mut block = chain.block();

        let eth_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_fri.to_u128().expect("should fit in u128");

        block.header.l1_data_gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_data_gas_price, strk_l1_data_gas_price) };

        block.header.l1_da_mode = forked_block.l1_da_mode;

        Ok(Self::new(provider_factory))
    }
}

impl<P> ProviderFactory for StorageProvider<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: DatabaseRO,
    <P as ProviderFactory>::ProviderMut: DatabaseRW,
{
    type Provider = Box<dyn DatabaseRO>;
    type ProviderMut = Box<dyn DatabaseRW>;

    fn provider(&self) -> Self::Provider {
        Box::new(self.provider_factory.provider_mut())
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        Box::new(self.provider_factory.provider_mut())
    }
}

pub trait DatabaseRO:
    BlockIdReader
    + BlockProvider
    + TransactionProvider
    + TransactionStatusProvider
    + TransactionTraceProvider
    + TransactionsProviderExt
    + ReceiptProvider
    + StateUpdateProvider
    + StateFactoryProvider
    + BlockEnvProvider
    + 'static
    + Send
    + Sync
    + core::fmt::Debug
{
}

pub trait DatabaseRW:
    DatabaseRO + BlockWriter + StateWriter + ContractClassWriter + TrieWriter + StageCheckpointProvider
{
}

impl<T> DatabaseRO for T where
    T: BlockProvider
        + BlockIdReader
        + TransactionProvider
        + TransactionStatusProvider
        + TransactionTraceProvider
        + TransactionsProviderExt
        + ReceiptProvider
        + StateUpdateProvider
        + StateFactoryProvider
        + BlockEnvProvider
        + 'static
        + Send
        + Sync
        + core::fmt::Debug
{
}

impl<T> DatabaseRW for T where
    T: DatabaseRO
        + BlockWriter
        + StateWriter
        + ContractClassWriter
        + TrieWriter
        + StageCheckpointProvider
{
}
