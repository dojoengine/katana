//! Cross-chain arcade — the whole thing is two plain Starknet contracts.
//!
//! `arcade` runs on the settlement layer ("L1") and, in a single transaction,
//! sends an L1 -> L2 message to *every* registered `machine`. Each `machine`
//! runs on the appchain ("L2") and receives its coin through an `insert_coin`
//! `#[l1_handler]`. The point of the demo is the fan-out to *distinct* target
//! contracts — see the crate README and katana PR #623.

mod arcade;
mod machine;
