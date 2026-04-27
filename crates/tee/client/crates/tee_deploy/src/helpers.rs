use std::sync::Arc;

use starknet_rust::core::types::Felt;
use starknet_rust::providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};

pub const POLLING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn watch_tx(
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    transaction_hash: Felt,
    interval: std::time::Duration,
) -> Result<
    starknet_rust::core::types::TransactionReceiptWithBlockInfo,
    Box<dyn std::error::Error + Send + Sync>,
> {
    loop {
        match provider.get_transaction_receipt(transaction_hash).await {
            Ok(receipt) => return Ok(receipt),
            Err(_) => {
                tokio::time::sleep(interval).await;
            }
        }
    }
}
