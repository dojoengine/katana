use katana_primitives::contract::Nonce;
use katana_primitives::execution::{EntryPointSelector, VmResources};
use katana_primitives::receipt::{DataAvailabilityResources, Event, GasUsed, MessageToL1};
use katana_primitives::transaction::TxHash;
use katana_primitives::{eth, ContractAddress, Felt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedReceipt {
    /// The hash of the transaction the receipt belongs to.
    pub transaction_hash: TxHash,
    /// The index of the transaction in the block.
    pub transaction_index: u64,
    /// The body of the receipt.
    #[serde(flatten)]
    pub body: ReceiptBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptBody {
    pub execution_resources: Option<ExecutionResources>,
    pub l1_to_l2_consumed_message: Option<L1ToL2Message>,
    pub l2_to_l1_messages: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub actual_fee: Felt,
    /// The status of the transaction execution.
    pub execution_status: Option<ExecutionStatus>,
    /// The error message if the transaction was reverted.
    ///
    /// This field should only be present if the transaction was reverted ie the `execution_status`
    /// field is `REVERTED`.
    pub revert_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    #[serde(rename = "SUCCEEDED")]
    Succeeded,

    #[serde(rename = "REVERTED")]
    Reverted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResources {
    #[serde(flatten)]
    pub vm_resources: VmResources,
    pub data_availability: Option<DataAvailabilityResources>,
    pub total_gas_consumed: Option<GasUsed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1ToL2Message {
    /// The address of the Ethereum (L1) contract that sent the message.
    pub from_address: eth::Address,
    pub to_address: ContractAddress,
    pub selector: EntryPointSelector,
    pub payload: Vec<Felt>,
    pub nonce: Option<Nonce>,
}

////////////////////////////////////////////////////////////////////////////////
// Conversion to Katana RPC types.
////////////////////////////////////////////////////////////////////////////////

impl From<ExecutionStatus> for katana_rpc_types::ExecutionResult {
    fn from(value: ExecutionStatus) -> Self {
        match value {
            ExecutionStatus::Succeeded => katana_rpc_types::ExecutionResult::Succeeded,
            ExecutionStatus::Reverted => {
                // When converting from gateway ExecutionStatus::Reverted, we don't have the
                // revert reason here. The caller should use the revert_error field from
                // ReceiptBody if available.
                katana_rpc_types::ExecutionResult::Reverted {
                    reason: String::from("Transaction reverted"),
                }
            }
        }
    }
}

impl From<ExecutionResources> for katana_rpc_types::ExecutionResources {
    fn from(value: ExecutionResources) -> Self {
        let gas = value.total_gas_consumed.unwrap_or_default();
        katana_rpc_types::ExecutionResources {
            l1_gas: gas.l1_gas,
            l1_data_gas: gas.l1_data_gas,
            l2_gas: gas.l2_gas,
        }
    }
}

impl ReceiptBody {
    /// Convert the receipt body to an RPC execution result.
    ///
    /// This uses the `execution_status` field if available, otherwise falls back to checking
    /// the `revert_error` field. If `revert_error` is present, the result is `Reverted`.
    pub fn to_execution_result(&self) -> katana_rpc_types::ExecutionResult {
        if let Some(revert_error) = &self.revert_error {
            katana_rpc_types::ExecutionResult::Reverted { reason: revert_error.clone() }
        } else if let Some(status) = &self.execution_status {
            match status {
                ExecutionStatus::Succeeded => katana_rpc_types::ExecutionResult::Succeeded,
                ExecutionStatus::Reverted => {
                    // Reverted status without error message
                    katana_rpc_types::ExecutionResult::Reverted {
                        reason: String::from("Transaction reverted"),
                    }
                }
            }
        } else {
            // If no status is provided, assume success
            katana_rpc_types::ExecutionResult::Succeeded
        }
    }

    /// Convert to an RPC FeePayment with the given price unit.
    pub fn to_fee_payment(
        &self,
        unit: katana_primitives::fee::PriceUnit,
    ) -> katana_rpc_types::FeePayment {
        katana_rpc_types::FeePayment { amount: self.actual_fee, unit }
    }
}

impl ConfirmedReceipt {
    /// Create an RPC Invoke receipt from this gateway receipt.
    ///
    /// # Arguments
    /// * `finality_status` - The finality status of the transaction
    /// * `fee_unit` - The price unit for the fee
    pub fn to_rpc_invoke_receipt(
        self,
        finality_status: katana_primitives::block::FinalityStatus,
        fee_unit: katana_primitives::fee::PriceUnit,
    ) -> katana_rpc_types::RpcInvokeTxReceipt {
        let execution_result = self.body.to_execution_result();
        let actual_fee = self.body.to_fee_payment(fee_unit);

        katana_rpc_types::RpcInvokeTxReceipt {
            actual_fee,
            finality_status,
            messages_sent: self.body.l2_to_l1_messages,
            events: self.body.events,
            execution_resources: self.body.execution_resources.unwrap_or_default().into(),
            execution_result,
        }
    }

    /// Create an RPC Declare receipt from this gateway receipt.
    ///
    /// # Arguments
    /// * `finality_status` - The finality status of the transaction
    /// * `fee_unit` - The price unit for the fee
    pub fn to_rpc_declare_receipt(
        self,
        finality_status: katana_primitives::block::FinalityStatus,
        fee_unit: katana_primitives::fee::PriceUnit,
    ) -> katana_rpc_types::RpcDeclareTxReceipt {
        let execution_result = self.body.to_execution_result();
        let actual_fee = self.body.to_fee_payment(fee_unit);

        katana_rpc_types::RpcDeclareTxReceipt {
            actual_fee,
            finality_status,
            messages_sent: self.body.l2_to_l1_messages,
            events: self.body.events,
            execution_resources: self.body.execution_resources.unwrap_or_default().into(),
            execution_result,
        }
    }

    /// Create an RPC Deploy receipt from this gateway receipt.
    ///
    /// # Arguments
    /// * `finality_status` - The finality status of the transaction
    /// * `fee_unit` - The price unit for the fee
    /// * `contract_address` - The deployed contract address
    pub fn to_rpc_deploy_receipt(
        self,
        finality_status: katana_primitives::block::FinalityStatus,
        fee_unit: katana_primitives::fee::PriceUnit,
        contract_address: ContractAddress,
    ) -> katana_rpc_types::RpcDeployTxReceipt {
        let execution_result = self.body.to_execution_result();
        let actual_fee = self.body.to_fee_payment(fee_unit);

        katana_rpc_types::RpcDeployTxReceipt {
            actual_fee,
            finality_status,
            messages_sent: self.body.l2_to_l1_messages,
            events: self.body.events,
            execution_resources: self.body.execution_resources.unwrap_or_default().into(),
            contract_address,
            execution_result,
        }
    }

    /// Create an RPC DeployAccount receipt from this gateway receipt.
    ///
    /// # Arguments
    /// * `finality_status` - The finality status of the transaction
    /// * `fee_unit` - The price unit for the fee
    /// * `contract_address` - The deployed account contract address
    pub fn to_rpc_deploy_account_receipt(
        self,
        finality_status: katana_primitives::block::FinalityStatus,
        fee_unit: katana_primitives::fee::PriceUnit,
        contract_address: ContractAddress,
    ) -> katana_rpc_types::RpcDeployAccountTxReceipt {
        let execution_result = self.body.to_execution_result();
        let actual_fee = self.body.to_fee_payment(fee_unit);

        katana_rpc_types::RpcDeployAccountTxReceipt {
            actual_fee,
            finality_status,
            messages_sent: self.body.l2_to_l1_messages,
            events: self.body.events,
            execution_resources: self.body.execution_resources.unwrap_or_default().into(),
            contract_address,
            execution_result,
        }
    }

    /// Create an RPC L1Handler receipt from this gateway receipt.
    ///
    /// # Arguments
    /// * `finality_status` - The finality status of the transaction
    /// * `fee_unit` - The price unit for the fee
    /// * `message_hash` - The L1 to L2 message hash
    pub fn to_rpc_l1_handler_receipt(
        self,
        finality_status: katana_primitives::block::FinalityStatus,
        fee_unit: katana_primitives::fee::PriceUnit,
        message_hash: katana_primitives::B256,
    ) -> katana_rpc_types::RpcL1HandlerTxReceipt {
        let execution_result = self.body.to_execution_result();
        let actual_fee = self.body.to_fee_payment(fee_unit);

        katana_rpc_types::RpcL1HandlerTxReceipt {
            actual_fee,
            finality_status,
            messages_sent: self.body.l2_to_l1_messages,
            events: self.body.events,
            execution_resources: self.body.execution_resources.unwrap_or_default().into(),
            message_hash,
            execution_result,
        }
    }
}

impl Default for ExecutionResources {
    fn default() -> Self {
        Self {
            vm_resources: Default::default(),
            data_availability: None,
            total_gas_consumed: Some(Default::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::block::FinalityStatus;
    use katana_primitives::fee::PriceUnit;
    use katana_primitives::receipt::GasUsed;
    use katana_primitives::{address, felt};

    use super::*;

    fn create_test_receipt_body() -> ReceiptBody {
        ReceiptBody {
            execution_resources: Some(ExecutionResources {
                vm_resources: Default::default(),
                data_availability: None,
                total_gas_consumed: Some(GasUsed { l1_gas: 100, l1_data_gas: 50, l2_gas: 200 }),
            }),
            l1_to_l2_consumed_message: None,
            l2_to_l1_messages: vec![],
            events: vec![],
            actual_fee: felt!("0x1234"),
            execution_status: Some(ExecutionStatus::Succeeded),
            revert_error: None,
        }
    }

    #[test]
    fn test_execution_status_to_execution_result() {
        let succeeded: katana_rpc_types::ExecutionResult = ExecutionStatus::Succeeded.into();
        assert_eq!(succeeded, katana_rpc_types::ExecutionResult::Succeeded);

        let reverted: katana_rpc_types::ExecutionResult = ExecutionStatus::Reverted.into();
        match reverted {
            katana_rpc_types::ExecutionResult::Reverted { reason } => {
                assert_eq!(reason, "Transaction reverted");
            }
            _ => panic!("Expected Reverted result"),
        }
    }

    #[test]
    fn test_execution_resources_conversion() {
        let gateway_resources = ExecutionResources {
            vm_resources: Default::default(),
            data_availability: None,
            total_gas_consumed: Some(GasUsed { l1_gas: 100, l1_data_gas: 50, l2_gas: 200 }),
        };

        let rpc_resources: katana_rpc_types::ExecutionResources = gateway_resources.into();
        assert_eq!(rpc_resources.l1_gas, 100);
        assert_eq!(rpc_resources.l1_data_gas, 50);
        assert_eq!(rpc_resources.l2_gas, 200);
    }

    #[test]
    fn test_receipt_body_to_execution_result_with_revert_error() {
        let body = ReceiptBody {
            execution_resources: None,
            l1_to_l2_consumed_message: None,
            l2_to_l1_messages: vec![],
            events: vec![],
            actual_fee: felt!("0x0"),
            execution_status: Some(ExecutionStatus::Succeeded),
            revert_error: Some("Out of gas".to_string()),
        };

        let result = body.to_execution_result();
        match result {
            katana_rpc_types::ExecutionResult::Reverted { reason } => {
                assert_eq!(reason, "Out of gas");
            }
            _ => panic!("Expected Reverted result"),
        }
    }

    #[test]
    fn test_receipt_body_to_execution_result_succeeded() {
        let body = create_test_receipt_body();
        let result = body.to_execution_result();
        assert_eq!(result, katana_rpc_types::ExecutionResult::Succeeded);
    }

    #[test]
    fn test_to_rpc_invoke_receipt() {
        let gateway_receipt = ConfirmedReceipt {
            transaction_hash: felt!("0xabc"),
            transaction_index: 5,
            body: create_test_receipt_body(),
        };

        let rpc_receipt = gateway_receipt
            .clone()
            .to_rpc_invoke_receipt(FinalityStatus::AcceptedOnL2, PriceUnit::Wei);

        assert_eq!(rpc_receipt.actual_fee.amount, felt!("0x1234"));
        assert_eq!(rpc_receipt.actual_fee.unit, PriceUnit::Wei);
        assert_eq!(rpc_receipt.finality_status, FinalityStatus::AcceptedOnL2);
        assert_eq!(rpc_receipt.execution_resources.l1_gas, 100);
        assert_eq!(rpc_receipt.execution_resources.l2_gas, 200);
        assert_eq!(rpc_receipt.execution_result, katana_rpc_types::ExecutionResult::Succeeded);
    }

    #[test]
    fn test_to_rpc_declare_receipt() {
        let gateway_receipt = ConfirmedReceipt {
            transaction_hash: felt!("0xdef"),
            transaction_index: 10,
            body: create_test_receipt_body(),
        };

        let rpc_receipt = gateway_receipt
            .clone()
            .to_rpc_declare_receipt(FinalityStatus::AcceptedOnL1, PriceUnit::Fri);

        assert_eq!(rpc_receipt.actual_fee.amount, felt!("0x1234"));
        assert_eq!(rpc_receipt.actual_fee.unit, PriceUnit::Fri);
        assert_eq!(rpc_receipt.finality_status, FinalityStatus::AcceptedOnL1);
    }

    #[test]
    fn test_to_rpc_deploy_receipt() {
        let gateway_receipt = ConfirmedReceipt {
            transaction_hash: felt!("0x123"),
            transaction_index: 1,
            body: create_test_receipt_body(),
        };

        let contract_address = address!("0x456");
        let rpc_receipt = gateway_receipt.clone().to_rpc_deploy_receipt(
            FinalityStatus::AcceptedOnL2,
            PriceUnit::Wei,
            contract_address,
        );

        assert_eq!(rpc_receipt.contract_address, contract_address);
        assert_eq!(rpc_receipt.actual_fee.amount, felt!("0x1234"));
    }

    #[test]
    fn test_to_rpc_deploy_account_receipt() {
        let gateway_receipt = ConfirmedReceipt {
            transaction_hash: felt!("0x789"),
            transaction_index: 2,
            body: create_test_receipt_body(),
        };

        let contract_address = address!("0xabc");
        let rpc_receipt = gateway_receipt.clone().to_rpc_deploy_account_receipt(
            FinalityStatus::AcceptedOnL2,
            PriceUnit::Wei,
            contract_address,
        );

        assert_eq!(rpc_receipt.contract_address, contract_address);
        assert_eq!(rpc_receipt.execution_result, katana_rpc_types::ExecutionResult::Succeeded);
    }

    #[test]
    fn test_to_rpc_l1_handler_receipt() {
        let gateway_receipt = ConfirmedReceipt {
            transaction_hash: felt!("0xfff"),
            transaction_index: 3,
            body: create_test_receipt_body(),
        };

        let message_hash = katana_primitives::B256::from([1u8; 32]);
        let rpc_receipt = gateway_receipt.clone().to_rpc_l1_handler_receipt(
            FinalityStatus::AcceptedOnL2,
            PriceUnit::Wei,
            message_hash,
        );

        assert_eq!(rpc_receipt.message_hash, message_hash);
        assert_eq!(rpc_receipt.actual_fee.amount, felt!("0x1234"));
    }
}
