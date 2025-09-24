use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    // Track specific source directories and files that should trigger a rebuild
    // Important: We don't track Scarb.lock as scarb will update it on every `scarb build`
    println!("cargo:rerun-if-changed=contracts/Scarb.toml");
    println!("cargo:rerun-if-changed=contracts/account");
    println!("cargo:rerun-if-changed=contracts/legacy");
    println!("cargo:rerun-if-changed=contracts/udc");
    println!("cargo:rerun-if-changed=contracts/messaging");
    println!("cargo:rerun-if-changed=contracts/test-contracts");
    println!("cargo:rerun-if-changed=contracts/vrf");
    println!("cargo:rerun-if-changed=build.rs");

    let contracts_dir = Path::new("contracts");
    let contracts_target_dir = contracts_dir.join("target/dev");
    // UDC is not part of the contracts workspace, so it needs a standalone build
    let udc_dir = contracts_dir.join("udc");
    let udc_target_dir = udc_dir.join("target/dev");
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
    run_scarb_build(contracts_dir, "contracts workspace");

    // Build UDC separately for the same reason as above
    println!("cargo:warning=Building UDC contract with scarb...");
    run_scarb_build(&udc_dir, "udc contract");

    // Create build directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(build_dir) {
        panic!("Failed to create build directory: {}", e);
    }

    // Copy artifacts from target/dev to build directory
    if contracts_target_dir.exists() {
        if let Err(e) = copy_dir_contents(&contracts_target_dir, build_dir) {
            panic!("Failed to copy contract artifacts: {}", e);
        }
        println!("cargo:warning=Contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No contract artifacts found in target/dev");
    }

    if udc_target_dir.exists() {
        if let Err(e) = copy_dir_contents(&udc_target_dir, build_dir) {
            panic!("Failed to copy UDC contract artifacts: {}", e);
        }
        println!("cargo:warning=UDC artifacts copied to build directory");
    } else {
        println!("cargo:warning=No UDC artifacts found in udc/target/dev");
    }
}

fn run_scarb_build(dir: &Path, label: &str) {
    let output = Command::new("scarb")
        .arg("build")
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| panic!("Failed to execute scarb build for {}", label));

    if output.status.success() {
        return;
    }

    let logs = String::from_utf8_lossy(&output.stdout);
    let stderr_logs = String::from_utf8_lossy(&output.stderr);
    let last_n_lines = logs
        .lines()
        .rev()
        .take(50)
        .collect::<Vec<&str>>()
        .into_iter()
        .rev()
        .collect::<Vec<&str>>()
        .join("\n");
    let last_n_stderr_lines = stderr_logs
        .lines()
        .rev()
        .take(50)
        .collect::<Vec<&str>>()
        .into_iter()
        .rev()
        .collect::<Vec<&str>>()
        .join("\n");

    panic!(
        "Contract compilation build script failed for {}. Below are the last 50 lines of `scarb \
         build` stdout and stderr output:\n\nstdout:\n{}\n\nstderr:\n{}",
        label, last_n_lines, last_n_stderr_lines
    );
}

fn copy_dir_contents(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_file() {
            fs::copy(&src_path, &dst_path)?;
        } else if src_path.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_contents(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
