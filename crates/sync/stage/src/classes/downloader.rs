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

pub mod json_rpc {
    use katana_primitives::block::BlockIdOrTag;
    use katana_rpc_types::Class;
    use katana_starknet::rpc::Client as JsonRpcClient;
    use tracing::error;

    use super::super::{ClassDownloadKey, ClassDownloader};

    /// A [`ClassDownloader`] that fetches classes via JSON-RPC using `starknet_getClass`.
    #[derive(Debug)]
    pub struct JsonRpcClassDownloader {
        client: JsonRpcClient,
    }

    impl JsonRpcClassDownloader {
        pub fn new(client: JsonRpcClient) -> Self {
            Self { client }
        }
    }

    impl ClassDownloader for JsonRpcClassDownloader {
        type Error = katana_starknet::rpc::Error;

        async fn download_classes(
            &self,
            keys: Vec<ClassDownloadKey>,
        ) -> Result<Vec<Class>, Self::Error> {
            let mut classes = Vec::with_capacity(keys.len());

            for key in keys {
                let block_id = BlockIdOrTag::Number(key.block);
                let class =
                    self.client.get_class(block_id, key.class_hash).await.inspect_err(|e| {
                        error!(
                            block = %key.block,
                            class_hash = %format!("{:#x}", key.class_hash),
                            error = %e,
                            "Error downloading class via JSON-RPC."
                        );
                    })?;
                classes.push(class);
            }

            Ok(classes)
        }
    }
}
