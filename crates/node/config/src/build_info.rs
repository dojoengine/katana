//! Build-time identity surfaced via `node_getInfo`.
//!
//! Constants are populated by `build.rs` using vergen when a `.git` directory is
//! available; otherwise `option_env!` falls back to `"unknown"` so crates.io
//! consumers don't hit build failures.

/// Semver version plus any dev-build suffix. Example: `"1.0.0-alpha.19-dev"`.
/// Does not embed the git SHA (that's in [`GIT_SHA`]).
pub const VERSION: &str = const_format::concatcp!(
    env!("CARGO_PKG_VERSION"),
    match option_env!("DEV_BUILD_SUFFIX") {
        Some(s) => s,
        None => "",
    },
);

/// Git commit SHA of the build, e.g. `"77d4800"`. `"unknown"` when vergen couldn't
/// read git metadata at build time.
pub const GIT_SHA: &str = match option_env!("VERGEN_GIT_SHA") {
    Some(s) => s,
    None => "unknown",
};

/// Build timestamp in ISO 8601. `"unknown"` when vergen couldn't emit it.
pub const BUILD_TIMESTAMP: &str = match option_env!("VERGEN_BUILD_TIMESTAMP") {
    Some(s) => s,
    None => "unknown",
};

/// Build-time identity of a Katana node.
///
/// Two constructors:
/// - [`BuildInfo::current`]: real compile-time values from [`VERSION`] / [`GIT_SHA`] /
///   [`BUILD_TIMESTAMP`]. Use this in production paths.
/// - [`BuildInfo::default`]: `"unknown"` sentinels. Use this in tests and library contexts where
///   deterministic values matter (version strings change every commit).
///
/// Callers who want additional metadata (e.g. compiled features visible only in their
/// own crate) can mutate the fields after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// Semver-ish version string, e.g. `"1.0.0-alpha.19-dev"`. Does not embed the git SHA.
    pub version: String,
    /// Git commit SHA of the build.
    pub git_sha: String,
    /// Build timestamp in ISO 8601.
    pub build_timestamp: String,
    /// Compiled-in features, e.g. `["native", "tee"]`.
    pub features: Vec<String>,
}

impl BuildInfo {
    /// Build info from the compile-time constants emitted by `build.rs`.
    ///
    /// `features` is empty; callers populate it from their own crate's `cfg!(feature = ...)`
    /// flags since cargo features are per-crate and not visible across crate boundaries.
    pub fn current() -> Self {
        Self {
            version: VERSION.to_string(),
            git_sha: GIT_SHA.to_string(),
            build_timestamp: BUILD_TIMESTAMP.to_string(),
            features: Vec::new(),
        }
    }
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            version: "unknown".into(),
            git_sha: "unknown".into(),
            build_timestamp: "unknown".into(),
            features: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BuildInfo;

    #[test]
    fn default_uses_unknown_sentinels() {
        let bi = BuildInfo::default();
        assert_eq!(bi.version, "unknown");
        assert_eq!(bi.git_sha, "unknown");
        assert_eq!(bi.build_timestamp, "unknown");
        assert!(bi.features.is_empty());
    }

    #[test]
    fn current_populates_version_from_cargo_pkg_version() {
        let bi = BuildInfo::current();
        // `VERSION` starts with CARGO_PKG_VERSION; assert the prefix rather than the
        // whole string since the DEV_BUILD_SUFFIX varies between clean and dirty trees.
        assert!(
            bi.version.starts_with(env!("CARGO_PKG_VERSION")),
            "expected version to start with CARGO_PKG_VERSION, got {:?}",
            bi.version,
        );
        // Either real VERGEN data or the "unknown" fallback; both are valid outputs.
        assert!(!bi.git_sha.is_empty());
        assert!(!bi.build_timestamp.is_empty());
    }
}
