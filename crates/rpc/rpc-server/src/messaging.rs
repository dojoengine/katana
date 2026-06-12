use jsonrpsee::core::{async_trait, RpcResult};
use katana_messaging::MessagingController;
use katana_provider::api::messaging::{MessagingCheckpointProvider, MessagingL1ToL2IndexWriter};
use katana_provider::{MutableProvider, ProviderFactory, ProviderRW};
use katana_rpc_api::error::messaging::MessagingApiError;
use katana_rpc_api::messaging::MessagingApiServer;
use katana_rpc_types::messaging::MessagingCheckpoint;

fn to_rpc(cp: katana_provider::api::messaging::MessagingCheckpoint) -> MessagingCheckpoint {
    MessagingCheckpoint { block: cp.block, tx_index: cp.tx_index }
}

/// JSON-RPC handler for the `messaging` namespace. Delegates to a
/// [`MessagingController`] obtained from a running [`MessagingServer`].
#[allow(missing_debug_implementations)]
pub struct MessagingApiHandler<P> {
    controller: MessagingController<P>,
}

impl<P> MessagingApiHandler<P> {
    pub fn new(controller: MessagingController<P>) -> Self {
        Self { controller }
    }
}

impl<P> MessagingApiHandler<P>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    fn get_checkpoint(&self) -> Result<Option<MessagingCheckpoint>, MessagingApiError> {
        self.controller
            .get_checkpoint()
            .map(|opt| opt.map(to_rpc))
            .map_err(|e| MessagingApiError::StorageError(e.to_string()))
    }

    async fn set_checkpoint(&self, block: u64, tx_index: u64) -> Result<(), MessagingApiError> {
        self.controller
            .set_checkpoint(block, tx_index)
            .await
            .map_err(|e| MessagingApiError::StorageError(e.to_string()))
    }

    async fn reset_checkpoint(&self) -> Result<(), MessagingApiError> {
        self.controller
            .reset_checkpoint()
            .await
            .map_err(|e| MessagingApiError::StorageError(e.to_string()))
    }
}

#[async_trait]
impl<P> MessagingApiServer for MessagingApiHandler<P>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    async fn get_checkpoint(&self) -> RpcResult<Option<MessagingCheckpoint>> {
        Ok(self.get_checkpoint()?)
    }

    async fn set_checkpoint(&self, block: u64, tx_index: u64) -> RpcResult<()> {
        Ok(self.set_checkpoint(block, tx_index).await?)
    }

    async fn reset_checkpoint(&self) -> RpcResult<()> {
        Ok(self.reset_checkpoint().await?)
    }
}
