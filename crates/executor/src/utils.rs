use blockifier::fee::receipt::TransactionReceipt;
use katana_primitives::execution::{CallInfo, TransactionExecutionInfo, TransactionResources};
use katana_primitives::fee::FeeInfo;
use katana_primitives::receipt::{
    self, DataAvailabilityResources, DeclareTxReceipt, DeployAccountTxReceipt, Event, GasUsed,
    InvokeTxReceipt, L1HandlerTxReceipt, MessageToL1, Receipt,
};
use katana_primitives::transaction::TxRef;
use tracing::trace;

pub(crate) const LOG_TARGET: &str = "executor";

pub fn log_resources(resources: &TransactionResources) {
    let mut mapped_strings = Vec::new();

    for (builtin, count) in &resources.computation.vm_resources.builtin_instance_counter {
        mapped_strings.push(format!("{builtin}: {count}"));
    }

    // Sort the strings alphabetically
    mapped_strings.sort();
    mapped_strings.insert(0, format!("steps: {}", resources.computation.vm_resources.n_steps));
    mapped_strings
        .insert(1, format!("memory holes: {}", resources.computation.vm_resources.n_memory_holes));

    trace!(target: LOG_TARGET, usage = mapped_strings.join(" | "), "Transaction resource usage.");
}

pub(crate) fn build_receipt(
    tx: TxRef<'_>,
    fee: FeeInfo,
    info: &TransactionExecutionInfo,
) -> Receipt {
    let events = events_from_exec_info(info);
    let messages_sent = l2_to_l1_messages_from_exec_info(info);
    let execution_resources = get_receipt_resources(&info.receipt);
    let revert_error = info.revert_error.as_ref().map(|e| e.to_string());

    match tx {
        TxRef::Invoke(_) => Receipt::Invoke(InvokeTxReceipt {
            events,
            fee,
            revert_error,
            messages_sent,
            execution_resources,
        }),

        TxRef::Declare(_) => Receipt::Declare(DeclareTxReceipt {
            events,
            fee,
            revert_error,
            messages_sent,
            execution_resources,
        }),

        TxRef::L1Handler(tx) => Receipt::L1Handler(L1HandlerTxReceipt {
            events,
            fee,
            revert_error,
            messages_sent,
            message_hash: tx.message_hash,
            execution_resources,
        }),

        TxRef::DeployAccount(tx) => Receipt::DeployAccount(DeployAccountTxReceipt {
            events,
            fee,
            revert_error,
            messages_sent,
            execution_resources,
            contract_address: tx.contract_address(),
        }),
    }
}

fn get_receipt_resources(receipt: &TransactionReceipt) -> receipt::ExecutionResources {
    let computation_resources = receipt.resources.computation.vm_resources.clone();

    let gas = GasUsed {
        l2_gas: receipt.gas.l2_gas.0,
        l1_gas: receipt.gas.l1_gas.0,
        l1_data_gas: receipt.gas.l1_data_gas.0,
    };

    let da_resources = DataAvailabilityResources {
        l1_gas: receipt.da_gas.l1_gas.0,
        l1_data_gas: receipt.da_gas.l1_data_gas.0,
    };

    receipt::ExecutionResources { da_resources, computation_resources, gas }
}

fn events_from_exec_info(info: &TransactionExecutionInfo) -> Vec<Event> {
    let mut events: Vec<Event> = vec![];

    if let Some(ref call) = info.validate_call_info {
        events.extend(get_events_recur(call));
    }

    if let Some(ref call) = info.execute_call_info {
        events.extend(get_events_recur(call));
    }

    if let Some(ref call) = info.fee_transfer_call_info {
        events.extend(get_events_recur(call));
    }

    events
}

fn l2_to_l1_messages_from_exec_info(info: &TransactionExecutionInfo) -> Vec<MessageToL1> {
    let mut messages = vec![];

    if let Some(ref info) = info.validate_call_info {
        messages.extend(get_l2_to_l1_messages_recur(info));
    }

    if let Some(ref info) = info.execute_call_info {
        messages.extend(get_l2_to_l1_messages_recur(info));
    }

    if let Some(ref info) = info.fee_transfer_call_info {
        messages.extend(get_l2_to_l1_messages_recur(info));
    }

    messages
}

fn get_events_recur(info: &CallInfo) -> Vec<Event> {
    let from_address = info.call.storage_address.into();
    let mut events = Vec::new();

    events.extend(info.execution.events.iter().map(|e| {
        let data = e.event.data.0.clone();
        let keys = e.event.keys.iter().map(|k| k.0).collect();
        Event { from_address, data, keys }
    }));

    info.inner_calls.iter().for_each(|call| events.extend(get_events_recur(call)));

    events
}

fn get_l2_to_l1_messages_recur(info: &CallInfo) -> Vec<MessageToL1> {
    let from_address = info.call.storage_address.into();
    let mut messages = Vec::new();

    messages.extend(info.execution.l2_to_l1_messages.iter().map(|m| {
        let payload = m.message.payload.0.clone();
        let to_address = m.message.to_address;
        MessageToL1 { from_address, to_address, payload }
    }));

    info.inner_calls.iter().for_each(|call| messages.extend(get_l2_to_l1_messages_recur(call)));

    messages
}
