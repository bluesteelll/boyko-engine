//! [`AssetPaths<A>`] — the per-asset-type path→[`Handle`](crate::ecs::core::asset::handle::Handle)
//! dedup index (asset-streaming plan F4).
//!
//! # Placement — a per-`A` resource, not a field on `AssetServer`/`Assets<T>`
//!
//! [`AssetServer`](crate::ecs::core::asset::server::AssetServer) used to own
//! ONE `HashMap<(TypeId, String), (u32, u32)>` keyed by `(asset TypeId,
//! path)` — the last `HashMap` in the asset system. Splitting it into one
//! [`PathIndex`] per concrete asset type, held in its own resource, removes
//! the map without reintroducing a per-type registry inside `AssetServer`
//! (which would just be a `HashMap` again): `AssetPaths<A>` and
//! [`Assets<A>`](crate::ecs::core::asset::assets::Assets) are already
//! independent per-type resources the caller passes to
//! [`AssetServer::load`](crate::ecs::core::asset::server::AssetServer::load)
//! by hand, so "same path, different asset types → different handles" holds
//! by construction (two distinct `AssetPaths<A>`/`AssetPaths<B>` values can
//! never alias), with zero extra keying logic. It also keeps the loading
//! concern out of `Assets<T>` itself, which stays a pure storage table.

use std::marker::PhantomData;

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::path_index::PathIndex;
use crate::ecs::core::resources::resource::NonSendResource;

/// World-global, per-asset-type path→[`Handle`](crate::ecs::core::asset::handle::Handle)
/// dedup index — registered as a [`NonSendResource`] and passed to
/// [`AssetServer::load`](crate::ecs::core::asset::server::AssetServer::load)
/// alongside that type's [`Assets<A>`](crate::ecs::core::asset::assets::Assets)
/// and [`AssetStaging<A>`](crate::ecs::core::asset::staging::AssetStaging).
///
/// # `NonSendResource`, not `Resource`
///
/// [`PathIndex`] owns a [`VmColumn`](crate::ecs::memory::vm_column::VmColumn),
/// which is unconditionally `!Send`/`!Sync` (a raw `NonNull` with no
/// conditional auto-trait reproduction, unlike
/// [`Assets<T>`](crate::ecs::core::asset::assets::Assets)'s own manual `Send`/
/// `Sync` impls) — so `AssetPaths<A>` is `!Send` regardless of `A`, exactly
/// like [`AssetStaging<A>`](crate::ecs::core::asset::staging::AssetStaging).
///
/// # Not `#[derive(Default)]`
///
/// A blind `#[derive(Default)]` on a struct generic over `A: Asset` would add
/// a spurious `A: Default` bound to the generated impl — the same pitfall
/// [`AssetStaging<A>`](crate::ecs::core::asset::staging::AssetStaging) and
/// [`Assets<T>`](crate::ecs::core::asset::assets::Assets) hand-implement
/// `Default` to avoid; this type follows the same precedent even though its
/// only real field, [`PathIndex`], is `Default`-free itself (constructed via
/// [`PathIndex::new`]).
pub struct AssetPaths<A: Asset> {
    /// The HashMap-free path-hash → `(slot, generation)` index (asset-
    /// streaming plan F4). `pub(crate)` so
    /// [`AssetServer::load`](crate::ecs::core::asset::server::AssetServer::load)
    /// can call [`PathIndex::lookup`]/[`PathIndex::insert`] directly.
    pub(crate) index: PathIndex,
    _marker: PhantomData<A>,
}

impl<A: Asset> Default for AssetPaths<A> {
    fn default() -> Self {
        Self {
            index: PathIndex::new(),
            _marker: PhantomData,
        }
    }
}

impl<A: Asset> NonSendResource for AssetPaths<A> {}
