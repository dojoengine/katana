use std::{str::FromStr, sync::Arc};

use blockifier::transaction::{
    account_transaction::AccountTransaction, transaction_execution::Transaction,
};
use starknet::core::{
    types::{CallFunction, FieldElement},
    utils::get_selector_from_name,
};
use starknet_api::{
    block::BlockNumber,
    core::ContractAddress,
    core::{EntryPointSelector, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
    state::StorageKey,
    transaction::{Calldata, InvokeTransactionV1, TransactionHash},
};

use crate::{
    constants::FEE_ERC20_CONTRACT_ADDRESS,
    starknet::{transaction::FunctionCall, Config, StarknetWrapper},
};

#[test]
fn test_sensible_default_state() {
    let starknet = StarknetWrapper::new(Config);

    assert_eq!(
        starknet
            .state
            .lock()
            .unwrap()
            .state
            .address_to_class_hash
            .len(),
        4
    );
    assert_eq!(
        starknet
            .state
            .lock()
            .unwrap()
            .state
            .class_hash_to_class
            .len(),
        3
    );
    // assert prefunded account has balance
    assert_eq!(
        starknet.state.lock().unwrap().state.storage_view.get(&(
            ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
            StorageKey(patricia_key!(
                "0x6037c05be3813c2957296d06b53f340b87a97b1cf38aa2966fda3d6f4a9e50a"
            )),
        )),
        Some(&stark_felt!("0x10000000000000"))
    );
}

#[test]
fn test_creating_blocks() {
    let mut starknet = StarknetWrapper::new(Config);
    starknet.generate_latest_block();
    starknet.generate_latest_block();
    starknet.generate_latest_block();

    assert_eq!(starknet.blocks.hash_to_num.len(), 3);
    assert_eq!(starknet.blocks.num_to_blocks.len(), 3);
    assert_eq!(starknet.blocks.current_height, BlockNumber(3));

    let block0 = starknet.blocks.get_by_number(0).unwrap();
    let block1 = starknet.blocks.get_by_number(1).unwrap();
    let last_block = starknet.blocks.get_last_block().unwrap();

    assert_eq!(block0.transactions(), &[]);
    assert_eq!(block0.block_number(), BlockNumber(0));
    assert_eq!(block1.block_number(), BlockNumber(1));
    assert_eq!(last_block.block_number(), BlockNumber(2));
}

#[test]
fn test_add_transaction() {
    let mut starknet = StarknetWrapper::new(Config);
    starknet.generate_pending_block();

    let tx_hash = TransactionHash(stark_felt!("0x1234"));
    let transaction =
        Transaction::AccountTransaction(AccountTransaction::Invoke(InvokeTransactionV1 {
            ..Default::default()
        }));

    starknet
        .handle_transaction(transaction)
        .expect("must execute succesfully");

    assert!(
        starknet.transactions.transactions.get(&tx_hash).is_some(),
        "tx with hash 0x1234 doesn't exist"
    );
}

#[test]
fn test_function_call() {
    let starknet = StarknetWrapper::new(Config);

    let call = FunctionCall {
        calldata: Calldata(Arc::new(vec![stark_felt!("0x111111111")])),
        contract_address: ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
        entry_point_selector: EntryPointSelector(StarkFelt::from(
            get_selector_from_name("balanceOf").unwrap(),
        )),
    };

    let res = starknet.call(call);

    assert!(res.is_ok(), "call must succeed");
    assert_eq!(
        res.unwrap().execution.retdata.0[0],
        stark_felt!("0x10000000000000"),
        "user must have balance of 0x10000000000000"
    );
}
