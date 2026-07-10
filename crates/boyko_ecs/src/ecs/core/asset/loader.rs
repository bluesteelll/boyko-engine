//! [`AssetLoader`] — pure-CPU byte decoding for one [`Asset`] type — and
//! [`HasLoaders`] — the compile-time-static dispatch table
//! [`AssetServer::decode_bytes`](crate::ecs::core::asset::server::AssetServer::decode_bytes)
//! scans (asset-streaming plan F3).

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::error::AssetError;

/// Decodes raw file bytes into an [`Asset::Cpu`] intermediate for one asset
/// type.
///
/// `decode` is pure host CPU work — no device, no allocator handle, no
/// upload. It is threadpool-parallelizable by construction (`Self::Out::Cpu:
/// Send`; see [`Asset::Cpu`]'s doc), which a future dispatch rung may
/// exploit. The complementary GPU-upload half (turning a decoded `Cpu`
/// into device-resident state) is dispatcher-serial by design and lives in
/// `boyko_render`, NOT here — do not add a `device` parameter to this trait.
pub trait AssetLoader: 'static {
    /// The asset type this loader produces.
    type Out: Asset;

    /// File extensions (lowercase, no leading dot) this loader claims, used
    /// by [`HasLoaders::LOADERS`] to route an extension to this loader.
    const EXTENSIONS: &'static [&'static str];

    /// Decodes `bytes` into the asset's CPU intermediate.
    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError>;
}

/// One extension-routing entry in a [`HasLoaders::LOADERS`] table: the
/// extensions an [`AssetLoader`] claims, plus its TYPED `decode` function
/// pointer (no erasure — `decode`'s type is `A`'s own `Cpu`, fixed at
/// monomorphization time by [`of`](Self::of)'s `L: AssetLoader<Out = A>`
/// bound).
pub struct LoaderEntry<A: Asset> {
    /// File extensions (lowercase, no leading dot) this entry's loader
    /// claims — mirrors the originating [`AssetLoader::EXTENSIONS`].
    pub extensions: &'static [&'static str],
    /// The originating loader's `decode`, monomorphized directly for `A` —
    /// dispatching through this pointer never erases or downcasts.
    pub decode: fn(&[u8]) -> Result<<A as Asset>::Cpu, AssetError>,
}

impl<A: Asset> LoaderEntry<A> {
    /// Builds a [`LoaderEntry`] from an [`AssetLoader`] that decodes into
    /// `A` — the sole, const-evaluable constructor a [`HasLoaders::LOADERS`]
    /// table entry is built with.
    pub const fn of<L: AssetLoader<Out = A>>() -> Self {
        Self { extensions: L::EXTENSIONS, decode: L::decode }
    }
}

/// An [`Asset`] type with a compile-time-static loader dispatch table
/// (asset-streaming plan F3).
///
/// Replaces the former runtime `AssetServer` loader registry (a
/// `HashMap<String, DecodeFn>` boxing each decoded value behind `dyn Any` and
/// resolving it back via `Any::downcast`): `LOADERS` is a `&'static` const
/// slice baked into the binary at compile time, so
/// [`AssetServer::decode_bytes`](crate::ecs::core::asset::server::AssetServer::decode_bytes)
/// dispatches by a plain linear scan over a handful of entries and calls the
/// matched entry's typed `decode` directly — zero allocation, zero dynamic
/// dispatch, zero `TypeId` check (the type is already `A`, not erased).
pub trait HasLoaders: Asset {
    /// The extension-routing table for this asset type. Typically one entry
    /// per supported file format (e.g. one [`AssetLoader`] per extension),
    /// built via [`LoaderEntry::of`].
    const LOADERS: &'static [LoaderEntry<Self>];
}
