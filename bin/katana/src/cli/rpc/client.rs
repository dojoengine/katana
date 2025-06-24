use anyhow::{Context, Result};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::transaction::TxHash;
use reqwest::Client;
use serde_json::{json, Value};
use url::Url;

pub struct RpcClient {
    client: Client,
    url: Url,
}

impl RpcClient {
    pub fn new(url: &str) -> Result<Self> {
        let url = Url::parse(url).context("Invalid URL format")?;
        let client = Client::new();
        
        Ok(Self { client, url })
    }
    
    pub async fn get_block_with_tx_hashes(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.call_rpc("starknet_getBlockWithTxHashes", vec![serde_json::to_value(block_id)?])
            .await
    }
    
    pub async fn get_block_with_txs(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.call_rpc("starknet_getBlockWithTxs", vec![serde_json::to_value(block_id)?])
            .await
    }
    
    pub async fn get_block_with_receipts(&self, block_id: BlockIdOrTag) -> Result<Value> {
        self.call_rpc("starknet_getBlockWithReceipts", vec![serde_json::to_value(block_id)?])
            .await
    }
    
    pub async fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Result<Value> {
        self.call_rpc("starknet_getTransactionByHash", vec![serde_json::to_value(tx_hash)?])
            .await
    }
    
    pub async fn get_transaction_receipt(&self, tx_hash: TxHash) -> Result<Value> {
        self.call_rpc("starknet_getTransactionReceipt", vec![serde_json::to_value(tx_hash)?])
            .await
    }
    
    pub async fn get_transaction_status(&self, tx_hash: TxHash) -> Result<Value> {
        self.call_rpc("starknet_getTransactionStatus", vec![serde_json::to_value(tx_hash)?])
            .await
    }
    
    pub async fn estimate_fee(&self, transactions: Vec<Value>, block_id: BlockIdOrTag) -> Result<Value> {
        let simulation_flags: Vec<Value> = vec![]; // Empty simulation flags
        self.call_rpc(
            "starknet_estimateFee", 
            vec![
                serde_json::to_value(transactions)?,
                serde_json::to_value(simulation_flags)?,
                serde_json::to_value(block_id)?
            ]
        ).await
    }
    
    pub async fn call_method(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        // Add starknet_ prefix if not present
        let method = if method.starts_with("starknet_") {
            method.to_string()
        } else {
            format!("starknet_{}", method)
        };
        
        self.call_rpc(&method, params).await
    }
    
    async fn call_rpc(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        
        let response = self.client
            .post(self.url.clone())
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to send RPC request")?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
        }
        
        let response_body: Value = response.json().await
            .context("Failed to parse JSON response")?;
        
        if let Some(error) = response_body.get("error") {
            return Err(anyhow::anyhow!("RPC error: {}", error));
        }
        
        response_body.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Missing result field in RPC response"))
    }
}