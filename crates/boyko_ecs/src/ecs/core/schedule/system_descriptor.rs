//! [`SystemDescriptor`] — per-system staging slot inside [`ScheduleBuilder`].
//!
//! See Phase 9 plan §5.6 / §5.5 (Round 3 W-NEW-3). The descriptor wraps a
//! [`SystemBox`] and the ordering hints / set memberships collected
//! through the [`SystemConfig`] fluent API. At [`ScheduleBuilder::build`]
//! time the descriptors are consumed and reshaped into the final
//! `Schedule::systems` slice in topological order.
//!
//! # `Vec` over `SmallVec`
//!
//! Plan §5.6 specifies `SmallVec<[X; 2]>` for the ordering fields. The
//! current implementation uses `Vec<X>` to keep the crate's direct
//! dependency surface tight (the Phase 8.5 baseline does not pull
//! `smallvec` and we don't want to add a dep solely for builder ergonomics
//! at arity ≤ 4). The hot path (`Schedule::run`) never touches these
//! fields — they exist only during `build`, which already allocates a
//! handful of Vecs per system.
//!
//! [`SystemBox`]: super::system_box::SystemBox
//! [`SystemConfig`]: super::system_config::SystemConfig
//! [`ScheduleBuilder`]: super::schedule_builder::ScheduleBuilder
//! [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build

use crate::ecs::core::schedule::ordering::OrderingEdge;
use crate::ecs::core::schedule::system_box::{BoolSystem, SystemBox};
use crate::ecs::core::schedule::system_set::SystemSetId;

/// Builder-side staging record for one system.
///
/// Constructed by `ScheduleBuilder::add_system`; consumed and dropped by
/// `ScheduleBuilder::build` after topological sorting. The struct is
/// `pub(crate)` because only the schedule internals manipulate it.
///
/// # Field order
///
/// `system_box → ordering_hints → sets` — `system_box` is moved into the
/// final `Schedule` on build, so it sits first to keep the move cheap on
/// the hot `build` path; the other two are dropped in place.
pub(crate) struct SystemDescriptor {
    /// Erased system body + cached [`SystemKind`] + name. Moved out by
    /// `build` into the final `Schedule::systems` slice.
    ///
    /// [`SystemKind`]: crate::ecs::core::system::system_kind::SystemKind
    pub(crate) system_box: SystemBox,

    /// Pre-build ordering edges collected by `SystemConfig::before` /
    /// `.after` / `.chain` calls on this descriptor's handle. Includes
    /// both directions (the receiver-side handle) — `build` reduces them
    /// to a flat `Vec<OrderingEdge>` before running Tarjan SCC.
    pub(crate) ordering_hints: Vec<OrderingEdge>,

    /// Set memberships collected by `SystemConfig::in_set`. Expanded
    /// into pairwise edges by the sync-point analyzer in Wave 5 Step 14;
    /// Wave 4 Step 9 ignores these for DAG construction.
    pub(crate) sets: Vec<SystemSetId>,

    /// Phase 16 — own run-conditions, in declaration order. Empty for the
    /// overwhelming majority of systems. Each is an initialized
    /// [`BoolSystem`]; multiple `.run_if(a).run_if(b)` accumulate here and
    /// fold to an AND (eager, never short-circuited — see `PHASE-16-PLAN.md`
    /// §6). Moved out into `Schedule::system_conditions` at build (§2.5).
    pub(crate) conditions: Vec<BoolSystem>,

    /// Phase 4 D5 / CR-B — GPU-compute marker. `false` for every existing
    /// system (zero change), so the `SystemKind` resolution at
    /// `ScheduleBuilder::build` stays byte-identical to the previous
    /// `is_exclusive` derivation. When `true`, the descriptor resolves
    /// [`SystemKind::GpuCompute`](crate::ecs::core::system::system_kind::SystemKind::GpuCompute)
    /// — a marker carve-out that is NOT derived from access (Phase-5-set).
    pub(crate) is_gpu: bool,
}

impl SystemDescriptor {
    /// Constructs a descriptor with no ordering hints / set memberships.
    /// `SystemConfig` chained calls append to the vecs in place.
    #[inline]
    pub(crate) fn new(system_box: SystemBox) -> Self {
        Self {
            system_box,
            ordering_hints: Vec::new(),
            sets: Vec::new(),
            conditions: Vec::new(),
            // Phase 4 D5 — default false: a plain `add_system` is never GPU.
            is_gpu: false,
        }
    }
}
