use futures::executor::block_on;
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
use katana_rpc_types::broadcasted::BroadcastedTx;
use layer::PaymasterLayer;
use starknet::core::types::Call;
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};

pub mod layer;

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

        let tx = ExecutableTxWithHash::new(tx);
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
                BroadcastedTx::Invoke(tx) => tx.sender_address,
                BroadcastedTx::Declare(tx) => tx.sender_address,
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
                    // todo!("convert from ExecutableTxWithHash to BroadcastedTx");
                    new_transactions.push(BroadcastedTx::from(tx));
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
    ) -> PaymasterResult<ExecutableTx> {
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

fn create_deploy_tx(
    deployer: ContractAddress,
    deployer_pk: SigningKey,
    nonce: Nonce,
    constructor_calldata: Vec<Felt>,
    chain_id: ChainId,
) -> PaymasterResult<ExecutableTx> {
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

    Ok(ExecutableTx::Invoke(InvokeTx::V3(tx)))
}
