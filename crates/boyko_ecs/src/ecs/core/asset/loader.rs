//! [`AssetLoader`] — pure-CPU byte decoding for one [`Asset`] type.

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::error::AssetError;

/// Decodes raw file bytes into an [`Asset::Cpu`] intermediate for one asset
/// type.
///
/// `decode` is pure host CPU work — no device, no allocator handle, no
/// upload. It is threadpool-parallelizable by construction (`Self::Out::Cpu:
/// Send`; see [`Asset::Cpu`]'s doc), which the loader-registry rung (A5)
/// will exploit. The complementary GPU-upload half (turning a decoded `Cpu`
/// into device-resident state) is dispatcher-serial by design and lives in
/// `boyko_render`, NOT here — do not add a `device` parameter to this trait.
pub trait AssetLoader: 'static {
    /// The asset type this loader produces.
    type Out: Asset;

    /// File extensions (lowercase, no leading dot) this loader claims, used
    /// by the loader registry (A3) to route a path to a loader.
    const EXTENSIONS: &'static [&'static str];

    /// Decodes `bytes` into the asset's CPU intermediate.
    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError>;
}
