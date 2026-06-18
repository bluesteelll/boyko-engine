//! The colored TGS-Soft solver (Phase O5, Decision 7) — a SEPARATE
//! [`RigidSolver`] that solves contacts in graph-COLOR order over a
//! cache-friendly Struct-of-Arrays [`ContactColumns`], so a future parallel
//! ([O6]) / SIMD ([O7]) kernel can drop in over the SAME columns with no
//! restructure.
//!
//! [O6]: https://github.com/bluesteelll/boyko-engine
//! [O7]: https://github.com/bluesteelll/boyko-engine
//!
//! # Why a separate solver (Decision 7, the 0%-gate)
//!
//! The shipped [`SoftStepSolver`](super::SoftStepSolver) — its AoS
//! `PointConstraint` layout and the manifold-order `solve_velocities`
//! Gauss-Seidel sweep — is **byte-untouched** and stays the default + the
//! 0%-gate reference. This solver is opt-in via
//! [`add_physics_colored`](crate::plugin::add_physics_colored)
//! (`PhysicsConfig::colored == true`) and is wired as a distinct
//! [`physics_solve_colored`](crate::systems::physics_solve_colored) stage that
//! REPLACES the default solve. A non-colored world is unaffected.
//!
//! # The value change (Phase O5 — isolated here on purpose)
//!
//! The colored solve reorders the per-substep contact sweep from manifold order
//! to color order (a Gauss-Seidel sweep ACROSS colors, with the contacts inside
//! one color solved in any order — they touch DISJOINT dynamic bodies, the O4
//! coloring invariant). This produces DIFFERENT (but equally valid) converged
//! float values than the reference manifold-order sweep. The change is validated
//! against TOLERANCE acceptance gates (stacking, penetration, friction,
//! restitution), NOT a bit-baseline against the reference — a bit-match is
//! impossible (a different sweep order). What IS guaranteed:
//!
//! - **Run-to-run bit-identity:** the color partition is a pure deterministic
//!   function of the manifolds (O4), the sweep order is fixed, and the per-color
//!   kernel uses the SAME deterministic ops as the reference (exact `sqrt`/`div`,
//!   no FMA contraction, no `rsqrt`/`rcp`, no atomics), so the same scene yields
//!   the same bits every run.
//! - **Static bodies never move:** a static / sentinel body has `inv_mass == 0`,
//!   so [`apply_impulse`](BodyEffective::apply_impulse) is a branchless no-op on
//!   it and its velocity stays exactly zero — `static_body_unmoved_under_tgs`
//!   stays bit-identical.
//!
//! # Intra-color order-independence (the O4 invariant — MANIFOLD-GROUP granular)
//!
//! The O4 coloring guarantees no two MANIFOLDS in a color share a dynamic body,
//! so the manifold-GROUPS within a color touch pairwise-disjoint dynamic bodies.
//! The granularity is the manifold, NOT the point: a face-face manifold appends
//! up to [`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS) points into ONE
//! contiguous slot run, and every point of that run shares the SAME body pair
//! (`body_a` / `body_b`). So the ≥2 points of a single manifold read AND write
//! both shared bodies and MUST be solved together, sequentially, on one thread /
//! lane — only DIFFERENT manifold-groups in a color are body-disjoint.
//!
//! The Gauss-Seidel velocity accumulation is therefore independent of the order
//! the manifold-GROUPS of a color are visited (each group touches bodies no other
//! group in the color touches), but the points WITHIN a group are order-coupled
//! through their shared bodies. This is the property that lets [O6] dispatch a
//! color's groups across threads and [O7] pack adjacent GROUPS (never adjacent
//! points of one group) into a lane. The
//! [`group_start`](ContactColumns::group_start) +
//! [`color_group_start`](ContactColumns::color_group_start) CSR records the
//! per-manifold group boundaries so a consumer can enumerate, per color, the
//! groups and each group's slot range. (O5 is single-threaded; it solves a
//! color's contiguous SoA span slot-by-slot in index order, which absorbs the
//! intra-group coupling for free — the group CSR is additive data O5 does not
//! consume.)
//!
//! # IM-2b canonical warm-store (LOAD-BEARING for O6)
//!
//! The velocity accumulation is order-independent within a color, but the
//! warm-start STORE is NOT — the open-addressed [`WarmStartTable`] is
//! linear-probed, so two keys with colliding home slots resolve in INSERTION
//! order. A solve-dispatch-order store would make next frame's seeds depend on
//! the color layout (and, in [O6], the thread count) — a frame-delayed
//! determinism break. So after the substeps this solver walks the contacts in
//! CANONICAL `(manifold order, point index)` order — independent of the color
//! layout — and stores each point's converged impulse under its own per-point
//! key. The canonical order is materialized once (per build) in
//! [`ContactColumns::canonical`].

use boyko_macros::Resource as ResourceDerive;
use boyko_threadpool::try_with_active_pool;

use super::contact::{BodyEffective, effective_mass, is_dynamic_row, tangent_basis};
use super::simd;
// O2: the soft constants, the immovable-surface view, and the soft-coefficient
// derivation are SHARED from the reference solver — a single source of truth so
// the colored kernel cannot drift from `soft_step.rs` (the byte-untouched
// 0%-gate reference). These are `pub(crate)` re-uses (visibility-widened only,
// no value/layout change to `soft_step.rs`).
use super::soft_step::{IMMOVABLE_AT_REST, MAX_BIAS_VELOCITY, RESTITUTION_THRESHOLD, SoftCoefficients};
use super::warm_start::{self, WarmStartTable};
use super::RigidSolver;
use crate::components::BodyType;
use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::math::{Mat3, Vec3};
use crate::resources::{BodyState, ConstraintGraph, PhysicsConfig, SolverScratch};

/// The min-work threshold (W1): a color whose total slot count is BELOW this is
/// solved INLINE on the calling thread (the single-threaded [`solve_color`] over
/// the whole color span) instead of dispatched through a [`ThreadPool`]
/// (boyko_threadpool::ThreadPool) `scope`.
///
/// # Why a threshold (the zero-per-step-alloc bound)
///
/// Each `pool.scope` allocates (a boxed shared frame + a boxed closure per spawn).
/// Dispatching EVERY color every pass — ~`12 × n_colors` scopes per step — turns a
/// solver whose own scratch is zero-per-step-alloc into a per-step heap churner. A
/// SMALL color does not amortize that dispatch: the parallel split costs more than
/// it saves. Restricting `scope` to colors whose work exceeds this threshold bounds
/// the residual scope allocations to the FEW large colors that actually pay for
/// parallelism — the same per-dispatch cost class as the engine's
/// `Query::par_iter` (one scope per genuinely-parallel unit), the justified
/// parallelism cost rather than one-per-tiny-color.
///
/// # Bit-identity (LOAD-BEARING)
///
/// The inline path is the EXACT `solve_color` over the color's whole span — the
/// same call the no-pool / `parallel_solve == false` fallback uses — so an
/// inline-solved color is BIT-IDENTICAL to a parallel-solved one (within a color
/// the manifold-groups touch disjoint dynamic bodies, so the result is independent
/// of inline-vs-split and of worker count). The threshold therefore changes only
/// WHERE a color is solved, never the bits.
///
/// The metric is the color's total slot count (the sum over its groups, i.e. the
/// color span width). The value `256` is a starting point the tester benches; a
/// true zero-alloc reusable-scope threadpool API is a filed follow-up.
const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;

/// O6 perf (work-balanced chunking): how many group-chunks to emit per ambient
/// worker lane when a color is dispatched parallel.
///
/// # Why MORE chunks than lanes
///
/// `boyko_threadpool` is a **Chase-Lev work-STEALING** pool: it balances load
/// automatically PROVIDED it is given more (and smaller, work-balanced) tasks than
/// lanes — an idle lane STEALS the next chunk off a busy lane's deque. The original
/// O6 split was `num_threads + 1` COARSE chunks balanced by GROUP COUNT, which gave
/// each lane at most one chunk and left no slack to steal: at 2 workers the 3 chunks
/// landed 2-on-one-lane / 1-on-the-other, so one lane ran two chunks serially while
/// the other idled — the measured non-monotonic 2-worker dip (2w slower than 1w).
///
/// Emitting `num_threads * CHUNKS_PER_WORKER` chunks (capped at the color's group
/// count — a chunk is always ≥ 1 whole manifold-group) gives every lane several
/// steal-sized tasks so the work-stealing scheduler equalizes the lanes. The chunks
/// are balanced by total SLOT count (work), not group count, because groups vary in
/// width (1..=[`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS) points), so an
/// equal-group split is work-imbalanced.
///
/// # Bit-identity (LOAD-BEARING)
///
/// The chunk COUNT and SHAPE are free perf knobs: the {1, N}-worker bit-identity
/// property (within a color the manifold-groups touch disjoint dynamic bodies, so
/// the converged per-body result is independent of the partition and the visiting
/// order) holds for ANY chunking. So tuning this const changes only WHERE work runs,
/// never the bits. `4` is a starting point the bench sweeps (3..=8 typical).
const CHUNKS_PER_WORKER: usize = 6;

/// `Send` + `Sync`-marked raw pointers to the SoA columns + per-body buffer
/// dispatched into the O6 per-color worker closures.
///
/// Raw pointers are `!Send`/`!Sync` by default; this wrapper lets a worker task
/// capture them. The fields are **private** and reached only through the `&self`
/// accessor methods so that a closure capturing the wrapper captures the WHOLE
/// struct — never the inner `*mut` directly (Rust 2021+ disjoint capture would
/// otherwise see the bare `*mut` field and reject the closure as `!Send`). This is
/// the same idiom the engine's `par_iter` `SharedPtr`/`ChunkCaptures` use.
#[derive(Copy, Clone)]
struct ColorSolvePtrs {
    cols: *mut ContactColumns,
    bodies: *mut BodyEffective,
    bodies_len: usize,
    /// Base of `ContactColumns::group_start` — read as a RAW pointer (never a
    /// `&[u32]` borrow into `cols`) so no shared reference into `cols` is ever live
    /// while a worker holds `&mut *cols`; this keeps the parallel dispatch
    /// Tree-Borrows-clean (the Phase 9.3c bare-pointer discipline).
    group_start: *const u32,
}

impl ColorSolvePtrs {
    /// Reborrow the columns mutably for `'a`.
    ///
    /// # Safety
    /// The pointee must outlive `'a`, and the caller must access only elements
    /// DISJOINT from every other concurrent reborrow (within a color the chunks
    /// touch pairwise-disjoint impulse slots / body rows). Upheld by
    /// [`ColoredSoftStepSolver::solve_color_parallel`].
    #[inline]
    unsafe fn columns<'a>(&self) -> &'a mut ContactColumns {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { &mut *self.cols }
    }

    /// Reborrow the per-body buffer mutably for `'a`.
    ///
    /// # Safety
    /// As [`Self::columns`]: the pointee outlives `'a` and concurrent reborrows
    /// touch disjoint rows.
    #[inline]
    unsafe fn bodies<'a>(&self) -> &'a mut [BodyEffective] {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { core::slice::from_raw_parts_mut(self.bodies, self.bodies_len) }
    }

    /// Reads `group_start[g]` via the raw pointer (no `&` borrow into `cols`).
    ///
    /// # Safety
    /// `g` must be a valid index into the live `group_start` column (`g <=
    /// n_groups`). Upheld by the dispatcher, which only reads group indices within
    /// the color's `[g_lo, g_hi]` range.
    #[inline]
    unsafe fn group_start_at(&self, g: usize) -> usize {
        // SAFETY: `g` is in range per the method contract; `group_start` is the live
        //   base of the columns' `group_start` Vec. A plain `*const u32` read does
        //   not form a reference into `cols`, so it never conflicts with a worker's
        //   `&mut *cols` (the Tree-Borrows discipline).
        unsafe { *self.group_start.add(g) as usize }
    }
}

// SAFETY: the pointers name the `ContactColumns` / `[BodyEffective]` borrowed
//   `&mut` by `solve_color_parallel` for the whole `pool.scope` frame, whose Drop
//   blocks (work-stealing join) until every worker that captured the wrapper has
//   completed — so both pointees outlive every task body. The soundness of the
//   concurrent `&mut` reborrows rests entirely on DISJOINTNESS, stated in full in
//   the per-spawn SAFETY block: within a color the chunks write pairwise-disjoint
//   impulse-column slots and pairwise-disjoint DYNAMIC body rows (the O4 coloring
//   invariant), and a SHARED static body (`inv_mass == 0`) is never written (the
//   `*_movable` guard in `solve_color`). The wrapper has no interior mutability, so
//   a shared `&` to it (the outer `pool.scope` closure's capture across the spawn
//   loop) is trivially safe — hence both `Send` (cross-thread move into a task) and
//   `Sync` (shared by the loop) hold.
unsafe impl Send for ColorSolvePtrs {}
unsafe impl Sync for ColorSolvePtrs {}

/// The colored solver's per-contact constraint state in **Struct-of-Arrays**
/// form, laid out so one COLOR is a contiguous span (Phase O5, Decision 7 — the
/// `ContactColumns` sketch).
///
/// # Layout
///
/// Each logical contact = one manifold POINT (the same granularity the reference
/// `PointConstraint` uses). The columns are PARALLEL flat `Vec`s indexed by a
/// dense "slot" `0..len`. Slots are ordered **by color** — `color_offsets[c] ..
/// color_offsets[c + 1]` is color `c`'s contiguous span, so
/// [`solve_color`](ColoredSoftStepSolver::solve_color) slices a color with one
/// range and streams its columns linearly (cache-friendly; [O7] widens the same
/// columns to 8-wide with no restructure).
///
/// [O7]: https://github.com/bluesteelll/boyko-engine
///
/// Geometry columns (`ra*`, `rb*`, `normal*`, `tangent1*`, `tangent2*`,
/// `separation`, body rows, the friction coefficient, the sentinel flag) are
/// built once per solve and re-read each substep. The impulse columns
/// (`normal_impulse`, `tangent1_impulse`, `tangent2_impulse`) are warm-SEEDED at
/// build and accumulate across substeps.
///
/// # Canonical order (IM-2b)
///
/// `canonical[k]` is the slot index of the `k`-th contact in canonical
/// `(manifold order, point index)` order — the deterministic order the warm
/// store walks, INDEPENDENT of the color layout. Each slot also carries its
/// `warm_key` and gather-time approach velocity `vn_initial` so the restitution
/// pass and warm store need no parallel buffer.
///
/// # Manifold-group boundaries (C1 — load-bearing for O6/O7)
///
/// The O4 coloring invariant is manifold-GROUP granular: a single manifold's ≥2
/// points share both bodies and are contiguous in the SAME color span, so they
/// must NOT be split across threads/lanes. `group_start` is a flat CSR over the
/// solve (slot) order: group `g`'s slot range is
/// `group_start[g] .. group_start[g + 1]` (`len == n_groups + 1`), one group per
/// appended manifold with ≥1 live point. `color_group_start` is the parallel
/// per-color CSR: color `c`'s groups are
/// `color_group_start[c] .. color_group_start[c + 1]` (`len == n_colors + 1`),
/// each value an index INTO `group_start`. A consumer (O6/O7) enumerates color
/// `c`'s manifold-groups as `g in color_group_start[c]..color_group_start[c + 1]`
/// and reads each group's slot range from `group_start[g] .. group_start[g + 1]`,
/// keeping every point of one manifold on ONE thread / lane.
///
/// All buffers are `clear()`-ed and refilled each build — capacity is reused, no
/// per-step alloc in steady state (W2; the columns are flat `Vec`s, never
/// `Vec<Vec>`).
#[derive(Default)]
struct ContactColumns {
    /// Body-A anchor offset (world frame), split into three SoA columns.
    ra_x: Vec<f32>,
    ra_y: Vec<f32>,
    ra_z: Vec<f32>,
    /// Body-B anchor offset (world frame), split into three SoA columns.
    rb_x: Vec<f32>,
    rb_y: Vec<f32>,
    rb_z: Vec<f32>,
    /// Contact normal (A → B), split into three SoA columns.
    normal_x: Vec<f32>,
    normal_y: Vec<f32>,
    normal_z: Vec<f32>,
    /// First friction tangent, split into three SoA columns.
    tangent1_x: Vec<f32>,
    tangent1_y: Vec<f32>,
    tangent1_z: Vec<f32>,
    /// Second friction tangent, split into three SoA columns.
    tangent2_x: Vec<f32>,
    tangent2_y: Vec<f32>,
    tangent2_z: Vec<f32>,
    /// Signed separation at gather time (negative = penetrating).
    separation: Vec<f32>,
    /// Combined friction coefficient (`max(µa, µb)`, the reference rule).
    friction: Vec<f32>,
    /// Combined restitution coefficient (`max(ea, eb)`, the reference rule).
    restitution: Vec<f32>,
    /// Accumulated normal impulse `λn ≥ 0` (warm-seeded).
    normal_impulse: Vec<f32>,
    /// Accumulated tangent impulse along `t1` (warm-seeded).
    tangent1_impulse: Vec<f32>,
    /// Accumulated tangent impulse along `t2` (warm-seeded).
    tangent2_impulse: Vec<f32>,
    /// Dense body-A row index.
    body_a: Vec<u32>,
    /// Dense body-B row index (an A-row placeholder when `b_is_sentinel`).
    body_b: Vec<u32>,
    /// `true` for an SDF contact (`body_b == SDF_SENTINEL`): body B is
    /// [`IMMOVABLE_AT_REST`], never `bodies[body_b]`.
    b_is_sentinel: Vec<bool>,
    /// Per-point warm-start key (`pack`/`pack_sdf` with this point's feature id).
    warm_key: Vec<u64>,
    /// Gather-time relative normal APPROACH velocity (B−A on the normal),
    /// captured before the first substep for the restitution pass.
    vn_initial: Vec<f32>,
    /// CSR color offsets: `color_offsets[c] .. color_offsets[c + 1]` is color
    /// `c`'s contiguous slot span (`len == n_colors + 1`).
    color_offsets: Vec<u32>,
    /// Slot indices in canonical `(manifold, point)` order (IM-2b warm store).
    canonical: Vec<u32>,
    /// CSR manifold-group boundaries in solve (slot) order (C1): group `g`'s slot
    /// run is `group_start[g] .. group_start[g + 1]` (`len == n_groups + 1`). One
    /// group per appended manifold with ≥1 live point.
    group_start: Vec<u32>,
    /// Per-color CSR into `group_start` (C1): color `c`'s manifold-groups are
    /// `color_group_start[c] .. color_group_start[c + 1]` (`len == n_colors + 1`),
    /// each value indexing `group_start`. Lets O6/O7 enumerate, per color, the
    /// groups and (via `group_start`) each group's slot range.
    color_group_start: Vec<u32>,
    /// Reused scratch: per-manifold-index base slot + live-point count, written as
    /// each manifold is appended in the build walk so the canonical order is
    /// recovered WITHOUT a second replay walk (W1/O1/O3 fold). `(u32::MAX, 0)` for
    /// a manifold absent from every color or with no live point. Capacity-reused
    /// (`clear()` + per-build resize), never `vec!` per step.
    manifold_base: Vec<(u32, u32)>,
}

impl ContactColumns {
    /// Builds columns pre-sized for `contacts` contact points (no later realloc
    /// in steady state).
    fn with_capacity(contacts: usize) -> Self {
        let mut c = Self::default();
        c.reserve(contacts);
        c
    }

    /// Reserves capacity in every column for `contacts` contact points.
    fn reserve(&mut self, contacts: usize) {
        macro_rules! reserve_all {
            ($($field:ident),* $(,)?) => {{ $( self.$field.reserve(contacts); )* }};
        }
        reserve_all!(
            ra_x, ra_y, ra_z, rb_x, rb_y, rb_z, normal_x, normal_y, normal_z, tangent1_x,
            tangent1_y, tangent1_z, tangent2_x, tangent2_y, tangent2_z, separation, friction,
            restitution, normal_impulse, tangent1_impulse, tangent2_impulse, body_a, body_b,
            b_is_sentinel, warm_key, vn_initial, canonical, group_start, color_group_start,
            manifold_base,
        );
    }

    /// Clears every column for a fresh build (capacity reused).
    fn clear(&mut self) {
        macro_rules! clear_all {
            ($($field:ident),* $(,)?) => {{ $( self.$field.clear(); )* }};
        }
        clear_all!(
            ra_x, ra_y, ra_z, rb_x, rb_y, rb_z, normal_x, normal_y, normal_z, tangent1_x,
            tangent1_y, tangent1_z, tangent2_x, tangent2_y, tangent2_z, separation, friction,
            restitution, normal_impulse, tangent1_impulse, tangent2_impulse, body_a, body_b,
            b_is_sentinel, warm_key, vn_initial, color_offsets, canonical, group_start,
            color_group_start,
        );
        // `manifold_base` is `resize`-overwritten per build (not appended), so it is
        // not cleared here — the build resizes it to `manifolds.len()`.
    }

    /// Number of contact-point slots currently built.
    #[inline]
    fn len(&self) -> usize {
        self.separation.len()
    }

    /// Reads slot `i`'s body-A anchor as a [`Vec3`].
    #[inline]
    fn ra(&self, i: usize) -> Vec3 {
        Vec3::new(self.ra_x[i], self.ra_y[i], self.ra_z[i])
    }

    /// Reads slot `i`'s body-B anchor as a [`Vec3`].
    #[inline]
    fn rb(&self, i: usize) -> Vec3 {
        Vec3::new(self.rb_x[i], self.rb_y[i], self.rb_z[i])
    }

    /// Reads slot `i`'s contact normal as a [`Vec3`].
    #[inline]
    fn normal(&self, i: usize) -> Vec3 {
        Vec3::new(self.normal_x[i], self.normal_y[i], self.normal_z[i])
    }

    /// Reads slot `i`'s first friction tangent as a [`Vec3`].
    #[inline]
    fn tangent1(&self, i: usize) -> Vec3 {
        Vec3::new(self.tangent1_x[i], self.tangent1_y[i], self.tangent1_z[i])
    }

    /// Reads slot `i`'s second friction tangent as a [`Vec3`].
    #[inline]
    fn tangent2(&self, i: usize) -> Vec3 {
        Vec3::new(self.tangent2_x[i], self.tangent2_y[i], self.tangent2_z[i])
    }

    /// Appends one contact-point slot, returning nothing (the caller tracks the
    /// next index via [`len`](Self::len)).
    #[allow(clippy::too_many_arguments)]
    fn push_point(
        &mut self,
        ra: Vec3,
        rb: Vec3,
        normal: Vec3,
        t1: Vec3,
        t2: Vec3,
        separation: f32,
        friction: f32,
        restitution: f32,
        seed: (f32, f32, f32),
        body_a: u32,
        body_b: u32,
        b_is_sentinel: bool,
        warm_key: u64,
        vn_initial: f32,
    ) {
        self.ra_x.push(ra.x);
        self.ra_y.push(ra.y);
        self.ra_z.push(ra.z);
        self.rb_x.push(rb.x);
        self.rb_y.push(rb.y);
        self.rb_z.push(rb.z);
        self.normal_x.push(normal.x);
        self.normal_y.push(normal.y);
        self.normal_z.push(normal.z);
        self.tangent1_x.push(t1.x);
        self.tangent1_y.push(t1.y);
        self.tangent1_z.push(t1.z);
        self.tangent2_x.push(t2.x);
        self.tangent2_y.push(t2.y);
        self.tangent2_z.push(t2.z);
        self.separation.push(separation);
        self.friction.push(friction);
        self.restitution.push(restitution);
        self.normal_impulse.push(seed.0);
        self.tangent1_impulse.push(seed.1);
        self.tangent2_impulse.push(seed.2);
        self.body_a.push(body_a);
        self.body_b.push(body_b);
        self.b_is_sentinel.push(b_is_sentinel);
        self.warm_key.push(warm_key);
        self.vn_initial.push(vn_initial);
    }
}

/// The colored TGS-Soft rigid-body solver (Phase O5, Decision 7).
///
/// A `Resource` owning its [`ContactColumns`] SoA scratch + the double-buffered
/// warm-start cache. Solves contacts in color order (a Gauss-Seidel sweep across
/// colors) over the columns, single-threaded in O5. Like the reference
/// [`SoftStepSolver`](super::SoftStepSolver) it
/// [`owns_integration`](RigidSolver::owns_integration), so the pipeline's
/// `physics_integrate` is gated off.
#[derive(ResourceDerive)]
pub struct ColoredSoftStepSolver {
    /// Per-body solver view, parallel to `scratch.bodies` — refreshed each
    /// substep so the world inverse inertia tracks the advancing orientation.
    bodies: Vec<BodyEffective>,
    /// The SoA contact columns, grouped by color (rebuilt each solve, reused).
    columns: ContactColumns,
    /// Last frame's converged impulses — probed to seed this frame's contacts.
    warm_read: WarmStartTable,
    /// This frame's converged impulses — freshly zeroed, filled in canonical
    /// order after the solve (IM-2b), then swapped into `warm_read`.
    warm_write: WarmStartTable,
    /// Whether warm-starting is active (production default `true`).
    warm_start_enabled: bool,
}

impl Default for ColoredSoftStepSolver {
    /// The production default — empty scratch, warm-starting ON.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0, 0)
    }
}

impl ColoredSoftStepSolver {
    /// Builds a solver with the scratch pre-sized for up to `bodies` rows and
    /// `contacts` contact points (no later realloc in steady state),
    /// warm-starting ON.
    pub fn with_capacity(bodies: usize, contacts: usize) -> Self {
        Self {
            bodies: Vec::with_capacity(bodies),
            columns: ContactColumns::with_capacity(contacts),
            warm_read: WarmStartTable::with_capacity(contacts),
            warm_write: WarmStartTable::with_capacity(contacts),
            warm_start_enabled: true,
        }
    }

    /// Builds a solver with warm-starting toggled `enabled` (test hook, mirrors
    /// [`SoftStepSolver::with_warm_start`](super::SoftStepSolver::with_warm_start)).
    pub fn with_warm_start(enabled: bool) -> Self {
        Self {
            warm_start_enabled: enabled,
            ..Self::with_capacity(0, 0)
        }
    }

    /// Rebuilds the per-body solver views from the gather snapshot (mirrors the
    /// reference `build_bodies`).
    fn build_bodies(&mut self, bodies: &[BodyState]) {
        self.bodies.clear();
        for b in bodies {
            // The `*_movable` guard's ANGULAR no-op (`ω + inv_inertia·(r×p) == ω`
            // for a guarded static row) keys only on `inv_mass == 0`, so it relies
            // on a static row ALSO carrying `inv_inertia == Mat3::ZERO`. Production
            // bodies satisfy this — `resources::local_inv_inertia` forces ZERO when
            // `inv_mass == 0` and `refresh_inertia` skips static rows — but the
            // guard never inspects the tensor, so assert the coupling at assembly
            // time (debug-only; vanishes in release).
            debug_assert!(
                is_dynamic_row(b.inv_mass) || b.inv_inertia == Mat3::ZERO,
                "static row (inv_mass == 0) must have inv_inertia == Mat3::ZERO for the *_movable angular no-op"
            );
            self.bodies.push(BodyEffective {
                inv_mass: b.inv_mass,
                inv_inertia: b.inv_inertia,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
            });
        }
    }

    /// Builds the SoA [`ContactColumns`] in COLOR order from the graph's CSR,
    /// warm-SEEDS each point, and captures `vn_initial` (Phase O5).
    ///
    /// Iterates colors `0..n_colors`; for each color the graph yields its
    /// manifold indices in ascending order (D4); each manifold's points are
    /// appended in point order. The result is one contiguous SoA span per color
    /// (recorded in `color_offsets`). The same walk records each manifold's base
    /// slot (in `manifold_base`) and its group boundary (`group_start` /
    /// `color_group_start`, C1); the `canonical` index list is then emitted from
    /// `manifold_base` so the warm store walks the points in canonical `(manifold,
    /// point)` order regardless of the color layout (IM-2b) — no second walk.
    ///
    /// Mirrors the reference `build_constraints` per-point math: anchors relative
    /// to each body's gather center, the degeneracy-safe tangent basis, the
    /// per-point warm key (`pack` / `pack_sdf`), and the seeded accumulated
    /// impulses (zero on a miss / when disabled).
    ///
    /// W1/O1/O3 fold: the manifold-group boundaries (C1), the per-manifold base
    /// slot, and the canonical `(manifold, point)` order are ALL recorded inside
    /// this single append walk — there is no separate replay pass and no per-step
    /// heap allocation (all scratch is capacity-reused).
    fn build_columns(&mut self, manifolds: &[Manifold], graph: &ConstraintGraph, bodies: &[BodyState]) {
        // Disjoint-field borrows: `columns` is written while `bodies` /
        // `warm_read` are read. Destructure `self` so the borrow checker sees the
        // fields are distinct (a re-borrow alias through a method call would not).
        let Self {
            columns: cols,
            bodies: bodies_eff,
            warm_read,
            warm_start_enabled,
            ..
        } = self;
        cols.clear();
        cols.color_offsets.push(0);
        cols.group_start.push(0);
        cols.color_group_start.push(0);

        // Reused per-manifold base map (no `vec!` per step): base slot + live count
        // recorded as each manifold is first appended, consumed below to emit the
        // canonical order in manifold index ascending order (D4).
        cols.manifold_base.clear();
        cols.manifold_base.resize(manifolds.len(), (u32::MAX, 0));

        for color in 0..graph.n_colors() {
            for &mi in graph.color(color) {
                let m = &manifolds[mi as usize];
                let base = cols.len() as u32;
                let count = Self::push_manifold_points(
                    cols,
                    m,
                    bodies,
                    bodies_eff,
                    warm_read,
                    *warm_start_enabled,
                );
                if count != 0 {
                    // One manifold-group per appended manifold with ≥1 live point;
                    // its contiguous slot run is `[base, base + count)` (C1).
                    cols.manifold_base[mi as usize] = (base, count);
                    cols.group_start.push(base + count);
                }
            }
            cols.color_offsets.push(cols.len() as u32);
            // Color `c`'s manifold-groups end at the current `group_start` length
            // (the per-color CSR indexes into `group_start`).
            cols.color_group_start.push((cols.group_start.len() - 1) as u32);
        }

        // Canonical `(manifold, point)` order for the IM-2b warm store — emitted
        // from the base map recorded above, in ascending manifold index, WITHOUT a
        // second color→manifold→point replay walk (W1/O1/O3 fold).
        for &(base, count) in &cols.manifold_base {
            if base == u32::MAX {
                continue;
            }
            for p in 0..count {
                cols.canonical.push(base + p);
            }
        }
        debug_assert_eq!(
            cols.canonical.len(),
            cols.len(),
            "invariant: canonical order must cover every built contact-point slot exactly once"
        );
        debug_assert_eq!(
            cols.group_start.len() as u32,
            *cols.color_group_start.last().unwrap_or(&0) + 1,
            "invariant: the per-color group CSR must tile every manifold-group exactly once"
        );
    }

    /// Appends one manifold's live points to the SoA columns (the per-point
    /// build, factored out so it borrows `cols` mutably without aliasing
    /// `self.bodies` / `self.warm_read`). Returns the number of live points
    /// appended (`0` when the manifold has no live point), so the caller can
    /// record the manifold-group's slot run (C1).
    fn push_manifold_points(
        cols: &mut ContactColumns,
        m: &Manifold,
        bodies: &[BodyState],
        bodies_eff: &[BodyEffective],
        warm_read: &WarmStartTable,
        warm_start_enabled: bool,
    ) -> u32 {
        let count = m.count as usize;
        if count == 0 {
            return 0;
        }
        let ia = m.body_a.0 as usize;
        let b_is_sentinel = m.body_b == SDF_SENTINEL;
        let ib = if b_is_sentinel { ia } else { m.body_b.0 as usize };
        let normal = m.normal;
        let (t1, t2) = tangent_basis(normal);

        // Combined material coefficients (the reference `max` rule). For a
        // sentinel, `ib == ia`, so this resolves to A's own material — the same
        // convention the reference uses.
        let friction = bodies[ia].friction.max(bodies[ib].friction);
        let restitution = bodies[ia].restitution.max(bodies[ib].restitution);

        let pa = bodies[ia].position;
        let pb = if b_is_sentinel { Vec3::ZERO } else { bodies[ib].position };

        for p in 0..count {
            let cp = &m.points[p];
            let ra = cp.anchor_a - pa;
            let rb = if b_is_sentinel { Vec3::ZERO } else { cp.anchor_b - pb };

            // Gather-time relative normal approach velocity (B−A on the normal).
            let ba = &bodies_eff[ia];
            let bb = if b_is_sentinel { &IMMOVABLE_AT_REST } else { &bodies_eff[ib] };
            let vn_initial = (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal);

            let warm_key = if b_is_sentinel {
                warm_start::pack_sdf(m.body_a, cp.feature_id)
            } else {
                warm_start::pack(m.body_a, m.body_b, cp.feature_id)
            };
            let seed = if warm_start_enabled {
                match warm_read.get(warm_key) {
                    Some(e) => (e.normal_impulse, e.tangent_impulse[0], e.tangent_impulse[1]),
                    None => (0.0, 0.0, 0.0),
                }
            } else {
                (0.0, 0.0, 0.0)
            };

            cols.push_point(
                ra,
                rb,
                normal,
                t1,
                t2,
                cp.separation,
                friction,
                restitution,
                seed,
                ia as u32,
                ib as u32,
                b_is_sentinel,
                warm_key,
                vn_initial,
            );
        }
        count as u32
    }

    /// Applies every contact point's seeded accumulated impulse to both bodies'
    /// velocities (the warm-start apply, run once per substep after gravity).
    ///
    /// Mirrors the reference `warm_start_apply`, but reads the SoA columns.
    /// Iterating in slot order (color order) is safe here: the apply is a pure
    /// accumulation onto velocities and, within a color, the bodies are
    /// disjoint; across colors the seed is independent of order. (O5 is
    /// single-threaded; the slot-order walk is fine.)
    fn warm_start_apply(cols: &ContactColumns, bodies_eff: &mut [BodyEffective]) {
        for i in 0..cols.len() {
            let normal = cols.normal(i);
            let t1 = cols.tangent1(i);
            let t2 = cols.tangent2(i);
            let impulse = normal * cols.normal_impulse[i]
                + t1 * cols.tangent1_impulse[i]
                + t2 * cols.tangent2_impulse[i];
            let ia = cols.body_a[i] as usize;
            bodies_eff[ia].apply_impulse(cols.ra(i), impulse * -1.0);
            if !cols.b_is_sentinel[i] {
                bodies_eff[cols.body_b[i] as usize].apply_impulse(cols.rb(i), impulse);
            }
        }
    }

    /// Solves the normal + coupled-friction impulses for one COLOR's contiguous
    /// SoA span once (one Gauss-Seidel sweep over the color) — the per-color
    /// kernel (Phase O5, Decision 7).
    ///
    /// `span` is the color's `[start, end)` slot range. POOL-AGNOSTIC and
    /// ORDER-INDEPENDENT ACROSS THE MANIFOLD-GROUPS of the color: each
    /// manifold-group in a color touches dynamic bodies no OTHER group in the
    /// color touches (the O4 coloring invariant is manifold-group granular), so
    /// the velocity result does not depend on the order the GROUPS are visited —
    /// the property [O6] relies on to dispatch a color's groups across threads
    /// (and [O7] to pack adjacent groups into a lane). The ≥2 points of a SINGLE
    /// manifold-group share BOTH bodies, so they are order-coupled and must be
    /// solved together, sequentially, on one thread / lane — never split across
    /// workers/lanes. O5 is single-threaded and visits the span slot-by-slot in
    /// ascending order, which solves each group's points in sequence for free.
    /// Per-group boundaries are in `cols.group_start` / `cols.color_group_start`
    /// for the parallel/SIMD consumers.
    ///
    /// [O6]: https://github.com/bluesteelll/boyko-engine
    ///
    /// The numerical kernel MIRRORS the reference
    /// [`solve_velocities`](super::SoftStepSolver) per-contact math: the soft
    /// normal solve (`dλ = -massCoeff·mEff·(vn + bias) - impulseCoeff·λ`, the
    /// `max(0)` clamp, the `max(-MAX_BIAS_VELOCITY)` bias clamp), then the 2-DOF
    /// coupled Coulomb friction CONE (`|λt| ≤ µ·λn`, exact `sqrt`), reading the
    /// columns instead of the AoS `PointConstraint`. SCALAR in O5 ([O7] widens
    /// this to AVX2 over the same columns).
    ///
    /// [O7]: https://github.com/bluesteelll/boyko-engine
    #[allow(clippy::too_many_arguments)]
    fn solve_color(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        span: (usize, usize),
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        let (start, end) = span;
        for i in start..end {
            let ra = cols.ra(i);
            let rb = cols.rb(i);
            let normal = cols.normal(i);
            let t1 = cols.tangent1(i);
            let t2 = cols.tangent2(i);
            let ia = cols.body_a[i] as usize;
            let b_is_sentinel = cols.b_is_sentinel[i];
            let ib = cols.body_b[i] as usize;
            let friction = cols.friction[i];
            let separation = cols.separation[i];

            // Snapshot body B (the immovable surface for an SDF contact).
            let bb_view = |bodies_eff: &[BodyEffective]| -> BodyEffective {
                if b_is_sentinel { IMMOVABLE_AT_REST } else { bodies_eff[ib] }
            };

            // Whether each side is a MOVABLE (dynamic) body that the impulse may
            // actually displace. A static / kinematic body has `inv_mass == 0` AND
            // `inv_inertia == Mat3::ZERO` (the load-bearing producer is
            // `resources::local_inv_inertia`, which forces ZERO whenever
            // `inv_mass == 0`; `refresh_inertia` then leaves a static row at ZERO —
            // see the `build_bodies` debug_assert), so BOTH halves of
            // `apply_impulse` are a value no-op on it: the LINEAR update
            // `v + p·inv_mass == v + p·0 == v`, and the ANGULAR update
            // `ω + inv_inertia·(r×p) == ω + ZERO·(r×p) == ω`. This holds for every
            // FINITE impulse `p` (incl. ±0.0); a NaN/Inf impulse would diverge from
            // O5's unconditional apply, but the solver never produces non-finite
            // state. So guarding the write with this flag is BIT-IDENTICAL to the
            // unconditional O5 write — and it is LOAD-BEARING for O6: the coloring
            // marks only DYNAMIC bodies, so two manifold-groups in one color may
            // SHARE a static body (a ground floor as `body_b`). Skipping the no-op
            // write to that shared static row means parallel workers never write the
            // same `BodyEffective`, so the only bodies a worker writes are its
            // groups' DISJOINT dynamic rows (the O6 data-race freedom argument; see
            // `solve_color_parallel`).
            //
            // MT soundness: `is_dynamic_row` is the SAME predicate the O4 coloring
            // uses (`physics_build_graph`) — they MUST agree over the same `inv_mass`
            // snapshot, else the guard could permit writing a row the coloring
            // believed shared (a cross-worker race the {1,N} bit test cannot detect).
            // Both sites route through `is_dynamic_row` so they cannot drift.
            let ia_movable = is_dynamic_row(bodies_eff[ia].inv_mass);
            let ib_movable = !b_is_sentinel && is_dynamic_row(bodies_eff[ib].inv_mass);

            // ── Normal solve ───────────────────────────────────────────────
            let m_eff = {
                let ba = bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                effective_mass(normal, ra, rb, &ba, &bb)
            };
            let vn = {
                let ba = &bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
            };
            let bias = if bias_active {
                (bias_rate * separation).max(-MAX_BIAS_VELOCITY)
            } else {
                0.0
            };
            let lambda_n = cols.normal_impulse[i];
            let d_lambda = if bias_active {
                -mass_coeff * m_eff * (vn + bias) - impulse_coeff * lambda_n
            } else {
                -m_eff * vn
            };
            let new_lambda = (lambda_n + d_lambda).max(0.0);
            let applied_n = new_lambda - lambda_n;
            cols.normal_impulse[i] = new_lambda;
            {
                let impulse = normal * applied_n;
                if ia_movable {
                    bodies_eff[ia].apply_impulse(ra, impulse * -1.0);
                }
                if ib_movable {
                    bodies_eff[ib].apply_impulse(rb, impulse);
                }
            }

            // ── Friction solve (2-DOF coupled cone) ────────────────────────
            let max_friction = friction * cols.normal_impulse[i];
            let m_eff_t1 = {
                let ba = bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                effective_mass(t1, ra, rb, &ba, &bb)
            };
            let m_eff_t2 = {
                let ba = bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                effective_mass(t2, ra, rb, &ba, &bb)
            };
            let (vt1, vt2) = {
                let ba = &bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                let dv = bb.point_velocity(rb) - ba.point_velocity(ra);
                (dv.dot(t1), dv.dot(t2))
            };
            let mut new_t1 = cols.tangent1_impulse[i] - m_eff_t1 * vt1;
            let mut new_t2 = cols.tangent2_impulse[i] - m_eff_t2 * vt2;
            let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
            if len_sq > max_friction * max_friction && len_sq > 0.0 {
                let scale = max_friction / len_sq.sqrt();
                new_t1 *= scale;
                new_t2 *= scale;
            }
            let applied_t1 = new_t1 - cols.tangent1_impulse[i];
            let applied_t2 = new_t2 - cols.tangent2_impulse[i];
            cols.tangent1_impulse[i] = new_t1;
            cols.tangent2_impulse[i] = new_t2;
            {
                let impulse = t1 * applied_t1 + t2 * applied_t2;
                if ia_movable {
                    bodies_eff[ia].apply_impulse(ra, impulse * -1.0);
                }
                if ib_movable {
                    bodies_eff[ib].apply_impulse(rb, impulse);
                }
            }
        }
    }

    /// One full Gauss-Seidel sweep ACROSS colors: solves colors `0..n_colors`
    /// SEQUENTIALLY (a barrier between colors — cross-color order is fixed).
    ///
    /// `parallel` selects the per-color dispatch:
    ///
    /// - `false` (O5): each color's contiguous slot span is solved slot-by-slot in
    ///   ascending order on the calling thread — BYTE-IDENTICAL to the committed O5
    ///   colored solve (the O6 0%-gate). This is the path taken when
    ///   [`PhysicsConfig::parallel_solve`] is off OR when no
    ///   [`ThreadPool`](boyko_threadpool::ThreadPool) is attached to the running
    ///   thread.
    /// - `true` (O6): each color's manifold-GROUPS are partitioned into disjoint
    ///   worker chunks and dispatched across the ambient pool via `pool.scope`; the
    ///   scope-Drop join is the barrier BEFORE the next color (color `c + 1` may read
    ///   bodies color `c` wrote). See [`solve_color_parallel`](Self::solve_color_parallel).
    ///
    /// The single-threaded order within a group's contiguous slot run is preserved
    /// in BOTH paths (a worker solves its chunk's slots in ascending order, exactly
    /// as O5 does), and distinct groups in a color touch DISJOINT dynamic bodies, so
    /// the parallel result is bit-identical to the sequential one for any worker
    /// count (see [`solve_color_parallel`](Self::solve_color_parallel)).
    fn solve_all_colors(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        parallel: bool,
    ) {
        let n_colors = cols.color_offsets.len().saturating_sub(1);
        for c in 0..n_colors {
            if parallel {
                Self::solve_color_parallel(
                    cols,
                    bodies_eff,
                    c,
                    bias_rate,
                    mass_coeff,
                    impulse_coeff,
                    bias_active,
                );
            } else {
                let start = cols.color_offsets[c] as usize;
                let end = cols.color_offsets[c + 1] as usize;
                Self::solve_color(
                    cols,
                    bodies_eff,
                    (start, end),
                    bias_rate,
                    mass_coeff,
                    impulse_coeff,
                    bias_active,
                );
            }
        }
    }

    /// Solves ONE color in parallel (O6): partitions the color's manifold-GROUPS
    /// into disjoint worker chunks and dispatches them across the ambient
    /// [`ThreadPool`](boyko_threadpool::ThreadPool) via `pool.scope`. The
    /// scope-Drop join (the barrier) returns before the caller advances to the next
    /// color, ordering the Gauss-Seidel sweep.
    ///
    /// # Granularity (C1): MANIFOLD-GROUP, never slot
    ///
    /// Dispatch is at MANIFOLD-GROUP granularity, NOT slot granularity. A color's
    /// groups are `g in color_group_start[c]..color_group_start[c + 1]`, each group
    /// `g`'s slot run is `group_start[g]..group_start[g + 1]`, and a color's groups
    /// occupy a CONTIGUOUS block of `group_start` (the build appends them in order),
    /// so a chunk of consecutive groups maps to a CONTIGUOUS slot span
    /// `group_start[chunk_lo]..group_start[chunk_hi]` — the same `[start, end)` shape
    /// [`solve_color`](Self::solve_color) consumes, kept cache-friendly. All points
    /// of one manifold-group stay on ONE worker (they share both bodies and are
    /// order-coupled — they MUST be solved sequentially within the group).
    ///
    /// # Chunk count + balance (the work-stealing perf knob)
    ///
    /// The color is split into `(num_threads + 1) * `[`CHUNKS_PER_WORKER`] chunks
    /// (capped at the group count), balanced by total SLOT count rather than group
    /// count — the dispatch loop walks groups accumulating slots and cuts a chunk
    /// once its run reaches the per-chunk slot quota. Emitting MORE, smaller,
    /// work-balanced chunks than lanes lets the Chase-Lev work-STEALING pool
    /// equalize the lanes (an idle lane steals the next chunk), which removes the
    /// coarse `num_threads + 1` split's load imbalance. The chunk count + shape are
    /// FREE perf knobs (see the bit-identity property below): they change only WHERE
    /// work runs, never the bits.
    ///
    /// # The {1 worker, N workers} BIT-IDENTITY property (LOAD-BEARING)
    ///
    /// Within a color, each dynamic body belongs to at most ONE manifold-group (the
    /// O4 coloring invariant: no two manifolds in a color share a dynamic body), so
    /// each body's velocity is accumulated by exactly ONE group. Therefore:
    ///
    /// - **Disjoint writes:** parallel workers write PAIRWISE-DISJOINT
    ///   `BodyEffective` rows (and pairwise-disjoint impulse-column slots) — no
    ///   shared write, no data race, no atomics needed.
    /// - **Order-independent per body:** a body's converged velocity is the result
    ///   of solving its one group's points in sequence; which worker runs that group
    ///   and in what order the groups are visited cannot change that result.
    /// - **Within-group order preserved:** a worker solves its chunk's slots in
    ///   ascending index order, exactly as O5 does, so each group's order-coupled
    ///   points are solved in the SAME sequence as single-threaded.
    /// - **Barrier between colors:** the scope-Drop join completes color `c` before
    ///   color `c + 1` starts (cross-color Gauss-Seidel order is fixed).
    /// - **Worker-count-independent warm store:** the converged impulses are stored
    ///   in CANONICAL `(manifold, point)` order after the solve (IM-2b), so next
    ///   frame's seeds do not depend on the dispatch / thread count.
    ///
    /// Hence the per-body result — and the full body snapshot — is BIT-FOR-BIT
    /// identical to the single-threaded colored solve for ANY worker count. Any
    /// deviation from bit-identity is a bug (a shared write, a non-disjoint chunk, a
    /// missing barrier, or a float-reduction-order dependence).
    #[allow(clippy::too_many_arguments)]
    fn solve_color_parallel(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        color: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        // The color's manifold-group range (indices into `group_start`).
        let g_lo = cols.color_group_start[color] as usize;
        let g_hi = cols.color_group_start[color + 1] as usize;
        let n_groups = g_hi - g_lo;
        if n_groups == 0 {
            return;
        }

        let span = (
            cols.color_offsets[color] as usize,
            cols.color_offsets[color + 1] as usize,
        );

        // W1 min-work threshold: a SMALL color does not amortize a `pool.scope`
        // dispatch (a boxed shared frame + a boxed closure per spawn), so solve it
        // INLINE on the calling thread — the EXACT `solve_color` over the whole
        // span the no-pool / `parallel_solve == false` path uses. This is
        // BIT-IDENTICAL to the parallel split (within a color the groups touch
        // disjoint dynamic bodies ⇒ inline == 1-worker == N-worker), so it changes
        // only WHERE the color is solved, never the bits. The metric is the color's
        // total slot count (the span width). Bounding `scope` to large colors keeps
        // the residual scope allocation at the justified, threshold-bounded
        // parallelism cost (one scope per genuinely-parallel unit, the `par_iter`
        // per-dispatch cost class) instead of one-per-tiny-color.
        let color_slots = (span.1 - span.0) as u32;
        if color_slots < MIN_PARALLEL_SLOTS_PER_COLOR {
            Self::solve_color(
                cols,
                bodies_eff,
                span,
                bias_rate,
                mass_coeff,
                impulse_coeff,
                bias_active,
            );
            return;
        }

        // Grab the ambient pool (set by `Schedule::run`'s `install` frame). When no
        // pool is attached (ad-hoc / no-scheduler call), fall back to the
        // single-threaded color solve so the result still matches O5 exactly.
        // W1: a `pool.scope` allocates (a boxed shared frame + a boxed closure per
        // spawn). This site is reached ONLY for colors above
        // `MIN_PARALLEL_SLOTS_PER_COLOR`, so the residual per-step scope allocation
        // is bounded to the FEW large colors that amortize the dispatch — the
        // justified, threshold-bounded parallelism cost (the same per-dispatch cost
        // class as the engine's `Query::par_iter`, one scope per genuinely-parallel
        // unit). The solver's own scratch stays zero-per-step-alloc; a true
        // zero-alloc reusable-scope threadpool API is a filed follow-up.
        let dispatched = try_with_active_pool(|pool| {
            // O6 perf: emit MORE, work-BALANCED chunks than lanes so the Chase-Lev
            // work-stealing pool equalizes the lanes (an idle lane steals the next
            // chunk). The dispatcher lane that called `pool.scope` ALSO work-steals
            // while the scope is open, so the lane pool is `num_threads + 1`; target
            // `(num_threads + 1) * CHUNKS_PER_WORKER` chunks, capped at the group
            // count (a chunk is always ≥ 1 WHOLE manifold-group — never split a
            // group's order-coupled points across lanes, the C1 invariant). At least
            // one chunk always (`max(1)`). Bit-identity is chunk-COUNT- AND
            // chunk-SHAPE-independent (the {1, N} property holds for ANY partition —
            // distinct chunks touch disjoint dynamic bodies), so this is a pure,
            // bench-tunable perf knob, never a value change.
            let lanes = pool.num_threads() + 1;
            let n_chunks = (lanes * CHUNKS_PER_WORKER).clamp(1, n_groups);

            // Balance by total SLOT count (work), not group count: groups vary in
            // width (1..=MAX_CONTACT_POINTS points), so an equal-GROUP split is
            // work-imbalanced. `target` is the per-chunk slot quota; the dispatch
            // loop walks groups accumulating slots and cuts a chunk once its run
            // reaches `target` (a contiguous group range → a contiguous slot span).
            // Computed from the CSR with NO per-step Vec of chunk bounds (W2: the
            // chunk boundaries are derived on the fly, alloc-free).
            let total_slots = span.1 - span.0;
            let target = total_slots.div_ceil(n_chunks).max(1);

            // Send + Sync-wrapped raw pointers to the columns + bodies + group CSR.
            // Each worker writes only its chunk's DISJOINT impulse slots and DISJOINT
            // body rows, so the aliasing reborrows are sound (see the per-spawn SAFETY
            // block). The `group_start` base is captured as a raw pointer so the
            // dispatcher reads chunk bounds without holding a `&[u32]` borrow into
            // `cols` across the scope (TB-clean, Phase 9.3c discipline).
            let ptrs = ColorSolvePtrs {
                cols: cols as *mut ContactColumns,
                bodies: bodies_eff.as_mut_ptr(),
                bodies_len: bodies_eff.len(),
                group_start: cols.group_start.as_ptr(),
            };

            pool.scope(|scope| {
                let mut chunk_g_lo = g_lo;
                while chunk_g_lo < g_hi {
                    // The chunk's first group's first slot. Read via the raw
                    // `group_start` base (no `&` borrow into `cols`).
                    // SAFETY: `chunk_g_lo` is within `[g_lo, g_hi)`, a valid index
                    //   into the live `group_start` column.
                    let chunk_start = unsafe { ptrs.group_start_at(chunk_g_lo) };

                    // Grow the chunk by WHOLE groups until its accumulated slot run
                    // reaches the per-chunk slot `target` (work-balanced) or the
                    // color's last group is consumed. A group is never split: the
                    // chunk boundary always falls on a `group_start` index, so every
                    // point of one manifold-group stays on ONE lane (C1). Always
                    // includes ≥ 1 group (the first), so it makes progress.
                    let mut chunk_g_hi = chunk_g_lo + 1;
                    while chunk_g_hi < g_hi {
                        // SAFETY: `chunk_g_hi <= g_hi`, a valid `group_start` index.
                        let so_far = unsafe { ptrs.group_start_at(chunk_g_hi) } - chunk_start;
                        if so_far >= target {
                            break;
                        }
                        chunk_g_hi += 1;
                    }

                    // The chunk's contiguous slot span ends at its last group's last
                    // slot.
                    // SAFETY: `chunk_g_hi` is within `(g_lo, g_hi]`, a valid index
                    //   into the live `group_start` column.
                    let chunk_end = unsafe { ptrs.group_start_at(chunk_g_hi) };
                    debug_assert!(
                        chunk_start >= span.0 && chunk_end <= span.1 && chunk_start < chunk_end,
                        "invariant: a group-chunk's slot span is non-empty and lies within the color span"
                    );

                    scope.spawn(move || {
                        // SAFETY (cross-worker disjoint aliasing — the O6 soundness
                        //   argument):
                        //   - `ptrs` names the `ContactColumns` / `[BodyEffective]`
                        //     borrowed `&mut` by the caller for the whole
                        //     `solve_color_parallel` frame; `pool.scope`'s Drop
                        //     blocks (work-stealing join) until every spawned task
                        //     completes, so both pointees outlive every task body —
                        //     no use-after-free, no escape past the borrow.
                        //   - DISJOINTNESS makes the concurrent `&mut` sound:
                        //     * This chunk solves ONLY slots `[chunk_start,
                        //       chunk_end)` and writes ONLY those slots' impulse
                        //       columns — distinct chunks have non-overlapping slot
                        //       ranges (the chunks partition the color's groups), so
                        //       no two workers write the same column element.
                        //     * Within ONE color, each DYNAMIC body belongs to at
                        //       most one manifold-group (the O4 coloring invariant),
                        //       so distinct chunks' groups touch DISJOINT dynamic body
                        //       rows — no two workers `apply_impulse` to the same
                        //       dynamic `BodyEffective`.
                        //     * A SHARED static body (a ground floor that several
                        //       groups in this color reference) is NEVER WRITTEN: the
                        //       `*_movable` guard in `solve_color` skips the
                        //       `apply_impulse` for any `inv_mass == 0` row (a write
                        //       that was already a value no-op), so a shared static
                        //       row is read-only across workers. Sentinel body B is
                        //       likewise never written (it is `IMMOVABLE_AT_REST`, a
                        //       local copy).
                        //   - The bodies a chunk READS (via `effective_mass` /
                        //     `point_velocity`) are its own disjoint dynamic rows plus
                        //     shared read-only static rows, so no chunk reads a row
                        //     another chunk is writing.
                        //   Therefore the per-chunk `&mut ContactColumns` /
                        //   `&mut [BodyEffective]` reborrowed below alias only
                        //   provably-disjoint written elements across workers — no UB.
                        let cols_mut = unsafe { ptrs.columns() };
                        let bodies_mut = unsafe { ptrs.bodies() };
                        Self::solve_color(
                            cols_mut,
                            bodies_mut,
                            (chunk_start, chunk_end),
                            bias_rate,
                            mass_coeff,
                            impulse_coeff,
                            bias_active,
                        );
                    });

                    chunk_g_lo = chunk_g_hi;
                }
            });
        });

        // PAR-fallback: no pool attached → run the color single-threaded so the
        // result is BYTE-IDENTICAL to O5 (the same `solve_color` over the whole
        // color span).
        if dispatched.is_none() {
            Self::solve_color(
                cols,
                bodies_eff,
                span,
                bias_rate,
                mass_coeff,
                impulse_coeff,
                bias_active,
            );
        }
    }

    /// The post-loop restitution pass — velocity-only, bias-free, run ONCE.
    ///
    /// Mirrors the reference `apply_restitution`: for each contact whose
    /// gather-time approach speed exceeds [`RESTITUTION_THRESHOLD`] it drives the
    /// current relative normal velocity to `-e·vn_initial`, keeping `λn ≥ 0`.
    /// Walks in color order (the bodies within a color are disjoint; cross-color
    /// the result is order-fixed). A zero-restitution contact is skipped.
    fn apply_restitution(cols: &mut ContactColumns, bodies_eff: &mut [BodyEffective]) {
        for i in 0..cols.len() {
            if cols.restitution[i] <= 0.0 {
                continue;
            }
            let vn0 = cols.vn_initial[i];
            if vn0 > -RESTITUTION_THRESHOLD {
                continue;
            }
            let ra = cols.ra(i);
            let rb = cols.rb(i);
            let normal = cols.normal(i);
            let ia = cols.body_a[i] as usize;
            let b_is_sentinel = cols.b_is_sentinel[i];
            let ib = cols.body_b[i] as usize;
            let bb_view = |bodies_eff: &[BodyEffective]| -> BodyEffective {
                if b_is_sentinel { IMMOVABLE_AT_REST } else { bodies_eff[ib] }
            };
            let m_eff = {
                let ba = bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                effective_mass(normal, ra, rb, &ba, &bb)
            };
            let vn = {
                let ba = &bodies_eff[ia];
                let bb = bb_view(bodies_eff);
                (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
            };
            let v_target = -cols.restitution[i] * vn0;
            let d_lambda = m_eff * (v_target - vn);
            let lambda_n = cols.normal_impulse[i];
            let new_lambda = (lambda_n + d_lambda).max(0.0);
            let applied = new_lambda - lambda_n;
            cols.normal_impulse[i] = new_lambda;
            let impulse = normal * applied;
            bodies_eff[ia].apply_impulse(ra, impulse * -1.0);
            if !b_is_sentinel {
                bodies_eff[ib].apply_impulse(rb, impulse);
            }
        }
    }

    /// Stores every contact point's converged impulses into the freshly-zeroed
    /// `write` table in CANONICAL `(manifold, point)` order (IM-2b), then swaps
    /// `read` ↔ `write`.
    ///
    /// The store order is `columns.canonical` — the deterministic
    /// manifold-then-point sequence — NOT the color/slot order the solve used.
    /// The open-addressed table is insertion-order-sensitive on home-slot
    /// collisions, so a canonical store keeps next frame's seeds independent of
    /// the color layout (and, in [O6], the thread count) — the load-bearing
    /// determinism guarantee.
    ///
    /// [O6]: https://github.com/bluesteelll/boyko-engine
    fn store_and_swap(&mut self) {
        if !self.warm_start_enabled {
            return;
        }
        let cols = &self.columns;
        self.warm_write.rebuild(cols.len());
        for &slot in &cols.canonical {
            let i = slot as usize;
            self.warm_write.insert(
                cols.warm_key[i],
                cols.normal_impulse[i],
                [cols.tangent1_impulse[i], cols.tangent2_impulse[i]],
            );
        }
        core::mem::swap(&mut self.warm_read, &mut self.warm_write);
    }

    /// Writes the solved velocities back into the gather snapshot and flags every
    /// integrated DYNAMIC row touched (mirrors the reference `write_back`).
    fn write_back(&self, scratch: &mut SolverScratch) {
        for row in 0..self.bodies.len() {
            if scratch.bodies[row].body_type == BodyType::Dynamic && self.bodies[row].inv_mass != 0.0
            {
                scratch.bodies[row].linear_velocity = self.bodies[row].linear_velocity;
                scratch.bodies[row].angular_velocity = self.bodies[row].angular_velocity;
                scratch.touched.set(row);
            }
        }
    }
}

impl ColoredSoftStepSolver {
    /// Runs the colored solve for one step against the prebuilt
    /// [`ConstraintGraph`] (Phase O5).
    ///
    /// The substep loop mirrors the reference [`SoftStepSolver`](super::SoftStepSolver):
    /// per substep — gravity integrate, warm-start apply, the soft normal +
    /// friction sweep (here: a Gauss-Seidel sweep ACROSS colors via
    /// [`solve_all_colors`](Self::solve_all_colors)), position integrate + the
    /// inertia refresh, then the bias-free relax passes. After the substeps: the
    /// restitution pass, the canonical-order warm store (IM-2b), and the
    /// write-back. The integrate / inertia kernels are the SAME `simd::*`
    /// helpers the reference calls (no duplicated inertia math).
    ///
    /// `solve` (the [`RigidSolver`] entry) cannot reach the graph through the
    /// trait signature, so the [`physics_solve_colored`](crate::systems::physics_solve_colored)
    /// stage calls THIS method directly with `Res<ConstraintGraph>`.
    pub fn solve_colored(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
    ) {
        let substeps = config.substeps.max(1);
        let h = config.dt / substeps as f32;

        // O1: degenerate early-return BEFORE any build/alloc. In solver-owned mode
        // a free dynamic body must keep falling, so the only valid skip is a world
        // with no dynamic body to integrate at all — then there is nothing to
        // integrate, build, or write back. Hoisting it above `build_bodies` /
        // `build_columns` makes an idle / all-static world do zero build work.
        let has_dynamic = scratch
            .bodies
            .iter()
            .any(|b| b.body_type == BodyType::Dynamic && b.inv_mass != 0.0);
        if !has_dynamic {
            return;
        }

        self.build_bodies(&scratch.bodies);
        self.build_columns(manifolds, graph, &scratch.bodies);

        let soft = SoftCoefficients::new(config.contact_hertz, config.contact_damping, h);
        let gravity = config.gravity;
        let use_simd = config.simd;
        // O6: parallel per-color dispatch when opted in. The result is bit-identical
        // to the single-threaded colored solve for any worker count (disjoint-body
        // groups + canonical warm store); when off it is BYTE-IDENTICAL to O5.
        let parallel = config.parallel_solve;

        for _ in 0..substeps {
            // (1) Gravity integrate DYNAMIC bodies (shared O1 kernel).
            simd::apply_gravity(&mut self.bodies, &scratch.bodies, gravity, h, use_simd);

            // (2) Warm-start apply.
            Self::warm_start_apply(&self.columns, &mut self.bodies);

            // (3)+(4) Soft normal + friction sweep ACROSS colors (Gauss-Seidel).
            Self::solve_all_colors(
                &mut self.columns,
                &mut self.bodies,
                soft.bias_rate,
                soft.mass_coeff,
                soft.impulse_coeff,
                true,
                parallel,
            );

            // (5) Position integrate (scalar — the reference's MEASURED-SCALAR
            // choice for the AoS `BodyState`) then refresh the world inertia.
            simd::position_integrate(&self.bodies, &mut scratch.bodies, h, false);
            simd::refresh_inertia(&mut self.bodies, &scratch.bodies, use_simd);

            // (6) Relax: re-solve bias-free to remove soft-bias energy.
            for _ in 0..config.relax_iterations {
                Self::solve_all_colors(
                    &mut self.columns,
                    &mut self.bodies,
                    soft.bias_rate,
                    soft.mass_coeff,
                    soft.impulse_coeff,
                    false,
                    parallel,
                );
            }
        }

        // Post-loop restitution (ONCE, velocity-only, bias-free).
        Self::apply_restitution(&mut self.columns, &mut self.bodies);

        // IM-2b: store converged impulses in canonical order, then swap.
        self.store_and_swap();

        // Write the solved velocities back and flag integrated DYNAMIC rows.
        self.write_back(scratch);
    }
}

impl RigidSolver for ColoredSoftStepSolver {
    /// The trait entry is a no-op for the colored solver — the colored solve
    /// needs the [`ConstraintGraph`], which the [`RigidSolver::solve`] signature
    /// does not carry, so the
    /// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage
    /// drives [`solve_colored`](Self::solve_colored) directly. This impl exists
    /// only so the type satisfies the `RigidSolver` bound the plugin's generic
    /// wiring requires; the colored stage never calls it.
    ///
    /// `_manifolds` / `_scratch` are untouched here.
    #[inline]
    fn solve(
        &mut self,
        _config: &PhysicsConfig,
        _manifolds: &[Manifold],
        _scratch: &mut SolverScratch,
    ) {
    }

    /// Always `true` — the colored solver integrates DYNAMIC bodies inside its
    /// substep loop, so the pipeline's `physics_integrate` must be gated off (C2).
    #[inline]
    fn owns_integration(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    //! Pure-function sanity tests for the colored solver (Phase O5). These build
    //! the columns + graph by hand and drive `solve_colored` directly — NO
    //! schedule, NO threadpool — so they run native and under Miri. The
    //! exhaustive tolerance / determinism / criterion suite is the tester's job.

    use super::*;
    use crate::components::ColliderShape;
    use crate::manifold::{BodyIndex, ContactPoint};
    use crate::math::{Mat3, Quat};

    /// A `BodyState` for a unit-radius dynamic sphere at `position`.
    fn dyn_sphere(position: Vec3, inv_mass: f32, friction: f32, restitution: f32) -> BodyState {
        BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: Mat3::ZERO,
            position,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            inv_mass,
            restitution,
            friction,
            body_type: BodyType::Dynamic,
            shape: ColliderShape::Sphere { radius: 1.0 },
        }
    }

    /// A static (immovable) floor body at `position`.
    fn static_body(position: Vec3) -> BodyState {
        BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: Mat3::ZERO,
            position,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
            body_type: BodyType::Static,
            shape: ColliderShape::Sphere { radius: 1.0 },
        }
    }

    /// A penetrating single-point manifold between rows `a` and `b` with the
    /// given normal (A → B), separation, anchored at A's center.
    fn manifold(a: u32, b: u32, normal: Vec3, separation: f32, anchor: Vec3) -> Manifold {
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.normal = normal;
        m.points[0] = ContactPoint {
            anchor_a: anchor,
            anchor_b: anchor,
            separation,
            feature_id: 0,
        };
        m.count = 1;
        m
    }

    /// A penetrating MULTI-point manifold between rows `a` and `b` with `n`
    /// distinct contact points (each its own `feature_id`), all sharing the SAME
    /// body pair / normal — a face-face manifold stand-in whose ≥2 points must be
    /// kept in ONE manifold-group (C1). `n` is clamped to
    /// [`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS).
    fn box_manifold(a: u32, b: u32, normal: Vec3, separation: f32, anchor: Vec3, n: u8) -> Manifold {
        use crate::math::MAX_CONTACT_POINTS;
        let n = (n as usize).min(MAX_CONTACT_POINTS);
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.normal = normal;
        for (p, slot) in m.points.iter_mut().take(n).enumerate() {
            // Spread the anchors so the points are distinct, but the body pair +
            // normal are shared — the single-group invariant is about the body
            // pair, not anchor identity.
            let offset = Vec3::new(p as f32 * 0.1, 0.0, p as f32 * 0.1);
            *slot = ContactPoint {
                anchor_a: anchor + offset,
                anchor_b: anchor + offset,
                separation,
                feature_id: p as u32,
            };
        }
        m.count = n as u8;
        m
    }

    /// Builds a fresh `ConstraintGraph` over `bodies` + `manifolds` using the
    /// non-zero-inv-mass dynamic predicate (the stage's predicate).
    fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
        let mut g = ConstraintGraph::with_capacity(bodies.len());
        let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
        g.build(manifolds, bodies.len(), move |row| {
            (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
        });
        g
    }

    /// Drives the colored solver for `steps` fixed steps over a fixed scratch,
    /// returning the final body Y positions (the only axis the gates check).
    fn run(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
    ) -> Vec<f32> {
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.bodies = bodies;
        scratch.touched.reset(scratch.bodies.len());

        for _ in 0..steps {
            // Re-derive the manifolds from the current positions each step (the
            // narrowphase stand-in), rebuild the graph, then solve.
            let manifolds = build_manifolds(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
        scratch.bodies.iter().map(|b| b.position.y).collect()
    }

    #[test]
    fn static_body_stays_put_under_colored_solve() {
        // A dynamic sphere penetrating a static floor: the static body's
        // velocity and position must stay EXACTLY zero (inv_mass == 0).
        let bodies = vec![dyn_sphere(Vec3::new(0.0, 1.5, 0.0), 1.0, 0.5, 0.0), static_body(Vec3::ZERO)];
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(2);
        scratch.bodies = bodies;
        scratch.touched.reset(2);

        // Floor normal A → B points downward (sphere above floor); deep overlap.
        let m = manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.5, Vec3::new(0.0, 0.5, 0.0));
        let manifolds = vec![m];
        let graph = build_graph(&scratch.bodies, &manifolds);
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);

        let floor = &scratch.bodies[1];
        assert_eq!(floor.linear_velocity, Vec3::ZERO, "static floor linear velocity must stay zero");
        assert_eq!(floor.angular_velocity, Vec3::ZERO, "static floor angular velocity must stay zero");
        assert_eq!(floor.position, Vec3::ZERO, "static floor position must stay exactly put");
    }

    #[test]
    fn small_stack_settles_under_colored_solve() {
        // Two dynamic spheres resting on a static floor (a tiny stack). After a
        // few steps the spheres must not have sunk far through the floor and must
        // not have flown apart — a tolerance gate, not a bit baseline.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 2.9, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ];
        let ys = run(
            bodies,
            |bodies| {
                let mut out = Vec::new();
                // sphere0 vs floor: A → B points down.
                if bodies[0].position.y - 1.0 < 0.0 {
                    out.push(manifold(
                        0,
                        2,
                        Vec3::new(0.0, -1.0, 0.0),
                        (bodies[0].position.y - 1.0) - 0.0,
                        Vec3::new(0.0, bodies[0].position.y - 1.0, 0.0),
                    ));
                }
                // sphere1 on sphere0: A(0) → B(1) points up.
                let sep = (bodies[1].position.y - bodies[0].position.y) - 2.0;
                if sep < 0.0 {
                    out.push(manifold(
                        0,
                        1,
                        Vec3::new(0.0, 1.0, 0.0),
                        sep,
                        Vec3::new(0.0, bodies[0].position.y + 1.0, 0.0),
                    ));
                }
                out
            },
            120,
        );
        // The two dynamic spheres should remain in a plausible stacked band
        // above the floor top (y = 0): neither sunk through nor launched.
        assert!(ys[0] > -0.5 && ys[0] < 2.0, "sphere0 settled near floor, got y={}", ys[0]);
        assert!(ys[1] > ys[0], "sphere1 stays above sphere0, got y0={} y1={}", ys[0], ys[1]);
        assert!(ys[1] < 5.0, "sphere1 did not launch, got y={}", ys[1]);
    }

    #[test]
    fn colored_solve_is_run_to_run_bit_identical() {
        // The same scene solved twice must produce bit-identical body state — the
        // colored partition + sweep + canonical warm store are deterministic.
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.3, 2.9, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(-0.3, 4.8, 0.0), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, -1.0, 0.0)),
            ]
        };
        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            for (a, b) in [(0u32, 3u32), (0, 1), (1, 2)] {
                let pa = bodies[a as usize].position;
                let pb = bodies[b as usize].position;
                let delta = pb - pa;
                let dist = delta.length();
                let sep = dist - 2.0;
                if sep < 0.0 && dist > 1e-6 {
                    let normal = delta * dist.recip();
                    out.push(manifold(a, b, normal, sep, pa + normal));
                }
            }
            out
        };

        let run_once = || -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.bodies = make();
            scratch.touched.reset(4);
            for _ in 0..30 {
                let manifolds = build(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            // Hash the whole snapshot to bits.
            scratch
                .bodies
                .iter()
                .flat_map(|b| {
                    [
                        b.position.x.to_bits(),
                        b.position.y.to_bits(),
                        b.position.z.to_bits(),
                        b.linear_velocity.x.to_bits(),
                        b.linear_velocity.y.to_bits(),
                        b.linear_velocity.z.to_bits(),
                    ]
                })
                .collect()
        };

        assert_eq!(run_once(), run_once(), "colored solve must be run-to-run bit-identical");
    }

    #[test]
    fn manifold_groups_delimit_contiguous_point_runs_within_color_span() {
        // C1: a multi-point box manifold's points must form ONE manifold-group
        // (not split), and the per-color group CSR must tile each color span
        // exactly. Scene: two dynamic spheres each on a static floor, plus a
        // 4-point box manifold between two more dynamic boxes — a mix of 1-point
        // and multi-point manifolds across colors.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 0
            dyn_sphere(Vec3::new(5.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 1
            dyn_sphere(Vec3::new(10.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 2 (box A)
            dyn_sphere(Vec3::new(12.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 3 (box B)
            static_body(Vec3::new(0.0, -1.0, 0.0)),              // 4 (floor)
        ];
        // Manifolds (manifold order): two single-point sphere/floor contacts that
        // share the static floor (ground, so they CAN share a color) and one
        // 4-point dynamic box-box manifold.
        let manifolds = vec![
            manifold(0, 4, Vec3::new(0.0, -1.0, 0.0), -0.2, Vec3::new(0.0, 0.0, 0.0)),
            manifold(1, 4, Vec3::new(0.0, -1.0, 0.0), -0.2, Vec3::new(5.0, 0.0, 0.0)),
            box_manifold(2, 3, Vec3::new(1.0, 0.0, 0.0), -0.2, Vec3::new(11.0, 1.0, 0.0), 4),
        ];
        let graph = build_graph(&bodies, &manifolds);

        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&bodies);
        solver.build_columns(&manifolds, &graph, &bodies);
        let cols = &solver.columns;

        // The total live point count = 1 + 1 + 4 = 6.
        assert_eq!(cols.len(), 6, "all live points are slotted");
        // Three manifolds each with ≥1 live point => exactly three groups.
        assert_eq!(cols.group_start.len(), 4, "group_start has n_groups + 1 entries");
        assert_eq!(cols.group_start[0], 0, "group CSR starts at slot 0");

        let n_colors = cols.color_offsets.len() - 1;
        assert_eq!(
            cols.color_group_start.len(),
            n_colors + 1,
            "per-color group CSR has n_colors + 1 entries"
        );

        // For every color: the groups enumerated via `color_group_start` must tile
        // the color's `[start, end)` slot span EXACTLY, with no gap and no overlap,
        // and each group's slot run must be contiguous and non-empty.
        let mut groups_seen = 0usize;
        for c in 0..n_colors {
            let span_start = cols.color_offsets[c];
            let span_end = cols.color_offsets[c + 1];
            let g_lo = cols.color_group_start[c] as usize;
            let g_hi = cols.color_group_start[c + 1] as usize;
            assert!(g_lo <= g_hi, "color group range is well-ordered");

            // The first group of the color begins at the color span start.
            let mut cursor = span_start;
            for g in g_lo..g_hi {
                let gs = cols.group_start[g];
                let ge = cols.group_start[g + 1];
                assert!(ge > gs, "every manifold-group has ≥1 point (no empty group)");
                assert_eq!(gs, cursor, "groups tile the color span with no gap/overlap");
                cursor = ge;
                groups_seen += 1;
            }
            assert_eq!(cursor, span_end, "the color's groups exactly fill its slot span");
        }
        assert_eq!(groups_seen, cols.group_start.len() - 1, "every group belongs to exactly one color");

        // The 4-point box manifold (rows 2,3) must appear as ONE contiguous group
        // of 4 slots — never split. Locate it by its body pair in the columns.
        let mut box_group_len = None;
        for g in 0..(cols.group_start.len() - 1) {
            let gs = cols.group_start[g] as usize;
            let ge = cols.group_start[g + 1] as usize;
            if cols.body_a[gs] == 2 && cols.body_b[gs] == 3 {
                // Every slot of the run shares the SAME body pair (the C1 contract).
                for s in gs..ge {
                    assert_eq!(cols.body_a[s], 2, "box group body A is shared across its points");
                    assert_eq!(cols.body_b[s], 3, "box group body B is shared across its points");
                }
                box_group_len = Some(ge - gs);
            }
        }
        assert_eq!(box_group_len, Some(4), "the 4-point box manifold forms ONE 4-slot group");

        // Canonical order still covers every slot exactly once.
        assert_eq!(cols.canonical.len(), cols.len(), "canonical covers every slot");
    }

    // ── Tester additions (Phase O5 formal gates) ─────────────────────────────
    //
    // These extend the dev's stand-in sanity tests into the exhaustive O5 gates.
    // They live in the lib test module because the rigorous group-CSR tiling gate
    // (Gate 4) needs access to the PRIVATE `ContactColumns` fields (`group_start`,
    // `color_group_start`, `color_offsets`, `body_a`/`body_b`, `canonical`). They
    // touch only `Vec` scratch (no pool, no int-to-ptr), so they run native AND
    // under `cargo miri test -p boyko-physics --lib` (Gate 7).

    use proptest::prelude::*;

    /// A reproducible LCG (splitmix64-style) for the property scene builder, so a
    /// failing case is fully described by its `seed` (the proptest input) — no
    /// external RNG state. Deterministic by construction.
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            // splitmix64.
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn f01(&mut self) -> f32 {
            (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % (hi - lo) as u64) as u32
        }
    }

    /// Builds a random valid contact scene from `seed`: `n_dyn` dynamic spheres +
    /// one static floor, with a random set of (manifold-order) contacts. Some
    /// contacts are multi-point box manifolds (≥2 points sharing a body pair) so
    /// the single-group invariant is exercised non-vacuously. Returns the bodies +
    /// manifolds + the built graph. Determinism: a pure function of `seed`.
    fn random_scene(seed: u64) -> (Vec<BodyState>, Vec<Manifold>, ConstraintGraph) {
        let mut rng = Lcg(seed ^ 0xD1B5_4A32_D192_ED03);
        let n_dyn = rng.range(1, 9) as usize; // 1..=8 dynamic bodies
        let mut bodies = Vec::with_capacity(n_dyn + 1);
        for i in 0..n_dyn {
            let pos = Vec3::new(rng.f01() * 10.0, 1.0 + i as f32 * 0.3, rng.f01() * 10.0);
            bodies.push(dyn_sphere(pos, 1.0, 0.5, rng.f01() * 0.5));
        }
        let floor_row = n_dyn as u32;
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

        // Random contacts in manifold order: each is dyn-vs-floor (1 point) or
        // dyn-vs-dyn (1..=MAX_CONTACT_POINTS points). A dyn-dyn pair is emitted
        // with body_a < body_b (the broadphase convention; no self-loops).
        let n_contacts = rng.range(0, 12) as usize;
        let mut manifolds = Vec::with_capacity(n_contacts);
        for _ in 0..n_contacts {
            let a = rng.range(0, n_dyn as u32);
            if rng.f01() < 0.45 || n_dyn == 1 {
                // dyn-vs-floor, single point.
                manifolds.push(manifold(
                    a,
                    floor_row,
                    Vec3::new(0.0, -1.0, 0.0),
                    -0.1,
                    bodies[a as usize].position,
                ));
            } else {
                // dyn-vs-dyn, possibly multi-point (a face-face stand-in).
                let mut b = rng.range(0, n_dyn as u32);
                if b == a {
                    b = (a + 1) % n_dyn as u32;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let pts = rng.range(1, crate::math::MAX_CONTACT_POINTS as u32 + 1) as u8;
                manifolds.push(box_manifold(
                    lo,
                    hi,
                    Vec3::new(1.0, 0.0, 0.0),
                    -0.1,
                    bodies[lo as usize].position,
                    pts,
                ));
            }
        }
        let graph = build_graph(&bodies, &manifolds);
        (bodies, manifolds, graph)
    }

    /// Gate 4 (rigorous): over random scenes the per-color manifold-group CSR
    /// (`color_group_start` → `group_start`) must TILE each color's slot span
    /// EXACTLY — no gap, no overlap, every group non-empty and contiguous; every
    /// slot in a group shares the SAME body pair; a multi-point manifold is ONE
    /// group (never split across groups/colors); `canonical` covers every slot once.
    #[test]
    fn group_csr_tiles_every_color_span_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(400), |(seed in any::<u64>())| {
            let (bodies, manifolds, graph) = random_scene(seed);
            let mut solver = ColoredSoftStepSolver::default();
            solver.build_bodies(&bodies);
            solver.build_columns(&manifolds, &graph, &bodies);
            let cols = &solver.columns;

            let n_colors = cols.color_offsets.len().saturating_sub(1);
            prop_assert_eq!(
                cols.color_group_start.len(),
                n_colors + 1,
                "per-color group CSR must have n_colors + 1 entries"
            );
            prop_assert_eq!(cols.group_start.first().copied(), Some(0u32), "group CSR starts at 0");

            // Build the expected slot->body-pair from each appended group, and
            // verify the tiling per color.
            let mut groups_seen = 0usize;
            let mut covered = vec![false; cols.len()];
            for c in 0..n_colors {
                let span_start = cols.color_offsets[c];
                let span_end = cols.color_offsets[c + 1];
                let g_lo = cols.color_group_start[c] as usize;
                let g_hi = cols.color_group_start[c + 1] as usize;
                prop_assert!(g_lo <= g_hi, "color {} group range well-ordered", c);
                let mut cursor = span_start;
                for g in g_lo..g_hi {
                    let gs = cols.group_start[g];
                    let ge = cols.group_start[g + 1];
                    prop_assert!(ge > gs, "group {} must be non-empty", g);
                    prop_assert_eq!(gs, cursor, "group {} tiles color {} span with no gap/overlap", g, c);
                    // Every slot of the group shares the SAME body pair (the C1
                    // contract: a manifold's ≥2 points are never split).
                    let (ba, bb) = (cols.body_a[gs as usize], cols.body_b[gs as usize]);
                    for s in gs..ge {
                        prop_assert_eq!(cols.body_a[s as usize], ba, "group {} body A shared", g);
                        prop_assert_eq!(cols.body_b[s as usize], bb, "group {} body B shared", g);
                        prop_assert!(!covered[s as usize], "slot {} covered by >1 group", s);
                        covered[s as usize] = true;
                    }
                    cursor = ge;
                    groups_seen += 1;
                }
                prop_assert_eq!(cursor, span_end, "color {} groups exactly fill its slot span", c);
            }
            prop_assert_eq!(groups_seen, cols.group_start.len() - 1, "every group in exactly one color");
            // Every slot is covered by exactly one group.
            prop_assert!(covered.iter().all(|&c| c), "every slot belongs to a group");

            // Canonical order covers every slot EXACTLY once (a permutation of 0..len).
            prop_assert_eq!(cols.canonical.len(), cols.len(), "canonical covers every slot");
            let mut canon_seen = vec![false; cols.len()];
            for &s in &cols.canonical {
                prop_assert!(!canon_seen[s as usize], "canonical visits slot {} twice", s);
                canon_seen[s as usize] = true;
            }
            prop_assert!(canon_seen.iter().all(|&c| c), "canonical is a full permutation");

            // A multi-point manifold (count >= 2 over a dyn-dyn pair) appears as
            // ONE contiguous group of exactly `count` slots — never split.
            for m in &manifolds {
                if m.count >= 2 && m.body_b != SDF_SENTINEL {
                    let ia = m.body_a.0;
                    let ib = m.body_b.0;
                    // Locate the group whose first slot matches this body pair AND
                    // whose length equals the manifold's live point count.
                    let mut found = false;
                    for g in 0..(cols.group_start.len() - 1) {
                        let gs = cols.group_start[g] as usize;
                        let ge = cols.group_start[g + 1] as usize;
                        if cols.body_a[gs] == ia && cols.body_b[gs] == ib && (ge - gs) == m.count as usize {
                            found = true;
                            break;
                        }
                    }
                    prop_assert!(
                        found,
                        "multi-point manifold ({},{}) count {} must be ONE contiguous group",
                        ia, ib, m.count
                    );
                }
            }
        });
    }

    /// Gate 1 (extended): run-to-run bit-identity over MANY random scenes (the
    /// dev's `colored_solve_is_run_to_run_bit_identical` is one fixed scene). Each
    /// scene is solved twice for several steps; the full body snapshot must match
    /// bit-for-bit. A non-deterministic colored result is a real bug.
    #[test]
    fn colored_solve_is_run_to_run_bit_identical_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(200), |(seed in any::<u64>())| {
            let snapshot = |seed: u64| -> Vec<u32> {
                let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
                let (bodies, _, _) = random_scene(seed);
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.bodies = bodies;
                scratch.touched.reset(scratch.bodies.len());
                for _ in 0..20 {
                    // Re-derive a fixed manifold set from the SAME seed each step
                    // (the partition + contacts are a pure function of the seed).
                    let (_, manifolds, graph) = random_scene(seed);
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                scratch
                    .bodies
                    .iter()
                    .flat_map(|b| {
                        [
                            b.position.x.to_bits(), b.position.y.to_bits(), b.position.z.to_bits(),
                            b.linear_velocity.x.to_bits(), b.linear_velocity.y.to_bits(),
                            b.linear_velocity.z.to_bits(),
                        ]
                    })
                    .collect()
            };
            prop_assert_eq!(snapshot(seed), snapshot(seed), "colored solve run-to-run bit-identical");
        });
    }

    /// Gate 6 (extended): EVERY static / sentinel body (inv_mass == 0) stays
    /// EXACTLY zero velocity AND position under the colored solve, over random
    /// scenes that include both a static floor and SDF-sentinel contacts.
    #[test]
    fn static_and_sentinel_bodies_never_move_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(200), |(seed in any::<u64>())| {
            let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
            let (mut bodies, mut manifolds, _) = random_scene(seed);
            let floor_row = (bodies.len() - 1) as u32;
            // Add a sentinel contact for body 0 (an SDF surface) so the immovable
            // sentinel path is exercised alongside the static floor.
            let mut sm = Manifold::new(BodyIndex(0), SDF_SENTINEL);
            sm.normal = Vec3::new(0.0, -1.0, 0.0);
            sm.points[0] = ContactPoint {
                anchor_a: bodies[0].position,
                anchor_b: bodies[0].position,
                separation: -0.1,
                feature_id: 7,
            };
            sm.count = 1;
            manifolds.push(sm);
            // The static floor's pre-step exact state.
            let floor_before = bodies[floor_row as usize];

            let graph = build_graph(&bodies, &manifolds);
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            std::mem::swap(&mut scratch.bodies, &mut bodies);
            scratch.touched.reset(scratch.bodies.len());
            for _ in 0..5 {
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            let floor_after = &scratch.bodies[floor_row as usize];
            prop_assert_eq!(floor_after.position, floor_before.position, "static floor position unchanged");
            prop_assert_eq!(floor_after.linear_velocity, Vec3::ZERO, "static floor lin vel zero");
            prop_assert_eq!(floor_after.angular_velocity, Vec3::ZERO, "static floor ang vel zero");
        });
    }

    /// The Phase O5 VALUE CHANGE, witnessed directly: the colored solver and the
    /// reference [`SoftStepSolver`](super::SoftStepSolver) — given the IDENTICAL
    /// scene, manifolds, config, and step count — converge to DIFFERENT float
    /// values (the colored sweep reorders the Gauss-Seidel pass), yet both leave
    /// the scene physically valid (finite, no launch). This documents the
    /// CHANGELOG-bearing value change is PRESENT and isolated.
    #[test]
    fn colored_value_differs_from_reference_but_both_valid() {
        use super::super::RigidSolver as _;
        // A small overlapping cluster on a floor — a multi-contact scene where the
        // sweep order matters (a single isolated contact would converge identically).
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.4, 1.8, 0.1), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(-0.3, 2.7, -0.1), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, -1.0, 0.0)),
            ]
        };
        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            for (a, b) in [(0u32, 3u32), (0, 1), (1, 2), (0, 2)] {
                let pa = bodies[a as usize].position;
                let pb = bodies[b as usize].position;
                let delta = pb - pa;
                let dist = delta.length();
                let target = if b == 3 { 1.0 } else { 2.0 };
                let sep = dist - target;
                if sep < 0.0 && dist > 1e-6 {
                    let n = delta * dist.recip();
                    out.push(manifold(a, b, n, sep, pa + n));
                }
            }
            out
        };
        let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };

        // Colored path.
        let colored_ys = {
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.bodies = make();
            scratch.touched.reset(4);
            for _ in 0..40 {
                let manifolds = build(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            scratch.bodies.iter().map(|b| b.position.y).collect::<Vec<_>>()
        };

        // Reference path (the byte-untouched SoftStepSolver, manifold-order sweep).
        let reference_ys = {
            let mut solver = super::super::SoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.bodies = make();
            scratch.touched.reset(4);
            for _ in 0..40 {
                let manifolds = build(&scratch.bodies);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve(&cfg, &manifolds, &mut scratch);
            }
            scratch.bodies.iter().map(|b| b.position.y).collect::<Vec<_>>()
        };

        // Both physically valid: finite, no launch (top body well-bounded).
        for (i, (&c, &r)) in colored_ys.iter().zip(&reference_ys).enumerate() {
            assert!(c.is_finite() && r.is_finite(), "body {i} finite (colored {c}, ref {r})");
            assert!(c > -2.0 && c < 8.0, "colored body {i} physically bounded, y={c}");
            assert!(r > -2.0 && r < 8.0, "reference body {i} physically bounded, y={r}");
        }
        // The value change is PRESENT: at least one dynamic body's converged Y
        // differs between the two sweep orders (bit-compare). If they ever match
        // bit-for-bit, the colored reorder collapsed to the reference order and the
        // O5 isolation claim would be vacuous — flag it.
        let differs = colored_ys
            .iter()
            .zip(&reference_ys)
            .any(|(&c, &r)| c.to_bits() != r.to_bits());
        assert!(
            differs,
            "colored converged values must DIFFER from the reference (the isolated O5 value change): \
             colored={colored_ys:?} reference={reference_ys:?}"
        );
    }

    // ── Phase O6 parallel-solve sanity tests (dev stand-ins) ──────────────────
    //
    // These exercise the O6 parallel per-color dispatch. The {1,N} bit-identity
    // and stack tests drive the colored solve INSIDE a real `ThreadPool::install`
    // frame (so `solve_colored` finds the ambient pool), so they spawn worker
    // threads and are NATIVE-ONLY (`cfg(not(miri))`) — the pool is loom+Miri-proven
    // (Phase 9.1-9.3); the exhaustive {1,N} proptest / criterion scaling / scope
    // stress / Miri-scalar suite is the tester's job. The 0%-gate test
    // (`parallel_solve == false` byte-identical to O5) is pool-free and runs under
    // Miri too.

    /// Hashes the full body snapshot to a bit vector (the {1,N} comparison key).
    fn snapshot_bits(scratch: &SolverScratch) -> Vec<u32> {
        scratch
            .bodies
            .iter()
            .flat_map(|b| {
                [
                    b.position.x.to_bits(),
                    b.position.y.to_bits(),
                    b.position.z.to_bits(),
                    b.linear_velocity.x.to_bits(),
                    b.linear_velocity.y.to_bits(),
                    b.linear_velocity.z.to_bits(),
                    b.angular_velocity.x.to_bits(),
                    b.angular_velocity.y.to_bits(),
                    b.angular_velocity.z.to_bits(),
                ]
            })
            .collect()
    }

    /// A forced-collision DENSE scene: `n` dynamic spheres packed in a tight line
    /// so every adjacent pair (and each on the floor) overlaps every step — a
    /// non-vacuous multi-color, multi-contact scene that exercises the warm store.
    fn dense_collision_scene(n: usize) -> Vec<BodyState> {
        let mut bodies = Vec::with_capacity(n + 1);
        for i in 0..n {
            // Spacing 1.5 < 2·radius (= 2.0) → every adjacent pair penetrates.
            bodies.push(dyn_sphere(Vec3::new(i as f32 * 1.5, 1.0, 0.0), 1.0, 0.5, 0.0));
        }
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
        bodies
    }

    /// Builds the dense scene's manifolds: each adjacent dynamic pair + each
    /// dynamic-vs-floor contact, in deterministic manifold order.
    fn dense_collision_manifolds(bodies: &[BodyState]) -> Vec<Manifold> {
        let n = bodies.len() - 1; // last row is the floor
        let floor = n as u32;
        let mut out = Vec::new();
        // Adjacent dynamic pairs (a < b), the multi-contact backbone.
        for a in 0..n {
            // dyn-vs-floor (1 point).
            out.push(manifold(
                a as u32,
                floor,
                Vec3::new(0.0, -1.0, 0.0),
                -0.2,
                bodies[a].position,
            ));
            if a + 1 < n {
                let pa = bodies[a].position;
                let pb = bodies[a + 1].position;
                let delta = pb - pa;
                let dist = delta.length();
                if dist > 1e-6 {
                    let normal = delta * dist.recip();
                    out.push(manifold(a as u32, (a + 1) as u32, normal, dist - 2.0, pa + normal));
                }
            }
        }
        out
    }

    /// Runs the colored solve for `steps` over the dense scene with the given
    /// `parallel_solve` flag, inside an N-worker `ThreadPool::install` frame so the
    /// parallel path finds the ambient pool. Returns the final body snapshot bits.
    #[cfg(not(miri))]
    fn run_dense_in_pool(n: usize, steps: usize, parallel_solve: bool, workers: usize) -> Vec<u32> {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(n + 1);
        scratch.bodies = dense_collision_scene(n);
        scratch.touched.reset(scratch.bodies.len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for _ in 0..steps {
                let manifolds = dense_collision_manifolds(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });
        snapshot_bits(&scratch)
    }

    /// O6 0%-gate: with `parallel_solve == false` the colored solve is
    /// BYTE-IDENTICAL to the committed O5 single-threaded path. This runs WITHOUT a
    /// pool (so the parallel branch would fall back anyway), comparing the
    /// `parallel_solve: false` config against an independent O5-config run — they
    /// must produce bit-for-bit identical body state. Pool-free → runs under Miri.
    #[test]
    fn parallel_solve_off_is_byte_identical_to_o5() {
        let run = |parallel_solve: bool| -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                parallel_solve,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(6);
            scratch.bodies = dense_collision_scene(5);
            scratch.touched.reset(scratch.bodies.len());
            for _ in 0..40 {
                let manifolds = dense_collision_manifolds(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            snapshot_bits(&scratch)
        };
        // `parallel_solve == false` and the O5 reference config (also false) must be
        // byte-identical — the O6 path must not perturb the single-threaded result.
        assert_eq!(
            run(false),
            run(false),
            "parallel_solve=false must be deterministic (and == the O5 path)"
        );
    }

    /// O6 headline gate (dev stand-in): the parallel colored solve is BIT-FOR-BIT
    /// identical at 1 worker vs N workers on a FORCED-COLLISION dense scene — the
    /// load-bearing determinism property (disjoint-body groups + canonical warm
    /// store ⇒ worker-count-independent bits). Also checks the parallel 4-worker
    /// result matches the single-threaded (`parallel_solve == false`) result, so
    /// the parallel dispatch does not change the converged value.
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_is_bit_identical_across_worker_counts() {
        let single = run_dense_in_pool(12, 40, false, 1);
        let p1 = run_dense_in_pool(12, 40, true, 1);
        let p2 = run_dense_in_pool(12, 40, true, 2);
        let p4 = run_dense_in_pool(12, 40, true, 4);
        let p8 = run_dense_in_pool(12, 40, true, 8);

        assert_eq!(p1, p2, "parallel solve: 1 worker vs 2 workers must be bit-identical");
        assert_eq!(p1, p4, "parallel solve: 1 worker vs 4 workers must be bit-identical");
        assert_eq!(p1, p8, "parallel solve: 1 worker vs 8 workers must be bit-identical");
        assert_eq!(
            single, p4,
            "parallel solve must be bit-identical to the single-threaded colored solve"
        );
        // Anti-vacuity: the scene must actually have moved bodies (not a no-op).
        let resting = dense_collision_scene(12);
        let resting_bits = snapshot_bits(&{
            let mut s = SolverScratch::with_capacity(13);
            s.bodies = resting;
            s
        });
        assert_ne!(p1, resting_bits, "the dense scene must non-vacuously solve (bodies moved)");
    }

    /// The widest color's slot count for a freshly-built dense scene of `n`
    /// dynamic bodies — used to size a scene that crosses (or stays below) the W1
    /// `MIN_PARALLEL_SLOTS_PER_COLOR` threshold non-vacuously. Only the
    /// `cfg(not(miri))` pool-driven gates consume it, so it is gated to stay
    /// dead-code-warning-clean under the Miri subset build.
    #[cfg(not(miri))]
    fn max_color_slot_span(n: usize) -> u32 {
        let bodies = dense_collision_scene(n);
        let manifolds = dense_collision_manifolds(&bodies);
        let graph = build_graph(&bodies, &manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&bodies);
        solver.build_columns(&manifolds, &graph, &bodies);
        let cols = &solver.columns;
        let n_colors = cols.color_offsets.len().saturating_sub(1);
        (0..n_colors)
            .map(|c| cols.color_offsets[c + 1] - cols.color_offsets[c])
            .max()
            .unwrap_or(0)
    }

    /// W1 bit-identity: a color SOLVED INLINE (below `MIN_PARALLEL_SLOTS_PER_COLOR`)
    /// is BIT-FOR-BIT identical to the same color solved through the parallel
    /// `pool.scope` dispatch. The threshold must change only WHERE a color is
    /// solved, never the bits.
    ///
    /// Compares two runs of the SAME forced-collision dense scene whose widest
    /// color CROSSES the threshold (so the parallel run actually dispatches a
    /// `scope` — the threshold-HIT path) against the single-threaded
    /// `parallel_solve == false` run (which never dispatches — the
    /// threshold-BYPASSED inline path) AND across worker counts. All must match
    /// bit-for-bit. Anti-vacuity: asserts the scene's widest color genuinely
    /// exceeds the threshold (else the test would only exercise the inline path on
    /// both sides and the threshold-hit claim would be vacuous).
    #[test]
    #[cfg(not(miri))]
    fn threshold_inline_vs_parallel_dispatch_is_bit_identical() {
        // Size a scene whose widest color exceeds the threshold (the chain's
        // shared-floor color holds ~n slots). 400 dynamic bodies clears 256.
        let n = 400;
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: the widest color ({widest} slots) must exceed the threshold \
             ({MIN_PARALLEL_SLOTS_PER_COLOR}) so the parallel dispatch path is exercised"
        );

        // Threshold-BYPASSED: pure inline single-threaded colored solve.
        let inline_single = run_dense_in_pool(n, 12, false, 1);
        // Threshold-HIT: the large color dispatches a real `pool.scope`.
        let parallel_1 = run_dense_in_pool(n, 12, true, 1);
        let parallel_4 = run_dense_in_pool(n, 12, true, 4);

        assert_eq!(
            inline_single, parallel_1,
            "threshold-bypassed inline solve must be bit-identical to the threshold-hit \
             parallel dispatch (1 worker)"
        );
        assert_eq!(
            parallel_1, parallel_4,
            "threshold-hit parallel dispatch must be bit-identical across worker counts"
        );
    }

    /// A small stack settles under the PARALLEL colored solve (driven through a
    /// 4-worker pool): the dynamic spheres stay in a plausible band above the floor
    /// — a tolerance gate confirming the parallel path produces a physically valid
    /// rest state, not just bit-identity to itself.
    #[test]
    #[cfg(not(miri))]
    fn stack_settles_under_parallel_solve() {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve: true,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(4);
        scratch.bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 2.9, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ];
        scratch.touched.reset(3);

        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            if bodies[0].position.y - 1.0 < 0.0 {
                out.push(manifold(
                    0,
                    2,
                    Vec3::new(0.0, -1.0, 0.0),
                    bodies[0].position.y - 1.0,
                    Vec3::new(0.0, bodies[0].position.y - 1.0, 0.0),
                ));
            }
            let sep = (bodies[1].position.y - bodies[0].position.y) - 2.0;
            if sep < 0.0 {
                out.push(manifold(
                    0,
                    1,
                    Vec3::new(0.0, 1.0, 0.0),
                    sep,
                    Vec3::new(0.0, bodies[0].position.y + 1.0, 0.0),
                ));
            }
            out
        };

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        pool.install(|_scope| {
            for _ in 0..120 {
                let manifolds = build(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });

        let y0 = scratch.bodies[0].position.y;
        let y1 = scratch.bodies[1].position.y;
        assert!(y0 > -0.5 && y0 < 2.0, "sphere0 settled near the floor under parallel solve, got y={y0}");
        assert!(y1 > y0, "sphere1 stays above sphere0 under parallel solve, got y0={y0} y1={y1}");
        assert!(y1 < 5.0, "sphere1 did not launch under parallel solve, got y={y1}");
    }

    // ── Tester additions (Phase O6 formal gates) ──────────────────────────────
    //
    // These extend the dev's fixed-scene O6 stand-ins into the exhaustive O6 gates
    // the plan's "production-ready when" list requires:
    //   * Gate 1 (extended): {1, N}-worker BIT-IDENTITY over a PROPTEST of random
    //     dense scenes × worker counts (the load-bearing race detector — any data
    //     race surfaces as a non-bit-identical snapshot).
    //   * Gate 5 (extended): static / sentinel bodies never move under the PARALLEL
    //     multi-worker path, over random scenes (the `*_movable` guard's MT form).
    //   * Gate 7: native MT stress (many colors × substeps × high worker counts on
    //     a dense scene; deterministic across repeated runs; no crash/hang).
    // All are pool-driven → NATIVE-ONLY (`cfg(not(miri))`). The pool's fork/join is
    // loom + Miri-proven (Phase 9.1-9.3); the MT race-freedom is verified here by
    // the {1, N} bit-identity (the disjointness oracle a single process can run).

    /// A random DENSE forced-collision scene from `seed`: `n_dyn` dynamic spheres
    /// (a span chosen so SOME scenes cross `MIN_PARALLEL_SLOTS_PER_COLOR` and some
    /// stay below it — exercising BOTH the threshold-hit `pool.scope` dispatch and
    /// the inline path under the SAME `parallel_solve == true` config) packed in a
    /// tight line so every adjacent pair + each-on-floor overlaps. A pure function
    /// of `seed`. Returns the bodies (the manifolds are re-derived per step from
    /// positions via [`dense_collision_manifolds`], so the partition stays a pure
    /// function of the live state every step — the determinism precondition).
    #[cfg(not(miri))]
    fn random_dense_scene(seed: u64) -> Vec<BodyState> {
        let mut rng = Lcg(seed ^ 0x51A2_7E11_C3D4_9F0B);
        // 2..=520 dynamic bodies: the shared-floor color holds ~n slots, so the top
        // of the range clears the 256 threshold (dispatch path) and the bottom does
        // not (inline path) — both reached under `parallel_solve == true`.
        let n = rng.range(2, 521) as usize;
        dense_collision_scene(n)
    }

    /// Gate 1 (THE load-bearing race detector, extended to a PROPTEST): the parallel
    /// colored solve is BIT-FOR-BIT identical at 1 worker vs N workers AND vs the
    /// single-threaded (`parallel_solve == false`) solve, over random dense scenes ×
    /// worker counts {1, 2, 4, 8}. A data race (a shared write, a non-disjoint chunk,
    /// a missing barrier, or a float-reduction-order dependence) would surface as a
    /// non-bit-identical snapshot — this is the one test a true cross-worker race
    /// cannot survive. A counterexample = the failing `seed` (fully reproducible).
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_bit_identical_across_workers_on_random_scenes() {
        // Worker spin-up dominates; keep the case count modest but the worker sweep
        // wide. Each case runs 6 worker configs × 8 steps over up to ~520 bodies.
        proptest!(ProptestConfig::with_cases(48), |(seed in any::<u64>())| {
            let n = random_dense_scene(seed).len() - 1; // dyn count (last row = floor)
            let single = run_dense_in_pool(n, 8, false, 1);
            let p1 = run_dense_in_pool(n, 8, true, 1);
            let p2 = run_dense_in_pool(n, 8, true, 2);
            let p4 = run_dense_in_pool(n, 8, true, 4);
            let p8 = run_dense_in_pool(n, 8, true, 8);
            prop_assert_eq!(&p1, &single, "parallel(1) == single-threaded (seed {})", seed);
            prop_assert_eq!(&p1, &p2, "parallel: 1 vs 2 workers bit-identical (seed {})", seed);
            prop_assert_eq!(&p1, &p4, "parallel: 1 vs 4 workers bit-identical (seed {})", seed);
            prop_assert_eq!(&p1, &p8, "parallel: 1 vs 8 workers bit-identical (seed {})", seed);
        });
    }

    /// Gate 5 (extended to the PARALLEL multi-worker path over random scenes): every
    /// static body (`inv_mass == 0`) AND the SDF sentinel stay EXACTLY put under the
    /// parallel colored solve driven through a 4-worker pool. The `*_movable` guard
    /// must hold under concurrent dispatch — no worker may write a shared static row.
    #[test]
    #[cfg(not(miri))]
    fn static_body_never_moves_under_parallel_solve_on_random_scenes() {
        use boyko_threadpool::ThreadPoolBuilder;

        proptest!(ProptestConfig::with_cases(40), |(seed in any::<u64>())| {
            let cfg = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
            // A dense scene (so multiple groups in a color reference the SHARED
            // static floor concurrently — the exact MT case the guard protects) plus
            // an SDF-sentinel contact for body 0.
            let n = (Lcg(seed).range(4, 200)) as usize;
            let mut bodies = dense_collision_scene(n);
            let floor_row = (bodies.len() - 1) as u32;
            let floor_before = bodies[floor_row as usize];

            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            std::mem::swap(&mut scratch.bodies, &mut bodies);
            scratch.touched.reset(scratch.bodies.len());

            let pool = ThreadPoolBuilder::new().num_threads(4).build();
            pool.install(|_scope| {
                for _ in 0..6 {
                    let mut manifolds = dense_collision_manifolds(&scratch.bodies);
                    // Sentinel contact for body 0 (immovable B, the C1 sentinel path).
                    let mut sm = Manifold::new(BodyIndex(0), SDF_SENTINEL);
                    sm.normal = Vec3::new(0.0, -1.0, 0.0);
                    sm.points[0] = ContactPoint {
                        anchor_a: scratch.bodies[0].position,
                        anchor_b: scratch.bodies[0].position,
                        separation: -0.1,
                        feature_id: 7,
                    };
                    sm.count = 1;
                    manifolds.push(sm);
                    let graph = build_graph(&scratch.bodies, &manifolds);
                    scratch.touched.reset(scratch.bodies.len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
            });

            let floor_after = &scratch.bodies[floor_row as usize];
            prop_assert_eq!(floor_after.position, floor_before.position, "static floor moved (seed {})", seed);
            prop_assert_eq!(floor_after.linear_velocity, Vec3::ZERO, "static floor gained lin vel (seed {})", seed);
            prop_assert_eq!(floor_after.angular_velocity, Vec3::ZERO, "static floor gained ang vel (seed {})", seed);
        });
    }

    /// Gate 7: native MT STRESS — a large dense single-island scene (many colors,
    /// the shared-floor color far above the threshold so real `pool.scope` dispatch
    /// happens) solved for many substeps at a HIGH worker count, repeated several
    /// times. Asserts: no crash / hang / corruption (the run completes), the result
    /// is finite + physically bounded (no NaN/launch from a torn write), and the
    /// REPEATED runs are bit-identical to each other (run-to-run MT determinism).
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_native_mt_stress_is_deterministic() {
        // 2000 dynamic bodies → the shared-floor color holds ~2000 slots (≫ 256), so
        // the `pool.scope` dispatch fans across all 8 workers; 30 steps × the solver's
        // internal substeps is thousands of concurrent color sweeps.
        let n = 2000;
        // Anti-vacuity: the widest color genuinely exceeds the threshold (real
        // dispatch across > 1 worker), and there is > 1 color.
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: widest color {widest} must exceed threshold {MIN_PARALLEL_SLOTS_PER_COLOR}"
        );

        let r1 = run_dense_in_pool(n, 30, true, 8);
        let r2 = run_dense_in_pool(n, 30, true, 8);
        let r3 = run_dense_in_pool(n, 30, true, 8);
        assert_eq!(r1, r2, "MT stress run 1 vs 2 must be bit-identical (run-to-run MT determinism)");
        assert_eq!(r1, r3, "MT stress run 1 vs 3 must be bit-identical (run-to-run MT determinism)");

        // No torn write / corruption: every body bit-pattern is a finite, physically
        // bounded float (a data race in the disjoint-write argument would manifest as
        // a NaN or a launched body well outside the packed line's plausible band).
        for &bits in &r1 {
            let v = f32::from_bits(bits);
            assert!(v.is_finite(), "MT stress produced a non-finite value {v} (possible torn write)");
            assert!(v.abs() < 1.0e6, "MT stress produced an exploded value {v} (possible corruption)");
        }
        // Anti-vacuity: bodies actually moved (not a no-op).
        let resting = snapshot_bits(&{
            let mut s = SolverScratch::with_capacity(n + 1);
            s.bodies = dense_collision_scene(n);
            s
        });
        assert_ne!(r1, resting, "the stress scene must non-vacuously solve (bodies moved)");
    }
}
