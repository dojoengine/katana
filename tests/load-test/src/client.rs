use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};
use starknet::core::types::{BlockId, BlockTag, Felt};
use starknet_crypto::{sign, PrivateKey, PublicKey};
use std::sync::Arc;
use tracing::{debug, info};

pub struct StarknetClient {
    client: Client,
    rpc_url: String,
}

#[derive(Clone)]
pub struct Account {
    pub address: Felt,
    pub private_key: PrivateKey,
    pub public_key: PublicKey,
}

impl StarknetClient {
    pub fn new(rpc_url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            client,
            rpc_url: rpc_url.to_string(),
        })
    }

    pub async fn validate_connection(&self) -> Result<()> {
        let response = self.chain_id().await?;
        info!("Connected to Katana - Chain ID: {:#x}", response);
        Ok(())
    }

    pub async fn chain_id(&self) -> Result<Felt> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_chainId",
            "params": [],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        let chain_id_hex = response["result"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid chain_id response"))?;

        Ok(Felt::from_hex(chain_id_hex)?)
    }

    pub async fn get_block_number(&self) -> Result<u64> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_blockNumber",
            "params": [],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        let block_number = response["result"]
            .as_u64()
            .ok_or_else(|| anyhow!("Invalid block_number response"))?;

        Ok(block_number)
    }

    pub async fn get_nonce(&self, address: Felt) -> Result<Felt> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_getNonce",
            "params": [
                {"block_id": "pending"},
                format!("{:#x}", address)
            ],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        let nonce_hex = response["result"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid nonce response"))?;

        Ok(Felt::from_hex(nonce_hex)?)
    }

    pub async fn add_invoke_transaction(&self, transaction: Value) -> Result<Felt> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_addInvokeTransaction",
            "params": [transaction],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        
        if let Some(error) = response.get("error") {
            return Err(anyhow!("Transaction failed: {}", error));
        }

        let tx_hash = response["result"]["transaction_hash"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid transaction response"))?;

        Ok(Felt::from_hex(tx_hash)?)
    }

    pub async fn get_transaction_status(&self, tx_hash: Felt) -> Result<String> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_getTransactionStatus",
            "params": [format!("{:#x}", tx_hash)],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        
        if let Some(error) = response.get("error") {
            return Err(anyhow!("Failed to get transaction status: {}", error));
        }

        let status = response["result"]["execution_status"]
            .as_str()
            .unwrap_or("UNKNOWN")
            .to_string();

        Ok(status)
    }

    pub async fn get_transaction_receipt(&self, tx_hash: Felt) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "starknet_getTransactionReceipt",
            "params": [format!("{:#x}", tx_hash)],
            "id": 1
        });

        let response = self.send_rpc_request(body).await?;
        
        if let Some(error) = response.get("error") {
            return Err(anyhow!("Failed to get transaction receipt: {}", error));
        }

        Ok(response["result"].clone())
    }

    async fn send_rpc_request(&self, body: Value) -> Result<Value> {
        debug!("Sending RPC request: {}", body);
        
        let response = self
            .client
            .post(&self.rpc_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let json_response: Value = response.json().await?;
        debug!("RPC response: {}", json_response);

        Ok(json_response)
    }
}

impl Account {
    pub fn new(private_key_hex: &str, address_hex: &str) -> Result<Self> {
        let private_key = PrivateKey::from_hex(private_key_hex)?;
        let public_key = PublicKey::from_private_key(&private_key);
        let address = Felt::from_hex(address_hex)?;

        Ok(Self {
            address,
            private_key,
            public_key,
        })
    }

    pub fn default_test_account() -> Self {
        // Default Katana test account
        let private_key = PrivateKey::from_hex(
            "0x1800000000300000180000000000030000000000003006001800006600"
        ).unwrap();
        let public_key = PublicKey::from_private_key(&private_key);
        let address = Felt::from_hex(
            "0x517ececd29116499f4a1b64b094da79ba08dfd54a3edaa316134c41f8160973"
        ).unwrap();

        Self {
            address,
            private_key,
            public_key,
        }
    }

    pub fn sign_transaction_hash(&self, tx_hash: Felt) -> Result<(Felt, Felt)> {
        let signature = sign(&self.private_key, &tx_hash)?;
        Ok((signature.r, signature.s))
    }
}
