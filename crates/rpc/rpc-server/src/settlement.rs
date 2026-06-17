//! Server handler for the settlement-status methods in the `katana` namespace.

use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::INTERNAL_ERROR_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::api::settlement::SettlementCheckpointProvider;
use katana_provider::ProviderFactory;
use katana_rpc_api::katana::KatanaSettlementApiServer;
use katana_rpc_types::settlement::SettlementStatus;

/// Serves `katana_settlementStatus` straight from storage: the durable settled-block checkpoint
/// (written by the embedded settlement service) alongside the live local chain head.
///
/// On a node that doesn't settle, the checkpoint is absent so the settled block reads as `0`, while
/// the head still reflects the actual chain tip.
#[derive(Debug, Clone)]
pub struct SettlementApi<PF> {
    provider: PF,
}

impl<PF> SettlementApi<PF> {
    pub fn new(provider: PF) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<PF> KatanaSettlementApiServer for SettlementApi<PF>
where
    PF: ProviderFactory,
    PF::Provider: BlockNumberProvider + SettlementCheckpointProvider,
{
    async fn settlement_status(&self) -> RpcResult<SettlementStatus> {
        let provider = self.provider.provider();

        let internal = |e: katana_provider::ProviderError| {
            ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, e.to_string(), None::<()>)
        };

        let head = provider.latest_number().map_err(internal)?;
        let settled_block = provider.settled_block().map_err(internal)?.unwrap_or(0);

        Ok(SettlementStatus { settled_block, head })
    }
}
