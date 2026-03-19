/// The currently supported version of the Starknet protocol.
///
/// ## IMPORTANT
///
/// This version must correspond to the minimum Cairo/Sierra version we want to support. Ideally,
/// this version should be synchronized with Dojo's Cairo version.
///
/// As of Cairo v2.10.0, contracts written with that version are only deployable on Starknet
/// >=0.13.4. Check out the [release notes] for more info.
///
/// [release notes]: https://community.starknet.io/t/cairo-v2-10-0-is-out/115362
pub const CURRENT_STARKNET_VERSION: StarknetVersion = StarknetVersion::new([0, 14, 0, 0]);

/// Starknet protocol version.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
pub struct StarknetVersion {
    /// Each segments represents a part of the version number.
    segments: [u8; 4],
}

impl StarknetVersion {
    /// Represents blocks that predate Starknet protocol versioning.
    ///
    /// Early Starknet mainnet blocks were produced before the protocol included a version
    /// field. This constant represents that absence as a first-class value rather than
    /// using `Option` or a sentinel error. No official Starknet release uses version `0.0.0`,
    /// so this is unambiguous.
    pub const UNVERSIONED: Self = Self::new([0, 0, 0, 0]);

    /// Starknet version 0.7.0.
    pub const V0_7_0: Self = Self::new([0, 7, 0, 0]);
    /// Starknet version 0.11.1.
    pub const V0_11_1: Self = Self::new([0, 11, 1, 0]);
    /// Starknet version 0.13.2.
    pub const V0_13_2: Self = Self::new([0, 13, 2, 0]);
    /// Starknet version 0.13.4.
    pub const V0_13_4: Self = Self::new([0, 13, 4, 0]);
}

#[derive(Debug, thiserror::Error)]
pub enum ParseVersionError {
    #[error("invalid version format")]
    InvalidFormat,
    #[error("failed to parse segment: {0}")]
    ParseSegment(#[from] std::num::ParseIntError),
}

impl StarknetVersion {
    pub const fn new(segments: [u8; 4]) -> Self {
        Self { segments }
    }

    /// Parses a version string in the format `x.y.z.w` where x, y, z, w are u8 numbers.
    /// The string can have fewer than 4 segments; missing segments are filled with zeros.
    pub fn parse(version: &str) -> Result<Self, ParseVersionError> {
        if version.is_empty() {
            return Ok(Self::UNVERSIONED);
        }

        let segments = version.split('.').collect::<Vec<&str>>();

        if segments.len() > 4 {
            return Err(ParseVersionError::InvalidFormat);
        }

        let mut buffer = [0u8; 4];
        for (buf, seg) in buffer.iter_mut().zip(segments) {
            *buf = if seg.is_empty() { 0 } else { seg.parse::<u8>()? };
        }

        Ok(Self::new(buffer))
    }
}

impl core::default::Default for StarknetVersion {
    fn default() -> Self {
        CURRENT_STARKNET_VERSION
    }
}

// Formats the version as a string, where each segment is separated by a dot.
// The last segment (fourth part) will not be printed if it's zero.
//
// For example:
// - Version::new([1, 2, 3, 4]) will be displayed as "1.2.3.4"
// - Version::new([1, 2, 3, 0]) will be displayed as "1.2.3"
// - Version::new([0, 2, 3, 0]) will be displayed as "0.2.3"
impl core::fmt::Display for StarknetVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Unversioned blocks display as empty string, matching the empty protocol
        // version field used in early Starknet block hash computation.
        if *self == Self::UNVERSIONED {
            return Ok(());
        }

        for (idx, segment) in self.segments.iter().enumerate() {
            // If it's the last segment, don't print it if it's zero.
            if idx == self.segments.len() - 1 {
                if *segment != 0 {
                    write!(f, ".{segment}")?;
                }
            } else if idx == 0 {
                write!(f, "{segment}")?;
            } else {
                write!(f, ".{segment}")?;
            }
        }

        Ok(())
    }
}

impl TryFrom<String> for StarknetVersion {
    type Error = ParseVersionError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        StarknetVersion::parse(&value)
    }
}

#[cfg(feature = "serde")]
mod serde {
    use super::*;

    // We de/serialize the version from/into a human-readable string format to prevent breaking the
    // database encoding format if ever decide to change its memory representation.

    impl ::serde::Serialize for StarknetVersion {
        fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&self.to_string())
        }
    }

    impl<'de> ::serde::Deserialize<'de> for StarknetVersion {
        fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let s = String::deserialize(deserializer)?;
            StarknetVersion::parse(&s).map_err(::serde::de::Error::custom)
        }
    }
}

/// An error when the version doesn't correspond to any of the official Starknet releases.
///
/// List for all of the official releases can be found at <https://docs.starknet.io/resources/version-notes/>
#[derive(thiserror::Error, Debug)]
#[error("invalid version: {0}")]
pub struct InvalidVersionError(StarknetVersion);

impl TryFrom<StarknetVersion> for starknet_api::block::StarknetVersion {
    type Error = InvalidVersionError;

    fn try_from(version: StarknetVersion) -> Result<Self, Self::Error> {
        match version.segments {
            // Unversioned blocks predate all known releases; map to the earliest.
            [0, 0, 0, 0] => Ok(Self::V0_9_1),
            [0, 9, 1, 0] => Ok(Self::V0_9_1),
            [0, 10, 0, 0] => Ok(Self::V0_10_0),
            [0, 10, 1, 0] => Ok(Self::V0_10_1),
            [0, 10, 2, 0] => Ok(Self::V0_10_2),
            [0, 10, 3, 0] => Ok(Self::V0_10_3),
            [0, 11, 0, 0] => Ok(Self::V0_11_0),
            [0, 11, 0, 2] => Ok(Self::V0_11_0_2),
            [0, 11, 1, 0] => Ok(Self::V0_11_1),
            [0, 11, 2, 0] => Ok(Self::V0_11_2),
            [0, 12, 0, 0] => Ok(Self::V0_12_0),
            [0, 12, 1, 0] => Ok(Self::V0_12_1),
            [0, 12, 2, 0] => Ok(Self::V0_12_2),
            [0, 12, 3, 0] => Ok(Self::V0_12_3),
            [0, 13, 0, 0] => Ok(Self::V0_13_0),
            [0, 13, 1, 0] => Ok(Self::V0_13_1),
            [0, 13, 1, 1] => Ok(Self::V0_13_3),
            [0, 13, 2, 0] => Ok(Self::V0_13_2),
            [0, 13, 2, 1] => Ok(Self::V0_13_2_1),
            [0, 13, 3, 0] => Ok(Self::V0_13_3),
            [0, 13, 4, 0] => Ok(Self::V0_13_4),
            [0, 13, 5, 0] => Ok(Self::V0_13_5),
            [0, 14, 0, 0] => Ok(Self::V0_14_0),
            [0, 14, 1, 0] => Ok(Self::V0_14_1),
            _ => Err(InvalidVersionError(version)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_valid() {
        let version = "1.9.0.0";
        let parsed = StarknetVersion::parse(version).unwrap();
        assert_eq!(parsed.segments, [1, 9, 0, 0]);
        assert_eq!(String::from("1.9.0"), parsed.to_string());
    }

    #[test]
    fn parse_version_missing_parts() {
        let version = "1.9.0";
        let parsed = StarknetVersion::parse(version).unwrap();
        assert_eq!(parsed.segments, [1, 9, 0, 0]);
        assert_eq!("1.9.0", parsed.to_string());
    }

    #[test]
    fn parse_version_invalid_digit_should_fail() {
        let version = "0.fv.1.0";
        assert!(StarknetVersion::parse(version).is_err());
    }

    #[test]
    fn parse_version_missing_digit_default_zero() {
        let version = "1...";
        let parsed = StarknetVersion::parse(version).unwrap();
        assert_eq!(parsed.segments, [1, 0, 0, 0]);
        assert_eq!("1.0.0", parsed.to_string());
    }

    #[test]
    fn parse_version_many_parts_should_succeed() {
        let version = "1.2.3.4";
        let parsed = StarknetVersion::parse(version).unwrap();
        assert_eq!(parsed.segments, [1, 2, 3, 4]);
        assert_eq!("1.2.3.4", parsed.to_string());
    }

    #[test]
    fn parse_invalid_formats() {
        let version = "1.2.3.4.5";
        assert!(StarknetVersion::parse(version).is_err());
    }

    #[test]
    fn parse_empty_string_returns_unversioned() {
        let parsed = StarknetVersion::parse("").unwrap();
        assert_eq!(parsed, StarknetVersion::UNVERSIONED);
        assert_eq!(parsed.segments, [0, 0, 0, 0]);
    }

    #[test]
    fn unversioned_displays_as_empty_string() {
        assert_eq!(StarknetVersion::UNVERSIONED.to_string(), "");
    }

    #[test]
    fn unversioned_roundtrips_through_string() {
        let display = StarknetVersion::UNVERSIONED.to_string();
        let parsed = StarknetVersion::parse(&display).unwrap();
        assert_eq!(parsed, StarknetVersion::UNVERSIONED);
    }

    #[test]
    fn version_ordering() {
        assert!(StarknetVersion::UNVERSIONED < StarknetVersion::V0_7_0);
        assert!(StarknetVersion::V0_7_0 < StarknetVersion::V0_13_2);
        assert!(StarknetVersion::V0_13_2 < CURRENT_STARKNET_VERSION);
    }

    #[cfg(feature = "serde")]
    mod serde {
        use super::*;

        #[test]
        fn rt_human_readable() {
            let version = StarknetVersion::new([1, 2, 3, 4]);
            let serialized = serde_json::to_string(&version).unwrap();
            let deserialized: StarknetVersion = serde_json::from_str(&serialized).unwrap();
            assert_eq!(version, deserialized);
        }

        #[test]
        fn rt_non_human_readable() {
            let version = StarknetVersion::new([1, 2, 3, 4]);
            let serialized = postcard::to_stdvec(&version).unwrap();
            let deserialized: StarknetVersion = postcard::from_bytes(&serialized).unwrap();
            assert_eq!(version, deserialized);
        }

        #[test]
        fn rt_unversioned_human_readable() {
            let version = StarknetVersion::UNVERSIONED;
            let serialized = serde_json::to_string(&version).unwrap();
            assert_eq!(serialized, "\"\"");
            let deserialized: StarknetVersion = serde_json::from_str(&serialized).unwrap();
            assert_eq!(version, deserialized);
        }

        #[test]
        fn rt_unversioned_non_human_readable() {
            let version = StarknetVersion::UNVERSIONED;
            let serialized = postcard::to_stdvec(&version).unwrap();
            let deserialized: StarknetVersion = postcard::from_bytes(&serialized).unwrap();
            assert_eq!(version, deserialized);
        }
    }
}
