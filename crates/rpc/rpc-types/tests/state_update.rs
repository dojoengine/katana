use std::collections::{BTreeMap, BTreeSet};

use assert_matches::assert_matches;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::state_update::{
    ConfirmedStateUpdate, StateUpdate, PreConfirmedStateUpdate,
};
use serde_json::Value;

mod fixtures;

/// Creates a BTreeMap from key-value pairs.
///
/// ## Example
/// ```
/// btreemap! {
///     "key1", "value1",
///     "key2", "value2",
/// };
/// ```
macro_rules! map {
    ($($key:expr, $value:expr),* $(,)?) => {
        BTreeMap::from_iter([$( ($key, $value) ),*])
    };
}

#[test]
fn preconfirmed_state_update() {
    let json = fixtures::test_data::<Value>("v0.9/state-updates/preconfirmed_state_update.json");

    let state_update: PreConfirmedStateUpdate = serde_json::from_value(json.clone()).unwrap();
    let as_enum: StateUpdate = serde_json::from_value(json.clone()).unwrap();
    assert_matches!(as_enum, StateUpdate::PreConfirmed(as_enum_update) => {
        similar_asserts::assert_eq!(as_enum_update, state_update);
    });

    let PreConfirmedStateUpdate { old_root, ref state_diff } = state_update;
    assert_eq!(
        old_root,
        felt!("0x6a59de5353d4050a800fd240020d014653d950df357ffa14319ee809a65427a")
    );
    assert_eq!(state_diff.deprecated_declared_classes, BTreeSet::new());
    assert_eq!(state_diff.replaced_classes, map!());
    assert_eq!(
        state_diff.deployed_contracts,
        map! {
            address!("0x39afe989b57658db2e56b808b32515fcf4a03fb6ab49c7ceb6ce81e2405aced"), felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93")
        }
    );
    assert_eq!(
        state_diff.nonces,
        map! {
            address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"), felt!("0x1cf4a1")
        }
    );
    assert_eq!(
        state_diff.storage_diffs,
        map! {
            address!("0x39afe989b57658db2e56b808b32515fcf4a03fb6ab49c7ceb6ce81e2405aced"), map! {
                felt!("0x3b28019ccfdbd30ffc65951d94bb85c9e2b8434111a000b5afd533ce65f57a4"), felt!("0x7075626c69635f6b6579")
            },
            address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"), map! {
                felt!("0x1bc06e01c8edbc022b2a63178e2e2f2bc1bc498a6b2fa743a0e99ec2be6ac16"), felt!("0x64eefbf434e26244877"),
                felt!("0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a"), felt!("0x2326bff25acc7269684a1"),
            }
        }
    );

    let serialized = serde_json::to_value(&state_update).unwrap();
    similar_asserts::assert_eq!(serialized, json);
}

#[test]
fn confirmed_state_update() {
    let json = fixtures::test_data::<Value>("v0.9/state-updates/confirmed_state_update.json");

    let state_update: ConfirmedStateUpdate = serde_json::from_value(json.clone()).unwrap();
    let as_enum: StateUpdate = serde_json::from_value(json.clone()).unwrap();
    assert_matches!(as_enum, StateUpdate::Update(as_enum_update) => {
        similar_asserts::assert_eq!(as_enum_update, state_update);
    });

    let ConfirmedStateUpdate { block_hash, new_root, old_root, ref state_diff } = state_update;
    assert_eq!(
        block_hash,
        felt!("0x1149bb6e1ac2c692d82ed6a0320c184f482a6ba0416618e90fd347d57b966c1")
    );
    assert_eq!(
        old_root,
        felt!("0x1dc40c7863db601e1153e99e019b5164b6558b5970eb2f0f706bdebf172a110")
    );
    assert_eq!(
        new_root,
        felt!("0x1ee480a6575a5a03d2ce3f0638257afe1bd2a343533ab66c2799832fcbb4e47")
    );
    assert_eq!(state_diff.deprecated_declared_classes, BTreeSet::new());
    assert_eq!(state_diff.replaced_classes, map! {});
    assert_eq!(state_diff.declared_classes, map! {});
    assert_eq!(state_diff.deployed_contracts, map! {});
    assert_eq!(
        state_diff.nonces,
        map! {
            address!("0x143fe26927dd6a302522ea1cd6a821ab06b3753194acee38d88a85c93b3cbc6"), felt!("0x2731a"),
            address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"), felt!("0x1cf3aa")
        }
    );
    assert_eq!(
        state_diff.storage_diffs,
        map! {
            address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"), map! {
                felt!("0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a"), felt!("0x2326ba58dc57e8fc6ce61"),
                felt!("0x72366c0a6ada44989c6fa9cb62d89fa92d30d8cd4ea5820aee50bdc62d164e7"), felt!("0x1cbf017f90a7dbf60f"),
                felt!("0x1bc06e01c8edbc022b2a63178e2e2f2bc1bc498a6b2fa743a0e99ec2be6ac16"), felt!("0x64f37e628f67b4891f7")
            },
            address!("0x79c0bc2a03570241c27235a2dca7696a658cbdaae0bad5762e30204b2791aba"), map! {
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e7"), felt!("0xa10a2ba1476"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695ea"), felt!("0x68b9227e"),
                felt!("0x329c7ad716328e6d50f9ca0db199b7680edd1f9888de9e870e256b4d829dd57"), felt!("0x124aa"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e8"), felt!("0x1c8bab"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e9"), felt!("0x68b92279"),
                felt!("0x322b1a6785274ae49206e79f3e95babd2a2a4083c719465edc3e7d137bdfd6d"), felt!("0x678d2a6f800"),
                felt!("0xefb0884a0332bee3218e5114a1f5a8b94b7f3a0aa4b620ecd81bc37c64598f"), felt!("0x5c3cd04"),
            },
            address!("0x2"), map! {
                felt!("0x0"), felt!("0x14d0191"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e7"), felt!("0x14d018d"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695ea"), felt!("0x14d0190"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e9"), felt!("0x14d018f"),
                felt!("0x6ced507d324a1d8a26febca5869e489cd569b848a5b60ceb54da6dc2ab695e8"), felt!("0x14d018e"),
            },
            address!("0x1"), map! {
                felt!("0x1c8ba1"), felt!("0x6d33bd9dc2f9c1f96a91314a3198d190a33431b425db0db8b71db14eee333e7"),
            }
        }
    );

    let serialized = serde_json::to_value(&state_update).unwrap();
    similar_asserts::assert_eq!(serialized, json);
}
