use std::fmt::Write;

use katana_cli::BuildInfo;
use katana_node_config::build_info::{BUILD_TIMESTAMP, GIT_SHA, VERSION};

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
/// Starts from [`BuildInfo::current`] (which pulls VERSION/GIT_SHA/BUILD_TIMESTAMP from
/// `katana-node-config`'s build-time constants) and layers on features that only
/// `bin/katana` can see via `cfg!(feature = ...)`.
pub fn build_info() -> BuildInfo {
    let mut bi = BuildInfo::current();
    bi.features = enabled_features();
    bi
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
