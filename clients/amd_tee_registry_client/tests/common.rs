use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::KDS;
use amd_sev_snp_attestation_verifier::{
    stub::{VerifierInput, VerifierJournal},
    AttestationReport,
};
use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use x509_verifier_rust_crypto::CertChain;

const REPORT_SIZE: usize = 1184;

#[allow(dead_code)]
pub fn fixture_paths_all() -> Vec<PathBuf> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("amd_sev_snp_attestation_sdk");
    vec![
        base.join("attestation_azure_snp.json"),
        base.join("attestation_gcp_snp.json"),
        base.join("sp1_gcp.json"),
        base.join("boundless_azure.json"),
        base.join("pico-inputs.json"),
    ]
}

pub fn fixture_paths_kds() -> Vec<PathBuf> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("amd_sev_snp_attestation_sdk");
    vec![
        base.join("attestation_azure_snp.json"),
        base.join("attestation_gcp_snp.json"),
    ]
}

pub fn load_report_bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    let raw =
        fs::read(path).with_context(|| format!("Failed to read fixture: {}", path.display()))?;
    let value: Value =
        serde_json::from_slice(&raw).with_context(|| "Failed to parse fixture JSON")?;

    if let Some(report_hex) = value.get("report").and_then(|v| v.as_str()) {
        return decode_report_hex(report_hex);
    }

    if let Some(journal_hex) = value
        .get("raw_proof")
        .and_then(|v| v.get("journal"))
        .and_then(|v| v.as_str())
    {
        return report_from_journal_hex(journal_hex);
    }

    if let Some(public_values_hex) = value.get("publicValues").and_then(|v| v.as_str()) {
        return report_from_journal_hex(public_values_hex);
    }

    bail!(
        "Fixture missing report or journal fields: {}",
        path.display()
    );
}

#[allow(dead_code)]
pub fn build_verifier_input(
    timestamp: u64,
    report_bytes: &[u8],
    cert_chain: &CertChain,
) -> VerifierInput {
    amd_tee_registry_client::prepare_verifier_input_with_storage(
        timestamp,
        Bytes::from(report_bytes.to_vec()),
        cert_chain.to_ders(),
        0,
        None,
    )
}

pub fn fetch_kds_chain_with_timeout(
    report: AttestationReport,
    timeout: Duration,
) -> anyhow::Result<Vec<Bytes>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = KDS::new().fetch_report_cert_chain(&report);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => result.map_err(|err| anyhow!("KDS fetch failed: {}", err)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(anyhow!(
            "KDS fetch timed out after {} seconds",
            timeout.as_secs()
        )),
        Err(err) => Err(anyhow!("KDS fetch channel error: {}", err)),
    }
}

pub fn pick_valid_timestamp(cert_chain: &CertChain) -> anyhow::Result<u64> {
    let mut max_not_before = i64::MIN;
    let mut min_not_after = i64::MAX;

    for cert in &cert_chain.certs {
        let (not_before, not_after) = cert.validity();
        max_not_before = max_not_before.max(not_before.timestamp());
        min_not_after = min_not_after.min(not_after.timestamp());
    }

    let candidate = max_not_before + 60;
    if candidate >= min_not_after {
        bail!(
            "No valid timestamp in cert range (not_before={}, not_after={})",
            max_not_before,
            min_not_after
        );
    }

    Ok(candidate as u64)
}

pub fn print_report(report: &AttestationReport, fixture_name: &str) {
    println!("=== Fixture: {} ===", fixture_name);
    println!("{}", report);
}

pub fn print_journal(journal: &VerifierJournal) {
    println!("Verification result: {:?}", journal.result);
    println!("Processor model (u8): {}", journal.processorModel);
    println!(
        "Trusted certs prefix length: {}",
        journal.trustedCertsPrefixLen
    );
    println!("Cert serial count: {}", journal.certSerials.len());
    println!("Raw report bytes: {}", journal.rawReport.len());
}

fn report_from_journal_hex(journal_hex: &str) -> anyhow::Result<Vec<u8>> {
    let journal_bytes = decode_hex_bytes(journal_hex)?;
    let journal = VerifierJournal::decode(&journal_bytes)
        .map_err(|err| anyhow!("Failed to decode verifier journal: {}", err))?;
    let report_bytes = journal.rawReport.to_vec();
    ensure_report_size(&report_bytes)?;
    Ok(report_bytes)
}

fn decode_report_hex(report_hex: &str) -> anyhow::Result<Vec<u8>> {
    let report_bytes = decode_hex_bytes(report_hex)?;
    ensure_report_size(&report_bytes)?;
    Ok(report_bytes)
}

fn decode_hex_bytes(value: &str) -> anyhow::Result<Vec<u8>> {
    let trimmed = value.trim_start_matches("0x");
    if trimmed.is_empty() {
        bail!("Hex string is empty");
    }
    hex::decode(trimmed).map_err(|err| anyhow!("Hex decode failed: {}", err))
}

fn ensure_report_size(report_bytes: &[u8]) -> anyhow::Result<()> {
    if report_bytes.len() != REPORT_SIZE {
        bail!(
            "Invalid report size: expected {}, got {}",
            REPORT_SIZE,
            report_bytes.len()
        );
    }
    Ok(())
}
