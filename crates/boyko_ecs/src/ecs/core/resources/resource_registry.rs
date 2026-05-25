//! Global registry of resource type metadata.
//!
//! # Resource ID assignment
//!
//! Each distinct type `R` implementing [`Resource`] is assigned a unique
//! [`ResourceId`] the first time `R::resource_id()` is called in the current
//! process. The assignment is lazy, lock-free on the cached read path, and
//! stable for the lifetime of the process — but **not** stable across
//! processes or across runs of the same binary if the order of first calls
//! differs. Mirror of the `component_registry` design.
//!
//! # Component-vs-Resource exclusivity (M6)
//!
//! A single Rust type may not be registered as both a `Component` and a
//! `Resource`. [`register_new`] checks the component registry on every call
//! and panics with a clear diagnostic if the type is already a component.
//!
//! # Threading
//!
//! All registry operations are safe to call from any thread. The global
//! `NEXT_RESOURCE_ID` counter uses `Relaxed` ordering (uniqueness is
//! sufficient; cross-thread happens-before is provided by `OnceLock::set` /
//! `get`). The Component/Resource exclusivity check is **best-effort** in a
//! concurrent setting (W4) — see [`register_new`] for the rationale.

// Phase 8a Step 1: the items below are exercised by this module's own
// `#[cfg(test)]` block; their non-test consumers (`Resources` slab in
// Step 2, `EcsMaster::insert_resource` in Step 9) are not yet checked in.
// The blanket `dead_code` allow is removed automatically as those consumers
// land in subsequent commits.
#![allow(dead_code)]

use std::any::TypeId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::component::component_registry;
use crate::ecs::core::resources::resource::Resource;

/// Number of resource slots in the global registry.
///
/// Locked to `BitSet256` width — raising this requires a wider bitset
/// (Phase 9+ backlog: parameterizable `BitSetN<N>`).
pub const RESOURCE_SLOT_COUNT: usize = 256;

/// Type-erased drop function pointer for a resource type `R`.
///
/// Stored in [`ResourceInfo::drop_fn`] for types where `mem::needs_drop::<R>()`
/// is true. Invoked by the `Resources` slab on `insert` (replace path),
/// `remove`, and during `Drop`.
///
/// # Safety
/// The caller must guarantee:
/// - `ptr` points at a properly-aligned, fully-initialized instance of `R`.
/// - `ptr` is not aliased and the value will not be read or dropped again
///   after this call.
pub type ResourceDropFn = unsafe fn(*mut u8);

/// Type-erased drop glue for `R`.
///
/// Stored as [`ResourceInfo::drop_fn`] when `mem::needs_drop::<R>()` is true.
///
/// # Safety
/// See [`ResourceDropFn`] contract above.
#[inline]
pub(crate) unsafe fn resource_drop_in_place_glue<R: 'static>(ptr: *mut u8) {
    // SAFETY: caller upholds the ResourceDropFn contract: ptr is aligned,
    // initialized, exclusively owned, and not accessed again after this call.
    unsafe { core::ptr::drop_in_place::<R>(ptr.cast::<R>()) }
}

/// Static metadata for one resource type.
///
/// Filled by [`register_new`] and read lock-free via [`get_resource_info`].
///
/// Field order is cache-line friendly: hot fields (size, alignment, drop_fn)
/// at lower offsets; cold fields (type_name, type_id) at higher offsets.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ResourceInfo {
    /// Size in bytes — hot.
    pub size: usize,
    /// Alignment requirement — hot.
    pub alignment: usize,
    /// Drop function pointer; `None` iff `!mem::needs_drop::<R>()` — hot.
    pub drop_fn: Option<ResourceDropFn>,
    /// Cold: type name for diagnostics.
    pub type_name: &'static str,
    /// Cold: `TypeId` for runtime type validation.
    pub type_id: TypeId,
}

impl ResourceInfo {
    /// Creates a new `ResourceInfo` with static information about type `R`.
    ///
    /// The `if needs_drop::<R>()` branch is const-folded per monomorphization.
    #[inline]
    pub fn new_static<R: Resource>() -> Self {
        Self {
            size: std::mem::size_of::<R>(),
            alignment: std::mem::align_of::<R>(),
            drop_fn: if std::mem::needs_drop::<R>() {
                Some(resource_drop_in_place_glue::<R> as ResourceDropFn)
            } else {
                None
            },
            type_name: std::any::type_name::<R>(),
            type_id: TypeId::of::<R>(),
        }
    }
}

/// Static storage for resource metadata. Each slot is independent and
/// initialized at most once via `OnceLock::set`. Read path is a single
/// acquire-load + branch — no `Mutex`, no `static mut`, no data race.
static RESOURCE_INFO: [OnceLock<ResourceInfo>; RESOURCE_SLOT_COUNT] =
    [const { OnceLock::new() }; RESOURCE_SLOT_COUNT];

/// Monotonic counter for resource IDs minted via [`register_new`].
static NEXT_RESOURCE_ID: AtomicUsize = AtomicUsize::new(0);

/// Allocates a fresh `ResourceId` from the global counter and stores
/// `ResourceInfo::new_static::<R>()` in the corresponding `RESOURCE_INFO`
/// slot.
///
/// Production path: called from `#[derive(Resource)]`-generated
/// `R::resource_id()` via a per-monomorphization `OnceLock`. Each concrete
/// `R` gets exactly one ID across the process lifetime, regardless of how
/// many threads call `R::resource_id()` concurrently.
///
/// # M6: Component-vs-Resource exclusivity
///
/// Before minting an ID, the function scans the component registry for the
/// `TypeId` of `R`. If a match is found, the call panics with a diagnostic
/// naming the offending type.
///
/// # W4: best-effort exclusivity in concurrent registration
///
/// The Component-vs-Resource check is **best-effort** under concurrency. It
/// assumes registration is single-threaded (matches the de-facto pattern of
/// `#[derive]`-generated lazy init via `OnceLock`). Concurrent registration
/// of the same `TypeId` as both a Component and a Resource from two threads
/// is not defended against. This is acceptable because:
///   1. The `OnceLock`-based registration model already implicitly assumes
///      single-threaded first-touch (the engine's startup phase).
///   2. A type is statically annotated with EITHER `#[derive(Component)]` OR
///      `#[derive(Resource)]` — never both — so the racy case requires
///      malicious code authoring, not user error.
///   3. Phase 9 may add a `world.is_single_threaded_phase()` guard if a
///      stronger atomicity is ever required.
///
/// # Panics
/// - If the type is already registered as a `Component` (M6).
/// - If `NEXT_RESOURCE_ID` reaches `RESOURCE_SLOT_COUNT`.
/// - If the slot at the minted index is already occupied by a *different*
///   type.
pub fn register_new<R: Resource>() -> usize {
    let type_id = TypeId::of::<R>();
    if component_registry::is_type_registered_as_component(type_id) {
        panic!(
            "type `{}` is already registered as a Component; \
             a type cannot be both Component and Resource. \
             Remove one of #[derive(Component)] / #[derive(Resource)].",
            std::any::type_name::<R>()
        );
    }

    let raw = NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < RESOURCE_SLOT_COUNT,
        "ResourceRegistry exhausted: NEXT_RESOURCE_ID reached {raw}, \
         RESOURCE_SLOT_COUNT = {RESOURCE_SLOT_COUNT}"
    );
    let info = ResourceInfo::new_static::<R>();
    match RESOURCE_INFO[raw].set(info) {
        Ok(()) => raw,
        Err(_) => {
            let existing = RESOURCE_INFO[raw]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id == type_id {
                raw
            } else {
                panic!(
                    "ResourceId {} occupied by type {}, refused to register {}",
                    raw,
                    existing.type_name,
                    std::any::type_name::<R>()
                )
            }
        }
    }
}

/// Retrieves metadata for a resource by its raw `usize` ID.
/// Returns `None` if the resource has not been registered yet.
#[inline]
pub fn get_resource_info(raw_id: usize) -> Option<&'static ResourceInfo> {
    RESOURCE_INFO.get(raw_id)?.get()
}

/// Optimized accessor for resource size — avoids an extra struct copy
/// compared to `get_resource_info(id)?.size`.
#[inline]
pub fn get_resource_size(raw_id: usize) -> Option<usize> {
    Some(get_resource_info(raw_id)?.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry::{
        is_type_registered_as_component, register_layout,
    };

    // Distinct local types per test — `TypeId::of::<T>()` is uniquely
    // determined by the type identity at the module path level, so tests
    // do not collide as long as the types are local to each test scope.

    /// Test resource with a non-trivial drop function (forces the
    /// `needs_drop` branch in `ResourceInfo::new_static`).
    struct TestRes(#[allow(dead_code)] u32);

    impl Drop for TestRes {
        fn drop(&mut self) {}
    }

    impl Resource for TestRes {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            static ID: OnceLock<crate::ecs::identifiers::primitives::ResourceId> =
                OnceLock::new();
            *ID.get_or_init(|| crate::ecs::identifiers::primitives::ResourceId(register_new::<Self>()))
        }
    }

    /// Second test resource — used for the distinctness check.
    struct TestRes2(#[allow(dead_code)] u64);

    impl Resource for TestRes2 {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            static ID: OnceLock<crate::ecs::identifiers::primitives::ResourceId> =
                OnceLock::new();
            *ID.get_or_init(|| crate::ecs::identifiers::primitives::ResourceId(register_new::<Self>()))
        }
    }

    /// Resource used by the direct double-`register_new` test — its
    /// `resource_id()` impl deliberately bypasses the `OnceLock` cache.
    struct DirectRes;

    impl Resource for DirectRes {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            // Bypasses OnceLock — each call hits the registry.
            crate::ecs::identifiers::primitives::ResourceId(register_new::<Self>())
        }
    }

    /// Type that will be registered first as a Component, then attempted
    /// as a Resource — must trigger the M6 exclusivity panic.
    #[allow(dead_code)]
    struct CompThenRes(u8);

    impl Resource for CompThenRes {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            crate::ecs::identifiers::primitives::ResourceId(register_new::<Self>())
        }
    }

    #[test]
    fn register_resource_then_get_returns_info() {
        let id = TestRes::resource_id();
        let info =
            get_resource_info(id.0).expect("info must be present after registration");
        assert_eq!(info.size, std::mem::size_of::<TestRes>(), "size must match");
        assert_eq!(
            info.alignment,
            std::mem::align_of::<TestRes>(),
            "alignment must match"
        );
        assert_eq!(
            info.type_id,
            TypeId::of::<TestRes>(),
            "type_id must match TypeId::of::<TestRes>()"
        );
        assert!(
            info.drop_fn.is_some(),
            "TestRes has a Drop impl — drop_fn must be Some"
        );
    }

    #[test]
    fn register_idempotent_same_type() {
        let first = TestRes::resource_id();
        let second = TestRes::resource_id();
        assert_eq!(
            first, second,
            "OnceLock-wrapped resource_id() must return the same ID on every call"
        );
    }

    #[test]
    fn next_id_distinctness() {
        let id_a = TestRes::resource_id();
        let id_b = TestRes2::resource_id();
        assert_ne!(
            id_a, id_b,
            "register_new must assign different IDs to TestRes and TestRes2 \
             (got id_a={id_a}, id_b={id_b})"
        );
        let info_a =
            get_resource_info(id_a.0).expect("slot for TestRes must be populated");
        let info_b =
            get_resource_info(id_b.0).expect("slot for TestRes2 must be populated");
        assert_eq!(info_a.type_id, TypeId::of::<TestRes>());
        assert_eq!(info_b.type_id, TypeId::of::<TestRes2>());
    }

    /// Direct double-registration via `register_new` — each call uses
    /// `NEXT_RESOURCE_ID::fetch_add`, so the IDs differ. Both slots hold
    /// the same `DirectRes` type. This exercises the `OnceLock::set` Ok
    /// branch twice (different slots), not the collision branch — which
    /// is only reachable when a pre-populated slot is hit by chance.
    ///
    /// Note: the truly-colliding branch (same slot already occupied by the
    /// same type) cannot be triggered deterministically from a unit test
    /// because `NEXT_RESOURCE_ID` is private and not resettable. The same
    /// limitation applies in `component_registry` (see test
    /// `register_new_second_call_for_same_type_occupies_new_slot`).
    #[test]
    fn register_collision_panics() {
        let id1 = DirectRes::resource_id();
        let id2 = DirectRes::resource_id();
        assert_ne!(
            id1, id2,
            "direct register_new calls are not idempotent — each call mints a new slot \
             (idempotency is provided by the macro-generated OnceLock wrapper)"
        );
        let info1 = get_resource_info(id1.0).expect("first slot must be populated");
        let info2 = get_resource_info(id2.0).expect("second slot must be populated");
        assert_eq!(info1.type_id, TypeId::of::<DirectRes>());
        assert_eq!(info2.type_id, TypeId::of::<DirectRes>());
    }

    /// M6 — Component-vs-Resource exclusivity.
    ///
    /// Strategy: pre-register `CompThenRes` as a Component via the
    /// component-registry test escape hatch under a fixed slot, then call
    /// `CompThenRes::resource_id()` — the resource path scans the
    /// component registry, finds the `TypeId` match, and panics.
    #[test]
    #[should_panic(expected = "already registered as a Component")]
    fn register_as_both_component_and_resource_panics() {
        // Use a high slot index unlikely to collide with other tests' direct
        // registrations. Existing reservations: archetype.rs 400-417,
        // archetype_master.rs 300-308, component_pool_bundle.rs 420-429,
        // archetype_bundle::miri_tests 480-482, benches/random_access.rs
        // 490-501. Slot 510 is in the free range and far from cluster edges.
        const COMP_SLOT_FOR_CLASH: usize = 510;
        register_layout::<CompThenRes>(COMP_SLOT_FOR_CLASH);
        // Sanity-precondition: the component registry now reports this
        // type as a component.
        assert!(is_type_registered_as_component(TypeId::of::<CompThenRes>()));
        // This call must panic with "already registered as a Component".
        let _ = CompThenRes::resource_id();
    }
}
