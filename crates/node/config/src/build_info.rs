/// Build-time identity of a running Katana node.
///
/// Populated by the top-level binary (`bin/katana`, or any downstream embedder) from
/// its own compile-time metadata and handed to the node via `Config::build_info`. The
/// node library itself does not self-identify, so that wrappers and forks can report
/// their own version/git SHA via `node_getInfo` rather than katana's.
///
/// Callers who don't set this field get `"unknown"` sentinels via [`Default`], which
/// is also what tests and programmatic library consumers see unless they explicitly
/// populate it.
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
}
