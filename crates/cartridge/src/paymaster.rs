use std::borrow::Cow;
use std::future::Future;

use futures::executor::block_on;
use jsonrpsee::core::middleware;
use jsonrpsee::core::middleware::{Batch, Notification};
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::Request;
use katana_executor::ExecutorFactory;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::block::{BlockIdOrTag, BlockTag};
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_primitives::transaction::{
    ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3, TxHash,
};
use katana_primitives::{ContractAddress, Felt};
use katana_rpc::starknet::StarknetApi;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::transaction::{BroadcastedDeclareTx, BroadcastedInvokeTx, BroadcastedTx};
use starknet::core::types::{Call, SimulationFlagForEstimateFee};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tracing::trace;

use crate::rpc::types::OutsideExecution;
use crate::utils::encode_calls;
use crate::Client;

pub type PaymasterResult<T> = Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no controller found for address {0}")]
    ControllerNotFound(ContractAddress),

    #[error("cartridge client error: {0}")]
    Client(#[from] crate::client::Error),

    #[error("starknet api error: {0}")]
    StarknetApi(#[from] StarknetApiError),

    #[error("paymaster not found")]
    PaymasterNotFound(ContractAddress),

    #[error("failed to add deploy controller transaction to the pool: {0}")]
    FailedToAddTransaction(#[from] katana_pool::PoolError),
}

#[derive(Debug)]
pub struct Paymaster<EF: ExecutorFactory> {
    starknet_api: StarknetApi<EF>,
    cartridge_api: Client,
    pool: TxPool,

    chain_id: ChainId,
    paymaster_key: SigningKey,
    paymaster_address: ContractAddress,
}

impl<EF: ExecutorFactory> Paymaster<EF> {
    pub fn new(
        starknet_api: StarknetApi<EF>,
        cartridge_api: Client,
        pool: TxPool,
        chain_id: ChainId,
        paymaster_address: ContractAddress,
        paymaster_key: SigningKey,
    ) -> Self {
        Self { starknet_api, cartridge_api, pool, chain_id, paymaster_key, paymaster_address }
    }

    /// Deploys the account contract of a Controller account.
    pub fn deploy_controller(&self, address: ContractAddress) -> PaymasterResult<TxHash> {
        let block_id = BlockIdOrTag::Tag(BlockTag::Pending);
        let tx = self.get_controller_deploy_tx(address, block_id)?;
        let tx_hash = self.pool.add_transaction(tx).map_err(Error::FailedToAddTransaction)?;
        Ok(tx_hash)
    }

    pub fn handle_estimate_fees(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
    ) -> PaymasterResult<Vec<BroadcastedTx>> {
        let mut new_transactions = Vec::with_capacity(transactions.len());

        for tx in transactions {
            let address = match &tx {
                BroadcastedTx::Invoke(BroadcastedInvokeTx(tx)) => tx.sender_address,
                BroadcastedTx::Declare(BroadcastedDeclareTx(tx)) => tx.sender_address,
                _ => continue,
            };

            // Check if the address has already been deployed
            if block_on(self.starknet_api.class_hash_at_address(block_id, address.into())).is_ok() {
                continue;
            }

            // Handles the deployment of a cartridge controller if the estimate fee is requested
            // for a cartridge controller.

            // The controller accounts are created with a specific version of the controller.
            // To ensure address determinism, the controller account must be deployed with the same
            // version, which is included in the calldata retrieved from the Cartridge API.
            match self.get_controller_deploy_tx(address.into(), block_id) {
                Ok(tx) => {
                    todo!("convert from ExecutableTxWithHash to BroadcastedTx");
                    // new_transactions.push(tx);
                }

                Err(Error::ControllerNotFound(..)) => continue,
                Err(err) => panic!("{err}"),
            }
        }

        Ok(new_transactions)
    }

    /// Returns a [`Layer`](tower::Layer) implementation of [`Paymaster`].
    ///
    /// This allows the paymaster to be used as a middleware in Katana RPC stack.
    pub fn layer(self) -> PaymasterLayer<EF> {
        PaymasterLayer { paymaster: self }
    }

    /// Crafts a deploy controller transaction for a cartridge controller.
    ///
    /// Returns None if the provided `controller_address` is not registered in the Cartridge API.
    fn get_controller_deploy_tx(
        &self,
        address: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> PaymasterResult<ExecutableTxWithHash> {
        let result = block_on(self.cartridge_api.get_account_calldata(address))?;
        let account = result.ok_or(Error::ControllerNotFound(address))?;

        // Check if any of the transactions are sent from an address associated with a Cartridge
        // Controller account. If yes, we craft a Controller deployment transaction
        // for each of the unique sender and push it at the beginning of the
        // transaction list so that all the requested transactions are executed against a state
        // with the Controller accounts deployed.

        let pm_address = self.paymaster_address;
        let pm_nonce = match block_on(self.starknet_api.nonce_at(block_id, pm_address)) {
            Ok(nonce) => nonce,
            Err(StarknetApiError::ContractNotFound) => {
                return Err(Error::PaymasterNotFound(pm_address))
            }
            Err(err) => return Err(Error::StarknetApi(err)),
        };

        create_deploy_tx(
            pm_address,
            self.paymaster_key.clone(),
            pm_nonce,
            account.constructor_calldata,
            self.chain_id,
        )
    }
}

impl<EF: ExecutorFactory> Clone for Paymaster<EF> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            chain_id: self.chain_id,
            starknet_api: self.starknet_api.clone(),
            cartridge_api: self.cartridge_api.clone(),
            paymaster_key: self.paymaster_key.clone(),
            paymaster_address: self.paymaster_address,
        }
    }
}

#[derive(Debug)]
pub struct PaymasterLayer<EF: ExecutorFactory> {
    paymaster: Paymaster<EF>,
}

impl<EF: ExecutorFactory> Clone for PaymasterLayer<EF> {
    fn clone(&self) -> Self {
        Self { paymaster: self.paymaster.clone() }
    }
}

impl<S, EF: ExecutorFactory> tower::Layer<S> for PaymasterLayer<EF> {
    type Service = PaymasterService<S, EF>;

    fn layer(&self, service: S) -> Self::Service {
        PaymasterService { service, paymaster: self.paymaster.clone() }
    }
}

#[derive(Debug)]
pub struct PaymasterService<S, EF: ExecutorFactory> {
    service: S,
    paymaster: Paymaster<EF>,
}

impl<S, EF> Clone for PaymasterService<S, EF>
where
    S: Clone,
    EF: ExecutorFactory + Clone,
{
    fn clone(&self) -> Self {
        Self { service: self.service.clone(), paymaster: self.paymaster.clone() }
    }
}

impl<S, EF> PaymasterService<S, EF>
where
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    fn call_on_estimate_fee(&self, request: &mut Request<'_>) {
        let params = request.params();

        let (mut requests, simulation_flags, block_id) = if params.is_object() {
            #[derive(serde::Deserialize)]
            struct ParamsObject<G0, G1, G2> {
                request: G0,
                #[serde(alias = "simulationFlags")]
                simulation_flags: G1,
                #[serde(alias = "blockId")]
                block_id: G2,
            }

            let parsed: ParamsObject<
                Vec<BroadcastedTx>,
                Vec<SimulationFlagForEstimateFee>,
                BlockIdOrTag,
            > = match params.parse() {
                Ok(p) => p,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse_as_object(&e);
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            (parsed.request, parsed.simulation_flags, parsed.block_id)
        } else {
            let mut seq = params.sequence();
            let request: Vec<BroadcastedTx> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "request",
                        "Vec < BroadcastedTx >",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            let simulation_flags: Vec<SimulationFlagForEstimateFee> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "simulation_flags",
                        "Vec < SimulationFlagForEstimateFee >",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            let block_id: BlockIdOrTag = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "block_id",
                        "BlockIdOrTag",
                        &e,
                        false,
                    );
                    // return jsonrpsee::ResponsePayload::error(e);
                    todo!()
                }
            };
            (request, simulation_flags, block_id)
        };

        let new_params = {
            let mut params = jsonrpsee::core::params::ArrayParams::new();

            if let Err(err) = params.insert(requests) {
                jsonrpsee::core::__reexports::panic_fail_serialize("request", err);
            }
            if let Err(err) = params.insert(simulation_flags) {
                jsonrpsee::core::__reexports::panic_fail_serialize("simulation_flags", err);
            }
            if let Err(err) = params.insert(block_id) {
                jsonrpsee::core::__reexports::panic_fail_serialize("block_id", err);
            }

            params
        };

        let params = new_params.to_rpc_params().unwrap();
        let params = params.map(Cow::Owned);
        request.params = params;
    }

    fn call_on_add_outside_execution(&self, request: &Request<'_>) {
        let params = request.params();

        let (controller_address, ..) = if params.is_object() {
            #[derive(serde::Deserialize)]
            struct ParamsObject<G0, G1, G2> {
                address: G0,
                #[serde(alias = "outsideExecution")]
                outside_execution: G1,
                signature: G2,
            }
            let parsed: ParamsObject<ContractAddress, OutsideExecution, Vec<Felt>> =
                match params.parse() {
                    Ok(p) => p,
                    Err(e) => {
                        jsonrpsee::core::__reexports::log_fail_parse_as_object(&e);
                        return;
                    }
                };
            (parsed.address, parsed.outside_execution, parsed.signature)
        } else {
            let mut seq = params.sequence();
            let address: ContractAddress = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "address",
                        "ContractAddress",
                        &e,
                        false,
                    );
                    return;
                }
            };
            let outside_execution: OutsideExecution = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "outside_execution",
                        "OutsideExecution",
                        &e,
                        false,
                    );
                    return;
                }
            };
            let signature: Vec<Felt> = match seq.next() {
                Ok(v) => v,
                Err(e) => {
                    jsonrpsee::core::__reexports::log_fail_parse(
                        "signature",
                        "Vec < Felt >",
                        &e,
                        false,
                    );
                    return;
                }
            };
            (address, outside_execution, signature)
        };

        match self.paymaster.deploy_controller(controller_address) {
            Ok(tx_hash) => {
                trace!(
                    tx_hash = format!("{tx_hash:#x}"),
                    "Controller deploy transaction submitted",
                );
            }
            Err(Error::ControllerNotFound(..)) => {}
            Err(error) => panic!("{error}"),
        }
    }
}

impl<S, EF> middleware::RpcServiceT for PaymasterService<S, EF>
where
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
    EF: ExecutorFactory,
{
    type BatchResponse = S::BatchResponse;
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;

    fn call<'a>(
        &self,
        mut request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        if request.method_name() == "starknet_estimateFee" {
            self.call_on_estimate_fee(&mut request);
            self.service.call(request)
        } else if request.method_name() == "cartridge_addExecuteOutsideTransaction" {
            self.call_on_add_outside_execution(&request);
            self.service.call(request)
        } else {
            self.service.call(request)
        }
    }

    fn batch<'a>(
        &self,
        requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.service.batch(requests)
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.service.notification(n)
    }
}

fn create_deploy_tx(
    deployer: ContractAddress,
    deployer_pk: SigningKey,
    nonce: Nonce,
    constructor_calldata: Vec<Felt>,
    chain_id: ChainId,
) -> PaymasterResult<ExecutableTxWithHash> {
    // Check if any of the transactions are sent from an address associated with a Cartridge
    // Controller account. If yes, we craft a Controller deployment transaction
    // for each of the unique sender and push it at the beginning of the
    // transaction list so that all the requested transactions are executed against a state
    // with the Controller accounts deployed.

    // let pm_address = self.paymaster_address;
    // let pm_nonce = match block_on(self.starknet_api.nonce_at(block_id, pm_address)) {
    //     Ok(nonce) => nonce,
    //     Err(StarknetApiError::ContractNotFound) => Err(Error::PaymasterNotFound(pm_address)),
    //     Err(err) => Err(Error::StarknetApi(err)),
    // };

    let call = Call {
        calldata: constructor_calldata,
        to: DEFAULT_UDC_ADDRESS.into(),
        selector: selector!("deployContract"),
    };

    let mut tx = InvokeTxV3 {
        nonce,
        chain_id,
        tip: 0_u64,
        signature: Vec::new(),
        sender_address: deployer,
        paymaster_data: Vec::new(),
        calldata: encode_calls(vec![call]),
        account_deployment_data: Vec::new(),
        nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
        fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
        resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
    };

    let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

    let signer = LocalWallet::from(deployer_pk);
    let signature = block_on(signer.sign_hash(&tx_hash)).unwrap();
    tx.signature = vec![signature.r, signature.s];

    let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));

    Ok(tx)
}
