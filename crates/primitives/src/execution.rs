pub use blockifier::execution::call_info::{CallInfo, ExecutionSummary};
pub use blockifier::execution::entry_point::{CallEntryPoint, CallType};
pub use blockifier::execution::stack_trace::ErrorStack;
pub use blockifier::fee::fee_checks::FeeCheckError;
pub use blockifier::fee::receipt::TransactionReceipt;
pub use blockifier::fee::resources::{
    ComputationResources, StarknetResources, TransactionResources,
};
pub use blockifier::transaction::objects::{RevertError, TransactionExecutionInfo};
pub use cairo_vm::types::builtin_name::BuiltinName;
pub use cairo_vm::vm::runners::cairo_runner::ExecutionResources;
pub use starknet_api::contract_class::EntryPointType;
pub use starknet_api::executable_transaction::TransactionType;
pub use starknet_api::execution_resources::{GasAmount, GasVector};
pub use starknet_api::transaction::fields::Fee;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypedTransactionExecutionInfo {
    Invoke(TransactionExecutionInfo),
    Declare(TransactionExecutionInfo),
    L1Handler(TransactionExecutionInfo),
    DeployAccount(TransactionExecutionInfo),
}

impl TypedTransactionExecutionInfo {
    /// Returns the [`TransactionExecutionInfo`]
    pub fn info(&self) -> &TransactionExecutionInfo {
        match self {
            Self::Invoke(info) => info,
            Self::Declare(info) => info,
            Self::L1Handler(info) => info,
            Self::DeployAccount(info) => info,
        }
    }
}

impl Default for TypedTransactionExecutionInfo {
    fn default() -> Self {
        Self::Invoke(TransactionExecutionInfo::default())
    }
}
