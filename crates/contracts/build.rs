use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    // Track specific source directories and files that should trigger a rebuild
    // Important: We don't track Scarb.lock as scarb will update it on every `scarb build`
    println!("cargo:rerun-if-changed=contracts/Scarb.toml");
    println!("cargo:rerun-if-changed=contracts/account");
    println!("cargo:rerun-if-changed=contracts/legacy");
    println!("cargo:rerun-if-changed=contracts/messaging");
    println!("cargo:rerun-if-changed=contracts/test-contracts");
    println!("cargo:rerun-if-changed=contracts/vrf");
    println!("cargo:rerun-if-changed=build.rs");

    let contracts_dir = Path::new("contracts");
    let target_dir = contracts_dir.join("target/dev");
    let build_dir = Path::new("build");

    // Check if scarb is available
    let scarb_available = Command::new("scarb")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !scarb_available {
        println!("cargo:warning=scarb not found in PATH, skipping contract compilation");
        return;
    }

    // Only build if we're not in a docs build
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:warning=Building contracts with scarb...");

    // Run scarb build in the contracts directory
    let output = Command::new("scarb")
        .arg("build")
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
            "Contract compilation build script failed. Below are the last 50 lines of `scarb \
             build` output:\n\n{last_n_lines}"
        );
    }

    // Create build directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(build_dir) {
        panic!("Failed to create build directory: {e}");
    }

    // Copy artifacts from target/dev to build directory
    if target_dir.exists() {
        if let Err(e) = copy_dir_contents(&target_dir, build_dir) {
            panic!("Failed to copy contract artifacts: {e}");
        }
        println!("cargo:warning=Contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No contract artifacts found in target/dev");
    }
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
