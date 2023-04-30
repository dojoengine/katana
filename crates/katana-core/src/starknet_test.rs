use std::sync::Arc;

use blockifier::transaction::{
    account_transaction::AccountTransaction, transaction_execution::Transaction,
};
use starknet::core::{types::TransactionStatus, utils::get_selector_from_name};
use starknet_api::{
    block::BlockNumber,
    core::ContractAddress,
    core::{EntryPointSelector, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
    transaction::{Calldata, InvokeTransactionV1, TransactionHash},
};

use crate::{
    constants::{DEFAULT_PREFUNDED_ACCOUNT_BALANCE, FEE_ERC20_CONTRACT_ADDRESS},
    starknet::{transaction::FunctionCall, Config, StarknetWrapper},
};

#[test]
fn test_creating_blocks() {
    let mut starknet = StarknetWrapper::new(Config);
    starknet.generate_pending_block();

    assert_eq!(
        starknet.blocks.current_height,
        BlockNumber(0),
        "pending block should not be added to the chain"
    );

    starknet.generate_latest_block();
    starknet.generate_latest_block();
    starknet.generate_latest_block();

    assert_eq!(starknet.blocks.hash_to_num.len(), 3);
    assert_eq!(starknet.blocks.num_to_blocks.len(), 3);
    assert_eq!(starknet.blocks.current_height, BlockNumber(3));

    let block0 = starknet.blocks.get_by_number(BlockNumber(0)).unwrap();
    let block1 = starknet.blocks.get_by_number(BlockNumber(1)).unwrap();
    let last_block = starknet.blocks.get_lastest().unwrap();

    assert_eq!(block0.transactions(), &[]);
    assert_eq!(block0.block_number(), BlockNumber(0));
    assert_eq!(block1.block_number(), BlockNumber(1));
    assert_eq!(last_block.block_number(), BlockNumber(2));
}

#[test]
fn test_add_transaction() {
    unimplemented!("TODO: implement test for adding transaction");
}

#[test]
fn test_add_reverted_transaction() {
    let mut starknet = StarknetWrapper::new(Config);
    starknet.generate_pending_block();

    let transaction_hash = TransactionHash(stark_felt!("0x1234"));
    let transaction =
        Transaction::AccountTransaction(AccountTransaction::Invoke(InvokeTransactionV1 {
            transaction_hash,
            ..Default::default()
        }));

    starknet.handle_transaction(transaction);

    let block = starknet.generate_latest_block();

    assert_eq!(block.transactions().len(), 0);

    let tx = starknet.transactions.transactions.get(&transaction_hash);

    assert_eq!(
        starknet.transactions.transactions.len(),
        1,
        "transaction must be stored even if execution fail"
    );
    assert_eq!(tx.unwrap().block_hash, None);
    assert_eq!(tx.unwrap().block_number, None);
    assert_eq!(tx.unwrap().status, TransactionStatus::Rejected);
}

#[test]
fn test_function_call() {
    let starknet = StarknetWrapper::new(Config);
    let account = &starknet.predeployed_accounts.accounts[0]
        .account_address
        .0
        .key();

    let call = FunctionCall {
        calldata: Calldata(Arc::new(vec![**account])),
        contract_address: ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
        entry_point_selector: EntryPointSelector(StarkFelt::from(
            get_selector_from_name("balanceOf").unwrap(),
        )),
    };

    let res = starknet.call(call);

    assert!(res.is_ok(), "call must succeed");
    assert_eq!(
        res.unwrap().execution.retdata.0[0],
        stark_felt!(DEFAULT_PREFUNDED_ACCOUNT_BALANCE),
    );
}
