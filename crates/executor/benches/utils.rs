use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::block::GasPrices;
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::{ExecutableTxWithHash, InvokeTx, InvokeTxV1};
use katana_primitives::Felt;
use starknet::macros::{felt, selector};

pub fn tx() -> ExecutableTxWithHash {
    let invoke = InvokeTx::V1(InvokeTxV1 {
        sender_address: felt!("0x1").into(),
        calldata: vec![
            DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
            selector!("transfer"),
            Felt::THREE,
            felt!("0x100"),
            Felt::ONE,
            Felt::ZERO,
        ],
        max_fee: 10_000,
        ..Default::default()
    });

    ExecutableTxWithHash::new(invoke.into())
}

pub fn envs() -> BlockEnv {
    BlockEnv {
        l1_gas_prices: GasPrices::MIN,
        sequencer_address: felt!("0x1337").into(),
        ..Default::default()
    }
}
