//! KE17 — sizing the apply-window barrier.
//!
//! # The mechanism this bench sizes
//!
//! `Schedule::executor_main_loop`
//! (`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:564`) clears a finished
//! system's `running` bit in exactly one place on the concurrent path:
//! `apply_window_drain` (`:719`, the clear at `:745`). That function is called
//! only when the gate at `:623` fires:
//!
//! ```text
//! let pending = completion.pending_load(Acquire);                  // :621
//! let running = self.executor_scratch.running.count_ones(..);      // :622
//! if pending > 0 && (pending == running || running == 0) { ... }   // :623
//! ```
//!
//! `pending` counts systems that have finished and pushed a completion;
//! `running` counts systems that were dispatched and not yet drained. The two
//! are equal only at QUIESCENCE — when every dispatched system has finished.
//! So a finished system's `running` bit, its `completed` bit and its
//! successors' `pred_remaining` all stay frozen until the LAST member of the
//! in-flight set finishes.
//!
//! Two things are blocked in that window, both checked in
//! `try_dispatch_ready` (`:990`): a system whose `conflict_bits` intersect the
//! stale `running` set (`:1026`), and a system whose predecessor has finished
//! but is not yet `completed` (`pred_remaining != 0`, `:1022`).
//!
//! # The counter
//!
//! Idleness alone is not a finding — a lane with nothing to do is not a
//! defect. The datum is idleness WITH WORK AVAILABLE:
//!
//!   wall-clock time during which at least one worker lane was idle AND at
//!   least one not-yet-started system had all predecessors FINISHED (by wall
//!   clock, not by `completed` bit) and no conflicting system actually
//!   executing.
//!
//! It is computed from MEASURED per-system start/end timestamps taken inside
//! the system bodies, so it describes the real run, not a model. It is
//! reported split two ways — `idle_pred` and `idle_nopred` — and the split is
//! by the STRUCTURE of the blocked system, not by the cause; see
//! [`blocked_idle`] for why naming it by cause would be a lie.
//!
//! Beside it, three makespans let the barrier be separated from dispatch
//! latency and from machine noise:
//!
//!   * `span` — measured, `max(end) - frame_base`.
//!   * `barrier_sim` — replay of the real gate semantics on the MEASURED
//!     per-system durations, zero dispatch latency.
//!   * `ideal_sim` — the same greedy index-order list schedule with the
//!     barrier removed (a completion retires immediately), zero dispatch
//!     latency.
//!
//! `barrier_share = (barrier_sim - ideal_sim) / span` is the barrier's own
//! share of frame time. `residual = (span - barrier_sim) / span` is everything
//! else: the dispatcher scan, park/unpark latency, and machine load.
//!
//! # The split (KE17 D10) — what `split_sim` is and what it is not
//!
//! `barrier_sim - ideal_sim` is the UPPER BOUND: what the barrier costs, not
//! what removing it from the systems that provably carry nothing would
//! recover. A `Commands`-carrying system keeps its barrier under the split, so
//! the recoverable part is strictly smaller and has to be measured separately.
//! `split_sim` is that measurement: the same greedy index-order list schedule
//! on the same measured durations, retiring a completion INSTANTLY when its
//! `may_defer` bit is clear and holding it to quiescence when set.
//!
//! Reported as `split/model = (barrier_sim - split_sim) / barrier_sim` beside
//! the existing `barrier/model`. Both are structural — same measured
//! durations replayed through models differing in exactly one rule — which is
//! why they move under a percentage point across runs while the criterion wall
//! clock on the same row moves 8.9x. **The wall clock is not a barrier
//! measurement and must not be quoted as one.**
//!
//! # Where the mask comes from
//!
//! A guessed mask makes the number worthless, so no entry is guessed.
//!
//! * `s6` and `s3` stand for NAMED systems. Their masks are read off those
//!   signatures one by one — the table on [`S6_DEFER`] lists all eleven, and
//!   `s3`'s chain is the twelve links of `boyko_physics/src/plugin.rs:531-676`,
//!   none of which takes `Commands`.
//! * `s1`, `s2`, `s4` and `s5` stand for a KIND rather than for named systems,
//!   so their masks come from a census instead: of the 72 distinct function
//!   names registered through `add_system` across the seven system-registering
//!   crates, 68 resolve to a real `fn`, and exactly FOUR take `Commands` —
//!   `snap_apply`, `visibility_sync`, `apply_refcount_deltas`,
//!   `validate_asset_refs`. Every one of the four is a per-entity
//!   apply/reconcile step. No gather, no resolve, no `sync_*_light_gate`, no
//!   solver stage and no sync-out takes one, and those are the roles these four
//!   shapes model. Their masks are therefore all-clear, which makes
//!   `split_sim == ideal_sim` on those rows BY CONSTRUCTION: the row states
//!   "these shapes model systems that carry no deferred payload", and it is
//!   not independent evidence about the split.
//!
//! Because an all-clear mask is a claim rather than a measurement, every row
//! also carries `worst/model` — the recovery under the WORST single-node mask,
//! found by trying each node in turn. That number needs no mask to be believed
//! and answers the question the mask cannot: what one future `Commands`
//! parameter, landing on the wrong node, would cost.
//!
//! The mask is not merely tabulated, either. `build_schedule` gives the marked
//! nodes a `Commands`-carrying body and then asserts the table against the
//! KERNEL's own `Schedule::may_defer` (KE17 D3), so the model's mask is the
//! engine's predicate rather than a second opinion about it.
//!
//! # Separation from KE16 defect A
//!
//! KE16 defect A is "a task spawned from inside a worker lands in that
//! worker's local injector and no sibling polls it" — a nested-spawn defect.
//! NO system body here spawns anything: the bodies busy-wait on `Instant` and
//! touch no `Query` rows, issue no `par_iter`, and queue no `Commands`. Every
//! body is a single leaf task. Defect A therefore has no surface in this
//! harness, and any idleness reported here is the round structure or nothing.
//!
//! # Shapes
//!
//! The barrier is invisible on an equal-cost schedule (every lane reaches
//! quiescence at the same instant), so every shape has DELIBERATELY unequal
//! costs. Shape `s4_wide_control` has no successors at all and is the control:
//! the barrier cannot cost anything there, and a non-zero reading would
//! condemn the instrument rather than the scheduler.
//!
//! Component ids 496-499 are reserved for this bench (492 is ke16,
//! 480-489 swap_remove, 470-479 query_iter; MAX_COMPONENTS = 512).

#![allow(clippy::needless_range_loop)]

#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Once};
use std::time::{Duration, Instant};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder, current_worker_id};
use criterion::{Criterion, criterion_group, criterion_main};

// ── Conflict carriers ───────────────────────────────────────────────────────

macro_rules! conflict_component {
    ($ty:ident, $id:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $ty(u32);
        impl Component for $ty {
            fn component_id() -> ComponentId {
                ComponentId($id)
            }
        }
    };
}

conflict_component!(K0, 496);
conflict_component!(K1, 497);
conflict_component!(K2, 498);
conflict_component!(K3, 499);

fn register_components() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        register_layout::<K0>(496);
        register_layout::<K1>(497);
        register_layout::<K2>(498);
        register_layout::<K3>(499);
    });
}

/// Conflict class meaning "declares no world access at all".
const FREE: u8 = 255;

// ── Timer resolution ────────────────────────────────────────────────────────

// `PARK_TIMEOUT`'s doc in `schedule.rs` is explicit: on Windows the dispatcher's
// `park_timeout` expiry is the process-wide timer quantum, not the 100 µs in
// the source, and KE16 App-12 measured this box at 1 021 µs with
// `timeBeginPeriod(1)` held against 15 296 µs without it. The shipped host
// holds the guard for the whole run (`boyko_app::timer_resolution`), so a bench
// that does NOT hold it is measuring a configuration the engine never ships in
// — every missed wake costs 15 ms instead of 1 ms and buries the datum.
//
// This bench therefore takes the guard by default, exactly as the host does,
// and prints which configuration each row was taken in.
// `BOYKO_KE17_NO_TIMER_GUARD=1` reproduces the unguarded configuration.
#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

fn timer_guard_label() -> &'static str {
    if std::env::var("BOYKO_KE17_NO_TIMER_GUARD").is_ok() {
        return "timer=unguarded";
    }
    #[cfg(windows)]
    {
        // SAFETY: `timeBeginPeriod` is a process-global setting with no
        // pointer arguments and no aliasing contract; the only obligation is a
        // matching `timeEndPeriod`, which the OS performs at process exit. The
        // bench process is short-lived and takes the guard once.
        let rc = unsafe { timeBeginPeriod(1) };
        if rc != 0 {
            return "timer=guard-failed";
        }
    }
    "timer=guarded(1ms)"
}

// ── Timestamp instrument ────────────────────────────────────────────────────

const MAX_SYS: usize = 40;

/// Process-global monotonic origin. All stamps are nanoseconds from here.
static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static START_NS: [AtomicU64; MAX_SYS] = [ZERO; MAX_SYS];
static END_NS: [AtomicU64; MAX_SYS] = [ZERO; MAX_SYS];
/// Per-system busy-wait budget for the shape currently installed.
static COST_NS: [AtomicU64; MAX_SYS] = [ZERO; MAX_SYS];
/// Worker id the body ran on. Lane identity, so per-lane gaps can be read off
/// the timeline and the "how many lanes were there really" question is a
/// receipt rather than an assumption.
static LANE: [AtomicU64; MAX_SYS] = [ZERO; MAX_SYS];

/// Busy-wait `ns` nanoseconds. Never sleeps: a serialised wave must show up as
/// wall-clock occupancy, not as an idle lane.
#[inline(never)]
fn spin_ns(ns: u64) {
    if ns == 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_nanos(ns);
    let mut acc: u64 = 0;
    while Instant::now() < deadline {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    black_box(acc);
}

/// The whole body of every system in this bench. One leaf task, no spawn.
#[inline(never)]
fn body(i: usize) {
    let s = ORIGIN.elapsed().as_nanos() as u64;
    spin_ns(COST_NS[i].load(Ordering::Relaxed));
    let e = ORIGIN.elapsed().as_nanos() as u64;
    START_NS[i].store(s, Ordering::Relaxed);
    END_NS[i].store(e, Ordering::Relaxed);
    LANE[i].store(current_worker_id() as u64, Ordering::Relaxed);
}

// ── Shapes ──────────────────────────────────────────────────────────────────

/// Which retire rule a [`simulate`] run replays.
///
/// Three, not two, so `Split` cannot be spelled without the mask it is
/// meaningless without.
#[derive(Clone, Copy)]
enum RetirePolicy<'a> {
    /// The barrier removed: a completion retires the instant it lands. The
    /// latency-free floor, `ideal_sim`.
    Immediate,
    /// The gate at `schedule.rs:623` replayed: nothing retires until every
    /// system in flight has finished. `barrier_sim`.
    Quiescence,
    /// KE17 D10 — the split window. A completion whose `may_defer` bit is
    /// CLEAR retires the instant it lands; one whose bit is SET is held to
    /// quiescence with the rest. Clearing a bit early also lowers the
    /// quiescence bar for the systems still held, exactly as clearing the real
    /// `running` bit would.
    Split(&'a [bool]),
}

struct Shape {
    name: &'static str,
    n: usize,
    /// Intended busy-wait per system, nanoseconds.
    cost_ns: &'static [u64],
    /// Conflict class per system; equal classes conflict, `FREE` conflicts
    /// with nothing. Realised as `Query<&mut Kx>` (write-write on one column).
    class: &'static [u8],
    /// Ordering edges `(from, to)`: `from` must complete before `to` starts.
    /// `from < to` always — the builder needs the predecessor key first.
    edges: &'static [(usize, usize)],
    /// KE17 D10 — per-system `may_defer`: `true` iff the REAL system this node
    /// stands for takes a `Commands` parameter. NOT invented; see the module's
    /// "Where the mask comes from" section for the derivation of every entry,
    /// and `build_schedule`, which registers a `Commands`-carrying body on
    /// exactly these nodes and then asserts the mask against the KERNEL's own
    /// `Schedule::may_defer` rather than against this table.
    may_defer: &'static [bool],
    /// One line on what real engine structure this stands in for.
    stands_for: &'static str,
}

impl Shape {
    fn conflicts(&self, a: usize, b: usize) -> bool {
        a != b && self.class[a] != FREE && self.class[a] == self.class[b]
    }
    fn preds_ok(&self, i: usize, done: &dyn Fn(usize) -> bool) -> bool {
        self.edges
            .iter()
            .filter(|(_, to)| *to == i)
            .all(|(from, _)| done(*from))
    }
}

const US: u64 = 1_000;

// S1 — four head systems, each with one successor whose cost mirrors its
// predecessor. The classic "the second wave real work is gated by the first
// wave slowest member".
static S1_COST: [u64; 8] = [
    400 * US,
    20 * US,
    20 * US,
    20 * US,
    20 * US,
    400 * US,
    400 * US,
    400 * US,
];
static S1_CLASS: [u8; 8] = [FREE; 8];
static S1_EDGES: [(usize, usize); 4] = [(0, 4), (1, 5), (2, 6), (3, 7)];

// S2 — twelve heads, twelve tails, one head far longer than the rest.
static S2_COST: [u64; 24] = [
    300 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    6 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
    25 * US,
];
static S2_CLASS: [u8; 24] = [FREE; 24];
static S2_EDGES: [(usize, usize); 12] = [
    (0, 12),
    (1, 13),
    (2, 14),
    (3, 15),
    (4, 16),
    (5, 17),
    (6, 18),
    (7, 19),
    (8, 20),
    (9, 21),
    (10, 22),
    (11, 23),
];

// S3 — one long system beside a serial chain of short ones. This is the worst
// case the mechanism admits: every link of the chain needs its predecessor
// RETIRED, and retirement waits for the long system.
static S3_COST: [u64; 21] = [
    400 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
    12 * US,
];
static S3_CLASS: [u8; 21] = [FREE; 21];
static S3_EDGES: [(usize, usize); 19] = [
    (1, 2),
    (2, 3),
    (3, 4),
    (4, 5),
    (5, 6),
    (6, 7),
    (7, 8),
    (8, 9),
    (9, 10),
    (10, 11),
    (11, 12),
    (12, 13),
    (13, 14),
    (14, 15),
    (15, 16),
    (16, 17),
    (17, 18),
    (18, 19),
    (19, 20),
];

// S4 — CONTROL. Sixteen independent systems, wildly unequal, NO successors and
// NO conflicts. The barrier has nothing to delay; the reading must be ~0.
static S4_COST: [u64; 16] = [
    200 * US,
    5 * US,
    150 * US,
    5 * US,
    120 * US,
    8 * US,
    90 * US,
    8 * US,
    70 * US,
    10 * US,
    50 * US,
    10 * US,
    30 * US,
    12 * US,
    20 * US,
    12 * US,
];
static S4_CLASS: [u8; 16] = [FREE; 16];
static S4_EDGES: [(usize, usize); 0] = [];

// S5 — narrow conflict graph: the six heads sit in three write-write classes,
// so at most three of them can run at once no matter how many lanes exist.
// Successors are conflict-free.
static S5_COST: [u64; 12] = [
    200 * US,
    20 * US,
    180 * US,
    20 * US,
    160 * US,
    20 * US,
    60 * US,
    60 * US,
    60 * US,
    60 * US,
    60 * US,
    60 * US,
];
static S5_CLASS: [u8; 12] = [0, 0, 1, 1, 2, 2, FREE, FREE, FREE, FREE, FREE, FREE];
static S5_EDGES: [(usize, usize); 6] = [(0, 6), (1, 7), (2, 8), (3, 9), (4, 10), (5, 11)];

// S6 — the engine own Main-schedule shape (EnginePlugins, boyko_app
// plugins.rs:617-708 plus LightingPlugin `collect_lights`):
//
//   0 sync_instance_model_cols  long,  writes InstanceModelCol      (class 0)
//   1 snap_apply                short, no edge                      (FREE)
//   2 sync_ssao_light_gate      tiny,  writes LightingConfig        (class 1)
//   3 sync_sv0_light_gate       tiny,  writes LightingConfig        (class 1)
//   4 sync_cluster_light_gate   tiny,  writes LightingConfig        (class 1)
//   5 gather_shadow_casters     long,  after 0                      (class 2)
//   6 sync_csm_light_gate       tiny,  after 5                      (class 1)
//   7 reduce_caster_bounds      short, after 5                      (class 3)
//   8 sync_punctual_light_gate  tiny,  after 5                      (class 1)
//   9 collect_lights            med,   after 2,3,4                  (class 1)
//  10 gather_mesh_draws         long,  after 0 and 1                (class 4)
static S6_COST: [u64; 11] = [
    600 * US,
    30 * US,
    US,
    US,
    US,
    300 * US,
    US,
    15 * US,
    US,
    80 * US,
    500 * US,
];
static S6_CLASS: [u8; 11] = [0, FREE, 1, 1, 1, 2, 1, 3, 1, 1, 4];
static S6_EDGES: [(usize, usize); 7] = [(0, 5), (5, 6), (5, 7), (5, 8), (2, 9), (3, 9), (0, 10)];

// ── The masks (KE17 D10) ────────────────────────────────────────────────────
//
// Read off the real signatures, node by node, never invented. See the module
// header's "Where the mask comes from" for the derivation and the census the
// KIND shapes rest on.

/// `s1` — a wave of unequal gathers each feeding one consumer. A KIND shape:
/// no gather or consumer in the census carries `Commands`.
static S1_DEFER: [bool; 8] = [false; 8];
/// `s2` — many cheap systems beside one dominant one. Same census class.
static S2_DEFER: [bool; 24] = [false; 24];
/// `s3` — the physics Fixed chain beside one long render gather. NAMED: the
/// twelve links registered at `boyko_physics/src/plugin.rs:531-676`
/// (`sync_transform_to_body` → … → `sync_body_to_transform`) take
/// `Query`/`Res`/`ResMut` only; zero `Commands` between them, and the long
/// sibling stands for a render gather, which carries none either.
static S3_DEFER: [bool; 21] = [false; 21];
/// `s4` — the CONTROL. All-clear, so `split_sim == ideal_sim == barrier_sim`
/// here and the row stays the instrument check it was.
static S4_DEFER: [bool; 16] = [false; 16];
/// `s5` — heads contending on three shared columns. KIND shape; the engine's
/// contending heads are the five `sync_*_light_gate` bridges, none of which
/// takes `Commands`.
static S5_DEFER: [bool; 12] = [false; 12];
/// `s6` — NAMED, and the row the decision rests on. Eleven nodes, eleven real
/// systems, read one by one:
///
/// | # | system | params | `Commands`? |
/// |---|---|---|---|
/// | 0 | `sync_instance_model_cols` (`instance_model.rs:110`) | one `Query` | no |
/// | 1 | `snap_apply` (`snap_interpolation.rs:124`) | `Commands` + `Query` | **YES** |
/// | 2 | `sync_ssao_light_gate` (`ssao_config.rs:272`) | 2 `Res` + 2 `ResMut` | no |
/// | 3 | `sync_sv0_light_gate` (`light.rs:1139`) | `Res` + 2 `ResMut` | no |
/// | 4 | `sync_cluster_light_gate` (`light.rs:1005`) | 2 `Res` + 2 `ResMut` | no |
/// | 5 | `gather_shadow_casters` (`csm_caster.rs:187`) | `Query` + `NonSendRes` + `ResMut` | no |
/// | 6 | `sync_csm_light_gate` (`csm_caster.rs:529`) | 2 `Res` + 2 `ResMut` | no |
/// | 7 | `reduce_caster_bounds` (`csm_caster.rs:449`) | 3 `Res` + `NonSendRes` + `ResMut` | no |
/// | 8 | `sync_punctual_light_gate` (`shadow_atlas.rs:1087`) | 2 `Res` + 2 `ResMut` | no |
/// | 9 | `collect_lights` (`light_system.rs:582`) | 8 `Query` + 2 `Res` + 3 `ResMut` | no |
/// | 10 | `gather_mesh_draws` (`mesh_draw.rs:1238`/`:1356`) | `Query` + `NonSendRes` + `ResMut` | no |
///
/// Exactly one of eleven, which is what the design claimed and what this pass
/// re-read rather than trusted.
static S6_DEFER: [bool; 11] = [
    false, true, false, false, false, false, false, false, false, false, false,
];

static SHAPES: &[Shape] = &[
    Shape {
        name: "s1_few_long_uneven",
        n: 8,
        cost_ns: &S1_COST,
        class: &S1_CLASS,
        edges: &S1_EDGES,
        may_defer: &S1_DEFER,
        stands_for: "a wave of unequal gathers each feeding one consumer",
    },
    Shape {
        name: "s2_many_short_uneven",
        n: 24,
        cost_ns: &S2_COST,
        class: &S2_CLASS,
        edges: &S2_EDGES,
        may_defer: &S2_DEFER,
        stands_for: "many cheap systems beside one dominant one",
    },
    Shape {
        name: "s3_chain_beside_long",
        n: 21,
        cost_ns: &S3_COST,
        class: &S3_CLASS,
        edges: &S3_EDGES,
        may_defer: &S3_DEFER,
        stands_for: "the physics stage chain running beside one long render gather",
    },
    Shape {
        name: "s4_wide_control",
        n: 16,
        cost_ns: &S4_COST,
        class: &S4_CLASS,
        edges: &S4_EDGES,
        may_defer: &S4_DEFER,
        stands_for: "CONTROL - no successors, the barrier cannot cost anything",
    },
    Shape {
        name: "s5_narrow_conflict",
        n: 12,
        cost_ns: &S5_COST,
        class: &S5_CLASS,
        edges: &S5_EDGES,
        may_defer: &S5_DEFER,
        stands_for: "heads contending on three shared columns",
    },
    Shape {
        name: "s6_engine_main_like",
        n: 11,
        cost_ns: &S6_COST,
        class: &S6_CLASS,
        edges: &S6_EDGES,
        may_defer: &S6_DEFER,
        stands_for: "EnginePlugins Main schedule (plugins.rs:617-708 + collect_lights)",
    },
];

// ── Schedule construction ───────────────────────────────────────────────────

fn build_schedule(shape: &Shape, world: &mut EcsMaster, pool: Arc<ThreadPool>) -> Schedule {
    let mut b = ScheduleBuilder::new(pool);
    let mut keys: Vec<Option<_>> = Vec::new();
    for i in 0..shape.n {
        let idx = i;
        // The `Commands` arms carry the mask into the REAL system signatures:
        // a marked node's param chain answers `SystemParam::HAS_DEFERRED =
        // true` for the same reason the system it stands for does, so the
        // kernel's own predicate can be checked against the table below rather
        // than the table being taken on trust. `Commands` declares no access
        // (`commands.rs`'s `init_access` is empty), so the conflict structure
        // every other column depends on is untouched.
        let mut cfg = match (shape.may_defer[i], shape.class[i]) {
            (false, 0) => b.add_system(move |_q: Query<&mut K0>| body(idx)),
            (false, 1) => b.add_system(move |_q: Query<&mut K1>| body(idx)),
            (false, 2) => b.add_system(move |_q: Query<&mut K2>| body(idx)),
            (false, 3) => b.add_system(move |_q: Query<&mut K3>| body(idx)),
            (false, _) => b.add_system(move || body(idx)),
            (true, 0) => b.add_system(move |_c: Commands, _q: Query<&mut K0>| body(idx)),
            (true, 1) => b.add_system(move |_c: Commands, _q: Query<&mut K1>| body(idx)),
            (true, 2) => b.add_system(move |_c: Commands, _q: Query<&mut K2>| body(idx)),
            (true, 3) => b.add_system(move |_c: Commands, _q: Query<&mut K3>| body(idx)),
            (true, _) => b.add_system(move |_c: Commands| body(idx)),
        };
        for &(from, to) in shape.edges {
            if to == i {
                let k = keys[from].expect("KE17: edges must point forward (from < to)");
                cfg = cfg.after(k);
            }
        }
        keys.push(Some(cfg.key()));
    }
    let sched = b.build(world);

    // The mask, checked against the kernel instead of against itself (KE17 D3
    // `Schedule::may_defer`). Asserted as a POPULATION COUNT, not per index:
    // `kahn_topological_sort` breaks ties from a FIFO ready queue, so a shape
    // with any edge at all comes out permuted (`s1`'s roots 0..3 and its four
    // tails interleave), and `Schedule::may_defer` is indexed by post-topo
    // position while every other array in this file is indexed by the shape's
    // own numbering. The count is permutation-invariant and is what the claim
    // needs: the engine agrees with this table on HOW MANY of the shape's
    // systems carry a deferred payload, and which ones is fixed by the match
    // above.
    let want = shape.may_defer.iter().filter(|&&m| m).count();
    let got = (0..shape.n).filter(|&i| sched.may_defer(i)).count();
    assert_eq!(
        got, want,
        "KE17 {}: the kernel's may_defer population ({got}) disagrees with the          shape's mask ({want}) — the model would be pricing a schedule the          engine did not build",
        shape.name,
    );
    sched
}

// ── Per-frame analysis ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct FrameStats {
    span_ns: u64,
    ideal_ns: u64,
    barrier_ns: u64,
    /// KE17 D10 — makespan under the split window with the shape's own mask.
    split_ns: u64,
    /// KE17 D10 — makespan under the WORST single-node mask. Needs no mask to
    /// be believed: it is the max over every one-hot mask the shape admits.
    split_worst_ns: u64,
    idle_nopred_ns: u64,
    idle_pred_ns: u64,
    max_inflight: usize,
}

/// Wall time during which at least one lane was idle AND some not-yet-started
/// system was legally startable (predecessors finished, no conflicting system
/// executing). Computed purely from the measured timeline.
///
/// Returned SPLIT BY THE STRUCTURE OF THE BLOCKED SYSTEM. It is deliberately
/// NOT split by cause: "idle with work available" is a joint counter, and the
/// barrier reaches a system through TWO different checks in
/// `try_dispatch_ready`, so no purely observational split can name the cause.
///
///   `.0` — `nopred`: at least one startable system has NO predecessor at all.
///          Such a system was ready from the first dispatch round, so the
///          `pred_remaining` path (`schedule.rs:1022`) is NOT what holds it.
///          Two things still can: the barrier's OTHER path — its conflict
///          bitset intersecting a `running` bit that a finished sibling has
///          not had cleared (`:1026`) — or the pool (a spawned task sitting in
///          an injector while a lane idles). `s4_wide_control` has no edges
///          and no conflicts, so its reading is the pool + dispatch floor and
///          nothing else; that is the number to subtract before reading this
///          column as a barrier cost anywhere else.
///   `.1` — `pred`: EVERY startable system has at least one predecessor, and
///          every one of those predecessors has FINISHED by wall clock. This
///          isolates the `pred_remaining` path: the work is legal, a lane is
///          free, and the only thing that can hold it is a `running` bit that
///          has not been cleared.
fn blocked_idle(shape: &Shape, workers: usize, s: &[u64], e: &[u64]) -> (u64, u64) {
    let n = shape.n;
    let mut ts: Vec<u64> = Vec::with_capacity(2 * n + 1);
    ts.push(0);
    for i in 0..n {
        ts.push(s[i]);
        ts.push(e[i]);
    }
    ts.sort_unstable();
    ts.dedup();

    let mut root = 0u64;
    let mut succ = 0u64;
    for w in ts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let busy = (0..n).filter(|&i| s[i] <= a && a < e[i]).count();
        if busy >= workers {
            continue;
        }
        let mut any_root = false;
        let mut any_succ = false;
        for i in 0..n {
            if s[i] <= a {
                continue;
            }
            if !shape.preds_ok(i, &|p| e[p] <= a) {
                continue;
            }
            if (0..n).any(|j| shape.conflicts(i, j) && s[j] <= a && a < e[j]) {
                continue;
            }
            if shape.edges.iter().any(|(_, to)| *to == i) {
                any_succ = true;
            } else {
                any_root = true;
            }
        }
        if any_root {
            root += b - a;
        } else if any_succ {
            succ += b - a;
        }
    }
    (root, succ)
}

fn max_inflight(shape: &Shape, s: &[u64], e: &[u64]) -> usize {
    let n = shape.n;
    let mut best = 0;
    for i in 0..n {
        let t = s[i];
        let c = (0..n).filter(|&j| s[j] <= t && t < e[j]).count();
        best = best.max(c);
    }
    best
}

/// Replay the schedule on measured durations under one of the three retire
/// rules of [`RetirePolicy`]. Every run is the same greedy index-order list
/// schedule with zero dispatch latency over the same DAG, the same conflict
/// classes and the same worker count, so any difference between two of them is
/// the retire rule and nothing else.
fn simulate(shape: &Shape, workers: usize, d: &[u64], policy: RetirePolicy<'_>) -> u64 {
    let n = shape.n;
    let mut completed = vec![false; n];
    let mut dispatched = vec![false; n];
    let mut end_at = vec![u64::MAX; n];
    let mut marked: Vec<usize> = Vec::with_capacity(n);
    let mut done = 0usize;
    let mut t = 0u64;
    let mut makespan = 0u64;
    let mut guard = 0usize;

    while done < n {
        guard += 1;
        if guard > 100_000 {
            return u64::MAX;
        }

        // Step 1 — the EARLY release. `Immediate` retires every finished
        // system (the barrier removed); `Split` retires only those whose
        // `may_defer` bit is clear; `Quiescence` retires none here. Running
        // this BEFORE step 2 is what makes the model faithful: clearing a
        // `running` bit early also lowers the quiescence bar for the systems
        // still held, which is what the real bit-clear would do.
        if !matches!(policy, RetirePolicy::Quiescence) {
            let mut k = 0;
            while k < marked.len() {
                let i = marked[k];
                let releasable = match policy {
                    RetirePolicy::Immediate => true,
                    RetirePolicy::Split(mask) => !mask[i],
                    RetirePolicy::Quiescence => false,
                };
                if releasable && end_at[i] <= t {
                    completed[i] = true;
                    done += 1;
                    marked.swap_remove(k);
                } else {
                    k += 1;
                }
            }
        }
        // Step 2 — the QUIESCENCE drain, the gate at `schedule.rs:623` replayed
        // over whatever is still marked.
        if !matches!(policy, RetirePolicy::Immediate) {
            let pending = marked.iter().filter(|&&i| end_at[i] <= t).count();
            if pending > 0 && pending == marked.len() {
                for &i in &marked {
                    completed[i] = true;
                    done += 1;
                }
                marked.clear();
            }
        }
        if done == n {
            break;
        }

        loop {
            let inflight = marked.iter().filter(|&&i| end_at[i] > t).count();
            if inflight >= workers {
                break;
            }
            let mut picked = None;
            for i in 0..n {
                if dispatched[i] {
                    continue;
                }
                if !shape.preds_ok(i, &|p| completed[p]) {
                    continue;
                }
                if marked.iter().any(|&j| shape.conflicts(i, j)) {
                    continue;
                }
                picked = Some(i);
                break;
            }
            match picked {
                Some(i) => {
                    dispatched[i] = true;
                    end_at[i] = t + d[i];
                    makespan = makespan.max(end_at[i]);
                    marked.push(i);
                }
                None => break,
            }
        }

        match marked.iter().map(|&i| end_at[i]).filter(|&x| x > t).min() {
            Some(nt) => t = nt,
            None => {
                if marked.is_empty() {
                    // Nothing running and nothing dispatchable: the shape is
                    // not a DAG, or the model is wrong. Fail loudly.
                    return u64::MAX;
                }
                if matches!(policy, RetirePolicy::Immediate) {
                    // Nothing is ever held under `Immediate`, so a marked set
                    // with no future end is a broken model, not a barrier.
                    return u64::MAX;
                }
                // A rule that HOLDS completions: every marked system has
                // finished; the next loop turn is the drain that releases the
                // successors.
            }
        }
    }
    makespan
}

fn run_frame_and_analyse(
    shape: &Shape,
    workers: usize,
    sched: &mut Schedule,
    world: &mut EcsMaster,
) -> FrameStats {
    for i in 0..shape.n {
        START_NS[i].store(0, Ordering::Relaxed);
        END_NS[i].store(0, Ordering::Relaxed);
    }
    let base = ORIGIN.elapsed().as_nanos() as u64;
    sched.run(world);

    let mut s = vec![0u64; shape.n];
    let mut e = vec![0u64; shape.n];
    let mut d = vec![0u64; shape.n];
    for i in 0..shape.n {
        let si = START_NS[i].load(Ordering::Relaxed);
        let ei = END_NS[i].load(Ordering::Relaxed);
        assert!(
            ei >= si && si >= base,
            "KE17: system {i} did not stamp this frame",
        );
        s[i] = si - base;
        e[i] = ei - base;
        d[i] = e[i] - s[i];
    }
    let span = *e.iter().max().expect("non-empty shape");
    let ideal = simulate(shape, workers, &d, RetirePolicy::Immediate);
    let barrier = simulate(shape, workers, &d, RetirePolicy::Quiescence);
    let split = simulate(shape, workers, &d, RetirePolicy::Split(shape.may_defer));

    // The mask-free bound: the worst a single `Commands` parameter could do to
    // this shape, whichever node it landed on. Answers the one question the
    // mask cannot answer about itself.
    let mut one_hot = vec![false; shape.n];
    let mut split_worst = split;
    for i in 0..shape.n {
        one_hot[i] = true;
        let m = simulate(shape, workers, &d, RetirePolicy::Split(&one_hot));
        one_hot[i] = false;
        split_worst = split_worst.max(m);
    }

    assert!(
        ideal != u64::MAX
            && barrier != u64::MAX
            && split != u64::MAX
            && split_worst != u64::MAX,
        "KE17: simulation deadlocked on shape {}",
        shape.name,
    );
    // NOT asserted: that `ideal <= split <= barrier`. It is true of every
    // reading this bench has taken, but it is not guaranteed — these are GREEDY
    // list schedules, and releasing work earlier can make a greedy scheduler
    // pick a different system first and finish later (Graham's anomaly). An
    // assert here would turn a real, reportable model artefact into a crash; a
    // negative `split/model` in the row is how such a frame would announce
    // itself.
    let (root, succ) = blocked_idle(shape, workers, &s, &e);
    FrameStats {
        span_ns: span,
        ideal_ns: ideal,
        barrier_ns: barrier,
        split_ns: split,
        split_worst_ns: split_worst,
        idle_nopred_ns: root,
        idle_pred_ns: succ,
        max_inflight: max_inflight(shape, &s, &e),
    }
}

fn median(v: &mut [u64]) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

const ANALYSIS_FRAMES: usize = 61;

fn analyse(
    shape: &Shape,
    workers: usize,
    sched: &mut Schedule,
    world: &mut EcsMaster,
    timer: &str,
) {
    // Warm the pool and page in every column before the recorded run.
    for _ in 0..8 {
        run_frame_and_analyse(shape, workers, sched, world);
    }
    let mut span = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut ideal = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut barrier = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut split = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut split_worst = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut b_root = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut b_succ = Vec::with_capacity(ANALYSIS_FRAMES);
    let mut infl = 0usize;
    for _ in 0..ANALYSIS_FRAMES {
        let f = run_frame_and_analyse(shape, workers, sched, world);
        span.push(f.span_ns);
        ideal.push(f.ideal_ns);
        barrier.push(f.barrier_ns);
        split.push(f.split_ns);
        split_worst.push(f.split_worst_ns);
        b_root.push(f.idle_nopred_ns);
        b_succ.push(f.idle_pred_ns);
        infl = infl.max(f.max_inflight);
    }
    if std::env::var("BOYKO_KE17_TIMELINE").is_ok() {
        // One extra frame, printed row by row. `lane` is the worker id the body
        // ran on, so a reader can see for himself how many lanes carried work
        // and where the dead time between two bodies on the SAME lane sits.
        run_frame_and_analyse(shape, workers, sched, world);
        let base_min = (0..shape.n)
            .map(|i| START_NS[i].load(Ordering::Relaxed))
            .min()
            .expect("non-empty shape");
        eprintln!("KE17-TL | {} W={} (ns, relative to first body start)", shape.name, workers);
        let mut order: Vec<usize> = (0..shape.n).collect();
        order.sort_by_key(|&i| START_NS[i].load(Ordering::Relaxed));
        for i in order {
            let s = START_NS[i].load(Ordering::Relaxed) - base_min;
            let e = END_NS[i].load(Ordering::Relaxed) - base_min;
            eprintln!(
                "KE17-TL |   sys {:>2} lane {:>3} start {:>9} end {:>9} dur {:>9} (intent {})",
                i,
                LANE[i].load(Ordering::Relaxed) as i64,
                s,
                e,
                e - s,
                shape.cost_ns[i],
            );
        }
    }

    let sp = median(&mut span);
    let id = median(&mut ideal);
    let ba = median(&mut barrier);
    let sl = median(&mut split);
    let sw = median(&mut split_worst);
    let br = median(&mut b_root);
    let bs = median(&mut b_succ);
    let barrier_share = (ba.saturating_sub(id) as f64) / (sp as f64) * 100.0;
    let barrier_share_model = (ba.saturating_sub(id) as f64) / (ba as f64) * 100.0;
    // KE17 D10 — what the SPLIT recovers, against what the barrier costs. The
    // two are equal exactly when every held node was free anyway; the gap
    // between them is the part of the barrier that a `Commands`-carrying
    // system keeps.
    let split_share_model = ((ba as f64) - (sl as f64)) / (ba as f64) * 100.0;
    let worst_share_model = ((ba as f64) - (sw as f64)) / (ba as f64) * 100.0;
    let defer_n = shape.may_defer.iter().filter(|&&m| m).count();
    let residual = (sp.saturating_sub(ba) as f64) / (sp as f64) * 100.0;
    eprintln!(
        "KE17 | {:<20} | W={:<2} | {} | span {:>8.1} | ideal {:>7.1} | barrier_sim {:>7.1} \
         | split_sim {:>7.1} (defer {}/{}) | worst_sim {:>7.1} \
         | barrier/span {:>6.2}% | barrier/model {:>6.2}% | split/model {:>6.2}% \
         | worst/model {:>6.2}% | idle_pred {:>7.1} ({:>5.2}%) \
         | idle_nopred {:>8.1} ({:>5.2}%) | residual {:>6.2}% | max_inflight {}",
        shape.name,
        workers,
        timer,
        sp as f64 / 1000.0,
        id as f64 / 1000.0,
        ba as f64 / 1000.0,
        sl as f64 / 1000.0,
        defer_n,
        shape.n,
        sw as f64 / 1000.0,
        barrier_share,
        barrier_share_model,
        split_share_model,
        worst_share_model,
        bs as f64 / 1000.0,
        (bs as f64) / (sp as f64) * 100.0,
        br as f64 / 1000.0,
        (br as f64) / (sp as f64) * 100.0,
        residual,
        infl,
    );
}

// ── Criterion entry ─────────────────────────────────────────────────────────

fn bench_apply_window(c: &mut Criterion) {
    register_components();
    LazyLock::force(&ORIGIN);
    let timer = timer_guard_label();

    let mut group = c.benchmark_group("ke17_apply_window");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(400));
    group.measurement_time(Duration::from_secs(2));

    eprintln!("KE17 | shape catalogue (what each stands for):");
    for shape in SHAPES {
        let defer: Vec<usize> = (0..shape.n).filter(|&i| shape.may_defer[i]).collect();
        eprintln!(
            "KE17 |   {:<20} n={:<3} may_defer={:?} {}",
            shape.name, shape.n, defer, shape.stands_for
        );
    }

    for &workers in &[4usize, 16usize] {
        for shape in SHAPES {
            assert!(shape.n <= MAX_SYS, "KE17: shape exceeds MAX_SYS");
            for i in 0..shape.n {
                COST_NS[i].store(shape.cost_ns[i], Ordering::Relaxed);
            }
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            let mut world = EcsMaster::new();
            let mut sched = build_schedule(shape, &mut world, Arc::clone(&pool));

            analyse(shape, workers, &mut sched, &mut world, timer);

            group.bench_function(format!("{}/w{}", shape.name, workers), |b| {
                b.iter(|| sched.run(&mut world));
            });

            drop(sched);
            drop(world);
            drop(pool);
        }
    }
    group.finish();
}

criterion_group!(benches, bench_apply_window);
criterion_main!(benches);
