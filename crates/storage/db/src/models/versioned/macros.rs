/// Macro for generating versioned types with automatic serialization/deserialization support.
///
/// This macro simplifies the process of adding new database versions by automatically generating:
/// - The versioned enum with all version variants
/// - `From` trait implementations for conversions
/// - `Compress` and `Decompress` implementations with fallback chain
/// - Optimized decompression with hot/cold path hints
///
/// ## Performance Optimization
///
/// The decompression implementation marks older version paths as `#[cold]` and `#[inline(never)]`
/// to optimize for the common case where most data is in the latest format. This helps the
/// compiler generate better code for the typical hot path (latest version)
///
/// # Example
///
/// ```rust
/// versioned_type! {
///     VersionedTx {
///         V6 => v6::Tx,
///         V7 => katana_primitives::transaction::Tx,
///     }
/// }
/// ```
///
/// To add a new version, simply add a new line:
/// ```rust
/// versioned_type! {
///     VersionedTx {
///         V6 => v6::Tx,
///         V7 => katana_primitives::transaction::Tx,
///         V8 => v8::Tx,  // New version added here
///     }
/// }
/// ```
macro_rules! versioned_type {
    (
        $enum_name:ident {
            $($version:ident => $type_path:ty),+ $(,)?
        }
    ) => {
        // Count versions to identify the latest one
        versioned_type!(@count $enum_name, $($version => $type_path),+);
    };

    // Helper to generate the enum and implementations
    (@count $enum_name:ident, $($version:ident => $type_path:ty),+) => {
        // Generate the versioned enum
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
        #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
        pub enum $enum_name {
            $($version($type_path)),+
        }

        // Get the latest version (last in the list)
        versioned_type!(@impl_latest $enum_name, $($version => $type_path),+);

        // Generate Compress implementation
        impl $crate::codecs::Compress for $enum_name {
            type Compressed = Vec<u8>;

            fn compress(self) -> Result<Self::Compressed, $crate::error::CodecError> {
                postcard::to_stdvec(&self)
                    .map_err(|e| $crate::error::CodecError::Compress(e.to_string()))
            }
        }

        // Generate Decompress implementation with fallback chain
        impl $crate::codecs::Decompress for $enum_name {
            fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, $crate::error::CodecError> {
                let bytes = bytes.as_ref();

                // First try to deserialize as the versioned enum itself
                if let Ok(value) = postcard::from_bytes::<Self>(bytes) {
                    return Ok(value);
                }

                // Try each version in reverse order (newest first)
                versioned_type!(@decompress_chain $enum_name, bytes, $($version => $type_path),+);

                Err($crate::error::CodecError::Decompress(
                    format!("failed to deserialize {}: unknown format", stringify!($enum_name))
                ))
            }
        }
    };

    // Helper to implement From trait for the latest version
    (@impl_latest $enum_name:ident, $($version:ident => $type_path:ty),+) => {
        // Extract the last version as the latest
        versioned_type!(@impl_latest_inner $enum_name, [$($version => $type_path),+] []);
    };

    (@impl_latest_inner $enum_name:ident, [$last_version:ident => $last_type:ty] [$($version:ident => $type_path:ty),*]) => {
        // Implement From for the latest version (converting to enum)
        impl From<$last_type> for $enum_name {
            fn from(value: $last_type) -> Self {
                $enum_name::$last_version(value)
            }
        }

        // Implement From for converting enum to latest version
        impl From<$enum_name> for $last_type {
            fn from(versioned: $enum_name) -> Self {
                match versioned {
                    $($enum_name::$version(value) => value.into(),)*
                    $enum_name::$last_version(value) => value,
                }
            }
        }

    };

    (@impl_latest_inner $enum_name:ident,
        [$current_version:ident => $current_type:ty, $($rest_version:ident => $rest_type:ty),+]
        [$($processed_version:ident => $processed_type:ty),*]) => {
        // Recursively process to find the last version
        versioned_type!(@impl_latest_inner $enum_name,
            [$($rest_version => $rest_type),+]
            [$($processed_version => $processed_type,)* $current_version => $current_type]);
    };

    // Helper to generate the decompress fallback chain
    (@decompress_chain $enum_name:ident, $bytes:ident, $($version:ident => $type_path:ty),+) => {
        // Generate in reverse order for newest-first attempts
        versioned_type!(@decompress_chain_inner $enum_name, $bytes, [$($version => $type_path),+] []);
    };

    (@decompress_chain_inner $enum_name:ident, $bytes:ident, [] [$($version:ident => $type_path:ty),+]) => {
        // Generate the actual deserialization attempts
        // Split into first (hot path) and rest (cold paths)
        versioned_type!(@decompress_chain_split $enum_name, $bytes, [$($version => $type_path),+]);
    };

    (@decompress_chain_inner $enum_name:ident, $bytes:ident,
        [$current_version:ident => $current_type:ty $(, $rest_version:ident => $rest_type:ty)*]
        [$($processed_version:ident => $processed_type:ty),*]) => {
        // Build the list in reverse order
        versioned_type!(@decompress_chain_inner $enum_name, $bytes,
            [$($rest_version => $rest_type),*]
            [$current_version => $current_type $(, $processed_version => $processed_type)*]);
    };

    // Split the versions into hot (latest) and cold (older) paths
    (@decompress_chain_split $enum_name:ident, $bytes:ident,
        [$latest_version:ident => $latest_type:ty $(, $older_version:ident => $older_type:ty)*]) => {
        // Latest version is the hot path
        if let Ok(value) = postcard::from_bytes::<$latest_type>($bytes) {
            return Ok($enum_name::$latest_version(value));
        }

        // Older versions are cold paths - mark with #[cold] attribute
        $(
            {
                // Use inline(never) and cold to hint this is unlikely
                #[inline(never)]
                #[cold]
                fn try_deserialize_old(bytes: &[u8]) -> Option<$older_type> {
                    postcard::from_bytes::<$older_type>(bytes).ok()
                }

                if let Some(value) = try_deserialize_old($bytes) {
                    return Ok($enum_name::$older_version(value));
                }
            }
        )*
    };
}
