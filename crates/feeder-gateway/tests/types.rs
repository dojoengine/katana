use std::collections::BTreeMap;

use katana_feeder_gateway::types::{
    ConfirmedReceipt, DeclaredContract, DeployedContract, ExecutionStatus, StateDiff, StorageDiff,
};
use katana_primitives::execution::BuiltinName;
use katana_primitives::{address, felt, ContractAddress};
use serde_json::json;

#[test]
fn state_diff_to_state_updates_conversion() {
    let mut storage_diffs = BTreeMap::new();
    let contract_addr = address!("0x64");
    storage_diffs.insert(
        contract_addr,
        vec![
            StorageDiff { key: felt!("0x1"), value: felt!("0x2a") },
            StorageDiff { key: felt!("0x2"), value: felt!("0x2b") },
        ],
    );

    let deployed_contracts = vec![
        DeployedContract { address: address!("0xc8"), class_hash: felt!("0x12c") },
        DeployedContract { address: address!("0xc9"), class_hash: felt!("0x12d") },
    ];

    let declared_classes = vec![
        DeclaredContract { class_hash: felt!("0x190"), compiled_class_hash: felt!("0x1f4") },
        DeclaredContract { class_hash: felt!("0x191"), compiled_class_hash: felt!("0x1f5") },
    ];

    let old_declared_contracts = vec![felt!("0x258"), felt!("0x259")];

    let mut nonces = BTreeMap::new();
    nonces.insert(address!("0x2bc"), felt!("0x1"));
    nonces.insert(address!("0x2bd"), felt!("0x2"));

    let replaced_classes =
        vec![DeployedContract { address: address!("0x320"), class_hash: felt!("0x384") }];

    let state_diff = StateDiff {
        storage_diffs,
        deployed_contracts,
        old_declared_contracts,
        declared_classes,
        nonces,
        replaced_classes,
    };

    let state_updates: katana_primitives::state::StateUpdates = state_diff.into();

    // storage updates
    assert_eq!(state_updates.storage_updates.len(), 1);
    let storage_map = state_updates.storage_updates.get(&contract_addr).unwrap();
    assert_eq!(storage_map.len(), 2);
    assert_eq!(storage_map.get(&felt!("0x1")).unwrap(), &felt!("0x2a"));
    assert_eq!(storage_map.get(&felt!("0x2")).unwrap(), &felt!("0x2b"));

    // deployed contracts
    assert_eq!(state_updates.deployed_contracts.len(), 2);
    assert_eq!(state_updates.deployed_contracts.get(&address!("0xc8")).unwrap(), &felt!("0x12c"));
    assert_eq!(state_updates.deployed_contracts.get(&address!("0xc9")).unwrap(), &felt!("0x12d"));

    // declared classes
    assert_eq!(state_updates.declared_classes.len(), 2);
    assert_eq!(state_updates.declared_classes.get(&felt!("0x190")).unwrap(), &felt!("0x1f4"));
    assert_eq!(state_updates.declared_classes.get(&felt!("0x191")).unwrap(), &felt!("0x1f5"));

    // deprecated declared classes
    assert_eq!(state_updates.deprecated_declared_classes.len(), 2);
    assert!(state_updates.deprecated_declared_classes.contains(&felt!("0x258")));
    assert!(state_updates.deprecated_declared_classes.contains(&felt!("0x259")));

    // nonces
    assert_eq!(state_updates.nonce_updates.len(), 2);
    assert_eq!(state_updates.nonce_updates.get(&address!("0x2bc")).unwrap(), &felt!("0x1"));
    assert_eq!(state_updates.nonce_updates.get(&address!("0x2bd")).unwrap(), &felt!("0x2"));

    // replaced classes
    assert_eq!(state_updates.replaced_classes.len(), 1);
    assert_eq!(state_updates.replaced_classes.get(&address!("0x320")).unwrap(), &felt!("0x384"));
}

#[test]
fn state_diff_empty_conversion() {
    let state_diff = StateDiff::default();
    let state_updates: katana_primitives::state::StateUpdates = state_diff.into();

    assert!(state_updates.storage_updates.is_empty());
    assert!(state_updates.deployed_contracts.is_empty());
    assert!(state_updates.declared_classes.is_empty());
    assert!(state_updates.deprecated_declared_classes.is_empty());
    assert!(state_updates.nonce_updates.is_empty());
    assert!(state_updates.replaced_classes.is_empty());
}

#[test]
fn receipt_serde_succeeded() {
    let json = json!({
          "actual_fee": "0x25756bce009ac0",
          "events": [
            {
              "data": ["0x1", "0x0"],
              "from_address": "0x1f51ffc7121aea2093fb95a5995b072b72c89172fbd82acf62498f388e3bde7",
              "keys": [
                "0x1dcde06aabdbca2f80aa51392b345d7549d7757aa855f7e37f5d335ac8243b1",
                "0x5f505603ff517ab44cd17b531be418a8f29930fbb5877144afc6a090696564b"
              ]
            },
            {
              "data": [
                "0x438761135b0d7a930694309d61c7e659789921c96434e5b78253af2c5b5b2b5",
                "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                "0x25756bce009ac0",
                "0x0"
              ],
              "from_address": "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
              "keys": [
                "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
              ]
            }
          ],
          "execution_resources": {
            "builtin_instance_counter": {
              "bitwise_builtin": 1,
              "ec_op_builtin": 6,
              "pedersen_builtin": 30,
              "poseidon_builtin": 112,
              "range_check_builtin": 1031
            },
            "data_availability": {
              "l1_data_gas": 384,
              "l1_gas": 0,
              "l2_gas": 0
            },
            "n_memory_holes": 0,
            "n_steps": 26124,
            "total_gas_consumed": {
              "l1_data_gas": 384,
              "l1_gas": 0,
              "l2_gas": 3514560
            }
          },
          "execution_status": "SUCCEEDED",
          "l2_to_l1_messages": [],
          "transaction_hash": "0x41c1b94456cb6c340ceaee425cc73a969bb5b5a55ec2406ffda14d7c5b0e1a9",
          "transaction_index": 0
        }
    );

    let receipt: ConfirmedReceipt = serde_json::from_value(json).unwrap();

    // Assert transaction_hash
    assert_eq!(
        receipt.transaction_hash,
        felt!("0x41c1b94456cb6c340ceaee425cc73a969bb5b5a55ec2406ffda14d7c5b0e1a9")
    );

    assert_eq!(receipt.transaction_index, 0);
    assert_eq!(receipt.execution_status, Some(ExecutionStatus::Succeeded));
    assert_eq!(receipt.actual_fee, felt!("0x25756bce009ac0"));
    assert_eq!(receipt.revert_error, None);

    // execution_resources
    assert!(receipt.execution_resources.is_some());
    let exec_resources = receipt.execution_resources.unwrap();

    // VM resources
    use katana_primitives::execution::BuiltinName;
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::bitwise),
        Some(&1)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::ec_op),
        Some(&6)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::pedersen),
        Some(&30)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::poseidon),
        Some(&112)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::range_check),
        Some(&1031)
    );
    assert_eq!(exec_resources.vm_resources.n_steps, 26124);
    assert_eq!(exec_resources.vm_resources.n_memory_holes, 0);

    // data availability resources
    assert!(exec_resources.data_availability.is_some());
    let da_resources = exec_resources.data_availability.unwrap();
    assert_eq!(da_resources.l1_data_gas, 384);
    assert_eq!(da_resources.l1_gas, 0);

    // total gas consumed
    assert!(exec_resources.total_gas_consumed.is_some());
    let gas_consumed = exec_resources.total_gas_consumed.unwrap();
    assert_eq!(gas_consumed.l1_data_gas, 384);
    assert_eq!(gas_consumed.l1_gas, 0);
    assert_eq!(gas_consumed.l2_gas, 3514560);

    assert_eq!(receipt.l1_to_l2_consumed_message, None);
    assert_eq!(receipt.l2_to_l1_messages.len(), 0);
    assert_eq!(receipt.events.len(), 2);

    // First event
    let event1 = &receipt.events[0];
    assert_eq!(
        event1.from_address,
        address!("0x1f51ffc7121aea2093fb95a5995b072b72c89172fbd82acf62498f388e3bde7")
    );
    similar_asserts::assert_eq!(
        event1.keys,
        vec![
            felt!("0x1dcde06aabdbca2f80aa51392b345d7549d7757aa855f7e37f5d335ac8243b1"),
            felt!("0x5f505603ff517ab44cd17b531be418a8f29930fbb5877144afc6a090696564b")
        ]
    );
    similar_asserts::assert_eq!(event1.data, vec![felt!("0x1"), felt!("0x0")]);

    // Second event
    let event2 = &receipt.events[1];
    assert_eq!(
        event2.from_address,
        address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
    );
    similar_asserts::assert_eq!(
        event2.keys,
        vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")]
    );

    similar_asserts::assert_eq!(
        event2.data,
        vec![
            felt!("0x438761135b0d7a930694309d61c7e659789921c96434e5b78253af2c5b5b2b5"),
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            felt!("0x25756bce009ac0"),
            felt!("0x0")
        ]
    );
}

#[test]
fn receipt_serde_reverted() {
    let json = json!({
         "actual_fee": "0xc48f5bda51840",
         "events": [
           {
             "data": [
               "0x6f7f65cb1f4f4cbd5a67ce5320fff2d4454c38c0b35d0692abfd024d7de67",
               "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
               "0xc48f5bda51840",
               "0x0"
             ],
             "from_address": "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
             "keys": [
               "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
             ]
           }
         ],
         "execution_resources": {
           "builtin_instance_counter": {
             "ecdsa_builtin": 1,
             "pedersen_builtin": 4,
             "poseidon_builtin": 20,
             "range_check_builtin": 185
           },
           "data_availability": {
             "l1_data_gas": 128,
             "l1_gas": 0,
             "l2_gas": 0
           },
           "n_memory_holes": 0,
           "n_steps": 7149,
           "total_gas_consumed": {
             "l1_data_gas": 128,
             "l1_gas": 0,
             "l2_gas": 776320
           }
         },
         "execution_status": "REVERTED",
         "l2_to_l1_messages": [],
         "revert_error": "Insufficient max L2Gas: max amount: 1152640, actual used: 1192640.",
         "transaction_hash": "0x2a52bf48199ce526b4597c4c84de01b32f403722480cc747045ec869a6f2274",
         "transaction_index": 16
       }
    );

    let receipt: ConfirmedReceipt = serde_json::from_value(json).unwrap();

    assert_eq!(
        receipt.transaction_hash,
        felt!("0x2a52bf48199ce526b4597c4c84de01b32f403722480cc747045ec869a6f2274")
    );
    assert_eq!(receipt.transaction_index, 16);
    assert_eq!(receipt.execution_status, Some(ExecutionStatus::Reverted));
    assert_eq!(receipt.actual_fee, felt!("0xc48f5bda51840"));
    assert_eq!(
        receipt.revert_error,
        Some("Insufficient max L2Gas: max amount: 1152640, actual used: 1192640.".to_string())
    );

    assert!(receipt.execution_resources.is_some());
    let exec_resources = receipt.execution_resources.unwrap();
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::ecdsa),
        Some(&1)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::pedersen),
        Some(&4)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::poseidon),
        Some(&20)
    );
    assert_eq!(
        exec_resources.vm_resources.builtin_instance_counter.get(&BuiltinName::range_check),
        Some(&185)
    );
    assert_eq!(exec_resources.vm_resources.n_steps, 7149);
    assert_eq!(exec_resources.vm_resources.n_memory_holes, 0);

    // data availability resources
    assert!(exec_resources.data_availability.is_some());
    let da_resources = exec_resources.data_availability.unwrap();
    assert_eq!(da_resources.l1_data_gas, 128);
    assert_eq!(da_resources.l1_gas, 0);

    // total gas consumed
    assert!(exec_resources.total_gas_consumed.is_some());
    let gas_consumed = exec_resources.total_gas_consumed.unwrap();
    assert_eq!(gas_consumed.l1_data_gas, 128);
    assert_eq!(gas_consumed.l1_gas, 0);
    assert_eq!(gas_consumed.l2_gas, 776320);

    assert_eq!(receipt.l1_to_l2_consumed_message, None);
    assert_eq!(receipt.l2_to_l1_messages.len(), 0);
    assert_eq!(receipt.events.len(), 1);

    // First event
    let event1 = &receipt.events[0];
    assert_eq!(
        event1.from_address,
        address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
    );
    similar_asserts::assert_eq!(
        event1.keys,
        vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")]
    );
    similar_asserts::assert_eq!(
        event1.data,
        vec![
            felt!("0x6f7f65cb1f4f4cbd5a67ce5320fff2d4454c38c0b35d0692abfd024d7de67"),
            felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
            felt!("0xc48f5bda51840"),
            felt!("0x0")
        ]
    );
}
