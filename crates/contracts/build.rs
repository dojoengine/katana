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
