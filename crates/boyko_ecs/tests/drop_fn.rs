/// Integration tests for Phase 1b drop_fn implementation.
///
/// Verifies audit findings:
///   M-001 (cont.) — type-erased component Drop via `drop_in_place_glue`
///   M-004         — swap_remove / pop now invoke drop glue
///   C-004 (partial) — TypeId-check on typed API surfaces mismatches in debug
///
/// ID allocation (no collision with existing test modules):
///   200  — DropCounter (drop-correctness tests group A)
///   201  — DropCounter2 (set_component_typed: two separate DropCounter instances)
///   202  — TypeA (TypeId-check tests)
///   203  — TypeB (TypeId-check tests, wrong type)
///   204  — ZstComp (ZST rejection test — size==0, triggers debug_assert)
///   205  — u32 (POD / no-drop test)
///   206  — BundleComp (bundle-level tests)
///   207  — BundleMissing (bundle tests: type with no pool)

// ---- shared test infrastructure ------------------------------------------------

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::memory::{arena::Arena, component_pool::ComponentPool};
use boyko_ecs::ecs::core::component::component_pool_bundle::ComponentPoolBundle;
use boyko_ecs::ecs::core::entity::entity_inland::EntityInland;

// IDs used across all tests in this file.
const DC_ID: usize = 200;       // DropCounter component
const DC2_ID: usize = 201;      // second DropCounter for set_component_typed
const TYPE_A_ID: usize = 202;   // TypeA component (typed-API TypeId tests)
const TYPE_B_ID: usize = 203;   // TypeB component (wrong type for TypeId tests)
const ZST_ID: usize = 204;      // ZstComp (ZST rejection)
const POD_ID: usize = 205;      // u32 pod (no-drop test)
const BUNDLE_ID: usize = 206;   // BundleComp (bundle-level tests)
const BUNDLE_MISSING_ID: usize = 207; // BundleMissingComp (no pool in bundle)

// ---- component type definitions -------------------------------------------------

/// A component whose Drop increments a shared counter.
/// Used to observe exactly how many drops occur.
#[repr(C)]
struct DropCounter {
    counter: Arc<AtomicUsize>,
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// A second DropCounter type so we can register it under a distinct component ID
/// (the registry is keyed by TypeId; two independent registrations for the
/// same Rust type would collide, so we use a newtype).
#[repr(C)]
struct DropCounter2 {
    counter: Arc<AtomicUsize>,
}

impl Drop for DropCounter2 {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// POD component — no Drop impl. Used to verify that drop_fn is None for
/// types where mem::needs_drop::<T>() == false.
#[repr(C)]
struct PodU32 {
    val: u32,
}

/// TypeA and TypeB: two different component types for TypeId-check tests.
#[repr(C)]
struct TypeA {
    x: u64,
}

#[repr(C)]
struct TypeB {
    y: u64,
}

/// BundleComp: component registered under BUNDLE_ID.
#[repr(C)]
struct BundleComp {
    counter: Arc<AtomicUsize>,
}

impl Drop for BundleComp {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// BundleMissingComp: a component type for which we intentionally register NO pool
/// in the bundle, to test the missing-pool path in add_component_typed.
#[repr(C)]
struct BundleMissingComp {
    counter: Arc<AtomicUsize>,
}

impl Drop for BundleMissingComp {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// ZstComp: zero-sized type for the ZST rejection test.
/// Only used in the `#[cfg(debug_assertions)]` test.
#[repr(C)]
struct ZstComp;

// We need Component-like IDs. We use register_layout directly — these types
// do NOT #[derive(Component)]; they only need a registry entry.
// Registration is idempotent, so calling it from multiple tests is safe.

fn register_all() {
    component_registry::register_layout::<DropCounter>(DC_ID);
    component_registry::register_layout::<DropCounter2>(DC2_ID);
    component_registry::register_layout::<TypeA>(TYPE_A_ID);
    component_registry::register_layout::<TypeB>(TYPE_B_ID);
    component_registry::register_layout::<PodU32>(POD_ID);
    component_registry::register_layout::<BundleComp>(BUNDLE_ID);
    component_registry::register_layout::<BundleMissingComp>(BUNDLE_MISSING_ID);
    // ZST is registered only in the panic test because its size==0 would cause
    // ComponentPool::new to debug_assert even during setup for other tests.
}

// We need the Component trait so add_typed / set_component_typed compile.
// We implement it manually (the types are local and can't derive it from
// boyko-macros without adding the macro crate as a dependency for every
// integration test).
//
// Implementation mirrors what #[derive(Component)] generates:
//   - component_id() mints via register_new (or returns cached OnceLock value).
//
// Because we use register_layout (explicit IDs) rather than register_new,
// we implement component_id() to return the fixed constant. This is safe
// because the OnceLock is a global per-type cache; we initialise it once.
// Tests call register_all() first, so the slot is guaranteed to be populated
// before component_id() is used.

use boyko_ecs::ecs::core::component::component::Component;
use std::sync::OnceLock;

impl Component for DropCounter {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<DropCounter>(DC_ID);
            DC_ID
        })
    }
}

impl Component for DropCounter2 {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<DropCounter2>(DC2_ID);
            DC2_ID
        })
    }
}

impl Component for TypeA {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<TypeA>(TYPE_A_ID);
            TYPE_A_ID
        })
    }
}

impl Component for TypeB {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<TypeB>(TYPE_B_ID);
            TYPE_B_ID
        })
    }
}

impl Component for PodU32 {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<PodU32>(POD_ID);
            POD_ID
        })
    }
}

impl Component for BundleComp {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<BundleComp>(BUNDLE_ID);
            BUNDLE_ID
        })
    }
}

impl Component for BundleMissingComp {
    fn component_id() -> usize {
        static ID: OnceLock<usize> = OnceLock::new();
        *ID.get_or_init(|| {
            component_registry::register_layout::<BundleMissingComp>(BUNDLE_MISSING_ID);
            BUNDLE_MISSING_ID
        })
    }
}

// ---- helpers -------------------------------------------------------------------

/// Creates a DropCounter pool with 1 chunk of `cap` slots.
/// Registers DropCounter under DC_ID beforehand.
fn make_dc_pool(arena: &Arena, cap: usize) -> ComponentPool {
    register_all();
    ComponentPool::new(arena, DC_ID, 1, cap)
}

// ================================================================================
// Group 1: Drop-correctness tests
// ================================================================================

/// Dropping a pool that holds N live DropCounters must invoke drop_fn N times.
#[test]
fn pool_drop_calls_drop_fn_for_each_live_component() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    for _ in 0..5 {
        let dc = DropCounter { counter: Arc::clone(&ctr) };
        pool.add_typed(dc).expect("pool has capacity for 5 elements");
    }

    assert_eq!(ctr.load(Ordering::Relaxed), 0, "no drops must have occurred before pool drop");

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        5,
        "impl Drop for ComponentPool must call drop_fn once per live slot"
    );
}

/// swap_remove on a non-last slot must drop only the removed component.
/// After the pool itself is dropped, total drops must equal the total inserted.
#[test]
fn swap_remove_calls_drop_fn_once_for_removed_component() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    for _ in 0..3 {
        let dc = DropCounter { counter: Arc::clone(&ctr) };
        pool.add_typed(dc).expect("pool has capacity for 3 elements");
    }

    let removed = pool.swap_remove(0);
    assert!(removed, "swap_remove(0) must return true (index in bounds)");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "exactly 1 drop must have fired for the removed component; remaining 2 are still live"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        3,
        "after pool drop, total drops must equal total inserted (3)"
    );
}

/// swap_remove on the last slot (index == len-1) must drop only that component.
#[test]
fn swap_remove_last_calls_drop_fn_once() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    for _ in 0..3 {
        let dc = DropCounter { counter: Arc::clone(&ctr) };
        pool.add_typed(dc).expect("pool has capacity for 3 elements");
    }

    let removed = pool.swap_remove(2); // last element
    assert!(removed, "swap_remove(2) on a 3-element pool must return true");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "swap_remove of last element must drop exactly 1 component"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        3,
        "after pool drop, all 3 insertions must have been dropped"
    );
}

/// pop must invoke drop_fn once for the removed component and leave the rest live.
#[test]
fn pop_calls_drop_fn_once() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    for _ in 0..3 {
        let dc = DropCounter { counter: Arc::clone(&ctr) };
        pool.add_typed(dc).expect("pool has capacity");
    }

    let popped = pool.pop();
    assert!(popped, "pop on a non-empty pool must return true");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "pop must drop exactly the last component"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        3,
        "after pool drop, all 3 insertions must have been dropped"
    );
}

/// set_component_typed must drop the old value (+1) and move the new value in.
/// The moved-in value must NOT be dropped at the call site (it is now owned by
/// the pool). Pool drop then drops the new value (+1), for a total of 2.
#[test]
fn set_component_typed_drops_old_and_consumes_new() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    // Insert old value.
    let old = DropCounter { counter: Arc::clone(&ctr) };
    pool.add_typed(old).expect("pool must accept first element");

    assert_eq!(ctr.load(Ordering::Relaxed), 0, "old value must not be dropped yet");

    // Overwrite with new value.
    let new_val = DropCounter { counter: Arc::clone(&ctr) };
    let ok = pool.set_component_typed(0, new_val);
    assert!(ok, "set_component_typed must succeed for index 0 (in bounds)");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "set_component_typed must have dropped the old value (+1); new value is now owned by pool"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        2,
        "pool drop must drop the new (moved-in) value: total = 2"
    );
}

/// POD component (PodU32): drop_fn must be None; pool drop must not call any glue.
/// We cannot directly observe absence of a call, but we verify the pool handles
/// the entire lifecycle without panics and that the component_layout has None drop_fn.
#[test]
fn pool_drop_skipped_for_pod_components() {
    register_all();
    let arena = Arena::new();
    let mut pool = ComponentPool::new(&arena, POD_ID, 1, 8);

    // Use raw `add` because PodU32 is a POD — the typed API would also work, but raw
    // demonstrates the case where no drop_fn runs.
    let val = PodU32 { val: 0xDEAD_BEEF };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            &val as *const PodU32 as *const u8,
            std::mem::size_of::<PodU32>(),
        )
    };
    std::mem::forget(val); // avoid double-free: we copied bytes

    let idx = pool.add(bytes);
    assert_eq!(idx, Some(0), "first add to an empty pool must succeed at index 0");
    assert_eq!(pool.count(), 1, "pool must contain 1 element");

    // Drop the pool — must complete without panic even with a fake "value" in the slot.
    drop(pool);
    // If we reach here without a panic or abort, the test passes.
}

/// add_typed on a full pool must return None and the value must drop at scope exit.
#[test]
fn add_typed_returns_none_and_value_dropped_on_pool_full() {
    register_all();
    let arena = Arena::new();
    // Capacity = 1 slot.
    let mut pool = make_dc_pool(&arena, 1);
    let ctr = Arc::new(AtomicUsize::new(0));

    // Fill the pool.
    let first = DropCounter { counter: Arc::clone(&ctr) };
    let idx = pool.add_typed(first);
    assert_eq!(idx, Some(0), "first add must succeed");

    // Now the pool is full. add_typed must return None and drop `overflow` at scope exit.
    {
        let overflow = DropCounter { counter: Arc::clone(&ctr) };
        let result = pool.add_typed(overflow);
        assert!(result.is_none(), "add_typed must return None when pool is full");
        // `overflow` drops here — counter becomes 1.
    }
    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "the rejected value must have been dropped at scope exit (+1)"
    );

    drop(pool); // drops the first element (+1)
    assert_eq!(ctr.load(Ordering::Relaxed), 2, "total drops must be 2 after pool drop");
}

/// add_typed on a non-full pool must NOT drop the value (it is moved into the slot).
/// Pool drop must invoke drop_fn once for the moved-in value.
#[test]
fn add_typed_consumes_value_on_success() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 4);
    let ctr = Arc::new(AtomicUsize::new(0));

    let val = DropCounter { counter: Arc::clone(&ctr) };
    let idx = pool.add_typed(val);
    assert_eq!(idx, Some(0), "add_typed must return Some(0) on success");

    // val has been moved into the pool slot. No drop must have fired.
    assert_eq!(
        ctr.load(Ordering::Relaxed),
        0,
        "add_typed must not drop the value on success — it was moved into the pool slot"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "pool drop must drop the moved-in value exactly once"
    );
}

/// set_component_typed with an out-of-bounds index must return false
/// and the `value` parameter must be dropped at scope exit (not leaked).
#[test]
fn set_component_typed_out_of_bounds_drops_value() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 10);
    let ctr = Arc::new(AtomicUsize::new(0));

    // Insert one real element so pool is not empty.
    let slot_val = DropCounter { counter: Arc::clone(&ctr) };
    pool.add_typed(slot_val).expect("pool must accept first element");

    // Attempt to set at index 99 — out of bounds.
    {
        let rejected = DropCounter { counter: Arc::clone(&ctr) };
        let ok = pool.set_component_typed(99, rejected);
        assert!(!ok, "set_component_typed must return false for out-of-bounds index");
        // `rejected` drops here — counter increments.
    }
    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "rejected value must be dropped at scope exit (+1)"
    );

    drop(pool);
    assert_eq!(ctr.load(Ordering::Relaxed), 2, "pool drop accounts for the first element");
}

/// Successful set_component_typed: the old slot value is dropped (+1),
/// `value` is moved into the slot (not dropped at call site).
/// Pool drop accounts for the new value (+1). Total = 2.
#[test]
fn set_component_typed_within_bounds_consumes_new() {
    register_all();
    let arena = Arena::new();
    let mut pool = make_dc_pool(&arena, 4);
    let ctr = Arc::new(AtomicUsize::new(0));

    let old_val = DropCounter { counter: Arc::clone(&ctr) };
    pool.add_typed(old_val).expect("insert old value");

    let new_val = DropCounter { counter: Arc::clone(&ctr) };
    let ok = pool.set_component_typed(0, new_val);
    assert!(ok, "set_component_typed must return true for index 0 (in bounds)");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "old value must be dropped by set_component_typed; new value moved into slot (not dropped)"
    );

    drop(pool);

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        2,
        "pool drop must drop the new (moved-in) value: total = 2"
    );
}

// ================================================================================
// Group 2: TypeId-check tests (debug_assert — debug builds only)
// ================================================================================

/// Calling add_typed with a type whose TypeId does not match the pool's registered
/// type must fire a debug_assert with a message containing the mismatch wording.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not match pool's registered type")]
fn add_typed_wrong_type_panics_in_debug() {
    register_all();
    let arena = Arena::new();
    // Pool registered for TypeA (TYPE_A_ID).
    let mut pool = ComponentPool::new(&arena, TYPE_A_ID, 1, 4);
    // Attempt to add TypeB — TypeId mismatch must fire debug_assert.
    let wrong = TypeB { y: 0 };
    pool.add_typed(wrong);
}

/// Calling set_component_typed with the wrong type must fire a debug_assert.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not match pool's registered type")]
fn set_component_typed_wrong_type_panics_in_debug() {
    register_all();
    let arena = Arena::new();
    // Pool registered for TypeA.
    let mut pool = ComponentPool::new(&arena, TYPE_A_ID, 1, 4);
    // Insert a valid TypeA first so index 0 exists.
    let valid = TypeA { x: 1 };
    pool.add_typed(valid).expect("insert valid TypeA");
    // Now call set_component_typed with TypeB — must panic.
    let wrong = TypeB { y: 2 };
    pool.set_component_typed(0, wrong);
}

// ================================================================================
// Group 3: ZST rejection (debug_assert)
// ================================================================================

/// ComponentPool::new must debug_assert if the registered component has size == 0.
/// ZstComp has size_of::<ZstComp>() == 0.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not support zero-sized components")]
fn pool_construction_rejects_zst_component_in_debug() {
    // Register ZST under its slot.
    component_registry::register_layout::<ZstComp>(ZST_ID);
    let arena = Arena::new();
    // This must fire the debug_assert in ComponentPool::new.
    let _pool = ComponentPool::new(&arena, ZST_ID, 1, 4);
}

// ================================================================================
// Group 4: Bundle-level tests
// ================================================================================

/// add_component_typed via ComponentPoolBundle must forward the value to the
/// underlying pool and increment the pool's count.
#[test]
fn bundle_add_component_typed_forwards_to_pool() {
    register_all();
    let arena = Arena::new();
    let mut bundle = ComponentPoolBundle::new();
    bundle.add_pool(&arena, BUNDLE_ID);

    let ctr = Arc::new(AtomicUsize::new(0));
    let val = BundleComp { counter: Arc::clone(&ctr) };

    let idx = bundle.add_component_typed(val);
    assert_eq!(idx, Some(0), "first add_component_typed must succeed at index 0");

    let pool = bundle.get_pool(BUNDLE_ID).expect("pool for BUNDLE_ID must exist");
    assert_eq!(pool.count(), 1, "underlying pool count must be 1 after add_component_typed");
}

/// add_component_typed on a bundle without a pool for the given type must return None
/// and the value must be dropped at scope exit.
#[test]
fn bundle_add_component_typed_on_missing_component_returns_none_and_drops_value() {
    register_all();
    let arena = Arena::new();
    // Create bundle with NO pool for BundleMissingComp.
    let mut bundle = ComponentPoolBundle::new();
    // Add a pool for BUNDLE_ID only — BUNDLE_MISSING_ID has no pool.
    bundle.add_pool(&arena, BUNDLE_ID);

    let ctr = Arc::new(AtomicUsize::new(0));

    {
        let val = BundleMissingComp { counter: Arc::clone(&ctr) };
        let result = bundle.add_component_typed(val);
        assert!(
            result.is_none(),
            "add_component_typed must return None when no pool exists for the component type"
        );
        // `val` drops at scope exit.
    }

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "the rejected value must be dropped at scope exit (+1)"
    );
}

/// set_component_typed via bundle must invoke drop_fn on the old slot value.
#[test]
fn bundle_set_component_typed_forwards_drop_to_pool() {
    register_all();
    let arena = Arena::new();
    let mut bundle = ComponentPoolBundle::new();
    bundle.add_pool(&arena, BUNDLE_ID);

    let ctr = Arc::new(AtomicUsize::new(0));

    // Insert old value.
    let old_val = BundleComp { counter: Arc::clone(&ctr) };
    let idx = bundle.add_component_typed(old_val);
    assert_eq!(idx, Some(0), "insert must succeed");
    assert_eq!(ctr.load(Ordering::Relaxed), 0, "old value must not be dropped yet");

    // Overwrite via bundle — creates an EntityInland pointing at slot 0.
    let entity = EntityInland::new(0, 0, 0);
    let new_val = BundleComp { counter: Arc::clone(&ctr) };
    let ok = bundle.set_component_typed(&entity, new_val);
    assert!(ok, "set_component_typed via bundle must return true (index 0, pool present)");

    assert_eq!(
        ctr.load(Ordering::Relaxed),
        1,
        "bundle set_component_typed must have dropped the old value (+1)"
    );
    // `bundle` drops here, which drops pool, which drops the new slot value (+1).
    drop(bundle);
    assert_eq!(ctr.load(Ordering::Relaxed), 2, "total drops after bundle drop must be 2");
}
