//! Process-global `BundleTypeId` minting (Phase 8.5 Step 0).
//!
//! See `docs/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md` §4.2 (`register_new`
//! semantics) and §7.3-§7.4 (atomic ordering) for the full contract. This
//! module owns three pieces:
//!
//! 1. [`BundleTypeId`] — a `#[repr(transparent)]` newtype over `usize`. Same
//!    shape as every other ECS identifier (mirrors `ComponentId` /
//!    `ArchetypeId` in `identifiers::primitives`). Auto `Send + Sync` via
//!    the transparent integer payload.
//!
//! 2. [`MAX_BUNDLE_TYPES`] — hard cap at `1024`. Sized to comfortably bound
//!    real engine usage (Bevy ships with roughly 50-300 distinct bundle
//!    types in shipping games; 1024 leaves 3-20x headroom) while keeping
//!    the per-world `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` cache
//!    at the **≤ 24 KB conservative upper bound** committed in §1.2, §4.3,
//!    §4.5, §10.4. The exact byte count is asserted at test time by
//!    `oncelock_size_assumptions` below — that test is the single source of
//!    truth for the memory-footprint claim.
//!
//! 3. [`register_new`] — the `#[cold] #[inline(never)]` minter. Same pattern
//!    as `component_registry::register_new`: bump a process-global counter
//!    with `Relaxed` ordering, panic if the cap is reached. The minter is
//!    invoked **exactly once per Bundle type process-wide**: the per-impl
//!    `OnceLock<BundleStaticInfo>` cell installed by `#[derive(Bundle)]`
//!    (lands in Step 4) serializes contending threads so all racers observe
//!    the same id (§7.3).
//!
//! # Atomic ordering (§7.4)
//!
//! The counter only needs uniqueness — two `fetch_add` callers must observe
//! distinct values, and we never depend on data published by any other
//! thread *via* the counter. `Relaxed` is therefore sufficient. Per-impl
//! happens-before is enforced by `OnceLock::set` (Release) /
//! `OnceLock::get` (Acquire) on the cell that holds the minted id, not by
//! the counter itself.
//!
//! # Exhaustion is terminal (W1)
//!
//! Reaching `MAX_BUNDLE_TYPES` is a configuration error, not a recoverable
//! runtime condition. `register_new` saturates the counter to
//! `MAX_BUNDLE_TYPES` **before** panicking so that re-entries (for example
//! if a panicking init closure is retried — `OnceLock::get_or_init` does
//! not poison the cell on panic; see std docs and §7.3) cannot drive the
//! counter past the cap. The panic message instructs the operator to lift
//! `MAX_BUNDLE_TYPES` and rebuild.
//!
//! # Step 0 dead-code allow
//!
//! `register_new` and `BUNDLE_NEXT_ID` have no production callers until
//! Step 4 lands `#[proc_macro_derive(Bundle)]`, which will emit per-impl
//! `OnceLock<BundleStaticInfo>` init closures that call into the minter
//! from outside this crate. Until then, the items are exercised only by
//! the `#[cfg(test)] mod tests` below. The `#[allow(dead_code)]` here is
//! Phase 8.5 step-scoped and must be removed when Step 4 wires the
//! derive — see §9 step gating in the plan.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

/// Process-global identifier for a Bundle type.
///
/// Minted lazily on the first `#[derive(Bundle)]`-generated call site for
/// each concrete `B` (Step 4 lands the derive). Two `BundleTypeId` values
/// compare equal iff they were minted from the same per-impl `OnceLock`
/// cell — that is, iff they correspond to the same Rust `Bundle` type.
///
/// `#[repr(transparent)]` over `usize` so layout / size / ABI match the raw
/// integer: zero overhead and trivially indexable into a
/// `[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]` cache.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BundleTypeId(pub usize);

/// Hard cap on the number of distinct Bundle types per process.
///
/// Sized at 1024 to comfortably exceed real-world Bundle counts (Bevy
/// games typically ship 50-300 distinct bundles) while bounding the
/// per-`EcsMaster` cache (`Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`)
/// at the **≤ 24 KB** conservative upper bound asserted across plan
/// sections §1.2, §4.3, §4.5, §10.4. Raising this constant is a deliberate
/// change — re-audit `oncelock_size_assumptions` and the §10.4 memory
/// table before bumping.
pub const MAX_BUNDLE_TYPES: usize = 1024;

/// Monotonic counter for `BundleTypeId` values minted via [`register_new`].
///
/// Same shape as `component_registry::NEXT_ID` (see that module for the
/// rationale). `Relaxed` is sufficient because (a) uniqueness across
/// concurrent callers is the only invariant the counter carries, and (b)
/// happens-before for the minted id is established by `OnceLock` in the
/// per-impl cell that the derive macro installs (Step 4).
static BUNDLE_NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Mints a fresh `BundleTypeId` from the process-global counter.
///
/// Called from each `Bundle` impl's per-impl `OnceLock<BundleStaticInfo>`
/// init closure (Step 4). `OnceLock::get_or_init` guarantees the closure
/// runs exactly once per Bundle type across all threads, so each Bundle
/// type burns exactly one slot in `BUNDLE_NEXT_ID`.
///
/// # Panics
///
/// Panics with a terminal, non-recoverable message when the counter
/// reaches [`MAX_BUNDLE_TYPES`]. The counter is **saturated** at
/// `MAX_BUNDLE_TYPES` before the panic so re-entries (for example, retries
/// of a panicking `OnceLock::get_or_init` closure — `OnceLock` does not
/// poison on panic) cannot run the counter past the cap. The intended
/// recovery is to lift `MAX_BUNDLE_TYPES` and rebuild the binary.
///
/// `#[cold] + #[inline(never)]`: this function is invoked at most once per
/// Bundle type per process; keeping it out of the hot path's i-cache
/// matters more than call overhead.
#[cold]
#[inline(never)]
pub fn register_new() -> BundleTypeId {
    // Relaxed: uniqueness only. The happens-before edge that publishes the
    // minted id to other threads is provided by the per-impl OnceLock
    // (Release on set, Acquire on get) — see module docs §7.4.
    let id = BUNDLE_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if id >= MAX_BUNDLE_TYPES {
        // Saturate so that subsequent re-entries (e.g. retries after a
        // panic in the init closure — OnceLock does not poison) cannot
        // push the counter beyond the cap. Relaxed store pairs with the
        // Relaxed fetch_add above: we only care that the value seen by
        // future loads is in [0, MAX_BUNDLE_TYPES], not any cross-thread
        // happens-before.
        BUNDLE_NEXT_ID.store(MAX_BUNDLE_TYPES, Ordering::Relaxed);
        panic!(
            "BundleTypeId exhaustion: MAX_BUNDLE_TYPES = {} reached. \
             This is a terminal panic — the process must restart. \
             If your project legitimately needs more than {} distinct \
             Bundle types, increase MAX_BUNDLE_TYPES (constant in \
             bundle_type_registry.rs).",
            MAX_BUNDLE_TYPES, MAX_BUNDLE_TYPES
        );
    }
    BundleTypeId(id)
}

/// Test-only escape hatch: forces the next [`register_new`] call to return
/// `BundleTypeId(value)`.
///
/// Exists solely to exercise the exhaustion branch in
/// `register_new_exhaustion_panics` without burning ~1024 real minter
/// slots. Never call from production code.
#[cfg(test)]
pub(crate) fn set_next_id_for_test(value: usize) {
    BUNDLE_NEXT_ID.store(value, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::mem;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use crate::ecs::identifiers::primitives::ArchetypeId;

    // ── Test serialization ───────────────────────────────────────────────────
    //
    // The tests below mutate `BUNDLE_NEXT_ID` (a process-global
    // `AtomicUsize`). Rust's default test harness runs tests in parallel
    // threads, so without serialization
    // `register_new_assigns_distinct_ids` and `register_new_exhaustion_panics`
    // would race: the exhaustion test would `set_next_id_for_test` to the
    // edge while the distinct-ids test was mid-`fetch_add`, producing
    // spurious panics. The mutex serializes the in-file test bodies;
    // `acquire_test_lock` is panic-tolerant because
    // `register_new_exhaustion_panics` poisons the mutex by design.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn acquire_test_lock() -> MutexGuard<'static, ()> {
        match TEST_MUTEX.lock() {
            Ok(g) => g,
            // The exhaustion test panics inside `register_new`. The unwind
            // poisons the mutex; we recover the guard so subsequent tests
            // keep running. Each test resets shared state up front, so
            // inheriting a "dirty" counter is fine.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Snapshot the counter on entry so we can restore it on exit. Without
    /// this the exhaustion test would leave the global counter clamped at
    /// `MAX_BUNDLE_TYPES`, poisoning any later test added to this module.
    struct CounterSnapshot(usize);

    impl CounterSnapshot {
        fn take() -> Self {
            Self(BUNDLE_NEXT_ID.load(Ordering::Relaxed))
        }
    }

    impl Drop for CounterSnapshot {
        fn drop(&mut self) {
            BUNDLE_NEXT_ID.store(self.0, Ordering::Relaxed);
        }
    }

    #[test]
    fn bundle_type_id_newtype_layout() {
        // `#[repr(transparent)]` over `usize` is load-bearing: the per-world
        // cache indexes a `[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]` array
        // by `BundleTypeId.0`, and the derive macro (Step 4) will hand the
        // raw `usize` straight to that index expression. Any layout drift
        // would silently break that contract.
        assert_eq!(
            mem::size_of::<BundleTypeId>(),
            mem::size_of::<usize>(),
            "BundleTypeId must be #[repr(transparent)] over usize"
        );
        assert_eq!(
            mem::align_of::<BundleTypeId>(),
            mem::align_of::<usize>(),
            "BundleTypeId alignment must match usize"
        );
    }

    #[test]
    fn register_new_assigns_distinct_ids() {
        let _guard = acquire_test_lock();
        let _snap = CounterSnapshot::take();

        // Park the counter well below the cap so this test never trips the
        // panic branch even if previous tests in the same binary advanced
        // BUNDLE_NEXT_ID via real Bundle derives.
        set_next_id_for_test(0);

        let a = register_new();
        let b = register_new();
        let c = register_new();

        assert_ne!(a, b, "register_new must return distinct ids (a vs b)");
        assert_ne!(b, c, "register_new must return distinct ids (b vs c)");
        assert_ne!(a, c, "register_new must return distinct ids (a vs c)");

        // Bonus: contiguity from a known base. `fetch_add` from 0 yields
        // 0, 1, 2 — anything else means the counter slipped under us.
        assert_eq!(a, BundleTypeId(0));
        assert_eq!(b, BundleTypeId(1));
        assert_eq!(c, BundleTypeId(2));
    }

    #[test]
    fn register_new_exhaustion_panics() {
        let _guard = acquire_test_lock();
        let _snap = CounterSnapshot::take();

        // Park the counter one slot below the cap: the first call must
        // succeed and return the last legal id; the second call must
        // observe `id >= MAX_BUNDLE_TYPES`, saturate, and panic.
        set_next_id_for_test(MAX_BUNDLE_TYPES - 1);

        let last = register_new();
        assert_eq!(
            last,
            BundleTypeId(MAX_BUNDLE_TYPES - 1),
            "edge call must return the final legal id"
        );

        // The exhaustion call panics. `AssertUnwindSafe` is correct because
        // we touch no `&mut` borrows that the panic could leave in a
        // logically-inconsistent state — the only side-effect is the
        // saturate `store` on the global counter, which `CounterSnapshot`
        // restores via Drop on exit.
        let result = panic::catch_unwind(AssertUnwindSafe(register_new));
        assert!(
            result.is_err(),
            "register_new must panic once the counter reaches MAX_BUNDLE_TYPES"
        );

        // The W1 saturate clamp: even if a future caller squeezes past
        // the `catch_unwind` boundary, the counter is pinned at the cap.
        let pinned = BUNDLE_NEXT_ID.load(Ordering::Relaxed);
        assert_eq!(
            pinned, MAX_BUNDLE_TYPES,
            "counter must be saturated at MAX_BUNDLE_TYPES after exhaustion"
        );
    }

    #[test]
    fn oncelock_size_assumptions() {
        // W6 ground-truth: the plan commits to "≤ 24 KB per EcsMaster
        // bundle_archetype_cache" in §1.2, §4.3, §4.5, §10.4. That number
        // is `MAX_BUNDLE_TYPES * size_of::<OnceLock<ArchetypeId>>()` rounded
        // up; the bound holds iff `size_of::<OnceLock<ArchetypeId>>() <= 24`.
        //
        // If this assertion fires on a future std update, the memory
        // footprint claims across the plan need a re-audit before Step 3
        // wires up `Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>` on
        // `EcsMaster`. Treat this test as a tripwire, not cosmetic.
        let observed = mem::size_of::<OnceLock<ArchetypeId>>();
        assert!(
            observed <= 24,
            "OnceLock<ArchetypeId> grew to {} bytes (>24); \
             re-audit plan §1.2/§4.3/§4.5/§10.4 memory footprint",
            observed
        );

        // Sanity for the cap itself — multiplication must not overflow on
        // any 64-bit target (it cannot at 24 * 1024, but make the bound
        // visible to future bumps of `MAX_BUNDLE_TYPES`).
        let total = MAX_BUNDLE_TYPES
            .checked_mul(observed)
            .expect("invariant: MAX_BUNDLE_TYPES * size_of::<OnceLock<ArchetypeId>>() fits in usize");
        assert!(
            total <= 24 * 1024,
            "per-EcsMaster cache budget {} B exceeds the ≤ 24 KB plan commitment",
            total
        );
    }

}
