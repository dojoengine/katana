use std::collections::BTreeSet;

use crate::types::{
    BlockStatus, ConfirmedStateUpdate, ConfirmedTransaction, DataAvailabilityMode, DeclareTx,
    DeclareTxV3, DeclaredContract, DeployAccountTx, DeployAccountTxV1, DeployAccountTxV3,
    DeployedContract, ExecutionResources, ExecutionStatus, InvokeTx, InvokeTxV3, L1HandlerTx,
    PreConfirmedStateUpdate, ReceiptBody, StateDiff, StateUpdate, StorageDiff, TypedTransaction,
};

impl From<katana_rpc_types::StateUpdate> for StateUpdate {
    fn from(value: katana_rpc_types::StateUpdate) -> Self {
        match value {
            katana_rpc_types::StateUpdate::Update(update) => StateUpdate::Confirmed(update.into()),
            katana_rpc_types::StateUpdate::PreConfirmed(pre_confirmed) => {
                StateUpdate::PreConfirmed(pre_confirmed.into())
            }
        }
    }
}

// Conversions from katana_primitives to feeder gateway types

impl From<katana_primitives::transaction::TxWithHash> for ConfirmedTransaction {
    fn from(tx: katana_primitives::transaction::TxWithHash) -> Self {
        Self { transaction_hash: tx.hash, transaction: tx.transaction.into() }
    }
}

impl From<katana_primitives::transaction::Tx> for TypedTransaction {
    fn from(tx: katana_primitives::transaction::Tx) -> Self {
        match tx {
            katana_primitives::transaction::Tx::Invoke(tx) => {
                TypedTransaction::InvokeFunction(tx.into())
            }
            katana_primitives::transaction::Tx::Declare(tx) => TypedTransaction::Declare(tx.into()),
            katana_primitives::transaction::Tx::L1Handler(tx) => {
                TypedTransaction::L1Handler(tx.into())
            }
            katana_primitives::transaction::Tx::DeployAccount(tx) => {
                TypedTransaction::DeployAccount(tx.into())
            }
            katana_primitives::transaction::Tx::Deploy(tx) => TypedTransaction::Deploy(tx),
        }
    }
}

impl From<katana_primitives::transaction::InvokeTx> for InvokeTx {
    fn from(tx: katana_primitives::transaction::InvokeTx) -> Self {
        match tx {
            katana_primitives::transaction::InvokeTx::V0(tx) => {
                InvokeTx::V0(katana_rpc_types::RpcInvokeTxV0 {
                    contract_address: tx.contract_address,
                    entry_point_selector: tx.entry_point_selector,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee,
                })
            }
            katana_primitives::transaction::InvokeTx::V1(tx) => {
                InvokeTx::V1(katana_rpc_types::RpcInvokeTxV1 {
                    sender_address: tx.sender_address,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee,
                    nonce: tx.nonce,
                })
            }
            katana_primitives::transaction::InvokeTx::V3(tx) => InvokeTx::V3(InvokeTxV3 {
                sender_address: tx.sender_address,
                nonce: tx.nonce,
                calldata: tx.calldata,
                signature: tx.signature,
                resource_bounds: tx.resource_bounds,
                tip: tx.tip.into(),
                paymaster_data: tx.paymaster_data,
                account_deployment_data: tx.account_deployment_data,
                nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
                fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            }),
        }
    }
}

impl From<katana_primitives::transaction::DeclareTx> for DeclareTx {
    fn from(tx: katana_primitives::transaction::DeclareTx) -> Self {
        match tx {
            katana_primitives::transaction::DeclareTx::V0(tx) => {
                DeclareTx::V0(katana_rpc_types::RpcDeclareTxV0 {
                    sender_address: tx.sender_address,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee,
                })
            }
            katana_primitives::transaction::DeclareTx::V1(tx) => {
                DeclareTx::V1(katana_rpc_types::RpcDeclareTxV1 {
                    sender_address: tx.sender_address,
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee,
                })
            }
            katana_primitives::transaction::DeclareTx::V2(tx) => {
                DeclareTx::V2(katana_rpc_types::RpcDeclareTxV2 {
                    sender_address: tx.sender_address,
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    compiled_class_hash: tx.compiled_class_hash,
                    max_fee: tx.max_fee,
                })
            }
            katana_primitives::transaction::DeclareTx::V3(tx) => DeclareTx::V3(DeclareTxV3 {
                sender_address: tx.sender_address,
                nonce: tx.nonce,
                signature: tx.signature,
                class_hash: tx.class_hash,
                compiled_class_hash: tx.compiled_class_hash,
                resource_bounds: tx.resource_bounds,
                tip: tx.tip.into(),
                paymaster_data: tx.paymaster_data,
                account_deployment_data: tx.account_deployment_data,
                nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
                fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            }),
        }
    }
}

impl From<katana_primitives::transaction::DeployAccountTx> for DeployAccountTx {
    fn from(tx: katana_primitives::transaction::DeployAccountTx) -> Self {
        match tx {
            katana_primitives::transaction::DeployAccountTx::V1(tx) => {
                DeployAccountTx::V1(DeployAccountTxV1 {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    contract_address: Some(tx.contract_address),
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata,
                    max_fee: tx.max_fee,
                })
            }
            katana_primitives::transaction::DeployAccountTx::V3(tx) => {
                DeployAccountTx::V3(DeployAccountTxV3 {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    contract_address: Some(tx.contract_address),
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata,
                    resource_bounds: tx.resource_bounds,
                    tip: tx.tip.into(),
                    paymaster_data: tx.paymaster_data,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
                    fee_data_availability_mode: tx.fee_data_availability_mode.into(),
                })
            }
        }
    }
}

impl From<katana_primitives::transaction::L1HandlerTx> for L1HandlerTx {
    fn from(tx: katana_primitives::transaction::L1HandlerTx) -> Self {
        Self {
            nonce: Some(tx.nonce),
            version: tx.version,
            calldata: tx.calldata,
            contract_address: tx.contract_address,
            entry_point_selector: tx.entry_point_selector,
        }
    }
}

impl From<katana_primitives::da::DataAvailabilityMode> for DataAvailabilityMode {
    fn from(mode: katana_primitives::da::DataAvailabilityMode) -> Self {
        match mode {
            katana_primitives::da::DataAvailabilityMode::L1 => DataAvailabilityMode::L1,
            katana_primitives::da::DataAvailabilityMode::L2 => DataAvailabilityMode::L2,
        }
    }
}

impl From<katana_primitives::receipt::Receipt> for ReceiptBody {
    fn from(receipt: katana_primitives::receipt::Receipt) -> Self {
        let execution_status = if receipt.is_reverted() {
            Some(ExecutionStatus::Reverted)
        } else {
            Some(ExecutionStatus::Succeeded)
        };

        let execution_resources = Some(ExecutionResources {
            vm_resources: receipt.resources_used().computation_resources.clone(),
            data_availability: Some(receipt.resources_used().da_resources.clone()),
            total_gas_consumed: Some(receipt.resources_used().gas.clone()),
        });

        Self {
            execution_resources,
            l1_to_l2_consumed_message: None, /* This would need to be populated from transaction
                                              * context */
            l2_to_l1_messages: receipt.messages_sent().to_vec(),
            events: receipt.events().to_vec(),
            actual_fee: receipt.fee().overall_fee.into(),
            execution_status,
            revert_error: receipt.revert_reason().map(|s| s.to_string()),
        }
    }
}

impl From<katana_rpc_types::ConfirmedStateUpdate> for ConfirmedStateUpdate {
    fn from(value: katana_rpc_types::ConfirmedStateUpdate) -> Self {
        Self {
            block_hash: value.block_hash,
            new_root: value.new_root,
            old_root: value.old_root,
            state_diff: value.state_diff.into(),
        }
    }
}

impl From<katana_rpc_types::StateDiff> for StateDiff {
    fn from(value: katana_rpc_types::StateDiff) -> Self {
        let storage_diffs = value
            .storage_diffs
            .into_iter()
            .map(|(addr, entries)| {
                let diffs =
                    entries.into_iter().map(|(key, value)| StorageDiff { key, value }).collect();
                (addr, diffs)
            })
            .collect();

        let deployed_contracts = value
            .deployed_contracts
            .into_iter()
            .map(|(address, class_hash)| DeployedContract { address, class_hash })
            .collect();

        let declared_classes = value
            .declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredContract {
                class_hash,
                compiled_class_hash,
            })
            .collect();

        let replaced_classes = value
            .replaced_classes
            .into_iter()
            .map(|(address, class_hash)| DeployedContract { address, class_hash })
            .collect();

        Self {
            storage_diffs,
            deployed_contracts,
            old_declared_contracts: value.deprecated_declared_classes.into_iter().collect(),
            declared_classes,
            nonces: value.nonces,
            replaced_classes,
        }
    }
}

impl From<katana_rpc_types::PreConfirmedStateUpdate> for PreConfirmedStateUpdate {
    fn from(value: katana_rpc_types::PreConfirmedStateUpdate) -> Self {
        Self { old_root: value.old_root, state_diff: value.state_diff.into() }
    }
}

impl From<StateDiff> for katana_primitives::state::StateUpdates {
    fn from(value: StateDiff) -> Self {
        let storage_updates = value
            .storage_diffs
            .into_iter()
            .map(|(addr, diffs)| {
                let storage_map = diffs.into_iter().map(|diff| (diff.key, diff.value)).collect();
                (addr, storage_map)
            })
            .collect();

        let deployed_contracts = value
            .deployed_contracts
            .into_iter()
            .map(|contract| (contract.address, contract.class_hash))
            .collect();

        let declared_classes = value
            .declared_classes
            .into_iter()
            .map(|contract| (contract.class_hash, contract.compiled_class_hash))
            .collect();

        let replaced_classes = value
            .replaced_classes
            .into_iter()
            .map(|contract| (contract.address, contract.class_hash))
            .collect();

        Self {
            storage_updates,
            declared_classes,
            replaced_classes,
            deployed_contracts,
            nonce_updates: value.nonces,
            deprecated_declared_classes: BTreeSet::from_iter(value.old_declared_contracts),
        }
    }
}

impl From<katana_primitives::block::FinalityStatus> for BlockStatus {
    fn from(value: katana_primitives::block::FinalityStatus) -> Self {
        match value {
            katana_primitives::block::FinalityStatus::AcceptedOnL2 => BlockStatus::AcceptedOnL2,
            katana_primitives::block::FinalityStatus::AcceptedOnL1 => BlockStatus::AcceptedOnL1,
            katana_primitives::block::FinalityStatus::PreConfirmed => BlockStatus::Pending,
        }
    }
}
