//! Two independently-built [`Node`]s must own independent [`ClassCache`] instances.
//!
//! This is the behavioural contract that replaced the process-global `OnceLock` cache:
//! each Node constructs its own cache and threads it into its block-producer, pool
//! validator, and RPC read path. A regression would surface as two Nodes silently
//! sharing cache state (the old failure mode when one Node's `build_global` won the
//! race against another's).

use katana_contracts::contracts;
use katana_primitives::felt;
use katana_sequencer_node::config::Config;
use katana_sequencer_node::Node;

#[tokio::test]
async fn two_nodes_own_independent_class_caches() {
    let node_a = Node::build(Config::default()).expect("failed to build node A");
    let node_b = Node::build(Config::default()).expect("failed to build node B");

    let cache_a = node_a.backend().executor_factory.class_cache();
    let cache_b = node_b.backend().executor_factory.class_cache();

    // Use distinct keys so an accidental shared cache can't be masked by both
    // sides inserting the same hash.
    let only_in_a = felt!("0xa");
    let only_in_b = felt!("0xb");

    cache_a.insert(only_in_a, contracts::Account::CLASS.clone());
    cache_b.insert(only_in_b, contracts::UniversalDeployer::CLASS.clone());

    assert!(cache_a.get(&only_in_a).is_some(), "node A must see its own insert");
    assert!(cache_b.get(&only_in_b).is_some(), "node B must see its own insert");

    assert!(
        cache_b.get(&only_in_a).is_none(),
        "node B saw a class inserted into node A's cache (regression: caches are sharing state, \
         as they did under the old OnceLock)"
    );
    assert!(
        cache_a.get(&only_in_b).is_none(),
        "node A saw a class inserted into node B's cache (regression: caches are sharing state, \
         as they did under the old OnceLock)"
    );
}
