//! Cairo Test Fixture Generator
//!
//! Generate Cairo test fixtures from SP1 proof artifacts.

use std::path::Path;

use crate::amd::prover::OnchainProof;
use crate::amd::Error;

/// Helper to safely extract a slice from data with bounds checking.
fn safe_slice(data: &[u8], start: usize, end: usize) -> Result<&[u8], Error> {
    if end > data.len() || start > end {
        return Err(Error::Calldata(format!(
            "Slice out of bounds: start={}, end={}, len={}",
            start,
            end,
            data.len()
        )));
    }
    Ok(&data[start..end])
}

/// Represents a decoded VerifierJournal for Cairo fixture generation
#[derive(Debug)]
pub struct DecodedJournal {
    pub result: u8,
    pub timestamp: u64,
    pub processor_model: u8,
    pub raw_report: Vec<u32>,
    pub certs: Vec<[u8; 32]>,
    pub cert_serials: Vec<[u8; 20]>,
    pub trusted_certs_prefix_len: u8,
}

/// Parse the journal from a proof file and decode it into a VerifierJournal structure.
///
/// The journal is ABI-encoded as a Solidity struct with dynamic fields.
pub fn decode_journal_from_proof(proof: &OnchainProof) -> Result<DecodedJournal, Error> {
    let journal_bytes = &proof.raw_proof.journal;

    if journal_bytes.len() < 256 {
        return Err(Error::Calldata(format!("Journal too short: {} bytes", journal_bytes.len())));
    }

    // Skip the first 32 bytes (ABI offset pointer 0x20)
    let data = &journal_bytes[32..];

    // Parse fixed fields (first 7 words = 224 bytes)
    let result = data[31]; // Last byte of word 0
    let timestamp = u64::from_be_bytes(data[56..64].try_into().unwrap()); // Word 1
    let processor_model = data[95]; // Last byte of word 2
    let raw_report_offset = u64::from_be_bytes(data[120..128].try_into().unwrap()) as usize; // Word 3
    let certs_offset = u64::from_be_bytes(data[152..160].try_into().unwrap()) as usize; // Word 4
    let cert_serials_offset = u64::from_be_bytes(data[184..192].try_into().unwrap()) as usize; // Word 5
    let trusted_certs_prefix_len = data[223]; // Last byte of word 6

    // Parse raw_report (dynamic bytes)
    let raw_report_len_slice = safe_slice(data, raw_report_offset + 24, raw_report_offset + 32)?;
    let raw_report_len_bytes =
        u64::from_be_bytes(raw_report_len_slice.try_into().unwrap()) as usize;
    let raw_report_start = raw_report_offset + 32;
    let raw_report_bytes =
        safe_slice(data, raw_report_start, raw_report_start + raw_report_len_bytes)?;

    // Convert to u32 array (little-endian as stored in attestation report)
    let mut raw_report = Vec::with_capacity(raw_report_len_bytes / 4);
    for chunk in raw_report_bytes.chunks(4) {
        raw_report.push(u32::from_le_bytes(chunk.try_into().unwrap()));
    }

    // Parse certs array (array of bytes32)
    let certs_len_slice = safe_slice(data, certs_offset + 24, certs_offset + 32)?;
    let certs_len = u64::from_be_bytes(certs_len_slice.try_into().unwrap()) as usize;
    let mut certs = Vec::with_capacity(certs_len);
    for i in 0..certs_len {
        let cert_start = certs_offset + 32 + i * 32;
        let cert_slice = safe_slice(data, cert_start, cert_start + 32)?;
        let mut cert = [0u8; 32];
        cert.copy_from_slice(cert_slice);
        certs.push(cert);
    }

    // Parse cert_serials array (array of uint160, but stored as bytes32)
    let cert_serials_len_slice =
        safe_slice(data, cert_serials_offset + 24, cert_serials_offset + 32)?;
    let cert_serials_len = u64::from_be_bytes(cert_serials_len_slice.try_into().unwrap()) as usize;
    let mut cert_serials = Vec::with_capacity(cert_serials_len);
    for i in 0..cert_serials_len {
        let serial_start = cert_serials_offset + 32 + i * 32;
        // uint160 is in the last 20 bytes of the 32-byte word
        let serial_slice = safe_slice(data, serial_start + 12, serial_start + 32)?;
        let mut serial = [0u8; 20];
        serial.copy_from_slice(serial_slice);
        cert_serials.push(serial);
    }

    Ok(DecodedJournal {
        result,
        timestamp,
        processor_model,
        raw_report,
        certs,
        cert_serials,
        trusted_certs_prefix_len,
    })
}

/// Generate Cairo test fixture code for a single block.
fn generate_block_fixture(block_num: usize, proof: &OnchainProof) -> Result<String, Error> {
    let journal = decode_journal_from_proof(proof)?;
    let journal_bytes = &proof.raw_proof.journal;

    // Convert journal bytes to u256 array (32 bytes per u256)
    let mut u256_inputs = Vec::new();
    for chunk in journal_bytes.chunks(32) {
        let mut padded = [0u8; 32];
        padded[32 - chunk.len()..].copy_from_slice(chunk);
        u256_inputs.push(padded);
    }

    let mut output = String::new();

    // Generate inputs function
    output.push_str(&format!("pub fn get_block_{block_num}_inputs() -> Array<u256> {{\n"));
    output.push_str("    array![\n");
    for bytes in u256_inputs.iter() {
        let high = u128::from_be_bytes(bytes[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(bytes[16..32].try_into().unwrap());
        output.push_str(&format!("        u256 {{ low: 0x{low:032x}, high: 0x{high:032x} }},"));
        output.push('\n');
    }
    output.push_str("    ]\n");
    output.push_str("}\n\n");

    // Generate expected function
    output.push_str(&format!("pub fn get_block_{block_num}_expected() -> VerifierJournal {{\n"));

    // Result enum
    let result_variant = match journal.result {
        0 => "VerificationResult::Success",
        1 => "VerificationResult::RootCertNotTrusted",
        2 => "VerificationResult::IntermediateCertsNotTrusted",
        3 => "VerificationResult::InvalidTimestamp",
        _ => return Err(Error::Calldata(format!("Unknown result: {}", journal.result))),
    };

    output.push_str(&format!("    let result = {result_variant};\n"));
    output.push_str(&format!("    let timestamp: u64 = {};\n", journal.timestamp));
    output.push_str(&format!("    let processor_model: u8 = {};\n", journal.processor_model));
    output.push_str(&format!(
        "    let trusted_certs_prefix_len: u8 = {};\n\n",
        journal.trusted_certs_prefix_len
    ));

    // Raw report array
    output.push_str("    let mut raw_report: Array<u32> = array![\n");
    for (i, word) in journal.raw_report.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            output.push('\n');
        }
        output.push_str(&format!("        0x{word:08x},"));
    }
    output.push_str("\n    ];\n\n");

    // Certs array
    output.push_str("    let certs: Array<u256> = array![\n");
    for cert in &journal.certs {
        let high = u128::from_be_bytes(cert[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(cert[16..32].try_into().unwrap());
        output.push_str(&format!("        u256 {{ low: 0x{low:032x}, high: 0x{high:032x} }},\n"));
    }
    output.push_str("    ];\n\n");

    // Cert serials array (uint160 fits in felt252)
    output.push_str("    let cert_serials: Array<felt252> = array![\n");
    for serial in &journal.cert_serials {
        // Convert 20 bytes to hex
        let hex: String = serial.iter().map(|b| format!("{b:02x}")).collect();
        output.push_str(&format!("        0x{hex},\n"));
    }
    output.push_str("    ];\n\n");

    output.push_str("    VerifierJournal {\n");
    output.push_str("        result,\n");
    output.push_str("        timestamp,\n");
    output.push_str("        processor_model,\n");
    output.push_str("        raw_report: raw_report.span(),\n");
    output.push_str("        certs,\n");
    output.push_str("        cert_serials,\n");
    output.push_str("        trusted_certs_prefix_len,\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    Ok(output)
}

/// Generate complete Cairo test fixtures file from proof files.
pub fn generate_cairo_fixtures(fixture_dir: &Path, output_path: &Path) -> Result<(), Error> {
    let mut output = String::new();

    // File header
    output.push_str("// Auto-generated by katana-tee generate-cairo-fixtures\n");
    output.push_str("// DO NOT EDIT MANUALLY\n\n");
    output.push_str("use amd_tee_registry::tee_types::{VerifierJournal, VerificationResult};\n\n");

    // Generate fixtures for each block
    for block_num in 0..3 {
        let proof_path = fixture_dir.join(format!("block_{block_num}/proof.json"));

        if !proof_path.exists() {
            return Err(Error::Calldata(format!("Proof file not found: {}", proof_path.display())));
        }

        let proof_data = std::fs::read(&proof_path).map_err(|e| {
            Error::Calldata(format!("Failed to read {}: {}", proof_path.display(), e))
        })?;

        let proof = OnchainProof::decode_json(&proof_data)
            .map_err(|e| Error::Calldata(format!("Failed to parse proof: {e}")))?;

        let fixture_code = generate_block_fixture(block_num, &proof)?;
        output.push_str(&fixture_code);
        output.push('\n');
    }

    // Write output file
    std::fs::write(output_path, &output)
        .map_err(|e| Error::Calldata(format!("Failed to write output: {e}")))?;

    Ok(())
}
