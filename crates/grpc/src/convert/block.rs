//! Block type conversions.

use katana_primitives::block::BlockIdOrTag;
use tonic::Status;

use super::{from_proto_felt, to_proto_felt, to_proto_felts};
use crate::protos::starknet::{
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, GetBlockWithTxsResponse,
    GetStateUpdateResponse,
};
use crate::protos::types::{
    block_id::Identifier, BlockHeader as ProtoBlockHeader, BlockId as ProtoBlockId,
    BlockWithReceipts as ProtoBlockWithReceipts, BlockWithTxHashes as ProtoBlockWithTxHashes,
    BlockWithTxs as ProtoBlockWithTxs, PendingBlockWithReceipts as ProtoPendingBlockWithReceipts,
    PendingBlockWithTxHashes as ProtoPendingBlockWithTxHashes,
    PendingBlockWithTxs as ProtoPendingBlockWithTxs, PendingStateUpdate as ProtoPendingStateUpdate,
    ResourcePrice as ProtoResourcePrice, StateUpdate as ProtoStateUpdate,
};

/// Converts a proto BlockId to a Katana BlockIdOrTag.
pub fn from_proto_block_id(proto: Option<&ProtoBlockId>) -> Result<BlockIdOrTag, Status> {
    let proto = proto.ok_or_else(|| Status::invalid_argument("Missing block_id"))?;

    let identifier =
        proto.identifier.as_ref().ok_or_else(|| Status::invalid_argument("Missing identifier"))?;

    match identifier {
        Identifier::Number(num) => Ok(BlockIdOrTag::Number(*num)),
        Identifier::Hash(hash) => {
            let felt = from_proto_felt(hash)?;
            Ok(BlockIdOrTag::Hash(felt))
        }
        Identifier::Tag(tag) => match tag.to_lowercase().as_str() {
            "latest" => Ok(BlockIdOrTag::Latest),
            "pending" => Ok(BlockIdOrTag::PreConfirmed),
            "l1_accepted" | "l1accepted" => Ok(BlockIdOrTag::L1Accepted),
            _ => Err(Status::invalid_argument(format!("Unknown block tag: {tag}"))),
        },
    }
}

/// Converts a block with tx hashes response from RPC types to proto.
pub fn to_proto_block_with_tx_hashes(
    response: katana_rpc_types::block::GetBlockWithTxHashesResponse,
) -> GetBlockWithTxHashesResponse {
    use katana_rpc_types::block::GetBlockWithTxHashesResponse as RpcResponse;

    match response {
        RpcResponse::Block(block) => GetBlockWithTxHashesResponse {
            result: Some(
                crate::protos::starknet::get_block_with_tx_hashes_response::Result::Block(
                    ProtoBlockWithTxHashes {
                        status: block.status.to_string(),
                        header: Some(to_proto_block_header(&block)),
                        transactions: to_proto_felts(&block.transactions),
                    },
                ),
            ),
        },
        RpcResponse::PreConfirmed(pending) => GetBlockWithTxHashesResponse {
            result: Some(
                crate::protos::starknet::get_block_with_tx_hashes_response::Result::PendingBlock(
                    ProtoPendingBlockWithTxHashes {
                        header: Some(to_proto_pending_block_header(&pending)),
                        transactions: to_proto_felts(&pending.transactions),
                    },
                ),
            ),
        },
    }
}

/// Converts a block with txs response from RPC types to proto.
pub fn to_proto_block_with_txs(
    response: katana_rpc_types::block::MaybePreConfirmedBlock,
) -> GetBlockWithTxsResponse {
    use katana_rpc_types::block::MaybePreConfirmedBlock as RpcResponse;

    match response {
        RpcResponse::Confirmed(block) => GetBlockWithTxsResponse {
            result: Some(crate::protos::starknet::get_block_with_txs_response::Result::Block(
                ProtoBlockWithTxs {
                    status: block.status.to_string(),
                    header: Some(to_proto_block_header_from_full(&block)),
                    transactions: block
                        .transactions
                        .into_iter()
                        .map(super::to_proto_transaction)
                        .collect(),
                },
            )),
        },
        RpcResponse::PreConfirmed(pending) => GetBlockWithTxsResponse {
            result: Some(
                crate::protos::starknet::get_block_with_txs_response::Result::PendingBlock(
                    ProtoPendingBlockWithTxs {
                        header: Some(to_proto_pending_header_from_full(&pending)),
                        transactions: pending
                            .transactions
                            .into_iter()
                            .map(super::to_proto_transaction)
                            .collect(),
                    },
                ),
            ),
        },
    }
}

/// Converts a block with receipts response from RPC types to proto.
pub fn to_proto_block_with_receipts(
    response: katana_rpc_types::block::GetBlockWithReceiptsResponse,
) -> GetBlockWithReceiptsResponse {
    use katana_rpc_types::block::GetBlockWithReceiptsResponse as RpcResponse;

    match response {
        RpcResponse::Block(block) => GetBlockWithReceiptsResponse {
            result: Some(
                crate::protos::starknet::get_block_with_receipts_response::Result::Block(
                    ProtoBlockWithReceipts {
                        status: block.status.to_string(),
                        header: Some(to_proto_block_header_from_with_receipts(&block)),
                        transactions: block
                            .transactions
                            .into_iter()
                            .map(super::to_proto_tx_with_receipt)
                            .collect(),
                    },
                ),
            ),
        },
        RpcResponse::PreConfirmed(pending) => GetBlockWithReceiptsResponse {
            result: Some(
                crate::protos::starknet::get_block_with_receipts_response::Result::PendingBlock(
                    ProtoPendingBlockWithReceipts {
                        header: Some(to_proto_pending_header_from_with_receipts(&pending)),
                        transactions: pending
                            .transactions
                            .into_iter()
                            .map(super::to_proto_tx_with_receipt)
                            .collect(),
                    },
                ),
            ),
        },
    }
}

/// Converts a state update response from RPC types to proto.
pub fn to_proto_state_update(
    response: katana_rpc_types::state_update::StateUpdate,
) -> GetStateUpdateResponse {
    use katana_rpc_types::state_update::StateUpdate as RpcStateUpdate;

    match response {
        RpcStateUpdate::Confirmed(update) => GetStateUpdateResponse {
            result: Some(
                crate::protos::starknet::get_state_update_response::Result::StateUpdate(
                    ProtoStateUpdate {
                        block_hash: Some(to_proto_felt(update.block_hash)),
                        old_root: Some(to_proto_felt(update.old_root)),
                        new_root: Some(to_proto_felt(update.new_root)),
                        state_diff: Some(super::to_proto_state_diff(&update.state_diff)),
                    },
                ),
            ),
        },
        RpcStateUpdate::PreConfirmed(pending) => GetStateUpdateResponse {
            result: Some(
                crate::protos::starknet::get_state_update_response::Result::PendingStateUpdate(
                    ProtoPendingStateUpdate {
                        old_root: Some(to_proto_felt(pending.old_root)),
                        state_diff: Some(super::to_proto_state_diff(&pending.state_diff)),
                    },
                ),
            ),
        },
    }
}

// Helper functions for block header conversion

fn to_proto_block_header(
    block: &katana_rpc_types::block::BlockWithTxHashes,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: Some(to_proto_felt(block.block_hash)),
        parent_hash: Some(to_proto_felt(block.parent_hash)),
        block_number: block.block_number,
        new_root: Some(to_proto_felt(block.new_root)),
        timestamp: block.timestamp,
        sequencer_address: Some(to_proto_felt(block.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&block.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&block.l1_data_gas_price)),
        l1_da_mode: block.l1_da_mode.to_string(),
        starknet_version: block.starknet_version.clone(),
    }
}

fn to_proto_block_header_from_full(
    block: &katana_rpc_types::block::BlockWithTxs,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: Some(to_proto_felt(block.block_hash)),
        parent_hash: Some(to_proto_felt(block.parent_hash)),
        block_number: block.block_number,
        new_root: Some(to_proto_felt(block.new_root)),
        timestamp: block.timestamp,
        sequencer_address: Some(to_proto_felt(block.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&block.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&block.l1_data_gas_price)),
        l1_da_mode: block.l1_da_mode.to_string(),
        starknet_version: block.starknet_version.clone(),
    }
}

fn to_proto_block_header_from_with_receipts(
    block: &katana_rpc_types::block::BlockWithReceipts,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: Some(to_proto_felt(block.block_hash)),
        parent_hash: Some(to_proto_felt(block.parent_hash)),
        block_number: block.block_number,
        new_root: Some(to_proto_felt(block.new_root)),
        timestamp: block.timestamp,
        sequencer_address: Some(to_proto_felt(block.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&block.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&block.l1_data_gas_price)),
        l1_da_mode: block.l1_da_mode.to_string(),
        starknet_version: block.starknet_version.clone(),
    }
}

fn to_proto_pending_block_header(
    pending: &katana_rpc_types::block::PendingBlockWithTxHashes,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: None,
        parent_hash: Some(to_proto_felt(pending.parent_hash)),
        block_number: pending.block_number,
        new_root: None,
        timestamp: pending.timestamp,
        sequencer_address: Some(to_proto_felt(pending.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&pending.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&pending.l1_data_gas_price)),
        l1_da_mode: pending.l1_da_mode.to_string(),
        starknet_version: pending.starknet_version.clone(),
    }
}

fn to_proto_pending_header_from_full(
    pending: &katana_rpc_types::block::PendingBlockWithTxs,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: None,
        parent_hash: Some(to_proto_felt(pending.parent_hash)),
        block_number: pending.block_number,
        new_root: None,
        timestamp: pending.timestamp,
        sequencer_address: Some(to_proto_felt(pending.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&pending.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&pending.l1_data_gas_price)),
        l1_da_mode: pending.l1_da_mode.to_string(),
        starknet_version: pending.starknet_version.clone(),
    }
}

fn to_proto_pending_header_from_with_receipts(
    pending: &katana_rpc_types::block::PendingBlockWithReceipts,
) -> ProtoBlockHeader {
    ProtoBlockHeader {
        block_hash: None,
        parent_hash: Some(to_proto_felt(pending.parent_hash)),
        block_number: pending.block_number,
        new_root: None,
        timestamp: pending.timestamp,
        sequencer_address: Some(to_proto_felt(pending.sequencer_address.into())),
        l1_gas_price: Some(to_proto_resource_price(&pending.l1_gas_price)),
        l1_data_gas_price: Some(to_proto_resource_price(&pending.l1_data_gas_price)),
        l1_da_mode: pending.l1_da_mode.to_string(),
        starknet_version: pending.starknet_version.clone(),
    }
}

fn to_proto_resource_price(price: &katana_rpc_types::block::ResourcePrice) -> ProtoResourcePrice {
    ProtoResourcePrice {
        price_in_wei: Some(to_proto_felt(price.price_in_wei)),
        price_in_fri: Some(to_proto_felt(price.price_in_fri)),
    }
}
