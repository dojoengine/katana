use std::fmt::Write;

/// The latest version from Cargo.toml.
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Suffix indicating if it is a dev build.
///
/// A build is considered a dev build if the working tree is dirty
/// or if the current git revision is not on a tag.
///
/// This suffix is typically empty for clean/release builds, and "-dev" for dev builds.
const DEV_BUILD_SUFFIX: &str = env!("DEV_BUILD_SUFFIX");

/// The SHA of the latest commit.
const VERGEN_GIT_SHA: &str = env!("VERGEN_GIT_SHA");

/// The build timestamp.
const VERGEN_BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");

// > 1.0.0-alpha.19 (77d4800)
// > if on dev (ie dirty):  1.0.0-alpha.19-dev (77d4800)
pub fn generate_short() -> &'static str {
    const_format::concatcp!(CARGO_PKG_VERSION, DEV_BUILD_SUFFIX, " (", VERGEN_GIT_SHA, ")")
}

pub fn generate_long() -> String {
    let mut out = String::new();
    writeln!(out, "{}", generate_short()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "features: {}", features().join(",")).unwrap();
    write!(out, "built on: {}", VERGEN_BUILD_TIMESTAMP).unwrap();
    out
}

/// Returns a list of "features" supported (or not) by this build of katana.
fn features() -> Vec<String> {
    let mut features = Vec::new();

    let native = cfg!(feature = "native");
    features.push(format!("{sign}native", sign = sign(native)));

    features
}

/// Returns `+` when `enabled` is `true` and `-` otherwise.
fn sign(enabled: bool) -> &'static str {
    if enabled {
        "+"
    } else {
        "-"
    }
}
