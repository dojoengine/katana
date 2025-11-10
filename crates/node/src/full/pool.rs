use std::future::Future;

use katana_pool::ordering::TipOrdering;
use katana_pool::pool::Pool;
use katana_pool_api::validation::{
    Error as ValidationError, ValidationOutcome, ValidationResult, Validator,
};
use katana_rpc_types::{BroadcastedTx, BroadcastedTxWithChainId};

pub type FullNodePool =
    Pool<BroadcastedTxWithChainId, GatewayProxyValidator, TipOrdering<BroadcastedTxWithChainId>>;

/// This is an implementation of the [`Validator`] trait that proxies incoming transactions to a
/// Starknet sequencer via the gateway endpoint.
///
/// Any transaction validation is performed by the Starknet sequencer.
#[derive(Debug)]
pub struct GatewayProxyValidator {
    gateway_client: katana_gateway_client::Client,
}

impl GatewayProxyValidator {
    pub fn new(gateway_client: katana_gateway_client::Client) -> Self {
        Self { gateway_client }
    }
}

impl Validator for GatewayProxyValidator {
    type Transaction = BroadcastedTxWithChainId;

    fn validate(
        &self,
        tx: Self::Transaction,
    ) -> impl Future<Output = ValidationResult<Self::Transaction>> + Send {
        let gateway_client = self.gateway_client.clone();

        async move {
            let hash = tx.calculate_hash();

            let result = match tx.tx.clone() {
                BroadcastedTx::Invoke(inner_tx) => {
                    gateway_client.add_invoke_transaction(inner_tx.into()).await.map(|_| ())
                }
                BroadcastedTx::Declare(inner_tx) => {
                    gateway_client.add_declare_transaction(inner_tx.into()).await.map(|_| ())
                }
                BroadcastedTx::DeployAccount(inner_tx) => {
                    gateway_client.add_deploy_account_transaction(inner_tx.into()).await.map(|_| ())
                }
            };

            match result {
                Ok(_) => ValidationResult::Ok(ValidationOutcome::Valid(tx)),
                Err(e) => ValidationResult::Err(ValidationError::new(hash, Box::new(e))),
            }
        }
    }
}
