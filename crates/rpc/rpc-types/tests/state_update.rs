use std::collections::{BTreeMap, BTreeSet};

use assert_matches::assert_matches;
use katana_primitives::{address, felt, ContractAddress};
use katana_rpc_types::state_update::{MaybePreConfirmedStateUpdate, PreConfirmedStateUpdate};
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
    let state_update: MaybePreConfirmedStateUpdate = serde_json::from_value(json.clone()).unwrap();

    assert_matches!(state_update.clone(), MaybePreConfirmedStateUpdate::PreConfirmed(update) => {
        let PreConfirmedStateUpdate { old_root, state_diff }  = update;
        assert_eq!(old_root, felt!("0x6a59de5353d4050a800fd240020d014653d950df357ffa14319ee809a65427a"));
        assert_eq!(state_diff.deprecated_declared_classes, BTreeSet::new());
        assert_eq!(state_diff.replaced_classes, map!());
        assert_eq!(state_diff.deployed_contracts, map! {
            address!("0x39afe989b57658db2e56b808b32515fcf4a03fb6ab49c7ceb6ce81e2405aced"), felt!("0x626c15d497f8ef78da749cbe21ac3006470829ee8b5d0d166f3a139633c6a93")
        });
        assert_eq!(state_diff.nonces, map! {
            address!("0x395a96a5b6343fc0f543692fd36e7034b54c2a276cd1a021e8c0b02aee1f43"), felt!("0x1cf4a1")
        });
        assert_eq!(state_diff.storage_diffs, map! {
            address!("0x39afe989b57658db2e56b808b32515fcf4a03fb6ab49c7ceb6ce81e2405aced"), map! {
                felt!("0x3b28019ccfdbd30ffc65951d94bb85c9e2b8434111a000b5afd533ce65f57a4"), felt!("0x7075626c69635f6b6579")
            },
            address!("0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"), map! {
                felt!("0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a"), felt!("0x2326bff25acc7269684a1"),
                felt!("0x1bc06e01c8edbc022b2a63178e2e2f2bc1bc498a6b2fa743a0e99ec2be6ac16"), felt!("0x64eefbf434e26244877")
            }
        });
    });

    let serialized = serde_json::to_value(&state_update).unwrap();
    assert_eq!(serialized, json);
}
