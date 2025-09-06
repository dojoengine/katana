use anyhow::{bail, Context, Result};
use katana_primitives::block::{
    BlockHashOrNumber, BlockIdOrTag, BlockNumber, FinalityStatus, GasPrices, Header, SealedBlock,
    SealedBlockWithStatus,
};
use katana_provider::api::block::{BlockProvider, BlockWriter};
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
use katana_provider::providers::db::DbProvider;
use katana_provider::providers::fork::ForkedProvider;
use katana_provider::BlockchainProvider;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_client::HttpClientBuilder;
use katana_rpc_types::GetBlockWithTxHashesResponse;
use num_traits::ToPrimitive;
use starknet::core::utils::parse_cairo_short_string;
use tracing::info;
use url::Url;

pub trait Database:
    BlockProvider
    + BlockWriter
    + TransactionProvider
    + TransactionStatusProvider
    + TransactionTraceProvider
    + TransactionsProviderExt
    + ReceiptProvider
    + StateUpdateProvider
    + StateWriter
    + ContractClassWriter
    + StateFactoryProvider
    + BlockEnvProvider
    + TrieWriter
    + StageCheckpointProvider
    + 'static
    + Send
    + Sync
    + core::fmt::Debug
{
}

impl<T> Database for T where
    T: BlockProvider
        + BlockWriter
        + TransactionProvider
        + TransactionStatusProvider
        + TransactionTraceProvider
        + TransactionsProviderExt
        + ReceiptProvider
        + StateUpdateProvider
        + StateWriter
        + ContractClassWriter
        + StateFactoryProvider
        + BlockEnvProvider
        + TrieWriter
        + StageCheckpointProvider
        + 'static
        + Send
        + Sync
        + core::fmt::Debug
{
}

#[derive(Debug, Clone)]
pub struct Blockchain {
    inner: BlockchainProvider<Box<dyn Database>>,
}

impl Blockchain {
    pub fn new(provider: impl Database) -> Self {
        Self { inner: BlockchainProvider::new(Box::new(provider)) }
    }

    /// Creates a new [Blockchain] from a database at `path` and `genesis` state.
    pub fn new_with_db(db: katana_db::Db) -> Self {
        Self::new(DbProvider::new(db))
    }

    /// Builds a new blockchain with a forked block.
    pub async fn new_from_forked(
        db: katana_db::Db,
        fork_url: Url,
        fork_block: Option<BlockHashOrNumber>,
        chain: &mut katana_chain_spec::dev::ChainSpec,
    ) -> Result<(Self, BlockNumber)> {
        let provider = StarknetClient::new(HttpClientBuilder::new().build(fork_url)?);
        let chain_id = provider.chain_id().await.context("failed to fetch forked network id")?;

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
            let res = provider.block_number().await?;
            BlockHashOrNumber::Num(res.block_number)
        };

        info!(chain = %parsed_id, block = %block_id, "Forking chain.");

        let block = provider
            .get_block_with_tx_hashes(BlockIdOrTag::from(block_id))
            .await
            .context("failed to fetch forked block")?;

        let GetBlockWithTxHashesResponse::Block(forked_block) = block else {
            bail!("forking a pending block is not allowed")
        };

        let block_num = forked_block.block_number;

        chain.id = chain_id.into();

        // adjust the genesis to match the forked block
        chain.genesis.timestamp = forked_block.timestamp;
        chain.genesis.number = forked_block.block_number;
        chain.genesis.state_root = forked_block.new_root;
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
        let database = ForkedProvider::new(db, block_id, provider.clone());

        // initialize parent fork block
        //
        // NOTE: this is just a workaround for allowing forked genesis block to be initialize using
        // `Backend::do_mine_block`.
        {
            let parent_block_id = BlockIdOrTag::from(forked_block.parent_hash);
            let parent_block = provider.get_block_with_tx_hashes(parent_block_id).await?;

            let GetBlockWithTxHashesResponse::Block(parent_block) = parent_block else {
                bail!("parent block is a preconfirmed block");
            };

            let parent_block = SealedBlockWithStatus {
                block: SealedBlock {
                    hash: parent_block.block_hash,
                    body: Vec::new(),
                    header: Header {
                        parent_hash: parent_block.parent_hash,
                        timestamp: parent_block.timestamp,
                        number: parent_block.block_number,
                        state_root: parent_block.new_root,
                        sequencer_address: parent_block.sequencer_address,
                        ..Default::default()
                    },
                },
                status: FinalityStatus::AcceptedOnL2,
            };

            database
                .insert_block_with_states_and_receipts(
                    parent_block,
                    Default::default(),
                    Default::default(),
                    Default::default(),
                )
                .context("failed to initialize provider with the parent of the forked block")?;
        }

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

        Ok((Self::new(database), block_num))
    }

    pub fn provider(&self) -> &BlockchainProvider<Box<dyn Database>> {
        &self.inner
    }
}
