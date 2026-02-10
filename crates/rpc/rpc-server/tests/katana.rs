use std::path::PathBuf;

use assert_matches::assert_matches;
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{felt, Felt};
use katana_rpc_api::katana::KatanaApiClient;
use katana_rpc_types::broadcasted::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx,
};
use katana_rpc_types::receipt::{RpcDeployAccountTxReceipt, RpcTxReceipt};
use katana_utils::node::test_config;
use katana_utils::TestNode;
use starknet::accounts::{
    Account, AccountFactory, ConnectedAccount, OpenZeppelinAccountFactory as OZAccountFactory,
};
use starknet::signers::{LocalWallet, SigningKey};

mod common;

use common::{Erc20Contract, Uint256};

fn convert_broadcasted_tx<T, U>(tx: T) -> U
where
    T: serde::Serialize,
    U: serde::de::DeserializeOwned,
{
    let value = serde_json::to_value(tx).expect("failed to serialize tx");
    serde_json::from_value(value).expect("failed to deserialize tx")
}

#[tokio::test]
async fn katana_add_transactions_return_receipts() {
    let mut config = test_config();
    config.dev.fee = false;
    config.dev.account_validation = false;

    let sequencer = TestNode::new_with_config(config).await;
    let rpc_client = sequencer.rpc_http_client();
    let provider = sequencer.starknet_rpc_client();
    let account = sequencer.account();

    // -----------------------------------------------------------------------
    // katana_addInvokeTransaction

    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    let fee = erc20.transfer(&recipient, &amount).estimate_fee().await.unwrap();
    let nonce = account.get_nonce().await.unwrap();

    let prepared_invoke = erc20
        .transfer(&recipient, &amount)
        .nonce(nonce)
        .l1_gas(fee.l1_gas_consumed)
        .l1_gas_price(fee.l1_gas_price)
        .l2_gas(fee.l2_gas_consumed)
        .l2_gas_price(fee.l2_gas_price)
        .l1_data_gas(fee.l1_data_gas_consumed)
        .l1_data_gas_price(fee.l1_data_gas_price)
        .tip(0)
        .prepared()
        .unwrap();

    let invoke_tx: BroadcastedInvokeTx =
        convert_broadcasted_tx(prepared_invoke.get_invoke_request(false, false).await.unwrap());

    let invoke_receipt =
        KatanaApiClient::add_invoke_transaction(&rpc_client, invoke_tx).await.unwrap();
    assert_matches!(invoke_receipt.receipt, RpcTxReceipt::Invoke(_));

    // -----------------------------------------------------------------------
    // katana_addDeclareTransaction

    let path: PathBuf = PathBuf::from("tests/test_data/cairo1_contract.json");
    let (contract, compiled_class_hash) =
        common::prepare_contract_declaration_params(&path).unwrap();

    let fee = account
        .declare_v3(contract.clone().into(), compiled_class_hash)
        .estimate_fee()
        .await
        .unwrap();
    let nonce = account.get_nonce().await.unwrap();

    let prepared_declare = account
        .declare_v3(contract.into(), compiled_class_hash)
        .nonce(nonce)
        .l1_gas(fee.l1_gas_consumed)
        .l1_gas_price(fee.l1_gas_price)
        .l2_gas(fee.l2_gas_consumed)
        .l2_gas_price(fee.l2_gas_price)
        .l1_data_gas(fee.l1_data_gas_consumed)
        .l1_data_gas_price(fee.l1_data_gas_price)
        .tip(0)
        .prepared()
        .unwrap();

    let declare_tx: BroadcastedDeclareTx =
        convert_broadcasted_tx(prepared_declare.get_declare_request(false, false).await.unwrap());

    let declare_receipt =
        KatanaApiClient::add_declare_transaction(&rpc_client, declare_tx).await.unwrap();
    assert_matches!(declare_receipt.receipt, RpcTxReceipt::Declare(_));

    // -----------------------------------------------------------------------
    // katana_addDeployAccountTransaction

    let chain_id = provider.chain_id().await.unwrap();
    let signer = LocalWallet::from(SigningKey::from_random());
    let class_hash = provider
        .get_class_hash_at(BlockIdOrTag::PreConfirmed, account.address().into())
        .await
        .unwrap();
    let salt = felt!("0x123");

    let factory =
        OZAccountFactory::new(class_hash, chain_id, &signer, account.provider()).await.unwrap();

    let deploy_account_tx = factory.deploy_v3(salt);
    let deployed_address = deploy_account_tx.address();

    let fee = deploy_account_tx.estimate_fee().await.unwrap();
    let nonce = deploy_account_tx.fetch_nonce().await.unwrap();

    let prepared_deploy = factory
        .deploy_v3(salt)
        .nonce(nonce)
        .l1_gas(fee.l1_gas_consumed)
        .l1_gas_price(fee.l1_gas_price)
        .l2_gas(fee.l2_gas_consumed)
        .l2_gas_price(fee.l2_gas_price)
        .l1_data_gas(fee.l1_data_gas_consumed)
        .l1_data_gas_price(fee.l1_data_gas_price)
        .tip(0)
        .prepared()
        .unwrap();

    let deploy_tx: BroadcastedDeployAccountTx =
        convert_broadcasted_tx(prepared_deploy.get_deploy_request(false, false).await.unwrap());

    let deploy_receipt =
        KatanaApiClient::add_deploy_account_transaction(&rpc_client, deploy_tx).await.unwrap();

    assert_matches!(
        deploy_receipt.receipt,
        RpcTxReceipt::DeployAccount(RpcDeployAccountTxReceipt { contract_address, .. })
        => assert_eq!(contract_address, deployed_address)
    );
}
