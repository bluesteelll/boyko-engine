//! The physics step's own profiling zones: the split inside the colored solve that no system span
//! can see, and the per-step counters the per-stage profile is normalised by.
//!
//! # What this is for
//!
//! The kernel already brackets every system run with a `SystemSpan`, so gather, broadphase,
//! narrowphase, graph build, solve and apply each get one span per step with no code here. What a
//! system span cannot show is how the solve divides its time, and that is the question the Jolt
//! parity campaign asks first (`docs/physics/perf-campaign/01-PLAN-REV1.md` §2): how much of a step
//! is serial, how much is parallel work, and how much is lost dispatching the parallel part. The
//! zones below are the permanent sites that answer it. They are read through the kernel's own
//! profiler — [`Profiler::lifetime`](boyko_ecs::ecs::core::profiling::Profiler::lifetime) per zone
//! id, folded between steps — so the physics crate carries no timer and no side store of its own.
//!
//! # The zones
//!
//! Every zone is `Deep`, on [`ROOT_SCOPE`]: the gate a system span already uses, so arming the
//! profiler arms these with it, and a build whose tier folds system spans folds these too. The
//! "per step" column is the structural expectation a reader checks each step against (a row whose
//! counts differ from it is void), for the shipped configuration of 4 substeps and 2 relax passes.
//!
//! | Zone | Site | Per step |
//! |---|---|---|
//! | [`PHYS_SOLVE_BUILD`] | `build_bodies` + `build_columns` | 1 |
//! | [`PHYS_GRAVITY`] | substep gravity integrate | `substeps` |
//! | [`PHYS_WARM_APPLY`] | substep warm-start apply over every slot | `substeps` |
//! | [`PHYS_PASS_BIASED`] | one biased `solve_all_colors` sweep | `substeps` |
//! | [`PHYS_INTEGRATE`] | substep position integrate + inertia refresh | `substeps` |
//! | [`PHYS_PASS_RELAX`] | one relax `solve_all_colors` sweep | `substeps × relax_iterations` |
//! | [`PHYS_COLOR_WIDE`] | one color of at least [`WIDE_COLOR_MIN_SLOTS`] slots, in a sweep | wide colors × sweeps |
//! | [`PHYS_COLOR_NARROW`] | one color below it, in a sweep | narrow colors × sweeps |
//! | [`PHYS_RESTITUTION`] | the post-loop restitution pass | 1 |
//! | [`PHYS_STORE`] | the warm store's write by manifold index (and the frozen carry) + swap | 1 |
//! | [`PHYS_WRITE_BACK`] | the velocity write-back | 1 |
//! | [`PHYS_SLEEP_BEGIN`] | `IslandSleep::begin_step` | 1, sleeping on only |
//! | [`PHYS_SLEEP_FREEZE`] | the frozen-row capture, then the restore | 2, sleeping on only |
//! | [`PHYS_SLEEP_END`] | `IslandSleep::end_step` | 1, sleeping on only |
//! | [`PHYS_NP_DISPATCH`] | the parallel narrowphase's stage growth, spawn and join | 1 when it dispatched, else 0 |
//! | [`PHYS_NP_COMPACT`] | its join of the chunks' runs into the two streams | 1 when it dispatched, else 0 |
//! | [`PHYS_NP_AXIS_COMMIT`] | its serial replay of the axis writes | 1 when it dispatched, else 0 |
//!
//! The narrowphase dispatches when `parallel_narrowphase` is on, a pool of at least two workers
//! is attached and the pair count yields at least two chunks — the chunk count
//! [`PHYS_NP_CHUNKS`] reports, recomputed from the exported `NP_*` constants
//! (`narrowphase/dispatch.rs`). Its three zones open only after that decision, so a step that
//! runs the serial loop opens none of them.
//!
//! Each of the seven counters is emitted exactly once per step, including when its value is zero,
//! so its per-step sample count is 1 and its per-step `total` is the value:
//!
//! | Counter | Value |
//! |---|---|
//! | [`PHYS_SLOTS_WIDE`] | contact slots in the wide colors |
//! | [`PHYS_SLOTS_NARROW`] | contact slots in the narrow colors |
//! | [`PHYS_NP_PAIRS`] | candidate pairs the narrowphase examined |
//! | [`PHYS_NP_MANIFOLDS`] | manifolds the narrowphase handed to the solver (sensor overlaps excluded) |
//! | [`PHYS_NP_POINTS`] | live contact points over those manifolds |
//! | [`PHYS_BP_PAIRS`] | candidate pairs the broadphase emitted |
//! | [`PHYS_NP_CHUNKS`] | chunks the narrowphase dispatched (0 when it ran the serial loop) |
//!
//! A step with no simulated dynamic body returns from the solve before its first zone, so the
//! solve zones and the two slot counters are absent on that step; the broadphase and narrowphase
//! counters are not, because those systems still run.
//!
//! # Wide and narrow are classed by the inline gate's own predicate
//!
//! A color is wide when its slot count reaches `MIN_PARALLEL_SLOTS_PER_COLOR`, the floor below
//! which the colored solve runs a color inline rather than dispatching it. The predicate reads only
//! the color's slot count, so the class of a color does not depend on the worker count or on
//! `parallel_solve`: the same step classes the same colors the same way at every W, which is what
//! lets a wide span at W be compared with the same span at W = 1.
//!
//! # No zone inside a worker's chunk task
//!
//! Every site here runs on the thread that called the solve or the narrowphase. A zone's guard does
//! two `lock`-prefixed adds on its handle's shared accumulators when it closes, which is harmless on
//! one thread and a contended line if every worker's chunk task closed one. The color zones therefore bracket the
//! whole per-color call — the dispatch and the join included — from the calling thread.

use boyko_diag::profiling_abi::{GLOBAL_TIER, ZoneHandle, ZoneTier, zone_id};
use boyko_diag::sample::{Sample, SampleKind};
use boyko_diag::{clock, declare_zone, sample};
use boyko_ecs::ecs::core::profiling::ROOT_SCOPE;

// ── The solve's zones ─────────────────────────────────────────────────────────

declare_zone!(PHYS_SOLVE_BUILD, name = "phys_solve_build", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_GRAVITY, name = "phys_gravity", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WARM_APPLY, name = "phys_warm_apply", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_INTEGRATE, name = "phys_integrate", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_PASS_BIASED, name = "phys_pass_biased", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_PASS_RELAX, name = "phys_pass_relax", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_COLOR_WIDE, name = "phys_color_wide", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_COLOR_NARROW, name = "phys_color_narrow", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_RESTITUTION, name = "phys_restitution", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_STORE, name = "phys_store", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WRITE_BACK, name = "phys_write_back", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLEEP_BEGIN, name = "phys_sleep_begin", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLEEP_FREEZE, name = "phys_sleep_freeze", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLEEP_END, name = "phys_sleep_end", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The parallel narrowphase's zones (L5) ─────────────────────────────────────

declare_zone!(PHYS_NP_DISPATCH, name = "phys_np_dispatch", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_COMPACT, name = "phys_np_compact", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_AXIS_COMMIT, name = "phys_np_axis_commit", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The per-step counters ─────────────────────────────────────────────────────

declare_zone!(PHYS_SLOTS_WIDE, name = "phys_slots_wide", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLOTS_NARROW, name = "phys_slots_narrow", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_PAIRS, name = "phys_np_pairs", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_MANIFOLDS, name = "phys_np_manifolds", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_POINTS, name = "phys_np_points", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_PAIRS, name = "phys_bp_pairs", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_CHUNKS, name = "phys_np_chunks", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

/// Every span zone this crate declares, in the order of the table in the module docs.
///
/// A reader resolves each one's id with [`zone_id`] and its name from `desc.name`, so it never
/// infers an id from declaration order: ids come from one process-wide counter shared with every
/// other zone and every system span.
pub static SPAN_ZONES: [&ZoneHandle; 17] = [
    &PHYS_SOLVE_BUILD,
    &PHYS_GRAVITY,
    &PHYS_WARM_APPLY,
    &PHYS_INTEGRATE,
    &PHYS_PASS_BIASED,
    &PHYS_PASS_RELAX,
    &PHYS_COLOR_WIDE,
    &PHYS_COLOR_NARROW,
    &PHYS_RESTITUTION,
    &PHYS_STORE,
    &PHYS_WRITE_BACK,
    &PHYS_SLEEP_BEGIN,
    &PHYS_SLEEP_FREEZE,
    &PHYS_SLEEP_END,
    &PHYS_NP_DISPATCH,
    &PHYS_NP_COMPACT,
    &PHYS_NP_AXIS_COMMIT,
];

/// Every counter this crate declares, in the order of the counter table in the module docs.
pub static COUNTER_ZONES: [&ZoneHandle; 7] = [
    &PHYS_SLOTS_WIDE,
    &PHYS_SLOTS_NARROW,
    &PHYS_NP_PAIRS,
    &PHYS_NP_MANIFOLDS,
    &PHYS_NP_POINTS,
    &PHYS_BP_PAIRS,
    &PHYS_NP_CHUNKS,
];

/// Whether this build compiles the physics zones at all. Every zone is `Deep`, so one `const`
/// answers for all twenty-four; `false` under a profile whose tier ceiling is below `Deep`, where
/// every site folds to nothing and an armed profiler records none of them.
pub const ZONES_COMPILED: bool = (PHYS_SOLVE_BUILD::TIER as u8) <= (GLOBAL_TIER as u8);

/// The slot count at and above which a color is [`PHYS_COLOR_WIDE`] rather than
/// [`PHYS_COLOR_NARROW`]: the colored solve's inline floor, re-exported so a reader classes colors
/// with the solver's own number rather than a copy of it.
pub const WIDE_COLOR_MIN_SLOTS: u32 = crate::solver::colored::MIN_PARALLEL_SLOTS_PER_COLOR;

/// Emits one `Counter` sample on a zone declared above, when both of its gates admit it — the
/// counter twin of [`boyko_diag::zone!`].
///
/// `$value` is evaluated only when the zone is admitted, so a disarmed site costs what a `zone!`
/// site costs (one load and one branch), a folded site costs nothing, and an armed one also pays
/// for computing the value it reports.
macro_rules! counter {
    ($handle:ident, $value:expr) => {
        if boyko_diag::zone_enabled!($handle) {
            $crate::profiling::push_counter(&$handle, $value);
        }
    };
}
pub(crate) use counter;

/// Pushes one `Counter` sample for `handle` into the calling thread's lane.
///
/// Out of line and cold: it runs only while the profiler is armed, and keeping its body out of the
/// systems that call it keeps their disarmed path to the gate alone. The shape is the one the
/// kernel's round probe writes for its width counter.
#[cold]
#[inline(never)]
pub(crate) fn push_counter(handle: &'static ZoneHandle, value: u64) {
    // A refusal (no buffer, full region) is already counted by the region's own `overflow`, which
    // the fold reports; there is nothing a physics system could do about a full ring mid-step.
    let _ = sample::push(
        crate::__BOYKO_ZONE_PARTITION,
        Sample {
            stamp: clock::ticks(),
            value,
            zone: zone_id(handle),
            flags: SampleKind::Counter as u16,
            _pad: 0,
        },
    );
}
