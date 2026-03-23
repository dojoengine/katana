use std::sync::LazyLock;

use zstd::dict::{DecoderDictionary, EncoderDictionary};

/// Dictionary version identifier stored in the envelope FLAGS field.
pub type DictVersion = u16;

/// Registry of all embedded dictionaries for a given payload type.
///
/// Each registry holds every historical dictionary version so that data compressed with any past
/// version can still be decompressed. New writes always use the latest (current) version.
pub struct DictRegistry {
    current_version: DictVersion,
    encoders: &'static [DictVersionedEncoder],
    decoders: &'static [DictVersionedDecoder],
}

struct DictVersionedEncoder {
    version: DictVersion,
    dict: &'static LazyLock<EncoderDictionary<'static>>,
}

struct DictVersionedDecoder {
    version: DictVersion,
    dict: &'static LazyLock<DecoderDictionary<'static>>,
}

impl DictRegistry {
    /// The dictionary version used for new writes.
    pub fn current_version(&self) -> DictVersion {
        self.current_version
    }

    /// Returns the encoder dictionary for the current (latest) version.
    pub fn encoder(&self) -> &EncoderDictionary<'static> {
        let entry = self
            .encoders
            .iter()
            .find(|e| e.version == self.current_version)
            .expect("current version must exist in registry");
        entry.dict
    }

    /// Returns the decoder dictionary for the given version, or `None` if unknown.
    pub fn decoder(&self, version: DictVersion) -> Option<&DecoderDictionary<'static>> {
        self.decoders.iter().find(|e| e.version == version).map(|e| &**e.dict)
    }
}

// -- Embedded dictionary data ------------------------------------------------

static RECEIPTS_V1_BYTES: &[u8] = include_bytes!("../../dictionaries/receipts_v1.dict");
static TX_V1_BYTES: &[u8] = include_bytes!("../../dictionaries/transactions_v1.dict");

// -- Receipt dictionaries ----------------------------------------------------

static RECEIPT_V1_ENCODER: LazyLock<EncoderDictionary<'static>> =
    LazyLock::new(|| EncoderDictionary::copy(RECEIPTS_V1_BYTES, 0));
static RECEIPT_V1_DECODER: LazyLock<DecoderDictionary<'static>> =
    LazyLock::new(|| DecoderDictionary::copy(RECEIPTS_V1_BYTES));

/// Dictionary registry for receipt payloads.
pub static RECEIPT_DICTS: DictRegistry = DictRegistry {
    current_version: 1,
    encoders: &[DictVersionedEncoder { version: 1, dict: &RECEIPT_V1_ENCODER }],
    decoders: &[DictVersionedDecoder { version: 1, dict: &RECEIPT_V1_DECODER }],
};

// -- Transaction dictionaries ------------------------------------------------

static TX_V1_ENCODER: LazyLock<EncoderDictionary<'static>> =
    LazyLock::new(|| EncoderDictionary::copy(TX_V1_BYTES, 0));
static TX_V1_DECODER: LazyLock<DecoderDictionary<'static>> =
    LazyLock::new(|| DecoderDictionary::copy(TX_V1_BYTES));

/// Dictionary registry for transaction payloads.
pub static TX_DICTS: DictRegistry = DictRegistry {
    current_version: 1,
    encoders: &[DictVersionedEncoder { version: 1, dict: &TX_V1_ENCODER }],
    decoders: &[DictVersionedDecoder { version: 1, dict: &TX_V1_DECODER }],
};
