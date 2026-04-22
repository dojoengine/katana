use katana_chain_spec::ChainSpec;
use katana_node_config::build_info::BuildInfo;
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

/// Identity and build information for a running Katana node.
///
/// On full nodes, `chain_id` and `chain_kind` reflect the *configured* chain (via
/// `config.network`), not the chain observed from the upstream sync source. Operators
/// who point a sync source at the wrong network will still see their configured
/// identity reported here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// Semver-ish version string, e.g. `"1.0.0-alpha.19-dev"`.
    /// Does not embed the git SHA (see `git_sha`).
    pub version: String,
    /// Git commit SHA of the build, e.g. `"77d4800"`. `"unknown"` for non-CLI builds.
    pub git_sha: String,
    /// Build timestamp in ISO 8601. `"unknown"` for non-CLI builds.
    pub build_timestamp: String,
    /// Names of enabled compile-time features. Currently only `"native"` is reported;
    /// detection for additional features (tee, grpc, cartridge) is not wired up. Empty
    /// for non-CLI builds.
    pub features: Vec<String>,
    /// Chain identifier as a raw Felt (hex-encoded, same wire format as
    /// `starknet_chainId`).
    pub chain_id: Felt,
    /// Discriminator for the `ChainSpec` variant the node was built with. Note that
    /// both mainnet and sepolia map to `FullNode` — consult `chain_id` to distinguish.
    pub chain_kind: ChainKind,
}

impl NodeInfo {
    /// Construct from a build-info snapshot and a chain spec. Both node implementations
    /// call this at registration time so the wire format stays consistent.
    pub fn from_parts(build_info: &BuildInfo, chain_spec: &ChainSpec) -> Self {
        Self {
            version: build_info.version.clone(),
            git_sha: build_info.git_sha.clone(),
            build_timestamp: build_info.build_timestamp.clone(),
            features: build_info.features.clone(),
            chain_id: chain_spec.id().id(),
            chain_kind: ChainKind::from(chain_spec),
        }
    }
}

/// JSON discriminator that mirrors the variants of [`ChainSpec`].
///
/// Serialized as PascalCase (`"Dev"`, `"Rollup"`, `"FullNode"`) rather than
/// camelCase, matching how enum tags are conventionally written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainKind {
    Dev,
    Rollup,
    FullNode,
}

impl From<&ChainSpec> for ChainKind {
    fn from(spec: &ChainSpec) -> Self {
        match spec {
            ChainSpec::Dev(_) => Self::Dev,
            ChainSpec::Rollup(_) => Self::Rollup,
            ChainSpec::FullNode(_) => Self::FullNode,
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_chain_spec::ChainSpec;
    use serde_json::json;

    use super::{ChainKind, NodeInfo};

    #[test]
    fn chain_kind_from_chain_spec_dev() {
        assert_eq!(ChainKind::from(&ChainSpec::dev()), ChainKind::Dev);
    }

    #[test]
    fn chain_kind_from_chain_spec_full_node() {
        assert_eq!(ChainKind::from(&ChainSpec::mainnet()), ChainKind::FullNode);
        assert_eq!(ChainKind::from(&ChainSpec::sepolia()), ChainKind::FullNode);
    }

    #[test]
    fn chain_kind_serializes_as_pascal_case() {
        assert_eq!(serde_json::to_value(ChainKind::Dev).unwrap(), json!("Dev"));
        assert_eq!(serde_json::to_value(ChainKind::Rollup).unwrap(), json!("Rollup"));
        assert_eq!(serde_json::to_value(ChainKind::FullNode).unwrap(), json!("FullNode"));
    }

    #[test]
    fn chain_kind_deserializes_pascal_case() {
        assert_eq!(serde_json::from_value::<ChainKind>(json!("Dev")).unwrap(), ChainKind::Dev);
        assert_eq!(
            serde_json::from_value::<ChainKind>(json!("Rollup")).unwrap(),
            ChainKind::Rollup
        );
        assert_eq!(
            serde_json::from_value::<ChainKind>(json!("FullNode")).unwrap(),
            ChainKind::FullNode,
        );
        // Pin the casing: lowercase must fail.
        assert!(serde_json::from_value::<ChainKind>(json!("dev")).is_err());
        assert!(serde_json::from_value::<ChainKind>(json!("fullNode")).is_err());
    }

    #[test]
    fn node_info_round_trips_through_serde() {
        let info = NodeInfo {
            version: "1.2.3-dev".into(),
            git_sha: "abcdef0".into(),
            build_timestamp: "2026-04-21T00:00:00Z".into(),
            features: vec!["native".into(), "tee".into()],
            chain_id: ChainSpec::dev().id().id(),
            chain_kind: ChainKind::Dev,
        };

        let json = serde_json::to_value(&info).unwrap();

        // Field names are camelCase; chain_kind value is PascalCase.
        assert_eq!(json["version"], json!("1.2.3-dev"));
        assert_eq!(json["gitSha"], json!("abcdef0"));
        assert_eq!(json["buildTimestamp"], json!("2026-04-21T00:00:00Z"));
        assert_eq!(json["features"], json!(["native", "tee"]));
        assert_eq!(json["chainKind"], json!("Dev"));
        // chain_id is a raw hex Felt string (same shape as starknet_chainId).
        assert!(
            json["chainId"].is_string(),
            "chainId must serialize as a hex string, not an enum object — got {}",
            json["chainId"]
        );

        let roundtrip: NodeInfo = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, info);
    }
}
