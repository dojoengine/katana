//! Block downloading abstractions and implementations for the Blocks stage.
//!
//! This module defines the [`BlockDownloader`] trait, which provides a stage-specific
//! interface for downloading block data. The trait is designed to be flexible and can
//! be implemented in various ways depending on the download strategy and data source
//! (e.g., gateway-based, JSON-RPC-based, gRPC-based, P2P-based, or custom implementations).
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
    use katana_starknet::rpc::Client as JsonRpcClient;
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
            client: JsonRpcClient,
            batch_size: usize,
        ) -> BatchBlockDownloader<JsonRpcDownloader> {
            Self::new(JsonRpcDownloader::new(client), batch_size)
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Rpc(#[from] katana_starknet::rpc::Error),

        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    /// Internal [`Downloader`] implementation that uses JSON-RPC for downloading a block.
    #[derive(Debug)]
    pub struct JsonRpcDownloader {
        client: JsonRpcClient,
    }

    impl JsonRpcDownloader {
        pub fn new(client: JsonRpcClient) -> Self {
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

pub mod grpc {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use katana_grpc::proto::{
        BlockId as ProtoBlockId, GetBlockRequest, GetBlockWithReceiptsResponse,
        GetStateUpdateResponse,
    };
    use katana_grpc::{ClientError, GrpcClient};
    use katana_primitives::block::{
        BlockNumber, FinalityStatus, GasPrices, Header, SealedBlock, SealedBlockWithStatus,
    };
    use katana_primitives::contract::ContractAddress;
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::fee::{FeeInfo, PriceUnit};
    use katana_primitives::receipt::{
        DeclareTxReceipt, DeployAccountTxReceipt, DeployTxReceipt, Event, ExecutionResources,
        GasUsed, InvokeTxReceipt, L1HandlerTxReceipt, MessageToL1, Receipt,
    };
    use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
    use katana_primitives::transaction::{Tx, TxWithHash};
    use katana_primitives::Felt;
    use num_traits::ToPrimitive;
    use tokio::sync::OnceCell;
    use tonic::Code;
    use tracing::error;

    use super::{BlockData, BlockDownloader};

    /// Type alias for the gRPC block downloader.
    pub type GrpcBlockDownloader = GrpcBatchBlockDownloader;

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Grpc(#[from] tonic::Status),

        #[error(transparent)]
        Transport(#[from] ClientError),

        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    /// A [`BlockDownloader`] that fetches blocks via gRPC.
    ///
    /// Uses lazy connection — the gRPC client connects on first download.
    /// The client is cloned per download call (tonic Channel is cheap to clone).
    #[derive(Debug)]
    pub struct GrpcBatchBlockDownloader {
        endpoint: String,
        batch_size: usize,
        client: Arc<OnceCell<GrpcClient>>,
    }

    impl GrpcBatchBlockDownloader {
        pub fn new(endpoint: String, batch_size: usize) -> Self {
            Self { endpoint, batch_size, client: Arc::new(OnceCell::new()) }
        }

        async fn get_or_connect(&self) -> Result<GrpcClient, Error> {
            let client = self
                .client
                .get_or_try_init(|| async { GrpcClient::connect(&self.endpoint).await })
                .await?;
            Ok(client.clone())
        }
    }

    impl BlockDownloader for GrpcBatchBlockDownloader {
        type Error = Error;

        async fn download_blocks(
            &self,
            from: BlockNumber,
            to: BlockNumber,
        ) -> Result<Vec<BlockData>, Self::Error> {
            let client = self.get_or_connect().await?;
            let downloader = GrpcDownloader { client };
            let batch = crate::downloader::BatchDownloader::new(downloader, self.batch_size);

            let block_keys = (from..=to).collect::<Vec<BlockNumber>>();
            let results = batch.download(block_keys).await?;
            Ok(results)
        }
    }

    /// Internal downloader that fetches a single block via gRPC.
    #[derive(Debug)]
    struct GrpcDownloader {
        client: GrpcClient,
    }

    impl crate::downloader::Downloader for GrpcDownloader {
        type Key = BlockNumber;
        type Value = BlockData;
        type Error = Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl std::future::Future<
            Output = crate::downloader::DownloaderResult<Self::Value, Self::Error>,
        > {
            let block_num = *key;
            let client = self.client.clone();
            async move {
                let block_id = Some(ProtoBlockId {
                    identifier: Some(katana_grpc::proto::block_id::Identifier::Number(block_num)),
                });

                let block_request = GetBlockRequest { block_id: block_id.clone() };
                let state_request = GetBlockRequest { block_id };

                // Clone the client for each concurrent call since GrpcClient
                // methods take &mut self.
                let mut block_client = client.clone();
                let mut state_client = client;

                let result = tokio::try_join!(
                    async {
                        block_client
                            .get_block_with_receipts(tonic::Request::new(block_request))
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading block via gRPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                    async {
                        state_client
                            .get_state_update(tonic::Request::new(state_request))
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading state update via gRPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                );

                match result {
                    Ok((block_resp, state_resp)) => {
                        match BlockData::from_grpc(block_resp.into_inner(), state_resp.into_inner())
                        {
                            Ok(data) => crate::downloader::DownloaderResult::Ok(data),
                            Err(e) => crate::downloader::DownloaderResult::Err(Error::from(e)),
                        }
                    }
                    Err(err) => {
                        if is_retryable(&err) {
                            crate::downloader::DownloaderResult::Retry(err)
                        } else {
                            crate::downloader::DownloaderResult::Err(err)
                        }
                    }
                }
            }
        }
    }

    fn is_retryable(err: &Error) -> bool {
        match err {
            Error::Grpc(status) => matches!(
                status.code(),
                Code::Unavailable | Code::ResourceExhausted | Code::DeadlineExceeded
            ),
            Error::Transport(_) => true,
            Error::Other(_) => false,
        }
    }

    // =========================================================================
    // Proto → BlockData conversion
    // =========================================================================

    fn felt_from_proto(f: Option<katana_grpc::proto::Felt>) -> Felt {
        f.map(|f| Felt::from_bytes_be_slice(&f.value)).unwrap_or_default()
    }

    fn to_gas_prices(price: Option<katana_grpc::proto::ResourcePrice>) -> GasPrices {
        let price = price.unwrap_or_default();
        let eth = felt_from_proto(price.price_in_wei).to_u128().unwrap_or(1);
        let strk = felt_from_proto(price.price_in_fri).to_u128().unwrap_or(1);
        let eth = if eth == 0 { 1 } else { eth };
        let strk = if strk == 0 { 1 } else { strk };
        unsafe { GasPrices::new_unchecked(eth, strk) }
    }

    fn l1_da_mode_from_proto(mode: i32) -> L1DataAvailabilityMode {
        use katana_grpc::proto::L1DataAvailabilityMode as ProtoMode;
        match ProtoMode::try_from(mode) {
            Ok(ProtoMode::Calldata) => L1DataAvailabilityMode::Calldata,
            _ => L1DataAvailabilityMode::Blob,
        }
    }

    fn finality_status_from_proto(status: i32) -> FinalityStatus {
        use katana_grpc::proto::FinalityStatus as ProtoStatus;
        match ProtoStatus::try_from(status) {
            Ok(ProtoStatus::AcceptedOnL1) => FinalityStatus::AcceptedOnL1,
            _ => FinalityStatus::AcceptedOnL2,
        }
    }

    fn events_from_proto(events: Vec<katana_grpc::proto::Event>) -> Vec<Event> {
        events
            .into_iter()
            .map(|e| Event {
                from_address: ContractAddress::new(felt_from_proto(e.from_address)),
                keys: e.keys.into_iter().map(|f| Felt::from_bytes_be_slice(&f.value)).collect(),
                data: e.data.into_iter().map(|f| Felt::from_bytes_be_slice(&f.value)).collect(),
            })
            .collect()
    }

    fn messages_from_proto(messages: Vec<katana_grpc::proto::MessageToL1>) -> Vec<MessageToL1> {
        messages
            .into_iter()
            .map(|m| MessageToL1 {
                from_address: ContractAddress::new(felt_from_proto(m.from_address)),
                to_address: felt_from_proto(m.to_address),
                payload: m
                    .payload
                    .into_iter()
                    .map(|f| Felt::from_bytes_be_slice(&f.value))
                    .collect(),
            })
            .collect()
    }

    fn execution_resources_from_proto(
        res: Option<katana_grpc::proto::ExecutionResources>,
    ) -> ExecutionResources {
        let res = res.unwrap_or_default();
        ExecutionResources {
            total_gas_consumed: GasUsed {
                l1_gas: res.l1_gas,
                l1_data_gas: res.l1_data_gas,
                l2_gas: res.l2_gas,
            },
            ..Default::default()
        }
    }

    fn tx_from_proto(
        proto_tx: katana_grpc::proto::Transaction,
        tx_hash: Felt,
    ) -> anyhow::Result<TxWithHash> {
        use katana_grpc::proto::transaction::Transaction as ProtoTxVariant;
        use katana_primitives::transaction::{
            DeclareTx, DeclareTxV0, DeclareTxV1, DeclareTxV2, DeclareTxV3, DeployAccountTx,
            DeployAccountTxV1, DeployAccountTxV3, DeployTx, InvokeTx, InvokeTxV0, InvokeTxV1,
            InvokeTxV3, L1HandlerTx,
        };

        let variant =
            proto_tx.transaction.ok_or_else(|| anyhow::anyhow!("missing transaction variant"))?;

        let felts = |v: Vec<katana_grpc::proto::Felt>| -> Vec<Felt> {
            v.into_iter().map(|f| Felt::from_bytes_be_slice(&f.value)).collect()
        };

        let tx = match variant {
            ProtoTxVariant::InvokeV1(v) => {
                let version = parse_version(&v.version);
                if version == Felt::ZERO {
                    Tx::Invoke(InvokeTx::V0(InvokeTxV0 {
                        max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                        signature: felts(v.signature),
                        contract_address: ContractAddress::new(felt_from_proto(
                            v.sender_address.clone(),
                        )),
                        entry_point_selector: Felt::ZERO,
                        calldata: felts(v.calldata),
                    }))
                } else {
                    Tx::Invoke(InvokeTx::V1(InvokeTxV1 {
                        chain_id: Default::default(),
                        max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                        signature: felts(v.signature),
                        nonce: felt_from_proto(v.nonce),
                        sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                        calldata: felts(v.calldata),
                    }))
                }
            }

            ProtoTxVariant::InvokeV3(v) => {
                let resource_bounds = v
                    .resource_bounds
                    .as_ref()
                    .map(resource_bounds_from_proto)
                    .unwrap_or_else(default_resource_bounds);
                Tx::Invoke(InvokeTx::V3(InvokeTxV3 {
                    chain_id: Default::default(),
                    signature: felts(v.signature),
                    nonce: felt_from_proto(v.nonce),
                    sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                    calldata: felts(v.calldata),
                    resource_bounds,
                    tip: felt_from_proto(v.tip).to_u64().unwrap_or(0),
                    paymaster_data: felts(v.paymaster_data),
                    account_deployment_data: felts(v.account_deployment_data),
                    nonce_data_availability_mode: da_mode_from_string(
                        &v.nonce_data_availability_mode,
                    ),
                    fee_data_availability_mode: da_mode_from_string(&v.fee_data_availability_mode),
                }))
            }

            ProtoTxVariant::DeclareV1(v) => {
                let version = parse_version(&v.version);
                if version == Felt::ZERO {
                    Tx::Declare(DeclareTx::V0(DeclareTxV0 {
                        chain_id: Default::default(),
                        max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                        signature: felts(v.signature),
                        sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                        class_hash: felt_from_proto(v.class_hash),
                    }))
                } else {
                    Tx::Declare(DeclareTx::V1(DeclareTxV1 {
                        chain_id: Default::default(),
                        max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                        signature: felts(v.signature),
                        nonce: felt_from_proto(v.nonce),
                        sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                        class_hash: felt_from_proto(v.class_hash),
                    }))
                }
            }

            ProtoTxVariant::DeclareV2(v) => Tx::Declare(DeclareTx::V2(DeclareTxV2 {
                chain_id: Default::default(),
                max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                signature: felts(v.signature),
                nonce: felt_from_proto(v.nonce),
                sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                compiled_class_hash: felt_from_proto(v.compiled_class_hash),
                class_hash: Felt::ZERO,
            })),

            ProtoTxVariant::DeclareV3(v) => {
                let resource_bounds = v
                    .resource_bounds
                    .as_ref()
                    .map(resource_bounds_from_proto)
                    .unwrap_or_else(default_resource_bounds);
                Tx::Declare(DeclareTx::V3(DeclareTxV3 {
                    chain_id: Default::default(),
                    signature: felts(v.signature),
                    nonce: felt_from_proto(v.nonce),
                    sender_address: ContractAddress::new(felt_from_proto(v.sender_address)),
                    compiled_class_hash: felt_from_proto(v.compiled_class_hash),
                    class_hash: felt_from_proto(v.class_hash),
                    resource_bounds,
                    tip: felt_from_proto(v.tip).to_u64().unwrap_or(0),
                    paymaster_data: felts(v.paymaster_data),
                    account_deployment_data: felts(v.account_deployment_data),
                    nonce_data_availability_mode: da_mode_from_string(
                        &v.nonce_data_availability_mode,
                    ),
                    fee_data_availability_mode: da_mode_from_string(&v.fee_data_availability_mode),
                }))
            }

            ProtoTxVariant::DeployAccount(v) => {
                Tx::DeployAccount(DeployAccountTx::V1(DeployAccountTxV1 {
                    chain_id: Default::default(),
                    max_fee: felt_from_proto(v.max_fee).to_u128().unwrap_or(0),
                    signature: felts(v.signature),
                    nonce: felt_from_proto(v.nonce),
                    class_hash: felt_from_proto(v.class_hash),
                    contract_address: Default::default(),
                    contract_address_salt: felt_from_proto(v.contract_address_salt),
                    constructor_calldata: felts(v.constructor_calldata),
                }))
            }

            ProtoTxVariant::DeployAccountV3(v) => {
                let resource_bounds = v
                    .resource_bounds
                    .as_ref()
                    .map(resource_bounds_from_proto)
                    .unwrap_or_else(default_resource_bounds);
                Tx::DeployAccount(DeployAccountTx::V3(DeployAccountTxV3 {
                    chain_id: Default::default(),
                    signature: felts(v.signature),
                    nonce: felt_from_proto(v.nonce),
                    class_hash: felt_from_proto(v.class_hash),
                    contract_address: Default::default(),
                    contract_address_salt: felt_from_proto(v.contract_address_salt),
                    constructor_calldata: felts(v.constructor_calldata),
                    resource_bounds,
                    tip: felt_from_proto(v.tip).to_u64().unwrap_or(0),
                    paymaster_data: felts(v.paymaster_data),
                    nonce_data_availability_mode: da_mode_from_string(
                        &v.nonce_data_availability_mode,
                    ),
                    fee_data_availability_mode: da_mode_from_string(&v.fee_data_availability_mode),
                }))
            }

            ProtoTxVariant::L1Handler(v) => Tx::L1Handler(L1HandlerTx {
                chain_id: Default::default(),
                version: parse_version(&v.version),
                nonce: felt_from_proto(v.nonce),
                contract_address: ContractAddress::new(felt_from_proto(v.contract_address)),
                entry_point_selector: felt_from_proto(v.entry_point_selector),
                calldata: felts(v.calldata),
                message_hash: Default::default(),
                paid_fee_on_l1: 0,
            }),

            ProtoTxVariant::Deploy(v) => Tx::Deploy(DeployTx {
                version: parse_version(&v.version),
                class_hash: felt_from_proto(v.class_hash),
                contract_address_salt: felt_from_proto(v.contract_address_salt),
                constructor_calldata: felts(v.constructor_calldata),
                contract_address: Felt::ZERO,
            }),
        };

        Ok(TxWithHash { hash: tx_hash, transaction: tx })
    }

    fn parse_version(version: &str) -> Felt {
        Felt::from_hex(version).unwrap_or_default()
    }

    fn da_mode_from_string(s: &str) -> katana_primitives::da::DataAvailabilityMode {
        match s.to_uppercase().as_str() {
            "L1" => katana_primitives::da::DataAvailabilityMode::L1,
            _ => katana_primitives::da::DataAvailabilityMode::L2,
        }
    }

    fn default_resource_bounds() -> katana_primitives::fee::ResourceBoundsMapping {
        use katana_primitives::fee::L1GasResourceBoundsMapping;
        katana_primitives::fee::ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping::default())
    }

    fn resource_bounds_from_proto(
        rbm: &katana_grpc::proto::ResourceBoundsMapping,
    ) -> katana_primitives::fee::ResourceBoundsMapping {
        use katana_primitives::fee::{
            AllResourceBoundsMapping, L1GasResourceBoundsMapping,
            ResourceBounds as PrimResourceBounds, ResourceBoundsMapping,
        };

        let rb_from = |rb: &katana_grpc::proto::ResourceBounds| -> PrimResourceBounds {
            PrimResourceBounds {
                max_amount: felt_from_proto(rb.max_amount.clone()).to_u64().unwrap_or(0),
                max_price_per_unit: felt_from_proto(rb.max_price_per_unit.clone())
                    .to_u128()
                    .unwrap_or(0),
            }
        };

        let l1_gas = rbm.l1_gas.as_ref().map(rb_from).unwrap_or_default();
        let l2_gas = rbm.l2_gas.as_ref().map(rb_from).unwrap_or_default();

        if let Some(ref l1_data_gas_proto) = rbm.l1_data_gas {
            let l1_data_gas = rb_from(l1_data_gas_proto);
            ResourceBoundsMapping::All(AllResourceBoundsMapping { l1_gas, l2_gas, l1_data_gas })
        } else {
            ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping { l1_gas, l2_gas })
        }
    }

    impl BlockData {
        /// Converts gRPC proto responses into [`BlockData`].
        pub fn from_grpc(
            block_resp: GetBlockWithReceiptsResponse,
            state_resp: GetStateUpdateResponse,
        ) -> anyhow::Result<Self> {
            use katana_grpc::proto::get_block_with_receipts_response::Result as BlockResult;
            use katana_grpc::proto::get_state_update_response::Result as StateResult;

            let block_result = block_resp
                .result
                .ok_or_else(|| anyhow::anyhow!("missing block result in gRPC response"))?;

            let (status, header, tx_with_receipts) = match block_result {
                BlockResult::Block(b) => {
                    let status = finality_status_from_proto(b.status);
                    (status, b.header, b.transactions)
                }
                BlockResult::PendingBlock(_) => {
                    anyhow::bail!("pre-confirmed blocks are not supported for syncing");
                }
            };

            let header =
                header.ok_or_else(|| anyhow::anyhow!("missing block header in gRPC response"))?;

            let mut transactions = Vec::with_capacity(tx_with_receipts.len());
            let mut receipts = Vec::with_capacity(tx_with_receipts.len());

            for twr in tx_with_receipts {
                let proto_receipt =
                    twr.receipt.ok_or_else(|| anyhow::anyhow!("missing receipt"))?;
                let proto_tx =
                    twr.transaction.ok_or_else(|| anyhow::anyhow!("missing transaction"))?;

                let tx_hash = felt_from_proto(proto_receipt.transaction_hash.clone());
                let tx = tx_from_proto(proto_tx, tx_hash)?;

                let fee_amount = proto_receipt
                    .actual_fee
                    .as_ref()
                    .map(|f| felt_from_proto(f.amount.clone()).to_u128().unwrap_or(0))
                    .unwrap_or(0);

                let fee_unit = proto_receipt
                    .actual_fee
                    .as_ref()
                    .map(|f| match f.unit.to_uppercase().as_str() {
                        "FRI" => PriceUnit::Fri,
                        _ => PriceUnit::Wei,
                    })
                    .unwrap_or(PriceUnit::Wei);

                let fee = FeeInfo { unit: fee_unit, overall_fee: fee_amount, ..Default::default() };
                let events = events_from_proto(proto_receipt.events);
                let messages_sent = messages_from_proto(proto_receipt.messages_sent);
                let execution_resources =
                    execution_resources_from_proto(proto_receipt.execution_resources);
                let revert_error = if proto_receipt.revert_reason.is_empty() {
                    None
                } else {
                    Some(proto_receipt.revert_reason)
                };

                let receipt = match &tx.transaction {
                    Tx::Invoke(_) => Receipt::Invoke(InvokeTxReceipt {
                        fee,
                        events,
                        revert_error,
                        messages_sent,
                        execution_resources,
                    }),
                    Tx::Declare(_) => Receipt::Declare(DeclareTxReceipt {
                        fee,
                        events,
                        revert_error,
                        messages_sent,
                        execution_resources,
                    }),
                    Tx::L1Handler(_) => Receipt::L1Handler(L1HandlerTxReceipt {
                        fee,
                        events,
                        messages_sent,
                        revert_error,
                        message_hash: Default::default(),
                        execution_resources,
                    }),
                    Tx::DeployAccount(da_tx) => Receipt::DeployAccount(DeployAccountTxReceipt {
                        fee,
                        events,
                        revert_error,
                        messages_sent,
                        contract_address: da_tx.contract_address(),
                        execution_resources,
                    }),
                    Tx::Deploy(d_tx) => Receipt::Deploy(DeployTxReceipt {
                        fee,
                        events,
                        revert_error,
                        messages_sent,
                        contract_address: d_tx.contract_address.into(),
                        execution_resources,
                    }),
                };

                transactions.push(tx);
                receipts.push(receipt);
            }

            let transaction_count = transactions.len() as u32;
            let events_count = receipts.iter().map(|r| r.events().len() as u32).sum::<u32>();

            let block = SealedBlock {
                body: transactions,
                hash: felt_from_proto(header.block_hash),
                header: Header {
                    transaction_count,
                    events_count,
                    timestamp: header.timestamp,
                    l1_da_mode: l1_da_mode_from_proto(header.l1_da_mode),
                    parent_hash: felt_from_proto(header.parent_hash),
                    number: header.block_number,
                    state_root: felt_from_proto(header.new_root),
                    sequencer_address: ContractAddress::new(felt_from_proto(
                        header.sequencer_address,
                    )),
                    l1_gas_prices: to_gas_prices(header.l1_gas_price),
                    l2_gas_prices: to_gas_prices(header.l2_gas_price),
                    l1_data_gas_prices: to_gas_prices(header.l1_data_gas_price),
                    starknet_version: header.starknet_version.try_into().unwrap_or_default(),
                    // Commitments will be computed by `compute_missing_commitments`
                    events_commitment: Felt::ZERO,
                    receipts_commitment: Felt::ZERO,
                    transactions_commitment: Felt::ZERO,
                    state_diff_length: Default::default(),
                    state_diff_commitment: Default::default(),
                },
            };

            // Parse state update
            let state_result = state_resp
                .result
                .ok_or_else(|| anyhow::anyhow!("missing state update result in gRPC response"))?;

            let state_diff = match state_result {
                StateResult::StateUpdate(update) => update.state_diff,
                StateResult::PendingStateUpdate(update) => update.state_diff,
            };

            let state_diff =
                state_diff.ok_or_else(|| anyhow::anyhow!("missing state_diff in gRPC response"))?;

            let state_updates = state_diff_from_proto(state_diff);
            let state_updates = StateUpdatesWithClasses { state_updates, ..Default::default() };

            Ok(BlockData {
                block: SealedBlockWithStatus { block, status },
                receipts,
                state_updates,
            })
        }
    }

    fn state_diff_from_proto(diff: katana_grpc::proto::StateDiff) -> StateUpdates {
        let storage_updates = diff
            .storage_diffs
            .into_iter()
            .map(|sd| {
                let address = ContractAddress::new(felt_from_proto(sd.address));
                let entries: BTreeMap<Felt, Felt> = sd
                    .storage_entries
                    .into_iter()
                    .map(|se| (felt_from_proto(se.key), felt_from_proto(se.value)))
                    .collect();
                (address, entries)
            })
            .collect();

        let nonce_updates = diff
            .nonces
            .into_iter()
            .map(|n| {
                (
                    ContractAddress::new(felt_from_proto(n.contract_address)),
                    felt_from_proto(n.nonce),
                )
            })
            .collect();

        let deployed_contracts = diff
            .deployed_contracts
            .into_iter()
            .map(|dc| {
                (ContractAddress::new(felt_from_proto(dc.address)), felt_from_proto(dc.class_hash))
            })
            .collect();

        let declared_classes = diff
            .declared_classes
            .into_iter()
            .map(|dc| (felt_from_proto(dc.class_hash), felt_from_proto(dc.compiled_class_hash)))
            .collect();

        let deprecated_declared_classes = diff
            .deprecated_declared_classes
            .into_iter()
            .map(|f| Felt::from_bytes_be_slice(&f.value))
            .collect();

        let replaced_classes = diff
            .replaced_classes
            .into_iter()
            .map(|rc| {
                (
                    ContractAddress::new(felt_from_proto(rc.contract_address)),
                    felt_from_proto(rc.class_hash),
                )
            })
            .collect();

        StateUpdates {
            nonce_updates,
            storage_updates,
            deployed_contracts,
            declared_classes,
            deprecated_declared_classes,
            replaced_classes,
            migrated_compiled_classes: Default::default(),
        }
    }
}
