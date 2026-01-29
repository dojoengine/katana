use cartridge_vrf::mocks::vrf_consumer_mock::IVrfConsumerMockDispatcher;
use openzeppelin::utils::serde::SerializedAppend;
use openzeppelin_testing::constants::AsAddressImpl;
use openzeppelin_testing::deployment::declare_and_deploy_at;
use snforge_std::{
    start_cheat_block_timestamp_global, start_cheat_caller_address, start_cheat_chain_id_global,
    stop_cheat_caller_address,
};
use starknet::ContractAddress;
use starknet::account::Call;
use crate::{PublicKey, Source};


// lauch vrf-server : cargo run -r -- -s 420
// pubkey.x =0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4
// pubkey.y =0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232

// starknetPublicKey1 0x111 0x14584bef56c98fbb91aba84c20724937d5b5d2d6e5a49b60e6c3a19696fad5f
// starknetPublicKey2 0x222 0x5cba218680f68130296ac34ed343d6186a98744c6ef66c39345fdaefe06c4d5

#[starknet::interface]
pub trait IAccount<T> {
    // SRC6 Account
    fn __execute__(self: @T, calls: Array<Call>);
    fn __validate__(self: @T, calls: Array<Call>) -> felt252;
    fn is_valid_signature(self: @T, hash: felt252, signature: Array<felt252>) -> felt252;
    // SRC? Outside Execution
}

#[starknet::interface]
pub trait IVrfAccount<T> {
    // SRC6 Account
    fn __execute__(self: @T, calls: Array<Call>);
    fn __validate__(self: @T, calls: Array<Call>) -> felt252;
    fn is_valid_signature(self: @T, hash: felt252, signature: Array<felt252>) -> felt252;

    // SRC? Outside Execution

    // VRF
    fn set_vrf_public_key(ref self: T, vrf_public_key: PublicKey);
    fn request_random(self: @T, caller: ContractAddress, source: Source);
}

pub const ZERO_ADDRESS: ContractAddress = 0.as_address();
pub const ANY_CALLER: ContractAddress = 'ANY_CALLER'.as_address();
pub const VRF_ACCOUNT: ContractAddress = 'VRF_ACCOUNT'.as_address();
pub const CONSUMER_ACCOUNT: ContractAddress = 'CONSUMER_ACCOUNT'.as_address();
pub const CONSUMER: ContractAddress = 'CONSUMER'.as_address();

#[derive(Drop, Copy, Clone)]
pub struct SetupResult {
    pub vrf_account: IVrfAccountDispatcher,
    pub consumer_account: IAccountDispatcher,
    pub consumer: IVrfConsumerMockDispatcher,
}

pub fn setup() -> SetupResult {
    start_cheat_block_timestamp_global(1);
    start_cheat_chain_id_global('SN_SEPOLIA');

    // VRF_ACCOUNT
    let mut vrf_account_calldata = array![
        0x14584bef56c98fbb91aba84c20724937d5b5d2d6e5a49b60e6c3a19696fad5f,
    ];

    declare_and_deploy_at("VrfAccount", VRF_ACCOUNT, vrf_account_calldata);

    start_cheat_caller_address(VRF_ACCOUNT, VRF_ACCOUNT);
    IVrfAccountDispatcher { contract_address: VRF_ACCOUNT }
        .set_vrf_public_key(
            PublicKey {
                x: 0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4,
                y: 0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232,
            },
        );
    stop_cheat_caller_address(VRF_ACCOUNT);

    // CONSUMER_ACCOUNT
    let mut consumer_account_calldata = array![
        0x5cba218680f68130296ac34ed343d6186a98744c6ef66c39345fdaefe06c4d5,
    ];
    declare_and_deploy_at("AccountMock", CONSUMER_ACCOUNT, consumer_account_calldata);

    // CONSUMER
    let mut consumer_calldata = array![];
    consumer_calldata.append_serde(VRF_ACCOUNT);

    declare_and_deploy_at("VrfConsumer", CONSUMER, consumer_calldata.clone());

    SetupResult {
        vrf_account: IVrfAccountDispatcher { contract_address: VRF_ACCOUNT },
        consumer_account: IAccountDispatcher { contract_address: CONSUMER_ACCOUNT },
        consumer: IVrfConsumerMockDispatcher { contract_address: CONSUMER },
    }
}

