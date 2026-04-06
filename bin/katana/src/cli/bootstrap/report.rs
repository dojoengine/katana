//! Pretty-printer for [`BootstrapReport`].

use comfy_table::{ContentArrangement, Table};

use super::executor::BootstrapReport;

/// Print a two-table summary (declared classes, deployed contracts) to stdout.
///
/// In v1 there is intentionally no machine-readable output. Add `--json` later if
/// downstream tooling needs to consume the report.
pub fn print(report: &BootstrapReport) {
    if !report.declared.is_empty() {
        println!("\nDeclared classes:");
        let mut table = Table::new();
        table
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(["name", "class hash", "status"]);
        for c in &report.declared {
            let status = if c.already_declared { "already declared" } else { "declared" };
            table.add_row([c.name.clone(), format!("{:#x}", c.class_hash), status.to_string()]);
        }
        println!("{table}");
    }

    if !report.deployed.is_empty() {
        println!("\nDeployed contracts:");
        let mut table = Table::new();
        table
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(["label", "class", "address", "tx hash"]);
        for d in &report.deployed {
            table.add_row([
                d.label.clone().unwrap_or_default(),
                d.class_name.clone(),
                format!("{:#x}", Into::<katana_primitives::Felt>::into(d.address)),
                format!("{:#x}", d.tx_hash),
            ]);
        }
        println!("{table}");
    }

    if report.declared.is_empty() && report.deployed.is_empty() {
        println!("Nothing to do.");
    }
}
