use std::fmt::Write;

use katana_cli::BuildInfo;

/// The latest version from Cargo.toml.
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Suffix indicating if it is a dev build.
///
/// A build is considered a dev build if the working tree is dirty
/// or if the current git revision is not on a tag.
///
/// This suffix is typically empty for clean/release builds, and "-dev" for dev builds.
const DEV_BUILD_SUFFIX: &str = env!("DEV_BUILD_SUFFIX");

/// Version string (pkg version + dev suffix), without the git SHA.
const VERSION: &str = const_format::concatcp!(CARGO_PKG_VERSION, DEV_BUILD_SUFFIX);

/// The SHA of the latest commit.
const GIT_SHA: &str = env!("VERGEN_GIT_SHA");

/// The build timestamp.
const BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");

// > 1.0.0-alpha.19 (77d4800)
// > if on dev (ie dirty):  1.0.0-alpha.19-dev (77d4800)
pub fn generate_short() -> &'static str {
    const_format::concatcp!(VERSION, " (", GIT_SHA, ")")
}

pub fn generate_long() -> String {
    let mut out = String::new();
    writeln!(out, "{}", generate_short()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "features: {}", features().join(",")).unwrap();
    write!(out, "built on: {BUILD_TIMESTAMP}").unwrap();
    out
}

/// Snapshot of this binary's build identity, for `node_getInfo`.
///
/// Unlike [`features`] (used for the human-facing `--version` output, which annotates
/// disabled features with `-` and enabled with `+`), this only lists the *names* of
/// compiled-in features.
pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: VERSION.to_string(),
        git_sha: GIT_SHA.to_string(),
        build_timestamp: BUILD_TIMESTAMP.to_string(),
        features: enabled_features(),
    }
}

/// Returns a list of "features" supported (or not) by this build of katana.
///
/// Human-facing format: `+native` / `-native` — used by the CLI `--version` long output.
fn features() -> Vec<String> {
    let mut features = Vec::new();

    let native = cfg!(feature = "native");
    features.push(format!("{sign}native", sign = sign(native)));

    features
}

/// Returns the set of enabled feature names (no sign prefix) for the `node_getInfo`
/// wire format. Disabled features are never listed.
fn enabled_features() -> Vec<String> {
    let mut feats = Vec::new();
    if cfg!(feature = "native") {
        feats.push("native".to_string());
    }
    feats
}

/// Returns `+` when `enabled` is `true` and `-` otherwise.
fn sign(enabled: bool) -> &'static str {
    if enabled {
        "+"
    } else {
        "-"
    }
}

#[cfg(test)]
mod tests {
    use super::{build_info, generate_short, GIT_SHA, VERSION};

    #[test]
    fn generate_short_is_version_plus_git_sha() {
        assert_eq!(generate_short(), format!("{VERSION} ({GIT_SHA})"));
    }

    #[test]
    fn build_info_features_have_no_sign_prefix() {
        // `build_info()` must publish bare feature names ("native") — never the
        // annotated `+native` / `-native` that `features()` uses for CLI output.
        for f in build_info().features {
            assert!(
                !f.starts_with('+') && !f.starts_with('-'),
                "build_info() leaked a signed feature name: {f:?}"
            );
        }
    }
}
