use std::sync::Arc;

use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool_api::validation::{
    Error as ValidationError, InvalidTransactionError, ValidationOutcome, Validator,
};
use katana_primitives::utils::get_contract_address;
use katana_rpc_client::starknet::Client;
use katana_rpc_types::{BroadcastedTx, BroadcastedTxWithChainId};

pub type TxPool = Pool<BroadcastedTxWithChainId, PoolValidator, FiFo<BroadcastedTxWithChainId>>;

/// A validator that forwards transactions to a remote Starknet RPC endpoint.
#[derive(Debug, Clone)]
pub struct PoolValidator {
    client: Arc<Client>,
}

impl PoolValidator {
    pub fn new(client: Client) -> Self {
        Self { client: Arc::new(client) }
    }

    pub fn new_shared(client: Arc<Client>) -> Self {
        Self { client }
    }
}

impl Validator for PoolValidator {
    type Transaction = BroadcastedTxWithChainId;

    async fn validate(
        &self,
        tx: Self::Transaction,
    ) -> Result<ValidationOutcome<Self::Transaction>, ValidationError> {
        // Forward the transaction to the remote node
        let result = match &tx.tx {
            BroadcastedTx::Invoke(invoke_tx) => {
                self.client.add_invoke_transaction(invoke_tx.clone()).await.map(|_| ())
            }
            BroadcastedTx::Declare(declare_tx) => {
                self.client.add_declare_transaction(declare_tx.clone()).await.map(|_| ())
            }
            BroadcastedTx::DeployAccount(deploy_account_tx) => self
                .client
                .add_deploy_account_transaction(deploy_account_tx.clone())
                .await
                .map(|_| ()),
        };

        match result {
            Ok(_) => Ok(ValidationOutcome::Valid(tx)),
            Err(err) => {
                let error = InvalidTransactionError::ValidationFailure {
                    address: match &tx.tx {
                        BroadcastedTx::Invoke(tx) => tx.sender_address,
                        BroadcastedTx::Declare(tx) => tx.sender_address,
                        BroadcastedTx::DeployAccount(tx) => tx.contract_address(),
                    },
                    class_hash: Default::default(),
                    error: err.to_string(),
                };

                Ok(ValidationOutcome::Invalid { tx, error })
            }
        }
    }
}
