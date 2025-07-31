use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ui/");

    // Check if we're in a build script
    if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
        // $CARGO_MANIFEST_DIR/ui/
        let ui_dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("ui");
        println!("Explorer UI directory: {}", ui_dir.display());

        // Check if ui/ doesn't exist or is empty. This determines whether we need to initialize the
        // git submodule.
        //
        // The condition is true if:
        // 1. The ui directory doesn't exist at all, OR
        // 2. The ui directory exists but is empty (no entries when reading the directory)
        let should_update_submodule = !ui_dir.exists()
            || (ui_dir.exists()
                && ui_dir.read_dir().map(|mut d| d.next().is_none()).unwrap_or(true));

        if should_update_submodule {
            // Check if we're in a git repository
            let git_check = Command::new("git").arg("rev-parse").arg("--git-dir").output();
            if git_check.is_ok() && git_check.unwrap().status.success() {
                // Update git submodule
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
                    panic!(
                        "Failed to update git submodule for UI directory at {}",
                        ui_dir.display()
                    );
                }
            } else {
                panic!(
                    "UI directory doesn't exist at {} and couldn't fetch it through git submodule \
                     (not in a git repository)",
                    ui_dir.display()
                );
            }
        }

        let bun_check = Command::new("bun").arg("--version").output();

        if bun_check.is_err() || !bun_check.unwrap().status.success() {
            panic!("Bun is not installed. Please install Bun at https://bun.sh .");
        }

        // Install dependencies if node_modules doesn't exist
        // $CARGO_MANIFEST_DIR/ui/node_modules
        println!("Installing UI dependencies...");

        let status = Command::new("bun")
            .current_dir(&ui_dir)
            .arg("install")
            .status()
            .expect("Failed to install UI dependencies");

        if !status.success() {
            panic!("Failed to install UI dependencies in {}", ui_dir.display());
        }

        println!("Building UI...");
        let status = Command::new("bun")
            .current_dir(&ui_dir)
            .env("IS_EMBEDDED", "true")
            .arg("run")
            .arg("build")
            .status()
            .expect("Failed to build UI");

        if !status.success() {
            panic!("Failed to build UI in {}", ui_dir.display());
        }
    }
}
