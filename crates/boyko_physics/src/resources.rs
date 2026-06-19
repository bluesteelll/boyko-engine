//! Physics step resources — the preallocated, reused step buffers (plan D1/D4/
//! IM-1).
//!
//! Every `Vec` here is sized once and refilled each step (cleared, capacity
//! reused): the foundation does no per-step / per-manifold heap allocation
//! (principle 5). [`SolverScratch`] is the dense, row-indexed snapshot the
//! gather→solve→apply pipeline addresses by [`BodyIndex`]
//! — see [`crate::systems`].

use boyko_macros::Resource;
use boyko_threadpool::try_with_active_pool;
use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass};
use crate::manifold::{BodyIndex, Manifold};
use crate::math::{Mat3, Quat, Vec3};
use crate::narrowphase::axis_cache::BoxAxisCache;
use crate::systems::body_bounding_radius;

/// Number of bits in one [`BitSet256`] chunk.
const BITS_PER_CHUNK: usize = 256;

/// Broadphase algorithm selector (plan O2, Decision 1; the 0%-gate flag).
///
/// [`AllPairs`](BroadphaseKind::AllPairs) is the DEFAULT and runs the shipped
/// O(n²) double loop byte-for-byte unchanged — so a world that never opts in is
/// bit-identical to today (the campaign 0%-gate). [`Grid`](BroadphaseKind::Grid)
/// opts into the uniform-grid CSR counting-sort, which emits candidate pairs then
/// applies the SAME sphere-bound feasibility predicate as all-pairs and SORTS the
/// survivors by `(min, max)` — so its [`ContactPairs`] output is bit-identical to
/// all-pairs (the O2 correctness gate). The choice is a single runtime branch in
/// [`physics_broadphase`](crate::systems::physics_broadphase) (the one-branch
/// floor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BroadphaseKind {
    /// The shipped O(n²) all-pairs loop (DEFAULT) — byte-identical to today.
    #[default]
    AllPairs,
    /// The uniform-grid CSR counting-sort broadphase (opt-in, O2). Produces a
    /// `(min, max)`-sorted pair set bit-identical to [`AllPairs`](Self::AllPairs).
    Grid,
}

/// Global physics tunables (plan D1; P2 W1 soft-constraint set).
///
/// `gravity`, `substeps`, `relax_iterations`, and the soft-constraint pair
/// (`contact_hertz` / `contact_damping`) are user-set; `dt` is NOT — it is
/// stamped by [`physics_gather`](crate::systems::physics_gather) from the
/// fixed clock each step (OQ-1), so a hand-set value is overwritten.
#[derive(Resource, Clone, Copy, Debug)]
pub struct PhysicsConfig {
    /// Constant acceleration applied to dynamic bodies each step (world
    /// units/s²).
    pub gravity: Vec3,
    /// The step delta in seconds, stamped each step by
    /// [`physics_gather`](crate::systems::physics_gather) from
    /// [`FixedTime::delta_secs`](boyko_ecs::ecs::core::time::FixedTime::delta_secs)
    /// (OQ-1). Not user-set — the TGS solver reads `h = dt / substeps`.
    pub dt: f32,
    /// Solver substep count (default `4`, OQ-5). The TGS solver loops this many
    /// times per step over the same contact set; the no-op solver ignores it.
    pub substeps: u32,
    /// Relaxation passes per substep (default `2`): post-solve iterations that
    /// re-solve the constraints bias-free to remove soft-bias energy.
    pub relax_iterations: u32,
    /// Soft-constraint stiffness, in hertz (default `30.0`): the natural
    /// frequency of the contact's penetration-recovery spring. Higher = stiffer
    /// (faster recovery, less squish).
    pub contact_hertz: f32,
    /// Soft-constraint damping ratio ζ (default `10.0`): the contact spring's
    /// damping. `1.0` is critically damped; the Box2D-v3 "Soft Step" default of
    /// `10.0` is heavily overdamped for stable resting contact.
    pub contact_damping: f32,
    /// Broadphase algorithm (default [`BroadphaseKind::AllPairs`] = the shipped
    /// O(n²) loop, byte-identical to today). Set to [`BroadphaseKind::Grid`] to
    /// opt into the O2 uniform-grid broadphase, whose pair set is bit-identical to
    /// all-pairs (the 0%-gate flag — a single runtime branch in
    /// [`physics_broadphase`](crate::systems::physics_broadphase)).
    pub broadphase: BroadphaseKind,
    /// Opt into the O1 AVX2 width-only SoA kernels for the hot per-substep
    /// `refresh_inertia` (`R · I⁻¹_local · Rᵀ`) and the gravity/position/quaternion
    /// integrate loop (default `false`). These are a PURE speed path: each AVX2
    /// lane mirrors the scalar op sequence exactly — exact `mul`/`add`/`sub`/`div`/
    /// `sqrt`, NO FMA contraction, NO `rsqrt`/`rcp` — so the SIMD output is
    /// BIT-IDENTICAL to the scalar path (the `simd_o1` differential proptest is the
    /// gate). The scalar path stays the default and the bit-oracle (the campaign
    /// 0%-gate); when this flag is `false`, or on a non-AVX2 build, the solver runs
    /// the byte-identical scalar kernels. Toggling it changes performance, never
    /// the result.
    pub simd: bool,
    /// Opt into the O7 AVX2 cohort-batched colored CONTACT SOLVE (default `false`),
    /// independent of [`simd`](Self::simd) (which gates only the O1 integrate /
    /// inertia kernels). This is the SEPARATE A/B + rollback knob for the
    /// 8-lane-per-cohort [`solve_color_avx2`](crate::solver::ColoredSoftStepSolver)
    /// kernel: when `true`, the colored solve widens its per-color sweep over
    /// cohorts of 8 body-disjoint manifold-groups; when `false`, it runs the
    /// byte-identical scalar `solve_color` oracle (the O6 0%-gate).
    ///
    /// Like [`simd`](Self::simd) it is a PURE speed path — each AVX2 lane mirrors the
    /// scalar op sequence exactly (exact `mul`/`add`/`sub`/`div`/`sqrt`, NO FMA, NO
    /// `rsqrt`/`rcp`), so the widened solve is BIT-IDENTICAL to the scalar colored
    /// result for any cohort shape and worker count (the differential + the
    /// `{1, N}×{simd}` parallel tests are the gate). It is effective only on the
    /// colored-solve path ([`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver)
    /// driven by [`physics_solve_colored`](crate::systems::physics_solve_colored));
    /// it is a no-op for the shipped [`SoftStepSolver`](crate::solver::SoftStepSolver)
    /// and on a non-AVX2 build / Miri (both arms then run the scalar oracle). Default
    /// OFF so an un-opted world is byte-identical to the O6 colored solve; enabling
    /// the O7 solve needs `simd_solve == true` (it does NOT follow [`simd`](Self::simd)).
    pub simd_solve: bool,
    /// Opt into the O3 PARALLEL candidate EMIT in the
    /// [`BroadphaseGrid`](BroadphaseGrid) (default `false`).
    ///
    /// Effective only on the grid broadphase path
    /// ([`BroadphaseKind::Grid`](BroadphaseKind::Grid)); it is a no-op for the
    /// shipped all-pairs loop. When `true`,
    /// [`physics_broadphase`](crate::systems::physics_broadphase) routes the grid
    /// to [`BroadphaseGrid::build_parallel`](BroadphaseGrid::build_parallel), which
    /// keeps the CSR build (count + prefix-sum + scatter) and the oversized emit
    /// SERIAL and byte-identical to O2 but fans the per-cell candidate EMIT (the
    /// survivor count + emit passes) across the ambient
    /// [`ThreadPool`](boyko_threadpool::ThreadPool)'s workers via `pool.scope`.
    /// Distinct workers own DISJOINT cell ranges, so they write disjoint per-cell
    /// counts (Pass A) and disjoint output sub-ranges (Pass B) — no atomics, no
    /// locks.
    ///
    /// **Bit-identity is the gate:** the parallel emit produces the SAME candidate
    /// pair MULTISET as O2's serial [`build`](BroadphaseGrid::build) (same pairs,
    /// same multiplicity) for ANY worker count and ANY cell-range partition — the
    /// final `(min, max)` sort canonicalizes ORDER, not multiplicity. When `false`
    /// — or when no pool is attached to the running thread, or the body count is
    /// below the parallel threshold — the grid runs the O2 serial
    /// [`build`](BroadphaseGrid::build), byte-identical to O2 (the campaign
    /// 0%-gate). Toggling it changes performance, never the result.
    pub parallel_broadphase: bool,
    /// Opt into building the [`ConstraintGraph`] after narrowphase — constraint
    /// islands + greedy graph coloring (plan O4, Decision 2 / Decision 7).
    ///
    /// When `true`, the [`physics_build_graph`](crate::systems::physics_build_graph)
    /// stage partitions each step's manifolds into islands (connected components
    /// over DYNAMIC bodies, Box2D's ground rule) and greedy-colors them so no color
    /// shares a dynamic body — the enabler for the future colored/SIMD/parallel
    /// solve (O5+). **In O4 the partition is built and validated but NOT consumed:
    /// the shipped [`SoftStepSolver`](crate::solver::SoftStepSolver) still solves
    /// in manifold order**, so the simulation output is byte-identical whether this
    /// flag is on or off (it is a pure pre-compute). The DEFAULT is `false`, so an
    /// un-opted world never runs the stage (the campaign 0%-gate). The colored path
    /// is registered ONLY by
    /// [`add_physics_colored`](crate::plugin::add_physics_colored).
    pub colored: bool,
    /// Opt into the O6 PARALLEL per-color solve (default `false`).
    ///
    /// Effective only on the colored-solve path (the
    /// [`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver) driven by
    /// the [`physics_solve_colored`](crate::systems::physics_solve_colored) stage);
    /// it is a no-op for the shipped
    /// [`SoftStepSolver`](crate::solver::SoftStepSolver). When `true`, each color's
    /// manifold-groups are dispatched across the ambient
    /// [`ThreadPool`](boyko_threadpool::ThreadPool)'s workers via `pool.scope`,
    /// with a barrier (the scope-Drop join) between colors (the Gauss-Seidel sweep
    /// across colors stays sequential). Within a color the groups touch
    /// pairwise-disjoint dynamic bodies (the O4 coloring invariant), so parallel
    /// workers never write the same body — no atomics, no locks.
    ///
    /// **Bit-identity is the gate:** the parallel result is BIT-FOR-BIT identical to
    /// the single-threaded colored solve for ANY worker count (the disjoint-body
    /// partition makes each body's accumulation independent of which worker runs
    /// which group, and the canonical IM-2b warm store is worker-count-independent).
    /// When `false` — or when no pool is attached to the running thread — the
    /// colored solve runs the O5 single-threaded path, BYTE-IDENTICAL to O5 (the
    /// O6 0%-gate). Toggling it changes performance, never the result.
    pub parallel_solve: bool,
    /// Opt into the O8 per-island SLEEPING / deactivation (default `false`).
    ///
    /// Effective only on the colored-solve path (the
    /// [`ColoredSoftStepSolver`](crate::solver::ColoredSoftStepSolver) driven by
    /// [`physics_solve_colored`](crate::systems::physics_solve_colored)) — it consumes
    /// the [`ConstraintGraph`] islands (O4), so it is a no-op for the shipped
    /// [`SoftStepSolver`](crate::solver::SoftStepSolver). When `true`, the solver
    /// tracks a per-island SPEED² metric (`max body |v|²+|ω|²`, mass-INDEPENDENT) with
    /// a per-row debounce counter; an island below
    /// [`sleep_threshold`](Self::sleep_threshold) for
    /// [`sleep_frames`](Self::sleep_frames) consecutive frames is FROZEN and thereafter
    /// SKIPS ONLY its SOLVE + INTEGRATE work — the gather still walks every row (IM-1
    /// intact), so a frozen body keeps its dense-row warm key (no warm-start thrash).
    ///
    /// The sleep state is keyed per BODY ROW (rows are stable across frames), and the
    /// per-frame freeze decision is DERIVED from the rows (an island is frozen iff
    /// every member row is latched asleep). So a slept pile that a faller / new body
    /// joins wakes the SAME frame the contact appears (wake-on-merge), and a topology
    /// change cannot spuriously freeze a moving island.
    ///
    /// **Determinism:** the speed² compare is EXACT (no `sqrt`/`rsqrt`/`algebraic_*`),
    /// the debounce is a per-row integer, and the freeze decision is a pure function of
    /// the per-row latch + this frame's island assignment (no `HashMap`, no volatile-id
    /// carry), so sleeping-ON is run-to-run bit-deterministic. It is NOT bit-equivalent
    /// to sleeping-off (sleeping deliberately stops integrating); the gate is
    /// "sleeping-ON rest state == sleeping-OFF rest state to ε".
    ///
    /// **Default OFF** so an un-opted colored world is BYTE-IDENTICAL to the O6/O7
    /// colored solve (the campaign 0%-gate); enabling it is the entire opt-in.
    pub sleeping: bool,
    /// Per-island SPEED² threshold below which an island is a sleep CANDIDATE (default
    /// [`DEFAULT_SLEEP_THRESHOLD`]); only meaningful when [`sleeping`](Self::sleeping)
    /// is `true`.
    ///
    /// The tracked metric is `max over the island's dynamic bodies of
    /// (|linear_velocity|² + |angular_velocity|²)` — pure speed² + angular speed²,
    /// **mass-INDEPENDENT** (no mass term — a light-fast body has a high `|v|²` and so
    /// correctly stays awake). It is the Box2D-style sleep metric, computed with exact
    /// arithmetic (no `sqrt`). An island whose busiest body is below this for
    /// [`sleep_frames`](Self::sleep_frames) consecutive frames sleeps. Units:
    /// (world-units/s)² + (rad/s)².
    pub sleep_threshold: f32,
    /// Consecutive frames an island must stay below
    /// [`sleep_threshold`](Self::sleep_threshold) before it is put to sleep — the
    /// debounce that prevents wake/sleep oscillation (default
    /// [`DEFAULT_SLEEP_FRAMES`]); only meaningful when [`sleeping`](Self::sleeping)
    /// is `true`.
    pub sleep_frames: u16,
    /// Opt into the SP1 XPBD soft-body position pass (default `false`).
    ///
    /// When `true`, the [`physics_soft_step`](crate::soft::physics_soft_step) stage
    /// — registered only by
    /// [`add_physics_soft`](crate::plugin::add_physics_soft) — advances every
    /// [`SoftBody`](crate::soft::SoftBody) component by one XPBD distance-constraint
    /// step after the rigid solve. It is a STRICTLY DISJOINT integrator: it operates
    /// entirely on the soft-body columns and never touches the rigid
    /// [`SolverScratch`] / `physics_apply`, so the rigid simulation is byte-identical
    /// whether this flag is on or off.
    ///
    /// **Default OFF** so an un-opted world runs no soft-body work (the campaign
    /// 0%-gate); enabling it is the entire opt-in (and a world must also register the
    /// stage via [`add_physics_soft`](crate::plugin::add_physics_soft)).
    pub soft_body: bool,
    /// SP2 D5(a): per-substep VISCOUS velocity damping for soft-body particles
    /// (default `0.0`).
    ///
    /// After each substep's velocity update the velocity is scaled by
    /// `(1.0 - soft_damping)`. The default `0.0` makes this an EXACT `* 1.0`
    /// identity, so an SP1-only world is byte-identical (the SP2 0%-gate). Only
    /// meaningful on the soft path.
    pub soft_damping: f32,
    /// SP2 D5(b): hard rest-velocity FLOOR for soft-body particles (default
    /// `false`).
    ///
    /// When `true`, a particle whose post-damping speed² is below
    /// [`REST_CLAMP_EPS²`](crate::soft::solver::REST_CLAMP_EPS) has its velocity
    /// zeroed (a squared compare, no `sqrt`), killing residual creep at rest.
    /// Default OFF so an un-opted world never floors (the SP2 0%-gate). Only
    /// meaningful on the soft path.
    pub soft_rest_clamp: bool,
    /// SP2 D6/D7: two-way soft↔rigid COUPLING (default `false`).
    ///
    /// When `true` (AND the pipeline was wired with coupling via
    /// [`add_physics_soft`](crate::plugin::add_physics_soft) `coupling == true`), the
    /// coupled soft step resolves per-particle soft-vs-rigid collisions against the
    /// rigid frame-N snapshot and applies the equal-and-opposite reaction to the
    /// rigid bodies AFTER `physics_apply`. Default OFF so an un-opted world is
    /// byte-identical (the SP2 0%-gate); the coupling is a pure opt-in. The flag is
    /// read by the coupled step — when the pipeline is wired for coupling but this
    /// flag is `false`, the coupled step behaves exactly like the non-coupling step.
    pub soft_rigid_coupling: bool,
    /// SP3: number of Gauss-Seidel SELF-COLLISION sweeps per substep (default `0`).
    ///
    /// When `> 0`, the soft step runs a SAME-BODY particle-vs-particle self-collision
    /// pass (a per-body open-addressed spatial hash + push-to-`2·radius` PBD distance
    /// constraint) for this many GS sweeps AFTER the volume sweep and BEFORE the
    /// soft↔rigid coupling. Default `0` makes the pass an early-return no-op before
    /// any hashing, so an SP1/SP2 world is byte-identical (the SP3 0%-gate). Only
    /// meaningful on the soft path, and only for a body with `particle_radius > 0`.
    pub self_collision_iters: usize,
}

/// Default per-island sleep SPEED² threshold (plan O8 / Decision 5).
///
/// The metric is `|v|² + |ω|²` (speed² + angular speed², mass-INDEPENDENT) in
/// (world-units/s)² + (rad/s)². Tuned conservative (a body must be nearly still) so a
/// slow-creeping stack does not falsely sleep; the gate "rest state == no-sleep rest
/// state to ε" tolerates the small residual.
pub const DEFAULT_SLEEP_THRESHOLD: f32 = 1.0e-4;

/// Default consecutive-frame debounce before an island sleeps (plan O8) — half a
/// second at 120 Hz, long enough that a transient low-speed frame does not sleep a
/// still-settling stack (the no-oscillation gate).
pub const DEFAULT_SLEEP_FRAMES: u16 = 60;

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            // Earth-like downward gravity by default (−Y is "down").
            gravity: Vec3::new(0.0, -9.81, 0.0),
            // Stamped by `physics_gather` from `FixedTime` every step before any
            // solver reads it; the integrate->gather->solve `.after` chain
            // (plugin.rs) guarantees gather precedes the solve, so this `0.0`
            // placeholder is never the value a solver actually sees.
            dt: 0.0,
            substeps: 4,
            relax_iterations: 2,
            contact_hertz: 30.0,
            contact_damping: 10.0,
            // Default to the shipped O(n²) loop so an un-opted world is
            // byte-identical to today (the campaign 0%-gate).
            broadphase: BroadphaseKind::AllPairs,
            // Default OFF so an un-opted world runs the scalar bit-oracle kernels
            // (the campaign 0%-gate); the SIMD path is a pure opt-in speed path.
            simd: false,
            // Default OFF (independent of `simd`) so the colored solve runs the
            // byte-identical scalar `solve_color` oracle (the O6 0%-gate); the O7
            // cohort-batched solve is a pure opt-in speed path with a bit-identical
            // result. Enabling it requires `simd_solve == true` explicitly.
            simd_solve: false,
            // Default OFF so an un-opted grid world runs the O2 serial `build`,
            // byte-identical to O2 (the campaign 0%-gate); the parallel emit is a
            // pure opt-in speed path with a bit-identical pair multiset.
            parallel_broadphase: false,
            // Default OFF so an un-opted world never builds the constraint graph
            // (the campaign 0%-gate); O4 only PRODUCES the partition — the solve is
            // byte-identical whether on or off.
            colored: false,
            // Default OFF so the colored solve runs the O5 single-threaded path,
            // BYTE-IDENTICAL to O5 (the O6 0%-gate); the parallel dispatch is a pure
            // opt-in speed path with a bit-identical result.
            parallel_solve: false,
            // Default OFF so an un-opted colored world is BYTE-IDENTICAL to the O6/O7
            // colored solve (the campaign 0%-gate); sleeping is a pure opt-in.
            sleeping: false,
            sleep_threshold: DEFAULT_SLEEP_THRESHOLD,
            sleep_frames: DEFAULT_SLEEP_FRAMES,
            // Default OFF so an un-opted world runs no soft-body work (the campaign
            // 0%-gate); the SP1 XPBD pass is a pure opt-in.
            soft_body: false,
            // SP2 D5(a): default `0.0` ⇒ the viscous scale is an exact `* 1.0`
            // identity, so an SP1-only world is byte-identical (the SP2 0%-gate).
            soft_damping: 0.0,
            // SP2 D5(b): default OFF so an un-opted world never floors velocity.
            soft_rest_clamp: false,
            // SP2 D6/D7: default OFF so an un-opted world has no soft↔rigid coupling
            // (the SP2 0%-gate); the coupling is a pure opt-in.
            soft_rigid_coupling: false,
            // SP3: default `0` ⇒ the self-collision pass early-returns before any
            // hashing, so an SP1/SP2 world is byte-identical (the SP3 0%-gate).
            self_collision_iters: 0,
        }
    }
}

/// Whether the pipeline's [`physics_integrate`](crate::systems::physics_integrate)
/// stage integrates, or the solver owns integration (C2).
///
/// Inserted by [`add_physics_systems`](crate::plugin::add_physics_systems) from
/// the chosen solver's
/// [`RigidSolver::owns_integration`](crate::solver::RigidSolver::owns_integration):
/// an owning TGS solver (the [`SoftStepSolver`](crate::solver::SoftStepSolver))
/// integrates DYNAMIC bodies inside its own substep loop, so the pipeline stage
/// must early-return to avoid double-integration. See the C2 contract block in
/// [`crate::systems`].
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntegrationMode {
    /// The pipeline's [`physics_integrate`](crate::systems::physics_integrate)
    /// stage integrates every body (the foundation default, used by the no-op /
    /// non-owning solvers).
    #[default]
    Foundation,
    /// The solver owns the substep integration (a TGS solver); the pipeline
    /// stage early-returns so it does NOT also integrate.
    SolverOwned,
}

/// Candidate collision pairs emitted by broadphase (plan D4).
///
/// Each pair is `(BodyIndex, BodyIndex)` keyed by the dense scratch row index
/// (IM-1). The list is sorted deterministically by `(min, max)` (D4) so contact
/// iteration order is reproducible (float add is non-associative). The `Vec` is
/// cleared and refilled each step, capacity reused.
#[derive(Resource, Default)]
pub struct ContactPairs {
    /// Candidate pairs in deterministic `(min, max)` order.
    pub pairs: Vec<(BodyIndex, BodyIndex)>,
}

impl ContactPairs {
    /// Builds an empty pair buffer pre-sized for `capacity` pairs.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pairs: Vec::with_capacity(capacity),
        }
    }
}

/// A body whose AABB spans more than [`MAX_CELL_SPAN`] grid cells goes here
/// instead of bucketing into every cell it touches (the standard uniform-grid
/// escape hatch, Decision 1 trade-off). An oversized body is tested against every
/// other body — an `n·k` residual where `k` is the oversized count — so a single
/// giant body among many tiny ones cannot blow up the histogram into millions of
/// cells. A body is "oversized" when its span along ANY axis exceeds this.
pub const MAX_CELL_SPAN: u32 = 8;

/// Hard ceiling on the total grid cell count (`dims.x · dims.y · dims.z`), so a
/// pathological extent/cell-size ratio cannot demand an unbounded histogram. When
/// the closed-form proxy would exceed this, the cell size is grown (dims clamped)
/// — coarser cells just mean larger per-cell slices, still correct.
pub const MAX_GRID_CELLS: u32 = 1 << 21;

/// Lower bound on a grid dimension (at least one cell per axis, so an empty or
/// flat-on-one-axis world still has a valid `1 × … ×` grid).
const MIN_GRID_DIM: u32 = 1;

/// O3 perf knob: how many cell-range chunks to emit per ambient worker lane on the
/// parallel candidate-emit passes (Pass A count / Pass B emit).
///
/// `boyko_threadpool` is a Chase-Lev work-STEALING pool — emitting MORE (and
/// smaller) chunks than lanes gives every lane several steal-sized tasks so the
/// scheduler equalizes them (an idle lane steals the next chunk). The chunk COUNT
/// and SHAPE are pure perf knobs: the candidate pair MULTISET is partition-
/// independent (distinct chunks own disjoint cell ranges, and a multi-cell pair is
/// emitted only at its minimum shared cell — a pure function of the two AABBs), so
/// tuning this changes only WHERE work runs, never the bits.
const CHUNKS_PER_WORKER: usize = 4;

/// O3 min-work threshold: below this body count the parallel emit's `pool.scope`
/// dispatch (a boxed shared frame + a boxed closure per spawn) costs more than it
/// saves, so [`BroadphaseGrid::build_parallel`] runs the emit-SHAPED path at one
/// chunk (no pool) — bit-identical to the parallel split (the multiset is partition-
/// independent), changing only WHERE the emit runs. A starting point the tester
/// benches.
const MIN_PARALLEL_BODIES: usize = 4096;

/// Uniform spatial grid broadphase, CSR counting-sort (plan O2, Decision 1).
///
/// Replaces the O(n²) all-pairs ITERATION (NOT the feasibility predicate): the
/// grid buckets every body into the cells its AABB spans, emits candidate pairs
/// (bodies sharing ≥ 1 cell, deduplicated), then applies the SAME sphere-bound
/// feasibility test all-pairs uses and sorts the survivors by `(min, max)` — so
/// the output [`ContactPairs`] set is bit-identical to all-pairs.
///
/// # Layout (CSR, capacity-reused — no per-step alloc in steady state)
///
/// `counts[c]` is the per-cell body count (the histogram); the exclusive
/// prefix-sum of `counts` is `cell_start`, so `cell_start[c]..cell_start[c + 1]`
/// indexes the bodies bucketed into cell `c` inside the flat `cell_bodies`. Every
/// `Vec` is `clear()`-ed and refilled each build — capacity is reused, never
/// `Vec::new` per step (principle 5).
///
/// The zero-per-step-allocation guarantee holds in **steady state**: a stable
/// scene geometry (body count + world AABB → cell count bounded by the warmed
/// capacities). A *growing* world — more bodies, or a larger AABB / finer cell
/// size demanding more cells — reallocates the affected buffers on the growth
/// frames (the capacity then settles and is reused). It is never a `Vec::new` per
/// step on an unchanged scene.
///
/// # Determinism
///
/// Bodies scatter in DENSE-ROW order, so within a cell the body slice is
/// row-sorted; candidates emit by scanning cells in ascending cell index then
/// rows in ascending order; the dedup rule ("emit a multi-cell pair only at its
/// minimum shared cell") is a pure function of the two AABBs; the surviving pairs
/// are sorted by `(min, max)`. The cell-size proxy is a closed-form
/// `extent / n^(1/3)` floored at the *median* body diameter (an order statistic
/// — a pure function of the radius multiset, not a sampled/seeded median), so the
/// grid geometry is a deterministic pure function of the body positions + radii.
///
/// Note (output-neutrality): the cell-size proxy is OUTPUT-SET-NEUTRAL — for ANY
/// cell size the feasibility-filtered `(min, max)` pair set is identical (it only
/// changes which cells a pair is bucketed into, never the surviving pairs). The
/// proxy is kept deterministic anyway for clean reasoning and the determinism
/// gate. The result is bit-identical run-to-run AND bit-identical to all-pairs.
#[derive(Resource, Default)]
pub struct BroadphaseGrid {
    /// Exclusive prefix sums of `counts`; `len == n_cells + 1`. CSR offsets.
    cell_start: Vec<u32>,
    /// Body rows bucketed by cell (the scatter target). CSR values.
    cell_bodies: Vec<u32>,
    /// Per-cell body-count histogram, reused; rebuilt then prefix-summed each
    /// build.
    counts: Vec<u32>,
    /// A running write cursor per cell during scatter (a working copy of
    /// `cell_start`), reused across builds.
    cursor: Vec<u32>,
    /// Bodies spanning more than [`MAX_CELL_SPAN`] cells on some axis — tested
    /// against every body, never bucketed (the size-disparity escape hatch).
    oversized: Vec<u32>,
    /// Scratch copy of the per-body bounding radii, reused across builds; used to
    /// compute the deterministic median radius (the typical-body cell-size proxy,
    /// O2 W1). `select_nth_unstable` reorders this in place — that is why it is a
    /// throwaway scratch buffer, not read after the median is taken.
    scratch_radii: Vec<f32>,
    /// Pre-filter candidate pairs (before the sphere-bound test), reused. Used
    /// only by the serial [`build`](Self::build); the parallel
    /// [`build_parallel`](Self::build_parallel) emits feasibility-filtered
    /// survivors straight into `out`.
    candidates: Vec<(BodyIndex, BodyIndex)>,
    /// O3 parallel emit (Pass A): per-cell SURVIVING-pair count (the count of
    /// within-cell pairs `(i, j)` with `min_shared_cell == c && feasible`),
    /// `len == n_cells`. Reused (clear + resize each parallel build).
    pair_count: Vec<u32>,
    /// O3 parallel emit: exclusive prefix-sum of `pair_count`, `len == n_cells + 1`
    /// — so `pair_offset[c]..pair_offset[c + 1]` is cell `c`'s contiguous out
    /// sub-range and `pair_offset[n_cells]` is the total survivor count. Reused.
    pair_offset: Vec<u32>,
    /// World-space origin of cell `(0, 0, 0)` (the AABB min corner).
    origin: Vec3,
    /// Reciprocal of the cell edge length, so a coordinate maps to a cell index by
    /// a multiply (`floor((p - origin) · inv_cell)`), no divide in the hot loop.
    inv_cell: f32,
    /// Grid resolution along each axis (`dims.x · dims.y · dims.z == n_cells`).
    dims: [u32; 3],
}

impl BroadphaseGrid {
    /// Builds an empty grid pre-sized for `capacity` bodies (no later realloc in
    /// steady state). The cell buffers grow on the first build to the live cell
    /// count and reuse that capacity thereafter.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cell_start: Vec::new(),
            cell_bodies: Vec::with_capacity(capacity),
            counts: Vec::new(),
            cursor: Vec::new(),
            oversized: Vec::with_capacity(capacity),
            scratch_radii: Vec::with_capacity(capacity),
            candidates: Vec::with_capacity(capacity),
            // The parallel-emit CSR scratch grows on the first parallel build to the
            // live cell count and reuses that capacity thereafter (like every other
            // cell-indexed buffer here); a fresh `Vec` is the cheap first reserve.
            pair_count: Vec::new(),
            pair_offset: Vec::new(),
            origin: Vec3::ZERO,
            inv_cell: 1.0,
            dims: [1, 1, 1],
        }
    }

    /// Total cell count for the current frame's grid (`dims` product).
    #[inline]
    fn n_cells(&self) -> usize {
        self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize
    }

    /// The linear cell index a world position falls into (SP2 D6 coupling
    /// accessor) — wraps the private [`cell_coord`](Self::cell_coord) +
    /// [`cell_index`](Self::cell_index). Always in `[0, n_cells)` (the coordinate
    /// is clamped per axis).
    #[inline]
    pub fn cell_of(&self, p: Vec3) -> u32 {
        self.cell_index(self.cell_coord(p))
    }

    /// The grid resolution along each axis (`dims.x · dims.y · dims.z ==
    /// n_cells`) — SP2 D6 coupling accessor for the 27-cell neighbourhood walk.
    #[inline]
    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    /// `true` once a [`build`](Self::build) / [`build_parallel`](Self::build_parallel)
    /// has populated the CSR (`cell_start` is `n_cells + 1` long); `false` for a
    /// freshly [`with_capacity`](Self::with_capacity)-constructed grid that has not
    /// been built (`cell_start` empty).
    ///
    /// SP2 M1 coupling precondition probe: the coupled soft step
    /// ([`physics_soft_step_coupled`](crate::soft::physics_soft_step_coupled))
    /// `debug_assert!`s this when coupling is on and bodies exist, since its
    /// `deepest_contact` reads the CSR slices — an unbuilt grid would be a silent
    /// no-op (see `add_physics_pipeline`, which forces
    /// [`BroadphaseKind::Grid`](BroadphaseKind) on the coupling path).
    #[inline]
    pub fn is_built(&self) -> bool {
        !self.cell_start.is_empty()
    }

    /// The bodies bucketed into cell `cell`, in ascending dense-row order (SP2 D6
    /// coupling accessor) — the CSR slice `cell_bodies[cell_start[cell]..cell_start
    /// [cell + 1]]`.
    ///
    /// Returns an empty slice when the grid has not been built yet
    /// (`cell_start.is_empty()` — an empty world) or `cell` is out of range, so the
    /// caller can iterate any candidate cell index without a separate guard.
    #[inline]
    pub fn cell_body_slice(&self, cell: u32) -> &[u32] {
        let c = cell as usize;
        // `cell_start` is `n_cells + 1` long after a build; empty before the first
        // build. Guard both so an out-of-range / pre-build cell yields `&[]`.
        if c + 1 >= self.cell_start.len() {
            return &[];
        }
        let start = self.cell_start[c] as usize;
        let end = self.cell_start[c + 1] as usize;
        &self.cell_bodies[start..end]
    }

    /// The oversized bodies — those spanning more than [`MAX_CELL_SPAN`] cells on
    /// some axis, never bucketed into a cell (SP2 D6 coupling accessor). The
    /// coupling walks these as a separate pass alongside the 27-cell neighbourhood.
    #[inline]
    pub fn oversized_slice(&self) -> &[u32] {
        &self.oversized
    }

    /// Maps a world position to its integer cell coordinate, clamped into
    /// `[0, dims)` per axis (a body exactly on or past the AABB max corner clamps
    /// to the last cell — never out of range).
    #[inline]
    fn cell_coord(&self, p: Vec3) -> [u32; 3] {
        let rel = p - self.origin;
        // `inv_cell > 0`, so the product is finite; the explicit clamp covers FP
        // rounding at the boundary and the (degenerate) negative-relative case.
        let to_cell = |v: f32, dim: u32| -> u32 {
            let idx = (v * self.inv_cell).floor();
            if idx <= 0.0 {
                0
            } else {
                (idx as u32).min(dim - 1)
            }
        };
        [
            to_cell(rel.x, self.dims[0]),
            to_cell(rel.y, self.dims[1]),
            to_cell(rel.z, self.dims[2]),
        ]
    }

    /// Flattens an integer cell coordinate to a linear cell index
    /// (`x + dims.x · (y + dims.y · z)`), the canonical row-major order the build
    /// scans in.
    #[inline]
    fn cell_index(&self, c: [u32; 3]) -> u32 {
        c[0] + self.dims[0] * (c[1] + self.dims[1] * c[2])
    }

    /// Inclusive cell-coordinate range `[lo, hi]` (per axis) an AABB spans.
    #[inline]
    fn cell_range(&self, min: Vec3, max: Vec3) -> ([u32; 3], [u32; 3]) {
        (self.cell_coord(min), self.cell_coord(max))
    }

    /// Recomputes the grid geometry (`origin`, `inv_cell`, `dims`) from the
    /// bodies' world AABB and a closed-form cell-size proxy (plan O2 step 1, W1).
    ///
    /// The cell size is `extent_max / n^(1/3)` (a body-count-balanced uniform
    /// grid), floored at the *typical* body diameter — `2 · median_radius` — NOT
    /// the largest body's diameter. Flooring at the typical body keeps the grid
    /// well-resolved for the many: a single huge body no longer forces giant cells
    /// that would coarsen the whole grid toward all-pairs within those cells (the
    /// size-disparity pathology). A genuinely outsized body (diameter ≫ a typical
    /// cell) then spans ≥ [`MAX_CELL_SPAN`] cells and is correctly routed to the
    /// `oversized` `n·k` escape hatch.
    ///
    /// The median is an *order statistic* over the per-body radii: a pure function
    /// of the radius multiset (independent of the bodies' enumeration order), so it
    /// is deterministic run-to-run and cross-run — it is NOT a sampled or seeded
    /// median. It is computed via `select_nth_unstable` over the reused
    /// [`scratch_radii`](Self::scratch_radii) buffer (alloc-free in steady state).
    ///
    /// `dims` is clamped up if it would exceed [`MAX_GRID_CELLS`]. With no bodies
    /// the grid is the degenerate `1 × 1 × 1`.
    fn recompute_geometry(&mut self, bodies: &[BodyState]) {
        if bodies.is_empty() {
            self.origin = Vec3::ZERO;
            self.inv_cell = 1.0;
            self.dims = [MIN_GRID_DIM, MIN_GRID_DIM, MIN_GRID_DIM];
            return;
        }

        // One linear pass over the bodies: the world AABB (position ± bounding
        // radius), and the per-body radii into the reused scratch buffer (the
        // median of which is the typical-body cell-size floor input).
        let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        self.scratch_radii.clear();
        self.scratch_radii.reserve(bodies.len());
        for b in bodies {
            let r = body_bounding_radius(b);
            self.scratch_radii.push(r);
            let p = b.position;
            min.x = min.x.min(p.x - r);
            min.y = min.y.min(p.y - r);
            min.z = min.z.min(p.z - r);
            max.x = max.x.max(p.x + r);
            max.y = max.y.max(p.y + r);
            max.z = max.z.max(p.z + r);
        }

        // Deterministic median radius: the (n/2)-th order statistic of the radius
        // multiset. `select_nth_unstable` returns the element that WOULD sit at the
        // pivot index in a fully sorted order — that value is a pure function of the
        // multiset, independent of `scratch_radii`'s current order, so it is
        // bit-stable run-to-run and cross-run (NOT a sampled/seeded median). It
        // reorders `scratch_radii` in place, which is fine: the buffer is throwaway
        // scratch. `partial_cmp.unwrap()` is sound here — every radius is finite
        // (a `body_bounding_radius` of a finite shape; a non-finite shape would
        // already have collapsed the extent below), so no NaN reaches the compare.
        let mid = self.scratch_radii.len() / 2;
        let (_, median, _) = self
            .scratch_radii
            .select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_radius = *median;

        // Clamp the extent to a finite, non-negative box before any cell-count
        // arithmetic: a diverged solver can emit a ±Inf/NaN `BodyState.position`,
        // which would make `extent` non-finite and `dim_of`'s `(ext / cell_size)
        // .floor() as i64` saturate to `i64::MAX` → the `+ 1` overflows and panics
        // in a debug build. The all-pairs arm simply yields no feasible pair for
        // the same input (an Inf/NaN delta fails `length_squared() <= bound²`), so
        // clamping here makes the Grid path at least as tolerant as AllPairs (M1):
        // a non-finite axis collapses to a degenerate-but-safe single cell, never a
        // panic. The output set is unchanged for finite inputs.
        let finite_axis = |lo: f32, hi: f32| -> f32 {
            let d = hi - lo;
            if d.is_finite() { d.max(0.0) } else { 0.0 }
        };
        let extent = Vec3::new(
            finite_axis(min.x, max.x),
            finite_axis(min.y, max.y),
            finite_axis(min.z, max.z),
        );
        let extent_max = extent.x.max(extent.y).max(extent.z).max(0.0);

        // Closed-form proxy: split the largest extent into ~n^(1/3) cells so the
        // average occupancy is ~1 body/cell. Floor the cell size at the TYPICAL
        // body DIAMETER (`2 · median_radius`), decoupled from the largest body
        // (O2 W1): a body whose diameter is ≫ the typical cell then genuinely
        // spans ≥ MAX_CELL_SPAN cells and is routed to the oversized hatch, instead
        // of forcing giant cells that would coarsen the whole grid toward all-pairs
        // (the size-disparity pathology). The floor only stops cells from going
        // absurdly fine for the typical body; the MAX_GRID_CELLS cap below prevents
        // them from going too fine overall.
        let n = bodies.len() as f32;
        let cbrt_n = n.cbrt().max(1.0);
        let median_floor = 2.0 * median_radius;
        let mut cell_size = (extent_max / cbrt_n).max(median_floor);
        // A degenerate world (all bodies one point with zero radius → zero extent
        // AND zero median) yields a ≤ 0 / non-finite cell size; floor it at a small
        // positive minimum. An all-same-radius scene is handled naturally: the
        // median equals that radius, so the typical body spans ~1 cell and none is
        // oversized.
        if !cell_size.is_finite() || cell_size <= 0.0 {
            cell_size = 1.0;
        }

        // Provisional dims from the cell size, at least one cell per axis.
        let dim_of = |ext: f32| -> u32 {
            let d = (ext / cell_size).floor() as i64 + 1;
            d.clamp(MIN_GRID_DIM as i64, u32::MAX as i64) as u32
        };
        let mut dims = [dim_of(extent.x), dim_of(extent.y), dim_of(extent.z)];

        // Clamp the total cell count: if the proxy would demand more than
        // MAX_GRID_CELLS, grow the cell size uniformly (coarser cells = larger
        // slices, still correct) and recompute dims.
        let mut total = dims[0] as u64 * dims[1] as u64 * dims[2] as u64;
        if total > MAX_GRID_CELLS as u64 {
            // Scale the cell size by the cube root of the overshoot so the product
            // lands at/under the ceiling, then recompute dims.
            let overshoot = total as f32 / MAX_GRID_CELLS as f32;
            cell_size *= overshoot.cbrt();
            let dim_of2 = |ext: f32| -> u32 {
                let d = (ext / cell_size).floor() as i64 + 1;
                d.clamp(MIN_GRID_DIM as i64, u32::MAX as i64) as u32
            };
            dims = [dim_of2(extent.x), dim_of2(extent.y), dim_of2(extent.z)];
            total = dims[0] as u64 * dims[1] as u64 * dims[2] as u64;
            // A residual overshoot (axis-aligned slabs) collapses the most
            // populous axis until the product fits — correctness is preserved
            // (coarser still over-approximates the candidate set).
            while total > MAX_GRID_CELLS as u64 {
                let widest = if dims[0] >= dims[1] && dims[0] >= dims[2] {
                    0
                } else if dims[1] >= dims[2] {
                    1
                } else {
                    2
                };
                dims[widest] = (dims[widest] / 2).max(MIN_GRID_DIM);
                total = dims[0] as u64 * dims[1] as u64 * dims[2] as u64;
            }
        }

        self.origin = min;
        self.inv_cell = 1.0 / cell_size;
        self.dims = dims;
    }

    /// Builds the grid over `bodies` and writes the feasibility-filtered,
    /// `(min, max)`-sorted candidate pairs into `out` (plan O2 steps 1–6).
    ///
    /// `out` is cleared then refilled; the pair set is bit-identical to all-pairs
    /// over the SAME [`body_bounding_radius`]-based sphere-bound predicate
    /// (`delta.length_squared() <= (rA + rB)²`). All scratch buffers are
    /// capacity-reused — no per-step heap allocation once warmed.
    pub fn build(&mut self, bodies: &[BodyState], out: &mut Vec<(BodyIndex, BodyIndex)>) {
        out.clear();
        self.candidates.clear();

        let n = bodies.len();
        if n == 0 {
            self.oversized.clear();
            return;
        }

        // (1–4) AABB + cell-size proxy, count, prefix-sum, scatter (the CSR build).
        self.build_csr(bodies);

        // (5) Candidate emit: per cell, all-pairs within its (small) body slice,
        // deduped so a pair sharing ≥ 2 cells emits only at its MINIMUM shared
        // cell; then each oversized body vs every body.
        self.emit_cell_candidates(bodies);
        self.emit_oversized_candidates(bodies, n);

        // (6) Feasibility filter (the SAME sphere-bound predicate as all-pairs) +
        // sort by (min, max). Bit-identical to the all-pairs output set.
        for &(a, b) in &self.candidates {
            let ia = a.0 as usize;
            let ib = b.0 as usize;
            if Self::feasible(&bodies[ia], &bodies[ib]) {
                out.push((a, b));
            }
        }
        out.sort_unstable();
    }

    /// The serial CSR build shared by [`build`](Self::build) and
    /// [`build_parallel`](Self::build_parallel) (plan O2 steps 1–4): recompute the
    /// grid geometry, count bodies per cell (routing oversized bodies to the escape
    /// hatch), exclusive-prefix-sum the counts into `cell_start`, then scatter rows
    /// into `cell_bodies` in dense-row order.
    ///
    /// Extracted VERBATIM from the original [`build`](Self::build) so the two paths'
    /// CSR (and thus `cell_bodies`) is byte-identical by construction — the O3 scope
    /// decision rests on this. Clears `oversized` first; the caller clears `out` /
    /// any candidate buffer. `bodies` must be non-empty (the caller early-returns on
    /// an empty world).
    fn build_csr(&mut self, bodies: &[BodyState]) {
        self.oversized.clear();

        // (1) AABB + closed-form cell-size proxy.
        self.recompute_geometry(bodies);
        let n_cells = self.n_cells();

        // (2) Count: per body, +1 to every cell its AABB spans; an AABB spanning
        // more than MAX_CELL_SPAN cells on any axis goes to `oversized` instead.
        self.counts.clear();
        self.counts.resize(n_cells, 0);
        for (row, b) in bodies.iter().enumerate() {
            let r = body_bounding_radius(b);
            let half = Vec3::new(r, r, r);
            let (lo, hi) = self.cell_range(b.position - half, b.position + half);
            if Self::is_oversized(lo, hi) {
                self.oversized.push(row as u32);
                continue;
            }
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let c = self.cell_index([x, y, z]) as usize;
                        self.counts[c] += 1;
                    }
                }
            }
        }

        // (3) Exclusive prefix-sum counts → cell_start (len n_cells + 1).
        self.cell_start.clear();
        self.cell_start.reserve(n_cells + 1);
        let mut acc = 0u32;
        self.cell_start.push(0);
        for &c in &self.counts {
            acc += c;
            self.cell_start.push(acc);
        }
        let total_inserts = acc as usize;

        // (4) Scatter rows into cell_bodies at cursor[cell]++ (a working copy of
        // cell_start), in dense-row order so each cell slice is row-sorted.
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.cell_start[..n_cells]);
        self.cell_bodies.clear();
        self.cell_bodies.resize(total_inserts, 0);
        for (row, b) in bodies.iter().enumerate() {
            let r = body_bounding_radius(b);
            let half = Vec3::new(r, r, r);
            let (lo, hi) = self.cell_range(b.position - half, b.position + half);
            if Self::is_oversized(lo, hi) {
                continue;
            }
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let c = self.cell_index([x, y, z]) as usize;
                        let slot = self.cursor[c] as usize;
                        self.cell_bodies[slot] = row as u32;
                        self.cursor[c] += 1;
                    }
                }
            }
        }
    }

    /// `true` when an AABB cell range spans more than [`MAX_CELL_SPAN`] cells on
    /// any axis (the oversized escape-hatch test).
    #[inline]
    fn is_oversized(lo: [u32; 3], hi: [u32; 3]) -> bool {
        (hi[0] - lo[0]) >= MAX_CELL_SPAN
            || (hi[1] - lo[1]) >= MAX_CELL_SPAN
            || (hi[2] - lo[2]) >= MAX_CELL_SPAN
    }

    /// The SAME sphere-bound feasibility predicate the all-pairs path uses
    /// (`delta.length_squared() <= (rA + rB)²`) — the O2 0%-correctness contract.
    #[inline]
    fn feasible(a: &BodyState, b: &BodyState) -> bool {
        let bound = body_bounding_radius(a) + body_bounding_radius(b);
        let delta = b.position - a.position;
        delta.length_squared() <= bound * bound
    }

    /// Emits within-cell all-pairs candidates, deduped to the minimum shared cell
    /// (plan O2 step 5). For a pair `(i, j)` found together in cell `c`, the pair
    /// is emitted only when `c` equals the FIRST (lowest-index) cell both bodies'
    /// AABB ranges share — so a pair co-occupying several cells is emitted exactly
    /// once, matching the all-pairs "each unordered pair once" set.
    fn emit_cell_candidates(&mut self, bodies: &[BodyState]) {
        let n_cells = self.n_cells();
        for c in 0..n_cells {
            let start = self.cell_start[c] as usize;
            let end = self.cell_start[c + 1] as usize;
            let slice = &self.cell_bodies[start..end];
            // Bodies are row-sorted within the slice, so `(slice[p], slice[q])`
            // with p < q is already `(min, max)`.
            for p in 0..slice.len() {
                let i = slice[p];
                for &j in &slice[p + 1..] {
                    if self.min_shared_cell(&bodies[i as usize], &bodies[j as usize]) == c as u32 {
                        self.candidates.push((BodyIndex(i), BodyIndex(j)));
                    }
                }
            }
        }
    }

    /// Emits oversized-body candidates: each oversized body against every other
    /// body (the `n·k` residual). Oversized bodies are never bucketed, so a pair
    /// with at least one oversized side is emitted ONLY here, never by the cell
    /// pass (the two emit sources are disjoint). An oversized–oversized pair would
    /// be reachable from both sides, so it is emitted only from the lower row
    /// (`o < j`); an oversized–normal pair is reachable only from the oversized
    /// side, so it is always emitted. Each pair is keyed `(min, max)`.
    fn emit_oversized_candidates(&mut self, _bodies: &[BodyState], n: usize) {
        // `oversized` is ascending (rows pushed in dense-row order).
        for idx in 0..self.oversized.len() {
            let o = self.oversized[idx];
            for j in 0..n as u32 {
                if j == o {
                    continue;
                }
                // Dedup oversized–oversized: emit it only from the lower row.
                if Self::row_is_oversized(&self.oversized, j) && j < o {
                    continue;
                }
                let (lo, hi) = if o < j { (o, j) } else { (j, o) };
                self.candidates.push((BodyIndex(lo), BodyIndex(hi)));
            }
        }
    }

    /// `true` if dense row `row` is in the (ascending) `oversized` list.
    #[inline]
    fn row_is_oversized(oversized: &[u32], row: u32) -> bool {
        oversized.binary_search(&row).is_ok()
    }

    /// The lowest linear cell index shared by both bodies' AABB cell ranges, or
    /// `u32::MAX` if they share none (then the within-cell dedup never emits them
    /// — they were only co-located by a clamp artifact, which cannot happen since
    /// they were found in a common cell). The dedup keys on this so a pair sharing
    /// several cells emits exactly once.
    #[inline]
    fn min_shared_cell(&self, a: &BodyState, b: &BodyState) -> u32 {
        let ra = body_bounding_radius(a);
        let rb = body_bounding_radius(b);
        let (alo, ahi) = self.cell_range(
            a.position - Vec3::new(ra, ra, ra),
            a.position + Vec3::new(ra, ra, ra),
        );
        let (blo, bhi) = self.cell_range(
            b.position - Vec3::new(rb, rb, rb),
            b.position + Vec3::new(rb, rb, rb),
        );
        // Per-axis overlap of the two cell ranges; the min shared cell is the
        // overlap's low corner (row-major flattened). If any axis has no overlap
        // there is no shared cell.
        let ox_lo = alo[0].max(blo[0]);
        let oy_lo = alo[1].max(blo[1]);
        let oz_lo = alo[2].max(blo[2]);
        let ox_hi = ahi[0].min(bhi[0]);
        let oy_hi = ahi[1].min(bhi[1]);
        let oz_hi = ahi[2].min(bhi[2]);
        if ox_lo > ox_hi || oy_lo > oy_hi || oz_lo > oz_hi {
            return u32::MAX;
        }
        self.cell_index([ox_lo, oy_lo, oz_lo])
    }

    /// Counts the SURVIVING within-cell candidate pairs of cell `cell` (plan O3
    /// Pass A): the within-cell all-pairs `(i, j)` (`i < j` in the row-sorted slice)
    /// that the cell-emit dedup keys to THIS cell AND that pass the sphere-bound
    /// feasibility predicate.
    ///
    /// Walks cell `cell`'s `cell_bodies[cell_start[c]..cell_start[c + 1]]` slice in
    /// the SAME `for p { for q in p+1.. }` nesting as the serial
    /// [`emit_cell_candidates`](Self::emit_cell_candidates), applying the LITERAL
    /// [`min_shared_cell`](Self::min_shared_cell)`== cell` dedup AND the LITERAL
    /// [`feasible`](Self::feasible) filter (the feasibility filter is FOLDED into the
    /// count, so the count equals the number of survivors
    /// [`emit_cell_pairs`](Self::emit_cell_pairs) writes — the C2 coupling). The two
    /// helpers MUST stay in lock-step: same predicates, same order, so the parallel
    /// emit's multiset matches O2's serial `build` bit-for-bit (float
    /// non-associativity makes re-deriving the predicate inline unsound — they call
    /// the one literal source).
    fn count_cell_pairs(&self, cell: u32, bodies: &[BodyState]) -> u32 {
        let c = cell as usize;
        let start = self.cell_start[c] as usize;
        let end = self.cell_start[c + 1] as usize;
        let slice = &self.cell_bodies[start..end];
        let mut count = 0u32;
        for p in 0..slice.len() {
            let i = slice[p];
            for &j in &slice[p + 1..] {
                let a = &bodies[i as usize];
                let b = &bodies[j as usize];
                if self.min_shared_cell(a, b) == cell && Self::feasible(a, b) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Emits the SURVIVING within-cell candidate pairs of cell `cell` into
    /// `out_slice`, returning the number written (plan O3 Pass B).
    ///
    /// The counterpart of [`count_cell_pairs`](Self::count_cell_pairs): it walks the
    /// SAME slice in the SAME order, applies the SAME literal
    /// [`min_shared_cell`](Self::min_shared_cell) dedup and
    /// [`feasible`](Self::feasible) filter, and writes each survivor `(min, max)`
    /// pair into `out_slice[0..]` in slice order. Because the predicates and order
    /// match `count_cell_pairs` exactly, the returned count equals this cell's
    /// pre-counted survivor total — the caller debug-asserts it fills its whole sub-
    /// range (the C2 coupling). `out_slice` must be at least that long.
    fn emit_cell_pairs(
        &self,
        cell: u32,
        bodies: &[BodyState],
        out_slice: &mut [(BodyIndex, BodyIndex)],
    ) -> usize {
        let c = cell as usize;
        let start = self.cell_start[c] as usize;
        let end = self.cell_start[c + 1] as usize;
        let slice = &self.cell_bodies[start..end];
        let mut w = 0usize;
        for p in 0..slice.len() {
            let i = slice[p];
            for &j in &slice[p + 1..] {
                let a = &bodies[i as usize];
                let b = &bodies[j as usize];
                if self.min_shared_cell(a, b) == cell && Self::feasible(a, b) {
                    out_slice[w] = (BodyIndex(i), BodyIndex(j));
                    w += 1;
                }
            }
        }
        w
    }

    /// The PARALLEL candidate-emit grid build (plan O3, scope b): the CSR build
    /// (count + prefix-sum + scatter) and the oversized emit stay SERIAL and
    /// byte-identical to O2's [`build`](Self::build), but the per-cell candidate
    /// EMIT (Pass A count + Pass B emit) is fanned across the ambient
    /// [`ThreadPool`](boyko_threadpool::ThreadPool)'s workers over DISJOINT cell-
    /// range chunks via `pool.scope`.
    ///
    /// The output candidate pair MULTISET is bit-identical to O2's serial `build`
    /// (same pairs, same multiplicity) for ANY worker count and ANY cell-range
    /// partition: distinct workers own disjoint cell ranges, the cell-emit dedup
    /// keys each multi-cell pair to its minimum shared cell (a pure function of the
    /// two AABBs, independent of which worker visits it), and the final
    /// `out.sort_unstable()` canonicalizes ORDER.
    ///
    /// **Production shortcut.** When there is no ambient pool, OR the pool offers a
    /// single effective lane (`num_threads() + 1 == 1`, i.e. zero worker threads),
    /// OR the body count is below [`MIN_PARALLEL_BODIES`], this delegates straight
    /// to the O2 serial [`build`](Self::build) and returns — a single effective lane
    /// is pure serial work, so the emit-shaped multi-pass dispatch would only add
    /// overhead (the W=1 regression). The parallel-shaped path is dispatched ONLY
    /// when there are `>= 2` lanes, `n >= MIN_PARALLEL_BODIES`, and a pool is
    /// present. Either way the result is byte-identical to `build`.
    pub fn build_parallel(&mut self, bodies: &[BodyState], out: &mut Vec<(BodyIndex, BodyIndex)>) {
        let n = bodies.len();

        // Shortcut: below the parallel threshold (or an empty world) → the O2 serial
        // path. `build` clears `out`/`oversized` and runs the verbatim O2 emit.
        if n < MIN_PARALLEL_BODIES {
            self.build(bodies, out);
            return;
        }

        // Dispatch the parallel-shaped path ONLY when a pool offers >= 2 effective
        // lanes; a single lane (no worker threads) is pure serial work, so route to
        // the O2 serial `build` (no shaped-path overhead — eliminates the W=1
        // regression). When no pool is attached, `try_with_active_pool` returns
        // `None` and we fall through to the serial `build`.
        let dispatched = try_with_active_pool(|pool| {
            let lanes = pool.num_threads() + 1;
            if lanes < 2 {
                return false;
            }
            // CSR build first (serial, byte-identical to O2), then fan the emit.
            out.clear();
            self.build_csr(bodies);
            self.emit_passes(bodies, out, lanes * CHUNKS_PER_WORKER, Some(pool));
            true
        });
        if dispatched != Some(true) {
            self.build(bodies, out);
        }
    }

    /// Runs the O3 multi-pass shaped emit (Pass A count → prefix-sum → Pass B emit
    /// → serial oversized → single final serial sort) over `n_chunks`
    /// cell-range chunks. When `pool` is `Some`, the count and emit passes are
    /// dispatched across its workers via `pool.scope`; when `None`, the same passes
    /// run as a single-threaded loop over the chunks.
    ///
    /// The chunk count and shape are perf knobs only — the candidate multiset is
    /// partition-independent (see [`build_parallel`](Self::build_parallel)), so any
    /// `n_chunks ∈ [1, n_cells]` and either dispatch mode yields the identical pair
    /// set after the final sort.
    fn emit_passes(
        &mut self,
        bodies: &[BodyState],
        out: &mut Vec<(BodyIndex, BodyIndex)>,
        n_chunks: usize,
        pool: Option<&boyko_threadpool::PoolInner>,
    ) {
        let n_cells = self.n_cells();
        let n_chunks = n_chunks.clamp(1, n_cells.max(1));

        // Pass A: per-cell surviving-pair COUNT into `pair_count` (disjoint slots).
        self.pair_count.clear();
        self.pair_count.resize(n_cells, 0);
        // Read-only grid + bodies + the write base, captured as raw pointers so a
        // worker never holds an outer `&self`/`&mut self` borrow across the scope
        // (the Phase 9.3c bare-pointer discipline). The chunk cell ranges are
        // disjoint, so the `pair_count` slots written are pairwise disjoint.
        let pass_a_ptrs = EmitPtrs {
            grid: self as *const BroadphaseGrid,
            bodies: bodies.as_ptr(),
            bodies_len: bodies.len(),
            pair_count: self.pair_count.as_mut_ptr(),
            pair_offset: core::ptr::null(),
            out_base: core::ptr::null_mut(),
        };
        Self::run_cell_chunks(n_cells, n_chunks, pool, |c_lo, c_hi| {
            // SAFETY: `[c_lo, c_hi)` is one chunk's disjoint cell range; the worker
            //   reads only the read-only grid (`&*grid`, reborrowed fresh — no outer
            //   protector) + `bodies`, and writes only `pair_count[c]` for c in this
            //   chunk's range. Distinct chunks own disjoint cell ranges, so no two
            //   workers write the same `pair_count` slot, and the read-only data is
            //   shared. The pointees outlive the scope (the `pool.scope` Drop joins
            //   every task before this method returns).
            let grid = unsafe { &*pass_a_ptrs.grid };
            let body_slice = unsafe { pass_a_ptrs.bodies_slice() };
            for c in c_lo..c_hi {
                let cnt = grid.count_cell_pairs(c as u32, body_slice);
                // SAFETY: `c < n_cells` (chunk ranges partition `[0, n_cells)`), so
                //   `pair_count + c` is in bounds; this chunk uniquely owns slot `c`.
                unsafe { *pass_a_ptrs.pair_count.add(c) = cnt };
            }
        });

        // Serial exclusive prefix-sum pair_count → pair_offset (len n_cells + 1);
        // m = pair_offset[n_cells] is the total surviving within-cell pair count.
        self.pair_offset.clear();
        self.pair_offset.reserve(n_cells + 1);
        let mut acc = 0u32;
        self.pair_offset.push(0);
        for &c in &self.pair_count {
            acc += c;
            self.pair_offset.push(acc);
        }
        let m = acc as usize;

        // Serial oversized emit (reuse the verbatim O2 emitter) into `candidates`,
        // then feasibility-filter it — counted now so `out` can be sized once.
        self.candidates.clear();
        self.emit_oversized_candidates(bodies, bodies.len());
        let oversized_reserve = self.candidates.len();

        // Size `out` once for the m survivors + the (≤ oversized_reserve) feasible
        // oversized pairs. The survivor region is filled by Pass B; the oversized
        // region is appended serially after.
        out.clear();
        out.resize(m + oversized_reserve, (BodyIndex(0), BodyIndex(0)));

        // Pass B: emit each cell's survivors into its `out[pair_offset[c]..]` sub-
        // range (disjoint per chunk), UNSORTED (cell-major). The single final serial
        // `out.sort_unstable()` below canonicalizes the order — no per-chunk block-
        // sort (it would be pure redundant work given the final full sort). Chunk
        // boundaries are cut on EQUAL counted-pair runs (work-balanced via
        // `pair_offset`).
        let pass_b_ptrs = EmitPtrs {
            grid: self as *const BroadphaseGrid,
            bodies: bodies.as_ptr(),
            bodies_len: bodies.len(),
            pair_count: core::ptr::null_mut(),
            pair_offset: self.pair_offset.as_ptr(),
            out_base: out.as_mut_ptr(),
        };
        Self::run_balanced_cell_chunks(n_cells, n_chunks, pass_b_ptrs, pool, |c_lo, c_hi| {
            // SAFETY: `[c_lo, c_hi)` is one chunk's disjoint cell range. The worker
            //   reads only the read-only grid (`&*grid`, fresh reborrow) + `bodies`
            //   + `pair_offset` (read), and writes only `out[pair_offset[c_lo]..
            //   pair_offset[c_hi])`. Distinct chunks own disjoint, contiguous out
            //   sub-ranges (`pair_offset` is monotone, the chunks partition the
            //   cells), so no two workers write the same `out` element; the read-
            //   only data is shared. The pointees outlive the scope (`pool.scope`
            //   Drop joins every task before this method returns).
            let grid = unsafe { &*pass_b_ptrs.grid };
            let body_slice = unsafe { pass_b_ptrs.bodies_slice() };
            for c in c_lo..c_hi {
                // SAFETY: `c < n_cells`, so `c` and `c + 1` are valid `pair_offset`
                //   indices; `[lo, hi)` lies within this chunk's owned out sub-range.
                let lo = unsafe { pass_b_ptrs.pair_offset_at(c) };
                let hi = unsafe { pass_b_ptrs.pair_offset_at(c + 1) };
                if lo == hi {
                    continue;
                }
                // SAFETY: `[lo, hi)` is this cell's disjoint, in-bounds out slot
                //   range (within the chunk's sub-range, within `[0, m)`); the cell
                //   uniquely owns it. The pointee region is live for the scope.
                let cell_out = unsafe { pass_b_ptrs.out_subslice(lo, hi) };
                let written = grid.emit_cell_pairs(c as u32, body_slice, cell_out);
                debug_assert_eq!(
                    written,
                    hi - lo,
                    "invariant (C2): cell {c} emitted exactly its pre-counted survivor total"
                );
            }
        });

        // Serial oversized emit appended after the m survivors (W3, feasibility-
        // filtered with the SAME predicate). `candidates` already holds this build's
        // oversized pairs (the verbatim O2 emitter above); filter into out[m..].
        let mut w = m;
        for &(a, b) in &self.candidates {
            let ia = a.0 as usize;
            let ib = b.0 as usize;
            if Self::feasible(&bodies[ia], &bodies[ib]) {
                out[w] = (a, b);
                w += 1;
            }
        }
        // Truncate any oversized slots that the feasibility filter dropped (the
        // reserve was an upper bound).
        out.truncate(w);

        // Single final serial sort over the whole `out` buffer (cell survivors +
        // oversized tail) — byte-identical to O2's / the pre-merge O3's final
        // `out.sort_unstable()`. All survivor keys are UNIQUE (the `min_shared_cell`
        // dedup) and disjoint from the distinct oversized pairs, so the sorted
        // permutation is unique: the result is the same multiset in the same
        // canonical (min, max) order as the serial `build`.
        out.sort_unstable();

        debug_assert!(
            out.windows(2).all(|p| p[0] <= p[1]),
            "invariant (O3): the sorted output is non-decreasing (== O2's sort)"
        );
        debug_assert_eq!(
            out.len(),
            w,
            "invariant (O3): output length == m survivors + feasible oversized"
        );
    }

    /// Runs `body` over `n_chunks` CONTIGUOUS, EQUAL-CELL-count chunks of
    /// `[0, n_cells)` (plan O3 Pass A chunking). When `pool` is `Some`, each chunk
    /// is a `pool.scope` task; when `None`, they run serially on the calling thread.
    ///
    /// `body(c_lo, c_hi)` is invoked once per chunk with that chunk's half-open cell
    /// range. The closure must be `Send + Sync` (it is dispatched by reference into
    /// every task) and own only `Copy`/`Send` captures (the parallel-emit raw-
    /// pointer wrappers).
    fn run_cell_chunks<F>(
        n_cells: usize,
        n_chunks: usize,
        pool: Option<&boyko_threadpool::PoolInner>,
        body: F,
    ) where
        F: Fn(usize, usize) + Sync + Send,
    {
        let per = n_cells.div_ceil(n_chunks).max(1);
        match pool {
            Some(pool) => {
                pool.scope(|scope| {
                    let mut c_lo = 0usize;
                    while c_lo < n_cells {
                        let c_hi = (c_lo + per).min(n_cells);
                        let body = &body;
                        scope.spawn(move || body(c_lo, c_hi));
                        c_lo = c_hi;
                    }
                });
            }
            None => {
                let mut c_lo = 0usize;
                while c_lo < n_cells {
                    let c_hi = (c_lo + per).min(n_cells);
                    body(c_lo, c_hi);
                    c_lo = c_hi;
                }
            }
        }
    }

    /// Runs `body` over `n_chunks` CONTIGUOUS chunks of `[0, n_cells)` cut on EQUAL
    /// COUNTED-PAIR runs — work-balanced via the `pair_offset` prefix-sum (plan O3
    /// Pass B chunking). When `pool` is `Some`, each chunk is a `pool.scope` task;
    /// when `None`, they run serially.
    ///
    /// The chunk boundaries are derived by reading `pair_offset` through `ptrs`' raw
    /// base (`EmitPtrs::pair_offset_at`) — the SAME source the workers read — so no
    /// `&[u32]` borrow into the grid is held across the `pool.scope` frame (the Phase
    /// 9.3c TB-clean discipline, matching `solve_color_parallel` / `group_start_at`).
    ///
    /// `target` is the per-chunk survivor quota (`m / n_chunks`, rounded up); the
    /// dispatch loop grows a chunk by whole cells until its accumulated survivor run
    /// reaches `target` or the last cell is consumed — a contiguous cell range maps
    /// to a contiguous out sub-range. Every chunk includes ≥ 1 cell (progress).
    fn run_balanced_cell_chunks<F>(
        n_cells: usize,
        n_chunks: usize,
        ptrs: EmitPtrs,
        pool: Option<&boyko_threadpool::PoolInner>,
        body: F,
    ) where
        F: Fn(usize, usize) + Sync + Send,
    {
        // Read chunk boundaries through the SAME raw `pair_offset` base the workers
        // use (`EmitPtrs::pair_offset_at`), so no `&[u32]` borrow into the grid is
        // held across the `pool.scope` frame — the Phase 9.3c TB-clean discipline,
        // matching `ColoredSoftStepSolver::solve_color_parallel` / `group_start_at`.
        //
        // SAFETY: `pair_offset` was resized to `n_cells + 1` valid entries by the
        //   serial prefix-sum BEFORE this call, so every index in `[0, n_cells]` is
        //   in bounds; the buffer is read-only for the whole Pass B scope (written
        //   only by that serial prefix-sum, which has already completed) and the base
        //   is non-null (the Pass B wrapper). A plain `*const u32` read forms no
        //   reference into the grid.
        let m = unsafe { ptrs.pair_offset_at(n_cells) };
        let target = m.div_ceil(n_chunks).max(1);
        // Closure that advances a chunk: from `c_lo`, grow by whole cells until the
        // accumulated survivor run reaches `target` or `n_cells` is reached.
        let next_hi = |c_lo: usize| -> usize {
            // SAFETY: `c_lo < n_cells` (loop guard below), so `c_lo` and each `c_hi`
            //   visited (`<= n_cells`) are valid `pair_offset` indices; same read-only
            //   base + lifetime invariant as the `m` read above.
            let base = unsafe { ptrs.pair_offset_at(c_lo) };
            let mut c_hi = c_lo + 1;
            while c_hi < n_cells && (unsafe { ptrs.pair_offset_at(c_hi) } - base) < target {
                c_hi += 1;
            }
            c_hi
        };
        match pool {
            Some(pool) => {
                pool.scope(|scope| {
                    let mut c_lo = 0usize;
                    while c_lo < n_cells {
                        let c_hi = next_hi(c_lo);
                        let body = &body;
                        scope.spawn(move || body(c_lo, c_hi));
                        c_lo = c_hi;
                    }
                });
            }
            None => {
                let mut c_lo = 0usize;
                while c_lo < n_cells {
                    let c_hi = next_hi(c_lo);
                    body(c_lo, c_hi);
                    c_lo = c_hi;
                }
            }
        }
    }

    /// Test-only entry that runs the FULL multi-pass shaped emit path at a caller-
    /// forced `n_chunks`, single-threaded (NO pool) so Miri / proptests can exercise
    /// the restructured Pass A / prefix-sum / Pass B `pair_offset` arithmetic at
    /// `W ∈ {1, 2, 4, …}` without an attached thread pool (plan O3 W2 anti-vacuity).
    ///
    /// It deliberately runs the SHAPED path (serial CSR + Pass A + prefix + Pass B +
    /// oversized + single final serial sort) — it must NOT delegate to
    /// [`build`](Self::build), so the gate genuinely covers the parallel-emit code.
    /// The forced chunk count is clamped to `[1, n_cells]` like the live dispatch.
    // `dead_code`: this is the W2 anti-vacuity entry the tester's not-yet-written
    // `{1, 2, 4}`-chunk gate tests call; it is intentionally unreferenced until then.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn build_emit_shaped_forced(
        &mut self,
        bodies: &[BodyState],
        out: &mut Vec<(BodyIndex, BodyIndex)>,
        n_chunks: usize,
    ) {
        out.clear();
        let n = bodies.len();
        if n == 0 {
            self.oversized.clear();
            return;
        }
        self.build_csr(bodies);
        self.emit_passes(bodies, out, n_chunks, None);
    }

    /// The number of bodies classified oversized in the most recent
    /// [`build`](Self::build) — a diagnostic accessor over the private `oversized`
    /// escape-hatch list.
    ///
    /// Exposed so the size-disparity gate can assert the oversized hatch is
    /// genuinely exercised (`>= 2` in the multi-oversized proptest, keeping it
    /// non-vacuous). It is a read-only count of internal state, not a hot-path API.
    #[inline]
    pub fn oversized_len(&self) -> usize {
        self.oversized.len()
    }
}

/// `Send` + `Sync`-marked raw pointers to the O3 parallel-emit working set,
/// dispatched into the per-cell-chunk count / emit closures.
///
/// Raw pointers are `!Send`/`!Sync` by default; this wrapper lets a worker task
/// capture them. The fields are **private** and reached only through the `&self`
/// accessor methods so a closure capturing the wrapper captures the WHOLE struct —
/// never a bare `*mut`/`*const` field directly (Rust 2021+ disjoint capture would
/// otherwise see the bare pointer and reject the closure as `!Send`). This mirrors
/// `ColorSolvePtrs` in the colored solver (the Phase 9.3c TB-clean idiom).
///
/// The read-only `grid` is reborrowed `&*grid` FRESH inside each worker (no outer
/// `&self`/`&mut self` borrow is held across the `pool.scope` frame), so the
/// shared reads never conflict with the disjoint raw-pointer writes through
/// `pair_count` (Pass A) / `out_base` (Pass B).
#[derive(Clone, Copy)]
struct EmitPtrs {
    /// Read-only base of the live [`BroadphaseGrid`] (its CSR + geometry), reborrowed
    /// `&*grid` fresh in each worker for the `&self` survivor-helper calls.
    grid: *const BroadphaseGrid,
    /// Read-only base of the `bodies` snapshot slice.
    bodies: *const BodyState,
    /// Length of the `bodies` slice.
    bodies_len: usize,
    /// Pass A write base of `pair_count` (one disjoint slot per owned cell); null on
    /// the Pass B wrapper.
    pair_count: *mut u32,
    /// Read-only base of `pair_offset` (the survivor prefix-sum) for the Pass B out
    /// sub-range bounds; null on the Pass A wrapper.
    pair_offset: *const u32,
    /// Pass B write base of `out` (one disjoint contiguous sub-range per owned cell
    /// chunk); null on the Pass A wrapper.
    out_base: *mut (BodyIndex, BodyIndex),
}

impl EmitPtrs {
    /// Reborrows the read-only `bodies` snapshot as a shared slice.
    ///
    /// # Safety
    /// `bodies`/`bodies_len` must name a live `[BodyState]` that outlives the
    /// reborrow. Upheld by [`BroadphaseGrid::emit_passes`]: the pointer names the
    /// caller's `bodies` argument, live for the whole `pool.scope` frame.
    #[inline]
    unsafe fn bodies_slice<'a>(&self) -> &'a [BodyState] {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { core::slice::from_raw_parts(self.bodies, self.bodies_len) }
    }

    /// Reads `pair_offset[c]` via the raw pointer (no `&[u32]` borrow into the grid).
    ///
    /// # Safety
    /// `c` must be a valid index into the live `pair_offset` column (`c <= n_cells`)
    /// and `pair_offset` must be non-null (the Pass B wrapper). Upheld by the Pass B
    /// dispatch, which reads only cell indices within `[0, n_cells]`.
    #[inline]
    unsafe fn pair_offset_at(&self, c: usize) -> usize {
        // SAFETY: `c` is in range and `pair_offset` is the live base per the method
        //   contract; a plain `*const u32` read forms no reference into the grid.
        unsafe { *self.pair_offset.add(c) as usize }
    }

    /// Reborrows `out[lo..hi]` as a mutable slice for `'a`.
    ///
    /// # Safety
    /// `out_base` must be non-null (the Pass B wrapper) and `[lo, hi)` must be an in-
    /// bounds sub-range of the live `out` buffer that NO concurrent worker also
    /// reborrows. Upheld by [`BroadphaseGrid::emit_passes`]: distinct chunks own
    /// disjoint, contiguous out sub-ranges and each cell owns a disjoint slot range
    /// within its chunk.
    #[inline]
    unsafe fn out_subslice<'a>(&self, lo: usize, hi: usize) -> &'a mut [(BodyIndex, BodyIndex)] {
        // SAFETY: forwarded to the caller; see method doc. `lo <= hi` and the range
        //   is in bounds + disjoint across workers, so the reborrow is unique.
        unsafe { core::slice::from_raw_parts_mut(self.out_base.add(lo), hi - lo) }
    }
}

// SAFETY: the pointers name the read-only `BroadphaseGrid` + `bodies` snapshot and
//   the `pair_count` / `out` buffers borrowed by `emit_passes` for the whole
//   `pool.scope` frame, whose Drop blocks (work-stealing join) until every worker
//   that captured the wrapper has completed — so every pointee outlives every task
//   body (no use-after-free). The soundness of the concurrent accesses rests on
//   DISJOINTNESS, stated in full at each spawn site:
//     * Pass A: distinct chunks own DISJOINT cell ranges, so the `pair_count` slots
//       they write are pairwise disjoint; the grid + bodies are read-only (shared
//       `&*grid` reborrowed fresh per worker — no outer protector).
//     * Pass B: distinct chunks own DISJOINT cell ranges, hence DISJOINT contiguous
//       out sub-ranges `[pair_offset[c_lo]..pair_offset[c_hi])` (`pair_offset` is
//       monotone), so no two workers write the same `out` element; `pair_offset` /
//       grid / bodies are read-only.
//   No two tasks touch the same byte mutably, and the wrapper has no interior
//   mutability, so a shared `&` to it (the scope closure's capture) is trivially
//   safe — hence both `Send` (cross-thread move into a task) and `Sync` (shared by
//   the spawn loop) hold.
unsafe impl Send for EmitPtrs {}
unsafe impl Sync for EmitPtrs {}

/// Dense manifold buffer produced by narrowphase, consumed by the solver
/// (plan D1/D4).
///
/// Sequential over the ordered pairs (matching [`ContactPairs`]), so the solve
/// iterates a packed array. Cleared and refilled each step, capacity reused. Also
/// carries the box-box reference-axis hysteresis store ([`BoxAxisCache`], P2 W4),
/// embedded here so the narrowphase stage reaches it via the same `ResMut` it
/// already holds — no extra resource wiring (the cache is a narrowphase-internal
/// detail of producing stable feature ids, not a solver input).
#[derive(Resource, Default)]
pub struct Manifolds {
    /// Manifolds in the deterministic pair order.
    pub manifolds: Vec<Manifold>,
    /// Per-body-pair last-frame SAT-axis index (box-box hysteresis, P2 W4).
    /// Persisted in place across frames; the box-box generator feeds the stored
    /// axis back to bias against feature-id flicker on a resting stack.
    pub box_axis_cache: BoxAxisCache,
}

impl Manifolds {
    /// Builds an empty manifold buffer pre-sized for `capacity` manifolds (and the
    /// box-axis hysteresis cache for `capacity` pairs).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            manifolds: Vec::with_capacity(capacity),
            box_axis_cache: BoxAxisCache::with_capacity(capacity),
        }
    }
}

/// Bit width of one `color_occ` word (a `u64` per-color body bitset cell).
const OCC_WORD_BITS: u32 = 64;

/// Constraint islands + greedy graph coloring of one step's manifolds (plan O4,
/// Decision 2 / Decision 7).
///
/// After narrowphase, [`build`](ConstraintGraph::build) partitions the manifolds
/// into:
///
/// - **Islands** — connected components of the contact graph over DYNAMIC bodies.
///   A static / kinematic / sentinel body is "ground" that does NOT connect
///   islands (Box2D's rule): two bodies are union'd ONLY when BOTH are dynamic, so
///   a `(dyn, static)` contact attaches the dynamic body's island to nothing.
/// - **Colors** — a greedy first-fit (in manifold order) such that **no color
///   contains two manifolds sharing a dynamic body**. The static/sentinel side of
///   a manifold imposes no occupancy constraint (ground is shared freely).
///
/// # Layout (CSR-flattened, capacity-reused — W2: NO `Vec<Vec>`, no per-step alloc)
///
/// Every data-dependent dimension is a flat `Vec` + offsets, mirroring the
/// [`BroadphaseGrid`] CSR pattern. All buffers are `clear()`-ed and refilled each
/// build — capacity is reused, never `Vec::new` per step (principle 5). Buffers
/// grow only when a frame's island/color/body count exceeds the warmed capacity;
/// in steady state the build does zero heap work.
///
/// - `island_of[row]` is the island id of dynamic body `row` (static/sentinel
///   bodies have no island and are never indexed here).
/// - `island_manifold_start[i]..island_manifold_start[i + 1]` indexes
///   `island_manifolds` — manifold indices grouped by island.
/// - `color_start[c]..color_start[c + 1]` (`len == n_colors + 1`) indexes
///   `color_contacts` — manifold indices grouped by color, **in ascending manifold
///   order within each color** (D4 — stable, deterministic).
/// - `color_occ` is a flat per-color body bitset matrix addressed
///   `color_occ[color * words_per_color + (body >> 6)]`, bit `body & 63`. It is the
///   coloring occupancy scratch (reused across frames — cleared, not realloc'd).
///
/// # Determinism (plan §determinism gates)
///
/// The partition is a pure deterministic function of (manifolds in manifold order,
/// dynamic-body count). Union-find unions in manifold order; coloring is first-fit
/// in manifold order; CSR groups preserve ascending manifold index. No `HashMap`,
/// no iteration-order-dependent containers, no atomics. Same input → identical
/// `island_of` / `color_start` / `color_contacts` every run.
#[derive(Resource, Default)]
pub struct ConstraintGraph {
    /// Union-find parent links over dynamic body rows, reused across builds. Sized
    /// to the dynamic-body count each build; a static/sentinel row is never a node.
    uf_parent: Vec<u32>,
    /// Union-find subtree sizes (union-by-size), reused across builds.
    uf_size: Vec<u32>,
    /// `island_of[row]` = compacted island id of dynamic body `row` (flat). A
    /// static/sentinel row holds [`NO_ISLAND`](ConstraintGraph::NO_ISLAND).
    island_of: Vec<u32>,
    /// CSR offsets: `island_manifold_start[i]..[i + 1]` indexes `island_manifolds`
    /// (`len == n_islands + 1`). NO `Vec<Vec>`.
    island_manifold_start: Vec<u32>,
    /// CSR values: manifold indices grouped by island (flat).
    island_manifolds: Vec<u32>,
    /// CSR offsets: `color_start[c]..[c + 1]` indexes `color_contacts`
    /// (`len == n_colors + 1`). NO `Vec<Vec>`.
    color_start: Vec<u32>,
    /// CSR values: manifold indices grouped by color, ascending within a color
    /// (manifold order — D4).
    color_contacts: Vec<u32>,
    /// Flat per-color body bitset matrix, addressed
    /// `color_occ[color * words_per_color + (body >> 6)]`; bit `body & 63` is set
    /// when `body` is occupied in `color`. Reused (clear, never realloc) — the
    /// coloring occupancy scratch.
    color_occ: Vec<u64>,
    /// `u64` words per color row in `color_occ` (`= n_dynamic.div_ceil(64)`).
    words_per_color: u32,
    /// Number of colors produced this build (`color_start.len() == n_colors + 1`).
    n_colors: u32,
    /// Number of islands produced this build.
    n_islands: u32,
}

impl ConstraintGraph {
    /// Island id stored in `island_of` for a body that is not a dynamic node
    /// (static / kinematic / sentinel — never part of any island).
    pub const NO_ISLAND: u32 = u32::MAX;

    /// Builds an empty graph pre-sized for `capacity` bodies (no later realloc in
    /// steady state). The CSR buffers grow on the first builds to the live counts
    /// and reuse that capacity thereafter.
    pub fn with_capacity(capacity: usize) -> Self {
        let words = capacity.div_ceil(OCC_WORD_BITS as usize);
        Self {
            uf_parent: Vec::with_capacity(capacity),
            uf_size: Vec::with_capacity(capacity),
            island_of: Vec::with_capacity(capacity),
            island_manifold_start: Vec::with_capacity(capacity + 1),
            island_manifolds: Vec::with_capacity(capacity),
            color_start: Vec::with_capacity(capacity + 1),
            color_contacts: Vec::with_capacity(capacity),
            // One color row worth of words is a cheap first reserve; it grows to the
            // live color count × words-per-color and reuses that capacity.
            color_occ: Vec::with_capacity(words),
            words_per_color: 0,
            n_colors: 0,
            n_islands: 0,
        }
    }

    /// Number of colors in the current partition (`color_start.len() - 1`).
    #[inline]
    pub fn n_colors(&self) -> u32 {
        self.n_colors
    }

    /// Number of islands in the current partition.
    #[inline]
    pub fn n_islands(&self) -> u32 {
        self.n_islands
    }

    /// Manifold indices of color `color`, in ascending manifold order (D4). Returns
    /// an empty slice for `color >= n_colors`.
    #[inline]
    pub fn color(&self, color: u32) -> &[u32] {
        let c = color as usize;
        if c + 1 >= self.color_start.len() {
            return &[];
        }
        let start = self.color_start[c] as usize;
        let end = self.color_start[c + 1] as usize;
        &self.color_contacts[start..end]
    }

    /// Manifold indices of island `island`. Returns an empty slice for
    /// `island >= n_islands`.
    #[inline]
    pub fn island(&self, island: u32) -> &[u32] {
        let i = island as usize;
        if i + 1 >= self.island_manifold_start.len() {
            return &[];
        }
        let start = self.island_manifold_start[i] as usize;
        let end = self.island_manifold_start[i + 1] as usize;
        &self.island_manifolds[start..end]
    }

    /// Compacted island id of dynamic body `row`, or [`NO_ISLAND`](Self::NO_ISLAND)
    /// for a static/sentinel row (or one out of range).
    #[inline]
    pub fn island_of(&self, row: u32) -> u32 {
        self.island_of.get(row as usize).copied().unwrap_or(Self::NO_ISLAND)
    }

    /// Partitions `manifolds` (in manifold order) into islands + colors over the
    /// `n_dynamic` dynamic body rows, identifying static/sentinel bodies via the
    /// `is_dynamic` predicate (plan O4 — the independently callable pure builder).
    ///
    /// `is_dynamic(row)` returns `true` iff body `row` is a real dynamic node (a
    /// finite `inv_mass != 0`); a static / kinematic / sentinel body returns
    /// `false`. The [`SDF_SENTINEL`](crate::manifold::SDF_SENTINEL) `body_b`
    /// (`u32::MAX`) is treated as static here — it is never a row, so the caller's
    /// predicate must return `false` for it (the stage's predicate bounds-checks the
    /// row, so any out-of-range row including the sentinel is non-dynamic).
    ///
    /// # Invariants (debug-checked)
    ///
    /// - **Coloring**: no color contains two manifolds sharing a dynamic body.
    /// - **CSR monotonicity**: `color_start` / `island_manifold_start` are
    ///   non-decreasing and their last entry equals the values length.
    /// - **Island id range**: every dynamic body's `island_of` is `< n_islands`;
    ///   every static/sentinel body's is [`NO_ISLAND`](Self::NO_ISLAND).
    ///
    /// Determinism: pure function of `(manifolds, n_dynamic, is_dynamic)`. No alloc
    /// in steady state (all buffers cleared + reused).
    pub fn build(
        &mut self,
        manifolds: &[Manifold],
        n_dynamic: usize,
        is_dynamic: impl Fn(u32) -> bool,
    ) {
        self.reset_islands(n_dynamic);
        self.build_islands(manifolds, &is_dynamic);
        self.flatten_islands(manifolds, n_dynamic, &is_dynamic);
        self.color_manifolds(manifolds, n_dynamic, &is_dynamic);
    }

    /// Resets the union-find forest to `n_dynamic` singleton sets and clears
    /// `island_of` to [`NO_ISLAND`](Self::NO_ISLAND) (capacity reused).
    #[inline]
    fn reset_islands(&mut self, n_dynamic: usize) {
        self.uf_parent.clear();
        self.uf_size.clear();
        self.uf_parent.reserve(n_dynamic);
        self.uf_size.reserve(n_dynamic);
        for row in 0..n_dynamic as u32 {
            self.uf_parent.push(row);
            self.uf_size.push(1);
        }
        self.island_of.clear();
        self.island_of.resize(n_dynamic, Self::NO_ISLAND);
    }

    /// Iterative union-find `find` with full path compression (no recursion — the
    /// stack depth is unbounded for adversarial inputs).
    #[inline]
    fn uf_find(&mut self, mut x: u32) -> u32 {
        // Walk to the root.
        let mut root = x;
        while self.uf_parent[root as usize] != root {
            root = self.uf_parent[root as usize];
        }
        // Path-compress: point every node on the path straight at the root.
        while self.uf_parent[x as usize] != root {
            let next = self.uf_parent[x as usize];
            self.uf_parent[x as usize] = root;
            x = next;
        }
        root
    }

    /// Union two dynamic body rows by subtree size (the smaller tree is grafted
    /// under the larger — keeps `find` near-flat).
    #[inline]
    fn uf_union(&mut self, a: u32, b: u32) {
        let ra = self.uf_find(a);
        let rb = self.uf_find(b);
        if ra == rb {
            return;
        }
        let (small, big) = if self.uf_size[ra as usize] < self.uf_size[rb as usize] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.uf_parent[small as usize] = big;
        self.uf_size[big as usize] += self.uf_size[small as usize];
    }

    /// Unions the two bodies of every manifold IFF BOTH are dynamic (Box2D's
    /// ground rule — a static/sentinel body never merges two islands).
    fn build_islands(&mut self, manifolds: &[Manifold], is_dynamic: &impl Fn(u32) -> bool) {
        for m in manifolds {
            let a = m.body_a.0;
            let b = m.body_b.0;
            // Only a dyn-dyn edge connects islands; a dyn-static edge attaches the
            // dynamic body to nothing (ground is not an island node).
            if is_dynamic(a) && is_dynamic(b) {
                self.uf_union(a, b);
            }
        }
    }

    /// Compacts the union-find roots into dense island ids, fills `island_of`, and
    /// builds the CSR `island_manifold_start` / `island_manifolds` grouping. A
    /// manifold with at least one dynamic body is filed under that body's island;
    /// a manifold with NO dynamic body (both static — degenerate) is filed under no
    /// island (skipped).
    fn flatten_islands(
        &mut self,
        manifolds: &[Manifold],
        n_dynamic: usize,
        is_dynamic: &impl Fn(u32) -> bool,
    ) {
        // Assign dense island ids to roots in ascending root order (deterministic).
        // `island_of` doubles as the root→dense-id map: a root maps itself, a child
        // is resolved through its compacted root.
        let mut next_id = 0u32;
        for row in 0..n_dynamic as u32 {
            if !is_dynamic(row) {
                continue;
            }
            let root = self.uf_find(row);
            if root == row {
                // This row is a root — claim the next dense island id for it.
                self.island_of[row as usize] = next_id;
                next_id += 1;
            }
        }
        // Resolve every dynamic child to its root's dense id.
        for row in 0..n_dynamic as u32 {
            if !is_dynamic(row) {
                continue;
            }
            let root = self.uf_find(row);
            self.island_of[row as usize] = self.island_of[root as usize];
        }
        self.n_islands = next_id;

        // CSR group manifolds by island via a counting sort (deterministic, stable
        // by manifold index). counts → exclusive prefix sum → scatter.
        let n_islands = self.n_islands as usize;
        self.island_manifold_start.clear();
        self.island_manifold_start.resize(n_islands + 1, 0);
        // Per manifold, resolve its island (the dynamic side's island).
        for m in manifolds {
            if let Some(isl) = self.manifold_island(m, is_dynamic) {
                self.island_manifold_start[isl as usize + 1] += 1;
            }
        }
        for i in 0..n_islands {
            self.island_manifold_start[i + 1] += self.island_manifold_start[i];
        }
        let total = self.island_manifold_start[n_islands] as usize;
        self.island_manifolds.clear();
        self.island_manifolds.resize(total, 0);
        // Scatter with a running cursor (a working copy of the starts). Reuse
        // `uf_size` as the cursor scratch to avoid a fresh alloc. Split-borrow the
        // fields (`island_of` read, `uf_size`/`island_manifolds` written) so the
        // resolution does not re-borrow `self` through `manifold_island`.
        let cursor = &mut self.uf_size;
        cursor.clear();
        cursor.extend_from_slice(&self.island_manifold_start[..n_islands]);
        let island_of = &self.island_of;
        let out = &mut self.island_manifolds;
        for (mi, m) in manifolds.iter().enumerate() {
            let a = m.body_a.0;
            let b = m.body_b.0;
            // The manifold's island = the dense island of its dynamic side (a if
            // dynamic, else b); skip a static-static degenerate contact.
            let isl = if is_dynamic(a) {
                island_of[a as usize]
            } else if is_dynamic(b) {
                island_of[b as usize]
            } else {
                continue;
            };
            let slot = cursor[isl as usize];
            out[slot as usize] = mi as u32;
            cursor[isl as usize] = slot + 1;
        }
    }

    /// The island a manifold belongs to: the dense island of its dynamic side
    /// (body_a if dynamic, else body_b if dynamic), or `None` if neither body is
    /// dynamic (a static-static degenerate contact — filed under no island).
    #[inline]
    fn manifold_island(&self, m: &Manifold, is_dynamic: &impl Fn(u32) -> bool) -> Option<u32> {
        let a = m.body_a.0;
        let b = m.body_b.0;
        if is_dynamic(a) {
            Some(self.island_of[a as usize])
        } else if is_dynamic(b) {
            Some(self.island_of[b as usize])
        } else {
            None
        }
    }

    /// Greedy first-fit coloring in manifold order (D4): for each manifold, assign
    /// the lowest color whose per-color body bitset has NEITHER of its DYNAMIC
    /// bodies set, then mark them. A static/sentinel side imposes no occupancy
    /// (ground is shared freely). Grows `n_colors` as needed. Produces the CSR
    /// `color_start` / `color_contacts`.
    ///
    /// Invariant guaranteed by construction: a color is chosen only when both
    /// dynamic bodies are FREE in that color's bitset, and they are then MARKED, so
    /// no later manifold sharing either body can reuse the color — hence no color
    /// holds two manifolds sharing a dynamic body.
    fn color_manifolds(
        &mut self,
        manifolds: &[Manifold],
        n_dynamic: usize,
        is_dynamic: &impl Fn(u32) -> bool,
    ) {
        let words = n_dynamic.div_ceil(OCC_WORD_BITS as usize);
        self.words_per_color = words as u32;
        self.color_occ.clear();
        self.n_colors = 0;

        // Per-manifold chosen color, then a counting sort into CSR (so the values
        // stay in ascending manifold order within each color — D4). Reuse
        // `uf_parent` as the per-manifold color scratch.
        let chosen = &mut self.uf_parent;
        chosen.clear();
        chosen.reserve(manifolds.len());

        for m in manifolds {
            let a = m.body_a.0;
            let b = m.body_b.0;
            let a_dyn = is_dynamic(a);
            let b_dyn = is_dynamic(b);
            // Find the lowest color where every dynamic side is free.
            let mut color = 0u32;
            loop {
                if color >= self.n_colors {
                    // Need a new color row: append `words` zeroed occupancy words.
                    self.color_occ.resize(self.color_occ.len() + words, 0);
                    self.n_colors += 1;
                }
                let base = color as usize * words;
                let free = (!a_dyn || !occ_get(&self.color_occ, base, a))
                    && (!b_dyn || !occ_get(&self.color_occ, base, b));
                if free {
                    if a_dyn {
                        occ_set(&mut self.color_occ, base, a);
                    }
                    if b_dyn {
                        occ_set(&mut self.color_occ, base, b);
                    }
                    chosen.push(color);
                    break;
                }
                color += 1;
            }
        }

        // CSR group manifolds by chosen color (counting sort; stable by manifold
        // index → ascending manifold order within a color).
        let n_colors = self.n_colors as usize;
        self.color_start.clear();
        self.color_start.resize(n_colors + 1, 0);
        for &c in chosen.iter() {
            self.color_start[c as usize + 1] += 1;
        }
        for c in 0..n_colors {
            self.color_start[c + 1] += self.color_start[c];
        }
        let total = self.color_start[n_colors] as usize;
        debug_assert_eq!(total, manifolds.len(), "invariant: every manifold colored");
        self.color_contacts.clear();
        self.color_contacts.resize(total, 0);
        // Scatter with a running cursor (working copy of the starts). Reuse
        // `uf_size` as the cursor scratch.
        let cursor = &mut self.uf_size;
        cursor.clear();
        cursor.extend_from_slice(&self.color_start[..n_colors]);
        for (mi, &c) in self.uf_parent.iter().enumerate() {
            let slot = cursor[c as usize];
            self.color_contacts[slot as usize] = mi as u32;
            cursor[c as usize] = slot + 1;
        }

        self.debug_assert_coloring(manifolds, n_dynamic, is_dynamic);
    }

    /// Debug-only re-scan of the coloring invariant: within every color, no two
    /// manifolds share a dynamic body (plan O4 gate, the in-debug guard). Compiles
    /// to nothing in release.
    #[inline]
    fn debug_assert_coloring(
        &self,
        manifolds: &[Manifold],
        n_dynamic: usize,
        is_dynamic: &impl Fn(u32) -> bool,
    ) {
        if cfg!(debug_assertions) {
            let words = n_dynamic.div_ceil(OCC_WORD_BITS as usize);
            let mut seen = vec![0u64; words];
            for c in 0..self.n_colors {
                seen.iter_mut().for_each(|w| *w = 0);
                for &mi in self.color(c) {
                    let m = &manifolds[mi as usize];
                    for &row in &[m.body_a.0, m.body_b.0] {
                        if is_dynamic(row) {
                            let w = row as usize >> 6;
                            let bit = 1u64 << (row & 63);
                            debug_assert_eq!(
                                seen[w] & bit,
                                0,
                                "coloring invariant: color {c} reuses dynamic body {row}"
                            );
                            seen[w] |= bit;
                        }
                    }
                }
            }
        }
    }
}

/// Reads bit `body & 63` of word `body >> 6` in the color row starting at `base`.
#[inline]
fn occ_get(occ: &[u64], base: usize, body: u32) -> bool {
    let word = base + (body as usize >> 6);
    (occ[word] >> (body & 63)) & 1 != 0
}

/// Sets bit `body & 63` of word `body >> 6` in the color row starting at `base`.
#[inline]
fn occ_set(occ: &mut [u64], base: usize, body: u32) {
    let word = base + (body as usize >> 6);
    occ[word] |= 1u64 << (body & 63);
}

/// Per-BODY-ROW sleeping / deactivation state (plan O8 / Decision 5, row-keyed
/// rewrite) — IM-1 SAFE: gather stays FULL, only SOLVE + INTEGRATE skip for frozen
/// islands.
///
/// # Why per ROW, not per island
///
/// Island ids are NOT stable: [`ConstraintGraph::build`] re-derives them every frame
/// (a pure function of the manifold set, in ascending-root order), so an id `k`
/// denotes a DIFFERENT island after any topology change (a merge, a split, a new
/// body). Keying the sleep latch by island id therefore breaks under exactly the
/// events sleeping must handle: a faller merging into a slept pile, or a pile
/// splitting. Body ROWS, by contrast, are STABLE across frames (the gather is FULL
/// and dense, IM-1 — rows never shift), so the latch is carried PER ROW and the
/// island-active decision is DERIVED from the rows fresh each frame. This makes the
/// model topology-robust by construction: there is no volatile-id carry to corrupt.
///
/// # The model
///
/// - [`asleep`](Self::asleep)`[row]` is a per-row LATCH: this row's island has been
///   at rest for [`PhysicsConfig::sleep_frames`] consecutive frames.
/// - Each frame, an island is FROZEN iff EVERY one of its member dynamic rows is
///   latched `asleep`. If ANY member row is awake — a never-slept row, a just-woken
///   row, a brand-new body (default `asleep = false`), or a faller that was moving
///   last frame — the WHOLE island is ACTIVE this frame: all its manifolds are solved
///   and all its bodies integrated. This is **wake-on-merge**: a slept island that
///   absorbs an awake/new row wakes the SAME frame the contact appears (no mid-air
///   freeze, no penetration-stick).
/// - A FROZEN island skips ONLY its SOLVE + INTEGRATE work — but
///   [`physics_gather`](crate::systems::physics_gather) still snapshots every row, so
///   a frozen body keeps its dense-row warm key and the IM-1 `physics_apply` desync
///   `debug_assert!` can never fire.
///
/// # Determinism
///
/// The per-island energy is `max over the island's dynamic rows of (|v|² + |ω|²)`,
/// accumulated with EXACT arithmetic (`v·v + ω·ω`, no `sqrt`/`rsqrt`/`algebraic_*`);
/// the debounce is a per-row integer counter; the freeze decision is a pure function
/// of the per-row latch + this frame's island assignment. No `HashMap`, no
/// iteration-order or volatile-id dependence. So sleeping-ON is run-to-run
/// bit-deterministic. It is NOT bit-equivalent to sleeping-off (a frozen island
/// deliberately stops integrating).
///
/// # Capacity reuse (zero per-step alloc)
///
/// The per-row buffers are resized to the live row count (a one-time grow like every
/// other physics buffer); the per-island scratch is cleared + resized each step. No
/// per-step heap allocation in steady state. The `awake_rows` mask reuses the
/// engine's growable [`TouchedMask`] bitset.
#[derive(Resource, Default)]
pub struct IslandSleep {
    /// Per-ROW sleep LATCH — `true` once this row's island has been below
    /// [`PhysicsConfig::sleep_threshold`] for [`PhysicsConfig::sleep_frames`]
    /// consecutive frames, `false` until then or after a wake. Indexed by BODY ROW
    /// (stable across frames), so it survives topology changes intact (the whole
    /// point of the rewrite). A brand-new row defaults `false` (awake).
    asleep: Vec<bool>,
    /// Per-ROW consecutive frames its island has been below
    /// [`PhysicsConfig::sleep_threshold`] — the debounce counter. Saturates at
    /// [`PhysicsConfig::sleep_frames`]; reset to `0` when the row's island is above
    /// threshold or the row is woken. Indexed by BODY ROW; a new row defaults `0`.
    below_count: Vec<u16>,
    /// Per-ISLAND "frozen this frame" decision, DERIVED in [`begin_step`](Self::begin_step)
    /// from the per-row latch (`frozen_islands[i]` is `true` iff every member dynamic
    /// row of island `i` is latched `asleep`). Pure per-frame scratch (cleared +
    /// resized to this build's island count each step) — it is NOT a persistent latch,
    /// so there is no volatile-id carry. Drives the manifold SOLVE-skip predicate.
    frozen_islands: Vec<bool>,
    /// Per-island SPEED² metric this frame (`max |v|²+|ω|² over the island's dynamic
    /// rows`, mass-INDEPENDENT), exact arithmetic. Pure scratch (clear + resize each
    /// step) — recomputed every [`end_step`](Self::end_step). Named `energy` for
    /// brevity; it is a speed² proxy, NOT a mass-weighted kinetic energy.
    energy: Vec<f32>,
    /// Body→awake mask (`true` = the row is awake this step). Drives the SOLVE +
    /// INTEGRATE skip — NOT the gather skip (IM-1). Rebuilt each step from
    /// `frozen_islands` + the graph's `island_of`; a row with no island (static /
    /// out-of-island) is awake (immovable bodies cost nothing to "integrate" — the
    /// kernels no-op them).
    awake_rows: TouchedMask,
    /// Whether a global wake was requested (config change / explicit
    /// [`wake_all`](Self::wake_all)) — consumed once on the next solve, which clears
    /// every row's latch before deciding afresh.
    wake_all: bool,
}

impl IslandSleep {
    /// Builds an empty sleep state pre-sized for `islands` islands and `rows` bodies
    /// (no later realloc in steady state).
    ///
    /// The per-row latch buffers reserve `rows`; the per-island scratch reserves
    /// `islands` (the worst case is one singleton island per row, so `islands` is a
    /// hint — the scratch grows to the live island count and reuses that capacity).
    pub fn with_capacity(islands: usize, rows: usize) -> Self {
        Self {
            asleep: Vec::with_capacity(rows),
            below_count: Vec::with_capacity(rows),
            frozen_islands: Vec::with_capacity(islands),
            energy: Vec::with_capacity(islands),
            awake_rows: TouchedMask::with_capacity(rows),
            wake_all: false,
        }
    }

    /// Requests that EVERY row wake on the next solve (the explicit-wake /
    /// config-change substrate, plan O8 / Decision 5 (ii)/(iii)).
    ///
    /// A pure signal the solver does NOT itself trip — unlike `Changed<RigidBody>`,
    /// which the solver sets every frame by writing velocities back (W6), so it is a
    /// sound wake key. Consumed once: the next solve clears every row's latch, then
    /// re-evaluates the energy/debounce from scratch.
    #[inline]
    pub fn wake_all(&mut self) {
        self.wake_all = true;
    }

    /// Returns `true` if island `island` is FROZEN this frame (its solve + integrate
    /// are skipped). `false` for an out-of-range island. This is the per-frame
    /// decision derived in [`begin_step`](Self::begin_step) (every member row latched
    /// asleep), NOT a persistent per-island latch.
    #[inline]
    pub fn is_island_frozen(&self, island: u32) -> bool {
        self.frozen_islands
            .get(island as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Returns `true` if body `row` is awake this step (drives the SOLVE / INTEGRATE
    /// skip). An out-of-range row reads NOT-awake — [`TouchedMask::get`] returns
    /// `false` past the live bit range, so such a row would be treated as frozen and
    /// have its solve / integrate SKIPPED. The invariant that makes that
    /// unreachable: every queried row is in range — `begin_step` sizes `awake_rows`
    /// to `n_rows == bodies.len()` and every caller iterates `0..bodies.len()`, so
    /// no live row ever falls outside the mask.
    #[inline]
    pub fn is_row_awake(&self, row: usize) -> bool {
        self.awake_rows.get(row)
    }

    /// Test hook: latch `row` into the slept state with the debounce fully elapsed,
    /// so a sanity test can isolate the WAKE / FREEZE decision without running the
    /// energy / debounce path. Call AFTER a `begin_step` has sized the per-row buffers.
    #[cfg(test)]
    pub(crate) fn force_sleep_row(&mut self, row: usize) {
        if row < self.asleep.len() {
            self.asleep[row] = true;
            self.below_count[row] = DEFAULT_SLEEP_FRAMES;
        }
    }

    /// Test hook: whether body `row`'s per-row LATCH is set (it has been at rest long
    /// enough). This is the persistent latch, distinct from `is_island_frozen` (the
    /// per-frame decision). `false` for an out-of-range row.
    #[cfg(test)]
    pub(crate) fn is_row_asleep(&self, row: usize) -> bool {
        self.asleep.get(row).copied().unwrap_or(false)
    }

    /// Resizes the per-ROW latch buffers to `n_rows`, preserving the latch / debounce
    /// of rows that still exist and defaulting any newly-appeared row to awake
    /// (`asleep = false`, debounce `0`) — so a brand-new body is awake on its first
    /// frame.
    ///
    /// Rows are STABLE across frames (the gather is full + dense, IM-1), so this carry
    /// is exact and topology-robust: a merge / split changes the island assignment,
    /// not the row identity, so the latch follows the body, not the (volatile) island
    /// id. There is no per-island carry to corrupt (the C3 class of bug is gone by
    /// construction).
    ///
    /// Caveat (caller contract): the latch follows a STABLE row, so a row that flips
    /// mass regime at RUNTIME (static ↔ dynamic) keeps its old latch — `end_step`
    /// skips static rows and so cannot refresh it. A freshly-dynamic row that carried
    /// a stale `asleep = true` would be a spurious freeze candidate; such a runtime
    /// flip MUST be paired with [`wake_all`](Self::wake_all) (or clearing that row's
    /// latch). Not reachable today (`inv_mass` is stable after spawn). See
    /// [`end_step`](Self::end_step).
    fn sync_rows(&mut self, n_rows: usize) {
        self.asleep.resize(n_rows, false);
        self.below_count.resize(n_rows, 0);
    }

    /// Step phase 1 (BEFORE the solve): resizes the per-row latch to this frame's row
    /// count, derives the per-island FROZEN decision from the row latch (THIS is the
    /// wake), and rebuilds the `awake_rows` body mask the solver reads to skip frozen
    /// islands' SOLVE + INTEGRATE (plan O8, row-keyed).
    ///
    /// # Freeze / wake decision (pure function of the per-row latch + this frame's
    /// island assignment)
    ///
    /// An island is FROZEN this frame IFF EVERY one of its member dynamic rows is
    /// latched `asleep`. If ANY member row is awake (`asleep[row] == false`) — a
    /// never-slept row, a just-woken row, a brand-new body, or a faller that was
    /// moving last frame — the WHOLE island is ACTIVE this frame: none of its rows is
    /// frozen, so all its manifolds are solved and all its bodies integrated.
    ///
    /// This IS **wake-on-merge**: a slept pile that absorbs an awake/new row wakes the
    /// SAME frame the contact appears (the merged island now contains an awake row, so
    /// it is not frozen — no mid-air freeze, no penetration-stick). It is also
    /// topology-robust: the decision is recomputed from the stable rows each frame, so
    /// a re-island'd scene cannot spuriously freeze a moving island (no volatile-id
    /// carry).
    ///
    /// `wake_all` (explicit [`wake_all`](Self::wake_all) / a config change) clears
    /// every row's latch first, so no island can be frozen this frame.
    ///
    /// The `Changed<RigidBody>` route is intentionally NOT a wake condition: the
    /// solver writes velocities back through `Mut<RigidBody>` every step for every
    /// awake body, so it trips `Changed` itself (W6).
    ///
    /// `awake_rows[row]` is set for every body in an ACTIVE island and for every row
    /// with no island (static / out-of-island bodies — they cost nothing to keep
    /// "awake" since the integrate kernels no-op an `inv_mass == 0` row).
    pub(crate) fn begin_step(&mut self, graph: &ConstraintGraph, n_rows: usize) {
        self.sync_rows(n_rows);

        // Explicit / config-change wake: clear every row's latch before deciding, so
        // no island can be frozen this frame.
        if self.wake_all {
            for s in &mut self.asleep {
                *s = false;
            }
            for c in &mut self.below_count {
                *c = 0;
            }
            self.wake_all = false;
        }

        // Derive the per-island FROZEN decision from the row latch: an island starts
        // a candidate to freeze (`true`) and is cleared the moment any member dynamic
        // row is found awake. A static/out-of-island row (`NO_ISLAND`) is not a member.
        let n_islands = graph.n_islands() as usize;
        self.frozen_islands.clear();
        self.frozen_islands.resize(n_islands, true);
        for row in 0..n_rows {
            let isl = graph.island_of(row as u32);
            if isl == ConstraintGraph::NO_ISLAND {
                continue;
            }
            if !self.asleep[row] {
                // An awake member row forces its whole island active this frame
                // (wake-on-merge: a new / moving row joining a slept pile wakes it).
                self.frozen_islands[isl as usize] = false;
            }
        }

        // Build the body→awake mask from the per-island frozen decision. A row is
        // awake iff it has no island, or its island is not frozen this frame.
        self.awake_rows.reset(n_rows);
        for row in 0..n_rows {
            let isl = graph.island_of(row as u32);
            let awake = isl == ConstraintGraph::NO_ISLAND || !self.frozen_islands[isl as usize];
            if awake {
                self.awake_rows.set(row);
            }
        }
    }

    /// Step phase 2 (AFTER the solve): accumulates this frame's per-island speed²
    /// metric from the solved body velocities and advances the per-ROW debounce / latch
    /// for NEXT frame (plan O8 / Decision 5, row-keyed).
    ///
    /// # The metric (speed², mass-INDEPENDENT)
    ///
    /// The per-island value is the **MAX over the island's dynamic rows of
    /// `|linear_velocity|² + |angular_velocity|²`** — pure speed² + angular speed²,
    /// NOT a mass-normalized kinetic energy (it carries no mass term). It is the
    /// Box2D-style sleep metric: a light-fast body has a high `|v|²` and so correctly
    /// stays awake. The arithmetic is EXACT (`v·v + ω·ω`, no `sqrt`/`rsqrt`/
    /// `algebraic_*`, order-fixed dot products), so it is run-to-run bit-deterministic.
    /// MAX (not SUM) is used so a single busy row keeps its whole island awake.
    ///
    /// # The per-row latch update
    ///
    /// For each island: if its speed² is below `threshold`, every member dynamic row
    /// increments its `below_count` (saturating at `frames`) and latches
    /// `asleep = below_count >= frames`; otherwise every member row resets
    /// `below_count = 0` and `asleep = false`. A frozen island's restored-low
    /// velocities keep it below threshold, so it stays latched asleep until a merge
    /// brings an awake row (handled in `begin_step`) or `wake_all` fires — a frozen
    /// island does NOT wake via its own frozen energy. No oscillation.
    ///
    /// # Mass-regime flip (caller contract)
    ///
    /// A row with `inv_mass == 0.0` (static) is SKIPPED here, so its `asleep` latch is
    /// frozen in place rather than re-evaluated. If a body's mass regime flips at
    /// RUNTIME (static ↔ dynamic — e.g. `inv_mass` zeroed then later restored) while
    /// its latch reads `asleep = true`, the freshly-dynamic body would be a freeze
    /// candidate in `begin_step` carrying a stale latch with no fresh debounce. This is
    /// NOT reachable today (per-body `inv_mass` is stable after spawn), but a future
    /// runtime mass-regime flip MUST be paired with [`wake_all`](Self::wake_all) (or
    /// clearing that row's latch) so a freshly-dynamic body isn't spuriously frozen by
    /// a stale latch.
    pub(crate) fn end_step(
        &mut self,
        bodies: &[BodyState],
        graph: &ConstraintGraph,
        threshold: f32,
        frames: u16,
    ) {
        // Reset the per-island speed² accumulators (scratch, sized to this build).
        let n_islands = graph.n_islands() as usize;
        self.energy.clear();
        self.energy.resize(n_islands, 0.0);

        // Accumulate the per-island MAX dynamic-row speed² (exact, deterministic).
        for (row, b) in bodies.iter().enumerate() {
            if b.inv_mass == 0.0 {
                // Static / immovable — not an island node, contributes no metric.
                continue;
            }
            let isl = graph.island_of(row as u32);
            if isl == ConstraintGraph::NO_ISLAND {
                continue;
            }
            let v = b.linear_velocity;
            let w = b.angular_velocity;
            // Exact speed² + angular speed² (no sqrt — order-fixed dot products).
            let e = v.dot(v) + w.dot(w);
            let slot = &mut self.energy[isl as usize];
            if e > *slot {
                *slot = e;
            }
        }

        // Advance the per-ROW debounce / latch from each island's speed². A row's
        // island is below threshold ⇒ tick its debounce; above ⇒ reset + wake.
        for row in 0..self.asleep.len() {
            if row >= bodies.len() || bodies[row].inv_mass == 0.0 {
                // No live dynamic body at this row this frame — leave its latch
                // untouched (it carries forward; `begin_step` defaults new rows awake).
                continue;
            }
            let isl = graph.island_of(row as u32);
            if isl == ConstraintGraph::NO_ISLAND {
                continue;
            }
            if self.energy[isl as usize] < threshold {
                let c = &mut self.below_count[row];
                if *c < frames {
                    *c += 1;
                }
                self.asleep[row] = *c >= frames;
            } else {
                self.below_count[row] = 0;
                self.asleep[row] = false;
            }
        }
    }
}

/// Dense, row-indexed snapshot of one body's state for the solve (plan IM-1).
///
/// `BodyState` carries the HOT integrator fields the solve mutates plus the
/// COLD mass fields the solve reads — gathered once at the seam boundary so the
/// solver works over a packed SoA-friendly array (the ideal Phase-10 constraint
/// buffer). `#[repr(C)]` + `Copy` for a flat cache-friendly layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyState {
    /// WORLD inverse inertia TENSOR `R₀ · inv_inertia_local · R₀ᵀ`, derived at
    /// gather; read by the solve and refreshed per substep as the orientation
    /// advances. Placed first so the larger-aligned fields lead the struct.
    pub inv_inertia: Mat3,
    /// Orientation-free LOCAL inverse inertia (the principal-axis diagonal
    /// tensor of the collider shape), derived at gather from the shape +
    /// inverse mass. The solver rotates it into world space each substep
    /// (`R · inv_inertia_local · Rᵀ`); kept here so that rotation needs no
    /// re-derivation. `Mat3::ZERO` for static / `inv_mass == 0` bodies.
    pub inv_inertia_local: Mat3,
    /// World position (mirrors [`RigidBody::position`]).
    pub position: Vec3,
    /// Linear velocity (mirrors [`RigidBody::linear_velocity`]).
    pub linear_velocity: Vec3,
    /// Angular velocity (mirrors [`RigidBody::angular_velocity`]).
    pub angular_velocity: Vec3,
    /// Orientation (mirrors [`RigidBody::rotation`]).
    pub rotation: Quat,
    /// Inverse mass (`0` = immovable); read by the solve.
    pub inv_mass: f32,
    /// Restitution; read by the solve.
    pub restitution: f32,
    /// Friction; read by the solve.
    pub friction: f32,
    /// The body's simulation role.
    pub body_type: BodyType,
    /// The collider shape, projected at gather so broad/narrowphase have the
    /// body's real geometry (P2 W2). The broadphase reads its bounding radius
    /// and the sphere-sphere narrowphase reads its sphere radius — neither phase
    /// may assume a fixed size. Placed last (after the scalar fields) so the
    /// tightly-packed hot fields lead the struct.
    pub shape: ColliderShape,
}

impl BodyState {
    /// Builds a snapshot row from the hot [`RigidBody`] + cold [`RigidBodyMass`]
    /// + [`Collider`] columns (the gather projection, IM-1; P2 W1).
    ///
    /// Derives the orientation-free LOCAL inverse inertia from the collider
    /// shape and inverse mass, then rotates it into the world tensor by the
    /// body's spawn orientation `R₀`:
    ///
    /// - Solid sphere of radius `r`: `I = (2/5)·m·r²`, so the local inverse
    ///   inertia is the isotropic diagonal `inv_mass · 5 / (2·r²)` on each axis.
    /// - Box of half-extents `(hx, hy, hz)` (full extents `(w, h, d) = 2·h`):
    ///   `Ixx = (1/12)·m·(h²+d²)`, `Iyy = (1/12)·m·(w²+d²)`,
    ///   `Izz = (1/12)·m·(w²+h²)`, inverted per axis.
    /// - Static / `inv_mass == 0` (or a degenerate `r ≤ 0`): [`Mat3::ZERO`]
    ///   (infinite inertia, no angular response).
    ///
    /// `inv_inertia` (the WORLD tensor) is then `R₀ · inv_inertia_local · R₀ᵀ`,
    /// auto-overriding any value authored on
    /// [`RigidBodyMass::inv_inertia`](crate::components::RigidBodyMass::inv_inertia)
    /// (which is retained for custom authoring but recomputed here).
    #[inline]
    pub fn from_columns(body: &RigidBody, mass: &RigidBodyMass, collider: &Collider) -> Self {
        let inv_inertia_local = local_inv_inertia(collider.shape, mass.inv_mass);
        // World tensor = R₀ · I⁻¹_local · R₀ᵀ (rotates the principal-axis
        // diagonal into the body's spawn orientation).
        let r0 = Mat3::from_quat(body.rotation);
        let inv_inertia = r0 * inv_inertia_local * r0.transpose();
        Self {
            inv_inertia,
            inv_inertia_local,
            position: body.position,
            linear_velocity: body.linear_velocity,
            angular_velocity: body.angular_velocity,
            rotation: body.rotation,
            inv_mass: mass.inv_mass,
            restitution: mass.restitution,
            friction: mass.friction,
            body_type: mass.body_type,
            shape: collider.shape,
        }
    }
}

/// Derives the orientation-free LOCAL inverse inertia of a collider shape from
/// its geometry and the body's inverse mass (P2 W1).
///
/// Returns [`Mat3::ZERO`] (infinite inertia) for an immovable body
/// (`inv_mass == 0`) or a degenerate shape (non-positive sphere radius), so no
/// torque produces an angular response. See [`BodyState::from_columns`] for the
/// per-shape formulae.
#[inline]
fn local_inv_inertia(shape: ColliderShape, inv_mass: f32) -> Mat3 {
    // Immovable body: infinite inertia, no angular response (single branch).
    if inv_mass == 0.0 {
        return Mat3::ZERO;
    }
    match shape {
        ColliderShape::Sphere { radius } => {
            if radius <= 0.0 {
                return Mat3::ZERO;
            }
            // Solid sphere I = (2/5)·m·r² ⇒ I⁻¹ = (5 / (2·r²)) / m
            //                              = inv_mass · 5 / (2·r²) (isotropic).
            let inv = inv_mass * 5.0 / (2.0 * radius * radius);
            Mat3::from_diagonal(Vec3::new(inv, inv, inv))
        }
        ColliderShape::Box { half_extents } => {
            // Full extents (w, h, d) = 2·half_extents.
            let w = 2.0 * half_extents.x;
            let h = 2.0 * half_extents.y;
            let d = 2.0 * half_extents.z;
            // Solid box principal inertia (·m): Ixx=(1/12)(h²+d²), etc. With
            // I = (1/12)·m·(..), I⁻¹ = 12·inv_mass / (..) per axis.
            let inv_axis = |sum_sq: f32| {
                if sum_sq > 0.0 {
                    12.0 * inv_mass / sum_sq
                } else {
                    0.0
                }
            };
            Mat3::from_diagonal(Vec3::new(
                inv_axis(h * h + d * d),
                inv_axis(w * w + d * d),
                inv_axis(w * w + h * h),
            ))
        }
    }
}

/// A growable per-row touched mask indexed by [`BodyIndex`] = row (plan IM-1).
///
/// Built from the `boyko_utils` [`BitSet256`] 256-bit chunk so it scales past
/// 256 rows (a `BitSet256` alone caps at 256; `BitSet<T>` caps at 128). Each
/// chunk is a fixed 256-bit word block; the `Vec<BitSet256>` grows in chunk
/// granularity and its capacity is reused across steps. The solver sets bit
/// `i` for every row it mutates; [`physics_apply`](crate::systems::physics_apply)
/// writes back only set rows.
#[derive(Default)]
pub struct TouchedMask {
    /// One 256-bit chunk per 256 rows; chunk `i >> 8` holds bit `i & 255`.
    chunks: Vec<BitSet256>,
}

impl TouchedMask {
    /// Builds an empty mask pre-sized for `rows` bodies (no later realloc in
    /// steady state).
    #[inline]
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            chunks: Vec::with_capacity(rows.div_ceil(BITS_PER_CHUNK)),
        }
    }

    /// Clears every bit and resizes the mask to hold exactly `rows` bits,
    /// reusing the chunk capacity (no realloc once warmed).
    #[inline]
    pub fn reset(&mut self, rows: usize) {
        let needed = rows.div_ceil(BITS_PER_CHUNK);
        self.chunks.clear();
        self.chunks.resize(needed, BitSet256::new());
    }

    /// Marks row `index` as touched.
    #[inline]
    pub fn set(&mut self, index: usize) {
        debug_assert!(
            index < self.chunks.len() * BITS_PER_CHUNK,
            "invariant: touched index {index} out of range; call reset(rows) first"
        );
        self.chunks[index >> 8].set(index & (BITS_PER_CHUNK - 1));
    }

    /// Returns `true` if row `index` was touched.
    #[inline]
    pub fn get(&self, index: usize) -> bool {
        let chunk = index >> 8;
        if chunk >= self.chunks.len() {
            return false;
        }
        self.chunks[chunk].get(index & (BITS_PER_CHUNK - 1))
    }
}

/// The dense, row-indexed solver scratch — the gather snapshot + touched mask
/// (plan IM-1).
///
/// All buffers are indexed by [`BodyIndex`] = archetype row, assigned by the
/// gather stage in archetype-row order. `bodies` is the SoA snapshot the solver
/// mutates; `touched` flags the rows
/// [`physics_apply`](crate::systems::physics_apply) writes back. Every buffer is
/// cleared and refilled each step, capacity reused.
///
/// A row→entity map (for the gameplay [`Contact`](crate::components::Contact)
/// producer) is intentionally NOT carried here in the foundation: `Entity` is not
/// yet a `QueryData`, so the gather cannot populate it, and shipping an
/// always-empty buffer whose "parallel to `bodies`" invariant is false from day
/// one is a footgun (review M2). Phase 10 adds it back together with the `Contact`
/// producer once `Entity`-as-`QueryData` lands.
#[derive(Resource, Default)]
pub struct SolverScratch {
    /// Dense snapshot, one row per body in archetype-row order.
    pub bodies: Vec<BodyState>,
    /// Per-row touched mask, indexed by [`BodyIndex`] = row.
    pub touched: TouchedMask,
    /// Per-contact-point relative normal APPROACH velocity captured BEFORE the
    /// first substep, consumed by the TGS solver's post-loop restitution pass
    /// (P2 W2). Indexed in the solver's flattened contact-point order (manifold
    /// order × point order); rebuilt and refilled each solve, capacity reused
    /// (no per-step alloc). Left empty by the no-op / non-owning solvers.
    pub vn_initial: Vec<f32>,
}

impl SolverScratch {
    /// Builds scratch buffers pre-sized for up to `rows` bodies (no later
    /// reallocation in steady state).
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            bodies: Vec::with_capacity(rows),
            touched: TouchedMask::with_capacity(rows),
            // One initial normal-velocity slot per body is a cheap first-frame
            // reserve; the TGS solver grows it to the live contact-point count
            // and reuses that capacity thereafter.
            vn_initial: Vec::with_capacity(rows),
        }
    }

    /// Clears the snapshot for a fresh gather, reusing capacity. The touched
    /// mask is reset by the gather once the row count is known; `vn_initial` is
    /// rebuilt by the solver, so it is cleared here for a fresh solve.
    #[inline]
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.vn_initial.clear();
    }
}

#[cfg(test)]
mod tests {
    //! W1 acceptance gate (plan §MAJOR W1): the `from_columns` /
    //! `local_inv_inertia` inertia DERIVATION. The `math.rs` suite covers the
    //! `Mat3` ops in isolation; these tests pin the per-shape local-tensor
    //! VALUES and the world-tensor `R₀ · I⁻¹_local · R₀ᵀ` construction that the
    //! gather builds — the values the solver's effective mass depends on.

    use super::*;
    use crate::components::{BodyType, ColliderShape};

    /// Builds a `RigidBody` at the given orientation with everything else default.
    fn body_with_rotation(rotation: Quat) -> RigidBody {
        RigidBody {
            position: Vec3::ZERO,
            linear_velocity: Vec3::ZERO,
            rotation,
            angular_velocity: Vec3::ZERO,
        }
    }

    /// Builds a `RigidBodyMass` with the given inverse mass (dynamic, the
    /// `inv_inertia` placeholder is overridden by `from_columns`).
    fn mass_with_inv_mass(inv_mass: f32) -> RigidBodyMass {
        RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass,
            restitution: 0.5,
            friction: 0.3,
            body_type: if inv_mass == 0.0 {
                BodyType::Static
            } else {
                BodyType::Dynamic
            },
        }
    }

    fn collider_shape(shape: ColliderShape) -> Collider {
        Collider {
            shape,
            layer: 1,
            mask: 1,
        }
    }

    /// A solid sphere derives the isotropic local inverse inertia
    /// `inv_mass · 5 / (2·r²)` on each diagonal (off-diagonals zero).
    #[test]
    fn from_columns_sphere_local_tensor_values() {
        // r = 0.5, inv_mass = 2.0 ⇒ inv = 2·5 / (2·0.25) = 10 / 0.5 = 20.
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(2.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.5 });

        let state = BodyState::from_columns(&body, &mass, &collider);
        let i = state.inv_inertia_local;
        let expected = 20.0_f32;
        assert!((i.rows[0].x - expected).abs() < 1e-4, "Ixx⁻¹: {}", i.rows[0].x);
        assert!((i.rows[1].y - expected).abs() < 1e-4, "Iyy⁻¹: {}", i.rows[1].y);
        assert!((i.rows[2].z - expected).abs() < 1e-4, "Izz⁻¹: {}", i.rows[2].z);
        // Isotropic ⇒ off-diagonals zero.
        assert_eq!(i.rows[0].y, 0.0);
        assert_eq!(i.rows[0].z, 0.0);
        assert_eq!(i.rows[1].x, 0.0);
    }

    /// A box derives the per-axis local inverse inertia `12·inv_mass / (sum of
    /// the two other full-extents squared)`.
    #[test]
    fn from_columns_box_local_tensor_values() {
        // half_extents (1,2,3) ⇒ full (w,h,d) = (2,4,6); inv_mass = 3.
        //   Ixx⁻¹ = 12·3 / (h²+d²) = 36 / (16+36) = 36/52
        //   Iyy⁻¹ = 12·3 / (w²+d²) = 36 / (4+36)  = 36/40
        //   Izz⁻¹ = 12·3 / (w²+h²) = 36 / (4+16)  = 36/20
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(3.0);
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
        let i = state.inv_inertia_local;
        assert!((i.rows[0].x - 36.0 / 52.0).abs() < 1e-5, "Ixx⁻¹: {}", i.rows[0].x);
        assert!((i.rows[1].y - 36.0 / 40.0).abs() < 1e-5, "Iyy⁻¹: {}", i.rows[1].y);
        assert!((i.rows[2].z - 36.0 / 20.0).abs() < 1e-5, "Izz⁻¹: {}", i.rows[2].z);
    }

    /// A static body (`inv_mass == 0`) derives `Mat3::ZERO` (infinite inertia),
    /// for both local AND world tensors — no angular response.
    #[test]
    fn from_columns_static_body_zero_inertia() {
        let body = body_with_rotation(Quat::new(0.2, -0.4, 0.5, 0.8).normalize());
        let mass = mass_with_inv_mass(0.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.5 });

        let state = BodyState::from_columns(&body, &mass, &collider);
        assert_eq!(state.inv_inertia_local, Mat3::ZERO, "static local tensor is ZERO");
        // World tensor R·ZERO·Rᵀ is also ZERO regardless of orientation.
        assert_eq!(state.inv_inertia, Mat3::ZERO, "static world tensor is ZERO");
        assert_eq!(state.inv_mass, 0.0);
    }

    /// A degenerate (non-positive radius) sphere derives `Mat3::ZERO` rather than
    /// dividing by zero (`local_inv_inertia` guards `radius <= 0`).
    #[test]
    fn from_columns_degenerate_sphere_zero_inertia() {
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(1.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.0 });

        let state = BodyState::from_columns(&body, &mass, &collider);
        assert_eq!(
            state.inv_inertia_local,
            Mat3::ZERO,
            "degenerate radius must not divide by zero"
        );
    }

    /// At identity orientation the WORLD tensor equals the LOCAL tensor
    /// (`R₀ = IDENTITY ⇒ R₀·I·R₀ᵀ = I`).
    #[test]
    fn from_columns_world_equals_local_at_identity() {
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(1.0);
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
        assert_eq!(
            state.inv_inertia, state.inv_inertia_local,
            "world tensor equals local tensor when R₀ == IDENTITY"
        );
    }

    /// For a rotated body the WORLD tensor `R₀ · I⁻¹_local · R₀ᵀ` is symmetric
    /// (a similarity transform of a diagonal tensor) and is NOT the local tensor
    /// (the rotation actually applied).
    #[test]
    fn from_columns_world_tensor_is_symmetric_under_rotation() {
        let body = body_with_rotation(Quat::new(0.2, -0.4, 0.5, 0.8).normalize());
        let mass = mass_with_inv_mass(1.0);
        // An anisotropic box so the rotation visibly mixes the axes.
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
        let w = state.inv_inertia;
        assert!((w.rows[0].y - w.rows[1].x).abs() < 1e-5, "M[0][1]==M[1][0]");
        assert!((w.rows[0].z - w.rows[2].x).abs() < 1e-5, "M[0][2]==M[2][0]");
        assert!((w.rows[1].z - w.rows[2].y).abs() < 1e-5, "M[1][2]==M[2][1]");
        assert_ne!(
            state.inv_inertia, state.inv_inertia_local,
            "a non-identity rotation must change the world tensor"
        );
    }

    /// `PhysicsConfig::default()` carries the W1 soft-constraint set (OQ-5:
    /// substeps 1→4) so a hand-built default matches the plan's tunables.
    #[test]
    fn physics_config_default_w1_tunables() {
        let cfg = PhysicsConfig::default();
        assert_eq!(cfg.substeps, 4, "OQ-5: default substeps is 4");
        assert_eq!(cfg.relax_iterations, 2);
        assert_eq!(cfg.contact_hertz, 30.0);
        assert_eq!(cfg.contact_damping, 10.0);
        assert_eq!(cfg.dt, 0.0, "dt is a placeholder until gather stamps it");
        assert!(!cfg.colored, "O4: colored is OFF by default (the 0%-gate)");
        assert!(!cfg.soft_body, "SP1: soft_body is OFF by default (the 0%-gate)");
    }

    // ── O4: ConstraintGraph islands + coloring sanity tests ──
    //
    // Tiny hand-built graphs (these are SANITY checks; the exhaustive coloring-
    // invariant / island-BFS / determinism proptests are the tester's).

    /// Builds an empty manifold between two dense body rows (the only fields the
    /// partition reads are `body_a` / `body_b`).
    fn edge(a: u32, b: u32) -> Manifold {
        Manifold::new(BodyIndex(a), BodyIndex(b))
    }

    /// Re-scans the produced coloring and asserts no color shares a dynamic body
    /// (the O4 invariant), returns the number of colors.
    fn assert_coloring_invariant(
        g: &ConstraintGraph,
        manifolds: &[Manifold],
        is_dynamic: &impl Fn(u32) -> bool,
    ) -> u32 {
        use std::collections::HashSet;
        let mut total = 0usize;
        for c in 0..g.n_colors() {
            let mut seen: HashSet<u32> = HashSet::new();
            for &mi in g.color(c) {
                let m = &manifolds[mi as usize];
                for &row in &[m.body_a.0, m.body_b.0] {
                    if is_dynamic(row) {
                        assert!(
                            seen.insert(row),
                            "color {c} reuses dynamic body {row} (coloring invariant)"
                        );
                    }
                }
            }
            total += g.color(c).len();
        }
        assert_eq!(total, manifolds.len(), "every manifold appears in exactly one color");
        g.n_colors()
    }

    /// A triangle of three dynamic bodies (every pair touching) is one island and
    /// needs 3 colors (each pair of edges shares a vertex), with the invariant held.
    #[test]
    fn graph_triangle_one_island_three_colors() {
        let manifolds = [edge(0, 1), edge(1, 2), edge(0, 2)];
        let dyn3 = |row: u32| row < 3; // all three dynamic
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 3, dyn3);

        assert_eq!(g.n_islands(), 1, "a connected triangle is one island");
        assert_eq!(g.island_of(0), 0);
        assert_eq!(g.island_of(1), 0);
        assert_eq!(g.island_of(2), 0);
        assert_eq!(g.island(0).len(), 3, "all three edges file under the island");

        let n_colors = assert_coloring_invariant(&g, &manifolds, &dyn3);
        // A triangle's edges pairwise share a vertex → each needs its own color.
        assert_eq!(n_colors, 3, "triangle edges need 3 colors");
    }

    /// Two disjoint dynamic pairs `(0-1)` and `(2-3)` form two islands; both edges
    /// are body-disjoint so a single color suffices.
    #[test]
    fn graph_two_disjoint_pairs_two_islands_one_color() {
        let manifolds = [edge(0, 1), edge(2, 3)];
        let dyn4 = |row: u32| row < 4;
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 4, dyn4);

        assert_eq!(g.n_islands(), 2, "two disjoint pairs are two islands");
        // The two edges share no body → both fit in color 0.
        assert_eq!(g.n_colors(), 1, "body-disjoint edges share one color");
        assert_coloring_invariant(&g, &manifolds, &dyn4);
        // The two islands carry one manifold each.
        assert_eq!(g.island(g.island_of(0)).len(), 1);
        assert_eq!(g.island(g.island_of(2)).len(), 1);
        assert_ne!(g.island_of(0), g.island_of(2), "bodies 0 and 2 are in different islands");
    }

    /// A static body (`inv_mass == 0`, row 0) is GROUND: it does NOT connect the
    /// islands of the two dynamic bodies it touches (Box2D's rule), is never an
    /// island node (`NO_ISLAND`), and imposes no coloring occupancy (both
    /// dyn-vs-ground edges share one color despite sharing the static body).
    #[test]
    fn graph_static_is_ground_does_not_merge_or_constrain() {
        // Row 0 = static ground; rows 1 and 2 are dynamic, each touching ground.
        let manifolds = [edge(0, 1), edge(0, 2)];
        let dyn_pred = |row: u32| row == 1 || row == 2; // row 0 static
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 3, dyn_pred);

        // Ground does not merge: bodies 1 and 2 are SEPARATE islands.
        assert_eq!(g.n_islands(), 2, "ground does not merge two dynamic islands");
        assert_eq!(g.island_of(0), ConstraintGraph::NO_ISLAND, "static = NO_ISLAND");
        assert_ne!(g.island_of(1), g.island_of(2), "1 and 2 stay in distinct islands");

        // Ground imposes no occupancy → both edges fit in one color even though
        // they share the static body 0.
        assert_eq!(g.n_colors(), 1, "shared ground does not split colors");
        assert_coloring_invariant(&g, &manifolds, &dyn_pred);
    }

    /// The partition is a pure deterministic function of its input: building the
    /// same chain twice (and reusing a warmed graph) yields identical CSR output.
    #[test]
    fn graph_build_is_deterministic_and_reusable() {
        // A 5-body chain 0-1-2-3-4 (one island; a path needs only 2 colors).
        let manifolds = [edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4)];
        let dyn5 = |row: u32| row < 5;

        let mut a = ConstraintGraph::default();
        a.build(&manifolds, 5, dyn5);
        let colors_a: Vec<Vec<u32>> = (0..a.n_colors()).map(|c| a.color(c).to_vec()).collect();
        let island_of_a: Vec<u32> = (0..5).map(|r| a.island_of(r)).collect();

        // Reuse the SAME warmed graph for a second build — capacity reused, output
        // must be identical (no stale state leaks across builds).
        a.build(&manifolds, 5, dyn5);
        let colors_a2: Vec<Vec<u32>> = (0..a.n_colors()).map(|c| a.color(c).to_vec()).collect();
        assert_eq!(colors_a, colors_a2, "rebuild on a warmed graph is identical");

        // A fresh graph must match too (no dependence on prior state).
        let mut b = ConstraintGraph::default();
        b.build(&manifolds, 5, dyn5);
        let colors_b: Vec<Vec<u32>> = (0..b.n_colors()).map(|c| b.color(c).to_vec()).collect();
        let island_of_b: Vec<u32> = (0..5).map(|r| b.island_of(r)).collect();
        assert_eq!(colors_a, colors_b, "fresh vs warmed graph: identical coloring");
        assert_eq!(island_of_a, island_of_b, "fresh vs warmed graph: identical islands");

        assert_eq!(a.n_islands(), 1, "the chain is one connected island");
        assert_eq!(a.n_colors(), 2, "a path graph is 2-colorable");
        assert_coloring_invariant(&a, &manifolds, &dyn5);
    }

    // ── O3 parallel candidate emit — shaped-path gates (in-lib) ───────────────
    //
    // These cover the multi-pass SHAPED emit (`build_emit_shaped_forced`) at
    // forced `n_chunks ∈ {1, 2, 4, 8}`, single-threaded (NO pool), so they:
    //   * exercise the restructured Pass A count / serial prefix-sum / Pass B
    //     `pair_offset` arithmetic + the `EmitPtrs` disjoint raw writes,
    //   * run under `cargo +nightly miri test` (the pool spin is Miri-intractable;
    //     the shaped path needs no pool — `pool = None`),
    //   * prove byte-identity to the O2 serial `build` AND non-vacuity (the W2
    //     anti-vacuity bar: it ran the shaped passes, not a `build` delegate).
    // The pool-driven `build_parallel` native MT gate + the dense criterion live
    // in `tests/broadphase_grid.rs` / `benches/broadphase.rs` (separate crates —
    // `build_emit_shaped_forced` is `#[cfg(test)] pub(crate)`, reachable ONLY here).
    mod o3_shaped {
        use super::*;
        use crate::components::ColliderShape;

        /// The forced chunk counts the shaped-path gates sweep (W ∈ {1, 2, 4, 8}).
        const FORCED_CHUNKS: [usize; 4] = [1, 2, 4, 8];

        /// A `BodyState` carrying only the broadphase-relevant fields.
        fn sphere(position: Vec3, radius: f32) -> BodyState {
            BodyState {
                position,
                shape: ColliderShape::Sphere { radius },
                ..Default::default()
            }
        }

        fn boxx(position: Vec3, half: Vec3) -> BodyState {
            BodyState {
                position,
                shape: ColliderShape::Box { half_extents: half },
                ..Default::default()
            }
        }

        /// The reference all-pairs broadphase — the LITERAL production predicate
        /// (same operand order, same `(min, max)` emission, already sorted).
        fn all_pairs(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
            let mut pairs = Vec::new();
            let n = bodies.len();
            for i in 0..n {
                for j in (i + 1)..n {
                    let bound =
                        body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
                    let delta = bodies[j].position - bodies[i].position;
                    if delta.length_squared() <= bound * bound {
                        pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
                    }
                }
            }
            pairs
        }

        /// The O2 serial `build` output for `bodies` (the bit-identity reference).
        fn serial_build(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build(bodies, &mut out);
            out
        }

        /// The shaped-path output at a forced `n_chunks` (single-threaded, no pool).
        fn shaped_build(bodies: &[BodyState], n_chunks: usize) -> Vec<(BodyIndex, BodyIndex)> {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_emit_shaped_forced(bodies, &mut out, n_chunks);
            out
        }

        /// Asserts the shaped path at EVERY forced chunk count is byte-for-byte
        /// equal to the O2 serial `build` AND to all-pairs (same multiset AND
        /// order — the headline C1/W4 partition-independence gate).
        fn assert_shaped_eq_serial_and_all_pairs(bodies: &[BodyState]) {
            let serial = serial_build(bodies);
            let reference = all_pairs(bodies);
            assert_eq!(
                serial, reference,
                "O2 serial build must equal all-pairs (sanity of the reference)"
            );
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(bodies, w);
                assert_eq!(
                    shaped, serial,
                    "shaped emit at n_chunks={w} must be byte-identical to O2 serial build"
                );
            }
        }

        // ── A tiny xorshift PRNG: a self-contained seeded scene generator so the
        //    in-lib gate needs no proptest harness (proptest's strategy runner is
        //    heavier here than a deterministic 1000-scene sweep). ──────────────
        struct Rng(u64);
        impl Rng {
            fn new(seed: u64) -> Self {
                // Avoid the zero fixed-point of xorshift.
                Rng(seed | 1)
            }
            fn next_u64(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            /// A uniform `f32` in `[lo, hi)`.
            fn range(&mut self, lo: f32, hi: f32) -> f32 {
                let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0, 1)
                lo + u * (hi - lo)
            }
            fn below(&mut self, n: usize) -> usize {
                (self.next_u64() % n as u64) as usize
            }
        }

        /// A seeded DENSE random scene: 1..=40 mixed sphere/box bodies in a
        /// bounded box so clusters AND gaps occur (the same domain as the O2
        /// `grid_equals_all_pairs` proptest, deterministically seeded).
        fn random_scene(seed: u64) -> Vec<BodyState> {
            let mut rng = Rng::new(seed);
            let n = 1 + rng.below(40);
            (0..n)
                .map(|_| {
                    let p = Vec3::new(
                        rng.range(-20.0, 20.0),
                        rng.range(-20.0, 20.0),
                        rng.range(-20.0, 20.0),
                    );
                    if rng.next_u64() & 3 < 2 {
                        sphere(p, rng.range(0.1, 3.0))
                    } else {
                        boxx(
                            p,
                            Vec3::new(
                                rng.range(0.1, 3.0),
                                rng.range(0.1, 3.0),
                                rng.range(0.1, 3.0),
                            ),
                        )
                    }
                })
                .collect()
        }

        // ── Gate 1 (shaped slice): {1, 2, 4, 8}-chunk multiset+order bit-identity
        //    over 1000 seeded dense scenes. The shaped path is partition-
        //    independent, so EVERY chunk count must reproduce the serial `build`
        //    and all-pairs output byte-for-byte. (The pool-DISPATCHED leg of
        //    Gate 1 lives in `tests/broadphase_grid.rs`'s native MT gate.) ──────
        #[test]
        fn shaped_emit_bit_identical_to_serial_over_1000_scenes() {
            for seed in 0..1000u64 {
                let bodies = random_scene(seed);
                assert_shaped_eq_serial_and_all_pairs(&bodies);
            }
        }

        // ── Gate 2: W=1-shaped == serial AND non-vacuous AND genuinely the shaped
        //    multi-pass path (not a `build` delegate). ─────────────────────────
        #[test]
        fn shaped_w1_equals_serial_and_is_non_vacuous() {
            // A dense overlapping lattice → a real, multi-cell candidate set.
            let bodies: Vec<BodyState> = (0..200)
                .map(|i| {
                    let t = i as f32;
                    sphere(
                        Vec3::new(
                            (t * 0.21).sin() * 8.0,
                            (t * 0.13).cos() * 8.0,
                            (t * 0.37).sin() * 8.0,
                        ),
                        0.5,
                    )
                })
                .collect();
            let serial = serial_build(&bodies);
            let shaped1 = shaped_build(&bodies, 1);
            assert_eq!(shaped1, serial, "shaped n_chunks=1 == O2 serial build");
            assert!(
                !shaped1.is_empty(),
                "anti-vacuity: the shaped path emitted pairs (the passes ran, not a no-op)"
            );
        }

        // ── Gate 2 (cont.): the shaped path is genuinely MULTI-pass — at
        //    n_chunks ∈ {2, 4, 8} the work is split across DISTINCT cell-range
        //    chunks (the `pair_offset` arithmetic), yet the output is unchanged.
        //    A scene with many cells AND many survivors guarantees the chunk cut
        //    actually partitions work (anti-vacuity for the parallel-emit code:
        //    a `build` delegate could not honor `n_chunks`). ────────────────────
        #[test]
        fn shaped_multichunk_partitions_work_yet_output_is_invariant() {
            // A 12³ overlapping lattice: many occupied cells AND many survivors,
            // so a chunk cut at n_chunks=8 splits the cell range into real,
            // non-trivial blocks (each emits into its own out sub-range).
            let mut bodies = Vec::new();
            for z in 0..12 {
                for y in 0..12 {
                    for x in 0..12 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            assert!(
                serial.len() > 100,
                "anti-vacuity: the lattice yields many survivors ({})",
                serial.len()
            );
            // Every chunk count reproduces the identical output despite different
            // Pass-A counts / prefix-sum cuts / Pass-B sub-range partitions.
            for w in [2usize, 4, 8] {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(
                    shaped, serial,
                    "n_chunks={w} reproduces the serial output (partition-independent)"
                );
            }
        }

        // ── Gate 3: oversized-heavy scene — the SERIAL oversized append leg. ──
        #[test]
        fn shaped_oversized_heavy_matches_serial() {
            // Pack many small bodies (fine cells) + a handful of giants that span
            // >= MAX_CELL_SPAN cells → the oversized hatch. 16³ = 4096 smalls so
            // cbrt(n) is large and the median floor keeps cells fine.
            let mut bodies = Vec::new();
            let side = 16;
            for z in 0..side {
                for y in 0..side {
                    for x in 0..side {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            for k in 0..4 {
                let f = k as f32;
                bodies.push(sphere(Vec3::new(f * 2.0 + 1.0, f * 2.0 + 1.0, f * 2.0), 25.0));
            }

            // Non-vacuity: >= 2 giants land in the hatch (oversized–oversized
            // dedup AND oversized–normal emit are both exercised).
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_emit_shaped_forced(&bodies, &mut out, 4);
            assert!(
                grid.oversized_len() >= 2,
                "size disparity must classify >= 2 bodies oversized (got {})",
                grid.oversized_len()
            );
            // The shaped path's oversized append at every chunk count == serial.
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        // ── Gate 4: edge cases via the shaped path at every chunk count. ──────
        #[test]
        fn shaped_empty_world() {
            for w in FORCED_CHUNKS {
                assert!(
                    shaped_build(&[], w).is_empty(),
                    "empty world emits no pairs (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_single_body() {
            let bodies = [sphere(Vec3::new(1.0, 2.0, 3.0), 0.5)];
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        #[test]
        fn shaped_all_coincident_c_n_2_no_dupes() {
            let bodies: Vec<BodyState> = (0..12).map(|_| sphere(Vec3::ZERO, 0.5)).collect();
            assert_shaped_eq_serial_and_all_pairs(&bodies);
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(
                    shaped.len(),
                    12 * 11 / 2,
                    "all-coincident → C(n,2) pairs, no dupes (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_far_apart_no_pairs() {
            let bodies: Vec<BodyState> = (0..16)
                .map(|i| sphere(Vec3::new(i as f32 * 1000.0, 0.0, 0.0), 0.5))
                .collect();
            assert_shaped_eq_serial_and_all_pairs(&bodies);
            for w in FORCED_CHUNKS {
                assert!(
                    shaped_build(&bodies, w).is_empty(),
                    "far-apart bodies pair with none (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_cell_boundary_and_mixed_shapes() {
            let bodies = [
                sphere(Vec3::new(0.0, 0.0, 0.0), 0.6),
                boxx(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.4, 0.4, 0.4)),
                sphere(Vec3::new(2.0, 0.0, 0.0), 0.5),
                boxx(Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.7, 0.3, 0.5)),
                sphere(Vec3::new(0.0, 1.0, 1.0), 0.5),
                boxx(Vec3::new(2.0, 2.0, 2.0), Vec3::new(0.2, 0.2, 0.2)),
            ];
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        // ── Gate 4 (cont.): a reused grid (capacity-reused pair_count/pair_offset)
        //    matches a fresh grid — no stale Pass-A/prefix state across builds. ─
        #[test]
        fn shaped_reused_grid_no_stale_pair_offset_state() {
            let scene_a: Vec<BodyState> = (0..50)
                .map(|i| sphere(Vec3::new(i as f32 * 0.7, (i % 5) as f32, 0.0), 0.5))
                .collect();
            let scene_b: Vec<BodyState> = (0..30)
                .map(|i| sphere(Vec3::new((i as f32).sin() * 5.0, 0.0, i as f32 * 0.4), 0.6))
                .collect();

            let mut reused = BroadphaseGrid::with_capacity(64);
            let mut out = Vec::new();
            // Warm on scene A at a DIFFERENT chunk count, then rebuild scene B.
            reused.build_emit_shaped_forced(&scene_a, &mut out, 8);
            reused.build_emit_shaped_forced(&scene_b, &mut out, 4);
            let reused_b = out.clone();

            let fresh_b = shaped_build(&scene_b, 4);
            assert_eq!(
                reused_b, fresh_b,
                "a reused grid matches a fresh shaped build (no stale pair_count/pair_offset)"
            );
            assert_eq!(reused_b, all_pairs(&scene_b), "and equals all-pairs");
        }

        // ── Gate 5 (Miri): the curated small-scene shaped sweep at every chunk
        //    count. `cargo +nightly miri test` runs THIS (no pool needed — the
        //    shaped path runs `pool = None`); it checks the restructured offset
        //    arithmetic + the `EmitPtrs` disjoint raw writes for TB/aliasing UB.
        //    Kept small (≈ 64 bodies) so the interpreter stays tractable. ───────
        #[test]
        fn shaped_miri_small_dense_all_chunk_counts() {
            // A 4³ overlapping lattice (64 bodies) — enough occupied cells that
            // n_chunks ∈ {2, 4, 8} cut real disjoint blocks, small enough for Miri.
            let mut bodies = Vec::new();
            for z in 0..4 {
                for y in 0..4 {
                    for x in 0..4 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            assert!(!serial.is_empty(), "anti-vacuity: the Miri lattice has survivors");
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(shaped, serial, "Miri shaped n_chunks={w} == serial");
            }
        }

        // ── Gate 5 (Miri, no-pool route): `build_parallel` called OUTSIDE any
        //    `install` frame. `try_with_active_pool` returns None under Miri (no
        //    pool — the work-stealing spin is Miri-intractable, Phase 9.1-9.3), so
        //    `build_parallel` routes through the no-pool shaped path
        //    (`emit_passes(.., 1, None)`). This pins that the production entry's
        //    Miri-reachable branch is exactly the (TB-clean) shaped path and still
        //    equals the serial build. The pool-DISPATCHED branch is covered by the
        //    native MT gate in `tests/broadphase_grid.rs` (Miri can't spin the
        //    pool — the same gating the colored-solver O6 parallel tests use). ───
        #[test]
        fn shaped_miri_build_parallel_no_pool_route_equals_serial() {
            let mut bodies = Vec::new();
            for z in 0..4 {
                for y in 0..4 {
                    for x in 0..4 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_parallel(&bodies, &mut out);
            assert_eq!(
                out, serial,
                "build_parallel's no-pool route (the Miri-reachable branch) == serial build"
            );
        }
    }
}
