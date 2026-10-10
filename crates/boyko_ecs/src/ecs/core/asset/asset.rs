//! The [`Asset`] marker trait and [`AssetLoadState`] — the two small types
//! [`Assets<T>`](crate::ecs::core::asset::assets::Assets) and
//! [`AssetLoader`](crate::ecs::core::asset::loader::AssetLoader) share.

/// Marker trait for a type stored in an
/// [`Assets<T>`](crate::ecs::core::asset::assets::Assets) table.
///
/// `T: Asset` is the host-resident representation the table holds directly
/// (e.g. a decoded mesh or material description). [`Asset::Cpu`] is the
/// intermediate an [`AssetLoader`](crate::ecs::core::asset::loader::AssetLoader)
/// decodes raw bytes into — GPU upload (turning `Cpu` into device-resident
/// state) is deliberately NOT part of this trait: it lives in `boyko_render`
/// at a later rung (A1+), because the kernel core stays render-agnostic
/// (`boyko_ecs` cannot depend on `boyko_render` / `boyko_rhi_vulkan`).
pub trait Asset: 'static + Sized {
    /// The decoded intermediate an [`AssetLoader`](crate::ecs::core::asset::loader::AssetLoader)
    /// produces from raw bytes.
    ///
    /// Bound by `Send` (not `Send + Sync`) so decode can ride the
    /// threadpool once the loader dispatch lands (rung A5): a decoded value
    /// is produced on a worker thread and handed to the dispatcher, which
    /// never requires shared (`Sync`) access to it. GPU upload consuming
    /// `Cpu` is dispatcher-serial by design (device calls are not
    /// thread-safe) — decode is the only half of loading that ever
    /// parallelizes.
    ///
    /// `+ 'static`: `Cpu` is stored inside a queued
    /// [`Staged<A>`](crate::ecs::core::asset::staging::Staged) entry on a
    /// [`NonSendResource`](crate::ecs::core::resources::resource::NonSendResource)
    /// (`AssetStaging<A>`), which — like every other `TypeId`-registered
    /// resource in the kernel — requires its whole type graph to carry no
    /// borrowed lifetime. Every concrete `Cpu` today (`MaterialGpu`,
    /// `MeshData`, `()`) already satisfies this trivially.
    type Cpu: Send + 'static;
}

/// The lifecycle state of one row in an
/// [`Assets<T>`](crate::ecs::core::asset::assets::Assets) table.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetLoadState {
    /// Decode/upload has been requested but has not completed yet.
    Loading = 0,
    /// The asset is fully resident and safe to read.
    Loaded = 1,
    /// Decode or upload failed; the row holds whatever value [`Assets::add`]
    /// was called with (callers that model failure explicitly should keep a
    /// sentinel/default `T` for this state).
    ///
    /// [`Assets::add`]: crate::ecs::core::asset::assets::Assets::add
    Failed = 2,
}
