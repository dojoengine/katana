use katana_primitives::state::StateUpdates;

use crate::models::envelope::{Envelope, EnvelopePayload};

impl EnvelopePayload for StateUpdates {
    const MAGIC: &[u8; 4] = b"KSTU";
    const NAME: &str = "state-update";
}

/// On-disk representation for `BlockStateUpdates` table values.
pub type StateUpdateEnvelope = Envelope<StateUpdates>;

impl From<StateUpdateEnvelope> for StateUpdates {
    fn from(value: StateUpdateEnvelope) -> Self {
        value.payload
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use katana_primitives::state::StateUpdates;
    use katana_primitives::{address, felt};

    use super::StateUpdateEnvelope;
    use crate::abstraction::{Database, DbTx, DbTxMut};
    use crate::codecs::{Compress, Decompress};
    use crate::{tables, Db};

    fn sample_state_updates() -> StateUpdates {
        let mut su = StateUpdates::default();
        su.nonce_updates.insert(address!("0x1234"), felt!("0x1"));
        su.deployed_contracts.insert(address!("0x5678"), felt!("0xabc"));
        su.storage_updates
            .insert(address!("0x1234"), BTreeMap::from([(felt!("0x10"), felt!("0x20"))]));
        su
    }

    #[test]
    fn state_update_envelope_roundtrip() {
        let su = sample_state_updates();
        let envelope = StateUpdateEnvelope::from(su.clone());

        let compressed = envelope.compress().expect("failed to compress state update envelope");
        assert_eq!(&compressed[..4], b"KSTU");
        assert_eq!(compressed[4], 1); // FORMAT_VERSION

        let decompressed = StateUpdateEnvelope::decompress(compressed)
            .expect("failed to decompress state update envelope");

        assert_eq!(decompressed, StateUpdateEnvelope::from(su));
    }

    #[test]
    fn state_update_envelope_rejects_legacy_postcard_bytes() {
        let su = sample_state_updates();
        let legacy = postcard::to_stdvec(&su).expect("failed to serialize legacy state update");

        StateUpdateEnvelope::decompress(legacy)
            .expect_err("legacy postcard bytes must not be accepted by envelope codec");
    }

    #[test]
    fn block_state_updates_table_roundtrip_uses_envelope() {
        let db = Db::in_memory().expect("failed to create in-memory db");
        let su = sample_state_updates();
        let envelope = StateUpdateEnvelope::from(su.clone());

        let tx = db.tx_mut().expect("failed to open write transaction");
        tx.put::<tables::BlockStateUpdates>(7, envelope).expect("failed to write");
        tx.commit().expect("failed to commit write transaction");

        let tx = db.tx().expect("failed to open read transaction");
        let stored = tx.get::<tables::BlockStateUpdates>(7).expect("failed to read");
        tx.commit().expect("failed to commit read transaction");

        assert_eq!(stored.map(StateUpdates::from), Some(su));
    }
}
