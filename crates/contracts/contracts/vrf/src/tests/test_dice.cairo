use cartridge_vrf::mocks::vrf_consumer_mock::IVrfConsumerMockDispatcherTrait;
use cartridge_vrf::{IVrfProviderDispatcherTrait, Source};
use snforge_std::{start_cheat_caller_address, stop_cheat_caller_address};
use stark_vrf::ecvrf::{ECVRFImpl, Point, Proof};
use super::common::{CONSUMER1, PLAYER1, setup, submit_random};

// private key: 420
// {"public_key_x":"0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4","public_key_y":"0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232"}

const SEED: felt252 = 0x148c79e57bc0ce25e079841517ce9d3499094429644b7288df57a4a16b27721;

// curl -X POST -H "Content-Type: application/json" -d '{"seed":
// ["0x148c79e57bc0ce25e079841517ce9d3499094429644b7288df57a4a16b27721"]}'
// http://0.0.0.0:3000/stark_vrf
pub fn proof() -> Proof {
    Proof {
        gamma: Point {
            x: 0x1b2146bdf5ef6d13d36e1731bcca759f5cc75baef29cd8d2db2d05356913304,
            y: 0x2ece98350f2ba9dfa54c7cead948912c5c6ab609afcc4a2af726094418c3318,
        },
        c: 0x3c6d3f3af11babb561b90643cff6a115db6ee91b017d0b5e8b716f1ec8eb0a2,
        s: 0x372acefcab4435982285495fbfa4ce6a8608e5b1dfdf9a31ac7df73a92ca202,
        sqrt_ratio_hint: 0x192ddce2f2872355bec6d18b4c6bb8033df94aa57e42442d78d41a9c91ce425,
    }
}


const SEED_FROM_SALT: felt252 = 0x767EBFD1241683397A6CB06FDE012811BB27FD6E768D7A4BB8670ED10DF95C0;

// curl -X POST -H "Content-Type: application/json" -d '{"seed":
// ["0x767EBFD1241683397A6CB06FDE012811BB27FD6E768D7A4BB8670ED10DF95C0"]}'
// http://0.0.0.0:3000/stark_vrf
pub fn proof_from_salt() -> Proof {
    Proof {
        gamma: Point {
            x: 0x28473f4cad1406e83a766a1137281340b93600661af4eda228d5f73ae4e0fe9,
            y: 0x36bf0d2884aa739d5e419f1eb9fdf4af61c489169830d748ae0bbc7707b95ae,
        },
        c: 0x137ed85cd1ae3b25d6f9c2e3e23912cde001696fb4c0caba403967ecdab4bb4,
        s: 0x74b0e76d8cfffc8a4755f4bcdefd3b1339d68e2c06b1072b36cfb2661fd4a27,
        sqrt_ratio_hint: 0x29523a3636251c7085188108d8912577076ab6337892a3aa492dea4015d3a0c,
    }
}

#[test]
fn test_dice() {
    let setup = setup();

    setup.provider.request_random(CONSUMER1, Source::Nonce(PLAYER1));

    submit_random(setup.provider, SEED, proof());

    let is_vrf_call = setup.provider.is_vrf_call();
    assert(is_vrf_call, 'is_vrf_call0 should be true');

    let consume_count = setup.provider.get_consume_count();
    assert(consume_count == 0, 'consume_count should be 0');

    // PLAYER1 call dice, CONSUMER1 is caller of consume_random
    start_cheat_caller_address(setup.consumer1.contract_address, PLAYER1);

    let dice1 = setup.consumer1.dice();
    assert(dice1 == 4, 'dice1 should be 4');

    let consume_count = setup.provider.get_consume_count();
    assert(consume_count == 1, 'consume_count should be 1');

    let dice2 = setup.consumer1.dice();
    assert(dice2 == 3, 'dice2 should be 3');

    let consume_count = setup.provider.get_consume_count();
    assert(consume_count == 2, 'consume_count should be 2');

    stop_cheat_caller_address(setup.consumer1.contract_address);

    let is_vrf_call = setup.provider.is_vrf_call();
    assert(is_vrf_call, 'is_vrf_call1 should be true');

    setup.provider.assert_consumed(SEED);

    let is_vrf_call = setup.provider.is_vrf_call();
    assert(!is_vrf_call, 'is_vrf_call2 should be false');
}

#[test]
#[should_panic(expected: 'VrfProvider: not consumed')]
fn test_not_consuming__must_consume() {
    let setup = setup();

    // noop just here for example
    setup.provider.request_random(CONSUMER1, Source::Nonce(PLAYER1));

    submit_random(setup.provider, SEED, proof());

    // PLAYER1 dont consume
    start_cheat_caller_address(setup.consumer1.contract_address, PLAYER1);
    setup.consumer1.not_consuming();

    stop_cheat_caller_address(setup.consumer1.contract_address);
    setup.provider.assert_consumed(SEED);
}

#[test]
fn test_dice_with_salt() {
    let setup = setup();

    // noop just here for example
    setup.provider.request_random(CONSUMER1, Source::Salt('salt'));

    submit_random(setup.provider, SEED_FROM_SALT, proof_from_salt());

    // PLAYER1 call dice_with_salt, CONSUMER1 is caller of consume_random
    start_cheat_caller_address(setup.consumer1.contract_address, PLAYER1);
    let dice1 = setup.consumer1.dice_with_salt();
    assert(dice1 == 2, 'dice1 should be 2');
    stop_cheat_caller_address(setup.consumer1.contract_address);

    setup.provider.assert_consumed(SEED);
}

#[test]
#[should_panic(expected: 'VrfProvider: not fulfilled')]
fn test_dice_with_salt__wrong_proof() {
    let setup = setup();

    // noop just here for example
    setup.provider.request_random(CONSUMER1, Source::Salt('salt'));

    submit_random(setup.provider, SEED, proof());

    // PLAYER1 consume
    start_cheat_caller_address(setup.consumer1.contract_address, PLAYER1);
    let _dice1 = setup.consumer1.dice_with_salt();
}
