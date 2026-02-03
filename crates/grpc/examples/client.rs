//! Example gRPC client for testing against a running Katana node.
//!
//! Run with: `cargo run -p katana-grpc --example grpc-client`
//!
//! Make sure a Katana node is running with gRPC enabled:
//! `katana --grpc`

use std::env;

use katana_grpc::proto::{
    BlockHashAndNumberRequest, BlockId, BlockNumberRequest, ChainIdRequest, Felt, GetBlockRequest,
    GetNonceRequest, GetStorageAtRequest, SpecVersionRequest, SyncingRequest,
    TraceBlockTransactionsRequest,
};
use katana_grpc::GrpcClient;
use tonic::Request;

fn felt_to_hex(felt: &Felt) -> String {
    format!("0x{}", hex::encode(&felt.value))
}

fn block_id_latest() -> Option<BlockId> {
    Some(BlockId {
        identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
            katana_grpc::proto::types::BlockTag::Latest as i32,
        )),
    })
}

fn zero_felt() -> Option<Felt> {
    Some(Felt { value: vec![0u8; 32] })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("GRPC_ENDPOINT").unwrap_or_else(|_| "http://localhost:5051".into());

    println!("Connecting to gRPC server at {endpoint}...");

    let mut client = GrpcClient::connect(&endpoint).await?;

    println!("Connected!\n");

    // Get spec version
    println!("=== Spec Version ===");
    let response = client.spec_version(Request::new(SpecVersionRequest {})).await?;
    println!("Version: {}\n", response.into_inner().version);

    // Get chain ID
    println!("=== Chain ID ===");
    let response = client.chain_id(Request::new(ChainIdRequest {})).await?;
    println!("Chain ID: {}\n", response.into_inner().chain_id);

    // Get syncing status
    println!("=== Syncing Status ===");
    let response = client.syncing(Request::new(SyncingRequest {})).await?;
    let syncing = response.into_inner();
    match syncing.result {
        Some(katana_grpc::proto::syncing_response::Result::NotSyncing(val)) => {
            println!("Syncing: {}\n", !val);
        }
        Some(katana_grpc::proto::syncing_response::Result::Status(status)) => {
            println!("Syncing status: {:?}\n", status);
        }
        None => println!("Unknown syncing status\n"),
    }

    // Get block number
    println!("=== Block Number ===");
    let response = client.block_number(Request::new(BlockNumberRequest {})).await?;
    let block_number = response.into_inner().block_number;
    println!("Latest block number: {block_number}\n");

    // Get block hash and number
    println!("=== Block Hash and Number ===");
    let response = client.block_hash_and_number(Request::new(BlockHashAndNumberRequest {})).await?;
    let result = response.into_inner();
    let block_hash = result.block_hash.as_ref().map(felt_to_hex).unwrap_or_default();
    println!("Block hash: {}", block_hash);
    println!("Block number: {}\n", result.block_number);

    // Get block with transaction hashes (latest)
    println!("=== Latest Block (with tx hashes) ===");
    let response = client
        .get_block_with_tx_hashes(Request::new(GetBlockRequest { block_id: block_id_latest() }))
        .await?;
    let block_result = response.into_inner();
    match block_result.result {
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::Block(block)) => {
            if let Some(header) = block.header {
                println!("Block number: {}", header.block_number);
                println!("Timestamp: {}", header.timestamp);
                println!("Transaction count: {}", block.transactions.len());
            }
        }
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::PendingBlock(
            pending,
        )) => {
            if let Some(header) = pending.header {
                println!("Pending block");
                println!("Timestamp: {}", header.timestamp);
                println!("Transaction count: {}", pending.transactions.len());
            }
        }
        None => println!("No block data"),
    }
    println!();

    // Example: Get nonce for an address (will fail if address doesn't exist)
    println!("=== Get Nonce (example - zero address) ===");
    let nonce_result = client
        .get_nonce(Request::new(GetNonceRequest {
            block_id: block_id_latest(),
            contract_address: zero_felt(),
        }))
        .await;
    match nonce_result {
        Ok(response) => {
            let nonce = response.into_inner().nonce.as_ref().map(felt_to_hex).unwrap_or_default();
            println!("Nonce: {nonce}");
        }
        Err(status) => {
            println!("Error getting nonce: {}", status.message());
        }
    }
    println!();

    // Example: Get storage at an address (will fail if contract doesn't exist)
    println!("=== Get Storage (example - zero address) ===");
    let storage_result = client
        .get_storage_at(Request::new(GetStorageAtRequest {
            block_id: block_id_latest(),
            contract_address: zero_felt(),
            key: zero_felt(),
        }))
        .await;
    match storage_result {
        Ok(response) => {
            let value = response.into_inner().value.as_ref().map(felt_to_hex).unwrap_or_default();
            println!("Storage value: {value}");
        }
        Err(status) => {
            println!("Error getting storage: {}", status.message());
        }
    }

    let trace_result = client
        .trace_block_transactions(Request::new(TraceBlockTransactionsRequest {
            block_id: block_id_latest(),
        }))
        .await;
    match trace_result {
        Ok(response) => {
            let value = response.into_inner().traces;
            println!("Traces: {value:?}");
        }
        Err(status) => {
            println!("Error getting storage: {}", status.message());
        }
    }

    println!("\nDone!");

    Ok(())
}
