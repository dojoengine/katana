use prost::Message;
use serde_json::Value;

pub mod types {
    include!(concat!(env!("OUT_DIR"), "/types.rs"));
}

pub mod starknet {
    include!(concat!(env!("OUT_DIR"), "/starknet.rs"));
}

#[test]
fn test_invoke_transaction_serialization() {
    use starknet::AddInvokeTransactionRequest;

    let invoke_v1 = types::BroadcastedInvokeTxnV1 {
        sender_address: Some(types::Felt { value: vec![1, 2, 3] }),
        calldata: vec![types::Felt { value: vec![4, 5, 6] }],
        max_fee: Some(types::Felt { value: vec![7, 8, 9] }),
        version: "0x1".to_string(),
        signature: vec![types::Felt { value: vec![10, 11, 12] }],
        nonce: Some(types::Felt { value: vec![13, 14, 15] }),
        is_query: false,
    };

    let request = AddInvokeTransactionRequest {
        invoke_transaction: Some(starknet::add_invoke_transaction_request::InvokeTransaction::V1(invoke_v1.clone())),
    };

    let bytes = request.encode_to_vec();
    let decoded = AddInvokeTransactionRequest::decode(bytes.as_slice()).unwrap();

    match decoded.invoke_transaction {
        Some(starknet::add_invoke_transaction_request::InvokeTransaction::V1(v1)) => {
            assert_eq!(v1.sender_address.as_ref().unwrap().value, vec![1, 2, 3]);
            assert_eq!(v1.calldata[0].value, vec![4, 5, 6]);
            assert_eq!(v1.max_fee.as_ref().unwrap().value, vec![7, 8, 9]);
            assert_eq!(v1.version, "0x1");
            assert_eq!(v1.signature[0].value, vec![10, 11, 12]);
            assert_eq!(v1.nonce.as_ref().unwrap().value, vec![13, 14, 15]);
            assert_eq!(v1.is_query, false);
        },
        _ => panic!("Expected V1 transaction"),
    }

    let json_value = serde_json::to_value(&invoke_v1).unwrap();
    
    assert!(json_value.is_object());
    let obj = json_value.as_object().unwrap();
    
    assert!(obj.contains_key("sender_address"));
    assert!(obj.contains_key("calldata"));
    assert!(obj.contains_key("max_fee"));
    assert!(obj.contains_key("version"));
    assert!(obj.contains_key("signature"));
    assert!(obj.contains_key("nonce"));
    
    assert!(!obj.contains_key("is_query"));
}

#[test]
fn test_declare_transaction_serialization() {
    use starknet::AddDeclareTransactionRequest;
    
    let declare_v2 = types::BroadcastedDeclareTxnV2 {
        sender_address: Some(types::Felt { value: vec![1, 2, 3] }),
        compiled_class_hash: Some(types::Felt { value: vec![4, 5, 6] }),
        max_fee: Some(types::Felt { value: vec![7, 8, 9] }),
        version: "0x2".to_string(),
        signature: vec![types::Felt { value: vec![10, 11, 12] }],
        nonce: Some(types::Felt { value: vec![13, 14, 15] }),
        contract_class: vec![16, 17, 18],
        is_query: false,
    };

    let request = AddDeclareTransactionRequest {
        declare_transaction: Some(starknet::add_declare_transaction_request::DeclareTransaction::V2(declare_v2.clone())),
    };

    let bytes = request.encode_to_vec();
    let decoded = AddDeclareTransactionRequest::decode(bytes.as_slice()).unwrap();
    
    match decoded.declare_transaction {
        Some(starknet::add_declare_transaction_request::DeclareTransaction::V2(v2)) => {
            assert_eq!(v2.sender_address.as_ref().unwrap().value, vec![1, 2, 3]);
            assert_eq!(v2.compiled_class_hash.as_ref().unwrap().value, vec![4, 5, 6]);
            assert_eq!(v2.max_fee.as_ref().unwrap().value, vec![7, 8, 9]);
            assert_eq!(v2.version, "0x2");
            assert_eq!(v2.signature[0].value, vec![10, 11, 12]);
            assert_eq!(v2.nonce.as_ref().unwrap().value, vec![13, 14, 15]);
            assert_eq!(v2.contract_class, vec![16, 17, 18]);
            assert_eq!(v2.is_query, false);
        },
        _ => panic!("Expected V2 transaction"),
    }

    let json_value = serde_json::to_value(&declare_v2).unwrap();
    
    assert!(json_value.is_object());
    let obj = json_value.as_object().unwrap();
    
    assert!(obj.contains_key("sender_address"));
    assert!(obj.contains_key("compiled_class_hash"));
    assert!(obj.contains_key("max_fee"));
    assert!(obj.contains_key("version"));
    assert!(obj.contains_key("signature"));
    assert!(obj.contains_key("nonce"));
    assert!(obj.contains_key("contract_class"));
    
    assert!(!obj.contains_key("is_query"));
}

#[test]
fn test_deploy_account_transaction_serialization() {
    use starknet::AddDeployAccountTransactionRequest;
    
    let deploy_account_v1 = types::BroadcastedDeployAccountTxnV1 {
        class_hash: Some(types::Felt { value: vec![1, 2, 3] }),
        contract_address_salt: Some(types::Felt { value: vec![4, 5, 6] }),
        constructor_calldata: vec![types::Felt { value: vec![7, 8, 9] }],
        max_fee: Some(types::Felt { value: vec![10, 11, 12] }),
        version: "0x1".to_string(),
        signature: vec![types::Felt { value: vec![13, 14, 15] }],
        nonce: Some(types::Felt { value: vec![16, 17, 18] }),
        is_query: false,
    };

    let request = AddDeployAccountTransactionRequest {
        deploy_account_transaction: Some(starknet::add_deploy_account_transaction_request::DeployAccountTransaction::V1(deploy_account_v1.clone())),
    };

    let bytes = request.encode_to_vec();
    let decoded = AddDeployAccountTransactionRequest::decode(bytes.as_slice()).unwrap();
    
    match decoded.deploy_account_transaction {
        Some(starknet::add_deploy_account_transaction_request::DeployAccountTransaction::V1(v1)) => {
            assert_eq!(v1.class_hash.as_ref().unwrap().value, vec![1, 2, 3]);
            assert_eq!(v1.contract_address_salt.as_ref().unwrap().value, vec![4, 5, 6]);
            assert_eq!(v1.constructor_calldata[0].value, vec![7, 8, 9]);
            assert_eq!(v1.max_fee.as_ref().unwrap().value, vec![10, 11, 12]);
            assert_eq!(v1.version, "0x1");
            assert_eq!(v1.signature[0].value, vec![13, 14, 15]);
            assert_eq!(v1.nonce.as_ref().unwrap().value, vec![16, 17, 18]);
            assert_eq!(v1.is_query, false);
        },
        _ => panic!("Expected V1 transaction"),
    }

    let json_value = serde_json::to_value(&deploy_account_v1).unwrap();
    
    assert!(json_value.is_object());
    let obj = json_value.as_object().unwrap();
    
    assert!(obj.contains_key("class_hash"));
    assert!(obj.contains_key("contract_address_salt"));
    assert!(obj.contains_key("constructor_calldata"));
    assert!(obj.contains_key("max_fee"));
    assert!(obj.contains_key("version"));
    assert!(obj.contains_key("signature"));
    assert!(obj.contains_key("nonce"));
    
    assert!(!obj.contains_key("is_query"));
}
