use std::env;

fn main() {
    // Auto-enable the `debug` feature in debug (non-release) builds.
    // Vergen-sourced VERSION / GIT_SHA / BUILD_TIMESTAMP now live in
    // `katana-node-config/build.rs`; see `katana_node_config::build_info`.
    let profile = env::var("PROFILE").unwrap_or_default();
    if profile == "debug" {
        println!("cargo:rustc-cfg=feature=\"debug\"");
    }
}
