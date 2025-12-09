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

use std::collections::HashSet;
use std::iter::once;

use cainome_cairo_serde::CairoSerde;
use katana_core::backend::storage::ProviderRO;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::{TransactionPool, TxPool};
use katana_pool_api::PoolError;
use katana_primitives::chain::ChainId;
use katana_primitives::contract::Nonce;
use katana_primitives::execution::FunctionCall;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::{ContractAddress, Felt};
use katana_provider::ProviderFactory;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi};
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::BlockIdOrTag;
use layer::PaymasterLayer;
use starknet::core::types::Call;
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use starknet_crypto::pedersen_hash;
use tracing::{debug, trace};

pub mod layer;

#[cfg(test)]
mod tests;

use crate::rpc::types::{
    Call as OutsideExecutionCall, OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use crate::utils::encode_calls;
use crate::vrf::{VrfContext, CARTRIDGE_VRF_CLASS_HASH, CARTRIDGE_VRF_SALT};
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

    #[error("VRF error: {0}")]
    Vrf(String),

    #[error("failed to sign with paymaster: {0}")]
    SigningError(String),

    #[error("failed to add deploy controller transaction to the pool: {0}")]
    FailedToAddTransaction(#[from] PoolError),
}

#[derive(Debug)]
pub struct Paymaster<Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    starknet_api: StarknetApi<Pool, PP, PF>,
    cartridge_api: Client,
    pool: TxPool,
    chain_id: ChainId,
    paymaster_key: SigningKey,
    paymaster_address: ContractAddress,
    vrf_ctx: VrfContext,
}

impl<Pool: TransactionPool + 'static, PP: PendingBlockProvider, PF: ProviderFactory>
    Paymaster<Pool, PP, PF>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub fn new(
        starknet_api: StarknetApi<Pool, PP, PF>,
        cartridge_api: Client,
        pool: TxPool,
        chain_id: ChainId,
        paymaster_address: ContractAddress,
        paymaster_key: SigningKey,
        vrf_ctx: VrfContext,
    ) -> Self {
        Self {
            starknet_api,
            cartridge_api,
            pool,
            chain_id,
            paymaster_key,
            paymaster_address,
            vrf_ctx,
        }
    }

    /// Handle the intercept of the 'cartridge_addExecuteOutsideTransaction' end point.
    ///
    /// Modify the provided `outside_execution` and `signature` to include the VRF calls if needed.
    ///
    /// This might submit new transactions to the pool for deploying the Controller account contract
    /// and/or the VRF provider contract.
    pub async fn handle_add_outside_execution(
        &self,
        address: ContractAddress,
        outside_execution: OutsideExecution,
        _signature: Vec<Felt>,
    ) -> PaymasterResult<Option<(OutsideExecution, Vec<Felt>)>> {
        let block_id = BlockIdOrTag::PreConfirmed;
        let mut paymaster_nonce = self.get_paymaster_nonce(block_id).await?;

        // craft a controller deploy tx if needed
        let tx_option = match self.craft_controller_deploy_tx(address, block_id).await {
            Ok(opt) => opt,
            Err(err) => return Err(err),
        };

        if let Some(tx) = tx_option {
            let tx = ExecutableTxWithHash::new(tx);
            let tx_hash =
                self.pool.add_transaction(tx).await.map_err(Error::FailedToAddTransaction)?;

            trace!(
                target: "cartridge",
                controller = %address,
                tx_hash = format!("{tx_hash:#x}"),
                "Controller deploy transaction submitted",
            );

            paymaster_nonce += Nonce::ONE;
        }

        // get VRF calls
        let vrf_calls =
            self.get_vrf_calls(&outside_execution, self.chain_id, &self.vrf_ctx).await?;

        if let Some(vrf_calls) = vrf_calls {
            // deploy VRF provider if not deployed yet
            let class_hash_result = self
                .starknet_api
                .class_hash_at_address(BlockIdOrTag::Latest, self.vrf_ctx.address())
                .await;

            match class_hash_result {
                Err(StarknetApiError::ContractNotFound) => {
                    let (public_key_x, public_key_y) = self.vrf_ctx.get_public_key_xy_felts();
                    let tx = self
                        .craft_vrf_provider_deploy_tx(
                            self.paymaster_address,
                            self.paymaster_key.secret_scalar(),
                            self.chain_id,
                            paymaster_nonce,
                            public_key_x,
                            public_key_y,
                        )
                        .await?;
                    let tx_hash = self
                        .pool
                        .add_transaction(tx)
                        .await
                        .map_err(Error::FailedToAddTransaction)?;

                    trace!(
                        target: "cartridge",
                        vrf_provider = %self.vrf_ctx.address(),
                        tx_hash = format!("{tx_hash:#x}"),
                        "VRF Provider deploy transaction submitted",
                    );

                    paymaster_nonce += Nonce::ONE;
                }
                Err(err) => return Err(Error::StarknetApi(err)),
                Ok(_) => {}
            }

            let (new_outside_execution, new_signature) = self
                .craft_new_outside_execution(paymaster_nonce, outside_execution, &vrf_calls)
                .await?;

            return Ok(Some((new_outside_execution, new_signature)));
        }

        Ok(None)
    }

    /// Handle the intercept of the 'starknet_estimateFee' end point.
    pub async fn handle_estimate_fees(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
    ) -> PaymasterResult<Option<Vec<BroadcastedTx>>> {
        let mut new_transactions = Vec::with_capacity(transactions.len());
        let mut deployed_controllers: HashSet<ContractAddress> = HashSet::new();

        for tx in &transactions {
            let address = match &tx {
                BroadcastedTx::Invoke(tx) => tx.sender_address,
                BroadcastedTx::Declare(tx) => tx.sender_address,
                _ => continue,
            };

            // if the address has already been processed in this txs batch, just ignore the tx
            if deployed_controllers.contains(&address) {
                continue;
            }

            match self.craft_controller_deploy_tx(address, block_id).await {
                Ok(Some(tx)) => {
                    deployed_controllers.insert(address);
                    new_transactions.push(BroadcastedTx::from(tx));

                    trace!(
                        target: "cartridge",
                        controller = %address,
                        "Controller deploy transaction submitted",
                    );
                }
                Ok(None) | Err(Error::ControllerNotFound(..)) => continue,
                Err(err) => return Err(err),
            }
        }

        if new_transactions.is_empty() {
            return Ok(None);
        }

        Ok(Some(new_transactions.into_iter().chain(transactions).collect()))
    }

    /// Returns a [`Layer`](tower::Layer) implementation of [`Paymaster`].
    ///
    /// This allows the paymaster to be used as a middleware in Katana RPC stack.
    pub fn layer(self) -> PaymasterLayer<Pool, PP, PF> {
        PaymasterLayer { paymaster: self }
    }

    /// Crafts a deploy controller transaction for a cartridge controller.
    ///
    /// Returns None if the provided `controller_address` is not registered in the Cartridge API,
    /// or if it has already been deployed.
    async fn craft_controller_deploy_tx(
        &self,
        address: ContractAddress,
        block_id: BlockIdOrTag,
    ) -> PaymasterResult<Option<ExecutableTx>> {
        // if the address is not a controller, just ignore the tx
        let controller_calldata = match self.get_controller_ctor_calldata(address).await? {
            Some(calldata) => calldata,
            None => return Ok(None),
        };

        // Check if the address has already been deployed
        if self.starknet_api.class_hash_at_address(block_id, address).await.is_ok() {
            return Ok(None);
        }

        // Create a Controller deploy transaction against the latest state of the network.
        let paymaster_nonce = self.get_paymaster_nonce(block_id).await?;

        debug!(target: "cartridge", controller = %address, "Crafting controller deploy transaction");

        let tx = create_deploy_tx(
            self.paymaster_address,
            self.paymaster_key.clone(),
            paymaster_nonce,
            controller_calldata,
            self.chain_id,
        )
        .await?;

        Ok(Some(tx))
    }

    /// Get the constructor calldata for a controller account or None if the address is not a
    /// controller.
    async fn get_controller_ctor_calldata(
        &self,
        address: ContractAddress,
    ) -> PaymasterResult<Option<Vec<Felt>>> {
        let result = self.cartridge_api.get_account_calldata(address).await?;
        Ok(result.map(|r| r.constructor_calldata))
    }

    /// Crafts a deploy VRF provider transaction.
    ///
    /// Returns None if the VRF provider has already been deployed.
    async fn craft_vrf_provider_deploy_tx(
        &self,
        paymaster_address: ContractAddress,
        paymaster_private_key: Felt,
        chain_id: ChainId,
        paymaster_nonce: Felt,
        public_key_x: Felt,
        public_key_y: Felt,
    ) -> PaymasterResult<ExecutableTxWithHash> {
        let calldata = vec![
            CARTRIDGE_VRF_CLASS_HASH,
            CARTRIDGE_VRF_SALT,
            // from zero
            Felt::ZERO,
            // Calldata len
            Felt::THREE,
            // owner
            paymaster_address.into(),
            // public key
            public_key_x,
            public_key_y,
        ];

        let call = FunctionCall {
            contract_address: DEFAULT_UDC_ADDRESS,
            entry_point_selector: selector!("deployContract"),
            calldata,
        };

        let mut tx = InvokeTxV3 {
            chain_id,
            tip: 0_u64,
            signature: vec![],
            paymaster_data: vec![],
            account_deployment_data: vec![],
            sender_address: paymaster_address,
            calldata: encode_calls(vec![call]),
            nonce: paymaster_nonce,
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
        };

        let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(paymaster_private_key));
        let signature =
            signer.sign_hash(&tx_hash).await.map_err(|e| Error::SigningError(e.to_string()))?;
        tx.signature = vec![signature.r, signature.s];

        let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));

        Ok(tx)
    }

    /// Get the VRF calls for a given outside execution.
    ///
    /// Returns None if the outside execution does not contain any 'request_random' VRF call
    /// targeting the VRF provider contract.
    async fn get_vrf_calls(
        &self,
        outside_execution: &OutsideExecution,
        chain_id: ChainId,
        vrf_ctx: &VrfContext,
    ) -> PaymasterResult<Option<[Call; 2]>> {
        let calls = match outside_execution {
            OutsideExecution::V2(v2) => &v2.calls,
            OutsideExecution::V3(v3) => &v3.calls,
        };

        if calls.is_empty() {
            return Ok(None);
        }

        // Refer to the module documentation for why this is expected and
        // cartridge documentation for more details:
        // <https://docs.cartridge.gg/slot/vrng#executing-vrng-transactions>.

        let first_call = calls.first().unwrap();

        if first_call.selector != selector!("request_random") || first_call.to != *vrf_ctx.address()
        {
            return Ok(None);
        }

        if first_call.calldata.len() != 3 {
            return Err(Error::Vrf(format!(
                "Invalid calldata for request_random: {:?}",
                first_call.calldata
            )));
        }

        // if request_random targeting the VRF provider is the only call, just ignore it
        // as the generated random value will not be consumed.
        if calls.len() == 1 {
            return Ok(None);
        }

        let caller = first_call.calldata[0];
        let salt_or_nonce_selector = first_call.calldata[1];
        // Salt or nonce being the salt for the `Salt` variant, and the contract address for the
        // `Nonce` variant.
        let salt_or_nonce = first_call.calldata[2];

        let source = if salt_or_nonce_selector == Felt::ZERO {
            let contract_address = salt_or_nonce;
            let state =
                self.starknet_api.state(&BlockIdOrTag::Latest).map_err(Error::StarknetApi)?;

            let key = pedersen_hash(&selector!("VrfProvider_nonces"), &contract_address);
            state
                .storage(vrf_ctx.address(), key)
                .expect("failed to get storage")
                .expect("storage not found")
        } else if salt_or_nonce_selector == Felt::ONE {
            salt_or_nonce
        } else {
            return Err(Error::Vrf(format!(
                "Invalid salt or nonce for VRF request, expecting 0 or 1, got \
                 {salt_or_nonce_selector}"
            )));
        };

        let seed = starknet_crypto::poseidon_hash_many(vec![&source, &caller, &chain_id.id()]);
        let proof = vrf_ctx.stark_vrf(seed).map_err(|e| Error::Vrf(e.to_string()))?;

        println!("source: {:?}", source);
        println!("caller: {:?}", caller);
        println!("chain_id: {:?}", chain_id.id());
        println!("seed: {:?}", seed);

        let submit_random_call = Call {
            to: vrf_ctx.address().into(),
            selector: selector!("submit_random"),
            calldata: vec![
                seed,
                Felt::from_hex_unchecked(&proof.gamma_x),
                Felt::from_hex_unchecked(&proof.gamma_y),
                Felt::from_hex_unchecked(&proof.c),
                Felt::from_hex_unchecked(&proof.s),
                Felt::from_hex_unchecked(&proof.sqrt_ratio),
            ],
        };

        let assert_consumed_call = Call {
            selector: selector!("assert_consumed"),
            to: vrf_ctx.address().into(),
            calldata: vec![seed],
        };

        Ok(Some([submit_random_call, assert_consumed_call]))
    }

    /// Crafts a new outside execution with the VRF calls sandwitched between the original calls.
    ///
    /// Returns the new outside execution and the signature for the new outside execution.
    async fn craft_new_outside_execution(
        &self,
        paymaster_nonce: Nonce,
        outside_execution: OutsideExecution,
        vrf_calls: &[Call; 2],
    ) -> PaymasterResult<(OutsideExecution, Vec<Felt>)> {
        fn pack_calls(
            calls: &[OutsideExecutionCall],
            vrf_calls: &[Call; 2],
        ) -> Vec<OutsideExecutionCall> {
            let [submit_call, assert_call] = vrf_calls;
            once(submit_call.into())
                .chain(calls.iter().cloned())
                .chain(once(assert_call.into()))
                .collect::<Vec<_>>()
        }

        let new_outside_execution = match &outside_execution {
            OutsideExecution::V2(v2) => OutsideExecution::V2(OutsideExecutionV2 {
                caller: self.paymaster_address,
                nonce: paymaster_nonce,
                execute_after: v2.execute_after,
                execute_before: v2.execute_before,
                calls: pack_calls(&v2.calls, vrf_calls),
            }),
            OutsideExecution::V3(v3) => OutsideExecution::V3(OutsideExecutionV3 {
                caller: self.paymaster_address,
                nonce: v3.nonce.copy_with_other_nonce(paymaster_nonce),
                execute_after: v3.execute_after,
                execute_before: v3.execute_before,
                calls: pack_calls(&v3.calls, vrf_calls),
            }),
        };

        let serialized_outside_execution =
            OutsideExecution::cairo_serialize(&new_outside_execution);
        let outside_execution_hash =
            starknet_crypto::poseidon_hash_many(&serialized_outside_execution);

        let signer = LocalWallet::from(self.paymaster_key.clone());
        let new_signature = signer
            .sign_hash(&outside_execution_hash)
            .await
            .map_err(|e| Error::SigningError(e.to_string()))?;

        let new_signature = vec![new_signature.r, new_signature.s];

        Ok((new_outside_execution, new_signature))
    }

    /// Get the nonce of the paymaster account.
    async fn get_paymaster_nonce(&self, block_id: BlockIdOrTag) -> PaymasterResult<Felt> {
        let res: PaymasterResult<Felt> =
            self.starknet_api.nonce_at(block_id, self.paymaster_address).await.map_err(
                |e| match e {
                    StarknetApiError::ContractNotFound => {
                        Error::PaymasterNotFound(self.paymaster_address)
                    }
                    _ => Error::StarknetApi(e),
                },
            );
        res
    }
}

impl<Pool: TransactionPool, PP: PendingBlockProvider, PF: ProviderFactory> Clone
    for Paymaster<Pool, PP, PF>
where
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            chain_id: self.chain_id,
            starknet_api: self.starknet_api.clone(),
            cartridge_api: self.cartridge_api.clone(),
            paymaster_key: self.paymaster_key.clone(),
            paymaster_address: self.paymaster_address,
            vrf_ctx: self.vrf_ctx.clone(),
        }
    }
}

/// Crafts a deploy controller transaction.
///
/// Returns the deploy controller transaction.
async fn create_deploy_tx(
    deployer: ContractAddress,
    deployer_pk: SigningKey,
    nonce: Nonce,
    constructor_calldata: Vec<Felt>,
    chain_id: ChainId,
) -> PaymasterResult<ExecutableTx> {
    let call = FunctionCall {
        contract_address: DEFAULT_UDC_ADDRESS,
        entry_point_selector: selector!("deployContract"),
        calldata: constructor_calldata,
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
    let signature = signer.sign_hash(&tx_hash).await.unwrap();
    tx.signature = vec![signature.r, signature.s];

    Ok(ExecutableTx::Invoke(InvokeTx::V3(tx)))
}
