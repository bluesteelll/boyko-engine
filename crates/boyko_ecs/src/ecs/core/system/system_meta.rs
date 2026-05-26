//! Per-system metadata: name, declared access, observed generations,
//! and (Phase 10 Wave A Step 3) the change-detection tick snapshots
//! `last_run` / `this_run` populated by the dispatcher via
//! [`System::set_change_ticks`].
//!
//! See Phase 8a plan §4.2 / §9 for the original field rationale and
//! Phase 10 plan §2.6 SCT1-SCT4 + §4.2-bis + §11.2 for the Phase 10
//! extension and the constructor-signature change (Round 2 W5).
//!
//! # Layout sizing (Round 2 O5 — naming harmonised)
//!
//! Field bytes (Phase 8a baseline): `Access` 192 B, `&'static str` 16 B,
//! two `ArchetypeGeneration` (`NonZeroUsize`) 8 B each = 224 B.
//!
//! Phase 10 Wave A appends two [`Tick`] fields (4 B each) = 232 B of field
//! bytes. `Access`'s `BitSet256` carries `#[repr(C, align(32))]`
//! (`bit_set_256.rs`), which drives `Access`'s alignment to 32 B and
//! therefore `SystemMeta`'s alignment to 32 B as well. The struct is
//! rounded up to **256 B = 4 cache lines exactly**.
//!
//! Note: plan §11.1 projected 232 B from the raw field arithmetic and
//! elided the `BitSet256` alignment bump. The real layout — 256 B —
//! already matched the Phase 8a baseline (which paid the same alignment
//! tax in tail padding); the Phase 10 ticks therefore slot into the
//! existing tail padding for a **zero `size_of` bump**.
//!
//! [`System::set_change_ticks`]: super::system::System::set_change_ticks

use std::sync::OnceLock;

use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use crate::ecs::core::system::access::Access;

/// Phase 12.5 Track B W3 — BSS footprint tripwire.
///
/// `SystemMeta::dummy()` is backed by a process-global
/// `static DUMMY: OnceLock<SystemMeta>`. `OnceLock<T>` on Rust 1.85
/// carries `MaybeUninit<T>` + an `AtomicBool` init flag, padded to
/// `align_of::<T>()`. With `SystemMeta` at align 32 (the inner
/// `BitSet256` field drives AVX2-friendly alignment), `OnceLock<SystemMeta>`
/// inherits align 32 and the total BSS footprint lands in 288-320 B
/// worst case.
///
/// This tripwire fails the build immediately if a future `SystemMeta`
/// growth or stdlib `OnceLock` layout change pushes the footprint past
/// the 320 B budget — caught at compile time with the documented
/// context.
const _: () = assert!(
    core::mem::size_of::<OnceLock<SystemMeta>>() <= 320,
    "SystemMeta::dummy() BSS footprint exceeded 320 B budget; \
     reduce SystemMeta size or revisit PHASE-12.5-QUERY-OPTIMIZATIONS-PLAN.md §10.5"
);

/// Per-system context carried alongside the system body.
///
/// Holds the declared [`Access`] surface, a diagnostic name, the
/// archetype generations observed at the last refresh (consumed by
/// Phase 8b `Query::new_archetype`), and the Phase 10 change-detection
/// tick snapshots (`last_run` / `this_run`).
///
/// `SystemMeta` is constructed via [`SystemMeta::new`] (which takes the
/// world's current tick — Round 2 W5) and filled during the system's
/// two-phase init (`SystemParam::init_state` then `SystemParam::init_access`).
/// After [`FilteredAccessSet::finalize`] copies the accumulated [`Access`]
/// into the meta, the access bits are read-only for the rest of the
/// system's lifetime. Tick fields continue to update each frame via
/// [`System::set_change_ticks`].
///
/// [`FilteredAccessSet::finalize`]: super::FilteredAccessSet::finalize
/// [`System::set_change_ticks`]: super::system::System::set_change_ticks
#[repr(C)]
pub struct SystemMeta {
    /// Read/write surface declared by the system's parameters.
    ///
    /// Filled during `init_access` via the [`FilteredAccessSet`] accumulator;
    /// read-only thereafter (write-once contract — see [`Access`] docs).
    ///
    /// [`FilteredAccessSet`]: super::FilteredAccessSet
    pub(crate) access: Access,

    /// Diagnostic name (`std::any::type_name::<Self>()` by default).
    pub(crate) name: &'static str,

    /// Last `archetype_generation` observed at the last `new_archetype` /
    /// `init_access` pass. Phase 8b `Query::new_archetype` uses this to
    /// decide when to refresh archetype caches.
    pub(crate) last_archetype_generation: ArchetypeGeneration,

    /// Last `structural_generation` observed. Same pattern; consumed by
    /// Phase 8b dual-generation cache to detect ArchetypeId-ABA hazards.
    pub(crate) last_structural_generation: ArchetypeGeneration,

    /// Phase 10 — last frame's `this_run` snapshot. Captures the upper
    /// bound of the previous observation window; reads / filters compare
    /// per-row ticks against `(last_run, this_run]`.
    ///
    /// Written exclusively by the dispatcher via
    /// [`System::set_change_ticks`] before each `run_unsafe` (plan
    /// §2.6 SCT4). Initialised by [`SystemMeta::new`] to
    /// `current_tick - MAX_CHANGE_AGE` so the first run reports every
    /// pre-existing tick as "Changed since last run" (plan §9.4 + W5).
    ///
    /// [`System::set_change_ticks`]: super::system::System::set_change_ticks
    pub(crate) last_run: Tick,

    /// Phase 10 — current frame's `this_run` snapshot. Mirrors the
    /// dispatcher-wide value bumped at the start of [`Schedule::run`];
    /// each system's copy is published by [`System::set_change_ticks`]
    /// immediately before its `run_unsafe`.
    ///
    /// [`System::set_change_ticks`]: super::system::System::set_change_ticks
    pub(crate) this_run: Tick,
}

impl SystemMeta {
    /// Constructs a fresh meta with the given diagnostic `name` and empty
    /// [`Access`].
    ///
    /// # Phase 10 Round 2 W5 — `current_tick` parameter
    ///
    /// Every System construction site MUST pass the world's current tick
    /// so this constructor can initialise `last_run = current_tick -
    /// MAX_CHANGE_AGE`. That value places the system's observation horizon
    /// at the oldest still-valid tick, guaranteeing the first run sees
    /// every existing entity as "Changed" — the desired semantic for
    /// late-added systems (plan §9.4).
    ///
    /// This eliminates the Round 1 bypass-`initialize` bug where custom
    /// `System` impls (`NoopSystem`, test stubs, etc.) ended up with
    /// `last_run = Tick::ZERO` and produced wrong `Changed<T>` filter
    /// results until their first frame completed.
    ///
    /// `this_run` is initialised to the same value as `last_run` (a
    /// pre-first-run sentinel) and overwritten by the dispatcher's first
    /// call to [`System::set_change_ticks`] (plan §2.6 SCT4 / §9.4 PHASE9.4).
    ///
    /// Both generation fields start at [`ArchetypeGeneration::FIRST`] —
    /// the canonical "never observed" sentinel that compares less than
    /// any post-bump value the master can produce. Phase 8b `Query`
    /// overwrites these on its first archetype-refresh pass.
    ///
    /// [`System::set_change_ticks`]: super::system::System::set_change_ticks
    pub fn new(name: &'static str, current_tick: Tick) -> Self {
        let last_run = Tick::new(current_tick.get().wrapping_sub(MAX_CHANGE_AGE));
        Self {
            access: Access::new(),
            name,
            last_archetype_generation: ArchetypeGeneration::FIRST,
            last_structural_generation: ArchetypeGeneration::FIRST,
            last_run,
            // Pre-first-run sentinel; updated by `set_change_ticks` on the
            // next dispatch (plan §9.4 PHASE9.4).
            this_run: last_run,
        }
    }

    /// Test-only convenience constructor — assumes a sentinel
    /// `current_tick = Tick::new(1)`.
    ///
    /// Production code MUST go through [`Self::new`] with the world's
    /// real tick (see Phase 10 plan §15.2 migration table). This helper
    /// exists to (a) make unit-test rewrites mechanical and (b) keep
    /// FunctionSystem / ExclusiveFunctionSystem / NoopSystem buildable in
    /// Wave A — Step 14 (Wave 8) will refit those constructors to thread
    /// the world tick. Until then, `for_testing` is the bridge.
    ///
    /// The resulting `last_run` is `1 - MAX_CHANGE_AGE` (a valid sentinel
    /// per the §9.4 pre-first-run analysis).
    pub fn for_testing(name: &'static str) -> Self {
        Self::new(name, Tick::new(1))
    }

    /// Returns the diagnostic name (typically `std::any::type_name::<F>()`
    /// of the underlying function or closure).
    #[inline]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the declared read/write surface.
    ///
    /// Phase 9 scheduler calls this on every system to build the conflict
    /// graph; Phase 8a uses it for diagnostics and end-to-end assertions.
    #[inline]
    pub fn access(&self) -> &Access {
        &self.access
    }

    /// Returns this system's last-run tick (plan §2.6 SCT3).
    ///
    /// Filters and `Mut<T>` / `Ref<T>` consume this through the dispatcher
    /// boundary captured by `Query` (Wave B+).
    #[inline]
    pub fn last_run(&self) -> Tick {
        self.last_run
    }

    /// Returns this system's current-run tick (plan §2.6 SCT3).
    #[inline]
    pub fn this_run(&self) -> Tick {
        self.this_run
    }

    /// Phase 12.5 Track B NCD7 (C-NEW-1 / W2) — lazy `'static` dummy meta.
    ///
    /// Sole consumer:
    /// [`QueryView`](crate::ecs::core::iters::query::query_view::QueryView)
    /// `iter` / `iter_mut` / `par_iter` / `par_iter_mut` / `single` / `get`
    /// pass `SystemMeta::dummy()` as the cursor's meta argument because the
    /// cursor's `meta: &SystemMeta` field is type-required even on the
    /// const-folded NCD = false dispatch path. The NCD6 dispatcher inside
    /// `QueryIter::next` / `QueryIterMut::next` / `for_each_impl`
    /// const-folds the meta-fetch branch away when neither `D` nor `F`
    /// needs change detection, so the dummy's tick/generation fields are
    /// **never observed** on the !NCD path.
    ///
    /// # Why lazy and not const (C-NEW-1)
    ///
    /// `Access::new()` builds via `ComponentMask::new()` → `BitSet::<u64>::new()`
    /// which calls `T::default()` (non-const trait method). `const_trait_impl`
    /// is unstable in Rust 1.85; promoting the entire `BitSet` foundation
    /// to const-fn is out of scope for Phase 12.5. Falling back to one-shot
    /// `OnceLock` initialisation: first call ≈ 50 ns (`OnceLock::set`
    /// Release store + body of `Access::new`); subsequent calls ≈ 1-2 ns
    /// (Acquire load).
    ///
    /// # Pointer stability (W2)
    ///
    /// Every call returns the same `'static SystemMeta` address; the Miri
    /// test `miri_system_meta_dummy_lazy_init` exercises this with a
    /// sequential 1000-iteration loop.
    ///
    /// # Why ZERO-initialised ticks and FIRST-initialised generations
    ///
    /// The direct API path that consumes `dummy()` never reads these fields
    /// (NCD6 const-fold elides the access). They are present only because
    /// `SystemMeta`'s field surface requires them; their values are
    /// arbitrary on the consumer path.
    #[inline]
    pub fn dummy() -> &'static SystemMeta {
        static DUMMY: OnceLock<SystemMeta> = OnceLock::new();
        DUMMY.get_or_init(|| SystemMeta {
            access: Access::new(),
            name: "<dummy>",
            last_archetype_generation: ArchetypeGeneration::FIRST,
            last_structural_generation: ArchetypeGeneration::FIRST,
            last_run: Tick::ZERO,
            this_run: Tick::ZERO,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SystemMeta::new` produces a meta whose `Access` carries no reads or
    /// writes.
    #[test]
    fn new_initialises_with_empty_access() {
        let meta = SystemMeta::new("test_system", Tick::new(1));
        let other = Access::new();
        // Two empty accesses must not conflict — neither has any bits set.
        assert!(
            !meta.access().conflicts_with(&other),
            "freshly-constructed SystemMeta must have empty Access"
        );
    }

    /// `SystemMeta::name` returns the `&'static str` passed at construction.
    #[test]
    fn name_returns_static_str() {
        let meta = SystemMeta::new("alpha_system", Tick::new(1));
        assert_eq!(meta.name(), "alpha_system");
    }

    /// Generation fields start at `ArchetypeGeneration::FIRST`.
    #[test]
    fn generations_start_at_first() {
        let meta = SystemMeta::new("gen_test", Tick::new(1));
        assert_eq!(meta.last_archetype_generation, ArchetypeGeneration::FIRST);
        assert_eq!(meta.last_structural_generation, ArchetypeGeneration::FIRST);
    }

    /// **Phase 10 Round 2 W5 — load-bearing regression test (plan §13.1).**
    ///
    /// `SystemMeta::new(name, current_tick)` initialises `last_run =
    /// current_tick - MAX_CHANGE_AGE`. That value is the oldest still-valid
    /// tick from the system's perspective — anything the world stored at
    /// or after construction-time appears "newer than last_run", which is
    /// the correct first-run behaviour.
    #[test]
    fn system_meta_new_initializes_last_run_correctly() {
        let current = Tick::new(1_000);
        let meta = SystemMeta::new("init_test", current);
        let expected = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(meta.last_run(), expected);
        // Pre-first-run sentinel: `this_run == last_run`. The dispatcher's
        // first `set_change_ticks` call promotes `this_run` to the real
        // current frame value (plan §9.4 PHASE9.4).
        assert_eq!(meta.this_run(), expected);
    }

    /// `for_testing` is a convenience wrapper over `new(name, Tick::new(1))`.
    #[test]
    fn for_testing_uses_sentinel_tick() {
        let meta = SystemMeta::for_testing("test_helper");
        let expected_last = Tick::new(1u32.wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(meta.last_run(), expected_last);
        assert_eq!(meta.this_run(), expected_last);
        assert_eq!(meta.name(), "test_helper");
    }

    /// Layout pin — see the module-level "Layout sizing" doc block for
    /// the algebra. Plan §11.1 projected 232 B from field bytes alone but
    /// missed `Access`'s `BitSet256` 32-byte alignment, which rounds the
    /// total up to 256 B (== Phase 8a's actual baseline; the two Phase 10
    /// `Tick` fields fit into the existing tail padding for a zero-cost
    /// extension).
    #[test]
    fn system_meta_size_is_256_bytes() {
        assert_eq!(
            core::mem::size_of::<SystemMeta>(),
            256,
            "BitSet256 inside Access drives 32 B alignment ⇒ 256 B total"
        );
        // Confirm Phase 10 added zero size_of cost beyond the existing
        // alignment-driven tail padding: 32 B alignment implies size is a
        // multiple of 32 B.
        assert_eq!(core::mem::align_of::<SystemMeta>(), 32);
    }

    /// Phase 12.5 Track B C3 — `SystemMeta::dummy()` field-value pin.
    ///
    /// The lazy-init `OnceLock<SystemMeta>` body for `dummy()` stores
    /// `last_run = Tick::ZERO` and `this_run = Tick::ZERO` — the
    /// zero-sentinel chosen because the NCD6 const-fold dispatcher
    /// elides the meta load on the !NCD direct API path, so these
    /// values are never observed. The test pins the sentinel choice so
    /// any future contributor who replaces it (e.g. with
    /// `Tick::new(u32::MAX)` or with `SystemMeta::default`) trips a
    /// failing assertion that points at this design.
    #[test]
    fn system_meta_dummy_field_values_match_zero_sentinel() {
        let dummy = SystemMeta::dummy();
        assert_eq!(
            dummy.last_run(),
            Tick::ZERO,
            "SystemMeta::dummy() must initialise last_run = Tick::ZERO"
        );
        assert_eq!(
            dummy.this_run(),
            Tick::ZERO,
            "SystemMeta::dummy() must initialise this_run = Tick::ZERO"
        );
        assert_eq!(
            dummy.name(),
            "<dummy>",
            "SystemMeta::dummy() must carry the documented sentinel name"
        );
    }
}
