fn main() {
    // Only build UI when embedded-ui feature is enabled
    #[cfg(feature = "embedded-ui")]
    {
        println!("cargo:rerun-if-changed=ui/");
        build_ui();
    }

    #[cfg(not(feature = "embedded-ui"))]
    {
        println!("cargo:warning=Embedded UI feature is disabled. UI assets will not be built.");
        println!(
            "cargo:warning=Use --features embedded-ui to enable embedded assets, or use \
             FileSystem/Proxy mode."
        );
    }
}

#[cfg(feature = "embedded-ui")]
fn build_ui() {
    use std::path::Path;

    // Check if we're in a build script
    if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let ui_dir = Path::new(&manifest_dir).join("ui");
        println!("Explorer UI directory: {}", ui_dir.display());

        // Check if ui/ doesn't exist or is empty
        let should_update_submodule = !ui_dir.exists()
            || (ui_dir.exists()
                && ui_dir.read_dir().map(|mut d| d.next().is_none()).unwrap_or(true));

        if should_update_submodule {
            initialize_submodule(&ui_dir);
        }

        // Check if dist directory exists, if not, build
        let dist_dir = ui_dir.join("dist");
        if !dist_dir.exists() || is_dist_empty(&dist_dir) {
            build_ui_assets(&ui_dir);
        } else {
            println!("UI assets already exist at {}", dist_dir.display());
        }
    }
}

#[cfg(feature = "embedded-ui")]
fn initialize_submodule(ui_dir: &std::path::Path) {
    use std::process::Command;

    // Check if we're in a git repository
    let git_check = Command::new("git").arg("rev-parse").arg("--git-dir").output();
    if git_check.is_ok() && git_check.unwrap().status.success() {
        println!("UI directory is empty, updating git submodule...");

        let status = Command::new("git")
            .arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--recursive")
            .arg("--force")
            .arg("ui")
            .status()
            .expect("Failed to update git submodule");

        if !status.success() {
            panic!("Failed to update git submodule for UI directory at {}", ui_dir.display());
        }
    } else {
        panic!(
            "UI directory doesn't exist at {} and couldn't fetch it through git submodule (not in \
             a git repository)",
            ui_dir.display()
        );
    }
}

#[cfg(feature = "embedded-ui")]
fn is_dist_empty(dist_dir: &std::path::Path) -> bool {
    dist_dir.read_dir().map(|mut entries| entries.next().is_none()).unwrap_or(true)
}

#[cfg(feature = "embedded-ui")]
fn build_ui_assets(ui_dir: &std::path::Path) {
    use std::process::Command;

    // Check for Bun
    let bun_check = Command::new("bun").arg("--version").output();
    if bun_check.is_err() || !bun_check.unwrap().status.success() {
        eprintln!("Warning: Bun is not installed. Attempting to skip UI build...");
        eprintln!("If you need embedded UI assets, please install Bun at https://bun.sh");
        return;
    }

    println!("Installing UI dependencies...");
    let status = Command::new("bun").current_dir(ui_dir).arg("install").status();

    match status {
        Ok(status) if status.success() => {}
        Ok(_) => {
            eprintln!("Warning: Failed to install UI dependencies in {}", ui_dir.display());
            return;
        }
        Err(e) => {
            eprintln!("Warning: Failed to run bun install: {}", e);
            return;
        }
    }

    println!("Building UI...");
    let status = Command::new("bun")
        .current_dir(ui_dir)
        .env("IS_EMBEDDED", "true")
        .arg("run")
        .arg("build")
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("UI build completed successfully");
        }
        Ok(_) => {
            eprintln!("Warning: Failed to build UI in {}", ui_dir.display());
        }
        Err(e) => {
            eprintln!("Warning: Failed to run bun build: {}", e);
        }
    }
}
