//! Pretty-printer for [`BootstrapReport`].

use comfy_table::{ContentArrangement, Table};
use serde_json::{json, Value};

use crate::executor::BootstrapReport;

/// Print a two-table summary (declared classes, deployed contracts) to stdout.
///
/// In v1 there is intentionally no machine-readable output. Add `--json` later if
/// downstream tooling needs to consume the report.
pub fn print(report: &BootstrapReport) {
    if !report.declared.is_empty() {
        println!("\nDeclared classes:");
        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Dynamic).set_header([
            "name",
            "class hash",
            "status",
        ]);
        for c in &report.declared {
            let status = if c.already_declared { "already declared" } else { "declared" };
            table.add_row([c.name.clone(), format!("{:#x}", c.class_hash), status.to_string()]);
        }
        println!("{table}");
    }

    if !report.deployed.is_empty() {
        println!("\nDeployed contracts:");
        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Dynamic).set_header([
            "label",
            "class",
            "address",
            "status / tx hash",
        ]);
        for d in &report.deployed {
            let status = match d.tx_hash {
                Some(hash) => format!("{hash:#x}"),
                None if d.already_deployed => "already deployed".to_string(),
                None => "—".to_string(),
            };
            table.add_row([
                d.label.clone().unwrap_or_default(),
                d.class_name.clone(),
                format!("{:#x}", Into::<katana_primitives::Felt>::into(d.address)),
                status,
            ]);
        }
        println!("{table}");
    }

    if report.declared.is_empty() && report.deployed.is_empty() {
        println!("Nothing to do.");
    }
}

/// Emit the report as a single line of compact JSON on stdout — the machine-readable
/// counterpart to [`print`], selected by `katana bootstrap --json`. Downstream tooling
/// (e.g. `scripts/bootstrap/`) parses this to recover deployed addresses.
pub fn print_json(report: &BootstrapReport) {
    println!("{}", to_json(report));
}

/// Build the stable JSON shape consumed by `--json`. All class hashes, addresses, and tx
/// hashes are rendered as `0x`-prefixed hex strings so downstream parsers don't have to
/// guess the felt encoding.
fn to_json(report: &BootstrapReport) -> Value {
    use katana_primitives::Felt;

    let declared: Vec<Value> = report
        .declared
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "class_hash": format!("{:#x}", c.class_hash),
                "already_declared": c.already_declared,
            })
        })
        .collect();

    let deployed: Vec<Value> = report
        .deployed
        .iter()
        .map(|d| {
            json!({
                "label": d.label,
                "class": d.class_name,
                "address": format!("{:#x}", Into::<Felt>::into(d.address)),
                "tx_hash": d.tx_hash.map(|h| format!("{h:#x}")),
                "already_deployed": d.already_deployed,
            })
        })
        .collect();

    json!({ "declared": declared, "deployed": deployed })
}

#[cfg(test)]
mod tests {
    use katana_primitives::{ContractAddress, Felt};

    use super::*;
    use crate::executor::{DeclaredClass, DeployedContract};

    #[test]
    fn json_shape_exposes_hex_address_and_class() {
        let report = BootstrapReport {
            declared: vec![DeclaredClass {
                name: "mock_amd_tee_registry".to_string(),
                class_hash: Felt::from(0xabcu32),
                already_declared: false,
            }],
            deployed: vec![DeployedContract {
                label: None,
                class_name: "mock_amd_tee_registry".to_string(),
                address: ContractAddress::from(Felt::from(0x7eeu32)),
                tx_hash: Some(Felt::from(0x123u32)),
                already_deployed: false,
            }],
        };

        let value = to_json(&report);
        assert_eq!(value["declared"][0]["class_hash"], "0xabc");
        assert_eq!(value["deployed"][0]["class"], "mock_amd_tee_registry");
        assert_eq!(value["deployed"][0]["address"], "0x7ee");
        assert_eq!(value["deployed"][0]["tx_hash"], "0x123");
        assert_eq!(value["deployed"][0]["already_deployed"], false);
    }
}
