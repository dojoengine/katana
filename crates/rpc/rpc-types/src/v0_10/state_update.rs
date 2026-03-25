//! State update types for Starknet spec v0.10.0.
//!
//! In v0.10:
//! - `StateDiff::migrated_compiled_classes` is required (always serialized, defaults to empty).
//! - `PreConfirmedStateUpdate::old_root` is optional.

use std::collections::BTreeMap;

use katana_primitives::block::BlockHash;
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StateUpdate {
    Confirmed(ConfirmedStateUpdate),
    PreConfirmed(PreConfirmedStateUpdate),
}

/// State update of a confirmed block (same structure as v0.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedStateUpdate {
    pub block_hash: BlockHash,
    pub new_root: Felt,
    pub old_root: Felt,
    pub state_diff: StateDiff,
}

/// State update of a pre-confirmed block.
///
/// In v0.10, `old_root` is optional (removed from required fields in the spec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedStateUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_root: Option<Felt>,
    pub state_diff: StateDiff,
}

/// v0.10 StateDiff — `migrated_compiled_classes` is always serialized (required).
///
/// We wrap the shared `StateDiff` and override serialization to always include the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDiff(pub crate::state_update::StateDiff);

impl serde::Serialize for StateDiff {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Ensure migrated_compiled_classes is always present (empty if None).
        let mut inner = self.0.clone();
        if inner.migrated_compiled_classes.is_none() {
            inner.migrated_compiled_classes = Some(BTreeMap::new());
        }

        // Delegate to the inner type's serialization.
        // Since the inner Serialize always includes migrated_compiled_classes when Some,
        // this will always include the field.
        inner.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for StateDiff {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut inner = crate::state_update::StateDiff::deserialize(deserializer)?;
        // In v0.10, migrated_compiled_classes is required — default to empty if not present.
        if inner.migrated_compiled_classes.is_none() {
            inner.migrated_compiled_classes = Some(BTreeMap::new());
        }
        Ok(StateDiff(inner))
    }
}

impl From<crate::state_update::StateDiff> for StateDiff {
    fn from(inner: crate::state_update::StateDiff) -> Self {
        StateDiff(inner)
    }
}

impl From<katana_primitives::state::StateUpdates> for StateDiff {
    fn from(value: katana_primitives::state::StateUpdates) -> Self {
        let inner: crate::state_update::StateDiff = value.into();
        StateDiff(inner)
    }
}

impl From<crate::state_update::StateUpdate> for StateUpdate {
    fn from(su: crate::state_update::StateUpdate) -> Self {
        match su {
            crate::state_update::StateUpdate::Confirmed(c) => {
                StateUpdate::Confirmed(ConfirmedStateUpdate {
                    block_hash: c.block_hash,
                    new_root: c.new_root,
                    old_root: c.old_root,
                    state_diff: StateDiff(c.state_diff),
                })
            }
            crate::state_update::StateUpdate::PreConfirmed(p) => {
                StateUpdate::PreConfirmed(PreConfirmedStateUpdate {
                    old_root: p.old_root,
                    state_diff: StateDiff(p.state_diff),
                })
            }
        }
    }
}
