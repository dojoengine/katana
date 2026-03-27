//! Block downloading abstractions and implementations for the Blocks stage.
//!
//! This module defines the [`BlockDownloader`] trait, which provides a stage-specific
//! interface for downloading block data. The trait is designed to be flexible and can
//! be implemented in various ways depending on the download strategy and data source
//! (e.g., gateway-based, JSON-RPC-based, P2P-based, or custom implementations).
//!
//! [`BatchBlockDownloader`] is one such implementation that leverages the generic
//! [`BatchDownloader`](crate::downloader::BatchDownloader) utility for concurrent
//! downloads with retry logic. This is suitable for many use cases but is not the
//! only way to implement block downloading.

use std::future::Future;

use katana_primitives::block::{BlockNumber, SealedBlockWithStatus};
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdatesWithClasses;

use crate::downloader::{BatchDownloader, Downloader};

/// The block data produced by a [`BlockDownloader`].
///
/// This is a source-agnostic representation of downloaded block data containing
/// everything needed by the [`Blocks`] stage.
#[derive(Debug)]
pub struct BlockData {
    pub block: SealedBlockWithStatus,
    pub receipts: Vec<Receipt>,
    pub state_updates: StateUpdatesWithClasses,
}

/// Trait for downloading block data.
///
/// This trait provides a stage-specific abstraction for downloading blocks, allowing different
/// implementations (e.g., gateway-based, JSON-RPC-based, custom strategies) to be used with the
/// [`Blocks`](crate::blocks::Blocks) stage.
///
/// Implementors can use any download strategy they choose, including but not limited to the
/// [`BatchDownloader`](crate::downloader::BatchDownloader) utility provided by this crate.
pub trait BlockDownloader: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Downloads blocks for the given block number range and returns them as [`BlockData`].
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<BlockData>, Self::Error>> + Send;
}

///////////////////////////////////////////////////////////////////////////////////
// Implementations
///////////////////////////////////////////////////////////////////////////////////

/// An implementation of [`BlockDownloader`] that uses the [`BatchDownloader`] utility.
///
/// This implementation leverages the generic
/// [`BatchDownloader`](crate::downloader::BatchDownloader) to download blocks concurrently in
/// batches with automatic retry logic. It's a straightforward approach suitable for many scenarios.
#[derive(Debug)]
pub struct BatchBlockDownloader<D> {
    inner: BatchDownloader<D>,
}

impl<D> BatchBlockDownloader<D> {
    /// Create a new [`BatchBlockDownloader`] with the given [`Downloader`] and batch size.
    pub fn new(downloader: D, batch_size: usize) -> Self {
        Self { inner: BatchDownloader::new(downloader, batch_size) }
    }
}

impl<D, V> BlockDownloader for BatchBlockDownloader<D>
where
    D: Downloader<Key = BlockNumber, Value = V>,
    D: Send + Sync,
    D::Error: Send + Sync + 'static,
    V: Into<BlockData> + Send + Sync,
{
    type Error = D::Error;

    async fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockData>, Self::Error> {
        let block_keys = (from..=to).collect::<Vec<BlockNumber>>();
        let results = self.inner.download(block_keys).await?;
        Ok(results.into_iter().map(Into::into).collect())
    }
}

pub mod gateway {
    use katana_gateway_client::Client as GatewayClient;
    use katana_gateway_types::{
        BlockStatus, StateUpdate as GatewayStateUpdate, StateUpdateWithBlock,
    };
    use katana_primitives::block::{
        BlockNumber, FinalityStatus, GasPrices, Header, SealedBlock, SealedBlockWithStatus,
    };
    use katana_primitives::fee::{FeeInfo, PriceUnit};
    use katana_primitives::receipt::{
        DeclareTxReceipt, DeployAccountTxReceipt, DeployTxReceipt, InvokeTxReceipt,
        L1HandlerTxReceipt, Receipt,
    };
    use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
    use katana_primitives::transaction::{Tx, TxWithHash};
    use katana_primitives::Felt;
    use num_traits::ToPrimitive;
    use starknet::core::types::ResourcePrice;

    use super::BatchBlockDownloader;
    use crate::blocks::BlockData;
    use crate::downloader::{Downloader, DownloaderResult};

    impl BatchBlockDownloader<GatewayDownloader> {
        /// Create a new [`BatchBlockDownloader`] using the Starknet gateway for downloading
        /// blocks.
        pub fn new_gateway(
            client: GatewayClient,
            batch_size: usize,
        ) -> BatchBlockDownloader<GatewayDownloader> {
            Self::new(GatewayDownloader::new(client), batch_size)
        }
    }

    /// Internal [`Downloader`] implementation that uses the sequencer gateway for downloading a
    /// block.
    #[derive(Debug)]
    pub struct GatewayDownloader {
        gateway: GatewayClient,
    }

    impl GatewayDownloader {
        pub fn new(gateway: GatewayClient) -> Self {
            Self { gateway }
        }
    }

    impl Downloader for GatewayDownloader {
        type Key = BlockNumber;
        type Value = StateUpdateWithBlock;
        type Error = katana_gateway_client::Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl std::future::Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            use katana_gateway_client::Error as GatewayClientError;
            use tracing::error;

            async {
                match self.gateway.get_state_update_with_block((*key).into()).await.inspect_err(
                    |error| error!(block = %*key, ?error, "Error downloading block from gateway."),
                ) {
                    Ok(data) => DownloaderResult::Ok(data),
                    Err(err) => match err {
                        GatewayClientError::RateLimited
                        | GatewayClientError::UnknownFormat { .. } => DownloaderResult::Retry(err),
                        _ => DownloaderResult::Err(err),
                    },
                }
            }
        }
    }

    /// Converts gateway [`StateUpdateWithBlock`] into [`BlockData`].
    impl From<katana_gateway_types::StateUpdateWithBlock> for BlockData {
        fn from(data: katana_gateway_types::StateUpdateWithBlock) -> Self {
            fn to_gas_prices(prices: ResourcePrice) -> GasPrices {
                let eth = prices.price_in_wei.to_u128().expect("valid u128");
                let strk = prices.price_in_fri.to_u128().expect("valid u128");
                let eth = if eth == 0 { 1 } else { eth };
                let strk = if strk == 0 { 1 } else { strk };
                unsafe { GasPrices::new_unchecked(eth, strk) }
            }

            let status = match data.block.status {
                BlockStatus::AcceptedOnL2 => FinalityStatus::AcceptedOnL2,
                BlockStatus::AcceptedOnL1 => FinalityStatus::AcceptedOnL1,
                status => panic!("unsupported block status: {status:?}"),
            };

            let transactions = data
                .block
                .transactions
                .into_iter()
                .map(|tx| tx.try_into())
                .collect::<std::result::Result<Vec<TxWithHash>, _>>()
                .expect("valid transaction conversion");

            let receipts = data
                .block
                .transaction_receipts
                .into_iter()
                .zip(transactions.iter())
                .map(|(receipt, tx)| {
                    let events = receipt.body.events;
                    let revert_error = receipt.body.revert_error;
                    let messages_sent = receipt.body.l2_to_l1_messages;
                    let overall_fee = receipt.body.actual_fee.to_u128().expect("valid u128");
                    let execution_resources = receipt.body.execution_resources.unwrap_or_default();

                    let unit = if tx.transaction.version() >= Felt::THREE {
                        PriceUnit::Fri
                    } else {
                        PriceUnit::Wei
                    };

                    let fee = FeeInfo { unit, overall_fee, ..Default::default() };

                    match &tx.transaction {
                        Tx::Invoke(_) => Receipt::Invoke(InvokeTxReceipt {
                            fee,
                            events,
                            revert_error,
                            messages_sent,
                            execution_resources: execution_resources.into(),
                        }),
                        Tx::Declare(_) => Receipt::Declare(DeclareTxReceipt {
                            fee,
                            events,
                            revert_error,
                            messages_sent,
                            execution_resources: execution_resources.into(),
                        }),
                        Tx::L1Handler(_) => Receipt::L1Handler(L1HandlerTxReceipt {
                            fee,
                            events,
                            messages_sent,
                            revert_error,
                            message_hash: Default::default(),
                            execution_resources: execution_resources.into(),
                        }),
                        Tx::DeployAccount(tx) => Receipt::DeployAccount(DeployAccountTxReceipt {
                            fee,
                            events,
                            revert_error,
                            messages_sent,
                            contract_address: tx.contract_address(),
                            execution_resources: execution_resources.into(),
                        }),
                        Tx::Deploy(tx) => Receipt::Deploy(DeployTxReceipt {
                            fee,
                            events,
                            revert_error,
                            messages_sent,
                            contract_address: tx.contract_address.into(),
                            execution_resources: execution_resources.into(),
                        }),
                    }
                })
                .collect::<Vec<Receipt>>();

            let transaction_count = transactions.len() as u32;
            let events_count = receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();

            let block = SealedBlock {
                body: transactions,
                hash: data.block.block_hash.unwrap_or_default(),
                header: Header {
                    transaction_count,
                    events_count,
                    timestamp: data.block.timestamp,
                    l1_da_mode: data.block.l1_da_mode,
                    parent_hash: data.block.parent_block_hash,
                    state_diff_length: Default::default(),
                    state_diff_commitment: Default::default(),
                    number: data.block.block_number.unwrap_or_default(),
                    l1_gas_prices: to_gas_prices(data.block.l1_gas_price),
                    l2_gas_prices: to_gas_prices(data.block.l2_gas_price),
                    state_root: data.block.state_root.unwrap_or_default(),
                    l1_data_gas_prices: to_gas_prices(data.block.l1_data_gas_price),
                    starknet_version: data
                        .block
                        .starknet_version
                        .unwrap_or_default()
                        .try_into()
                        .unwrap(),
                    events_commitment: data.block.event_commitment.unwrap_or_default(),
                    receipts_commitment: data.block.receipt_commitment.unwrap_or_default(),
                    sequencer_address: data.block.sequencer_address.unwrap_or_default(),
                    transactions_commitment: data.block.transaction_commitment.unwrap_or_default(),
                },
            };

            let state_updates: StateUpdates = match data.state_update {
                GatewayStateUpdate::Confirmed(update) => update.state_diff.into(),
                GatewayStateUpdate::PreConfirmed(update) => update.state_diff.into(),
            };

            let state_updates = StateUpdatesWithClasses { state_updates, ..Default::default() };

            BlockData { block: SealedBlockWithStatus { block, status }, receipts, state_updates }
        }
    }
}

pub mod json_rpc {
    use katana_primitives::block::{
        BlockIdOrTag, BlockNumber, GasPrices, Header, SealedBlock, SealedBlockWithStatus,
    };
    use katana_primitives::receipt::Receipt;
    use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
    use katana_primitives::transaction::TxWithHash;
    use katana_primitives::Felt;
    use katana_rpc_types::block::{BlockWithReceipts, GetBlockWithReceiptsResponse};
    use katana_starknet::rpc::StarknetRpcClient;
    use num_traits::ToPrimitive;
    use starknet::core::types::ResourcePrice;
    use tracing::error;

    use super::{BatchBlockDownloader, BlockData};
    use crate::downloader::{Downloader, DownloaderResult};

    pub type JsonRpcBlockDownloader = BatchBlockDownloader<JsonRpcDownloader>;

    impl BatchBlockDownloader<JsonRpcDownloader> {
        /// Create a new [`BatchBlockDownloader`] using a JSON-RPC endpoint for downloading
        /// blocks.
        pub fn new_json_rpc(
            client: StarknetRpcClient,
            batch_size: usize,
        ) -> BatchBlockDownloader<JsonRpcDownloader> {
            Self::new(JsonRpcDownloader::new(client), batch_size)
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Rpc(#[from] katana_starknet::rpc::StarknetRpcClientError),

        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    /// Internal [`Downloader`] implementation that uses JSON-RPC for downloading a block.
    #[derive(Debug)]
    pub struct JsonRpcDownloader {
        client: StarknetRpcClient,
    }

    impl JsonRpcDownloader {
        pub fn new(client: StarknetRpcClient) -> Self {
            Self { client }
        }
    }

    impl Downloader for JsonRpcDownloader {
        type Key = BlockNumber;
        type Value = BlockData;
        type Error = Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl std::future::Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            let block_num = *key;
            async move {
                let block_id = BlockIdOrTag::Number(block_num);

                let result = tokio::try_join!(
                    async {
                        self.client
                            .get_block_with_receipts(block_id)
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading block via JSON-RPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                    async {
                        self.client
                            .get_state_update(block_id)
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading state update via JSON-RPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                );

                match result {
                    Ok((block_resp, state_update)) => {
                        match BlockData::from_rpc(block_resp, state_update) {
                            Ok(data) => DownloaderResult::Ok(data),
                            Err(e) => DownloaderResult::Err(Error::from(e)),
                        }
                    }
                    Err(err) => match err {
                        Error::Rpc(ref rpc_err) if rpc_err.is_retryable() => {
                            DownloaderResult::Retry(err)
                        }
                        _ => DownloaderResult::Err(err),
                    },
                }
            }
        }
    }

    impl BlockData {
        /// Converts a JSON-RPC block-with-receipts response and state update into [`BlockData`].
        pub fn from_rpc(
            block_resp: katana_rpc_types::GetBlockWithReceiptsResponse,
            state_update: katana_rpc_types::StateUpdate,
        ) -> anyhow::Result<Self> {
            fn to_gas_prices(prices: ResourcePrice) -> GasPrices {
                let eth = prices.price_in_wei.to_u128().expect("valid u128");
                let strk = prices.price_in_fri.to_u128().expect("valid u128");
                let eth = if eth == 0 { 1 } else { eth };
                let strk = if strk == 0 { 1 } else { strk };
                unsafe { GasPrices::new_unchecked(eth, strk) }
            }

            let BlockWithReceipts {
                status,
                block_hash,
                parent_hash,
                block_number,
                new_root,
                timestamp,
                sequencer_address,
                l1_gas_price,
                l2_gas_price,
                l1_data_gas_price,
                l1_da_mode,
                starknet_version,
                transactions: tx_with_receipts,
            } = match block_resp {
                GetBlockWithReceiptsResponse::Block(b) => b,
                GetBlockWithReceiptsResponse::PreConfirmed(_) => {
                    anyhow::bail!("pre-confirmed blocks are not supported for syncing");
                }
            };

            let mut transactions = Vec::with_capacity(tx_with_receipts.len());
            let mut receipts = Vec::with_capacity(tx_with_receipts.len());

            for tx_receipt in tx_with_receipts {
                let tx_with_hash = katana_rpc_types::RpcTxWithHash {
                    transaction_hash: tx_receipt.receipt.transaction_hash,
                    transaction: tx_receipt.transaction,
                };
                let tx: TxWithHash = tx_with_hash.into();
                let receipt: Receipt = tx_receipt.receipt.receipt.into();
                transactions.push(tx);
                receipts.push(receipt);
            }

            let transaction_count = transactions.len() as u32;
            let events_count = receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();

            let block = SealedBlock {
                body: transactions,
                hash: block_hash,
                header: Header {
                    transaction_count,
                    events_count,
                    timestamp,
                    l1_da_mode,
                    parent_hash,
                    number: block_number,
                    state_root: new_root,
                    sequencer_address,
                    l1_gas_prices: to_gas_prices(l1_gas_price),
                    l2_gas_prices: to_gas_prices(l2_gas_price),
                    l1_data_gas_prices: to_gas_prices(l1_data_gas_price),
                    starknet_version: starknet_version.try_into().unwrap(),
                    // RPC doesn't return commitments; they'll be computed by
                    // `compute_missing_commitments`
                    events_commitment: Felt::ZERO,
                    receipts_commitment: Felt::ZERO,
                    transactions_commitment: Felt::ZERO,
                    state_diff_length: Default::default(),
                    state_diff_commitment: Default::default(),
                },
            };

            let state_updates: StateUpdates = match state_update {
                katana_rpc_types::StateUpdate::Confirmed(update) => update.state_diff.into(),
                katana_rpc_types::StateUpdate::PreConfirmed(update) => update.state_diff.into(),
            };

            let state_updates = StateUpdatesWithClasses { state_updates, ..Default::default() };

            Ok(BlockData {
                block: SealedBlockWithStatus { block, status },
                receipts,
                state_updates,
            })
        }
    }
}
