use cartridge_vrf::mocks::vrf_consumer_mock::IVrfConsumerMockDispatcher;
use openzeppelin::upgrades::interface::{
    IUpgradeAndCallDispatcher, IUpgradeAndCallDispatcherTrait, IUpgradeableDispatcher,
    IUpgradeableDispatcherTrait,
};
use openzeppelin::utils::serde::SerializedAppend;
use openzeppelin_testing::constants::{AsAddressImpl, OWNER};
use openzeppelin_testing::deployment::declare_and_deploy_at;
use snforge_std::{
    CheatSpan, DeclareResultTrait, cheat_caller_address, declare, load, map_entry_address,
    start_cheat_block_timestamp_global, start_cheat_caller_address, start_cheat_chain_id_global,
    stop_cheat_caller_address, store,
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
pub const VRF_PROVIDER: ContractAddress = 'VRF_PROVIDER'.as_address();
pub const CONSUMER_ACCOUNT: ContractAddress = 'CONSUMER_ACCOUNT'.as_address();
pub const CONSUMER: ContractAddress = 'CONSUMER'.as_address();


pub const VRF_PROVIDER_PUBLIC_KEY: PublicKey = PublicKey {
    x: 0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4,
    y: 0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232,
};

pub const VRF_ACCOUNT_PUBLIC_KEY: felt252 =
    0x14584bef56c98fbb91aba84c20724937d5b5d2d6e5a49b60e6c3a19696fad5f;

#[derive(Drop, Copy, Clone)]
pub struct SetupResult {
    pub vrf_account: IVrfAccountDispatcher,
    pub consumer_account: IAccountDispatcher,
    pub consumer: IVrfConsumerMockDispatcher,
}

#[test]
pub fn test_upgrade() {
    start_cheat_block_timestamp_global(1);
    start_cheat_chain_id_global('SN_SEPOLIA');

    // DEPLOY VrfProvider
    let mut provider_calldata = array![];
    provider_calldata.append_serde(OWNER);
    provider_calldata.append_serde(VRF_PROVIDER_PUBLIC_KEY);

    declare_and_deploy_at("VrfProvider", VRF_PROVIDER, provider_calldata);

    // set some VrfProvider_nonces storage
    store(
        VRF_PROVIDER,
        map_entry_address(
            selector!("VrfProvider_nonces"), // storage variable name
            array![CONSUMER.into()].span() // map key
        ),
        array![21_000_000].span(),
    );

    // DECLARE VrfProviderUpgrader
    let vrf_upgrader_class_hash = declare("VrfProviderUpgrader")
        .unwrap()
        .contract_class()
        .class_hash;

    // UPGRADE VrfProvider with VrfProviderUpgrader
    start_cheat_caller_address(VRF_PROVIDER, OWNER);
    let vrf_disp = IUpgradeableDispatcher { contract_address: VRF_PROVIDER };
    vrf_disp.upgrade(*vrf_upgrader_class_hash);
    stop_cheat_caller_address(VRF_PROVIDER);

    let owner = load(VRF_PROVIDER, selector!("Ownable_owner"), 1);
    assert!(*owner.at(0) == OWNER.into(), "invalid owner");

    // DECLARE VrfAccount
    let vrf_account_class_hash = declare("VrfAccount").unwrap().contract_class().class_hash;

    // UPGRADE VrfProviderUpgrader to VrfAccount & CALL initializer
    cheat_caller_address(VRF_PROVIDER, OWNER, CheatSpan::TargetCalls(1));
    let vrf_disp = IUpgradeAndCallDispatcher { contract_address: VRF_PROVIDER };
    vrf_disp
        .upgrade_and_call(
            *vrf_account_class_hash,
            selector!("initializer"),
            array![VRF_ACCOUNT_PUBLIC_KEY].span(),
        );

    //
    // CHECKS
    //

    // account is initialized with pubkey
    let vrf_account_public_key = load(VRF_PROVIDER, selector!("Account_public_key"), 1);
    assert!(*vrf_account_public_key.at(0) == VRF_ACCOUNT_PUBLIC_KEY, "invalid Account_public_key");

    // same VrfProvider_pubkey
    let vrf_provider_public_key = load(VRF_PROVIDER, selector!("VrfProvider_pubkey"), 2);
    assert!(
        *vrf_provider_public_key.at(0) == VRF_PROVIDER_PUBLIC_KEY.x, "invalid VrfProvider_pubkey.x",
    );
    assert!(
        *vrf_provider_public_key.at(1) == VRF_PROVIDER_PUBLIC_KEY.y, "invalid VrfProvider_pubkey.y",
    );

    // same VrfProvider_nonces
    let nonce = load(
        VRF_PROVIDER,
        map_entry_address(
            selector!("VrfProvider_nonces"), // start of the read memory chunk
            array![CONSUMER.into()].span() // map key
        ),
        1 // length of the read memory chunk
    );
    assert!(*nonce.at(0) == 21_000_000, "invalid VrfProvider_nonces");
}

