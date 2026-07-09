//! [`AssetError`] — the domain error type for asset decode failures.
//!
//! Mirrors [`EcsError`](crate::ecs::error::EcsError)'s convention: a
//! `#[non_exhaustive]` enum with a `Display` + `std::error::Error` impl,
//! never `anyhow`.

/// Failures that can arise while loading or decoding an asset.
///
/// `#[non_exhaustive]`: new variants may be added in minor versions.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    /// Reading the source bytes failed (file missing, permission denied,
    /// truncated stream, ...). Carries a human-readable description rather
    /// than `std::io::Error` (which is neither `Clone` nor `PartialEq`,
    /// both required here to keep `AssetError` usable in equality-based
    /// tests and caches).
    Io(String),

    /// [`AssetLoader::decode`](crate::ecs::core::asset::loader::AssetLoader::decode)
    /// rejected the bytes (malformed header, checksum mismatch, unsupported
    /// version, ...).
    Decode(String),

    /// The path's extension does not match any registered
    /// [`AssetLoader::EXTENSIONS`](crate::ecs::core::asset::loader::AssetLoader::EXTENSIONS).
    UnsupportedExtension {
        /// The extension actually found on the path (lowercased, no dot).
        extension: String,
    },
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::Io(reason) => write!(f, "asset io error: {reason}"),
            AssetError::Decode(reason) => write!(f, "asset decode failed: {reason}"),
            AssetError::UnsupportedExtension { extension } => {
                write!(f, "unsupported asset extension: '{extension}'")
            }
        }
    }
}

impl std::error::Error for AssetError {}
