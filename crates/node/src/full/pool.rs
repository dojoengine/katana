use std::future::Future;

use katana_pool::ordering::TipOrdering;
use katana_pool::pool::Pool;
use katana_pool_api::validation::{ValidationOutcome, ValidationResult, Validator};
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_rpc_types::BroadcastedTx;

pub type FullNodePool =
    Pool<ExecutableTxWithHash, GatewayProxyValidator, TipOrdering<ExecutableTxWithHash>>;

/// This is an implementation of the [`Validator`] trait that proxies incoming transactions to a
/// Starknet sequencer via the gateway endpoint.
///
/// Any transaction validation is performed by the Starknet sequencer.
#[derive(Debug)]
pub struct GatewayProxyValidator {
    gateway_client: katana_gateway::client::Client,
}

impl GatewayProxyValidator {
    pub fn new(gateway_client: katana_gateway::client::Client) -> Self {
        Self { gateway_client }
    }
}

impl Validator for GatewayProxyValidator {
    type Transaction = ExecutableTxWithHash;

    fn validate(
        &self,
        tx: Self::Transaction,
    ) -> impl Future<Output = ValidationResult<Self::Transaction>> + Send {
        let gateway_client = self.gateway_client.clone();

        async move {
            match BroadcastedTx::from(tx.transaction.clone()) {
                BroadcastedTx::Invoke(invoke_tx) => {
                    gateway_client.add_invoke_transaction(invoke_tx).await.unwrap();
                }
                BroadcastedTx::Declare(declare_tx) => {
                    gateway_client.add_declare_transaction(declare_tx).await.unwrap();
                }
                BroadcastedTx::DeployAccount(deploy_account_tx) => {
                    gateway_client.add_deploy_account_transaction(deploy_account_tx).await.unwrap();
                }
            }

            ValidationResult::Ok(ValidationOutcome::Valid(tx))
        }
    }
}
