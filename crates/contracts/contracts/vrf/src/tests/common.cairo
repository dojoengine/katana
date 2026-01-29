use cartridge_vrf::mocks::vrf_consumer_mock::IVrfConsumerMockDispatcher;
use cartridge_vrf::vrf_provider::vrf_provider_component::{
    IVrfProviderDispatcher, IVrfProviderDispatcherTrait, PublicKey,
};
use openzeppelin::utils::serde::SerializedAppend;
use openzeppelin_testing::constants::{AUTHORIZED, AsAddressImpl, OWNER};
use openzeppelin_testing::deployment::{declare_and_deploy_at, deploy_another_at};
use snforge_std::{
    start_cheat_caller_address, start_cheat_max_fee_global, stop_cheat_caller_address,
};
use stark_vrf::ecvrf::Proof;
use starknet::ContractAddress;


pub const PROVIDER: ContractAddress = 'PROVIDER'.as_address();
pub const CONSUMER1: ContractAddress = 'CONSUMER1'.as_address();
pub const CONSUMER2: ContractAddress = 'CONSUMER2'.as_address();
pub const PLAYER1: ContractAddress = 'PLAYER1'.as_address();

#[derive(Drop, Copy, Clone)]
pub struct SetupResult {
    pub provider: IVrfProviderDispatcher,
    pub consumer1: IVrfConsumerMockDispatcher,
    pub consumer2: IVrfConsumerMockDispatcher,
}

// lauch vrf-server : cargo run -r -- -s 420
// pubkey.x =0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4
// pubkey.y =0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232

pub fn setup() -> SetupResult {
    let mut provider_calldata = array![];
    provider_calldata.append_serde(OWNER);
    provider_calldata
        .append_serde(
            PublicKey {
                x: 0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4,
                y: 0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232,
            },
        );

    declare_and_deploy_at("VrfProvider", PROVIDER, provider_calldata);

    let mut consumer_calldata = array![];
    consumer_calldata.append_serde(PROVIDER);

    declare_and_deploy_at("VrfConsumer", CONSUMER1, consumer_calldata.clone());
    deploy_another_at(CONSUMER1, CONSUMER2, consumer_calldata);

    start_cheat_max_fee_global(10000000000000000);

    SetupResult {
        provider: IVrfProviderDispatcher { contract_address: PROVIDER },
        consumer1: IVrfConsumerMockDispatcher { contract_address: CONSUMER1 },
        consumer2: IVrfConsumerMockDispatcher { contract_address: CONSUMER2 },
    }
}

pub fn submit_random(provider: IVrfProviderDispatcher, seed: felt252, proof: Proof) {
    start_cheat_caller_address(provider.contract_address, AUTHORIZED);
    provider.submit_random(seed, proof);
    stop_cheat_caller_address(provider.contract_address);
}
