//! The [`Resource`] trait — marker for world-global singleton types.

use std::any::TypeId;

use crate::ecs::identifiers::primitives::ResourceId;

/// Marker trait for ECS resource types — `World`-global singletons.
///
/// Implemented automatically via `#[derive(Resource)]`. Each type gets a
/// unique [`ResourceId`] assigned on first call to [`resource_id`].
///
/// # `Send + Sync` requirement
/// Resources are read/written by `SystemParam` closures across multiple
/// systems. Phase 9 scheduler will run systems on multiple threads; a
/// non-Sync resource would be unsound to `Res<&'w T>` from a worker. The
/// bound matches Bevy.
///
/// **Future migration path (Phase 9 §9.4):** types that legitimately cannot
/// be `Send + Sync` (e.g., FFI handles, `Rc<T>`-wrapped state) will be
/// supported via a separate `NonSendResource` trait + `NonSendRes<T>` param.
/// Phase 8a does NOT ship this — track in `docs/plans/PHASE-09-scheduler.md`.
///
/// # Drop discipline (C5)
/// A `Resource`'s `Drop` impl MUST NOT call back into `EcsMaster` — no
/// `EcsMaster::insert_*`, `remove_*`, `resource*`, `run_system_once`, no
/// archetype/entity queries. The world is mid-teardown when `Drop` runs and
/// only the resource itself is guaranteed to be valid. Violations are
/// detectable in debug builds via re-entrancy guards (Phase 9 §9.5).
///
/// # Component-vs-Resource exclusivity (M6)
/// A type may not be both `#[derive(Component)]` and `#[derive(Resource)]`.
/// The runtime registration at `register_new` panics with a clear
/// diagnostic if the type is already registered as a Component.
///
/// # Panic safety
/// `<Self as Drop>::drop` must not panic. If it does, `Resources::Drop`
/// (or `insert` replace path) clears the registered_mask bit BEFORE
/// calling drop, so the observable state on unwind is "slot empty"
/// (leak rather than UB). See R4.
///
/// [`resource_id`]: Resource::resource_id
pub trait Resource: 'static + Send + Sync + Sized {
    /// Returns the unique identifier for this resource type.
    ///
    /// The first call mints the ID via the global registry; subsequent calls
    /// return the cached value from a per-type `OnceLock` — no atomic on the
    /// hot path after initialization.
    fn resource_id() -> ResourceId;

    /// Returns the demangled type name for diagnostics.
    #[inline]
    fn debug_type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Returns the compile-time `TypeId` for this resource type.
    #[inline]
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }

    /// Returns `size_of::<Self>()`.
    #[inline]
    fn mem_size() -> usize {
        std::mem::size_of::<Self>()
    }

    /// Returns `align_of::<Self>()`.
    #[inline]
    fn alignment() -> usize {
        std::mem::align_of::<Self>()
    }
}

/// Marker trait for **non-`Send`** world-global singleton types (Phase 4
/// Seam 2 — D6 + CR-A).
///
/// Unlike [`Resource`], a `NonSendResource` carries **no `Send + Sync`
/// bound** — that is the entire point. It homes types that legitimately
/// cannot cross thread boundaries: RHI `Device`/`Queue` handles, FFI
/// pointers, `Rc<T>`-wrapped state. Implemented for any `'static` type
/// (manually or via a future derive).
///
/// # Where the `!Send` payload is touched (CR-A)
///
/// The value lives in a **separate** type-erased slab
/// ([`NonSendResources`]) on `EcsMaster`, kept structurally `Send + Sync`
/// by erasing the type to a raw pointer + drop fn + `TypeId` — exactly like
/// [`Resources`]. The `!Send` payload `R` is reachable only through the
/// `unsafe` `NonSendRes`/`NonSendResMut::get_param` accessors, whose SAFETY
/// contract is the apply-window single-thread-touch invariant: a system
/// that takes a NonSend param declares **universal access**, so it resolves
/// to `SystemKind::CpuExclusive` and runs ONLY on the dispatcher thread when
/// `running == 0` (no worker is live, no concurrent touch). This reuses the
/// existing apply-window discipline — it introduces **no new soundness
/// model**, mirroring Bevy's separate non-send storage.
///
/// # Drop discipline (C5)
///
/// As with [`Resource`], a `NonSendResource`'s `Drop` impl MUST NOT call
/// back into `EcsMaster`. The non-send slab drops AFTER the `Send + Sync`
/// resources slab (it is declared immediately after `resources` — the C5
/// drop-order contract keeps `resources` first).
///
/// [`Resources`]: crate::ecs::core::resources::resources::Resources
/// [`NonSendResources`]: crate::ecs::core::resources::nonsend_resources::NonSendResources
pub trait NonSendResource: 'static {}
