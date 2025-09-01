use assert_matches::assert_matches;
use katana_primitives::receipt::Event;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::receipt::{ReceiptBlockInfo, RpcTxReceipt, TxReceiptWithBlockInfo};
use serde_json::Value;
use starknet::core::types::{ExecutionResult, Hash256, PriceUnit, TransactionFinalityStatus};

mod fixtures;

#[test]
fn invoke_confirmed_receipt() {
    let json = fixtures::test_data::<Value>("v0.9/receipts/v3/invoke.json");
    let receipt: TxReceiptWithBlockInfo = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&receipt.block, ReceiptBlockInfo::Block { block_hash, block_number } => {
        assert_eq!(*block_number, 1832699);
        assert_eq!(
            *block_hash,
            felt!("0x2b0639a2a01ad67ec2496be09bf6ee4c5a125b6b27a09447be8405e40ff3511")
        );
    });

    assert_matches!(&receipt.receipt, RpcTxReceipt::Invoke(receipt) => {
        assert_eq!(
            receipt.transaction_hash,
            felt!("0x47ad063062288bcb3d6e9d56428625206e6b8ca0a0414389836c337badc4678")
        );
        assert_eq!(receipt.finality_status, TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);

        assert_eq!(receipt.actual_fee.amount, felt!("0x8d13974114d80"));
        assert_eq!(receipt.actual_fee.unit, PriceUnit::Fri);

        assert_eq!(receipt.events.len(), 1);
        assert_eq!(receipt.events[0], Event {
            from_address: address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            keys: vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
            data: vec![
                felt!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"),
                felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
                felt!("0x8d13974114d80"),
                felt!("0x0"),
            ],
        });

        assert_eq!(receipt.execution_resources.l1_data_gas, 128);
        assert_eq!(receipt.execution_resources.l2_gas, 800595);
        assert_eq!(receipt.execution_resources.l1_gas, 0);
    });

    let serialized = serde_json::to_value(&receipt).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn declare_confirmed_receipt() {
    let json = fixtures::test_data::<Value>("v0.9/receipts/v3/declare.json");
    let receipt: TxReceiptWithBlockInfo = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&receipt.block, ReceiptBlockInfo::Block { block_hash, block_number } => {
        assert_eq!(*block_number, 1831872);
        assert_eq!(
            *block_hash,
            felt!("0x1b6660888efffde8cf3064bdc2d7d826e63850ba6acf67f9b628af472335a6b")
        );
    });

    assert_matches!(&receipt.receipt, RpcTxReceipt::Declare(receipt) => {
        assert_eq!(
            receipt.transaction_hash,
            felt!("0x5d5522b21bd46a27eff36e10d431cf974df5a68bcc260164f40ece60c898d82")
        );
        assert_eq!(receipt.finality_status, TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);

        assert_eq!(receipt.actual_fee.amount, felt!("0x97d2e342fa59c0"));
        assert_eq!(receipt.actual_fee.unit, PriceUnit::Fri);

        assert_eq!(receipt.events.len(), 1);
        assert_eq!(receipt.events[0], Event {
            from_address: address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            keys: vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
            data: vec![
                felt!("0x352057331d5ad77465315d30b98135ddb815b86aa485d659dfeef59a904f88d"),
                felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
                felt!("0x97d2e342fa59c0"),
                felt!("0x0"),
            ],
        });

        assert_eq!(receipt.execution_resources.l1_data_gas, 192);
        assert_eq!(receipt.execution_resources.l2_gas, 14244865);
        assert_eq!(receipt.execution_resources.l1_gas, 0);
    });

    let serialized = serde_json::to_value(&receipt).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn deploy_account_confirmed_receipt() {
    let json = fixtures::test_data::<Value>("v0.9/receipts/v3/deploy_account.json");
    let receipt: TxReceiptWithBlockInfo = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&receipt.block, ReceiptBlockInfo::Block { block_hash, block_number } => {
        assert_eq!(*block_number, 1831872);
        assert_eq!(
            *block_hash,
            felt!("0x1b6660888efffde8cf3064bdc2d7d826e63850ba6acf67f9b628af472335a6b")
        );
    });

    assert_matches!(&receipt.receipt, RpcTxReceipt::DeployAccount(receipt) => {
        assert_eq!(
            receipt.transaction_hash,
            felt!("0x7ed8d22d2da21c072a61661888f15cb4039ee2370711d7a82fb142fa805941d")
        );
        assert_eq!(receipt.finality_status, TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);

        assert_eq!(receipt.actual_fee.amount, felt!("0x7327522833800"));
        assert_eq!(receipt.actual_fee.unit, PriceUnit::Fri);

        assert_eq!(receipt.events.len(), 1);
        assert_eq!(receipt.events[0], Event {
            from_address: address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            keys: vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
            data: vec![
                felt!("0x583d1ea19135ccdea1e42c2772687b765bba714b1f2104213c5a3919a2574b"),
                felt!("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
                felt!("0x7327522833800"),
                felt!("0x0"),
            ],
        });

        assert_eq!(receipt.execution_resources.l1_data_gas, 256);
        assert_eq!(receipt.execution_resources.l2_gas, 653485);
        assert_eq!(receipt.execution_resources.l1_gas, 0);

        assert_eq!(receipt.contract_address, address!("0x583d1ea19135ccdea1e42c2772687b765bba714b1f2104213c5a3919a2574b"));
    });

    let serialized = serde_json::to_value(&receipt).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn l1_handler_confirmed_receipt() {
    let json = fixtures::test_data::<Value>("v0.9/receipts/v3/l1_handler.json");
    let receipt: TxReceiptWithBlockInfo = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(&receipt.block, ReceiptBlockInfo::Block { block_hash, block_number } => {
        assert_eq!(*block_number, 1832108);
        assert_eq!(
            *block_hash,
            felt!("0x5768d8dc56af8832b48619e7e901fec74d4addfb5364d8ddf0cffdcceb50769")
        );
    });

    assert_matches!(&receipt.receipt, RpcTxReceipt::L1Handler(receipt) => {
        assert_eq!(
            receipt.transaction_hash,
            felt!("0x5e7e5063a7106ba707f3084cdfe77b3dee2f08f4a3c6b37665077499ed9259f")
        );
        assert_eq!(receipt.finality_status, TransactionFinalityStatus::AcceptedOnL2);
        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);

        assert_eq!(receipt.actual_fee.amount, felt!("0x0"));
        assert_eq!(receipt.actual_fee.unit, PriceUnit::Wei);

        assert_eq!(receipt.events.len(), 2);
        assert_eq!(receipt.events[0], Event {
            from_address: address!("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
            keys: vec![felt!("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
            data: vec![
                felt!("0x0"),
                felt!("0x3f8b686ceb9c230eb6682140c2472d35c6ce8dba2d05a40c4c049facf6e2e3b"),
                felt!("0x3d66bede065244"),
                felt!("0x0"),
            ],
        });
        assert_eq!(receipt.events[1], Event {
            from_address: address!("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"),
            keys: vec![
                felt!("0x374396cb322ab5ffd35ddb8627514609289d22c07d039ead5327782f61bb833"),
                felt!("0x455448"),
                felt!("0x3f8b686ceb9c230eb6682140c2472d35c6ce8dba2d05a40c4c049facf6e2e3b"),
            ],
            data: vec![
                felt!("0x3d66bede065244"),
                felt!("0x0"),
            ],
        });

        assert_eq!(receipt.execution_resources.l1_data_gas, 160);
        assert_eq!(receipt.execution_resources.l2_gas, 617160);
        assert_eq!(receipt.execution_resources.l1_gas, 20163);

        assert_eq!(receipt.message_hash, Hash256::from_hex("0xb720c23367e1ebcb73f909ce13c3773d74c9a06b212d6dca1e6f55c3d4b44fde").unwrap());
    });

    let serialized = serde_json::to_value(&receipt).unwrap();
    assert_eq!(serialized, json);
}
