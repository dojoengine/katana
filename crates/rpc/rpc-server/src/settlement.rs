//! Server handler for the settlement-status methods in the `katana` namespace.

use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::INTERNAL_ERROR_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::ProviderFactory;
use katana_rpc_api::katana::KatanaSettlementApiServer;
use katana_rpc_types::settlement::SettlementStatus;
use katana_settlement::SettlementStatusHandle;

/// Serves `katana_settlementStatus`: the settlement service's most recent settled block (from a
/// [`SettlementStatusHandle`]) alongside the live local chain head (read from storage).
///
/// Registered even on nodes that don't settle — there the settled block is always `0`, while the
/// head still reflects the actual chain tip.
#[derive(Debug, Clone)]
pub struct SettlementApi<PF> {
    status: SettlementStatusHandle,
    provider: PF,
}

impl<PF> SettlementApi<PF> {
    pub fn new(status: SettlementStatusHandle, provider: PF) -> Self {
        Self { status, provider }
    }
}

#[async_trait]
impl<PF> KatanaSettlementApiServer for SettlementApi<PF>
where
    PF: ProviderFactory,
    PF::Provider: BlockNumberProvider,
{
    async fn settlement_status(&self) -> RpcResult<SettlementStatus> {
        let head =
            self.provider.provider().latest_number().map_err(|e| {
                ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, e.to_string(), None::<()>)
            })?;

        Ok(SettlementStatus { settled_block: self.status.settled_block(), head })
    }
}
