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
use crate::ecs::core::system::gpu_intent::GpuAccessIntent;

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

    /// Phase 4 Seam 4 (D7 / IM-6) — abstract per-system GPU access descriptor.
    ///
    /// `None` for every CPU system (the 0%-gate): zero alloc, never deref'd.
    /// A GPU-compute system sets it via [`set_gpu_intent`](Self::set_gpu_intent)
    /// so a future `boyko_render` can lower `(conflict edge, intent_src,
    /// intent_dst)` into a precise `vkCmdPipelineBarrier`. Graphics-pure:
    /// [`GpuAccessIntent`] names no `Vk*` type.
    ///
    /// `#[repr(C)]` append-only — added AFTER `this_run` so the existing
    /// fields keep their offsets; the 8-byte `Option<Box<_>>` (niche-optimised
    /// to a nullable pointer) fits the struct's 24-byte tail padding (232 + 8
    /// + 1 = 241 ≤ 256), so `size_of::<SystemMeta>()` stays 256 (IM-6 pin).
    pub(crate) gpu_intent: Option<Box<GpuAccessIntent>>,

    /// Phase 4 Seam 2 (D6 / CR-B / IM-4) — `true` iff a `SystemParam` in this
    /// system requires the dispatcher thread (currently any NonSend param).
    ///
    /// Set by `NonSendRes` / `NonSendResMut::init_access` via
    /// [`mark_requires_dispatcher`](Self::mark_requires_dispatcher). Those
    /// params ALSO declare universal access (CR-B), so the existing
    /// `is_universal()` derivation resolves the system to
    /// `SystemKind::CpuExclusive` independently — this flag is the explicit
    /// record of WHY (diagnostics + a future non-universal dispatcher kind).
    /// `false` for every existing system (the 0%-gate). Fits the same tail
    /// padding as `gpu_intent`.
    pub(crate) requires_dispatcher: bool,
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
            // Phase 4 — no GPU intent / no dispatcher requirement by default
            // (the 0%-gate: every CPU system starts here).
            gpu_intent: None,
            requires_dispatcher: false,
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

    /// Returns this system's abstract GPU access descriptor, or `None`
    /// (Phase 4 Seam 4 / D7). `None` for every CPU system.
    #[inline]
    pub fn gpu_intent(&self) -> Option<&GpuAccessIntent> {
        self.gpu_intent.as_deref()
    }

    /// Sets this system's abstract GPU access descriptor (Phase 4 Seam 4).
    ///
    /// Cold setup path — called by a GPU system's adapter during init. Boxes
    /// the intent so `SystemMeta`'s footprint stays at 8 B for the field
    /// (and 0 B of heap for the CPU-only common case where this is never
    /// called).
    #[inline]
    pub fn set_gpu_intent(&mut self, intent: GpuAccessIntent) {
        self.gpu_intent = Some(Box::new(intent));
    }

    /// Returns `true` iff a `SystemParam` in this system requires the
    /// dispatcher thread (Phase 4 Seam 2 / D6). Set by NonSend params.
    #[inline]
    pub fn requires_dispatcher(&self) -> bool {
        self.requires_dispatcher
    }

    /// Marks this system as requiring the dispatcher thread (Phase 4 Seam 2).
    ///
    /// Called by `NonSendRes` / `NonSendResMut::init_access`. Idempotent
    /// (sets a flag). The param ALSO declares universal access (CR-B), which
    /// is what actually drives the `SystemKind::CpuExclusive` resolution; this
    /// flag records the requirement explicitly.
    #[inline]
    pub fn mark_requires_dispatcher(&mut self) {
        self.requires_dispatcher = true;
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
            gpu_intent: None,
            requires_dispatcher: false,
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

    /// Phase 4 IM-6 — appending `gpu_intent` (8 B) + `requires_dispatcher`
    /// (1 B) after `this_run` consumed only the existing 24 B tail padding
    /// (232 + 9 = 241 ≤ 256), so `SystemMeta` stays exactly 256 B and the
    /// `OnceLock<SystemMeta> <= 320` BSS tripwire (module-level const-assert)
    /// is unaffected.
    #[test]
    fn system_meta_stays_256_after_phase4_fields() {
        // The const-assert at the top of this module already pins
        // `OnceLock<SystemMeta> <= 320`; assert the 256 B size again here so a
        // future field addition that pushes past the tail padding is caught
        // with a Phase-4-specific message.
        assert_eq!(
            core::mem::size_of::<SystemMeta>(),
            256,
            "Phase 4 gpu_intent + requires_dispatcher must fit the 24 B tail padding"
        );
    }

    /// Phase 4 D7 — a fresh meta carries no GPU intent (the 0%-gate default).
    #[test]
    fn gpu_intent_defaults_to_none() {
        let meta = SystemMeta::for_testing("gpu_default");
        assert!(
            meta.gpu_intent().is_none(),
            "a CPU system must carry no GPU intent"
        );
        assert!(
            !meta.requires_dispatcher(),
            "a CPU system must not require the dispatcher"
        );
    }

    /// Phase 4 D7 — set/get round-trip for the GPU intent.
    #[test]
    fn gpu_intent_set_get_round_trip() {
        use crate::ecs::core::system::gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage};
        use crate::ecs::memory::device_column::DeviceColumnHandle;

        let mut meta = SystemMeta::for_testing("gpu_set");
        let mut intent = GpuAccessIntent::new(GpuStage::Compute);
        intent.push(DeviceColumnHandle(3), GpuAccess::Write);
        meta.set_gpu_intent(intent);

        let got = meta.gpu_intent().expect("intent must be Some after set");
        assert_eq!(got.stage(), GpuStage::Compute);
        assert_eq!(got.touches().len(), 1);
        assert_eq!(got.touches()[0].column, DeviceColumnHandle(3));
        assert_eq!(got.touches()[0].access, GpuAccess::Write);
    }

    /// Phase 4 D6 — `mark_requires_dispatcher` flips the flag.
    #[test]
    fn mark_requires_dispatcher_sets_flag() {
        let mut meta = SystemMeta::for_testing("dispatcher_mark");
        assert!(!meta.requires_dispatcher());
        meta.mark_requires_dispatcher();
        assert!(meta.requires_dispatcher());
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
