//! `snp-derivekey`: emit a SEV-SNP derived key on stdout.
//!
//! Called once from the measured initrd to produce the 32-byte key that
//! unlocks the LUKS-sealed data disk. The key is derived from the chip's
//! VCEK mixed with the VM's launch measurement and guest policy — any
//! change to chip, kernel, initrd, cmdline, or policy produces a
//! different key and the disk fails to open.
//!
//! Field-select bits follow the AMD SEV-SNP ABI (via the sev-snp SDK's
//! 6-digit binary-string encoding):
//!
//! ```text
//! index  5    4   3            2          1         0
//! field  TCB  SVN MEASUREMENT  FAMILY_ID  IMAGE_ID  GUEST_POLICY
//! value  0    0   1            0          0         1
//! ```
//!
//! String "001001" selects MEASUREMENT | GUEST_POLICY.
//!
//! TCB and SVN bits are deliberately *off*: mixing them would rotate the
//! derived key on every firmware/SVN update and brick the sealed disk
//! across routine upgrades.
//!
//! On success: writes 32 raw bytes to stdout and exits 0.
//! On failure: writes a labeled error to stderr and exits 1.
//! On panic: aborts (SIGABRT / exit 134) so the key is never unwound
//! through arbitrary destructors.

use std::io::Write;
use std::{panic, process};

use sev_snp::device::{DerivedKeyOptions, RootKeyType};
use sev_snp::SevSnp;
use zeroize::Zeroizing;

/// 6-digit binary string selecting MEASUREMENT | GUEST_POLICY.
const FIELD_SELECT: &str = "001001";

/// VMPL level requested for derivation. Used as a domain separator,
/// not a privilege boundary (a VMPL0 caller can request a key for VMPL1).
const VMPL: u32 = 1;

fn main() {
    panic::set_hook(Box::new(|info| {
        // Stderr only — stdout carries the key on the success path.
        eprintln!("snp-derivekey: fatal panic: {info}");
        process::abort();
    }));

    match run() {
        Ok(()) => process::exit(0),
        Err(e) => {
            eprintln!("snp-derivekey: error: {e}");
            process::exit(1);
        }
    }
}

fn run() -> Result<(), String> {
    // Reject any CLI args. This binary is invoked as `snp-derivekey` with no
    // arguments; surprise arguments are almost certainly a misconfiguration
    // (e.g. an init script that means to call a different tool).
    if std::env::args_os().len() > 1 {
        return Err("takes no arguments; usage: snp-derivekey".into());
    }

    let sev_snp = SevSnp::new().map_err(|e| format!("SevSnp::new failed: {e:?}"))?;

    let options = DerivedKeyOptions {
        root_key_type: Some(RootKeyType::VCEK),
        guest_field_sel: Some(FIELD_SELECT.into()),
        guest_svn: Some(0),
        tcb_version: Some(0),
        vmpl: Some(VMPL),
    };

    let key: Zeroizing<[u8; 32]> = Zeroizing::new(
        sev_snp
            .get_derived_key_with_options(&options)
            .map_err(|e| format!("SNP_GET_DERIVED_KEY failed: {e:?}"))?,
    );

    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&key[..]).map_err(|e| format!("stdout write failed: {e}"))?;
    stdout.flush().map_err(|e| format!("stdout flush failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field-select bitmap encodes the security contract: which launch-time
    /// identity bits the derived key is bound to. Rotating it silently would
    /// either (a) leak the sealed disk across image changes or (b) brick it on
    /// routine upgrades, depending on the direction of change.
    ///
    /// "001001" = MEASUREMENT (bit 3) | GUEST_POLICY (bit 0) = 9 decimal.
    #[test]
    fn field_select_binds_measurement_and_policy_only() {
        assert_eq!(FIELD_SELECT, "001001");
        let decoded = u64::from_str_radix(FIELD_SELECT, 2).expect("valid binary");
        assert_eq!(decoded, 0b001001, "MEASUREMENT | GUEST_POLICY");
        assert_eq!(decoded & (1 << 0), 1 << 0, "GUEST_POLICY bit set");
        assert_eq!(decoded & (1 << 3), 1 << 3, "MEASUREMENT bit set");
        assert_eq!(decoded & (1 << 4), 0, "GUEST_SVN bit MUST be off (upgrade pain)");
        assert_eq!(decoded & (1 << 5), 0, "TCB_VERSION bit MUST be off (upgrade pain)");
    }

    #[test]
    fn vmpl_is_one() {
        // Fixed to 1 as a domain separator for the sealed-storage use case.
        // Changing this rotates the key.
        assert_eq!(VMPL, 1);
    }
}
