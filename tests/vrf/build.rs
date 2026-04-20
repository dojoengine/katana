use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    let contracts_dir = Path::new("contracts");
    let target_dir = contracts_dir.join("target/dev");
    let build_dir = Path::new("build");

    println!("cargo:rerun-if-changed=contracts/Scarb.toml");
    println!("cargo:rerun-if-changed=build.rs");
    track_dir_excluding_lock(&contracts_dir.join("src"));

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

    // Skip in docs builds
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:warning=Building VRF test contracts with scarb...");

    let output = Command::new("asdf")
        .args(["exec", "scarb", "build"])
        .current_dir(contracts_dir)
        .output()
        .expect("Failed to execute scarb build for VRF test contracts");

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
            "VRF test contracts compilation failed. Below are the last 50 lines of `scarb build` \
             output:\n\n{last_n_lines}"
        );
    }

    // Create build directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(build_dir) {
        panic!("Failed to create build directory: {e}");
    }

    // Copy contract artifacts from target/dev to build directory
    if target_dir.exists() {
        if let Err(e) = copy_dir_contents(&target_dir, build_dir) {
            panic!("Failed to copy VRF test contract artifacts: {e}");
        }
        println!("cargo:warning=VRF test contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No VRF test contract artifacts found in target/dev");
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

fn track_dir_excluding_lock(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            track_dir_excluding_lock(&path);
        } else if path.file_name().is_some_and(|name| name != "Scarb.lock") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
