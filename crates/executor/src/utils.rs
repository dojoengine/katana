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
        events.extend(collect_all_events(call));
    }

    if let Some(ref call) = info.execute_call_info {
        events.extend(collect_all_events(call));
    }

    if let Some(ref call) = info.fee_transfer_call_info {
        events.extend(collect_all_events(call));
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

fn collect_all_events(call_info: &CallInfo) -> Vec<Event> {
    fn inner(call_info: &CallInfo, events: &mut Vec<(usize, Event)>) {
        let from_address = call_info.call.storage_address.into();

        events.extend(call_info.execution.events.iter().map(|e| {
            let order = e.order;
            let data = e.event.data.0.clone();
            let keys = e.event.keys.iter().map(|k| k.0).collect();
            (order, Event { from_address, data, keys })
        }));

        for inner_call in &call_info.inner_calls {
            inner(inner_call, events);
        }
    }

    let mut events = Vec::new();
    inner(call_info, &mut events);
    events.sort_by_key(|(order, _)| *order);
    events.into_iter().map(|(_, event)| event).collect()
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

#[cfg(test)]
mod tests {
    use blockifier::execution::call_info::OrderedEvent;
    use katana_primitives::execution::{CallExecution, CallInfo};
    use katana_primitives::Felt;
    use katana_utils::arbitrary;
    use starknet_api::transaction::{EventContent, EventData, EventKey};

    use super::collect_all_events;

    macro_rules! rand_ordered_event {
        ($order:expr) => {{
            let keys = vec![EventKey(arbitrary!(Felt))];
            let data = EventData(vec![arbitrary!(Felt)]);
            OrderedEvent { order: $order, event: EventContent { keys, data } }
        }};
    }

    // TODO: perform this test on the RPC level.
    #[test]
    fn get_events_in_order() {
        let event_0 = rand_ordered_event!(0);
        let event_1 = rand_ordered_event!(1);
        let event_2 = rand_ordered_event!(2);
        let event_3 = rand_ordered_event!(3);
        let event_4 = rand_ordered_event!(4);
        let event_5 = rand_ordered_event!(5);
        let event_6 = rand_ordered_event!(6);
        let event_7 = rand_ordered_event!(7);

        // ## Nested Calls Structure And Event Emitted Ordering
        //
        //  [Call 1]
        //   │
        //   │   -> Event 0
        //   │
        //   │  [Call 2]
        //   │   │
        //   │   │  [Call 3]
        //   │   │   │
        //   │   │   │  -> Event 1
        //   │   │   │  -> Event 2
        //   │   │   │
        //   │   │   │
        //   │   │
        //   │   │  [Call 4]
        //   │   │   │
        //   │   │   │  -> Event 3
        //   │   │   │  -> Event 4
        //   │   │   │
        //   │   │   │
        //   │   │   │
        //   │   │
        //   │   │   -> Event 5
        //   │   │   -> Event 6
        //   │   │
        //   │
        //   │   -> Event 7
        //   │
        //

        let events_call_1 = vec![event_0.clone(), event_7.clone()];
        let events_call_2 = vec![event_5.clone(), event_6.clone()];
        let events_call_3 = vec![event_1.clone(), event_2.clone()];
        let events_call_4 = vec![event_3.clone(), event_4.clone()];

        let call_4 = CallInfo {
            execution: CallExecution { events: events_call_4, ..Default::default() },
            ..Default::default()
        };

        let call_3 = CallInfo {
            execution: CallExecution { events: events_call_3, ..Default::default() },
            ..Default::default()
        };

        let call_2 = CallInfo {
            execution: CallExecution { events: events_call_2, ..Default::default() },
            inner_calls: vec![call_3, call_4],
            ..Default::default()
        };

        let call_1 = CallInfo {
            execution: CallExecution { events: events_call_1, ..Default::default() },
            inner_calls: vec![call_2],
            ..Default::default()
        };

        // The expected order should be in the order they were emitted in the call info stack.
        let expected_events =
            vec![event_0, event_1, event_2, event_3, event_4, event_5, event_6, event_7];

        let actual_events = collect_all_events(&call_1);

        for (idx, event) in actual_events.iter().enumerate() {
            let expected_event = expected_events[idx].clone();

            let expected_data = expected_event.event.data.0;
            let expected_keys =
                expected_event.event.keys.iter().map(|k| k.0).collect::<Vec<Felt>>();

            similar_asserts::assert_eq!(expected_keys, event.keys);
            similar_asserts::assert_eq!(expected_data, event.data);
        }
    }
}
