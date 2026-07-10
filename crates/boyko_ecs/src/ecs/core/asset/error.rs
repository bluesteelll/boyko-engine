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

    /// The extension resolved to a loader registered for a DIFFERENT asset
    /// type than the one requested — the loader-registry rung's runtime
    /// erasure check (a downcast mismatch), e.g. `decode_bytes::<Material>`
    /// called with an extension whose loader produces a `Mesh`.
    LoaderTypeMismatch {
        /// The extension whose registered loader does not match the
        /// requested asset type.
        extension: String,
    },

    /// [`Assets::fill`](crate::ecs::core::asset::assets::Assets::fill) or
    /// [`Assets::fail`](crate::ecs::core::asset::assets::Assets::fail) was
    /// called with a [`Handle`](crate::ecs::core::asset::handle::Handle)
    /// that does not resolve to a `Reserved` row awaiting exactly this call:
    /// the row is out of range, already `Occupied` (a double-fill), `Vacant`,
    /// or the handle's generation is stale.
    StaleHandle,
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::Io(reason) => write!(f, "asset io error: {reason}"),
            AssetError::Decode(reason) => write!(f, "asset decode failed: {reason}"),
            AssetError::UnsupportedExtension { extension } => {
                write!(f, "unsupported asset extension: '{extension}'")
            }
            AssetError::LoaderTypeMismatch { extension } => {
                write!(f, "extension '{extension}' is registered for a different asset type")
            }
            AssetError::StaleHandle => {
                write!(f, "handle does not resolve to a row awaiting fill/fail")
            }
        }
    }
}

impl std::error::Error for AssetError {}
