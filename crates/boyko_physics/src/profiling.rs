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
//! | [`PHYS_SLEEP_CLASSIFY`] | L10's sleep-skip classification in the colored broadphase's prologue | 1 on a colored step whose sleep-skip mode is `Sets`, else 0 |
//! | [`PHYS_NP_DISPATCH`] | the parallel narrowphase's stage growth, spawn and join | 1 when it dispatched, else 0 |
//! | [`PHYS_NP_COMPACT`] | its join of the chunks' runs into the two streams | 1 when it dispatched, else 0 |
//! | [`PHYS_NP_AXIS_COMMIT`] | its serial replay of the axis writes | 1 when it dispatched, else 0 |
//! | [`PHYS_BP_VERIFY`] | the tree broadphase's verify pass and maintenance | 1 on a tree-path step, else 0 |
//! | [`PHYS_BP_BUILD`] | its active-tree build | 1 on a tree-path step, else 0 |
//! | [`PHYS_BP_QUERY`] | its queries and Wide-row loops | 1 on a tree-path step, else 0 |
//! | [`PHYS_BP_ASSEMBLE`] | its pair assembly | 1 on a tree-path step, else 0 |
//! | [`PHYS_SB_BODIES`] | `build_bodies` + the warm cursor's remap classification | 1 |
//! | [`PHYS_SB_PA`] | the build's P-a: the sizing and the tags | 1 |
//! | [`PHYS_SB_PB`] | the build's P-b: the cohort layout | 1 |
//! | [`PHYS_SB_PC`] | the build's P-c: the warm source search and the cohort fill | 1 |
//!
//! The four `phys_sb_*` spans (W8S, `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md` §7)
//! split [`PHYS_SOLVE_BUILD`]: they nest inside it and do not overlap, so `phys_solve_build`
//! equals their sum plus a residue (the build's asserts and its statistics). They open on the
//! no-awake fast path as well, which still builds. Since S4 the warm source search runs after
//! P-b (the layout reads only the tags' counts and frozen flags), so it is P-c's, not P-a's:
//! commit (1) of `u/phys-w8s` measured it inside `phys_sb_pa`, commit (4) inside `phys_sb_pc`.
//! An armed setup wave closes `phys_sb_pc` at its join.
//!
//! On a step of the colored solve's no-awake fast path (L10 C3a: sleeping on, no dynamic row
//! awake and no contact point laid out) the substep loop, the restitution pass and the freeze
//! capture and restore do not run: [`PHYS_GRAVITY`], [`PHYS_WARM_APPLY`], [`PHYS_INTEGRATE`],
//! [`PHYS_PASS_BIASED`], [`PHYS_PASS_RELAX`], the two color spans, [`PHYS_RESTITUTION`] and
//! [`PHYS_SLEEP_FREEZE`] read 0 on it, and `ColoredSoftStepSolver::fast_path_steps` counts it.
//! A reader derives the path from the world — the awake mask and the recomputed slots — never
//! from that counter.
//!
//! The narrowphase dispatches when `parallel_narrowphase` is on, a pool of at least two workers
//! is attached and the pair count yields at least two chunks — the chunk count
//! [`PHYS_NP_CHUNKS`] reports, recomputed from the exported `NP_*` constants
//! (`narrowphase/dispatch.rs`). Its three zones open only after that decision, so a step that
//! runs the serial loop opens none of them.
//!
//! A **tree-path step** is one with `PhysicsConfig::broadphase == Tree` and more rows than
//! `BroadphaseTree::brute_max_rows()`; the `AllPairs` and `Grid` arms, and the Tree's brute
//! path at or below that count, open none of the four `phys_bp_*` spans and emit none of the
//! three `phys_bp_*` structural counters (`broadphase_tree/mod.rs`). A reader derives the path
//! from the configuration, the row count and `brute_max_rows()` — never from the implementation.
//!
//! The sleep-skip's step mode is `Off` when `PhysicsConfig::sleeping` is off and
//! `PhysicsConfig::sleep_skip` otherwise; only the colored broadphase records it (a world without
//! the colored pipeline has no sleep-skip), and a reader derives it from the configuration.
//!
//! Each of the ten step counters is emitted exactly once per step, including when its value is
//! zero, so its per-step sample count is 1 and its per-step `total` is the value; the three
//! tree counters are emitted once per tree-path step and not otherwise, and the sleep-skip's
//! counter once per step whose classification ran (the [`PHYS_SLEEP_CLASSIFY`] span's steps):
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
//! | [`PHYS_BP_QUERIED`] | rows the tree broadphase queried or looped (`\|Q\| + \|Wide\|`), tree path only |
//! | [`PHYS_BP_MEMBERS`] | rows in its persistent sets (`\|S\| + \|Z\|`), tree path only |
//! | [`PHYS_BP_REBUILDS`] | its admissions and compactions this step, tree path only |
//! | [`PHYS_NP_REUSED`] | box pairs whose output came from their contact-reuse record (L9b) |
//! | [`PHYS_NP_SEP_HITS`] | box pairs their carried separating axis rejected, the SAT not run (L9a) |
//! | [`PHYS_NP_FULL`] | box pairs whose full collision ran (neither of the two above) |
//! | [`PHYS_SLEEP_HELD`] | rows L10's sleep-skip holds after the broadphase |
//!
//! # The W8S telemetry (armed only)
//!
//! Thirty-five more counters read the dispatch machinery itself (W8S instruments 2 to 4,
//! `levers/scaling/01-DESIGN.md` §7, the seven its review added, §10.1b, and S4's two setup
//! counters, §10.4 and §10.6). Each is pushed from the thread that called the solve or the
//! narrowphase, never from a worker's task (ruling 3, 2026-09-26):
//!
//! | Counter | Samples | Value per sample |
//! |---|---|---|
//! | [`PHYS_WAVE_RAMP`] | 1 per solving step | Σ over the step's dispatched solve waves of the ticks from the scope's opening to the first task a thread other than the caller started (0 for a wave whose caller ran every task) |
//! | [`PHYS_WAVE_TAIL`] | 1 per solving step | Σ over those waves of the ticks from the caller's own last task end (or the end of its spawn loop, whichever is later) to the join's return |
//! | [`PHYS_WAVE_INFLIGHT`] | 1 per solving step | Σ over those waves of the most task intervals open at one instant (KE16's `max_in_flight`) |
//! | [`PHYS_WAVE_LANES`] | 1 per solving step | Σ over those waves of the distinct threads that ran a task |
//! | [`PHYS_WAVE_OVERFLOW`] | 1 per solving step | tasks of the step's waves past [`WAVE_STAMP_CAPACITY`], which went unstamped; nonzero voids the row |
//! | [`PHYS_COLOR_SCOPES`] | 1 per solving step | colour scopes the step opened (a barrier is a scope's join, so this is the barrier count too) |
//! | [`PHYS_COLOR_TASKS`] | 1 per solving step | tasks those scopes spawned |
//! | [`PHYS_ROUTE_WORKER`] / [`PHYS_ROUTE_EXTERNAL`] | 1 each per solving step | the step's solve scopes whose joiner was a worker of the pool / anything else |
//! | `phys_hist_colors_*` / `phys_hist_slots_*` ([`HIST_COLOR_ZONES`], [`HIST_SLOT_ZONES`]) | 1 each per solving step | colours, and their slots, whose width lies in the bin: [1,32) [32,64) [64,128) [128,256) [256,∞) ([`HIST_BIN_EDGES`]); a colour with no slot is in no bin |
//! | [`PHYS_S6_GRAPH_HIT`] / [`PHYS_S6_PB_HIT`] | 1 each per solving step | 1 when the step's graph (and P-b's layout) inputs equal the previous armed step's, else 0 (S6's hit-rate counter; 0 on the first armed step) |
//! | [`PHYS_NP_WAVE_RAMP`], [`PHYS_NP_WAVE_TAIL`], [`PHYS_NP_WAVE_INFLIGHT`], [`PHYS_NP_WAVE_LANES`], [`PHYS_NP_WAVE_OVERFLOW`] | 1 each when the narrowphase dispatched | the same four readings for its one wave, and its unstamped tasks |
//! | [`PHYS_WAVE_JOIN`] | 1 per solving step | Σ over the step's dispatched solve waves of the join's own latency: the ticks from the moment the join could first return — the later of the end of the caller's spawn loop and the last task's end, on any thread — to its return. At most [`PHYS_WAVE_TAIL`]; the tail less it is the end imbalance |
//! | [`PHYS_WAVE_HELPED`] | 1 per solving step | the step's dispatched solve waves in which a thread other than the caller started a task: the waves whose ramp is a reading, not the 0 of a wave the caller ran alone |
//! | [`PHYS_WAVE_FIRST_RAMP`] / [`PHYS_WAVE_FIRST_TAIL`] | 1 each per solving step | the ramp and the tail of the step's first dispatched colour wave (0 on a step that dispatched none) |
//! | [`PHYS_WAVE_PASS_RAMP`] | 1 per solving step | Σ over the step's passes of the ramp of each pass's first dispatched colour wave |
//! | [`PHYS_NP_WAVE_JOIN`] | 1 when the narrowphase dispatched | its wave's join latency, as [`PHYS_WAVE_JOIN`]'s; at most [`PHYS_NP_WAVE_TAIL`] |
//! | [`PHYS_NP_ROUTE_WORKER`] | 1 when the narrowphase dispatched | 1 when its wave's joiner was a worker of the pool, else 0 (the colour waves' [`PHYS_ROUTE_WORKER`] rule) |
//! | [`PHYS_SETUP_CHUNKS`] | 1 per solving step | the tasks S4's setup scope spawned: 0 when P-c ran inline, else `2..=`[`SETUP_MAX_TASKS`] |
//! | [`PHYS_SETUP_STAMPED`] | 1 when the setup scope dispatched | the tasks the setup wave's record stamped: every task it spawned, so [`PHYS_SETUP_CHUNKS`]'s value on that step |
//!
//! A dispatched solve wave is a colour whose `pool.scope` opened — a wide colour of a pass when
//! the step's parallel gate holds (`parallel_solve`, a pool of at least two workers, the widest
//! colour at least [`WIDE_COLOR_MIN_SLOTS`]) — or S4's setup scope, which opens under the same
//! gate when the step has at least two setup tasks' worth of points and cohorts (the setup wave:
//! one per such step, counted by the wave sums and the route counters, not by
//! [`PHYS_COLOR_SCOPES`] / [`PHYS_COLOR_TASKS`]). The ramp and tail are raw clock ticks, like a
//! span's value; a reader scales them by the calibrated ticks per nanosecond.
//!
//! **S4's setup task count** is `tasks = min(lanes × `[`SETUP_CHUNKS_PER_LANE`]`, points /
//! `[`SETUP_MIN_POINTS_PER_CHUNK`]`, cohorts, `[`SETUP_MAX_TASKS`]`)`; when it is at least 2 the
//! cohorts are cut by a point quota of `points / tasks` (rounded up) on cohort boundaries, the
//! last range taking the rest, and the scope opens when that cut makes at least two ranges (a
//! lumpy cut can make fewer than `tasks`). [`PHYS_SETUP_CHUNKS`] is the range count. A reader
//! recomputes both from the world as the per-colour task count is recomputed.
//!
//! **Why the setup wave has a stamp counter of its own** (review round 2, O1). A colour wave's
//! stamped tasks are [`PHYS_COLOR_TASKS`], which a reader checks exactly, but the setup wave's
//! reading enters only the sums, and [`PHYS_SETUP_CHUNKS`] counts the ranges spawned, not the
//! record. A setup wave spawned without its stamps still reduces to a reading — no task, a tail
//! and a join equal to the join's absolute tick — that the ramp, tail and join sums take in, and
//! no range check on them can see one such wave among a step's hundred colour waves.
//! [`PHYS_SETUP_STAMPED`] is that record's task count, pushed with its reading after the join, so a
//! reader that checks it equal to [`PHYS_SETUP_CHUNKS`] sees an unstamped setup wave.
//!
//! **How a wave is read, and what it costs.** The caller decides armed once per wave from the
//! colour span's own guard (the narrowphase: its dispatch span's guard), so a disarmed wave adds
//! no load. Disarmed, the spawn loop spawns the task it always spawned; armed, it spawns that
//! task wrapped by [`WaveStamps::task`], which costs the task one `Relaxed` `fetch_add` (its
//! slot), two clock reads, one thread-local read and three `Relaxed` stores into a stack-local
//! [`WaveStamps`] record. The caller stamps the scope's opening, the end of its spawn loop and
//! the join's return, and reduces the record after the colour span has closed: an
//! `O(n log n)` sort of at most [`WAVE_STAMP_CAPACITY`] stamps, which lands in the pass span, not
//! in the colour span. The joiner route is read from `boyko_threadpool::current_worker_id`: an id
//! below the pool's worker count is the worker route; the dispatcher and an unattached thread
//! are external. A worker inside an `install` frame of its own pool reads as the dispatcher,
//! which is the route that frame's joiner takes; a worker of ANOTHER pool would read as a worker,
//! and the physics schedule never creates one (cut Q4).
//!
//! **Why the four wave readings are per-step sums.** A reader needs them per wave, and gets
//! them as the sum over the step divided by its solve scopes ([`PHYS_ROUTE_WORKER`] +
//! [`PHYS_ROUTE_EXTERNAL`]): the split of the step's dispatch loss into ramp, imbalance and join
//! is a sum to begin with. A sample per wave does not fit the solve's lane. Measured on the
//! parity runner's armed rows at W8 (`worker_lane_max`, the most samples one worker lane held
//! in a step): the parent read 236 on J-T and 247 on L10's rest pile; four samples per wave
//! read 826 on J-T and filled a `dev` region's 1024 (`boyko_diag::profile::REGION_CAPACITY`)
//! on the rest pile's first step, 192 waves, with 248 of its 600 steps above 900. A refused
//! sample is a drop and a row with a drop is void, so the per-wave form voided L10's armed rows.
//! As sums the instrument's load no longer grows with the waves: on the same rows the most one
//! worker lane held in a step reads 277 on the rest pile and 266 on J-T, 1 to 55 samples above
//! the parent's step for step.
//!
//! **What a sum cannot say, and the counters that say it** (the instrument's review, B1, B2,
//! N3, N6; `01-DESIGN.md` §10.1b). A step's ramp sum cannot tell one slow recruitment on the
//! step's first wave from a small ramp on every wave, so the first dispatched colour wave's ramp
//! and tail are kept apart ([`PHYS_WAVE_FIRST_RAMP`], [`PHYS_WAVE_FIRST_TAIL`]), and so is the
//! sum over each pass's first wave ([`PHYS_WAVE_PASS_RAMP`]), where a pool that parked between
//! passes pays its wake. A pass is recognised without a marker on the disarmed path: a pass walks
//! its colours in ascending order, so a stamped colour index at or below the previous stamped one
//! opens a new pass. A wave the caller ran alone reads a ramp of 0, so the waves whose ramp is a
//! reading are counted ([`PHYS_WAVE_HELPED`]): the mean ramp of a recruited wave is the ramp sum
//! over that count. The tail is the end imbalance plus the join's own latency; the join's is
//! kept apart ([`PHYS_WAVE_JOIN`], [`PHYS_NP_WAVE_JOIN`]), measured from the later of the spawn
//! loop's end and the last task's end, so it never exceeds the tail and the imbalance is the
//! difference. The narrowphase's one wave records its route ([`PHYS_NP_ROUTE_WORKER`]), which
//! its ramp is read by. Seven samples at most per step on the calling thread's lane.
//!
//! **What the stamps cost the armed wave.** A task's stamp is its own 64-byte line and the slot
//! counter another, so no two workers write one line and no worker writes the caller's (the
//! review's N2: 24-byte slots put about three tasks of different workers on one line, and the slot
//! counter shared the caller's). The record is 8.3 KiB, zeroed on the calling thread before the
//! scope's opening stamp: its cost lands in the colour span, never in a ramp, tail or join.
//!
//! The three narrowphase class counters are computed after the pair loop from the pairs' tags,
//! and close: `PHYS_NP_FULL + PHYS_NP_REUSED + PHYS_NP_SEP_HITS` plus the non-box pairs and the
//! pairs L10 skipped for a held endpoint (`Manifolds::pair_classes`) is `PHYS_NP_PAIRS`. With
//! contact reuse off `PHYS_NP_REUSED` is 0.
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
//! whole per-color call — the dispatch and the join included — from the calling thread. The W8S
//! per-task stamps above are not zones: a task writes its own slot of a record no other wave
//! shares, and no lock-prefixed accumulator outlives the wave (ruling 3).
//!
//! # The in-zone canary (W8S, `levers/scaling/01-DESIGN.md` §7)
//!
//! [`ColoredSoftStepSolver::set_zone_canary`](crate::solver::ColoredSoftStepSolver::set_zone_canary)
//! names one of [`CANARY_ZONES`] and a busy-wait length; the named zone then spins that long on
//! the calling thread each time it opens ARMED. It demonstrates a span gate's resolution: the
//! smallest injected cost a window reads back. A disarmed zone never spins, because the check sits
//! in the zone's admitted arm, so a build whose tier folds the zone folds the canary with it.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use boyko_diag::profiling_abi::{GLOBAL_TIER, ZoneHandle, ZoneTier, zone_id};
use boyko_diag::sample::{Sample, SampleKind};
use boyko_diag::{clock, declare_zone, sample};
use boyko_ecs::ecs::core::profiling::ROOT_SCOPE;
use boyko_threadpool::current_worker_id;

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
declare_zone!(PHYS_SLEEP_CLASSIFY, name = "phys_sleep_classify", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The parallel narrowphase's zones (L5) ─────────────────────────────────────

declare_zone!(PHYS_NP_DISPATCH, name = "phys_np_dispatch", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_COMPACT, name = "phys_np_compact", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_AXIS_COMMIT, name = "phys_np_axis_commit", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The tree broadphase's zones ───────────────────────────────────────────────

declare_zone!(PHYS_BP_VERIFY, name = "phys_bp_verify", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_BUILD, name = "phys_bp_build", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_QUERY, name = "phys_bp_query", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_ASSEMBLE, name = "phys_bp_assemble", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The solve build's split (W8S) ─────────────────────────────────────────────

declare_zone!(PHYS_SB_BODIES, name = "phys_sb_bodies", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SB_PA, name = "phys_sb_pa", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SB_PB, name = "phys_sb_pb", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SB_PC, name = "phys_sb_pc", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The per-step counters ─────────────────────────────────────────────────────

declare_zone!(PHYS_SLOTS_WIDE, name = "phys_slots_wide", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLOTS_NARROW, name = "phys_slots_narrow", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_PAIRS, name = "phys_np_pairs", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_MANIFOLDS, name = "phys_np_manifolds", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_POINTS, name = "phys_np_points", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_PAIRS, name = "phys_bp_pairs", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_CHUNKS, name = "phys_np_chunks", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_QUERIED, name = "phys_bp_queried", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_MEMBERS, name = "phys_bp_members", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_BP_REBUILDS, name = "phys_bp_rebuilds", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_REUSED, name = "phys_np_reused", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_SEP_HITS, name = "phys_np_sep_hits", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_FULL, name = "phys_np_full", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SLEEP_HELD, name = "phys_sleep_held", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

// ── The W8S telemetry counters (armed only) ───────────────────────────────────

declare_zone!(PHYS_WAVE_RAMP, name = "phys_wave_ramp", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_TAIL, name = "phys_wave_tail", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_INFLIGHT, name = "phys_wave_inflight", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_LANES, name = "phys_wave_lanes", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_OVERFLOW, name = "phys_wave_overflow", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_COLOR_SCOPES, name = "phys_color_scopes", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_COLOR_TASKS, name = "phys_color_tasks", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_ROUTE_WORKER, name = "phys_route_worker", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_ROUTE_EXTERNAL, name = "phys_route_external", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_COLORS_LT32, name = "phys_hist_colors_lt32", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_COLORS_LT64, name = "phys_hist_colors_lt64", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_COLORS_LT128, name = "phys_hist_colors_lt128", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_COLORS_LT256, name = "phys_hist_colors_lt256", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_COLORS_GE256, name = "phys_hist_colors_ge256", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_SLOTS_LT32, name = "phys_hist_slots_lt32", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_SLOTS_LT64, name = "phys_hist_slots_lt64", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_SLOTS_LT128, name = "phys_hist_slots_lt128", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_SLOTS_LT256, name = "phys_hist_slots_lt256", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_HIST_SLOTS_GE256, name = "phys_hist_slots_ge256", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_S6_GRAPH_HIT, name = "phys_s6_graph_hit", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_S6_PB_HIT, name = "phys_s6_pb_hit", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_RAMP, name = "phys_np_wave_ramp", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_TAIL, name = "phys_np_wave_tail", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_INFLIGHT, name = "phys_np_wave_inflight", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_LANES, name = "phys_np_wave_lanes", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_OVERFLOW, name = "phys_np_wave_overflow", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_JOIN, name = "phys_wave_join", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_HELPED, name = "phys_wave_helped", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_FIRST_RAMP, name = "phys_wave_first_ramp", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_FIRST_TAIL, name = "phys_wave_first_tail", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_WAVE_PASS_RAMP, name = "phys_wave_pass_ramp", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_WAVE_JOIN, name = "phys_np_wave_join", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_NP_ROUTE_WORKER, name = "phys_np_route_worker", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SETUP_CHUNKS, name = "phys_setup_chunks", scope = ROOT_SCOPE, tier = ZoneTier::Deep);
declare_zone!(PHYS_SETUP_STAMPED, name = "phys_setup_stamped", scope = ROOT_SCOPE, tier = ZoneTier::Deep);

/// Span zones this crate declares: the length of [`SPAN_ZONES`], so a reader's expectation
/// table is typed by it and a zone without an expectation does not compile.
pub const SPAN_ZONE_COUNT: usize = 26;

/// Counter zones this crate declares: the length of [`COUNTER_ZONES`].
pub const COUNTER_ZONE_COUNT: usize = 49;

/// Every span zone this crate declares, in the order of the table in the module docs.
///
/// A reader resolves each one's id with [`zone_id`] and its name from `desc.name`, so it never
/// infers an id from declaration order: ids come from one process-wide counter shared with every
/// other zone and every system span.
pub static SPAN_ZONES: [&ZoneHandle; SPAN_ZONE_COUNT] = [
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
    &PHYS_SLEEP_CLASSIFY,
    &PHYS_NP_DISPATCH,
    &PHYS_NP_COMPACT,
    &PHYS_NP_AXIS_COMMIT,
    &PHYS_BP_VERIFY,
    &PHYS_BP_BUILD,
    &PHYS_BP_QUERY,
    &PHYS_BP_ASSEMBLE,
    &PHYS_SB_BODIES,
    &PHYS_SB_PA,
    &PHYS_SB_PB,
    &PHYS_SB_PC,
];

/// Every counter this crate declares, in the order of the counter table in the module docs.
pub static COUNTER_ZONES: [&ZoneHandle; COUNTER_ZONE_COUNT] = [
    &PHYS_SLOTS_WIDE,
    &PHYS_SLOTS_NARROW,
    &PHYS_NP_PAIRS,
    &PHYS_NP_MANIFOLDS,
    &PHYS_NP_POINTS,
    &PHYS_BP_PAIRS,
    &PHYS_NP_CHUNKS,
    &PHYS_BP_QUERIED,
    &PHYS_BP_MEMBERS,
    &PHYS_BP_REBUILDS,
    &PHYS_NP_REUSED,
    &PHYS_NP_SEP_HITS,
    &PHYS_NP_FULL,
    &PHYS_SLEEP_HELD,
    &PHYS_WAVE_RAMP,
    &PHYS_WAVE_TAIL,
    &PHYS_WAVE_INFLIGHT,
    &PHYS_WAVE_LANES,
    &PHYS_WAVE_OVERFLOW,
    &PHYS_COLOR_SCOPES,
    &PHYS_COLOR_TASKS,
    &PHYS_ROUTE_WORKER,
    &PHYS_ROUTE_EXTERNAL,
    &PHYS_HIST_COLORS_LT32,
    &PHYS_HIST_COLORS_LT64,
    &PHYS_HIST_COLORS_LT128,
    &PHYS_HIST_COLORS_LT256,
    &PHYS_HIST_COLORS_GE256,
    &PHYS_HIST_SLOTS_LT32,
    &PHYS_HIST_SLOTS_LT64,
    &PHYS_HIST_SLOTS_LT128,
    &PHYS_HIST_SLOTS_LT256,
    &PHYS_HIST_SLOTS_GE256,
    &PHYS_S6_GRAPH_HIT,
    &PHYS_S6_PB_HIT,
    &PHYS_NP_WAVE_RAMP,
    &PHYS_NP_WAVE_TAIL,
    &PHYS_NP_WAVE_INFLIGHT,
    &PHYS_NP_WAVE_LANES,
    &PHYS_NP_WAVE_OVERFLOW,
    &PHYS_WAVE_JOIN,
    &PHYS_WAVE_HELPED,
    &PHYS_WAVE_FIRST_RAMP,
    &PHYS_WAVE_FIRST_TAIL,
    &PHYS_WAVE_PASS_RAMP,
    &PHYS_NP_WAVE_JOIN,
    &PHYS_NP_ROUTE_WORKER,
    &PHYS_SETUP_CHUNKS,
    &PHYS_SETUP_STAMPED,
];

/// S4's point floor per setup task: the solver's own constant, re-exported so a reader recomputes
/// the setup's task count with the solver's number.
pub const SETUP_MIN_POINTS_PER_CHUNK: usize = crate::solver::colored::SETUP_MIN_POINTS_PER_CHUNK;

/// S4's setup task cap (the scope's one-block cell budget).
pub const SETUP_MAX_TASKS: usize = crate::solver::colored::SETUP_MAX_TASKS;

/// S4's setup tasks per worker lane: the colour cut's `CHUNKS_PER_WORKER`, which the setup's lanes
/// term shares.
pub const SETUP_CHUNKS_PER_LANE: usize = crate::solver::colored::CHUNKS_PER_WORKER;

/// The histogram's bins.
pub const HIST_BINS: usize = 5;

/// The histogram's inner bin edges: bin `i` holds the colours whose width lies in
/// `[HIST_BIN_EDGES[i - 1], HIST_BIN_EDGES[i])` — `[1, 32)` for bin 0, `[256, ∞)` for the last. The
/// last edge is the solve's inline floor, so the last bin is exactly the wide colours.
pub const HIST_BIN_EDGES: [u32; HIST_BINS - 1] = [32, 64, 128, 256];

const _: () = assert!(
    HIST_BIN_EDGES[HIST_BINS - 2] == crate::solver::colored::MIN_PARALLEL_SLOTS_PER_COLOR,
    "the histogram's last bin is the wide colours"
);

/// The colour histogram's colour counters, by bin (module docs, "The W8S telemetry").
pub static HIST_COLOR_ZONES: [&ZoneHandle; HIST_BINS] = [
    &PHYS_HIST_COLORS_LT32,
    &PHYS_HIST_COLORS_LT64,
    &PHYS_HIST_COLORS_LT128,
    &PHYS_HIST_COLORS_LT256,
    &PHYS_HIST_COLORS_GE256,
];

/// The colour histogram's slot counters, by bin.
pub static HIST_SLOT_ZONES: [&ZoneHandle; HIST_BINS] = [
    &PHYS_HIST_SLOTS_LT32,
    &PHYS_HIST_SLOTS_LT64,
    &PHYS_HIST_SLOTS_LT128,
    &PHYS_HIST_SLOTS_LT256,
    &PHYS_HIST_SLOTS_GE256,
];

/// The histogram bin of a colour `slots` wide, or `None` for a colour with no slot (every
/// manifold of it frozen).
#[inline]
pub const fn hist_bin(slots: u32) -> Option<usize> {
    if slots == 0 {
        return None;
    }
    let mut i = 0;
    while i < HIST_BIN_EDGES.len() {
        if slots < HIST_BIN_EDGES[i] {
            return Some(i);
        }
        i += 1;
    }
    Some(HIST_BINS - 1)
}

/// The zones the in-zone canary may sit in (module docs, "The in-zone canary"): the solve build
/// and its four sub-zones, the spans a W8S span gate reads.
pub static CANARY_ZONES: [&ZoneHandle; 5] =
    [&PHYS_SOLVE_BUILD, &PHYS_SB_BODIES, &PHYS_SB_PA, &PHYS_SB_PB, &PHYS_SB_PC];

/// Whether this build compiles the physics zones at all. Every zone is `Deep`, so one `const`
/// answers for all seventy-five; `false` under a profile whose tier ceiling is below `Deep`, where
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

// ── W8S: the per-wave record (ruling 3) ───────────────────────────────────────

/// Tasks one wave's record stamps; a task past it runs unstamped and is counted as overflow
/// ([`PHYS_WAVE_OVERFLOW`], [`PHYS_NP_WAVE_OVERFLOW`]). Above every wave the engine spawns at 16
/// workers: a colour cuts at most `16 × CHUNKS_PER_WORKER` = 96 tasks, the narrowphase at most
/// `16 × NP_CHUNKS_PER_LANE` = 96.
pub const WAVE_STAMP_CAPACITY: usize = 128;

/// Every [`WaveStamps`] ever built, process-wide: the witness that a disarmed step took the
/// disarmed spawn loop (cut review W4). A stamped task borrows a record, so a disarmed path that
/// spawned the stamped wrapper would have had to build one, and this count would move.
static WAVE_RECORDS: AtomicU64 = AtomicU64::new(0);

/// How many per-wave records this process has built (W8S; the `profiling_bit_identity` gate
/// reads it around a disarmed run, where it must not move, and an armed one, where it must).
#[doc(hidden)]
pub fn wave_records_built() -> u64 {
    WAVE_RECORDS.load(Ordering::Relaxed)
}

/// One task's interval and thread, written by the task itself. A line of its own (review N2):
/// the tasks of one wave run on different workers, and a slot that shared a line with another
/// task's slot made each stamp a cross-core line transfer inside the wave it measures.
#[repr(C, align(64))]
struct TaskStamp {
    start: AtomicU64,
    end: AtomicU64,
    lane: AtomicU32,
}

impl TaskStamp {
    /// A slot no task has written.
    const fn new() -> Self {
        Self { start: AtomicU64::new(0), end: AtomicU64::new(0), lane: AtomicU32::new(0) }
    }
}

/// The record's slot counter, on a line of its own (review N2): every task of the wave
/// `fetch_add`s it, and the caller's own stamps must not share its line.
#[repr(C, align(64))]
struct SlotCounter(AtomicU32);

/// One parallel wave's telemetry record (W8S, ruling 3): the calling thread's stamps of the
/// scope's opening, the end of its spawn loop and the join's return, and one [`TaskStamp`] per
/// task. Stack-local to the armed arm of one wave — transient function-local scratch, never a
/// side store — and built only while armed (module docs, "The W8S telemetry").
///
/// Every field is an atomic written `Relaxed`: a task claims its slot with one `fetch_add`, so
/// no two tasks write one slot, and the caller reads the record only after the scope's join, whose
/// completion protocol orders every task's stores before the join returns.
///
/// The layout is the point of `repr(C)`: the slot counter's line, then the caller's line (its
/// five stamps and ids), then one line per task slot.
#[repr(C)]
pub(crate) struct WaveStamps {
    /// The next free slot; after the join, the number of tasks that ran.
    next: SlotCounter,
    /// The scope's opening, `0` until the wave dispatched.
    open: AtomicU64,
    /// The end of the caller's spawn loop.
    spawned: AtomicU64,
    /// The join's return.
    joined: AtomicU64,
    /// The caller's `current_worker_id()`.
    caller: AtomicU32,
    /// The pool's worker count.
    workers: AtomicU32,
    /// One slot per task, `[0, min(next, WAVE_STAMP_CAPACITY))` written.
    slots: [TaskStamp; WAVE_STAMP_CAPACITY],
}

const _: () = assert!(
    size_of::<TaskStamp>() == 64 && align_of::<TaskStamp>() == 64,
    "a task stamp is one cache line"
);
const _: () = assert!(
    std::mem::offset_of!(WaveStamps, open) == 64 && std::mem::offset_of!(WaveStamps, slots) == 128,
    "the slot counter, the caller's stamps and the slots each begin a line of their own"
);

/// What one wave's record reduces to.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WaveReading {
    /// [`PHYS_WAVE_RAMP`]'s value.
    pub(crate) ramp: u64,
    /// [`PHYS_WAVE_TAIL`]'s value.
    pub(crate) tail: u64,
    /// [`PHYS_WAVE_INFLIGHT`]'s value.
    pub(crate) inflight: u64,
    /// [`PHYS_WAVE_LANES`]'s value.
    pub(crate) lanes: u64,
    /// [`PHYS_WAVE_JOIN`]'s value: the join's own latency, at most `tail`.
    pub(crate) join: u64,
    /// Tasks that ran.
    pub(crate) tasks: u64,
    /// Of those, the ones past [`WAVE_STAMP_CAPACITY`].
    pub(crate) overflow: u64,
    /// Whether the joiner was a worker of the pool (the worker route).
    pub(crate) worker_route: bool,
    /// Whether a thread other than the caller started a task ([`PHYS_WAVE_HELPED`]); `ramp` is 0
    /// when it did not.
    pub(crate) helped: bool,
}

impl WaveStamps {
    /// An empty record. Out of line: it runs only armed, and its 8.3 KiB initialisation stays out
    /// of the disarmed caller's body. The one `fetch_add` on the process-wide witness is the
    /// review-W4 exception (caller-side, once per armed wave, uncontended in a physics step).
    #[cold]
    #[inline(never)]
    pub(crate) fn new() -> Self {
        WAVE_RECORDS.fetch_add(1, Ordering::Relaxed);
        Self {
            next: SlotCounter(AtomicU32::new(0)),
            open: AtomicU64::new(0),
            spawned: AtomicU64::new(0),
            joined: AtomicU64::new(0),
            caller: AtomicU32::new(0),
            workers: AtomicU32::new(0),
            slots: [const { TaskStamp::new() }; WAVE_STAMP_CAPACITY],
        }
    }

    /// The caller, just before `pool.scope`: the opening stamp, its thread and the pool's width.
    #[inline]
    pub(crate) fn begin(&self, workers: usize) {
        self.caller.store(current_worker_id(), Ordering::Relaxed);
        self.workers.store(u32::try_from(workers).unwrap_or(u32::MAX), Ordering::Relaxed);
        // A tick of 0 would read as "never dispatched"; the counter is never 0 past its first
        // nanosecond, and `max(1)` keeps the flag exact regardless.
        self.open.store(clock::ticks().max(1), Ordering::Relaxed);
    }

    /// The caller, after its spawn loop and before the join.
    #[inline]
    pub(crate) fn spawned(&self) {
        self.spawned.store(clock::ticks(), Ordering::Relaxed);
    }

    /// The caller, after `pool.scope` returned.
    #[inline]
    pub(crate) fn joined(&self) {
        self.joined.store(clock::ticks(), Ordering::Relaxed);
    }

    /// Runs one task's `body` between two stamps: one `Relaxed` `fetch_add` (the slot), then the
    /// start, end and thread stores into that slot (ruling 3's letter, cut Q2).
    #[inline]
    pub(crate) fn task(&self, body: impl FnOnce()) {
        let slot = self.next.0.fetch_add(1, Ordering::Relaxed) as usize;
        let start = clock::ticks();
        body();
        let end = clock::ticks();
        if let Some(s) = self.slots.get(slot) {
            s.start.store(start, Ordering::Relaxed);
            s.end.store(end, Ordering::Relaxed);
            s.lane.store(current_worker_id(), Ordering::Relaxed);
        }
    }

    /// Reduces the record, after the join: `None` when the wave never dispatched (it ran
    /// inline). Out of line and cold: armed only.
    #[cold]
    #[inline(never)]
    pub(crate) fn reduce(&self) -> Option<WaveReading> {
        let open = self.open.load(Ordering::Relaxed);
        if open == 0 {
            return None;
        }
        let tasks = u64::from(self.next.0.load(Ordering::Relaxed));
        let n = usize::try_from(tasks).map_or(WAVE_STAMP_CAPACITY, |t| t.min(WAVE_STAMP_CAPACITY));
        let caller = self.caller.load(Ordering::Relaxed);
        let workers = self.workers.load(Ordering::Relaxed);
        let mut starts = [0u64; WAVE_STAMP_CAPACITY];
        let mut ends = [0u64; WAVE_STAMP_CAPACITY];
        let mut first_helper = u64::MAX;
        let spawned = self.spawned.load(Ordering::Relaxed);
        let mut caller_last = spawned;
        // The join could not return before the spawn loop ended nor before the last task ended,
        // on whichever thread ran it.
        let mut join_ready = spawned;
        // Distinct threads: worker ids below 64 as bits, the two sentinels (dispatcher,
        // unattached) as two more.
        let mut workers_seen = 0u64;
        let mut sentinels_seen = 0u8;
        for (i, s) in self.slots[..n].iter().enumerate() {
            let (start, end) = (s.start.load(Ordering::Relaxed), s.end.load(Ordering::Relaxed));
            let lane = s.lane.load(Ordering::Relaxed);
            starts[i] = start;
            ends[i] = end;
            join_ready = join_ready.max(end);
            if lane == caller {
                caller_last = caller_last.max(end);
            } else {
                first_helper = first_helper.min(start);
            }
            if lane < 64 {
                workers_seen |= 1 << lane;
            } else {
                sentinels_seen |= 1 << (lane & 1);
            }
        }
        let (starts, ends) = (&mut starts[..n], &mut ends[..n]);
        starts.sort_unstable();
        ends.sort_unstable();
        // The sweep: an interval that ends at the tick another starts does not overlap it.
        let (mut open_now, mut inflight, mut j) = (0u64, 0u64, 0usize);
        for &start in starts.iter() {
            while j < ends.len() && ends[j] <= start {
                // Saturating: a zero-length interval sorts its end beside its own start.
                open_now = open_now.saturating_sub(1);
                j += 1;
            }
            open_now += 1;
            inflight = inflight.max(open_now);
        }
        let joined = self.joined.load(Ordering::Relaxed);
        let helped = first_helper != u64::MAX;
        let tail = joined.saturating_sub(caller_last);
        // `join_ready >= caller_last`: both start at `spawned`, and every task end that moves
        // `caller_last` moves `join_ready` too.
        let join = joined.saturating_sub(join_ready);
        debug_assert!(join <= tail, "invariant: the join's latency is part of the tail");
        Some(WaveReading {
            ramp: if helped { first_helper.saturating_sub(open) } else { 0 },
            tail,
            inflight,
            lanes: u64::from(workers_seen.count_ones()) + u64::from(sentinels_seen.count_ones()),
            join,
            tasks,
            overflow: tasks.saturating_sub(WAVE_STAMP_CAPACITY as u64),
            worker_route: caller < workers,
            helped,
        })
    }
}

impl WaveReading {
    /// Pushes the narrowphase wave's seven counters.
    #[cold]
    #[inline(never)]
    pub(crate) fn push_np(&self) {
        counter!(PHYS_NP_WAVE_RAMP, self.ramp);
        counter!(PHYS_NP_WAVE_TAIL, self.tail);
        counter!(PHYS_NP_WAVE_INFLIGHT, self.inflight);
        counter!(PHYS_NP_WAVE_LANES, self.lanes);
        counter!(PHYS_NP_WAVE_OVERFLOW, self.overflow);
        counter!(PHYS_NP_WAVE_JOIN, self.join);
        counter!(PHYS_NP_ROUTE_WORKER, u64::from(self.worker_route));
    }
}

/// One solving step's dispatch totals (W8S), summed over its waves on the calling thread and
/// pushed once at the step's end, never per wave (module docs, "Why the four wave readings are
/// per-step sums"). Written only in the armed arm of a wave; a disarmed step pays its
/// zero-initialisation, once per solving step (120 bytes, 72 before the review's five sums).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WaveTally {
    /// Σ of the solve waves' ramps, in ticks.
    pub(crate) ramp: u64,
    /// Σ of their tails, in ticks.
    pub(crate) tail: u64,
    /// Σ of their in-flight maxima.
    pub(crate) inflight: u64,
    /// Σ of their lane counts.
    pub(crate) lanes: u64,
    /// Colour scopes opened.
    pub(crate) scopes: u64,
    /// Their tasks.
    pub(crate) tasks: u64,
    /// Tasks past the record's capacity.
    pub(crate) overflow: u64,
    /// Solve scopes joined by a worker of the pool.
    pub(crate) route_worker: u64,
    /// Solve scopes joined by anything else.
    pub(crate) route_external: u64,
    /// Σ of the solve waves' join latencies ([`PHYS_WAVE_JOIN`]).
    pub(crate) join: u64,
    /// Solve waves a thread other than the caller helped ([`PHYS_WAVE_HELPED`]).
    pub(crate) helped: u64,
    /// The step's first dispatched colour wave's ramp ([`PHYS_WAVE_FIRST_RAMP`]).
    pub(crate) first_ramp: u64,
    /// Its tail ([`PHYS_WAVE_FIRST_TAIL`]).
    pub(crate) first_tail: u64,
    /// Σ of each pass's first dispatched colour wave's ramp ([`PHYS_WAVE_PASS_RAMP`]).
    pub(crate) pass_ramp: u64,
    /// The colour index of the last colour wave added, `None` before the step's first: how
    /// [`add`](Self::add) recognises a pass without a marker on the disarmed path.
    last_colour: Option<u32>,
}

impl WaveTally {
    /// Adds one solve wave's reading. `colour` is the wave's colour index, `None` for a solve
    /// scope that is not a colour's (S4's setup scope). A colour index at or below the previous
    /// colour wave's starts a new pass: a pass visits its colours in ascending order, each once.
    #[cold]
    #[inline(never)]
    pub(crate) fn add(&mut self, r: &WaveReading, colour: Option<usize>) {
        self.ramp += r.ramp;
        self.tail += r.tail;
        self.inflight += r.inflight;
        self.lanes += r.lanes;
        self.join += r.join;
        self.helped += u64::from(r.helped);
        if let Some(c) = colour {
            let c = u32::try_from(c).unwrap_or(u32::MAX);
            self.scopes += 1;
            self.tasks += r.tasks;
            if self.last_colour.is_none() {
                self.first_ramp = r.ramp;
                self.first_tail = r.tail;
            }
            if self.last_colour.is_none_or(|last| c <= last) {
                self.pass_ramp += r.ramp;
            }
            self.last_colour = Some(c);
        }
        self.overflow += r.overflow;
        if r.worker_route {
            self.route_worker += 1;
        } else {
            self.route_external += 1;
        }
    }

    /// Pushes the step's fourteen totals.
    #[cold]
    #[inline(never)]
    pub(crate) fn push(&self) {
        counter!(PHYS_WAVE_RAMP, self.ramp);
        counter!(PHYS_WAVE_TAIL, self.tail);
        counter!(PHYS_WAVE_INFLIGHT, self.inflight);
        counter!(PHYS_WAVE_LANES, self.lanes);
        counter!(PHYS_WAVE_OVERFLOW, self.overflow);
        counter!(PHYS_COLOR_SCOPES, self.scopes);
        counter!(PHYS_COLOR_TASKS, self.tasks);
        counter!(PHYS_ROUTE_WORKER, self.route_worker);
        counter!(PHYS_ROUTE_EXTERNAL, self.route_external);
        counter!(PHYS_WAVE_JOIN, self.join);
        counter!(PHYS_WAVE_HELPED, self.helped);
        counter!(PHYS_WAVE_FIRST_RAMP, self.first_ramp);
        counter!(PHYS_WAVE_FIRST_TAIL, self.first_tail);
        counter!(PHYS_WAVE_PASS_RAMP, self.pass_ramp);
    }
}

// ── W8S: the in-zone canary ───────────────────────────────────────────────────

/// Which of [`CANARY_ZONES`] spins, and for how long (module docs, "The in-zone canary"), and how
/// many times it has. The default spins nowhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ZoneCanary {
    /// `1 + index` into [`CANARY_ZONES`]; `0` for none.
    slot: u8,
    /// The busy-wait per opening, in ns.
    ns: u64,
    /// Busy-waits run: the receipt that the canary landed on every armed opening of its zone. A
    /// span check alone cannot show it, because a zone whose own cost exceeds `ns` passes
    /// "span ≥ ns per opening" with no canary at all (measured on `phys_solve_build`).
    spins: u64,
}

impl ZoneCanary {
    /// The canary on `zone` for `ns` nanoseconds, or `None` when `zone` is not one of
    /// [`CANARY_ZONES`]. `ns == 0` is no canary.
    pub(crate) fn new(zone: &ZoneHandle, ns: u64) -> Option<Self> {
        let index = CANARY_ZONES.iter().position(|&h| std::ptr::eq(h, zone))?;
        Some(if ns == 0 {
            Self::default()
        } else {
            Self { slot: index as u8 + 1, ns, spins: 0 }
        })
    }

    /// Spins when `zone` is the canary's and its guard was admitted (`admitted`: the zone's
    /// `Option<ZoneGuard>` is `Some`), and counts the spin. One compare when disarmed or
    /// elsewhere.
    #[inline]
    pub(crate) fn at(&mut self, zone: &ZoneHandle, admitted: bool) {
        if admitted && self.slot != 0 && std::ptr::eq(CANARY_ZONES[usize::from(self.slot - 1)], zone) {
            spin_ns(self.ns);
            self.spins += 1;
        }
    }

    /// Busy-waits run so far.
    #[inline]
    pub(crate) fn spins(self) -> u64 {
        self.spins
    }
}

/// The canary's busy-wait: `ns` nanoseconds on the calling thread.
#[cold]
#[inline(never)]
fn spin_ns(ns: u64) {
    let target = Duration::from_nanos(ns);
    let start = Instant::now();
    while start.elapsed() < target {
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    //! The W8S record's reduction and the step tally on synthetic stamps (the instrument's
    //! review, `levers/scaling/01-DESIGN.md` §10.1b): exact values a live wave cannot give.

    use std::sync::atomic::Ordering;

    use boyko_threadpool::WORKER_ID_DISPATCHER;

    use super::{WaveReading, WaveStamps, WaveTally};

    /// A record opened at tick 100 by `caller` on a pool of `workers`, its spawn loop ending at
    /// `spawned` and its join returning at `joined`, with one task per `(start, end, lane)`.
    fn record(
        caller: u32,
        workers: u32,
        spawned: u64,
        joined: u64,
        tasks: &[(u64, u64, u32)],
    ) -> WaveStamps {
        let rec = WaveStamps::new();
        rec.caller.store(caller, Ordering::Relaxed);
        rec.workers.store(workers, Ordering::Relaxed);
        rec.open.store(100, Ordering::Relaxed);
        rec.spawned.store(spawned, Ordering::Relaxed);
        rec.joined.store(joined, Ordering::Relaxed);
        for &(start, end, lane) in tasks {
            let slot = rec.next.0.fetch_add(1, Ordering::Relaxed) as usize;
            rec.slots[slot].start.store(start, Ordering::Relaxed);
            rec.slots[slot].end.store(end, Ordering::Relaxed);
            rec.slots[slot].lane.store(lane, Ordering::Relaxed);
        }
        rec
    }

    /// A reading of `ramp` and `tail`, helped when `ramp > 0`, joined in `tail / 4`.
    fn reading(ramp: u64, tail: u64) -> WaveReading {
        WaveReading { ramp, tail, join: tail / 4, helped: ramp > 0, tasks: 3, ..WaveReading::default() }
    }

    #[test]
    fn w8s_reduce_splits_the_tail_into_imbalance_and_join() {
        // Caller 0 ends its own task at 200; helpers 1 and 2 end at 350 and 300; the join returns
        // at 400: the tail is 400 − 200, of which the join's own latency is 400 − 350.
        let r = record(0, 4, 150, 400, &[(160, 200, 0), (120, 350, 1), (130, 300, 2)])
            .reduce()
            .expect("a dispatched wave reduces");
        assert_eq!((r.tail, r.join), (200, 50), "tail and join: {r:?}");
        // The spawn loop ended after every task: the join could not return before it, so the
        // join is the whole tail and the imbalance is 0.
        let r = record(0, 4, 500, 520, &[(120, 200, 1)]).reduce().expect("a dispatched wave reduces");
        assert_eq!((r.tail, r.join), (20, 20), "a spawn loop that ends last: {r:?}");
    }

    #[test]
    fn w8s_reduce_reads_helped_and_ramp() {
        let helped = record(0, 4, 150, 400, &[(160, 200, 0), (120, 350, 1)])
            .reduce()
            .expect("a dispatched wave reduces");
        assert_eq!((helped.helped, helped.ramp), (true, 20), "helper 1 started at 120: {helped:?}");
        assert!(helped.worker_route, "a worker joined: {helped:?}");
        let d = WORKER_ID_DISPATCHER;
        let alone = record(d, 4, 150, 300, &[(160, 200, d), (200, 260, d)])
            .reduce()
            .expect("a dispatched wave reduces");
        assert_eq!((alone.helped, alone.ramp), (false, 0), "the caller ran every task: {alone:?}");
        assert!(!alone.worker_route, "the dispatcher joined: {alone:?}");
    }

    #[test]
    fn w8s_tally_keeps_the_step_first_colour_wave() {
        let mut t = WaveTally::default();
        // A solve wave that is not a colour's (S4's setup) comes first and is not the first
        // colour wave.
        t.add(&reading(1000, 7), None);
        for (c, ramp, tail) in [(2, 10, 20), (5, 11, 21), (2, 12, 22), (5, 13, 23)] {
            t.add(&reading(ramp, tail), Some(c));
        }
        assert_eq!((t.first_ramp, t.first_tail), (10, 20), "the step's first colour wave: {t:?}");
        assert_eq!(
            (t.scopes, t.route_worker + t.route_external),
            (4, 5),
            "colour scopes, solve scopes: {t:?}"
        );
        assert_eq!((t.helped, t.join), (5, 1 + 5 + 5 + 5 + 5), "helped waves and the join sum: {t:?}");
    }

    #[test]
    fn w8s_tally_sums_each_pass_first_colour_wave() {
        // Three passes over colours 2 and 5: each pass's first wave is its colour 2.
        let mut t = WaveTally::default();
        t.add(&reading(1000, 7), None);
        for (c, ramp) in [(2, 10), (5, 11), (2, 12), (5, 13), (2, 14), (5, 15)] {
            t.add(&reading(ramp, 1), Some(c));
        }
        assert_eq!(t.pass_ramp, 10 + 12 + 14, "each pass's colour 2: {t:?}");
        // One wide colour per pass: every wave opens a pass.
        let mut t = WaveTally::default();
        for ramp in [1, 2, 4] {
            t.add(&reading(ramp, 1), Some(4));
        }
        assert_eq!(t.pass_ramp, 7, "one colour a pass: {t:?}");
    }
}
