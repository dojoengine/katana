use std::collections::BTreeMap;

use katana_feeder_gateway::types::{
    ConfirmedReceipt, DeclaredContract, DeployedContract, ExecutionStatus, StateDiff, StorageDiff,
};
use katana_primitives::execution::BuiltinName;
use katana_primitives::{address, eth_address, felt, ContractAddress};
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
          "actual_fee": "0x220c76e10ea291c8",
          "events": [
            {
              "data": [
                "0x7c730bd4054155d6f6d19db15b24bbf76087f77814a66f80a43ab2b09173b25",
                "0x51ba9be967d17aaafac92f9bc7ca4b035dfd3c4a97b32be1773f63e27b0526a",
                "0x5f012ad4535e8000",
                "0x0"
              ],
              "from_address": "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
              "keys": [
                "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
              ]
            },
            {
              "data": [
                "0x7c730bd4054155d6f6d19db15b24bbf76087f77814a66f80a43ab2b09173b25",
                "0x0",
                "0xe60e960",
                "0x0"
              ],
              "from_address": "0x53c91253bc9682c04929ca02ed00b3e423f6710d2ee7e0d5ebb06f3ecf368a8",
              "keys": [
                "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
              ]
            }
          ],
          "execution_resources": {
            "builtin_instance_counter": {
              "ec_op_builtin": 9,
              "pedersen_builtin": 16,
              "poseidon_builtin": 60,
              "range_check_builtin": 908
            },
            "data_availability": {
              "l1_data_gas": 576,
              "l1_gas": 0,
              "l2_gas": 0
            },
            "n_memory_holes": 0,
            "n_steps": 29184,
            "total_gas_consumed": {
              "l1_data_gas": 608,
              "l1_gas": 32284,
              "l2_gas": 3574720
            }
          },
          "execution_status": "SUCCEEDED",
          "l1_to_l2_consumed_message": {
            "from_address": "0xF6080D9fbEEbcd44D89aFfBFd42F098cbFf92816",
            "nonce": "0x19983e",
            "payload": [
              "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
              "0x91fb1ab0d8d4c6c02e53d488f4c15a3717f3fec0",
              "0x58d59dc23df1146b92c929d387ec3bdba7b499ab1d164af9b688d2cbade4801",
              "0x4a817c800",
              "0x0"
            ],
            "selector": "0x1b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb19",
            "to_address": "0x5cd48fccbfd8aa2773fe22c217e808319ffcc1c5a6a463f7d8fa2da48218196"
          },
          "l2_to_l1_messages": [
            {
              "from_address": "0x5cd48fccbfd8aa2773fe22c217e808319ffcc1c5a6a463f7d8fa2da48218196",
              "payload": [
                "0x0",
                "0x6826be6819eee574b89b886289c454c4e9b73b6d",
                "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "0xe60e960",
                "0x0"
              ],
              "to_address": "0xF6080D9fbEEbcd44D89aFfBFd42F098cbFf92816"
            }
          ],
          "transaction_hash": "0x7d5b4a356efb2808d8b7ea83912d8e73fa8649ec34d2db6262a5b9a2826a20b",
          "transaction_index": 51
        }
    );

    let receipt: ConfirmedReceipt = serde_json::from_value(json).unwrap();

    // Assert transaction_hash
    assert_eq!(
        receipt.transaction_hash,
        felt!("0x7d5b4a356efb2808d8b7ea83912d8e73fa8649ec34d2db6262a5b9a2826a20b")
    );

    assert_eq!(receipt.transaction_index, 51);
    assert_eq!(receipt.body.execution_status, Some(ExecutionStatus::Succeeded));
    assert_eq!(receipt.body.actual_fee, felt!("0x220c76e10ea291c8"));
    assert_eq!(receipt.body.revert_error, None);

    // execution_resources

    let exec_resources = receipt.body.execution_resources.expect("`execution_resources` missing");
    let builtin_instance_counter = exec_resources.vm_resources.builtin_instance_counter;

    assert_eq!(builtin_instance_counter.len(), 4);
    assert_eq!(builtin_instance_counter.get(&BuiltinName::ec_op), Some(&9));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::pedersen), Some(&16));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::poseidon), Some(&60));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::range_check), Some(&908));

    assert_eq!(exec_resources.vm_resources.n_steps, 29184);
    assert_eq!(exec_resources.vm_resources.n_memory_holes, 0);

    let gas_consumed = exec_resources.total_gas_consumed.expect("`total_gas_consumed` missing");
    assert_eq!(gas_consumed.l1_data_gas, 608);
    assert_eq!(gas_consumed.l1_gas, 32284);
    assert_eq!(gas_consumed.l2_gas, 3574720);

    let da_resources = exec_resources.data_availability.expect("`data_availability` missing");
    assert_eq!(da_resources.l1_data_gas, 576);
    assert_eq!(da_resources.l1_gas, 0);

    // messages

    let l1_msg =
        receipt.body.l1_to_l2_consumed_message.expect("`l1_to_l2_consumed_message` missing");
    assert_eq!(l1_msg.from_address, eth_address!("0xF6080D9fbEEbcd44D89aFfBFd42F098cbFf92816"));
    assert_eq!(l1_msg.nonce, Some(felt!("0x19983e")));
    similar_asserts::assert_eq!(
        l1_msg.payload,
        vec![
            felt!("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            felt!("0x91fb1ab0d8d4c6c02e53d488f4c15a3717f3fec0"),
            felt!("0x58d59dc23df1146b92c929d387ec3bdba7b499ab1d164af9b688d2cbade4801"),
            felt!("0x4a817c800"),
            felt!("0x0"),
        ]
    );
    assert_eq!(
        l1_msg.selector,
        felt!("0x1b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb19")
    );
    assert_eq!(
        l1_msg.to_address,
        address!("0x5cd48fccbfd8aa2773fe22c217e808319ffcc1c5a6a463f7d8fa2da48218196")
    );

    assert_eq!(receipt.body.l2_to_l1_messages.len(), 1);

    let l2_msg1 = &receipt.body.l2_to_l1_messages[0];
    assert_eq!(l2_msg1.to_address, felt!("0xF6080D9fbEEbcd44D89aFfBFd42F098cbFf92816"));
    assert_eq!(
        l2_msg1.from_address,
        felt!("0x5cd48fccbfd8aa2773fe22c217e808319ffcc1c5a6a463f7d8fa2da48218196")
    );
    similar_asserts::assert_eq!(
        l2_msg1.payload,
        vec![
            felt!("0x0"),
            felt!("0x6826be6819eee574b89b886289c454c4e9b73b6d"),
            felt!("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
            felt!("0xe60e960"),
            felt!("0x0"),
        ]
    );

    // events

    assert_eq!(receipt.body.events.len(), 2);

    // First event
    let event1 = &receipt.body.events[0];
    assert_eq!(
        event1.from_address,
        address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
    );
    similar_asserts::assert_eq!(
        event1.keys,
        vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),]
    );
    similar_asserts::assert_eq!(
        event1.data,
        vec![
            felt!("0x7c730bd4054155d6f6d19db15b24bbf76087f77814a66f80a43ab2b09173b25"),
            felt!("0x51ba9be967d17aaafac92f9bc7ca4b035dfd3c4a97b32be1773f63e27b0526a"),
            felt!("0x5f012ad4535e8000"),
            felt!("0x0"),
        ]
    );

    // Second event
    let event2 = &receipt.body.events[1];
    assert_eq!(
        event2.from_address,
        address!("0x53c91253bc9682c04929ca02ed00b3e423f6710d2ee7e0d5ebb06f3ecf368a8")
    );
    similar_asserts::assert_eq!(
        event2.keys,
        vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")]
    );
    similar_asserts::assert_eq!(
        event2.data,
        vec![
            felt!("0x7c730bd4054155d6f6d19db15b24bbf76087f77814a66f80a43ab2b09173b25"),
            felt!("0x0"),
            felt!("0xe60e960"),
            felt!("0x0"),
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
    assert_eq!(receipt.body.execution_status, Some(ExecutionStatus::Reverted));
    assert_eq!(receipt.body.actual_fee, felt!("0xc48f5bda51840"));
    assert_eq!(
        receipt.body.revert_error,
        Some("Insufficient max L2Gas: max amount: 1152640, actual used: 1192640.".to_string())
    );

    // execution_resources

    let exec_resources = receipt.body.execution_resources.expect("`execution_resources` missing");
    let builtin_instance_counter = exec_resources.vm_resources.builtin_instance_counter;

    assert_eq!(builtin_instance_counter.get(&BuiltinName::ecdsa), Some(&1));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::pedersen), Some(&4));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::poseidon), Some(&20));
    assert_eq!(builtin_instance_counter.get(&BuiltinName::range_check), Some(&185));

    assert_eq!(exec_resources.vm_resources.n_steps, 7149);
    assert_eq!(exec_resources.vm_resources.n_memory_holes, 0);

    let gas_consumed = exec_resources.total_gas_consumed.expect("`total_gas_consumed` missing");
    assert_eq!(gas_consumed.l1_data_gas, 128);
    assert_eq!(gas_consumed.l1_gas, 0);
    assert_eq!(gas_consumed.l2_gas, 776320);

    let da_resources = exec_resources.data_availability.expect("`data_availability` missing");
    assert_eq!(da_resources.l1_data_gas, 128);
    assert_eq!(da_resources.l1_gas, 0);

    // messages

    assert_eq!(receipt.body.l1_to_l2_consumed_message, None);
    assert_eq!(receipt.body.l2_to_l1_messages.len(), 0);

    // events

    assert_eq!(receipt.body.events.len(), 1);

    // First event
    let event1 = &receipt.body.events[0];
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
