//! Block type conversions.

use katana_primitives::block::BlockIdOrTag;
use katana_primitives::Felt;
use tonic::Status;

use super::FeltVecExt;
use crate::protos::starknet::{
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, GetBlockWithTxsResponse,
    GetStateUpdateResponse,
};
use crate::protos::types::block_id::Identifier;
use crate::protos::types::{
    BlockHeader as ProtoBlockHeader, BlockId as ProtoBlockId,
    BlockWithReceipts as ProtoBlockWithReceipts, BlockWithTxHashes as ProtoBlockWithTxHashes,
    BlockWithTxs as ProtoBlockWithTxs, Felt as ProtoFelt,
    PendingBlockWithReceipts as ProtoPendingBlockWithReceipts,
    PendingBlockWithTxHashes as ProtoPendingBlockWithTxHashes,
    PendingBlockWithTxs as ProtoPendingBlockWithTxs, PendingStateUpdate as ProtoPendingStateUpdate,
    ResourcePrice as ProtoResourcePrice, StateUpdate as ProtoStateUpdate,
};

/// Convert proto BlockId to Katana BlockIdOrTag.
impl TryFrom<&ProtoBlockId> for BlockIdOrTag {
    type Error = Status;

    fn try_from(proto: &ProtoBlockId) -> Result<Self, Self::Error> {
        let identifier = proto
            .identifier
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing identifier"))?;

        match identifier {
            Identifier::Number(num) => Ok(BlockIdOrTag::Number(*num)),
            Identifier::Hash(hash) => {
                let felt = Felt::try_from(hash)?;
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
}

/// Helper to convert Option<&ProtoBlockId> to BlockIdOrTag.
pub fn block_id_from_proto(proto: Option<&ProtoBlockId>) -> Result<BlockIdOrTag, Status> {
    let proto = proto.ok_or_else(|| Status::invalid_argument("Missing block_id"))?;
    BlockIdOrTag::try_from(proto)
}

/// Convert block with tx hashes response to proto.
impl From<katana_rpc_types::block::GetBlockWithTxHashesResponse> for GetBlockWithTxHashesResponse {
    fn from(response: katana_rpc_types::block::GetBlockWithTxHashesResponse) -> Self {
        use katana_rpc_types::block::GetBlockWithTxHashesResponse as RpcResponse;

        match response {
            RpcResponse::Block(block) => GetBlockWithTxHashesResponse {
                result: Some(
                    crate::protos::starknet::get_block_with_tx_hashes_response::Result::Block(
                        ProtoBlockWithTxHashes {
                            status: block.status.to_string(),
                            header: Some(ProtoBlockHeader::from(&block)),
                            transactions: block.transactions.to_proto_felts(),
                        },
                    ),
                ),
            },
            RpcResponse::PreConfirmed(pending) => GetBlockWithTxHashesResponse {
                result: Some(
                    crate::protos::starknet::get_block_with_tx_hashes_response::Result::PendingBlock(
                        ProtoPendingBlockWithTxHashes {
                            header: Some(ProtoBlockHeader::from(&pending)),
                            transactions: pending.transactions.to_proto_felts(),
                        },
                    ),
                ),
            },
        }
    }
}

/// Convert block with txs response to proto.
impl From<katana_rpc_types::block::MaybePreConfirmedBlock> for GetBlockWithTxsResponse {
    fn from(response: katana_rpc_types::block::MaybePreConfirmedBlock) -> Self {
        use katana_rpc_types::block::MaybePreConfirmedBlock as RpcResponse;

        use crate::protos::types::Transaction as ProtoTx;

        match response {
            RpcResponse::Confirmed(block) => GetBlockWithTxsResponse {
                result: Some(crate::protos::starknet::get_block_with_txs_response::Result::Block(
                    ProtoBlockWithTxs {
                        status: block.status.to_string(),
                        header: Some(ProtoBlockHeader::from(&block)),
                        transactions: block.transactions.into_iter().map(ProtoTx::from).collect(),
                    },
                )),
            },
            RpcResponse::PreConfirmed(pending) => GetBlockWithTxsResponse {
                result: Some(
                    crate::protos::starknet::get_block_with_txs_response::Result::PendingBlock(
                        ProtoPendingBlockWithTxs {
                            header: Some(ProtoBlockHeader::from(&pending)),
                            transactions: pending
                                .transactions
                                .into_iter()
                                .map(ProtoTx::from)
                                .collect(),
                        },
                    ),
                ),
            },
        }
    }
}

/// Convert block with receipts response to proto.
impl From<katana_rpc_types::block::GetBlockWithReceiptsResponse> for GetBlockWithReceiptsResponse {
    fn from(response: katana_rpc_types::block::GetBlockWithReceiptsResponse) -> Self {
        use katana_rpc_types::block::GetBlockWithReceiptsResponse as RpcResponse;

        use crate::protos::types::TransactionWithReceipt;

        match response {
            RpcResponse::Block(block) => GetBlockWithReceiptsResponse {
                result: Some(
                    crate::protos::starknet::get_block_with_receipts_response::Result::Block(
                        ProtoBlockWithReceipts {
                            status: block.status.to_string(),
                            header: Some(ProtoBlockHeader::from(&block)),
                            transactions: block
                                .transactions
                                .into_iter()
                                .map(TransactionWithReceipt::from)
                                .collect(),
                        },
                    ),
                ),
            },
            RpcResponse::PreConfirmed(pending) => GetBlockWithReceiptsResponse {
                result: Some(
                    crate::protos::starknet::get_block_with_receipts_response::Result::PendingBlock(
                        ProtoPendingBlockWithReceipts {
                            header: Some(ProtoBlockHeader::from(&pending)),
                            transactions: pending
                                .transactions
                                .into_iter()
                                .map(TransactionWithReceipt::from)
                                .collect(),
                        },
                    ),
                ),
            },
        }
    }
}

/// Convert state update response to proto.
impl From<katana_rpc_types::state_update::StateUpdate> for GetStateUpdateResponse {
    fn from(response: katana_rpc_types::state_update::StateUpdate) -> Self {
        use katana_rpc_types::state_update::StateUpdate as RpcStateUpdate;

        use crate::protos::types::StateDiff as ProtoStateDiff;

        match response {
            RpcStateUpdate::Confirmed(update) => GetStateUpdateResponse {
                result: Some(
                    crate::protos::starknet::get_state_update_response::Result::StateUpdate(
                        ProtoStateUpdate {
                            block_hash: Some(update.block_hash.into()),
                            old_root: Some(update.old_root.into()),
                            new_root: Some(update.new_root.into()),
                            state_diff: Some(ProtoStateDiff::from(&update.state_diff)),
                        },
                    ),
                ),
            },
            RpcStateUpdate::PreConfirmed(pending) => GetStateUpdateResponse {
                result: Some(
                    crate::protos::starknet::get_state_update_response::Result::PendingStateUpdate(
                        ProtoPendingStateUpdate {
                            old_root: Some(pending.old_root.into()),
                            state_diff: Some(ProtoStateDiff::from(&pending.state_diff)),
                        },
                    ),
                ),
            },
        }
    }
}

// Block header conversions

impl From<&katana_rpc_types::block::BlockWithTxHashes> for ProtoBlockHeader {
    fn from(block: &katana_rpc_types::block::BlockWithTxHashes) -> Self {
        ProtoBlockHeader {
            block_hash: Some(block.block_hash.into()),
            parent_hash: Some(block.parent_hash.into()),
            block_number: block.block_number,
            new_root: Some(block.new_root.into()),
            timestamp: block.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(block.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&block.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&block.l1_data_gas_price)),
            l1_da_mode: block.l1_da_mode.to_string(),
            starknet_version: block.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::BlockWithTxs> for ProtoBlockHeader {
    fn from(block: &katana_rpc_types::block::BlockWithTxs) -> Self {
        ProtoBlockHeader {
            block_hash: Some(block.block_hash.into()),
            parent_hash: Some(block.parent_hash.into()),
            block_number: block.block_number,
            new_root: Some(block.new_root.into()),
            timestamp: block.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(block.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&block.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&block.l1_data_gas_price)),
            l1_da_mode: block.l1_da_mode.to_string(),
            starknet_version: block.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::BlockWithReceipts> for ProtoBlockHeader {
    fn from(block: &katana_rpc_types::block::BlockWithReceipts) -> Self {
        ProtoBlockHeader {
            block_hash: Some(block.block_hash.into()),
            parent_hash: Some(block.parent_hash.into()),
            block_number: block.block_number,
            new_root: Some(block.new_root.into()),
            timestamp: block.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(block.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&block.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&block.l1_data_gas_price)),
            l1_da_mode: block.l1_da_mode.to_string(),
            starknet_version: block.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PendingBlockWithTxHashes> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PendingBlockWithTxHashes) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: Some(pending.parent_hash.into()),
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l1_da_mode: pending.l1_da_mode.to_string(),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PendingBlockWithTxs> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PendingBlockWithTxs) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: Some(pending.parent_hash.into()),
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l1_da_mode: pending.l1_da_mode.to_string(),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PendingBlockWithReceipts> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PendingBlockWithReceipts) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: Some(pending.parent_hash.into()),
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l1_da_mode: pending.l1_da_mode.to_string(),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::ResourcePrice> for ProtoResourcePrice {
    fn from(price: &katana_rpc_types::block::ResourcePrice) -> Self {
        ProtoResourcePrice {
            price_in_wei: Some(price.price_in_wei.into()),
            price_in_fri: Some(price.price_in_fri.into()),
        }
    }
}
