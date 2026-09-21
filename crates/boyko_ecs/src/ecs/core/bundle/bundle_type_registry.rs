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

#[cfg(test)]
mod tests {
    use super::*;

    use std::mem;
    use std::sync::OnceLock;

    use crate::ecs::identifiers::primitives::ArchetypeId;

    // Nothing here STORES to `BUNDLE_NEXT_ID`. The lib-test binary runs every src/ test module in
    // one process, and the ones that spawn a first-sight bundle (`bundle_api.rs`,
    // `hierarchy/bundles.rs`, `migration_helpers.rs`, `spawn_batch_command.rs`, ...) mint through
    // their per-type `static INFO: OnceLock<BundleStaticInfo>` closure without any lock this
    // module could take: a test that parked the counter at the cap redded a sibling's unrelated
    // test with the exhaustion panic, and one that parked it at 0 handed a sibling a
    // `BundleTypeId` the dispenser had already given out -- the per-world cache's index (A4b).
    // The exhaustion contract lives in `tests/bundle_table_exhaustion.rs`, a process of its own.

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

    /// Three mints on one thread come back distinct and strictly increasing. Strictly increasing,
    /// not contiguous: other harness threads mint from the same counter, and the gaps are theirs.
    #[test]
    fn register_new_assigns_distinct_ids() {
        let a = register_new();
        let b = register_new();
        let c = register_new();

        assert!(
            a.0 < b.0 && b.0 < c.0,
            "the dispenser is monotonic: expected {a:?} < {b:?} < {c:?}"
        );
        assert!(
            c.0 < MAX_BUNDLE_TYPES,
            "the lib-test binary must stay well below the cap; got {c:?}"
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
