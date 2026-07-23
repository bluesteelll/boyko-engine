//! [`AssetBacking`] — the trait an [`Assets<T>`](crate::ecs::core::asset::assets::Assets)
//! element type implements to obtain a store-owned
//! [`ComponentPool`](crate::ecs::memory::component_pool::ComponentPool) layout
//! (asset-streaming plan F1).
//!
//! # Why a macro, not a blanket impl
//!
//! A blanket `impl<T: bytemuck::Pod> AssetBacking for T` plus a concrete
//! `impl AssetBacking for MeshGpu` (a non-`Pod` resident type) is **E0119** on
//! stable: coherence cannot prove `MeshGpu: !Pod` (no negative reasoning over
//! a foreign trait), so the two impls would be seen as potentially
//! overlapping. [`impl_asset_pod_backing!`](crate::impl_asset_pod_backing) sidesteps this entirely by
//! generating one concrete, non-generic `impl AssetBacking for $t` per
//! invocation — no blanket impl ever exists, so a later hand-written impl
//! (e.g. [`MeshGpu`](../../../../../boyko_render/struct.MeshGpu.html)'s
//! manual-drop-glue impl in `boyko_render`) can never collide with it.

use std::any::TypeId;
// Once-per-type `TypeId → ComponentId` mint registry (`ASSET_LAYOUTS` below):
// rust#22991 collapses a `static` declared inside a generic fn body across all
// monomorphisations, so the memoization MUST be one shared TypeId-keyed map
// behind one lock. Touched once per concrete asset type at store construction
// (`Assets::with_reserved` → `T::register_layout()`); the resolved `ComponentId`
// is then owned by the store's `ComponentPool`, so no per-frame path re-enters.
#[allow(clippy::disallowed_types)]
use std::collections::HashMap;
#[allow(clippy::disallowed_types)]
use std::sync::{Mutex, OnceLock};

use crate::ecs::core::component::component_registry::{self, ComponentLayout, DropFn};
use crate::ecs::identifiers::primitives::ComponentId;

/// A type storable directly in an [`Assets<T>`](crate::ecs::core::asset::assets::Assets)
/// table's [`ComponentPool`](crate::ecs::memory::component_pool::ComponentPool)
/// column.
///
/// Implemented via the [`impl_asset_pod_backing!`](crate::impl_asset_pod_backing) macro for a plain-old-data
/// type with no device teardown (`NEEDS_TEARDOWN = false`, `drop_fn = None`),
/// or by hand for a type whose teardown must be threaded through explicitly
/// (`NEEDS_TEARDOWN = true`, a manual `drop_fn` — e.g. a GPU-resident asset
/// record whose destructor cannot run device calls, see `MeshGpu`'s
/// `boyko_render`-side impl).
///
/// `register_layout` mints (once per concrete `T`, memoized) the
/// [`ComponentId`] the store's
/// [`ComponentPool`](crate::ecs::memory::component_pool::ComponentPool) is built from — see
/// [`register_asset_layout`].
///
/// # No `Send + Sync` supertrait bound
///
/// Deliberately `Sized + 'static` only, NOT `Send + Sync`: a GPU-resident
/// asset record (`MeshGpu`) owns non-`Send` RHI buffers and is registered as
/// a [`NonSendResource`](crate::ecs::core::resources::resource::NonSendResource),
/// so it must be able to implement `AssetBacking` while staying `!Send`.
/// [`Assets<T>`](crate::ecs::core::asset::assets::Assets)'s OWN `Send`/`Sync`
/// are instead reproduced conditionally on `T`'s auto-trait profile (`unsafe
/// impl<T: AssetBacking + Send> Send for Assets<T>` + the `Sync` mirror,
/// exactly the `Vec<T>` profile) — requiring `Send + Sync` here would make
/// that conditional impl vacuous (every `AssetBacking` type would already
/// satisfy both) and would make `impl AssetBacking for MeshGpu` itself
/// impossible to write.
pub trait AssetBacking: Sized + 'static {
    /// `true` iff a live row of this type owns a resource that must be freed
    /// through something other than `Self`'s own `Drop` (e.g. a device
    /// buffer). Purely descriptive in F1 (the streaming teardown path that
    /// consults it lands at F6) — a POD type backed by
    /// [`impl_asset_pod_backing!`](crate::impl_asset_pod_backing) is always `false`.
    const NEEDS_TEARDOWN: bool;

    /// Returns the [`ComponentId`] this type's store column is built from,
    /// registering the layout (once per concrete `T`, memoized) on first call.
    fn register_layout() -> ComponentId;
}

/// Process-global registry mapping `TypeId::of::<T>()` to the [`ComponentId`]
/// minted for that asset-backing type — the `AssetBacking::register_layout`
/// memoization.
///
/// A literal `static ID: OnceLock<ComponentId>` declared INSIDE a generic
/// function body would not be soundly monomorphised per `T` (the same
/// rust#22991 / rfcs#2130 trap [`resource_id_for`](crate::ecs::core::resources::resource_type_registry::resource_id_for)
/// documents and works around for generic `Resource`s) — a `TypeId`-keyed map
/// behind ONE process-global lock is the proven fix, reused verbatim here.
// Once-per-type mint registry (rust#22991 forces the shared map); store
// construction only — never a per-frame path.
#[allow(clippy::disallowed_types)]
static ASSET_LAYOUTS: OnceLock<Mutex<HashMap<TypeId, ComponentId>>> = OnceLock::new();

/// Mints (once per concrete `T`, memoized) the [`ComponentId`] an
/// [`AssetBacking`] impl's `register_layout` returns.
///
/// Registers a [`ComponentLayout`] built directly from `T`'s own
/// `size_of`/`align_of`/`TypeId` plus the CALLER-SUPPLIED `drop_fn` — NOT
/// `T`'s auto-derived `mem::needs_drop` glue (unlike
/// [`component_registry::register_new`]): a resident asset type may need a
/// device teardown its own (or no) `Drop` impl cannot express (see
/// `MeshGpu`'s `boyko_render`-side impl, which passes a manual drop_fn even
/// though `MeshGpu` itself implements no `Drop`), so the caller controls the
/// glue explicitly rather than deferring to `T::drop`.
///
/// The get-or-mint is atomic under the registry `Mutex`, mirroring
/// `resource_id_for`'s discipline: the map is probed, a fresh id minted via
/// [`component_registry::try_register_dynamic`] only if absent, inserted, and
/// returned, all inside one lock.
///
/// # Panics
///
/// If the process-global `ComponentId` space
/// ([`component_registry::MAX_COMPONENTS`]) is exhausted, or if the registry
/// `Mutex` is poisoned by a previous panicking caller.
// Once-per-concrete-`T` mint, called from `Assets::with_reserved` at store
// construction; the minted `ComponentId` is cached in the store, so no
// per-frame path locks this map.
#[allow(clippy::disallowed_types)]
pub fn register_asset_layout<T: 'static>(drop_fn: Option<DropFn>) -> ComponentId {
    let registry = ASSET_LAYOUTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .expect("invariant: asset layout registry mutex poisoned");
    let key = TypeId::of::<T>();
    if let Some(&id) = map.get(&key) {
        return id;
    }
    let layout = ComponentLayout {
        size: std::mem::size_of::<T>(),
        alignment: std::mem::align_of::<T>(),
        drop_fn,
        type_name: std::any::type_name::<T>(),
        type_id: key,
    };
    let id = component_registry::try_register_dynamic(layout)
        .expect("invariant: asset ComponentId space exhausted (MAX_COMPONENTS)");
    map.insert(key, id);
    id
}

/// Generates one concrete, non-generic `impl AssetBacking for $t` per type
/// listed — the sanctioned way to back a plain-old-data asset type with NO
/// device teardown (`NEEDS_TEARDOWN = false`, `drop_fn = None`).
///
/// See the module doc for why this is a macro rather than a blanket impl.
/// `$crate`-qualified so the macro is usable from ANY crate (the defining
/// crate, `boyko_ecs`, is fixed by `$crate` hygiene regardless of the
/// invoking crate) without the caller needing to `use` `AssetBacking` /
/// `ComponentId` / `register_asset_layout` first.
#[macro_export]
macro_rules! impl_asset_pod_backing {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::ecs::core::asset::backing::AssetBacking for $t {
                const NEEDS_TEARDOWN: bool = false;

                #[inline]
                fn register_layout() -> $crate::ecs::identifiers::primitives::ComponentId {
                    $crate::ecs::core::asset::backing::register_asset_layout::<$t>(None)
                }
            }
        )+
    };
}
