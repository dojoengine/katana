use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use fixtures::{db, TempDb};
use katana::cli::Cli;
use katana_db::abstraction::Database;
use katana_primitives::Felt;
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::block::BlockNumberProvider;
use katana_provider::traits::state::{StateFactoryProvider, StateRootProvider};
use rstest::*;

mod fixtures;

/// Verify that historical state roots can be retrieved for specific blocks
/// Returns Ok(()) if the state root can be retrieved, Err if it cannot
fn historical_roots<Db: Database>(
    provider: &DbProvider<Db>,
    block_number: u64,
) -> Result<(Felt, Felt)> {
    let historical = provider
        .historical(block_number.into())
        .context("failed to get historical state provider")?
        .ok_or_else(|| anyhow!("Historical state not available for block {block_number}"))?;

    let classes_root =
        historical.classes_root().context("failed to get historical classes root")?;
    let contracts_root =
        historical.contracts_root().context("failed to get historical contracts root")?;

    Ok((classes_root, contracts_root))
}

/// Get the current state roots (classes and contracts)
fn latest_roots<Db: Database>(provider: &DbProvider<Db>) -> Result<(Felt, Felt)> {
    let state_provider = provider.latest().context("failed to get latest state provider")?;
    let classes_root = state_provider.classes_root().context("failed to get classes root")?;
    let contracts_root = state_provider.contracts_root().context("failed to get contracts root")?;
    Ok((classes_root, contracts_root))
}

#[rstest]
fn prune_latest_removes_all_history(db: TempDb) {
    let provider = db.provider_ro();
    let latest_block = provider.latest_number().unwrap();

    for num in 1..=latest_block {
        let (classes_root, contracts_root) = historical_roots(&provider, num).unwrap();
        assert!(classes_root != Felt::ZERO, "classes root for block {num} cannot be zero");
        assert!(contracts_root != Felt::ZERO, "contracts root for block {num} cannot be zero");
    }

    let (initial_classes_root, initial_contracts_root) = latest_roots(&provider).unwrap();
    drop(provider);

    // Will prune all historical tries (blocks < 15)
    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "latest"]).run().unwrap();

    let provider = db.provider_ro();

    // Verify historical states (0 -> 14) are no longer accessible (ie zero)
    for num in 0..=14u64 {
        // Right now, non existent tries default to zero.
        //
        // Ref:
        // * crates/storage/db/src/trie/mod.rs#43-46
        // * bonsai_trie::trie::trees::MerkleTrees::root_hash()
        let (classes_root, contracts_root) = historical_roots(&provider, num).unwrap();
        assert_eq!(classes_root, Felt::ZERO);
        assert_eq!(contracts_root, Felt::ZERO);
    }

    let (final_classes_root, final_contracts_root) = latest_roots(&provider).unwrap();
    assert_eq!(final_classes_root, initial_classes_root);
    assert_eq!(final_contracts_root, initial_contracts_root);

    // Getting historical roots for a block that is equal to the latest block, returns the latest
    // state roots.
    let (classes_root, contracts_root) = historical_roots(&provider, 15).unwrap();
    assert_eq!(final_classes_root, classes_root);
    assert_eq!(final_contracts_root, contracts_root);
}

#[rstest]
fn prune_keep_last_n_blocks(db: TempDb) {
    let provider = db.provider_ro();
    let latest_block = provider.latest_number().unwrap();

    // block -> (classes root, contracts root)
    let mut historical_roots_reg = HashMap::new();

    for num in 1..=latest_block {
        let (classes_root, contracts_root) = historical_roots(&provider, num).unwrap();
        assert!(classes_root != Felt::ZERO, "classes root for block {num} cannot be zero");
        assert!(contracts_root != Felt::ZERO, "contracts root for block {num} cannot be zero");
        historical_roots_reg.insert(num, (classes_root, contracts_root));
    }

    let (initial_classes_root, initial_contracts_root) = latest_roots(&provider).unwrap();
    drop(provider);

    let keep_last_n = 3;
    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "keep-last-n", "3"]).run().unwrap();

    let provider = db.provider_ro();
    let (final_classes_root, final_contracts_root) = latest_roots(&provider).unwrap();

    // pruned blocks (ie blocks before the cuttoff point) should be zero
    for num in 0..=(latest_block - keep_last_n) {
        // Right now, non existent tries default to zero.
        //
        // Ref:
        // * crates/storage/db/src/trie/mod.rs#43-46
        // * bonsai_trie::trie::trees::MerkleTrees::root_hash()
        let (classes_root, contracts_root) = historical_roots(&provider, num).unwrap();
        assert_eq!(classes_root, Felt::ZERO);
        assert_eq!(contracts_root, Felt::ZERO);
    }

    // blocks after the cuttoff point should be the same as before pruning
    for num in (latest_block - keep_last_n + 1)..=latest_block {
        let (classes_root, contracts_root) = historical_roots(&provider, num).unwrap();
        let (expected_classes_root, expected_contracts_root) =
            historical_roots_reg.get(&num).unwrap();

        assert_eq!(classes_root, *expected_classes_root, "invalid classes root for block {num}");
        assert_eq!(
            contracts_root, *expected_contracts_root,
            "invalid contracts root for block {num}"
        );
    }

    assert_eq!(final_classes_root, initial_classes_root);
    assert_eq!(final_contracts_root, initial_contracts_root);
}
