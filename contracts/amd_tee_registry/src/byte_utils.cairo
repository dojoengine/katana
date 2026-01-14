// Byte manipulation utilities for working with Span<u32>
use core::integer::u512;

// ============================================================================
// Fixed-size byte array structures
// ============================================================================

/// Fixed-size byte array for 24 bytes (6 u32 words)
/// Layout: [low_bits: 0-15] [high_bits: 16-23]
#[derive(Drop, Copy, Default, PartialEq, Debug)]
pub struct Bytes24 {
    /// Low 128 bits (bytes 0-15, u32 words 0-3)
    pub low_bits: u128,
    /// High 64 bits (bytes 16-23, u32 words 4-5)
    pub high_bits: u64,
}

/// Fixed-size byte array for 48 bytes (12 u32 words)
/// Layout: [low_bits: 0-15] [mid_bits: 16-31] [high_bits: 32-47]
#[derive(Drop, Copy, Default, PartialEq, Debug)]
pub struct Bytes48 {
    /// Low 128 bits (bytes 0-15, u32 words 0-3)
    pub low_bits: u128,
    /// Mid 128 bits (bytes 16-31, u32 words 4-7)
    pub mid_bits: u128,
    /// High 128 bits (bytes 32-47, u32 words 8-11)
    pub high_bits: u128,
}

// Note: For 64-byte values, use core::integer::u512 from corelib:
// u512 { limb0, limb1, limb2, limb3 } where val = limb0 + 2^128 * limb1 + ...
// limb0 = bytes 0-15, limb1 = bytes 16-31, limb2 = bytes 32-47, limb3 = bytes 48-63

// ============================================================================
// Helper functions for reading BytesXX from Span<u32>
// ============================================================================

/// Read Bytes24 from 6 consecutive u32 words
pub fn get_bytes24_at(data: Span<u32>, index: u32) -> Bytes24 {
    Bytes24 { low_bits: get_u128_at(data, index), high_bits: get_u64_at(data, index + 4) }
}

/// Read Bytes48 from 12 consecutive u32 words
pub fn get_bytes48_at(data: Span<u32>, index: u32) -> Bytes48 {
    Bytes48 {
        low_bits: get_u128_at(data, index),
        mid_bits: get_u128_at(data, index + 4),
        high_bits: get_u128_at(data, index + 8),
    }
}

/// Read u512 (64 bytes) from 16 consecutive u32 words
/// Returns u512 where limb0 = bytes 0-15, limb1 = bytes 16-31, etc.
pub fn get_u512_at(data: Span<u32>, index: u32) -> u512 {
    u512 {
        limb0: get_u128_at(data, index),
        limb1: get_u128_at(data, index + 4),
        limb2: get_u128_at(data, index + 8),
        limb3: get_u128_at(data, index + 12),
    }
}

// ============================================================================
// Helper functions for reading from Span<u32> using arithmetic operations
// ============================================================================

/// Read a single u32 at the given index
pub fn get_u32_at(data: Span<u32>, index: u32) -> u32 {
    *data.at(index)
}

/// Read u64 from 2 consecutive u32 words (little-endian)
/// u64 = word0 + word1 * 2^32
pub fn get_u64_at(data: Span<u32>, index: u32) -> u64 {
    let w0: u64 = (*data.at(index)).into();
    let w1: u64 = (*data.at(index + 1)).into();
    w0 + (w1 * 0x100000000) // w1 * 2^32
}

/// Read u128 from 4 consecutive u32 words (little-endian)
/// u128 = word0 + word1 * 2^32 + word2 * 2^64 + word3 * 2^96
pub fn get_u128_at(data: Span<u32>, index: u32) -> u128 {
    let w0: u128 = (*data.at(index)).into();
    let w1: u128 = (*data.at(index + 1)).into();
    let w2: u128 = (*data.at(index + 2)).into();
    let w3: u128 = (*data.at(index + 3)).into();
    w0 + (w1 * 0x100000000) + (w2 * 0x10000000000000000) + (w3 * 0x1000000000000000000000000)
}

/// Read u256 from 8 consecutive u32 words (little-endian)
pub fn get_u256_at(data: Span<u32>, index: u32) -> u256 {
    let low = get_u128_at(data, index);
    let high = get_u128_at(data, index + 4);
    u256 { low, high }
}

/// Extract individual u8 bytes from a u32 word using division
/// byte_index: 0 = LSB, 3 = MSB
pub fn get_u8_from_word(word: u32, byte_index: u32) -> u8 {
    let divisor: u32 = match byte_index {
        0 => 1,
        1 => 256,
        2 => 65536,
        3 => 16777216,
        _ => panic!("byte_index must be 0-3"),
    };
    let quotient = word / divisor;
    let byte_val = quotient % 256;
    byte_val.try_into().unwrap()
}

/// Slice a span of u32 words
pub fn slice_u32_span(data: Span<u32>, start: u32, len: u32) -> Span<u32> {
    let mut result: Array<u32> = array![];
    let mut i: u32 = 0;
    while i < len {
        result.append(*data.at(start + i));
        i += 1;
    }
    result.span()
}
