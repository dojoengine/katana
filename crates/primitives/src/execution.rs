use std::collections::{hash_map, HashMap};

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
pub use cairo_vm::vm::runners::cairo_runner::ExecutionResources as VmResources;
pub use starknet_api::contract_class::EntryPointType;
pub use starknet_api::executable_transaction::TransactionType;
pub use starknet_api::execution_resources::{GasAmount, GasVector};
pub use starknet_api::transaction::fields::Fee;

use crate::transaction::TxType;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypedTransactionExecutionInfo {
    Invoke(TransactionExecutionInfo),
    Declare(TransactionExecutionInfo),
    L1Handler(TransactionExecutionInfo),
    DeployAccount(TransactionExecutionInfo),
}

impl TypedTransactionExecutionInfo {
    /// Constructs a new [`TypedTransactionExecutionInfo`].
    pub fn new(r#type: TxType, execution_info: TransactionExecutionInfo) -> Self {
        match r#type {
            TxType::Declare => Self::Declare(execution_info),
            TxType::Invoke => Self::Invoke(execution_info),
            TxType::DeployAccount => Self::DeployAccount(execution_info),
            TxType::L1Handler => Self::L1Handler(execution_info),
            TxType::Deploy => unimplemented!("deploy tx is unsupported"),
        }
    }

    /// Returns the [`TransactionExecutionInfo`]
    pub fn info(&self) -> &TransactionExecutionInfo {
        match self {
            Self::Invoke(info) => info,
            Self::Declare(info) => info,
            Self::L1Handler(info) => info,
            Self::DeployAccount(info) => info,
        }
    }

    /// Returns the transaction tyoe of the execution info.
    pub fn r#type(&self) -> TxType {
        match self {
            Self::Invoke(_) => TxType::Invoke,
            Self::Declare(_) => TxType::Declare,
            Self::L1Handler(_) => TxType::L1Handler,
            Self::DeployAccount(_) => TxType::DeployAccount,
        }
    }
}

impl From<TypedTransactionExecutionInfo> for TransactionExecutionInfo {
    fn from(value: TypedTransactionExecutionInfo) -> Self {
        match value {
            TypedTransactionExecutionInfo::Invoke(info) => info,
            TypedTransactionExecutionInfo::Declare(info) => info,
            TypedTransactionExecutionInfo::L1Handler(info) => info,
            TypedTransactionExecutionInfo::DeployAccount(info) => info,
        }
    }
}

impl From<(TxType, TransactionExecutionInfo)> for TypedTransactionExecutionInfo {
    fn from(value: (TxType, TransactionExecutionInfo)) -> Self {
        Self::new(value.0, value.1)
    }
}

impl Default for TypedTransactionExecutionInfo {
    fn default() -> Self {
        Self::Invoke(TransactionExecutionInfo::default())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[serde(transparent)]
pub struct BuiltinCounters(HashMap<BuiltinName, usize>);

impl BuiltinCounters {
    /// Returns the number of instances of the `output` builtin, if any.
    pub fn output(&self) -> Option<u64> {
        self.builtin(BuiltinName::output)
    }

    /// Returns the number of instances of the `range_check` builtin, if any.
    pub fn range_check(&self) -> Option<u64> {
        self.builtin(BuiltinName::range_check)
    }

    /// Returns the number of instances of the `pedersen` builtin, if any.
    pub fn pedersen(&self) -> Option<u64> {
        self.builtin(BuiltinName::pedersen)
    }

    /// Returns the number of instances of the `ecdsa` builtin, if any.
    pub fn ecdsa(&self) -> Option<u64> {
        self.builtin(BuiltinName::ecdsa)
    }

    /// Returns the number of instances of the `keccak` builtin, if any.
    pub fn keccak(&self) -> Option<u64> {
        self.builtin(BuiltinName::keccak)
    }

    /// Returns the number of instances of the `bitwise` builtin, if any.
    pub fn bitwise(&self) -> Option<u64> {
        self.builtin(BuiltinName::bitwise)
    }

    /// Returns the number of instances of the `ec_op` builtin, if any.
    pub fn ec_op(&self) -> Option<u64> {
        self.builtin(BuiltinName::ec_op)
    }

    /// Returns the number of instances of the `poseidon` builtin, if any.
    pub fn poseidon(&self) -> Option<u64> {
        self.builtin(BuiltinName::poseidon)
    }

    /// Returns the number of instances of the `segment_arena` builtin, if any.
    pub fn segment_arena(&self) -> Option<u64> {
        self.builtin(BuiltinName::segment_arena)
    }

    /// Returns the number of instances of the `range_check96` builtin, if any.
    pub fn range_check96(&self) -> Option<u64> {
        self.builtin(BuiltinName::range_check96)
    }

    /// Returns the number of instances of the `add_mod` builtin, if any.
    pub fn add_mod(&self) -> Option<u64> {
        self.builtin(BuiltinName::add_mod)
    }

    /// Returns the number of instances of the `mul_mod` builtin, if any.
    pub fn mul_mod(&self) -> Option<u64> {
        self.builtin(BuiltinName::mul_mod)
    }

    fn builtin(&self, builtin: BuiltinName) -> Option<u64> {
        self.0.get(&builtin).map(|&x| x as u64)
    }
}

impl From<BuiltinCounters> for HashMap<BuiltinName, usize> {
    fn from(value: BuiltinCounters) -> Self {
        value.0
    }
}

impl<T: Into<usize>> From<HashMap<BuiltinName, T>> for BuiltinCounters {
    fn from(map: HashMap<BuiltinName, T>) -> Self {
        // Filter out the builtins with 0 count.
        BuiltinCounters(
            map.into_iter()
                .filter_map(|(builtin, count)| {
                    let count = count.into();
                    (count != 0).then_some((builtin, count))
                })
                .collect(),
        )
    }
}

impl IntoIterator for BuiltinCounters {
    type Item = (BuiltinName, usize);
    type IntoIter = hash_map::IntoIter<BuiltinName, usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a BuiltinCounters {
    type Item = (&'a BuiltinName, &'a usize);
    type IntoIter = hash_map::Iter<'a, BuiltinName, usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{BuiltinCounters, BuiltinName};

    #[test]
    fn test_builtin_counters_from_hashmap_removes_zero_entries() {
        let mut map = HashMap::new();
        map.insert(BuiltinName::output, 1usize);
        map.insert(BuiltinName::range_check, 0);
        map.insert(BuiltinName::pedersen, 2);
        map.insert(BuiltinName::ecdsa, 0);

        let counters = BuiltinCounters::from(map);

        assert_eq!(counters.range_check(), None);
        assert_eq!(counters.pedersen(), Some(2));
        assert_eq!(counters.ecdsa(), None);
    }
}
