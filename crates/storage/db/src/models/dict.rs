use std::fmt::Debug;
use std::sync::LazyLock;

use katana_primitives::receipt::Receipt;
use zstd::dict::{DecoderDictionary, EncoderDictionary};

use crate::models::envelope::EnvelopePayload;
use crate::models::VersionedTx;

/// Current version for both the receipts and transactions dictionaries.
pub const CURRENT_DICTIONARY_VERSION: u16 = 1;

static RECEIPTS_V1_BYTES: &[u8] = include_bytes!("../../dictionaries/receipts_v1.dict");
static TX_V1_BYTES: &[u8] = include_bytes!("../../dictionaries/transactions_v1.dict");

pub static DICTIONARY_REGISTRY: LazyLock<DictRegistry> = LazyLock::new(|| {
    let receipts = Dictionary {
        name: <Receipt as EnvelopePayload>::NAME,
        encoder: EncoderDictionary::copy(RECEIPTS_V1_BYTES, 0),
        decoder: DecoderDictionary::copy(RECEIPTS_V1_BYTES),
    };

    let transactions = Dictionary {
        name: <VersionedTx as EnvelopePayload>::NAME,
        encoder: EncoderDictionary::copy(TX_V1_BYTES, 0),
        decoder: DecoderDictionary::copy(TX_V1_BYTES),
    };

    DictRegistry { dicts: [receipts, transactions] }
});

#[derive(Debug)]
pub struct DictRegistry {
    dicts: [Dictionary; 2],
}

impl DictRegistry {
    pub fn get(&self, name: &str) -> Option<&Dictionary> {
        self.dicts.iter().find(|d| d.name == name)
    }
}

pub struct Dictionary {
    name: &'static str,
    encoder: EncoderDictionary<'static>,
    decoder: DecoderDictionary<'static>,
}

impl Dictionary {
    pub fn encoder(&self) -> &EncoderDictionary<'static> {
        &self.encoder
    }

    pub fn decoder(&self) -> &DecoderDictionary<'static> {
        &self.decoder
    }
}

impl Debug for Dictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dictionary").field("name", &self.name).finish_non_exhaustive()
    }
}
