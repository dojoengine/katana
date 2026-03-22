pub mod gateway {
    use std::future::Future;

    use katana_gateway_client::Client as SequencerGateway;
    use katana_rpc_types::Class;

    use super::super::{ClassDownloadKey, ClassDownloader};
    use crate::downloader::{BatchDownloader, Downloader, DownloaderResult};

    /// A [`ClassDownloader`] that fetches classes from the Starknet feeder gateway.
    #[derive(Debug)]
    pub struct GatewayClassDownloader {
        inner: BatchDownloader<GatewayDownloader>,
    }

    impl GatewayClassDownloader {
        pub fn new(gateway: SequencerGateway, batch_size: usize) -> Self {
            Self { inner: BatchDownloader::new(GatewayDownloader { gateway }, batch_size) }
        }
    }

    impl ClassDownloader for GatewayClassDownloader {
        type Error = katana_gateway_client::Error;

        async fn download_classes(
            &self,
            keys: Vec<ClassDownloadKey>,
        ) -> Result<Vec<Class>, Self::Error> {
            self.inner.download(keys).await
        }
    }

    #[derive(Debug)]
    struct GatewayDownloader {
        gateway: SequencerGateway,
    }

    impl Downloader for GatewayDownloader {
        type Key = ClassDownloadKey;
        type Value = Class;
        type Error = katana_gateway_client::Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            async {
                match self.gateway.get_class(key.class_hash, key.block.into()).await {
                    Ok(data) => DownloaderResult::Ok(data),
                    Err(err) if err.is_rate_limited() => DownloaderResult::Retry(err),
                    Err(err) => DownloaderResult::Err(err),
                }
            }
        }
    }
}

pub mod grpc {
    use std::future::Future;
    use std::sync::Arc;

    use katana_grpc::{ClientError, GrpcClient};
    use katana_rpc_types::Class;
    use tokio::sync::OnceCell;
    use tonic::Code;
    use tracing::error;

    use super::super::{ClassDownloadKey, ClassDownloader};
    use crate::downloader::{BatchDownloader, Downloader, DownloaderResult};

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Grpc(#[from] tonic::Status),

        #[error(transparent)]
        Transport(#[from] ClientError),

        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    /// A [`ClassDownloader`] that fetches classes via gRPC.
    ///
    /// Uses lazy connection and clones the client per download (tonic Channel is
    /// cheap to clone).
    #[derive(Debug)]
    pub struct GrpcClassDownloader {
        endpoint: String,
        batch_size: usize,
        client: Arc<OnceCell<GrpcClient>>,
    }

    impl GrpcClassDownloader {
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

    impl ClassDownloader for GrpcClassDownloader {
        type Error = Error;

        async fn download_classes(
            &self,
            keys: Vec<ClassDownloadKey>,
        ) -> Result<Vec<Class>, Self::Error> {
            let client = self.get_or_connect().await?;
            let downloader = GrpcDownloaderInner { client };
            let batch = BatchDownloader::new(downloader, self.batch_size);
            batch.download(keys).await
        }
    }

    #[derive(Debug)]
    struct GrpcDownloaderInner {
        client: GrpcClient,
    }

    impl Downloader for GrpcDownloaderInner {
        type Key = ClassDownloadKey;
        type Value = Class;
        type Error = Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            let mut client = self.client.clone();
            let block = key.block;
            let class_hash = key.class_hash;

            async move {
                let block_id = Some(katana_grpc::proto::BlockId {
                    identifier: Some(katana_grpc::proto::block_id::Identifier::Number(block)),
                });

                let request = katana_grpc::proto::GetClassRequest {
                    block_id,
                    class_hash: Some(class_hash.into()),
                };

                match client.get_class(tonic::Request::new(request)).await.inspect_err(|e| {
                    error!(
                        block = %block,
                        class_hash = %format!("{:#x}", class_hash),
                        error = %e,
                        "Error downloading class via gRPC."
                    );
                }) {
                    Ok(resp) => match class_from_proto(resp.into_inner()) {
                        Ok(class) => DownloaderResult::Ok(class),
                        Err(e) => DownloaderResult::Err(Error::from(e)),
                    },
                    Err(status) => {
                        let err = Error::from(status);
                        if is_retryable(&err) {
                            DownloaderResult::Retry(err)
                        } else {
                            DownloaderResult::Err(err)
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

    /// Deserializes a `Class` from the gRPC response.
    ///
    /// The server serializes the full class as JSON bytes into the oneof
    /// variant (because the class types are not compatible with non-human-
    /// readable serialization formats).
    fn class_from_proto(resp: katana_grpc::proto::GetClassResponse) -> anyhow::Result<Class> {
        use katana_grpc::proto::get_class_response::Result as ClassResult;

        let result =
            resp.result.ok_or_else(|| anyhow::anyhow!("missing class result in gRPC response"))?;

        let json = match &result {
            ClassResult::ContractClass(bytes) | ClassResult::DeprecatedContractClass(bytes) => {
                bytes
            }
        };

        let class: Class = serde_json::from_slice(json)?;
        Ok(class)
    }
}

pub mod json_rpc {
    use std::future::Future;

    use katana_primitives::block::BlockIdOrTag;
    use katana_rpc_types::Class;
    use katana_starknet::rpc::Client as JsonRpcClient;
    use tracing::error;

    use super::super::{ClassDownloadKey, ClassDownloader};
    use crate::downloader::{BatchDownloader, Downloader, DownloaderResult};

    /// A [`ClassDownloader`] that fetches classes via JSON-RPC using `starknet_getClass`.
    #[derive(Debug)]
    pub struct JsonRpcClassDownloader {
        inner: BatchDownloader<JsonRpcDownloader>,
    }

    impl JsonRpcClassDownloader {
        pub fn new(client: JsonRpcClient, batch_size: usize) -> Self {
            Self { inner: BatchDownloader::new(JsonRpcDownloader { client }, batch_size) }
        }
    }

    impl ClassDownloader for JsonRpcClassDownloader {
        type Error = katana_starknet::rpc::Error;

        async fn download_classes(
            &self,
            keys: Vec<ClassDownloadKey>,
        ) -> Result<Vec<Class>, Self::Error> {
            self.inner.download(keys).await
        }
    }

    #[derive(Debug)]
    struct JsonRpcDownloader {
        client: JsonRpcClient,
    }

    impl Downloader for JsonRpcDownloader {
        type Key = ClassDownloadKey;
        type Value = Class;
        type Error = katana_starknet::rpc::Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            async {
                let block_id = BlockIdOrTag::Number(key.block);
                match self.client.get_class(block_id, key.class_hash).await.inspect_err(|e| {
                    error!(
                        block = %key.block,
                        class_hash = %format!("{:#x}", key.class_hash),
                        error = %e,
                        "Error downloading class via JSON-RPC."
                    );
                }) {
                    Ok(data) => DownloaderResult::Ok(data),
                    Err(err) if err.is_retryable() => DownloaderResult::Retry(err),
                    Err(err) => DownloaderResult::Err(err),
                }
            }
        }
    }
}
