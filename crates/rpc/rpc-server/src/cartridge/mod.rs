//! Handles management of Cartridge controller accounts.
//!
//! When a Controller account is created, the username is used as a salt,
//! and the latest controller class hash is used.
//! This ensures that the controller account address is deterministic.
//!
//! A consequence of that, is that all the controller class hashes must be
//! known by Katana to ensure it can first deploy the controller account with the
//! correct address, and then upgrade it to the latest version.
//!
//! This module contains the function to work around this behavior, which also relies
//! on the updated code into `katana-primitives` to ensure all the controller class hashes
//! are available.
//!
//! Two flows:
//!
//! 1. When a Controller account is created, an execution from outside is received from the very
//!    first transaction that the user will want to achieve using the session. In this case, this
//!    module will hook the execution from outside to ensure the controller account is deployed.
//!
//! 2. When a Controller account is already deployed, and the user logs in, the client code of
//!    controller is actually performing a `estimate_fee` to estimate the fee for the account
//!    upgrade. In this case, this module contains the code to hook the fee estimation, and return
//!    the associated transaction to be executed in order to deploy the controller account. See the
//!    fee estimate RPC method of [StarknetApi](crate::starknet::StarknetApi) to see how the
//!    Controller deployment is handled during fee estimation.

mod vrf;

use std::sync::Arc;

use cainome::cairo_serde::CairoSerde;
use http::{HeaderMap, HeaderName, HeaderValue};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use katana_core::backend::Backend;
use katana_core::service::block_producer::BlockProducer;
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_pool::TxPool;
use katana_primitives::execution::Call;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_api::paymaster::PaymasterApiClient;
use katana_rpc_types::broadcasted::AddInvokeTransactionResponse;
use katana_rpc_types::cartridge::FeeSource;
use katana_rpc_types::outside_execution::OutsideExecution;
use katana_rpc_types::{FunctionCall, SignedOutsideExecution};
use katana_tasks::TaskSpawner;
use paymaster_rpc::{
    DirectInvokeParameters, ExecuteDirectRequest, ExecuteDirectTransactionParameters,
    ExecutionParameters, FeeMode,
};
use starknet_paymaster::core::types::Call as StarknetRsCall;
use tracing::{debug, info, trace_span, Instrument};
use url::Url;
pub use vrf::{VrfService, VrfServiceConfig};

#[derive(Debug, Clone)]
pub struct CartridgeConfig {
    pub api_url: Url,
    pub paymaster_url: Url,
    pub paymaster_api_key: Option<String>,
}

#[allow(missing_debug_implementations)]
pub struct CartridgeApi<PF: ProviderFactory> {
    task_spawner: TaskSpawner,
    backend: Arc<Backend<PF>>,
    block_producer: BlockProducer<PF>,
    pool: TxPool,
    api_client: cartridge::CartridgeApiClient,
    paymaster_client: HttpClient,
}

impl<PF> Clone for CartridgeApi<PF>
where
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self {
            task_spawner: self.task_spawner.clone(),
            backend: self.backend.clone(),
            block_producer: self.block_producer.clone(),
            pool: self.pool.clone(),
            api_client: self.api_client.clone(),
            paymaster_client: self.paymaster_client.clone(),
        }
    }
}

impl<PF> CartridgeApi<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub fn new(
        backend: Arc<Backend<PF>>,
        block_producer: BlockProducer<PF>,
        pool: TxPool,
        task_spawner: TaskSpawner,
        config: CartridgeConfig,
    ) -> anyhow::Result<Self> {
        let api_client = cartridge::CartridgeApiClient::new(config.api_url);

        info!(target: "rpc::cartridge", "Cartridge API initialized.");

        let paymaster_client = {
            let headers = if let Some(api_key) = &config.paymaster_api_key {
                let name = HeaderName::from_static("x-paymaster-api-key");
                let value = HeaderValue::from_str(api_key)?;
                HeaderMap::from_iter([(name, value)])
            } else {
                HeaderMap::default()
            };

            HttpClientBuilder::default().set_headers(headers).build(config.paymaster_url)?
        };

        Ok(Self { task_spawner, backend, block_producer, pool, api_client, paymaster_client })
    }

    pub async fn execute_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> Result<AddInvokeTransactionResponse, CartridgeApiError> {
        debug!(target: "rpc::cartridge", %address, ?fee_source, "Adding execute outside transaction.");

        let fee_mode = match fee_source {
            Some(FeeSource::Credits) => FeeMode::Default {
                gas_token: DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(),
                tip: Default::default(),
            },

            Some(FeeSource::Paymaster) | None => FeeMode::Sponsored { tip: Default::default() },
        };

        let call = Call::from(SignedOutsideExecution { address, outside_execution, signature });
        let invoke = DirectInvokeParameters {
            user_address: call.contract_address.into(),
            execute_from_outside_call: StarknetRsCall {
                calldata: call.calldata,
                to: call.contract_address.into(),
                selector: call.entry_point_selector,
            },
        };

        let transaction = ExecuteDirectTransactionParameters::Invoke { invoke };
        let parameters = ExecutionParameters::V1 { fee_mode, time_bounds: None };

        self.paymaster_client
            .execute_direct_transaction(ExecuteDirectRequest { transaction, parameters })
            .instrument(trace_span!(target: "rpc::cartridge", "paymaster.execute_raw_transaction"))
            .await
            .map(|resp| AddInvokeTransactionResponse { transaction_hash: resp.transaction_hash })
            .map_err(|e| CartridgeApiError::PaymasterExecutionFailed { reason: e.to_string() })
    }
}

#[async_trait]
impl<PF> CartridgeApiServer for CartridgeApi<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    async fn add_execute_outside_transaction(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.execute_outside(address, outside_execution, signature, fee_source).await?)
    }

    async fn add_execute_from_outside(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        signature: Vec<Felt>,
        fee_source: Option<FeeSource>,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.execute_outside(address, outside_execution, signature, fee_source).await?)
    }
}

/// Encodes the given calls into a vector of Felt values (New encoding, cairo 1),
/// since controller accounts are Cairo 1 contracts.
pub fn encode_calls(calls: Vec<FunctionCall>) -> Vec<Felt> {
    let mut execute_calldata: Vec<Felt> = vec![calls.len().into()];
    for call in calls {
        execute_calldata.extend(Call::cairo_serialize(&call));
    }
    execute_calldata
}
