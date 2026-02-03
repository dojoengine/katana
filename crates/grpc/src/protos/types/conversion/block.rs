//! Block type conversions.

use katana_primitives::block::{BlockIdOrTag, FinalityStatus};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::Felt;
use tonic::Status;

use super::FeltVecExt;
use crate::protos::common::Felt as ProtoFelt;
use crate::protos::starknet::{
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, GetBlockWithTxsResponse,
    GetStateUpdateResponse,
};
use crate::protos::types::block_id::Identifier;
use crate::protos::types::{
    BlockHeader as ProtoBlockHeader, BlockTag as ProtoBlockTag,
    BlockWithReceipts as ProtoBlockWithReceipts, BlockWithTxHashes as ProtoBlockWithTxHashes,
    BlockWithTxs as ProtoBlockWithTxs, FinalityStatus as ProtoFinalityStatus,
    L1DataAvailabilityMode as ProtoL1DataAvailabilityMode,
    PendingBlockWithReceipts as ProtoPendingBlockWithReceipts,
    PendingBlockWithTxHashes as ProtoPendingBlockWithTxHashes,
    PendingBlockWithTxs as ProtoPendingBlockWithTxs, PendingStateUpdate as ProtoPendingStateUpdate,
    ResourcePrice as ProtoResourcePrice, StateUpdate as ProtoStateUpdate,
};

/// Convert FinalityStatus to proto enum.
fn finality_status_to_proto(status: FinalityStatus) -> i32 {
    match status {
        FinalityStatus::AcceptedOnL2 => ProtoFinalityStatus::AcceptedOnL2 as i32,
        FinalityStatus::AcceptedOnL1 => ProtoFinalityStatus::AcceptedOnL1 as i32,
        FinalityStatus::PreConfirmed => ProtoFinalityStatus::PreConfirmed as i32,
    }
}

/// Convert L1DataAvailabilityMode to proto enum.
fn l1_da_mode_to_proto(mode: L1DataAvailabilityMode) -> i32 {
    match mode {
        L1DataAvailabilityMode::Blob => ProtoL1DataAvailabilityMode::Blob as i32,
        L1DataAvailabilityMode::Calldata => ProtoL1DataAvailabilityMode::Calldata as i32,
    }
}

impl TryFrom<crate::proto::BlockId> for katana_primitives::block::BlockIdOrTag {
    type Error = Status;

    fn try_from(proto: crate::proto::BlockId) -> Result<Self, Self::Error> {
        let identifier = proto
            .identifier
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Missing identifier"))?;

        match identifier {
            Identifier::Number(num) => Ok(BlockIdOrTag::Number(*num)),
            Identifier::Hash(hash) => Ok(BlockIdOrTag::Hash(Felt::try_from(hash)?)),
            Identifier::Tag(tag) => {
                let tag = ProtoBlockTag::try_from(*tag)
                    .map_err(|_| Status::invalid_argument(format!("Unknown block tag: {tag}")))?;

                match tag {
                    ProtoBlockTag::Latest => Ok(BlockIdOrTag::Latest),
                    ProtoBlockTag::L1Accepted => Ok(BlockIdOrTag::L1Accepted),
                    ProtoBlockTag::PreConfirmed => Ok(BlockIdOrTag::PreConfirmed),
                }
            }
        }
    }
}

/// Helper to convert Option<&ProtoBlockId> to BlockIdOrTag.
#[allow(clippy::result_large_err)]
pub fn block_id_from_proto(proto: Option<crate::proto::BlockId>) -> Result<BlockIdOrTag, Status> {
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
                            status: finality_status_to_proto(block.status),
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
                        status: finality_status_to_proto(block.status),
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
                            status: finality_status_to_proto(block.status),
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
                            old_root: pending.old_root.map(|f| f.into()),
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
            l2_gas_price: Some(ProtoResourcePrice::from(&block.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(block.l1_da_mode),
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
            l2_gas_price: Some(ProtoResourcePrice::from(&block.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(block.l1_da_mode),
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
            l2_gas_price: Some(ProtoResourcePrice::from(&block.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(block.l1_da_mode),
            starknet_version: block.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PreConfirmedBlockWithTxHashes> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PreConfirmedBlockWithTxHashes) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: None, // PreConfirmed blocks don't have parent_hash
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l2_gas_price: Some(ProtoResourcePrice::from(&pending.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(pending.l1_da_mode),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PreConfirmedBlockWithTxs> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PreConfirmedBlockWithTxs) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: None, // PreConfirmed blocks don't have parent_hash
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l2_gas_price: Some(ProtoResourcePrice::from(&pending.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(pending.l1_da_mode),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&katana_rpc_types::block::PreConfirmedBlockWithReceipts> for ProtoBlockHeader {
    fn from(pending: &katana_rpc_types::block::PreConfirmedBlockWithReceipts) -> Self {
        ProtoBlockHeader {
            block_hash: None,
            parent_hash: None, // PreConfirmed blocks don't have parent_hash
            block_number: pending.block_number,
            new_root: None,
            timestamp: pending.timestamp,
            sequencer_address: Some(ProtoFelt::from(Felt::from(pending.sequencer_address))),
            l1_gas_price: Some(ProtoResourcePrice::from(&pending.l1_gas_price)),
            l1_data_gas_price: Some(ProtoResourcePrice::from(&pending.l1_data_gas_price)),
            l2_gas_price: Some(ProtoResourcePrice::from(&pending.l2_gas_price)),
            l1_da_mode: l1_da_mode_to_proto(pending.l1_da_mode),
            starknet_version: pending.starknet_version.clone(),
        }
    }
}

impl From<&starknet::core::types::ResourcePrice> for ProtoResourcePrice {
    fn from(price: &starknet::core::types::ResourcePrice) -> Self {
        ProtoResourcePrice {
            price_in_wei: Some(price.price_in_wei.into()),
            price_in_fri: Some(price.price_in_fri.into()),
        }
    }
}
