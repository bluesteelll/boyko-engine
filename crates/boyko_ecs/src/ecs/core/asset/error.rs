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

    /// The path's extension is not claimed by any entry in the requested
    /// asset type's [`HasLoaders::LOADERS`](crate::ecs::core::asset::loader::HasLoaders::LOADERS)
    /// table.
    UnsupportedExtension {
        /// The extension actually found on the path (lowercased, no dot).
        extension: String,
    },

    /// Formerly: the extension resolved to a loader registered for a
    /// DIFFERENT asset type than the one requested — the old runtime
    /// registry's `Any::downcast` mismatch. Unconstructible since
    /// asset-streaming plan F3 replaced the `Box<dyn Any>` registry with the
    /// compile-time-static [`HasLoaders`](crate::ecs::core::asset::HasLoaders)
    /// dispatch table (a type mismatch is now a compile error, not a runtime
    /// one). Kept only to avoid churn on this `#[non_exhaustive]` enum.
    #[deprecated(
        note = "unconstructible since F3 static HasLoaders dispatch replaced the Box<dyn Any> \
                registry; a loader/asset-type mismatch is now a compile error"
    )]
    #[doc(hidden)]
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
    // Matching the deprecated `LoaderTypeMismatch` arm below is itself deprecated
    // usage; this impl is the sole remaining reader (kept so the variant still
    // formats sensibly if a caller somehow constructs one via `..`-update or a
    // future non-deprecated re-add).
    #[allow(deprecated)]
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
