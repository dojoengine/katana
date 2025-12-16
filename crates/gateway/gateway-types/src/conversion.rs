use std::collections::BTreeSet;

use crate::{
    BlockStatus, ConfirmedStateUpdate, ConfirmedTransaction, DataAvailabilityMode, DeclareTx,
    DeclareTxV3, DeclaredContract, DeployAccountTx, DeployAccountTxV1, DeployAccountTxV3,
    DeployedContract, ExecutionResources, ExecutionStatus, InvokeTx, InvokeTxV3, L1HandlerTx,
    PreConfirmedStateUpdate, ReceiptBody, StateDiff, StateUpdate, StorageDiff, TypedTransaction,
};

// Conversions between katana-rpc to feeder gateway types

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

impl From<katana_rpc_types::PreConfirmedStateUpdate> for PreConfirmedStateUpdate {
    fn from(value: katana_rpc_types::PreConfirmedStateUpdate) -> Self {
        Self { old_root: value.old_root, state_diff: value.state_diff.into() }
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

        let migrated_compiled_classes = value
            .migrated_compiled_classes
            .unwrap_or_default()
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
            migrated_compiled_classes,
        }
    }
}

// Conversions between katana-primitives and feeder gateway types

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

        match receipt {
            katana_primitives::receipt::Receipt::Invoke(receipt) => {
                Self {
                    execution_resources: Some(receipt.execution_resources.into()),
                    // This would need to be populated from transaction context
                    l1_to_l2_consumed_message: None,
                    l2_to_l1_messages: receipt.messages_sent,
                    events: receipt.events,
                    actual_fee: receipt.fee.overall_fee.into(),
                    execution_status,
                    revert_error: receipt.revert_error,
                }
            }

            katana_primitives::receipt::Receipt::Declare(receipt) => Self {
                execution_resources: Some(receipt.execution_resources.into()),
                // This would need to be populated from transaction context
                l1_to_l2_consumed_message: None,
                l2_to_l1_messages: receipt.messages_sent,
                events: receipt.events,
                actual_fee: receipt.fee.overall_fee.into(),
                execution_status,
                revert_error: receipt.revert_error,
            },

            katana_primitives::receipt::Receipt::DeployAccount(receipt) => {
                Self {
                    execution_resources: Some(receipt.execution_resources.into()),
                    // This would need to be populated from transaction context
                    l1_to_l2_consumed_message: None,
                    l2_to_l1_messages: receipt.messages_sent,
                    events: receipt.events,
                    actual_fee: receipt.fee.overall_fee.into(),
                    execution_status,
                    revert_error: receipt.revert_error,
                }
            }

            katana_primitives::receipt::Receipt::L1Handler(receipt) => Self {
                execution_resources: Some(receipt.execution_resources.into()),
                // This would need to be populated from transaction context
                l1_to_l2_consumed_message: None,
                l2_to_l1_messages: receipt.messages_sent,
                events: receipt.events,
                actual_fee: receipt.fee.overall_fee.into(),
                execution_status,
                revert_error: receipt.revert_error,
            },
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

        let migrated_compiled_classes = value
            .migrated_compiled_classes
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
            migrated_compiled_classes,
        }
    }
}

impl From<katana_primitives::receipt::ExecutionResources> for ExecutionResources {
    fn from(value: katana_primitives::receipt::ExecutionResources) -> Self {
        Self {
            vm_resources: value.vm_resources,
            data_availability: Some(value.data_availability),
            total_gas_consumed: Some(value.total_gas_consumed),
        }
    }
}

impl From<ExecutionResources> for katana_primitives::receipt::ExecutionResources {
    fn from(value: ExecutionResources) -> Self {
        Self {
            vm_resources: value.vm_resources,
            data_availability: value.data_availability.unwrap_or_default(),
            total_gas_consumed: value.total_gas_consumed.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod from_primitives_test {
    use katana_primitives::transaction::TxWithHash;
    use katana_primitives::{address, felt, ContractAddress};
    use katana_utils::arbitrary;

    use super::*;

    #[test]
    fn tx_with_hash_conversion() {
        let tx_with_hash: TxWithHash = arbitrary!(TxWithHash);
        let converted: ConfirmedTransaction = tx_with_hash.clone().into();
        assert_eq!(converted.transaction_hash, tx_with_hash.hash);
        // The inner transaction body conversion is tested separately
    }

    #[test]
    fn invoke_tx_v0_conversion() {
        let invoke_v0 = arbitrary!(katana_primitives::transaction::InvokeTxV0);

        let tx = katana_primitives::transaction::InvokeTx::V0(invoke_v0.clone());
        let converted: InvokeTx = tx.into();

        match converted {
            InvokeTx::V0(v0) => {
                assert_eq!(v0.contract_address, invoke_v0.contract_address);
                assert_eq!(v0.entry_point_selector, invoke_v0.entry_point_selector);
                assert_eq!(v0.calldata, invoke_v0.calldata);
                assert_eq!(v0.signature, invoke_v0.signature);
                assert_eq!(v0.max_fee, invoke_v0.max_fee);
            }
            _ => panic!("Expected V0 variant"),
        }
    }

    #[test]
    fn invoke_tx_v1_conversion() {
        let invoke_v1: katana_primitives::transaction::InvokeTxV1 =
            arbitrary!(katana_primitives::transaction::InvokeTxV1);
        let tx = katana_primitives::transaction::InvokeTx::V1(invoke_v1.clone());
        let converted: InvokeTx = tx.into();

        match converted {
            InvokeTx::V1(v1) => {
                assert_eq!(v1.sender_address, invoke_v1.sender_address);
                assert_eq!(v1.calldata, invoke_v1.calldata);
                assert_eq!(v1.signature, invoke_v1.signature);
                assert_eq!(v1.max_fee, invoke_v1.max_fee);
                assert_eq!(v1.nonce, invoke_v1.nonce);
            }
            _ => panic!("Expected V1 variant"),
        }
    }

    #[test]
    fn invoke_tx_v3_conversion() {
        let invoke_v3: katana_primitives::transaction::InvokeTxV3 =
            arbitrary!(katana_primitives::transaction::InvokeTxV3);
        let tx = katana_primitives::transaction::InvokeTx::V3(invoke_v3.clone());
        let converted: InvokeTx = tx.into();

        match converted {
            InvokeTx::V3(v3) => {
                assert_eq!(v3.sender_address, invoke_v3.sender_address);
                assert_eq!(v3.nonce, invoke_v3.nonce);
                assert_eq!(v3.calldata, invoke_v3.calldata);
                assert_eq!(v3.signature, invoke_v3.signature);
                assert_eq!(v3.resource_bounds, invoke_v3.resource_bounds);
                assert_eq!(v3.tip, invoke_v3.tip.into());
                assert_eq!(v3.paymaster_data, invoke_v3.paymaster_data);
                assert_eq!(v3.account_deployment_data, invoke_v3.account_deployment_data);
            }
            _ => panic!("Expected V3 variant"),
        }
    }

    #[test]
    fn declare_tx_v0_conversion() {
        let declare_v0: katana_primitives::transaction::DeclareTxV0 =
            arbitrary!(katana_primitives::transaction::DeclareTxV0);
        let tx = katana_primitives::transaction::DeclareTx::V0(declare_v0.clone());
        let converted: DeclareTx = tx.into();

        match converted {
            DeclareTx::V0(v0) => {
                assert_eq!(v0.sender_address, declare_v0.sender_address);
                assert_eq!(v0.signature, declare_v0.signature);
                assert_eq!(v0.class_hash, declare_v0.class_hash);
                assert_eq!(v0.max_fee, declare_v0.max_fee);
            }
            _ => panic!("Expected V0 variant"),
        }
    }

    #[test]
    fn declare_tx_v1_conversion() {
        let declare_v1: katana_primitives::transaction::DeclareTxV1 =
            arbitrary!(katana_primitives::transaction::DeclareTxV1);
        let tx = katana_primitives::transaction::DeclareTx::V1(declare_v1.clone());
        let converted: DeclareTx = tx.into();

        match converted {
            DeclareTx::V1(v1) => {
                assert_eq!(v1.sender_address, declare_v1.sender_address);
                assert_eq!(v1.nonce, declare_v1.nonce);
                assert_eq!(v1.signature, declare_v1.signature);
                assert_eq!(v1.class_hash, declare_v1.class_hash);
                assert_eq!(v1.max_fee, declare_v1.max_fee);
            }
            _ => panic!("Expected V1 variant"),
        }
    }

    #[test]
    fn declare_tx_v2_conversion() {
        let declare_v2: katana_primitives::transaction::DeclareTxV2 =
            arbitrary!(katana_primitives::transaction::DeclareTxV2);
        let tx = katana_primitives::transaction::DeclareTx::V2(declare_v2.clone());
        let converted: DeclareTx = tx.into();

        match converted {
            DeclareTx::V2(v2) => {
                assert_eq!(v2.sender_address, declare_v2.sender_address);
                assert_eq!(v2.nonce, declare_v2.nonce);
                assert_eq!(v2.signature, declare_v2.signature);
                assert_eq!(v2.class_hash, declare_v2.class_hash);
                assert_eq!(v2.compiled_class_hash, declare_v2.compiled_class_hash);
                assert_eq!(v2.max_fee, declare_v2.max_fee);
            }
            _ => panic!("Expected V2 variant"),
        }
    }

    #[test]
    fn declare_tx_v3_conversion() {
        let declare_v3: katana_primitives::transaction::DeclareTxV3 =
            arbitrary!(katana_primitives::transaction::DeclareTxV3);
        let tx = katana_primitives::transaction::DeclareTx::V3(declare_v3.clone());
        let converted: DeclareTx = tx.into();

        match converted {
            DeclareTx::V3(v3) => {
                assert_eq!(v3.sender_address, declare_v3.sender_address);
                assert_eq!(v3.nonce, declare_v3.nonce);
                assert_eq!(v3.signature, declare_v3.signature);
                assert_eq!(v3.class_hash, declare_v3.class_hash);
                assert_eq!(v3.compiled_class_hash, declare_v3.compiled_class_hash);
                assert_eq!(v3.resource_bounds, declare_v3.resource_bounds);
                assert_eq!(v3.tip, declare_v3.tip.into());
                assert_eq!(v3.paymaster_data, declare_v3.paymaster_data);
                assert_eq!(v3.account_deployment_data, declare_v3.account_deployment_data);
            }
            _ => panic!("Expected V3 variant"),
        }
    }

    #[test]
    fn deploy_account_tx_v1_conversion() {
        let deploy_v1: katana_primitives::transaction::DeployAccountTxV1 =
            arbitrary!(katana_primitives::transaction::DeployAccountTxV1);
        let tx = katana_primitives::transaction::DeployAccountTx::V1(deploy_v1.clone());
        let converted: DeployAccountTx = tx.into();

        match converted {
            DeployAccountTx::V1(v1) => {
                assert_eq!(v1.nonce, deploy_v1.nonce);
                assert_eq!(v1.signature, deploy_v1.signature);
                assert_eq!(v1.class_hash, deploy_v1.class_hash);
                assert_eq!(v1.contract_address, Some(deploy_v1.contract_address));
                assert_eq!(v1.contract_address_salt, deploy_v1.contract_address_salt);
                assert_eq!(v1.constructor_calldata, deploy_v1.constructor_calldata);
                assert_eq!(v1.max_fee, deploy_v1.max_fee);
            }
            _ => panic!("Expected V1 variant"),
        }
    }

    #[test]
    fn deploy_account_tx_v3_conversion() {
        let deploy_v3: katana_primitives::transaction::DeployAccountTxV3 =
            arbitrary!(katana_primitives::transaction::DeployAccountTxV3);
        let tx = katana_primitives::transaction::DeployAccountTx::V3(deploy_v3.clone());
        let converted: DeployAccountTx = tx.into();

        match converted {
            DeployAccountTx::V3(v3) => {
                assert_eq!(v3.nonce, deploy_v3.nonce);
                assert_eq!(v3.signature, deploy_v3.signature);
                assert_eq!(v3.class_hash, deploy_v3.class_hash);
                assert_eq!(v3.contract_address, Some(deploy_v3.contract_address));
                assert_eq!(v3.contract_address_salt, deploy_v3.contract_address_salt);
                assert_eq!(v3.constructor_calldata, deploy_v3.constructor_calldata);
                assert_eq!(v3.resource_bounds, deploy_v3.resource_bounds);
                assert_eq!(v3.tip, deploy_v3.tip.into());
                assert_eq!(v3.paymaster_data, deploy_v3.paymaster_data);
            }
            _ => panic!("Expected V3 variant"),
        }
    }

    #[test]
    fn l1_handler_tx_conversion() {
        let l1_handler: katana_primitives::transaction::L1HandlerTx =
            arbitrary!(katana_primitives::transaction::L1HandlerTx);
        let converted: L1HandlerTx = l1_handler.clone().into();

        assert_eq!(converted.nonce, Some(l1_handler.nonce));
        assert_eq!(converted.version, l1_handler.version);
        assert_eq!(converted.calldata, l1_handler.calldata);
        assert_eq!(converted.contract_address, l1_handler.contract_address);
        assert_eq!(converted.entry_point_selector, l1_handler.entry_point_selector);
    }

    #[test]
    fn data_availability_mode_conversion() {
        let l1_mode = katana_primitives::da::DataAvailabilityMode::L1;
        let converted: DataAvailabilityMode = l1_mode.into();
        assert!(matches!(converted, DataAvailabilityMode::L1));

        let l2_mode = katana_primitives::da::DataAvailabilityMode::L2;
        let converted: DataAvailabilityMode = l2_mode.into();
        assert!(matches!(converted, DataAvailabilityMode::L2));
    }

    #[test]
    fn block_status_conversion() {
        use katana_primitives::block::FinalityStatus;

        let accepted_l2 = FinalityStatus::AcceptedOnL2;
        let converted: BlockStatus = accepted_l2.into();
        assert!(matches!(converted, BlockStatus::AcceptedOnL2));

        let accepted_l1 = FinalityStatus::AcceptedOnL1;
        let converted: BlockStatus = accepted_l1.into();
        assert!(matches!(converted, BlockStatus::AcceptedOnL1));

        let pre_confirmed = FinalityStatus::PreConfirmed;
        let converted: BlockStatus = pre_confirmed.into();
        assert!(matches!(converted, BlockStatus::Pending));
    }

    #[test]
    fn tx_type_conversion() {
        // Test Invoke transaction
        let invoke_tx: katana_primitives::transaction::InvokeTx =
            arbitrary!(katana_primitives::transaction::InvokeTx);
        let tx = katana_primitives::transaction::Tx::Invoke(invoke_tx);
        let converted: TypedTransaction = tx.into();
        assert!(matches!(converted, TypedTransaction::InvokeFunction(_)));

        // Test Declare transaction
        let declare_tx: katana_primitives::transaction::DeclareTx =
            arbitrary!(katana_primitives::transaction::DeclareTx);
        let tx = katana_primitives::transaction::Tx::Declare(declare_tx);
        let converted: TypedTransaction = tx.into();
        assert!(matches!(converted, TypedTransaction::Declare(_)));

        // Test L1Handler transaction
        let l1_handler_tx: katana_primitives::transaction::L1HandlerTx =
            arbitrary!(katana_primitives::transaction::L1HandlerTx);
        let tx = katana_primitives::transaction::Tx::L1Handler(l1_handler_tx);
        let converted: TypedTransaction = tx.into();
        assert!(matches!(converted, TypedTransaction::L1Handler(_)));

        // Test DeployAccount transaction
        let deploy_account_tx: katana_primitives::transaction::DeployAccountTx =
            arbitrary!(katana_primitives::transaction::DeployAccountTx);
        let tx = katana_primitives::transaction::Tx::DeployAccount(deploy_account_tx);
        let converted: TypedTransaction = tx.into();
        assert!(matches!(converted, TypedTransaction::DeployAccount(_)));

        // Test Deploy transaction
        let deploy_tx: katana_primitives::transaction::DeployTx =
            arbitrary!(katana_primitives::transaction::DeployTx);
        let tx = katana_primitives::transaction::Tx::Deploy(deploy_tx.clone());
        let converted: TypedTransaction = tx.into();
        assert!(matches!(converted, TypedTransaction::Deploy(_)));
    }

    #[test]
    fn state_diff_to_state_updates_conversion() {
        use std::collections::BTreeMap;

        let mut storage_diffs = BTreeMap::new();
        storage_diffs.insert(
            address!("0x1"),
            vec![StorageDiff { key: felt!("0x10"), value: felt!("0x20") }],
        );

        let state_diff = StateDiff {
            storage_diffs,
            deployed_contracts: vec![DeployedContract {
                address: address!("0x2"),
                class_hash: felt!("0x64"),
            }],
            old_declared_contracts: vec![felt!("0x4")],
            declared_classes: vec![DeclaredContract {
                class_hash: felt!("0x3"),
                compiled_class_hash: felt!("0xc8"),
            }],
            nonces: BTreeMap::new(),
            replaced_classes: vec![],
            migrated_compiled_classes: Vec::new(),
        };

        let converted: katana_primitives::state::StateUpdates = state_diff.into();

        // Verify storage updates
        assert_eq!(converted.storage_updates.len(), 1);
        let storage_map = converted.storage_updates.get(&address!("0x1")).unwrap();
        assert_eq!(storage_map.get(&felt!("0x10")), Some(&felt!("0x20")));

        // Verify deployed contracts
        assert_eq!(converted.deployed_contracts.len(), 1);
        assert_eq!(converted.deployed_contracts.get(&address!("0x2")), Some(&felt!("0x64")));

        // Verify declared classes
        assert_eq!(converted.declared_classes.len(), 1);
        assert_eq!(converted.declared_classes.get(&felt!("0x3")), Some(&felt!("0xc8")));

        // Verify deprecated declared classes
        assert!(converted.deprecated_declared_classes.contains(&felt!("0x4")));
    }
}

#[cfg(test)]
mod from_rpc_test {
    use std::collections::{BTreeMap, BTreeSet};

    use katana_primitives::contract::ContractAddress;
    use katana_primitives::{address, felt};

    use crate::{DeclaredContract, StateDiff};

    #[test]
    fn state_diff_conversion() {
        // Create test data
        let mut storage_diffs = BTreeMap::new();
        storage_diffs.insert(address!("0x1"), BTreeMap::from([(felt!("0x10"), felt!("0x20"))]));

        let mut deployed_contracts = BTreeMap::new();
        deployed_contracts.insert(address!("0x2"), felt!("0x64"));

        let mut declared_classes = BTreeMap::new();
        declared_classes.insert(felt!("0x3"), felt!("0xc8"));

        let rpc_state_diff = katana_rpc_types::StateDiff {
            storage_diffs,
            deployed_contracts,
            deprecated_declared_classes: BTreeSet::from([felt!("0x4")]),
            declared_classes,
            nonces: BTreeMap::new(),
            replaced_classes: BTreeMap::new(),
            migrated_compiled_classes: Some(BTreeMap::from_iter([
                (felt!("0xa1"), felt!("0xb1")),
                (felt!("0xa2"), felt!("0xb2")),
            ])),
        };

        let converted: StateDiff = rpc_state_diff.into();

        // Verify storage diffs
        assert_eq!(converted.storage_diffs.len(), 1);
        let storage_entries = converted.storage_diffs.get(&address!("0x1")).unwrap();
        assert_eq!(storage_entries.len(), 1);
        assert_eq!(storage_entries[0].key, felt!("0x10"));
        assert_eq!(storage_entries[0].value, felt!("0x20"));

        // Verify deployed contracts
        assert_eq!(converted.deployed_contracts.len(), 1);
        assert_eq!(converted.deployed_contracts[0].address, address!("0x2"));
        assert_eq!(converted.deployed_contracts[0].class_hash, felt!("0x64"));

        // Verify declared classes
        assert_eq!(converted.declared_classes.len(), 1);
        assert_eq!(converted.declared_classes[0].class_hash, felt!("0x3"));
        assert_eq!(converted.declared_classes[0].compiled_class_hash, felt!("0xc8"));

        // Verify deprecated declared classes
        assert_eq!(converted.old_declared_contracts.len(), 1);
        assert!(converted.old_declared_contracts.contains(&felt!("0x4")));

        // Verify migrated class hashes
        assert_eq!(converted.migrated_compiled_classes.len(), 2);
        assert_eq!(
            converted.migrated_compiled_classes,
            vec![
                DeclaredContract { class_hash: felt!("0xa1"), compiled_class_hash: felt!("0xb1") },
                DeclaredContract { class_hash: felt!("0xa2"), compiled_class_hash: felt!("0xb2") }
            ]
        );
    }
}
