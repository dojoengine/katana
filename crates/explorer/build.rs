use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ui/");

    // Check if we're in a build script
    if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
        // $CARGO_MANIFEST_DIR/ui/
        let ui_dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("ui");
        println!("Explorer UI directory: {}", ui_dir.display());

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
            panic!("Failed to update git submodule");
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
            panic!("Failed to install UI dependencies");
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
            panic!("Failed to build UI");
        }
    }
}
