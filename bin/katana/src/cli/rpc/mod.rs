use std::str::FromStr;

use anyhow::{Context, Result};  
use clap::{Args, Subcommand};
use katana_primitives::block::{BlockIdOrTag, BlockNumber, BlockTag};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use serde_json::Value;
use starknet::core::types::BlockId;

mod client;

use client::RpcClient;

#[derive(Debug, Args)]
pub struct RpcArgs {
    #[command(subcommand)]
    command: RpcCommand,
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            RpcCommand::Block(args) => args.execute().await,
            RpcCommand::Tx(args) => args.execute().await,
            RpcCommand::EstimateFee(args) => args.execute().await,
            RpcCommand::Invoke(args) => args.execute().await,
            RpcCommand::Call(args) => args.execute().await,
        }
    }
}

#[derive(Debug, Subcommand)]
enum RpcCommand {
    #[command(about = "Get block information")]
    Block(BlockArgs),
    
    #[command(about = "Get transaction information")]
    Tx(TxArgs),
    
    #[command(about = "Estimate transaction fees")]
    EstimateFee(EstimateFeeArgs),
    
    #[command(about = "Submit invoke transaction")]
    Invoke(InvokeArgs),
    
    #[command(about = "Call RPC method directly")]
    Call(CallArgs),
}

#[derive(Debug, Args)]
struct BlockArgs {
    #[arg(help = "Block ID (number, hash, 'latest', or 'pending'). Defaults to 'latest'")]
    block_id: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
    
    #[arg(long, help = "Get block with transaction hashes only")]
    tx_hashes: bool,
    
    #[arg(long, help = "Get block with receipts")]
    receipts: bool,
}

impl BlockArgs {
    async fn execute(self) -> Result<()> {
        let client = RpcClient::new(&self.url)?;
        let block_id = parse_block_id(self.block_id.as_deref())?;
        
        let result = if self.receipts {
            client.get_block_with_receipts(block_id).await?
        } else if self.tx_hashes {
            client.get_block_with_tx_hashes(block_id).await?
        } else {
            client.get_block_with_txs(block_id).await?
        };
        
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

#[derive(Debug, Args)]
struct TxArgs {
    #[arg(help = "Transaction hash")]
    tx_hash: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
    
    #[arg(long, help = "Get transaction receipt instead of transaction details")]
    receipt: bool,
    
    #[arg(long, help = "Get transaction status")]
    status: bool,
}

impl TxArgs {
    async fn execute(self) -> Result<()> {
        let client = RpcClient::new(&self.url)?;
        let tx_hash = TxHash::from_str(&self.tx_hash)
            .context("Invalid transaction hash format")?;
        
        let result = if self.receipt {
            client.get_transaction_receipt(tx_hash).await?
        } else if self.status {
            client.get_transaction_status(tx_hash).await?
        } else {
            client.get_transaction_by_hash(tx_hash).await?
        };
        
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

#[derive(Debug, Args)]
struct CallArgs {
    #[arg(help = "RPC method name")]
    method: String,
    
    #[arg(help = "Method arguments (JSON)")]
    args: Vec<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
}

impl CallArgs {
    async fn execute(self) -> Result<()> {
        let client = RpcClient::new(&self.url)?;
        
        let params: Vec<Value> = self.args.iter()
            .map(|arg| serde_json::from_str(arg))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse arguments as JSON")?;
        
        let result = client.call_method(&self.method, params).await?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

#[derive(Debug, Args)]
struct EstimateFeeArgs {
    #[arg(help = "Transaction JSON (as broadcasted transaction)")]
    transaction: String,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
    
    #[arg(long, help = "Block ID for estimation context. Defaults to 'latest'")]
    block_id: Option<String>,
}

impl EstimateFeeArgs {
    async fn execute(self) -> Result<()> {
        let client = RpcClient::new(&self.url)?;
        let block_id = parse_block_id(self.block_id.as_deref())?;
        
        let tx: Value = serde_json::from_str(&self.transaction)
            .context("Failed to parse transaction JSON")?;
        
        let result = client.estimate_fee(vec![tx], block_id).await?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

#[derive(Debug, Args)]
struct InvokeArgs {
    #[arg(long, help = "Account private key (hex string)")]
    private_key: Option<String>,
    
    #[arg(long, help = "Account address (hex string)")]
    address: Option<String>,
    
    #[arg(long, default_value = "http://localhost:5050", help = "RPC endpoint URL")]
    url: String,
    
    #[arg(help = "Contract address to invoke")]
    contract_address: String,
    
    #[arg(help = "Function selector or name")]
    function: String,
    
    #[arg(help = "Function call data (hex values)")]
    calldata: Vec<String>,
}

impl InvokeArgs {
    async fn execute(self) -> Result<()> {
        // For now, just use todo!() as specified in the issue
        // This would require complex account management and transaction signing
        todo!("Account-based transaction submission not yet implemented. Use the 'call' command for direct RPC calls.");
    }
}

fn parse_block_id(block_id: Option<&str>) -> Result<BlockIdOrTag> {
    let block_id = block_id.unwrap_or("latest");
    
    match block_id {
        "latest" => Ok(BlockId::Tag(BlockTag::Latest)),
        "pending" => Ok(BlockId::Tag(BlockTag::Pending)),
        _ => {
            if block_id.starts_with("0x") {
                // Parse as block hash
                let hash = Felt::from_str(block_id)
                    .context("Invalid block hash format")?;
                Ok(BlockId::Hash(hash))
            } else {
                // Parse as block number
                let number = block_id.parse::<BlockNumber>()
                    .context("Invalid block number format")?;
                Ok(BlockId::Number(number))
            }
        }
    }
}