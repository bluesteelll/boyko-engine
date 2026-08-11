//! Process-global `QueryTypeId` minting (Phase 12.5 Track B Wave A).
//!
//! Mirrors the [`bundle_type_registry`](crate::ecs::core::bundle::bundle_type_registry)
//! pattern: a `#[repr(transparent)]` `usize` newtype indexed into a
//! fixed-size per-world `Box<[OnceLock<_>; MAX_QUERY_TYPES]>` cache.
//!
//! See `docs/PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md` §4.1 (data structure),
//! §7.4 (atomic ordering), and §10.3 (cache memory footprint).
//!
//! # Why a `(TypeId, TypeId) → QueryTypeId` HashMap instead of a per-impl `static SLOT`
//!
//! An earlier draft of this module placed `static SLOT: OnceLock<QueryTypeId>`
//! inside the blanket `impl<D, F> QueryTypeKey for (D, F)` body and called
//! `SLOT.get_or_init(register_new)`. That pattern works for **non-generic**
//! function bodies (e.g. Phase 8.5's `BundleTypeId` mints, where each
//! `impl Bundle for ConcreteBundle` carries its own `bundle_type_id` body),
//! but it does **NOT** work inside a generic function body: per
//! [rust-lang/rust#22991](https://github.com/rust-lang/rust/issues/22991) and
//! [rust-lang/rfcs#2130](https://github.com/rust-lang/rfcs/pull/2130),
//! statics declared in generic functions are NOT monomorphised — every
//! instantiation of the function shares a single static. Consequence: every
//! `(D, F)` pair would receive the same `QueryTypeId(0)` and the per-world
//! cache would collapse to one slot.
//!
//! v1 fix: a process-global
//! `OnceLock<Mutex<HashMap<(TypeId, TypeId), QueryTypeId>>>` keyed by
//! `(TypeId::of::<D>(), TypeId::of::<F>())`. Cost:
//!
//! * Warm path: one `OnceLock::get_or_init` Acquire load (~1 ns) + one
//!   `Mutex::lock` (~10 ns uncontended) + one `HashMap::get` (~10 ns).
//!   Total ~20-30 ns per `world.query::<D, F>()` call.
//! * `EcsMaster::query` is called ~50 times per frame across all systems —
//!   not 10 000 times per entity. The combined overhead is ~1 µs/frame,
//!   invisible at 60 Hz.
//!
//! This technically violates CLAUDE.md principle 1 ("no HashMap on the
//! hot path"), but the cost shows up at most once per system-level call,
//! never per-entity. Documented trade-off; revisit in Phase 13 if profiling
//! ever surfaces this on the hot path.
//!
//! # Atomic ordering (§7.4)
//!
//! Counter ordering is `Relaxed` — uniqueness is the only invariant the
//! counter itself carries. Per-(D, F) happens-before is enforced by the
//! global `Mutex<HashMap<...>>` (mutex acquire/release establishes the
//! necessary ordering for every subsequent reader).
//!
//! # Exhaustion is terminal (mirrors Phase 8.5 W1)
//!
//! Reaching [`MAX_QUERY_TYPES`] is a configuration error, not a recoverable
//! runtime condition. [`register_new`] saturates the counter **before**
//! panicking so that re-entries (e.g. retrying after a panic inside the
//! init closure) cannot drive the counter past the cap.

use std::any::TypeId;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_log::codes::{B0502, OnceSite, W0501};
use boyko_utils::type_intern::TypeIntern;

use crate::ecs::core::iters::query::data::QueryData;
use crate::ecs::core::iters::query::filter::QueryFilter;

/// Process-global identifier for a `(D, F)` query shape.
///
/// Minted lazily on the first call to [`QueryTypeKey::query_type_id`] for
/// each concrete `(D, F)` pair. Two `QueryTypeId` values compare equal iff
/// they correspond to the same Rust `(D, F)` pair — guaranteed by the
/// `(TypeId::of::<D>(), TypeId::of::<F>())` key in the global
/// `Mutex<HashMap<...>>` registry maintained by [`QueryTypeKey`].
///
/// `#[repr(transparent)]` over `usize` so the id can be used directly as
/// an index into the per-world `Box<[OnceLock<_>; MAX_QUERY_TYPES]>` cache
/// without any pointer arithmetic overhead.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryTypeId(pub usize);

/// Hard cap on the number of distinct `(D, F)` query shapes per process.
///
/// Default of 1024 comfortably bounds real ECS games. Lifting this cap
/// requires re-auditing `oncelock_query_slot_size_assumptions` and the
/// §10.3 memory-footprint table.
#[cfg(not(feature = "big_query_table"))]
pub const MAX_QUERY_TYPES: usize = 1024;

/// Hard cap on the number of distinct `(D, F)` query shapes per process
/// when the `big_query_table` feature is enabled (I5).
#[cfg(feature = "big_query_table")]
pub const MAX_QUERY_TYPES: usize = 4096;

/// Monotonic counter for `QueryTypeId` values minted via [`register_new`].
///
/// `Relaxed` is sufficient: uniqueness across concurrent callers is the
/// only invariant the counter itself carries. Happens-before for the minted id is
/// established by [`REGISTRY`] — the intern publishes each `(key, id)` cell through a
/// `OnceLock` release-store that every reader's acquire load pairs with.
static QUERY_NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Mints a fresh [`QueryTypeId`] from the process-global counter.
///
/// Called from the `(D, F)` blanket impl's mint path (see the
/// `impl<D, F> QueryTypeKey for (D, F)` body below), under [`REGISTRY`]'s mint gate, which
/// guarantees that each `(D, F)` pair burns exactly one slot.
///
/// # Cost
///
/// Zero on the steady-state path: `world.query::<D, F>()` resolves its id from the lock-free
/// intern (a hash plus one acquire load) and never reaches here after the first sight of a
/// given `(D, F)`. Until the 2026-07 audit this call sat behind an unconditional global mutex
/// acquire — see [`REGISTRY`] for what that actually cost.
///
/// # Panics
///
/// Panics with a terminal, non-recoverable message when the counter
/// reaches [`MAX_QUERY_TYPES`]. The counter is **saturated** at
/// `MAX_QUERY_TYPES` before the panic so re-entries cannot run the
/// counter past the cap. The intended recovery is to enable the
/// `big_query_table` feature on `boyko_ecs` (raises the cap to 4096) or
/// to consolidate query shapes.
///
/// `#[cold] + #[inline(never)]`: invoked at most once per `(D, F)` per
/// process; keeping it out of the hot path's i-cache matters more than
/// call overhead.
#[cold]
#[inline(never)]
pub fn register_new() -> QueryTypeId {
    // Relaxed: uniqueness only. Happens-before is provided by the
    // surrounding `Mutex<HashMap<...>>` in `QueryTypeKey::query_type_id`
    // (the mutex acquire/release synchronises every reader behind the
    // writer that inserted the entry).
    let id = QUERY_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if id >= MAX_QUERY_TYPES {
        // Saturate so re-entries cannot push past the cap.
        QUERY_NEXT_ID.store(MAX_QUERY_TYPES, Ordering::Relaxed);
        // POSITIONAL, never `{B0502}`: an inline format argument lives inside the string
        // literal, which the registry walker's LIT stream sees and its CODE stream does not.
        panic!(
            "{}: QueryTypeId exhaustion: MAX_QUERY_TYPES = {} reached. \
             This is a terminal panic — the process must restart. \
             Enable the `big_query_table` cargo feature on boyko_ecs \
             (raises the cap to 4096) or consolidate query shapes.",
            B0502, MAX_QUERY_TYPES
        );
    }
    // `id + 1` is the occupancy AFTER this mint, so the equality fires exactly once, on the mint
    // that crossed the line -- and the number in the message is a measurement rather than the
    // threshold restated.
    if id + 1 == QUERY_TABLE_HIGH_WATER {
        report_query_table_filling(id + 1);
    }
    QueryTypeId(id)
}

/// 75 % of the query-type table.
///
/// A fraction rather than a fixed remaining count, because what matters is the **rate** relative
/// to the cap: a codebase at 768 of 1024 is one refactor from the wall whether the cap is 1024 or
/// the `big_query_table` 4096.
const QUERY_TABLE_HIGH_WATER: usize = MAX_QUERY_TYPES / 4 * 3;

/// `boyko-W0501` — the query-type table crossed 75 % occupancy.
///
/// **Without this the table's only observable behaviour was 1023 silent mints and then a process
/// kill.** `boyko-B0502` is correct and unhelpful alone: by the time it fires it names the shape
/// that happened to be last, and not the ones that filled the table. A title that grows its query
/// surface gradually crosses this line long before the other, and the gap is where the cheap fix
/// lives.
#[cold]
#[inline(never)]
fn report_query_table_filling(used: usize) {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::warn!(
            boyko_log::Query,
            W0501.number(),
            "the query-type table is {} of {} slots used (75 %); at {} the next distinct \
             Query<D, F> shape is a terminal panic (boyko-B0502) -- enable the \
             `big_query_table` feature or consolidate query shapes",
            used,
            MAX_QUERY_TYPES,
            MAX_QUERY_TYPES
        );
    }
}

/// Table size backing [`REGISTRY`] — twice [`MAX_QUERY_TYPES`], the load factor
/// [`TypeIntern`] documents for short probes.
const REGISTRY_SLOTS: usize = MAX_QUERY_TYPES * 2;

/// Process-global registry mapping `(TypeId::of::<D>(), TypeId::of::<F>())`
/// to the assigned [`QueryTypeId`].
///
/// Replaces the per-impl `static SLOT: OnceLock<QueryTypeId>` pattern
/// (which is unsound inside a generic function body — see the module
/// doc-comment for the rustc#22991 / rfcs#2130 discussion).
///
/// 2026-07 audit: this was a `OnceLock<Mutex<HashMap<(TypeId, TypeId), QueryTypeId>>>`
/// carrying the comment "the PER-FRAME system path never reaches it". Only the SystemParam
/// half of that was true. The immediate-mode `EcsMaster::query::<D, F>()` escape hatch DOES
/// run inside the frame, and it took the process-global lock UNCONDITIONALLY on every call —
/// before any memo could short-circuit it — so the admitted "~20-30 ns … ~50 times per frame"
/// was in reality one contended global lock per worker thread per query call. [`TypeIntern`]
/// keeps the rust#22991 fix (a `TypeId` key, because a `static` in a generic body collapses)
/// and drops the lock: the hit path is a hash plus one acquire load.
static REGISTRY: TypeIntern<(TypeId, TypeId), REGISTRY_SLOTS> = TypeIntern::new();

/// Static-typed key for a `(D, F)` query shape.
///
/// Implemented for every `(D, F)` pair where `D: QueryData + 'static` and
/// `F: QueryFilter + 'static`. The global `Mutex<HashMap<(TypeId, TypeId),
/// QueryTypeId>>` serialises racing callers so all observers see the same
/// id for each pair.
///
/// # Usage
///
/// `EcsMaster::query<D, F>()` calls `<(D, F) as QueryTypeKey>::query_type_id()`
/// once per cache lookup. Cost: ~20-30 ns (Mutex lock + HashMap lookup).
/// Acceptable at system-call frequency (`query()` is called ~50 times per
/// frame), not at per-entity frequency.
pub trait QueryTypeKey: 'static {
    /// Returns the process-global [`QueryTypeId`] for this `(D, F)` pair.
    fn query_type_id() -> QueryTypeId;
}

impl<D, F> QueryTypeKey for (D, F)
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    // Lock-free get-or-mint over the once-per-`(D, F)` intern (rust#22991 forces the
    // `TypeId` key). A hit is a hash plus one acquire load; only a first-sight `(D, F)`
    // takes the table's cold mint gate, and `register_new` keeps owning the id dispenser
    // and its terminal exhaustion panic.
    #[inline]
    fn query_type_id() -> QueryTypeId {
        let key = (TypeId::of::<D>(), TypeId::of::<F>());
        let id = REGISTRY
            .get_or_mint_with(key, |_| register_new().0 as u32)
            .unwrap_or_else(query_intern_full);
        QueryTypeId(id as usize)
    }
}

/// Terminal panic for a full [`REGISTRY`] table.
///
/// Distinct from [`register_new`]'s cap panic: that one fires when the ID DISPENSER is
/// exhausted, this one when the intern TABLE cannot seat another key. With
/// `REGISTRY_SLOTS = MAX_QUERY_TYPES * 2` the dispenser is always the first to give out, so
/// reaching here means the two caps drifted apart in a later edit.
#[cold]
#[inline(never)]
fn query_intern_full() -> u32 {
    panic!(
        "query type intern table full: REGISTRY_SLOTS = {REGISTRY_SLOTS} cannot seat another \
         (D, F) key while MAX_QUERY_TYPES = {MAX_QUERY_TYPES} ids remain mintable. The table \
         must stay at least twice the id cap — see TypeIntern's load-factor contract."
    );
}

/// Test-only escape hatch: forces the next [`register_new`] call to return
/// `QueryTypeId(value)`.
///
/// Exists solely to exercise the exhaustion branch without burning ~1024
/// real minter slots. Never call from production code.
#[cfg(test)]
pub(crate) fn set_next_id_for_test(value: usize) {
    QUERY_NEXT_ID.store(value, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    // Test-only serialisation of the process-global query-id minter: the `Mutex`
    // is the harness's exclusion lock (these tests mutate a process-wide counter
    // and must not overlap), not engine data. Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use super::*;

    use std::mem;
    use std::panic::{self, AssertUnwindSafe};
    use std::ptr::NonNull;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    // ── Test serialization (mirrors Phase 8.5 pattern) ──────────────────
    //
    // The tests below mutate `QUERY_NEXT_ID`. Rust's default test harness
    // runs tests in parallel, so without serialization
    // `register_new_assigns_distinct_ids` and `register_new_exhaustion_panics`
    // would race.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn acquire_test_lock() -> MutexGuard<'static, ()> {
        match TEST_MUTEX.lock() {
            Ok(g) => g,
            // The exhaustion test panics inside `register_new`. The unwind
            // poisons the mutex; recover the guard so subsequent tests run.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Snapshot the counter on entry so it can be restored on exit.
    struct CounterSnapshot(usize);

    impl CounterSnapshot {
        fn take() -> Self {
            Self(QUERY_NEXT_ID.load(Ordering::Relaxed))
        }
    }

    impl Drop for CounterSnapshot {
        fn drop(&mut self) {
            QUERY_NEXT_ID.store(self.0, Ordering::Relaxed);
        }
    }

    #[test]
    fn query_type_id_newtype_layout() {
        assert_eq!(
            mem::size_of::<QueryTypeId>(),
            mem::size_of::<usize>(),
            "QueryTypeId must be #[repr(transparent)] over usize"
        );
        assert_eq!(
            mem::align_of::<QueryTypeId>(),
            mem::align_of::<usize>(),
            "QueryTypeId alignment must match usize"
        );
    }

    #[test]
    fn register_new_assigns_distinct_ids() {
        let _guard = acquire_test_lock();
        let _snap = CounterSnapshot::take();

        set_next_id_for_test(0);

        let a = register_new();
        let b = register_new();
        let c = register_new();

        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);

        assert_eq!(a, QueryTypeId(0));
        assert_eq!(b, QueryTypeId(1));
        assert_eq!(c, QueryTypeId(2));
    }

    #[test]
    fn register_new_exhaustion_panics() {
        let _guard = acquire_test_lock();
        let _snap = CounterSnapshot::take();

        set_next_id_for_test(MAX_QUERY_TYPES - 1);
        let last = register_new();
        assert_eq!(last, QueryTypeId(MAX_QUERY_TYPES - 1));

        let result = panic::catch_unwind(AssertUnwindSafe(register_new));
        assert!(result.is_err());

        // W1 saturate clamp.
        let pinned = QUERY_NEXT_ID.load(Ordering::Relaxed);
        assert_eq!(pinned, MAX_QUERY_TYPES);
    }

    /// QC8 tripwire — the per-world cache slot footprint must fit
    /// `Box<[OnceLock<(NonNull<()>, fn(NonNull<()>))>; MAX_QUERY_TYPES]>`
    /// in ≤ 32 KB at `MAX_QUERY_TYPES = 1024` (≤ 128 KB at the
    /// `big_query_table` cap of 4096).
    #[test]
    fn oncelock_query_slot_size_assumptions() {
        let observed = mem::size_of::<OnceLock<(NonNull<()>, fn(NonNull<()>))>>();
        assert!(
            observed <= 32,
            "OnceLock<(NonNull<()>, fn(NonNull<()>))> grew to {} bytes (>32); \
             re-audit plan §10.3 memory footprint",
            observed
        );
        let total = MAX_QUERY_TYPES
            .checked_mul(observed)
            .expect("invariant: MAX_QUERY_TYPES * size_of slot fits in usize");
        let budget = if cfg!(feature = "big_query_table") {
            128 * 1024
        } else {
            32 * 1024
        };
        assert!(
            total <= budget,
            "per-EcsMaster cache budget {} B exceeds the {} B plan commitment",
            total, budget
        );
    }
}
