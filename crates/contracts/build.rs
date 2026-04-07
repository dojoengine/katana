use std::path::Path;
use std::process::Command;
use std::{env, fs, thread};

// Used by the controller artifact codegen
const CONTROLLER_CLASSES_SUBDIR: &str = "contracts/controller/account_sdk/artifacts/classes";
const OPENZEPPELIN_SUBMODULE_DIR: &str = "contracts/openzeppelin";
const OPENZEPPELIN_ACCOUNT_PRESET_PACKAGE: &str = "openzeppelin_presets";
const OPENZEPPELIN_ACCOUNT_PRESET_ARTIFACT: &str =
    "openzeppelin_presets_AccountUpgradeable.contract_class.json";
const OPENZEPPELIN_SCARB_VERSION: &str = "2.11.4";

fn main() {
    // Track specific source directories and files that should trigger a rebuild.
    // We track individual files instead of whole directories to exclude Scarb.lock files,
    // which scarb updates on every `scarb build` and would cause unnecessary rebuilds.
    println!("cargo:rerun-if-changed=contracts/Scarb.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let watch_dirs = [
        "contracts/account",
        "contracts/legacy",
        "contracts/messaging",
        "contracts/test-contracts",
        "contracts/vrf",
        "contracts/avnu",
        OPENZEPPELIN_SUBMODULE_DIR,
        CONTROLLER_CLASSES_SUBDIR,
    ];

    for dir in &watch_dirs {
        track_dir_excluding_lock(Path::new(dir));
    }

    // Generate controller contract bindings in parallel — it only reads pre-compiled
    // artifacts from the controller submodule and has no dependency on the scarb builds.
    let controller_handle = thread::spawn(generate_controller_bindings);

    let contracts_dir = Path::new("contracts");
    let target_dir = contracts_dir.join("target/dev");
    let build_dir = Path::new("build");
    let openzeppelin_dir = contracts_dir.join("openzeppelin");

    // Check if asdf is available (used to manage scarb versions)
    let asdf_available = Command::new("asdf")
        .args(["exec", "scarb", "--version"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !asdf_available {
        println!("cargo:warning=asdf or scarb not found, skipping contract compilation");
        return;
    }

    // Only build if we're not in a docs build
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:warning=Building main contracts with scarb...");

    // Run scarb build in the contracts directory (uses asdf to pick correct scarb version)
    let output = Command::new("asdf")
        .args(["exec", "scarb", "build"])
        .current_dir(contracts_dir)
        .output()
        .expect("Failed to execute scarb build");

    if !output.status.success() {
        let logs = String::from_utf8_lossy(&output.stdout);
        let last_n_lines = logs
            .split('\n')
            .rev()
            .take(50)
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n");

        panic!(
            "Main contracts compilation failed. Below are the last 50 lines of `scarb build` \
             output:\n\n{last_n_lines}"
        );
    }

    // Build VRF contracts (uses different scarb version via asdf)
    let vrf_dir = contracts_dir.join("vrf");
    build_vrf_contracts(&vrf_dir);

    // Build AVNU contracts (uses different scarb version via asdf)
    let avnu_dir = contracts_dir.join("avnu/contracts");
    build_avnu_contracts(&avnu_dir);

    // Build the canonical OpenZeppelin account preset with a newer scarb than the
    // local contracts workspace uses, then copy only the account class artifact.
    build_openzeppelin_account_preset(&openzeppelin_dir);

    // Create build directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(build_dir) {
        panic!("Failed to create build directory: {e}");
    }

    // Copy main contract artifacts from target/dev to build directory
    if target_dir.exists() {
        if let Err(e) = copy_dir_contents(&target_dir, build_dir) {
            panic!("Failed to copy main contract artifacts: {e}");
        }
        println!("cargo:warning=Main contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No main contract artifacts found in target/dev");
    }

    // Copy VRF contract artifacts from vrf/target/dev to build directory
    let vrf_target_dir = vrf_dir.join("target/dev");
    if vrf_target_dir.exists() {
        if let Err(e) = copy_dir_contents(&vrf_target_dir, build_dir) {
            panic!("Failed to copy VRF contract artifacts: {e}");
        }
        println!("cargo:warning=VRF contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No VRF contract artifacts found in vrf/target/dev");
    }

    // Copy AVNU contract artifacts from avnu/contracts/target/dev to build directory
    let avnu_target_dir = avnu_dir.join("target/dev");
    if avnu_target_dir.exists() {
        if let Err(e) = copy_dir_contents(&avnu_target_dir, build_dir) {
            panic!("Failed to copy AVNU contract artifacts: {e}");
        }
        println!("cargo:warning=AVNU contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No AVNU contract artifacts found in avnu/contracts/target/dev");
    }

    let openzeppelin_account_artifact =
        openzeppelin_dir.join("target/dev").join(OPENZEPPELIN_ACCOUNT_PRESET_ARTIFACT);
    if openzeppelin_account_artifact.exists() {
        if let Err(e) = copy_file_to_dir(&openzeppelin_account_artifact, build_dir) {
            panic!("Failed to copy OpenZeppelin account preset artifact: {e}");
        }
        println!("cargo:warning=OpenZeppelin account preset artifact copied to build directory");
    } else {
        println!(
            "cargo:warning=No OpenZeppelin account preset artifact found at {}",
            openzeppelin_account_artifact.display()
        );
    }

    controller_handle.join().expect("Controller bindings generation failed");
}

fn copy_dir_contents(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn copy_file_to_dir(src: &Path, dst_dir: &Path) -> std::io::Result<()> {
    let file_name =
        src.file_name().expect("artifact source path must point to a file with a file name");
    fs::copy(src, dst_dir.join(file_name))?;
    Ok(())
}

fn build_vrf_contracts(vrf_dir: &Path) {
    println!("cargo:warning=Building VRF contracts with scarb...");

    let output = Command::new("asdf")
        .args(["exec", "scarb", "build"])
        .current_dir(vrf_dir)
        .output()
        .expect("Failed to execute scarb build for VRF contracts");

    if !output.status.success() {
        let logs = String::from_utf8_lossy(&output.stdout);
        let last_n_lines = logs
            .split('\n')
            .rev()
            .take(50)
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n");

        panic!(
            "VRF contracts compilation failed. Below are the last 50 lines of `scarb build` \
             output:\n\n{last_n_lines}"
        );
    }
}

fn build_openzeppelin_account_preset(openzeppelin_dir: &Path) {
    let presets_manifest = openzeppelin_dir.join("packages/presets/Scarb.toml");
    if !presets_manifest.exists() {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        initialize_submodule(
            Path::new(&manifest_dir).join(OPENZEPPELIN_SUBMODULE_DIR).as_path(),
            "OpenZeppelin",
        );
    }

    println!(
        "cargo:warning=Building OpenZeppelin account preset with scarb \
         {OPENZEPPELIN_SCARB_VERSION}..."
    );

    let output = Command::new("asdf")
        .args(["exec", "scarb", "build", "-p", OPENZEPPELIN_ACCOUNT_PRESET_PACKAGE])
        .env("ASDF_SCARB_VERSION", OPENZEPPELIN_SCARB_VERSION)
        .current_dir(openzeppelin_dir)
        .output()
        .expect("Failed to execute scarb build for OpenZeppelin account preset");

    if !output.status.success() {
        let logs = String::from_utf8_lossy(&output.stdout);
        let last_n_lines = logs
            .split('\n')
            .rev()
            .take(50)
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n");

        panic!(
            "OpenZeppelin account preset compilation failed. Below are the last 50 lines of \
             `scarb build` output:\n\n{last_n_lines}"
        );
    }
}

fn track_dir_excluding_lock(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip target directories as they are build outputs that change on every build
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            track_dir_excluding_lock(&path);
        } else if path.file_name().is_some_and(|name| name != "Scarb.lock") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn generate_controller_bindings() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let classes_dir = Path::new(&manifest_dir).join(CONTROLLER_CLASSES_SUBDIR);
    let dest_path = Path::new(&manifest_dir).join("src/controller.rs");

    // Check if controller submodule is initialized
    if !classes_dir.exists() {
        initialize_submodule(
            Path::new(&manifest_dir).join("contracts/controller").as_path(),
            "Controller",
        );
    }

    let mut generated_code = String::new();

    generated_code.push_str(
        "//! This file is automatically generated by build.rs. Do not edit manually.\n\n",
    );

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
                "::katana_contracts_macro::contract!(\n    {struct_name},\n    \
                 \"{{CARGO_MANIFEST_DIR}}/{CONTROLLER_CLASSES_SUBDIR}/{file_name}.json\"\n);\n"
            ));
        }
    }

    fs::write(dest_path, generated_code).unwrap();
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

fn initialize_submodule(submodule_dir: &Path, submodule_name: &str) {
    // Check if we're in a git repository
    let git_check = Command::new("git").arg("rev-parse").arg("--git-dir").output();
    if git_check.is_ok() && git_check.unwrap().status.success() {
        println!("{submodule_name} directory is empty, updating git submodule...");

        let status = Command::new("git")
            .arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--recursive")
            .arg("--force")
            .arg(submodule_dir.to_str().unwrap())
            .status()
            .expect("Failed to update git submodule");

        if !status.success() {
            panic!(
                "Failed to update git submodule for {submodule_name} directory at {}",
                submodule_dir.display()
            );
        }
    } else {
        panic!(
            "{submodule_name} directory doesn't exist at {} and couldn't fetch it through git \
             submodule (not in a git repository)",
            submodule_dir.display()
        );
    }
}

fn build_avnu_contracts(avnu_dir: &Path) {
    // AVNU contracts require scarb 2.11.4 (cairo-version = "2.11.4" in Scarb.toml)
    const AVNU_SCARB_VERSION: &str = "2.11.4";

    println!("cargo:warning=Building AVNU contracts with scarb {AVNU_SCARB_VERSION}...");

    let output = Command::new("asdf")
        .args(["exec", "scarb", "build"])
        .env("ASDF_SCARB_VERSION", AVNU_SCARB_VERSION)
        .current_dir(avnu_dir)
        .output()
        .expect("Failed to execute scarb build for AVNU contracts");

    if !output.status.success() {
        let logs = String::from_utf8_lossy(&output.stdout);
        let last_n_lines = logs
            .split('\n')
            .rev()
            .take(50)
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n");

        panic!(
            "AVNU contracts compilation failed. Below are the last 50 lines of `scarb build` \
             output:\n\n{last_n_lines}"
        );
    }
}
