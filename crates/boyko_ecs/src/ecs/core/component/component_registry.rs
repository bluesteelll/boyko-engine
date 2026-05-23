//! Global registry of component layouts.
//!
//! # Component ID assignment
//!
//! Each distinct type `T` implementing [`Component`](crate::ecs::core::component::component::Component)
//! is assigned a unique [`ComponentId`] the first time `T::component_id()` is
//! called in the current process. The assignment is lazy, lock-free on the
//! cached read path, and stable for the lifetime of the process — but **not**
//! stable across processes or across runs of the same binary if the order of
//! first calls differs.
//!
//! # Startup warm-up contract
//!
//! Code that ingests `ComponentId`s from external sources (network, save
//! files, scripts, etc.) MUST warm up the registry by calling
//! `T::component_id()` for every component type `T` it expects to receive,
//! *before* the first external ID arrives. Without warm-up, an incoming
//! id `i` may refer to type `A` in this process but type `B` in a peer
//! process — IDs are assigned in first-call order.
//!
//! Recommended pattern: at engine startup, call `<T as Component>::component_id()`
//! for every component type that will be serialized, in a deterministic order.
//!
//! # Collision detection
//!
//! Every `set` call site ([`register_new`] and [`register_layout`]) checks
//! the slot before declaring success. If the slot is already occupied by a
//! *different* type than the one being registered, the call panics in both
//! debug and release builds, naming both types. This catches accidental
//! ID-space overlaps between the production counter and the test escape
//! hatch immediately.
//!
//! # Threading
//!
//! All registry operations are safe to call from any thread. The global
//! `NEXT_ID` counter uses `Relaxed` ordering (uniqueness is sufficient;
//! cross-thread happens-before is provided by `OnceLock::set` / `get`).
//! Per-slot `OnceLock`s provide acquire/release synchronization of the
//! `ComponentLayout` payload.

use std::alloc::Layout;
use std::any::TypeId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of components supported by the ECS system.
pub const MAX_COMPONENTS: usize = 512;

/// Holds layout information for a specific component type.
///
/// Filled in by [`register_new`] or [`register_layout`]. Each entry is written
/// exactly once via `OnceLock::set` and read lock-free via `OnceLock::get`.
/// Fixes audit findings M-002 / C-002 / Q-010.
#[derive(Clone, Copy, Debug)]
pub struct ComponentLayout {
    /// Size of the component in bytes.
    pub size: usize,
    /// Alignment requirement for the component.
    pub alignment: usize,
    /// The component's type name (for debugging and collision messages).
    pub type_name: &'static str,
    /// Unique type identifier.
    pub type_id: TypeId,
}

impl ComponentLayout {
    /// Creates a new `ComponentLayout` with static information about type `T`.
    #[inline]
    pub fn new_static<T: 'static>() -> Self {
        Self {
            size: std::mem::size_of::<T>(),
            alignment: std::mem::align_of::<T>(),
            type_name: std::any::type_name::<T>(),
            type_id: TypeId::of::<T>(),
        }
    }

    /// Returns a memory layout object for this component.
    #[inline]
    pub fn layout(&self) -> Layout {
        // SAFETY: size/alignment originated from `size_of::<T>()` /
        // `align_of::<T>()` for some `T: 'static`. Those are valid by the
        // language definition — alignment is a power of two and size fits in
        // `isize::MAX` (otherwise `T` would not have a layout).
        unsafe { Layout::from_size_align_unchecked(self.size, self.alignment) }
    }
}

/// Static storage for component layouts. Each slot is independent and
/// initialized at most once via `OnceLock::set`. Read path is a single
/// acquire-load + branch — no `Mutex`, no `static mut`, no data race.
static LAYOUTS: [OnceLock<ComponentLayout>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Monotonic counter for component IDs minted via [`register_new`].
/// Test code that needs explicit IDs uses [`register_layout`] and bypasses
/// this counter — collisions between the two paths are detected per-slot.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Allocates a fresh `ComponentId` from the global counter and stores
/// `ComponentLayout::new_static::<T>()` in the corresponding `LAYOUTS` slot.
///
/// Production path: called from `#[derive(Component)]`-generated
/// `T::component_id()` via a per-monomorphization `OnceLock`. Each concrete
/// `T` gets exactly one ID across the process lifetime, regardless of how
/// many threads call `T::component_id()` concurrently.
///
/// # Panics
/// - If `NEXT_ID` reaches `MAX_COMPONENTS`.
/// - If the slot at the minted index is already occupied by a *different*
///   type — see module-level "Collision detection" docs.
pub fn register_new<T: 'static>() -> usize {
    let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < MAX_COMPONENTS,
        "ComponentRegistry exhausted: NEXT_ID reached {}, MAX_COMPONENTS = {}",
        raw,
        MAX_COMPONENTS
    );
    let layout = ComponentLayout::new_static::<T>();
    match LAYOUTS[raw].set(layout) {
        Ok(()) => raw,
        Err(_) => {
            let existing = LAYOUTS[raw]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id == TypeId::of::<T>() {
                raw
            } else {
                panic!(
                    "ComponentId {} occupied by type {}, refused to register {}",
                    raw,
                    existing.type_name,
                    std::any::type_name::<T>()
                )
            }
        }
    }
}

/// Test-only escape hatch: registers `T` under an explicit `component_id`.
///
/// Production code must not call this — use `T::component_id()` (which goes
/// through [`register_new`]). Tests use this to install components under
/// known, fixed IDs without depending on `NEXT_ID`'s value.
///
/// # Panics
/// - If `component_id >= MAX_COMPONENTS`.
/// - If the slot is already occupied by a *different* type. Same-type
///   re-registration is silently idempotent.
#[doc(hidden)]
pub fn register_layout<T: 'static>(component_id: usize) {
    assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let layout = ComponentLayout::new_static::<T>();
    match LAYOUTS[component_id].set(layout) {
        Ok(()) => {}
        Err(_) => {
            let existing = LAYOUTS[component_id]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id != TypeId::of::<T>() {
                panic!(
                    "ComponentId {} occupied by type {}, refused to register {}",
                    component_id,
                    existing.type_name,
                    std::any::type_name::<T>()
                )
            }
            // Same type — silent no-op (idempotent).
        }
    }
}

/// Retrieves layout information for a component by its ID.
/// Returns `None` if the component hasn't been registered yet.
#[inline]
pub fn get_layout(component_id: usize) -> Option<&'static ComponentLayout> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );

    if component_id >= MAX_COMPONENTS {
        return None;
    }

    LAYOUTS[component_id].get()
}

/// Optimized function to get the size of a component by ID.
#[inline]
pub fn get_component_size(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.size)
}

/// Optimized function to get the alignment of a component by ID.
#[inline]
pub fn get_component_alignment(component_id: usize) -> Option<usize> {
    Some(get_layout(component_id)?.alignment)
}

/// Creates a memory layout for a component without going through `ComponentLayout`.
#[inline]
pub fn get_component_memory_layout(component_id: usize) -> Option<Layout> {
    Some(get_layout(component_id)?.layout())
}

/// Ultra-fast access to component size when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_component_size_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().size }
}

/// Ultra-fast access to component alignment when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_component_alignment_unchecked(component_id: usize) -> usize {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked().alignment }
}

/// Ultra-fast access to component layout when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_layout_unchecked(component_id: usize) -> &'static ComponentLayout {
    debug_assert!(
        component_id < MAX_COMPONENTS && LAYOUTS[component_id].get().is_some(),
        "Component ID {} is invalid or not initialized",
        component_id
    );
    // SAFETY: per the function contract, the slot is initialized and
    // `component_id < MAX_COMPONENTS`.
    unsafe { LAYOUTS[component_id].get().unwrap_unchecked() }
}

/// Ultra-fast access to component memory layout when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_component_memory_layout_unchecked(component_id: usize) -> Layout {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    let layout = unsafe { get_layout_unchecked(component_id) };
    // SAFETY: size/alignment come from a registered `ComponentLayout`, valid by construction.
    unsafe { Layout::from_size_align_unchecked(layout.size, layout.alignment) }
}

/// Ultra-fast access to component type ID when you're confident the component exists.
///
/// # Safety
/// Caller guarantees that `component_id < MAX_COMPONENTS` and that one of
/// the following has already completed for the corresponding type `T`:
/// - [`register_new::<T>()`] (production path, via `T::component_id()`), or
/// - [`register_layout::<T>(component_id)`] (test-only escape hatch).
/// Violating either yields UB.
#[inline(always)]
pub unsafe fn get_component_type_id_unchecked(component_id: usize) -> TypeId {
    // SAFETY: forwarded to the unchecked accessor; caller satisfies the same contract.
    unsafe { get_layout_unchecked(component_id).type_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use large IDs in this module to avoid colliding with other test modules
    // that register components under low IDs (0-10). OnceLock slots are global
    // and persist for the lifetime of the test binary.
    const TEST_BASE: usize = 450;

    // --- register_layout + get_layout ---

    #[test]
    fn register_layout_then_get_returns_matching_fields() {
        let id = TEST_BASE;
        register_layout::<u32>(id);
        let layout = get_layout(id).expect("layout must be present after register");
        assert_eq!(layout.size, std::mem::size_of::<u32>(), "size must match u32");
        assert_eq!(
            layout.alignment,
            std::mem::align_of::<u32>(),
            "alignment must match u32"
        );
        assert_eq!(
            layout.type_id,
            TypeId::of::<u32>(),
            "type_id must match TypeId::of::<u32>()"
        );
    }

    #[test]
    fn get_layout_unregistered_returns_none() {
        // ID 499 is unused by any other test in this crate.
        assert!(
            get_layout(499).is_none(),
            "unregistered component must return None"
        );
    }

    #[test]
    fn register_layout_idempotent_same_type() {
        // Registering the same type twice must keep the first registration.
        let id = TEST_BASE + 1;
        register_layout::<u64>(id);
        register_layout::<u64>(id); // second call — must be silent no-op
        let layout = get_layout(id).expect("slot must remain populated");
        assert_eq!(layout.size, std::mem::size_of::<u64>());
    }

    // ----- NEW TESTS: Phase 1b C-003 / M-015 -----

    // Slot allocation map for new tests (must not collide with existing tests):
    //   TEST_BASE+0 = 450 → u32 (register_layout_then_get_returns_matching_fields)
    //   TEST_BASE+1 = 451 → u64 (register_layout_idempotent_same_type)
    //   TEST_BASE+2 = 452 → (unused by old tests; reserved for collision tests below)
    //   TEST_BASE+3 = 453 → f32
    //   TEST_BASE+4 = 454 → f64
    //   TEST_BASE+5 = 455 → f32
    //   TEST_BASE+6 = 456 → u128
    //   TEST_BASE+7..+11 = 457..461 → new tests below
    //   499, 498 → already claimed by get_layout / get_component_size unregistered tests

    // Distinct local struct types for collision tests — defined at module scope so
    // that `TypeId::of::<T>()` is unambiguous across parallel test threads.
    #[repr(C)] struct ColTypeA(u32);
    #[repr(C)] struct ColTypeB(u64);
    // A third type to test register_new distinctness independently.
    #[repr(C)] struct RegNewTypeA(u8);
    #[repr(C)] struct RegNewTypeB(u16);

    /// register_new<TypeA> and register_new<TypeB> must return different IDs.
    ///
    /// This test calls register_new directly (bypassing the macro-generated OnceLock).
    /// Since register_new uses fetch_add, each call is guaranteed a unique slot.
    #[test]
    fn register_new_assigns_distinct_ids_for_distinct_types() {
        let id_a = register_new::<RegNewTypeA>();
        let id_b = register_new::<RegNewTypeB>();
        assert_ne!(
            id_a,
            id_b,
            "register_new must assign different IDs to different types (got id_a={id_a}, id_b={id_b})"
        );
        // Verify both slots are populated with the correct type.
        assert_eq!(
            get_layout(id_a).expect("slot for RegNewTypeA must be populated").type_id,
            TypeId::of::<RegNewTypeA>(),
            "layout at id_a must carry RegNewTypeA type_id"
        );
        assert_eq!(
            get_layout(id_b).expect("slot for RegNewTypeB must be populated").type_id,
            TypeId::of::<RegNewTypeB>(),
            "layout at id_b must carry RegNewTypeB type_id"
        );
    }

    /// register_new<T> collision-idempotent branch: if the slot that NEXT_ID picks
    /// is already occupied by the same type (e.g., pre-populated via register_layout
    /// or by a concurrent thread), the call returns the existing ID without panic.
    ///
    /// We manufacture this by:
    ///   1. Pre-registering ColTypeA under a known slot (TEST_BASE+7) via register_layout.
    ///   2. Loading that slot from LAYOUTS directly via get_layout — verifying it is set.
    ///   3. Calling register_new::<ColTypeA>() again (a second monomorphization call).
    ///      Because NEXT_ID has already advanced past TEST_BASE+7 in the process,
    ///      the second call gets a fresh slot — it does NOT hit the collision branch.
    ///
    /// LIMITATION: The collision-idempotent branch in register_new (the Err arm of
    /// OnceLock::set where existing.type_id == TypeId::of::<T>()) can only be triggered
    /// if NEXT_ID happens to pick a slot that was pre-populated for the same type via
    /// register_layout. Since NEXT_ID is private and not resettable from tests, we
    /// cannot manufacture this scenario deterministically. The branch is exercised
    /// indirectly through the integration test `derive_component_emits_lazy_id` (concurrent
    /// first-call scenario). See also: test_next_id_exhaustion_not_testable note below.
    ///
    /// What we CAN test here: register_new<T> followed by register_new<T> registers T
    /// under TWO different slots (since fetch_add is monotone), both returning Some(T).
    #[test]
    fn register_new_second_call_for_same_type_occupies_new_slot() {
        // Both calls return distinct IDs because fetch_add always advances.
        let id1 = register_new::<ColTypeA>();
        let id2 = register_new::<ColTypeA>();
        // The IDs differ — fetch_add is strictly monotone.
        assert_ne!(
            id1,
            id2,
            "direct register_new calls are not idempotent — each call mints a new slot \
             (idempotency is provided by the macro-generated OnceLock wrapper)"
        );
        // Both slots hold ColTypeA.
        assert_eq!(
            get_layout(id1).expect("first slot must be populated").type_id,
            TypeId::of::<ColTypeA>(),
            "first slot must hold ColTypeA"
        );
        assert_eq!(
            get_layout(id2).expect("second slot must be populated").type_id,
            TypeId::of::<ColTypeA>(),
            "second slot must also hold ColTypeA (two separate registrations)"
        );
    }

    /// Collision detection in register_layout: registering a different type in an
    /// already-occupied slot must panic with a message naming both types.
    ///
    /// Slot 462 is reserved for this test. The panic expected substring matches the
    /// format string: "ComponentId {} occupied by type {}, refused to register {}".
    #[test]
    #[should_panic(expected = "occupied by type")]
    fn register_layout_collision_with_different_type_panics() {
        const COLLISION_SLOT: usize = 462;
        // First registration occupies the slot.
        register_layout::<ColTypeA>(COLLISION_SLOT);
        // Second registration with a different type must panic.
        register_layout::<ColTypeB>(COLLISION_SLOT);
    }

    /// Collision detection idempotent path: registering the SAME type twice under the
    /// same explicit slot must be a silent no-op (no panic, slot remains valid).
    ///
    /// Slot 465 is reserved for this test.
    #[test]
    fn register_layout_collision_with_same_type_is_silent_noop() {
        const IDEMPOTENT_SLOT: usize = 465;
        register_layout::<ColTypeA>(IDEMPOTENT_SLOT);
        register_layout::<ColTypeA>(IDEMPOTENT_SLOT); // second call — must be silent
        let layout = get_layout(IDEMPOTENT_SLOT)
            .expect("slot must remain populated after idempotent re-registration");
        assert_eq!(
            layout.type_id,
            TypeId::of::<ColTypeA>(),
            "slot type_id must remain ColTypeA after silent no-op"
        );
        assert_eq!(
            layout.size,
            std::mem::size_of::<ColTypeA>(),
            "slot size must remain correct after silent no-op"
        );
    }

    /// register_layout panics when component_id == MAX_COMPONENTS (out-of-range by one).
    ///
    /// The assert! in register_layout fires in both debug and release. Expected message
    /// matches: "Component ID {} exceeds maximum allowed ({})".
    ///
    /// Note: the developer left a test `register_layout_at_max_components_boundary_panics`
    /// that already covers this via catch_unwind. This companion test uses #[should_panic]
    /// with a tighter expected substring to lock in the panic message format.
    #[test]
    #[should_panic(expected = "exceeds maximum allowed")]
    fn register_layout_at_max_components_panics_with_expected_message() {
        register_layout::<u8>(MAX_COMPONENTS);
    }

    // NOTE: register_new exhaustion test (driving NEXT_ID to MAX_COMPONENTS) is NOT
    // included because NEXT_ID is a private AtomicUsize with no test-only accessor.
    // Options considered:
    //   (a) Expose via #[cfg(test)] pub(crate) fn reset_next_id_for_tests(value: usize) —
    //       requires developer to add the accessor; deferred.
    //   (b) Loop register_new N+1 times with distinct types — pollutes the global counter
    //       irreversibly for subsequent tests in the same process; not acceptable.
    // TODO: developer to add #[cfg(test)] fn set_next_id_for_test(v: usize) so tester
    // can write register_new_exhaustion_panics.

    // ----- END NEW TESTS -----

    #[test]
    fn register_layout_at_max_components_boundary_panics() {
        // MAX_COMPONENTS is out-of-range (valid indices: 0..MAX_COMPONENTS-1).
        // register_layout now asserts (not debug_assert) — always panics OOB.
        let result = std::panic::catch_unwind(|| {
            register_layout::<u32>(MAX_COMPONENTS);
        });
        assert!(result.is_err(), "out-of-range register_layout must panic");
    }

    #[test]
    fn get_layout_at_index_zero_is_none_before_register() {
        // Index 0 is a valid index but we never register a component there
        // in this test module. It may have been registered by another module —
        // we can only assert that get_layout does not panic on index 0.
        let _ = get_layout(0); // must not panic regardless of value
    }

    // --- get_component_size / get_component_alignment ---

    #[test]
    fn get_component_size_matches_registered_layout() {
        let id = TEST_BASE + 3;
        register_layout::<f32>(id);
        assert_eq!(
            get_component_size(id),
            Some(std::mem::size_of::<f32>()),
            "get_component_size must agree with size_of::<f32>()"
        );
    }

    #[test]
    fn get_component_alignment_matches_registered_layout() {
        let id = TEST_BASE + 4;
        register_layout::<f64>(id);
        assert_eq!(
            get_component_alignment(id),
            Some(std::mem::align_of::<f64>()),
            "get_component_alignment must agree with align_of::<f64>()"
        );
    }

    #[test]
    fn get_component_size_unregistered_returns_none() {
        assert!(
            get_component_size(498).is_none(),
            "unregistered component must return None from get_component_size"
        );
    }

    // --- get_layout_unchecked (unsafe hot path) ---

    #[test]
    fn get_layout_unchecked_after_register_returns_correct_size() {
        let id = TEST_BASE + 5;
        register_layout::<f32>(id);
        // SAFETY: `id` is < MAX_COMPONENTS and `register_layout` was just called.
        let layout = unsafe { get_layout_unchecked(id) };
        assert_eq!(
            layout.size,
            std::mem::size_of::<f32>(),
            "unchecked accessor must return the registered layout"
        );
    }

    // --- get_component_memory_layout ---

    #[test]
    fn get_component_memory_layout_produces_valid_layout() {
        let id = TEST_BASE + 6;
        register_layout::<u128>(id);
        let mem_layout =
            get_component_memory_layout(id).expect("must return Some after register");
        assert_eq!(mem_layout.size(), std::mem::size_of::<u128>());
        assert_eq!(mem_layout.align(), std::mem::align_of::<u128>());
    }
}
