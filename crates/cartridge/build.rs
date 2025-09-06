use std::path::Path;
use std::{env, fs};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let classes_dir = Path::new(&manifest_dir).join("controller/account_sdk/artifacts/classes");
    let dest_path = Path::new(&manifest_dir).join("src/controller.rs");

    let mut generated_code = String::new();

    // Read all .json files from the classes directory
    if let Ok(entries) = fs::read_dir(&classes_dir) {
        let mut contracts = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(extension) = path.extension() {
                if extension == "json" {
                    if let Some(file_name) = path.file_stem() {
                        let file_name_str = file_name.to_string_lossy();
                        // Only include controller.*.contract_class.json files, not compiled
                        // ones
                        if file_name_str.starts_with("controller.")
                            && file_name_str.ends_with("contract_class")
                            && !file_name_str.contains("compiled")
                        {
                            contracts.push(file_name_str.to_string());
                        }
                    }
                }
            }
        }

        // Sort contracts for consistent ordering
        contracts.sort();

        for file_name in contracts {
            // Convert filename to struct name (e.g., controller.latest.contract_class ->
            // ControllerLatest)
            let struct_name = filename_to_struct_name(&file_name);

            generated_code.push_str(&format!(
                "::katana_contracts::contract!(\n    {},\n    \
                 \"{{CARGO_MANIFEST_DIR}}/controller/account_sdk/artifacts/classes/{}.json\"\n);
                 ",
                struct_name, file_name
            ));
        }
    }

    fs::write(dest_path, generated_code).unwrap();

    // Tell Cargo to rerun this build script if the classes directory changes
    println!("cargo:rerun-if-changed={}", classes_dir.display());
}

fn filename_to_struct_name(filename: &str) -> String {
    // Split by dots and convert each part to PascalCase
    let parts: Vec<&str> = filename.split('.').collect();
    let mut struct_name = String::new();

    for part in parts {
        if part == "json" || part == "contract_class" || part == "compiled_contract_class" {
            continue;
        }

        // Convert to PascalCase
        let pascal_part = part
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<String>();

        struct_name.push_str(&pascal_part);
    }

    struct_name
}
