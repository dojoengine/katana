use assert_matches::assert_matches;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{
    AllResourceBoundsMapping, L1GasResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping,
    Tip,
};
use katana_primitives::{address, felt, transaction as primitives};
use katana_rpc_types::transaction::{
    RpcDeclareTx, RpcDeployAccountTx, RpcInvokeTx, RpcTx, RpcTxWithHash,
};
use katana_rpc_types::{
    RpcDeclareTxV0, RpcDeclareTxV1, RpcDeclareTxV2, RpcDeclareTxV3, RpcDeployAccountTxV1,
    RpcDeployAccountTxV3, RpcDeployTx, RpcInvokeTxV0, RpcInvokeTxV1, RpcInvokeTxV3, RpcL1HandlerTx,
};
use serde_json::Value;

mod fixtures;

#[test]
fn invoke_transaction() {
    let json = fixtures::test_data::<Value>("v0.9/transactions/v3/invoke.json");
    let tx: RpcTxWithHash = serde_json::from_value(json.clone()).unwrap();
    let RpcTxWithHash { transaction_hash, transaction } = tx.clone();

    assert_eq!(
        transaction_hash,
        felt!("0x47ad063062288bcb3d6e9d56428625206e6b8ca0a0414389836c337badc4678")
    );

    assert_matches!(&transaction, RpcTx::Invoke(RpcInvokeTx::V3(tx)) => {
        assert_eq!(tx.sender_address, address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"));
        assert_eq!(tx.nonce, felt!("0x18dd67"));

        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L1);

        assert_eq!(tx.calldata,  vec![
            felt!("0x1"),
            felt!("0x7a6f440b8632053d221c4bfa7fe9d7896a51f9752981934d8e1fe7c106e910b"),
            felt!("0x5df99ae77df976b4f0e5cf28c7dcfe09bd6e81aab787b19ac0c08e03d928cf"),
            felt!("0x1"),
            felt!("0x2f8"),
        ]);

        assert_eq!(tx.signature,  vec![
            felt!("0x5518243e023820fbaa574ac03291aef4a8336e55a8d5336c4980e6923776cb3"),
            felt!("0x2190f9408198423328147f15a1e816c5f81b0a46792655825ace229cc78626e"),
        ]);

        assert_eq!(tx.account_deployment_data, vec![]);

        assert_eq!(tx.tip, Tip::new(0x5f5e100));
        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l2_gas, ResourceBounds { max_amount: 0x5f5e100, max_price_per_unit: 0xba43b7400 });
        });
    });

    let serialized = serde_json::to_value(&tx).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn declare_transaction() {
    let json = fixtures::test_data::<Value>("v0.9/transactions/v3/declare.json");
    let tx: RpcTxWithHash = serde_json::from_value(json.clone()).unwrap();
    let RpcTxWithHash { transaction_hash, transaction } = tx.clone();

    assert_eq!(
        transaction_hash,
        felt!("0x5d5522b21bd46a27eff36e10d431cf974df5a68bcc260164f40ece60c898d82")
    );

    assert_matches!(&transaction, RpcTx::Declare(RpcDeclareTx::V3(tx)) => {
        assert_eq!(tx.sender_address, address!("0x352057331d5ad77465315d30b98135ddb815b86aa485d659dfeef59a904f88d"));
        assert_eq!(tx.nonce, felt!("0x47824e"));

        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L1);

        assert_eq!(tx.class_hash, felt!("0x32e39f7822757c912b254026368ca4ddba0011f59917bf889c11280e2f3d555"));
        assert_eq!(tx.compiled_class_hash, felt!("0x1b5d213e134d827c5325a530a94e33f06f0fd96b75af21802809923929f2871"));

        assert_eq!(tx.signature,  vec![
            felt!("0x4cc6d149973a9f546634064662e3c63c865236e8fc2efe34d15cae733e17cb0"),
            felt!("0x6edb6d5f8ae526a3f27bf3fc694c02e8dc36d74650ff49d0703c212616fbbc9"),
        ]);

        assert_eq!(tx.account_deployment_data, vec![]);
        assert_eq!(tx.paymaster_data, vec![]);

        assert_eq!(tx.tip, Tip::new(0x0));
        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l2_gas, ResourceBounds { max_amount: 0x6c76900, max_price_per_unit: 0xba43b7400 });
        });

    });

    let serialized = serde_json::to_value(&tx).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn deploy_account_transaction() {
    let json = fixtures::test_data::<Value>("v0.9/transactions/v3/deploy_account.json");
    let tx: RpcTxWithHash = serde_json::from_value(json.clone()).unwrap();
    let RpcTxWithHash { transaction_hash, transaction } = tx.clone();

    assert_eq!(
        transaction_hash,
        felt!("0x7ed8d22d2da21c072a61661888f15cb4039ee2370711d7a82fb142fa805941d")
    );

    assert_matches!(&transaction, RpcTx::DeployAccount(RpcDeployAccountTx::V3(tx)) => {
        assert_eq!(tx.nonce, felt!("0x0"));

        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L1);

        assert_eq!(tx.class_hash, felt!("0x345354e2d801833068de73d1a2028e2f619f71045dd5229e79469fa7f598038"));
        assert_eq!(tx.contract_address_salt, felt!("0x30cf3785921671a5367b01813d9ca1db2bc8539abf6e4bf31e8b78d0b5b37fc"));

        assert_eq!(tx.constructor_calldata, vec![
            felt!("0x406a640b3b70dad390d661c088df1fbaeb5162a07d57cf29ba794e2b0e3c804"),
        ]);

        assert_eq!(tx.signature,  vec![
            felt!("0x5ab916cc1aeea8cc25f2d993dc6a385a732a328fab831e7a56dc1e71964d742"),
            felt!("0x271941e40c7bd1560b2d45091af4ec1cca5b7221d16b1c3983f672a2225c07"),
        ]);

        assert_eq!(tx.paymaster_data, vec![]);

        assert_eq!(tx.tip, Tip::new(0x5f5e100));
        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
            assert_eq!(bounds.l2_gas, ResourceBounds { max_amount: 0x5f5e100, max_price_per_unit: 0xba43b7400 });
        });
    });

    let serialized = serde_json::to_value(&tx).unwrap();
    assert_eq!(serialized, json);
}

#[test]
fn l1_handler_transaction() {
    let json = fixtures::test_data::<Value>("v0.9/transactions/v3/l1_handler.json");
    let tx: RpcTxWithHash = serde_json::from_value(json.clone()).unwrap();
    let RpcTxWithHash { transaction_hash, transaction } = tx.clone();

    assert_eq!(
        transaction_hash,
        felt!("0x5e7e5063a7106ba707f3084cdfe77b3dee2f08f4a3c6b37665077499ed9259f")
    );

    assert_matches!(&transaction, RpcTx::L1Handler(tx) => {
        assert_eq!(tx.nonce, felt!("0x5bfd"));
        assert_eq!(tx.version, felt!("0x0"));

        assert_eq!(tx.contract_address, address!("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"));
        assert_eq!(tx.entry_point_selector, felt!("0x1b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb19"));

        assert_eq!(tx.calldata, vec![
            felt!("0x8453fc6cd1bcfe8d4dfc069c400b433054d47bdc"),
            felt!("0x455448"),
            felt!("0xa4a9453ef47a820c51082759b3daeaa97a08e7c3"),
            felt!("0x3f8b686ceb9c230eb6682140c2472d35c6ce8dba2d05a40c4c049facf6e2e3b"),
            felt!("0x3d66bede065244"),
            felt!("0x0"),
        ]);
    });

    let serialized = serde_json::to_value(&tx).unwrap();
    assert_eq!(serialized, json);
}

// ========================================================================================
// Tests for conversions between RPC and primitives types with round-trip verification
// ========================================================================================

#[test]
fn rpc_to_primitives_invoke_v3() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x123456"),
        transaction: RpcTx::Invoke(RpcInvokeTx::V3(RpcInvokeTxV3 {
            sender_address: address!("0x123"),
            calldata: vec![felt!("0x1"), felt!("0x2"), felt!("0x3")],
            signature: vec![felt!("0xabc"), felt!("0xdef")],
            nonce: felt!("0x5"),
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0x1000, max_price_per_unit: 0x100 },
                l2_gas: ResourceBounds { max_amount: 0x2000, max_price_per_unit: 0x200 },
                l1_data_gas: ResourceBounds { max_amount: 0x3000, max_price_per_unit: 0x300 },
            }),
            tip: Tip::new(0x50),
            paymaster_data: vec![felt!("0x999")],
            account_deployment_data: vec![felt!("0x888"), felt!("0x777")],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L2,
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x123456"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Invoke(primitives::InvokeTx::V3(tx)) => {
        assert_eq!(tx.sender_address, address!("0x123"));
        assert_eq!(tx.calldata, vec![felt!("0x1"), felt!("0x2"), felt!("0x3")]);
        assert_eq!(tx.signature, vec![felt!("0xabc"), felt!("0xdef")]);
        assert_eq!(tx.nonce, felt!("0x5"));

        // Check resource bounds
        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x1000);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x100);
            assert_eq!(bounds.l2_gas.max_amount, 0x2000);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x200);
            assert_eq!(bounds.l1_data_gas.max_amount, 0x3000);
            assert_eq!(bounds.l1_data_gas.max_price_per_unit, 0x300);
        });

        assert_eq!(tx.tip, 0x50);
        assert_eq!(tx.paymaster_data, vec![felt!("0x999")]);
        assert_eq!(tx.account_deployment_data, vec![felt!("0x888"), felt!("0x777")]);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L2);
    });

    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_invoke_v1() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0xabc123"),
        transaction: RpcTx::Invoke(RpcInvokeTx::V1(RpcInvokeTxV1 {
            sender_address: address!("0x456"),
            calldata: vec![felt!("0xa"), felt!("0xb")],
            max_fee: 0x1000,
            signature: vec![felt!("0x111"), felt!("0x222")],
            nonce: felt!("0x10"),
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0xabc123"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Invoke(primitives::InvokeTx::V1(tx)) => {
        assert_eq!(tx.sender_address, address!("0x456"));
        assert_eq!(tx.calldata, vec![felt!("0xa"), felt!("0xb")]);
        assert_eq!(tx.max_fee, 0x1000);
        assert_eq!(tx.signature, vec![felt!("0x111"), felt!("0x222")]);
        assert_eq!(tx.nonce, felt!("0x10"));
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_invoke_v0() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0xdef456"),
        transaction: RpcTx::Invoke(RpcInvokeTx::V0(RpcInvokeTxV0 {
            max_fee: 0x2000,
            signature: vec![felt!("0x333")],
            contract_address: address!("0x789"),
            entry_point_selector: felt!("0xaaa"),
            calldata: vec![felt!("0xc"), felt!("0xd"), felt!("0xe")],
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0xdef456"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Invoke(primitives::InvokeTx::V0(tx)) => {
        assert_eq!(tx.contract_address, address!("0x789"));
        assert_eq!(tx.entry_point_selector, felt!("0xaaa"));
        assert_eq!(tx.calldata, vec![felt!("0xc"), felt!("0xd"), felt!("0xe")]);
        assert_eq!(tx.signature, vec![felt!("0x333")]);
        assert_eq!(tx.max_fee, 0x2000);
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_declare_v3() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x999888"),
        transaction: RpcTx::Declare(RpcDeclareTx::V3(RpcDeclareTxV3 {
            sender_address: address!("0xabc"),
            compiled_class_hash: felt!("0x111222"),
            signature: vec![felt!("0x444"), felt!("0x555")],
            nonce: felt!("0x20"),
            class_hash: felt!("0x666777"),
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0x100, max_price_per_unit: 0x10 },
                l2_gas: ResourceBounds { max_amount: 0x200, max_price_per_unit: 0x20 },
                l1_data_gas: ResourceBounds { max_amount: 0x300, max_price_per_unit: 0x30 },
            }),
            tip: Tip::new(0x99),
            paymaster_data: vec![felt!("0xfff")],
            account_deployment_data: vec![felt!("0xeee")],
            nonce_data_availability_mode: DataAvailabilityMode::L2,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x999888"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Declare(primitives::DeclareTx::V3(tx)) => {
        assert_eq!(tx.sender_address, address!("0xabc"));
        assert_eq!(tx.compiled_class_hash, felt!("0x111222"));
        assert_eq!(tx.signature, vec![felt!("0x444"), felt!("0x555")]);
        assert_eq!(tx.nonce, felt!("0x20"));
        assert_eq!(tx.class_hash, felt!("0x666777"));

        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x100);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x10);
            assert_eq!(bounds.l2_gas.max_amount, 0x200);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x20);
            assert_eq!(bounds.l1_data_gas.max_amount, 0x300);
            assert_eq!(bounds.l1_data_gas.max_price_per_unit, 0x30);
        });

        assert_eq!(tx.tip, 0x99);
        assert_eq!(tx.paymaster_data, vec![felt!("0xfff")]);
        assert_eq!(tx.account_deployment_data, vec![felt!("0xeee")]);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L2);
        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L1);
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_declare_v2() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x777666"),
        transaction: RpcTx::Declare(RpcDeclareTx::V2(RpcDeclareTxV2 {
            sender_address: address!("0xdef"),
            compiled_class_hash: felt!("0x888999"),
            max_fee: 0x3000,
            signature: vec![felt!("0x666")],
            nonce: felt!("0x30"),
            class_hash: felt!("0xaaabbb"),
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x777666"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Declare(primitives::DeclareTx::V2(tx)) => {
        assert_eq!(tx.sender_address, address!("0xdef"));
        assert_eq!(tx.compiled_class_hash, felt!("0x888999"));
        assert_eq!(tx.max_fee, 0x3000);
        assert_eq!(tx.signature, vec![felt!("0x666")]);
        assert_eq!(tx.nonce, felt!("0x30"));
        assert_eq!(tx.class_hash, felt!("0xaaabbb"));
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_declare_v1() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x555444"),
        transaction: RpcTx::Declare(RpcDeclareTx::V1(RpcDeclareTxV1 {
            sender_address: address!("0x123abc"),
            max_fee: 0x4000,
            signature: vec![felt!("0x777"), felt!("0x888")],
            nonce: felt!("0x40"),
            class_hash: felt!("0xcccddd"),
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x555444"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Declare(primitives::DeclareTx::V1(tx)) => {
        assert_eq!(tx.sender_address, address!("0x123abc"));
        assert_eq!(tx.max_fee, 0x4000);
        assert_eq!(tx.signature, vec![felt!("0x777"), felt!("0x888")]);
        assert_eq!(tx.nonce, felt!("0x40"));
        assert_eq!(tx.class_hash, felt!("0xcccddd"));
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_declare_v0() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x333222"),
        transaction: RpcTx::Declare(RpcDeclareTx::V0(RpcDeclareTxV0 {
            sender_address: address!("0x456def"),
            max_fee: 0x5000,
            signature: vec![felt!("0x999")],
            class_hash: felt!("0xeeefff"),
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x333222"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Declare(primitives::DeclareTx::V0(tx)) => {
        assert_eq!(tx.sender_address, address!("0x456def"));
        assert_eq!(tx.max_fee, 0x5000);
        assert_eq!(tx.signature, vec![felt!("0x999")]);
        assert_eq!(tx.class_hash, felt!("0xeeefff"));
    });

    // Round-trip conversion
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_deploy_account_v3() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x111222333"),
        transaction: RpcTx::DeployAccount(RpcDeployAccountTx::V3(RpcDeployAccountTxV3 {
            signature: vec![felt!("0xaaa"), felt!("0xbbb")],
            nonce: felt!("0x50"),
            contract_address_salt: felt!("0xccc"),
            constructor_calldata: vec![felt!("0xddd"), felt!("0xeee")],
            class_hash: felt!("0xfff111"),
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0x400, max_price_per_unit: 0x40 },
                l2_gas: ResourceBounds { max_amount: 0x500, max_price_per_unit: 0x50 },
                l1_data_gas: ResourceBounds { max_amount: 0x600, max_price_per_unit: 0x60 },
            }),
            tip: Tip::new(0x88),
            paymaster_data: vec![felt!("0x222333")],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x111222333"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::DeployAccount(primitives::DeployAccountTx::V3(tx)) => {
        assert_eq!(tx.signature, vec![felt!("0xaaa"), felt!("0xbbb")]);
        assert_eq!(tx.nonce, felt!("0x50"));
        assert_eq!(tx.contract_address_salt, felt!("0xccc"));
        assert_eq!(tx.constructor_calldata, vec![felt!("0xddd"), felt!("0xeee")]);
        assert_eq!(tx.class_hash, felt!("0xfff111"));

        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x400);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x40);
            assert_eq!(bounds.l2_gas.max_amount, 0x500);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x50);
            assert_eq!(bounds.l1_data_gas.max_amount, 0x600);
            assert_eq!(bounds.l1_data_gas.max_price_per_unit, 0x60);
        });

        assert_eq!(tx.tip, 0x88);
        assert_eq!(tx.paymaster_data, vec![felt!("0x222333")]);
        assert_eq!(tx.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(tx.fee_data_availability_mode, DataAvailabilityMode::L1);
    });

    // Round-trip conversion (note: contract_address field is lost as it's not in RPC)
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_deploy_account_v1() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x444555666"),
        transaction: RpcTx::DeployAccount(RpcDeployAccountTx::V1(RpcDeployAccountTxV1 {
            max_fee: 0x6000,
            signature: vec![felt!("0x111222")],
            nonce: felt!("0x60"),
            contract_address_salt: felt!("0x333444"),
            constructor_calldata: vec![felt!("0x555")],
            class_hash: felt!("0x666777"),
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x444555666"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::DeployAccount(primitives::DeployAccountTx::V1(tx)) => {
        assert_eq!(tx.max_fee, 0x6000);
        assert_eq!(tx.signature, vec![felt!("0x111222")]);
        assert_eq!(tx.nonce, felt!("0x60"));
        assert_eq!(tx.contract_address_salt, felt!("0x333444"));
        assert_eq!(tx.constructor_calldata, vec![felt!("0x555")]);
        assert_eq!(tx.class_hash, felt!("0x666777"));
    });

    // Round-trip conversion (note: contract_address field is lost as it's not in RPC)
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_l1_handler() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0x777888999"),
        transaction: RpcTx::L1Handler(RpcL1HandlerTx {
            version: felt!("0x0"),
            nonce: felt!("0x70"),
            contract_address: address!("0xaaabbbccc"),
            entry_point_selector: felt!("0xdddeee"),
            calldata: vec![felt!("0xfff"), felt!("0x111"), felt!("0x222")],
        }),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0x777888999"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::L1Handler(tx) => {
        assert_eq!(tx.version, felt!("0x0"));
        assert_eq!(tx.nonce, felt!("0x70"));
        assert_eq!(tx.contract_address, address!("0xaaabbbccc"));
        assert_eq!(tx.entry_point_selector, felt!("0xdddeee"));
        assert_eq!(tx.calldata, vec![felt!("0xfff"), felt!("0x111"), felt!("0x222")]);
    });

    // Round-trip conversion (note: paid_fee_on_l1 and message_hash are lost as they're not in RPC)
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_deploy() {
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0xaaabbbccc"),
        transaction: RpcTx::Deploy(RpcDeployTx {
            version: felt!("0x1"),
            contract_address_salt: felt!("0x333"),
            constructor_calldata: vec![felt!("0x444"), felt!("0x555")],
            class_hash: felt!("0x666"),
        }),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_eq!(primitives_tx.hash, felt!("0xaaabbbccc"));

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Deploy(tx) => {
        assert_eq!(tx.version, felt!("0x1"));
        assert_eq!(tx.contract_address_salt, felt!("0x333"));
        assert_eq!(tx.constructor_calldata, vec![felt!("0x444"), felt!("0x555")]);
        assert_eq!(tx.class_hash, felt!("0x666"));
    });

    // Round-trip conversion (note: contract_address field is lost as it's not in RPC)
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}

#[test]
fn rpc_to_primitives_resource_bounds_l1_only() {
    // Test the case where only L1 gas bounds are set (legacy support)
    let rpc_tx = RpcTxWithHash {
        transaction_hash: felt!("0xdeadbeef"),
        transaction: RpcTx::Invoke(RpcInvokeTx::V3(RpcInvokeTxV3 {
            sender_address: address!("0x123"),
            calldata: vec![],
            signature: vec![],
            nonce: felt!("0x1"),
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0x1000, max_price_per_unit: 0x100 },
                l2_gas: ResourceBounds { max_amount: 0x99, max_price_per_unit: 0x88 },
            }),
            tip: Tip::new(0),
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        })),
    };

    let primitives_tx: primitives::TxWithHash = rpc_tx.clone().into();

    assert_matches!(&primitives_tx.transaction, primitives::Tx::Invoke(primitives::InvokeTx::V3(tx)) => {
        // When l2_gas and l1_data_gas are zero, it should be converted to L1Gas variant
        assert_matches!(&tx.resource_bounds, ResourceBoundsMapping::L1Gas(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x1000);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x100);
            assert_eq!(bounds.l2_gas.max_amount, 0x99);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x88);
        });
    });

    // Round-trip conversion should maintain the legacy format
    let rpc_tx_roundtrip: RpcTxWithHash = primitives_tx.into();
    assert_eq!(rpc_tx, rpc_tx_roundtrip);
}
