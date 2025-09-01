use assert_matches::assert_matches;
use katana_primitives::fee::Tip;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::transaction::{
    RpcDeclareTx, RpcDeployAccountTx, RpcInvokeTx, RpcTx, RpcTxWithHash,
};
use serde_json::Value;
use starknet::core::types::{DataAvailabilityMode, ResourceBounds};

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
        assert_eq!(tx.resource_bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l2_gas, ResourceBounds { max_amount: 0x5f5e100, max_price_per_unit: 0xba43b7400 });
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
        assert_eq!(tx.resource_bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l2_gas, ResourceBounds { max_amount: 0x6c76900, max_price_per_unit: 0xba43b7400 });
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
        assert_eq!(tx.resource_bounds.l1_data_gas, ResourceBounds { max_amount: 0x2710, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l1_gas, ResourceBounds { max_amount: 0x249f0, max_price_per_unit: 0x8d79883d20000 });
        assert_eq!(tx.resource_bounds.l2_gas, ResourceBounds { max_amount: 0x5f5e100, max_price_per_unit: 0xba43b7400 });
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
