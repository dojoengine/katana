use std::sync::Arc;

use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool_api::validation::{
    Error as ValidationError, InvalidTransactionError, ValidationOutcome, Validator,
};
use katana_primitives::utils::get_contract_address;
use katana_rpc_client::starknet::Client;
use katana_rpc_types::BroadcastedTx;

pub type TxPool = Pool<BroadcastedTx, PoolValidator, FiFo<BroadcastedTx>>;

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
    type Transaction = BroadcastedTx;

    async fn validate(
        &self,
        tx: Self::Transaction,
    ) -> Result<ValidationOutcome<Self::Transaction>, ValidationError> {
        // Forward the transaction to the remote node
        let result = match &tx {
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
                // For client-based validation, any error from the remote node
                // indicates the transaction is invalid
                let error = InvalidTransactionError::ValidationFailure {
                    address: match &tx {
                        BroadcastedTx::Invoke(tx) => tx.sender_address,
                        BroadcastedTx::Declare(tx) => tx.sender_address,
                        BroadcastedTx::DeployAccount(tx) => get_contract_address(
                            tx.contract_address_salt,
                            tx.class_hash,
                            &tx.constructor_calldata,
                            katana_primitives::Felt::ZERO,
                        )
                        .into(),
                    },
                    class_hash: Default::default(),
                    error: err.to_string(),
                };

                Ok(ValidationOutcome::Invalid { tx, error })
            }
        }
    }
}
