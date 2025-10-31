use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Instant;

use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::transaction::{
    DeclareTx, DeployAccountTx, ExecutableTx, ExecutableTxWithHash, InvokeTx, TxHash,
};
use katana_primitives::utils::get_contract_address;
use katana_primitives::Felt;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::BroadcastedTxWithChainId;

use crate::ordering::PoolOrd;
use crate::PoolTransaction;

/// the tx id in the pool. identified by its sender and nonce.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxId {
    sender: ContractAddress,
    nonce: Nonce,
}

impl TxId {
    pub fn new(sender: ContractAddress, nonce: Nonce) -> Self {
        Self { sender, nonce }
    }

    pub fn parent(&self) -> Option<Self> {
        if self.nonce == Nonce::ZERO {
            None
        } else {
            Some(Self { sender: self.sender, nonce: self.nonce - 1 })
        }
    }

    pub fn descendent(&self) -> Self {
        Self { sender: self.sender, nonce: self.nonce + 1 }
    }
}

#[derive(Debug)]
pub struct PendingTx<T, O: PoolOrd> {
    pub id: TxId,
    pub tx: Arc<T>,
    pub priority: O::PriorityValue,
    pub added_at: std::time::Instant,
}

impl<T, O: PoolOrd> PendingTx<T, O> {
    pub fn new(id: TxId, tx: T, priority: O::PriorityValue) -> Self {
        Self { id, tx: Arc::new(tx), priority, added_at: Instant::now() }
    }
}

// We can't just derive these traits because the derive implementation would require that
// the generics also implement these traits, which is not necessary.

impl<T, O: PoolOrd> Clone for PendingTx<T, O> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            added_at: self.added_at,
            tx: Arc::clone(&self.tx),
            priority: self.priority.clone(),
        }
    }
}

impl<T, O: PoolOrd> PartialEq for PendingTx<T, O> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl<T, O: PoolOrd> Eq for PendingTx<T, O> {}

impl<T, O: PoolOrd> PartialOrd for PendingTx<T, O> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// When two transactions have the same priority, we want to prioritize the one that was added
// first. So, when an incoming transaction with similar priority value is added to the
// [BTreeSet](std::collections::BTreeSet), the transaction is assigned a 'greater'
// [Ordering](std::cmp::Ordering) so that it will be placed after the existing ones. This is
// because items in a BTree is ordered from lowest to highest.
impl<T, O: PoolOrd> Ord for PendingTx<T, O> {
    fn cmp(&self, other: &Self) -> Ordering {
        // If the txs are coming from the same account, then we completely ignore their assigned
        // priority value and the ordering will be relative to the sender's nonce.
        if self.id.sender == other.id.sender {
            return match self.id.nonce.cmp(&other.id.nonce) {
                Ordering::Equal => Ordering::Greater,
                other => other,
            };
        }

        match self.priority.cmp(&other.priority) {
            Ordering::Equal => Ordering::Greater,
            other => other,
        }
    }
}

impl PoolTransaction for ExecutableTxWithHash {
    fn hash(&self) -> TxHash {
        self.hash
    }

    fn nonce(&self) -> Nonce {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(..) => unimplemented!("v0 transaction not supported"),
                InvokeTx::V1(v1) => v1.nonce,
                InvokeTx::V3(v3) => v3.nonce,
            },
            ExecutableTx::L1Handler(tx) => tx.nonce,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(..) => unimplemented!("v0 transaction not supported"),
                DeclareTx::V1(v1) => v1.nonce,
                DeclareTx::V2(v2) => v2.nonce,
                DeclareTx::V3(v3) => v3.nonce,
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(v1) => v1.nonce,
                DeployAccountTx::V3(v3) => v3.nonce,
            },
        }
    }

    fn sender(&self) -> ContractAddress {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(v0) => v0.contract_address,
                InvokeTx::V1(v1) => v1.sender_address,
                InvokeTx::V3(v3) => v3.sender_address,
            },
            ExecutableTx::L1Handler(tx) => tx.contract_address,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(v0) => v0.sender_address,
                DeclareTx::V1(v1) => v1.sender_address,
                DeclareTx::V2(v2) => v2.sender_address,
                DeclareTx::V3(v3) => v3.sender_address,
            },
            ExecutableTx::DeployAccount(tx) => tx.contract_address(),
        }
    }

    fn max_fee(&self) -> u128 {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V0(v0) => v0.max_fee,
                InvokeTx::V1(v1) => v1.max_fee,
                InvokeTx::V3(_) => 0, // V3 doesn't have max_fee
            },
            ExecutableTx::L1Handler(tx) => tx.paid_fee_on_l1,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V0(v0) => v0.max_fee,
                DeclareTx::V1(v1) => v1.max_fee,
                DeclareTx::V2(v2) => v2.max_fee,
                DeclareTx::V3(_) => 0, // V3 doesn't have max_fee
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(v1) => v1.max_fee,
                DeployAccountTx::V3(_) => 0, // V3 doesn't have max_fee
            },
        }
    }

    fn tip(&self) -> u64 {
        match &self.transaction {
            ExecutableTx::Invoke(tx) => match tx {
                InvokeTx::V3(v3) => v3.tip,
                _ => 0,
            },
            ExecutableTx::L1Handler(_) => 0,
            ExecutableTx::Declare(tx) => match &tx.transaction {
                DeclareTx::V3(v3) => v3.tip,
                _ => 0,
            },
            ExecutableTx::DeployAccount(tx) => match tx {
                DeployAccountTx::V3(v3) => v3.tip,
                _ => 0,
            },
        }
    }
}

impl PoolTransaction for BroadcastedTxWithChainId {
    fn hash(&self) -> TxHash {
        // BroadcastedTx doesn't have a precomputed hash, so we compute a deterministic
        // hash from the transaction content for pool identification purposes.
        use starknet_types_core::hash::{Poseidon, StarkHash};

        match self {
            BroadcastedTx::Invoke(tx) => {
                // Hash based on sender, nonce, and calldata
                let mut data = vec![tx.sender_address.into(), tx.nonce];
                data.extend_from_slice(&tx.calldata);
                Poseidon::hash_array(&data)
            }
            BroadcastedTx::Declare(tx) => {
                // Hash based on sender, nonce, and compiled class hash
                let data = [tx.sender_address.into(), tx.nonce, tx.compiled_class_hash.into()];
                Poseidon::hash_array(&data)
            }
            BroadcastedTx::DeployAccount(tx) => {
                // Hash based on computed contract address, nonce, and class hash
                let contract_address = get_contract_address(
                    tx.contract_address_salt,
                    tx.class_hash,
                    &tx.constructor_calldata,
                    Felt::ZERO,
                );
                let data = [contract_address, tx.nonce, tx.class_hash.into()];
                Poseidon::hash_array(&data)
            }
        }
    }

    fn nonce(&self) -> Nonce {
        match self {
            BroadcastedTx::Invoke(tx) => tx.nonce,
            BroadcastedTx::Declare(tx) => tx.nonce,
            BroadcastedTx::DeployAccount(tx) => tx.nonce,
        }
    }

    fn sender(&self) -> ContractAddress {
        match self {
            BroadcastedTx::Invoke(tx) => tx.sender_address,
            BroadcastedTx::Declare(tx) => tx.sender_address,
            BroadcastedTx::DeployAccount(tx) => {
                // Compute the contract address for deploy account transactions
                get_contract_address(
                    tx.contract_address_salt,
                    tx.class_hash,
                    &tx.constructor_calldata,
                    Felt::ZERO,
                )
                .into()
            }
        }
    }

    fn max_fee(&self) -> u128 {
        // BroadcastedTx only supports V3 transactions which use resource bounds instead of max_fee.
        // For V3 transactions, we can derive an equivalent max fee from resource bounds,
        // but for simplicity in the pool, we return 0.
        // The actual fee validation happens on the remote node.
        0
    }

    fn tip(&self) -> u64 {
        match self {
            BroadcastedTx::Invoke(tx) => tx.tip.into(),
            BroadcastedTx::Declare(tx) => tx.tip.into(),
            BroadcastedTx::DeployAccount(tx) => tx.tip.into(),
        }
    }
}
