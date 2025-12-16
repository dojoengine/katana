use assert_matches::assert_matches;
use katana_primitives::execution::EntryPointType;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::trace::{
    CallType, ExecuteInvocation, FunctionInvocation, InnerCallExecutionResources, InvokeTxTrace,
    OrderedEvent, OrderedL2ToL1Message, RevertedInvocation, TxTrace,
};
use katana_rpc_types::ExecutionResources;
use serde_json::Value;
use similar_asserts::assert_eq;

mod fixtures;

#[test]
fn invoke_trace() {
    let json = fixtures::test_data::<Value>("v0.9/traces/invoke_trace.json");
    let trace: TxTrace = serde_json::from_value(json.clone()).unwrap();

    let TxTrace::Invoke(invoke) = trace.clone() else {
        panic!("invalid type");
    };

    // Test execute_invocation
    assert_matches!(&invoke.execute_invocation, ExecuteInvocation::Success(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.caller_address, address!("0x0"));
        assert_eq!(invocation.contract_address, address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"));
        assert_eq!(invocation.entry_point_selector, felt!("0x15d40a3d6ca2ac30f4031e42be28da9b056fef9bb7357ac5e85627ee876e5ad"));
        assert_eq!(invocation.entry_point_type, EntryPointType::External);
        assert_eq!(invocation.class_hash, felt!("0x345354e2d801833068de73d1a2028e2f619f71045dd5229e79469fa7f598038"));
        assert!(!invocation.is_reverted);
        assert_eq!(invocation.calldata, vec![
            felt!("0x2"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x382be990ca34815134e64a9ac28f41a907c62e5ad10547f97174362ab94dc89"),
            felt!("0x0"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x27c3334165536f239cfd400ed956eabff55fc60de4fb56728b6a4f6b87db01c"),
            felt!("0x4"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x5df99ae77df976b4f0e5cf28c7dcfe09bd6e81aab787b19ac0c08e03d928cf"),
            felt!("0x1"),
            felt!("0x365")
        ]);
        assert_eq!(invocation.execution_resources.l1_gas, 0);
        assert_eq!(invocation.execution_resources.l2_gas, 1251905);

        assert_eq!(invocation.result, vec![
            felt!("0x2"),
            felt!("0x0"),
            felt!("0x0"),
        ]);

        // Check inner calls
        assert_eq!(invocation.calls.len(), 2);

        // First inner call
        let first_call = &invocation.calls[0];
        assert_eq!(first_call.call_type, CallType::Call);
        assert_eq!(first_call.entry_point_type, EntryPointType::External);
        assert_eq!(first_call.contract_address, address!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"));
        assert_eq!(first_call.entry_point_selector, felt!("0x382be990ca34815134e64a9ac28f41a907c62e5ad10547f97174362ab94dc89"));
        assert_eq!(first_call.caller_address, address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"));
        assert_eq!(first_call.class_hash, felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93"));
        assert_eq!(first_call.execution_resources.l1_gas, 0);
        assert_eq!(first_call.execution_resources.l2_gas, 870555);
        assert_eq!(first_call.calldata, vec![]);
        assert_eq!(first_call.calls, vec![]);
        assert_eq!(first_call.result, vec![]);
        assert_eq!(first_call.events, vec![]);
        assert_eq!(first_call.messages, vec![]);

        // Second inner call
        let second_call = &invocation.calls[1];
        assert_eq!(second_call.call_type, CallType::Call);
        assert_eq!(second_call.entry_point_type, EntryPointType::External);
        assert_eq!(second_call.contract_address, address!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"));
        assert_eq!(second_call.entry_point_selector, felt!("0x27c3334165536f239cfd400ed956eabff55fc60de4fb56728b6a4f6b87db01c"));
        assert_eq!(second_call.caller_address, address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"));
        assert_eq!(second_call.class_hash, felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93"));
        assert_eq!(second_call.execution_resources.l1_gas, 0);
        assert_eq!(second_call.execution_resources.l2_gas, 120200);
        assert_eq!(second_call.calldata, vec![
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x5df99ae77df976b4f0e5cf28c7dcfe09bd6e81aab787b19ac0c08e03d928cf"),
            felt!("0x1"),
            felt!("0x365"),
        ]);
        assert_eq!(second_call.result, vec![]);
        assert_eq!(second_call.events, vec![]);
        assert_eq!(second_call.messages, vec![]);

        // Second inner call - inner call
        let second_call_first_call = &second_call.calls[0];
        assert_eq!(second_call_first_call.call_type, CallType::Call);
        assert_eq!(second_call_first_call.entry_point_type, EntryPointType::External);
        assert_eq!(second_call_first_call.contract_address, address!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"));
        assert_eq!(second_call_first_call.entry_point_selector, felt!("0x5df99ae77df976b4f0e5cf28c7dcfe09bd6e81aab787b19ac0c08e03d928cf"));
        assert_eq!(second_call_first_call.caller_address, address!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"));
        assert_eq!(second_call_first_call.class_hash, felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93"));
        assert_eq!(second_call_first_call.execution_resources.l1_gas, 0);
        assert_eq!(second_call_first_call.execution_resources.l2_gas, 15650);
        assert_eq!(second_call_first_call.calldata, vec![felt!("0x365")]);
        assert_eq!(second_call_first_call.result, vec![felt!("0x76ae0bae894c28c926421539e969bc00ced005bef9bc4cee129e505f9dfc2bc")]);
        assert_eq!(second_call_first_call.events, vec![]);
        assert_eq!(second_call_first_call.messages, vec![]);
        assert_eq!(second_call_first_call.calls, vec![]);
        assert!(!second_call_first_call.is_reverted);
    });

    // Test validate_invocation
    assert_matches!(&invoke.validate_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.caller_address, address!("0x0"));
        assert_eq!(invocation.class_hash, felt!("0x345354e2d801833068de73d1a2028e2f619f71045dd5229e79469fa7f598038"));
        assert_eq!(invocation.contract_address, address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"));
        assert_eq!(invocation.entry_point_selector, felt!("0x162da33a4585851fe8d3af3c2a9c60b557814e221e0d4f30ff0b2189d9c7775"));
        assert_eq!(invocation.entry_point_type, EntryPointType::External);
        assert_eq!(invocation.execution_resources.l2_gas, 87715);
        assert_eq!(invocation.calldata, vec![
            felt!("0x2"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x382be990ca34815134e64a9ac28f41a907c62e5ad10547f97174362ab94dc89"),
            felt!("0x0"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x27c3334165536f239cfd400ed956eabff55fc60de4fb56728b6a4f6b87db01c"),
            felt!("0x4"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x5df99ae77df976b4f0e5cf28c7dcfe09bd6e81aab787b19ac0c08e03d928cf"),
            felt!("0x1"),
            felt!("0x365")
        ]);
        assert!(!invocation.is_reverted);
        assert_eq!(invocation.calls, vec![]);
        assert_eq!(invocation.events, vec![]);
        assert_eq!(invocation.messages, vec![]);
        assert_eq!(invocation.result, vec![felt!("0x56414c4944")]);
    });

    // Test fee_transfer_invocation
    assert_matches!(&invoke.fee_transfer_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.contract_address, address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"));
        assert_eq!(invocation.entry_point_selector, felt!("0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e"));
        assert_eq!(invocation.entry_point_type, EntryPointType::External);
        assert_eq!(invocation.execution_resources.l1_gas, 0);
        assert_eq!(invocation.execution_resources.l2_gas, 189030);
        assert!(!invocation.is_reverted);
        assert_eq!(invocation.calldata, vec![
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            felt!("0x1536bcb089b900"),
            felt!("0x0")
        ]);

        // Check events
        assert_eq!(invocation.events.len(), 1);
        let event = &invocation.events[0];
        assert_eq!(event.order, 0);
        assert_eq!(event.keys, vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")]);
        assert_eq!(event.data, vec![
            felt!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"),
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            felt!("0x1536bcb089b900"),
            felt!("0x0")
        ]);
    });

    // Test execution_resources
    assert_eq!(invoke.execution_resources.l1_gas, 0);
    assert_eq!(invoke.execution_resources.l2_gas, 1926180);
    assert_eq!(invoke.execution_resources.l1_data_gas, 128);

    // Test state_diff
    assert!(invoke.state_diff.is_none());

    let serialized = serde_json::to_value(trace).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn declare_trace() {
    let json = fixtures::test_data::<Value>("v0.9/traces/declare_trace.json");
    let trace: TxTrace = serde_json::from_value(json.clone()).unwrap();

    let TxTrace::Declare(declare) = trace.clone() else {
        panic!("invalid type");
    };

    // Test validate_invocation
    assert_matches!(&declare.validate_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.contract_address, address!("0x3e6c6f41d833ec5e7d38e6007df5b0ab6b48bc4c2d3dbeac1ed665456ae4766"));
        assert_eq!(invocation.entry_point_selector, felt!("0x289da278a8dc833409cabfdad1581e8e7d40e42dcaed693fa4008dcdb4963b3"));
        assert_eq!(invocation.entry_point_type, EntryPointType::External);
        assert_eq!(invocation.class_hash, felt!("0x5b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564"));
        assert_eq!(invocation.execution_resources.l2_gas, 320000);
        assert!(!invocation.is_reverted);

        // Check calldata
        assert_eq!(invocation.calldata.len(), 1);
        assert_eq!(invocation.calldata[0], felt!("0x7947f453169580affc156557222cb32ad91e1f5c59773aec79261a716afdf07"));
    });

    // Test fee_transfer_invocation
    assert_matches!(&declare.fee_transfer_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.contract_address, address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"));
        assert_eq!(invocation.entry_point_selector, felt!("0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e"));
        assert_eq!(invocation.execution_resources.l2_gas, 189030);

        // Check events
        assert_eq!(invocation.events.len(), 1);
        let event = &invocation.events[0];
        assert_eq!(event.order, 0);
        assert_eq!(event.keys[0], felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"));
    });

    // Test execution_resources
    assert_eq!(declare.execution_resources.l1_gas, 0);
    assert_eq!(declare.execution_resources.l2_gas, 1575437440);
    assert_eq!(declare.execution_resources.l1_data_gas, 192);

    // Test state_diff
    assert!(declare.state_diff.is_none());

    let serialized = serde_json::to_value(trace).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn deploy_account_trace() {
    let json = fixtures::test_data::<Value>("v0.9/traces/deploy_account_trace.json");
    let trace: TxTrace = serde_json::from_value(json.clone()).unwrap();

    let TxTrace::DeployAccount(deploy_account) = trace.clone() else {
        panic!("invalid type");
    };

    // Test constructor_invocation
    let constructor = &deploy_account.constructor_invocation;
    assert_eq!(constructor.call_type, CallType::Call);
    assert_eq!(
        constructor.contract_address,
        address!("0x445175dc7ae5f7054b0b05ab69dbfbfdd33867405158bc83941f47b285fbb8f")
    );
    assert_eq!(
        constructor.entry_point_selector,
        felt!("0x28ffe4ff0f226a9107253e17a904099aa4f63a02a5621de0576e5aa71bc5194")
    );
    assert_eq!(constructor.entry_point_type, EntryPointType::Constructor);
    assert_eq!(
        constructor.class_hash,
        felt!("0x5400e90f7e0ae78bd02c77cd75527280470e2fe19c54970dd79dc37a9d3645c")
    );
    assert_eq!(constructor.execution_resources.l2_gas, 95360);
    assert!(!constructor.is_reverted);

    // Check constructor calldata
    assert_eq!(constructor.calldata.len(), 1);
    assert_eq!(
        constructor.calldata[0],
        felt!("0x39d630974b4a48bc946fcc9514c4add698198a4bb2c47fc9e9f4a9c0f6ae2d8")
    );

    // Test validate_invocation
    assert_matches!(&deploy_account.validate_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.contract_address, address!("0x445175dc7ae5f7054b0b05ab69dbfbfdd33867405158bc83941f47b285fbb8f"));
        assert_eq!(invocation.entry_point_selector, felt!("0x36fcbf06cd96843058359e1a75928beacfac10727dab22a3972f0af8aa92895"));
        assert_eq!(invocation.entry_point_type, EntryPointType::External);
        assert_eq!(invocation.execution_resources.l2_gas, 320000);
        assert!(!invocation.is_reverted);
    });

    // Test fee_transfer_invocation
    assert_matches!(&deploy_account.fee_transfer_invocation, Some(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.contract_address, address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"));
        assert_eq!(invocation.entry_point_selector, felt!("0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e"));
        assert_eq!(invocation.execution_resources.l2_gas, 189030);

        // Check events
        assert_eq!(invocation.events.len(), 1);
        let event = &invocation.events[0];
        assert_eq!(event.order, 0);
        assert_eq!(event.keys[0], felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"));
    });

    // Test execution_resources
    assert_eq!(deploy_account.execution_resources.l1_gas, 0);
    assert_eq!(deploy_account.execution_resources.l2_gas, 750720);
    assert_eq!(deploy_account.execution_resources.l1_data_gas, 352);

    // Test state_diff
    assert!(deploy_account.state_diff.is_none());

    let serialized = serde_json::to_value(trace).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn l1_handler_trace() {
    let json = fixtures::test_data::<Value>("v0.9/traces/l1_handler_trace.json");
    let trace: TxTrace = serde_json::from_value(json.clone()).unwrap();

    let TxTrace::L1Handler(l1_handler) = trace.clone() else {
        panic!("invalid type");
    };

    // Test function_invocation
    assert_matches!(&l1_handler.function_invocation, ExecuteInvocation::Success(invocation) => {
        assert_eq!(invocation.call_type, CallType::Call);
        assert_eq!(invocation.caller_address, address!("0x0"));
        assert_eq!(invocation.contract_address, address!("0x140ab62001bb99cdf9685fd3be123aeeb41073e31942b80622fda6b66d54d4f"));
        assert_eq!(invocation.entry_point_selector, felt!("0x205500a208d0d49d79197fea83cc3f5fde99ac2e1909ae0a5d9f394c0c52ed0"));
        assert_eq!(invocation.entry_point_type, EntryPointType::L1Handler);
        assert_eq!(invocation.class_hash, felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93"));
        assert!(!invocation.is_reverted);

        // Check calldata
        assert_eq!(invocation.calldata.len(), 3);
        assert_eq!(invocation.calldata[0], felt!("0xeb4fb36931836751f265dd9930a410d9a543978"));
        assert_eq!(invocation.calldata[1], felt!("0x876"));

        // Check inner calls
        assert_eq!(invocation.calls.len(), 0);

        // Check execution resources
        assert_eq!(invocation.execution_resources.l2_gas, 0);
        assert_eq!(invocation.execution_resources.l1_gas, 1);
    });

    // Test execution_resources
    assert_eq!(l1_handler.execution_resources.l1_gas, 16023);
    assert_eq!(l1_handler.execution_resources.l2_gas, 187190);
    assert_eq!(l1_handler.execution_resources.l1_data_gas, 0);

    // Test state_diff
    assert!(l1_handler.state_diff.is_none());

    let serialized = serde_json::to_value(trace).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn reverted_invocation() {
    let reverted =
        RevertedInvocation { revert_reason: "Transaction execution has failed".to_string() };

    let invocation = ExecuteInvocation::Reverted(reverted.clone());

    // Test serialization
    let serialized = serde_json::to_value(&invocation).unwrap();
    assert_eq!(
        serialized,
        serde_json::json!({
            "revert_reason": "Transaction execution has failed"
        })
    );

    // Test deserialization
    let deserialized: ExecuteInvocation = serde_json::from_value(serialized).unwrap();
    assert_matches!(deserialized, ExecuteInvocation::Reverted(rev) => {
        assert_eq!(rev.revert_reason, reverted.revert_reason);
    });
}

#[test]
fn ordered_event_serialization() {
    let event = OrderedEvent {
        order: 42,
        keys: vec![
            felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
            felt!("0x1234567890abcdef"),
        ],
        data: vec![felt!("0x1"), felt!("0x2"), felt!("0x3")],
    };

    // Test serialization
    let serialized = serde_json::to_value(&event).unwrap();
    assert_eq!(serialized["order"], 42);
    assert_eq!(serialized["keys"].as_array().unwrap().len(), 2);
    assert_eq!(serialized["data"].as_array().unwrap().len(), 3);

    // Test deserialization roundtrip
    let deserialized: OrderedEvent = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, event);
}

#[test]
fn ordered_l2_to_l1_message_serialization() {
    let message = OrderedL2ToL1Message {
        order: 10,
        from_address: address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"),
        to_address: felt!("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc"),
        payload: vec![felt!("0x1"), felt!("0x2")],
    };

    // Test serialization
    let serialized = serde_json::to_value(&message).unwrap();
    assert_eq!(serialized["order"], 10);
    assert_eq!(
        serialized["from_address"],
        "0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"
    );
    assert_eq!(serialized["to_address"], "0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc");
    assert_eq!(serialized["payload"].as_array().unwrap().len(), 2);

    // Test deserialization roundtrip
    let deserialized: OrderedL2ToL1Message = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, message);
}

#[test]
fn call_type_serialization() {
    // Test CALL
    let call = CallType::Call;
    let serialized = serde_json::to_value(&call).unwrap();
    assert_eq!(serialized, "CALL");
    let deserialized: CallType = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, CallType::Call);

    // Test LIBRARY_CALL
    let library_call = CallType::LibraryCall;
    let serialized = serde_json::to_value(&library_call).unwrap();
    assert_eq!(serialized, "LIBRARY_CALL");
    let deserialized: CallType = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, CallType::LibraryCall);

    // Test DELEGATE
    let delegate = CallType::Delegate;
    let serialized = serde_json::to_value(&delegate).unwrap();
    assert_eq!(serialized, "DELEGATE");
    let deserialized: CallType = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, CallType::Delegate);
}

#[test]
fn function_invocation_with_nested_calls() {
    let nested_call = FunctionInvocation {
        call_type: CallType::Delegate,
        calldata: vec![felt!("0x123")],
        caller_address: address!("0x456"),
        calls: vec![],
        class_hash: felt!("0x789"),
        contract_address: address!("0xabc"),
        entry_point_selector: felt!("0xdef"),
        entry_point_type: EntryPointType::External,
        events: vec![],
        execution_resources: InnerCallExecutionResources { l1_gas: 100, l2_gas: 200 },
        is_reverted: false,
        messages: vec![],
        result: vec![felt!("0x111")],
    };

    let main_call = FunctionInvocation {
        call_type: CallType::Call,
        calldata: vec![felt!("0x222"), felt!("0x333")],
        caller_address: address!("0x0"),
        calls: vec![nested_call.clone()],
        class_hash: felt!("0x444"),
        contract_address: address!("0x555"),
        entry_point_selector: felt!("0x666"),
        entry_point_type: EntryPointType::External,
        events: vec![OrderedEvent {
            order: 0,
            keys: vec![felt!("0x777")],
            data: vec![felt!("0x888"), felt!("0x999")],
        }],
        execution_resources: InnerCallExecutionResources { l1_gas: 500, l2_gas: 1000 },
        is_reverted: false,
        messages: vec![],
        result: vec![],
    };

    // Test serialization
    let serialized = serde_json::to_value(&main_call).unwrap();

    // Verify structure
    assert_eq!(serialized["call_type"], "CALL");
    assert_eq!(serialized["calls"].as_array().unwrap().len(), 1);
    assert_eq!(serialized["calls"][0]["call_type"], "DELEGATE");
    assert_eq!(serialized["events"].as_array().unwrap().len(), 1);

    // Test deserialization
    let deserialized: FunctionInvocation = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized.calls.len(), 1);
    assert_eq!(deserialized.calls[0].call_type, CallType::Delegate);
    assert_eq!(deserialized.events.len(), 1);
}

#[test]
fn execute_invocation_with_revert() {
    let revert_reason = "Custom revert: insufficient balance";
    let reverted = RevertedInvocation { revert_reason: revert_reason.to_string() };
    let invocation = ExecuteInvocation::Reverted(reverted);

    // Test serialization
    let serialized = serde_json::to_value(&invocation).unwrap();
    assert_eq!(serialized["revert_reason"], revert_reason);

    // Test deserialization
    let deserialized: ExecuteInvocation = serde_json::from_value(serialized).unwrap();
    assert_matches!(deserialized, ExecuteInvocation::Reverted(rev) => {
        assert_eq!(rev.revert_reason, revert_reason);
    });
}

#[test]
fn tx_trace_with_state_diff() {
    use std::collections::{BTreeMap, BTreeSet};

    use katana_rpc_types::state_update::StateDiff;

    let mut nonces = BTreeMap::new();
    nonces.insert(address!("0x123"), felt!("0x456"));

    let mut deployed_contracts = BTreeMap::new();
    deployed_contracts.insert(address!("0x789"), felt!("0xabc"));

    let mut replaced_classes = BTreeMap::new();
    replaced_classes.insert(address!("0xdef"), felt!("0x111"));

    let mut declared_classes = BTreeMap::new();
    declared_classes.insert(felt!("0x222"), felt!("0x333"));

    let state_diff = StateDiff {
        storage_diffs: BTreeMap::new(),
        nonces,
        deployed_contracts,
        replaced_classes,
        declared_classes,
        deprecated_declared_classes: BTreeSet::new(),
        migrated_compiled_classes: None,
    };

    let trace = InvokeTxTrace {
        validate_invocation: None,
        execute_invocation: ExecuteInvocation::Reverted(RevertedInvocation {
            revert_reason: "Test revert".to_string(),
        }),
        fee_transfer_invocation: None,
        state_diff: Some(state_diff.clone()),
        execution_resources: ExecutionResources { l1_gas: 100, l2_gas: 200, l1_data_gas: 50 },
    };

    // Test serialization
    let serialized = serde_json::to_value(&trace).unwrap();
    assert!(serialized["state_diff"].is_object());
    assert_eq!(serialized["state_diff"]["nonces"].as_array().unwrap().len(), 1);

    // Test deserialization
    let deserialized: InvokeTxTrace = serde_json::from_value(serialized).unwrap();
    assert_matches!(deserialized.state_diff, Some(diff) => {
        assert_eq!(diff.nonces.len(), 1);
        assert_eq!(diff.deployed_contracts.len(), 1);
        assert_eq!(diff.replaced_classes.len(), 1);
        assert_eq!(diff.declared_classes.len(), 1);
    });
}

#[test]
fn inner_call_execution_resources_edge_cases() {
    // Test with zero resources
    let zero_resources = InnerCallExecutionResources { l1_gas: 0, l2_gas: 0 };
    let serialized = serde_json::to_value(&zero_resources).unwrap();
    assert_eq!(serialized["l1_gas"], 0);
    assert_eq!(serialized["l2_gas"], 0);

    // Test with maximum values
    let max_resources = InnerCallExecutionResources { l1_gas: u64::MAX, l2_gas: u64::MAX };
    let serialized = serde_json::to_value(&max_resources).unwrap();
    let deserialized: InnerCallExecutionResources = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized.l1_gas, u64::MAX);
    assert_eq!(deserialized.l2_gas, u64::MAX);
}

#[test]
fn empty_function_invocation() {
    let empty_invocation = FunctionInvocation {
        call_type: CallType::Call,
        calldata: vec![],
        caller_address: address!("0x0"),
        calls: vec![],
        class_hash: felt!("0x0"),
        contract_address: address!("0x0"),
        entry_point_selector: felt!("0x0"),
        entry_point_type: EntryPointType::External,
        events: vec![],
        execution_resources: InnerCallExecutionResources { l1_gas: 0, l2_gas: 0 },
        is_reverted: false,
        messages: vec![],
        result: vec![],
    };

    // Test serialization
    let serialized = serde_json::to_value(&empty_invocation).unwrap();
    assert_eq!(serialized["calldata"].as_array().unwrap().len(), 0);
    assert_eq!(serialized["calls"].as_array().unwrap().len(), 0);
    assert_eq!(serialized["events"].as_array().unwrap().len(), 0);
    assert_eq!(serialized["messages"].as_array().unwrap().len(), 0);
    assert_eq!(serialized["result"].as_array().unwrap().len(), 0);

    // Test deserialization
    let deserialized: FunctionInvocation = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, empty_invocation);
}
