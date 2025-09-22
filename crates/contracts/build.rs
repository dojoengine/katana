use std::path::Path;
use std::process::Command;
use std::time::SystemTime;
use std::{env, fs};

fn main() {
    // Track specific source directories and files that should trigger a rebuild
    // Important: We don't track Scarb.lock as scarb itself updates it
    println!("cargo:rerun-if-changed=contracts/Scarb.toml");
    println!("cargo:rerun-if-changed=contracts/account");
    println!("cargo:rerun-if-changed=contracts/legacy");
    println!("cargo:rerun-if-changed=contracts/messaging");
    println!("cargo:rerun-if-changed=contracts/test-contracts");
    println!("cargo:rerun-if-changed=contracts/vrf");

    // Also track the build script itself
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

    // Check if we need to rebuild by comparing source and target timestamps
    // This prevents unnecessary scarb runs which update Scarb.lock
    if should_skip_build(&contracts_dir, &build_dir) {
        println!("cargo:warning=Contracts are up to date, skipping scarb build");
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
             build` output:\n\n{}",
            last_n_lines
        );
    }

    // Create build directory if it doesn't exist
    if let Err(e) = fs::create_dir_all(build_dir) {
        panic!("Failed to create build directory: {}", e);
    }

    // Copy artifacts from target/dev to build directory
    if target_dir.exists() {
        if let Err(e) = copy_dir_contents(&target_dir, build_dir) {
            panic!("Failed to copy contract artifacts: {}", e);
        }
        println!("cargo:warning=Contract artifacts copied to build directory");
    } else {
        println!("cargo:warning=No contract artifacts found in target/dev");
    }
}

fn should_skip_build(contracts_dir: &Path, build_dir: &Path) -> bool {
    // If build directory doesn't exist, we need to build
    if !build_dir.exists() {
        return false;
    }

    // Get the oldest modification time from the build directory
    // We use oldest to ensure all build artifacts are newer than sources
    let build_time = get_oldest_mtime_in_dir(build_dir);

    if build_time.is_none() {
        return false;
    }

    let build_time = build_time.unwrap();

    // Check if any source files are newer than the build artifacts
    let source_dirs = [
        contracts_dir.join("account"),
        contracts_dir.join("legacy"),
        contracts_dir.join("messaging"),
        contracts_dir.join("test-contracts"),
        contracts_dir.join("vrf"),
    ];

    for dir in &source_dirs {
        if let Some(source_time) = get_newest_mtime_recursive(dir) {
            if source_time > build_time {
                return false;
            }
        }
    }

    // Check Scarb.toml (but not Scarb.lock)
    if let Ok(metadata) = fs::metadata(contracts_dir.join("Scarb.toml")) {
        if let Ok(source_time) = metadata.modified() {
            if source_time > build_time {
                return false;
            }
        }
    }

    true
}

fn get_oldest_mtime_in_dir(path: &Path) -> Option<SystemTime> {
    fs::read_dir(path).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                if e.path().is_file() {
                    e.metadata().ok().and_then(|m| m.modified().ok())
                } else {
                    None
                }
            })
            .min()
    })
}

fn get_newest_mtime_recursive(path: &Path) -> Option<SystemTime> {
    let mut latest_time = None;

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_file() {
                // Check all source files, not just .cairo
                // This includes .toml files in subdirectories
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_str().unwrap_or("");
                    if ext_str == "cairo" || ext_str == "toml" {
                        if let Ok(metadata) = entry.metadata() {
                            if let Ok(mtime) = metadata.modified() {
                                latest_time = Some(match latest_time {
                                    None => mtime,
                                    Some(t) if mtime > t => mtime,
                                    Some(t) => t,
                                });
                            }
                        }
                    }
                }
            } else if path.is_dir() {
                // Recursively check subdirectories
                if let Some(dir_time) = get_newest_mtime_recursive(&path) {
                    latest_time = Some(match latest_time {
                        None => dir_time,
                        Some(t) if dir_time > t => dir_time,
                        Some(t) => t,
                    });
                }
            }
        }
    }

    latest_time
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
