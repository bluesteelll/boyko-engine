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
use crate::resources::{BodyState, ConstraintGraph, IslandSleep, PhysicsConfig, SolverScratch};

/// Loads one SoA `[f32; 8]` column into a `__m256` (the O7 cohort kernel's scalar
/// vector load helper). Unaligned — any alignment.
///
/// # Safety
///
/// AVX2-gated (the `cfg` + `target_feature`); the load reads 8 in-bounds `f32`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn load1(col: &[f32; 8]) -> core::arch::x86_64::__m256 {
    use core::arch::x86_64::_mm256_loadu_ps;
    // SAFETY: `col` is an in-bounds `[f32; 8]`; the unaligned load is valid.
    unsafe { _mm256_loadu_ps(col.as_ptr()) }
}

/// Stores a `__m256` into one SoA `[f32; 8]` column (the O7 cohort kernel's scalar
/// vector store helper, used to stage impulse/velocity for the guarded write-back).
///
/// # Safety
///
/// AVX2-gated; the store writes 8 in-bounds `f32`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn store1(out: &mut [f32; 8], reg: core::arch::x86_64::__m256) {
    use core::arch::x86_64::_mm256_storeu_ps;
    // SAFETY: `out` is an in-bounds `[f32; 8]`; the unaligned store is valid.
    unsafe { _mm256_storeu_ps(out.as_mut_ptr(), reg) }
}

/// Loads 3 SoA `[f32; 8]` columns into a `[__m256; 3]` register triple (the O7
/// cohort kernel's vector load helper). Unaligned loads — any alignment.
///
/// # Safety
///
/// AVX2-gated (the `cfg` + `target_feature`); each load reads 8 in-bounds `f32`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn load3(cols: &[[f32; 8]; 3]) -> [core::arch::x86_64::__m256; 3] {
    use core::arch::x86_64::_mm256_loadu_ps;
    // SAFETY: each `cols[c]` is an in-bounds `[f32; 8]`; unaligned loads are valid.
    unsafe {
        [
            _mm256_loadu_ps(cols[0].as_ptr()),
            _mm256_loadu_ps(cols[1].as_ptr()),
            _mm256_loadu_ps(cols[2].as_ptr()),
        ]
    }
}

/// Loads 9 SoA `[f32; 8]` columns (a row-major `Mat3` per lane) into a
/// `[__m256; 9]` register array (the O7 cohort kernel's tensor load helper).
///
/// # Safety
///
/// AVX2-gated; each load reads 8 in-bounds `f32`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn load9(cols: &[[f32; 8]; 9]) -> [core::arch::x86_64::__m256; 9] {
    use core::arch::x86_64::_mm256_loadu_ps;
    // SAFETY: each `cols[c]` is an in-bounds `[f32; 8]`; unaligned loads are valid.
    unsafe {
        [
            _mm256_loadu_ps(cols[0].as_ptr()),
            _mm256_loadu_ps(cols[1].as_ptr()),
            _mm256_loadu_ps(cols[2].as_ptr()),
            _mm256_loadu_ps(cols[3].as_ptr()),
            _mm256_loadu_ps(cols[4].as_ptr()),
            _mm256_loadu_ps(cols[5].as_ptr()),
            _mm256_loadu_ps(cols[6].as_ptr()),
            _mm256_loadu_ps(cols[7].as_ptr()),
            _mm256_loadu_ps(cols[8].as_ptr()),
        ]
    }
}

/// Stores a `[__m256; 3]` register triple into 3 SoA `[f32; 8]` columns (the O7
/// cohort kernel's vector store helper, used at the scatter-once exit).
///
/// # Safety
///
/// AVX2-gated; each store writes 8 in-bounds `f32`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
fn store3(regs: &[core::arch::x86_64::__m256; 3], out: &mut [[f32; 8]; 3]) {
    use core::arch::x86_64::_mm256_storeu_ps;
    // SAFETY: each `out[c]` is an in-bounds `[f32; 8]`; unaligned stores are valid.
    unsafe {
        _mm256_storeu_ps(out[0].as_mut_ptr(), regs[0]);
        _mm256_storeu_ps(out[1].as_mut_ptr(), regs[1]);
        _mm256_storeu_ps(out[2].as_mut_ptr(), regs[2]);
    }
}

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

/// O7 SIMD cohort width: the number of body-disjoint manifold-GROUPS packed into
/// one AVX2 batch (one group per lane = 8 lanes per `__m256`).
///
/// Under the parallel + SIMD path the chunk boundaries are SNAPPED to multiples of
/// this (Decision 7) so every dispatched task solves only full-width cohorts (the
/// last cohort of a COLOR may be partial — the masked kernel handles it). This is a
/// fixed SIMD-width constant, NOT a perf knob: it MUST equal the AVX2 lane count
/// the [`solve_color_avx2`](ColoredSoftStepSolver::solve_color_avx2) kernel uses.
const COHORT: usize = 8;

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
    /// O8 integrate-freeze scratch: the pre-solve `(row, BodyState)` snapshot of each
    /// slept body, captured before the substep loop and restored after — so a slept
    /// island's bodies are NOT integrated (their hot state is frozen) without masking
    /// the per-lane O1 SIMD integrate kernels. Capacity-reused (cleared each step);
    /// empty when sleeping is off (the byte-identical O6/O7 path).
    frozen: Vec<(u32, BodyState)>,
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
            frozen: Vec::with_capacity(bodies),
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
    fn build_columns(
        &mut self,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        bodies: &[BodyState],
        sleep: Option<&IslandSleep>,
    ) {
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
                // O8 sleep skip (SOLVE half): a manifold whose island is FROZEN this
                // frame is NOT pushed — it never enters a color span, so it is never
                // solved. This is the IM-1-safe solve skip (gather stays full; only the
                // solve is elided). `sleep == None` (sleeping off) takes the
                // byte-identical O6/O7 path. The island is the manifold's dynamic side
                // (the same resolution the graph build uses).
                if sleep.is_some_and(|s| Self::manifold_frozen(m, graph, s)) {
                    continue;
                }
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

    /// Whether a manifold belongs to a FROZEN island this frame (plan O8) — the
    /// SOLVE-skip predicate for [`build_columns`](Self::build_columns).
    ///
    /// A manifold's island is its dynamic side's island (`body_a` if dynamic, else
    /// `body_b`) — the SAME resolution [`ConstraintGraph::build`] uses. A manifold to
    /// a static / sentinel surface has its island on the dynamic side, so a frozen body
    /// resting on a floor (the common case) is correctly skipped via that body's
    /// island. A static-static degenerate contact has [`NO_ISLAND`] on both sides and
    /// is never frozen (it is also a no-op to solve).
    ///
    /// "Frozen this frame" is the per-island decision derived in
    /// [`IslandSleep::begin_step`](crate::resources::IslandSleep) (every member dynamic
    /// row latched asleep), NOT a persistent per-island latch — so a slept island that
    /// merged with an awake/new row this frame is NOT frozen, and its manifolds ARE
    /// solved (wake-on-merge).
    ///
    /// [`NO_ISLAND`]: crate::resources::ConstraintGraph::NO_ISLAND
    #[inline]
    fn manifold_frozen(m: &Manifold, graph: &ConstraintGraph, sleep: &IslandSleep) -> bool {
        let isl_a = graph.island_of(m.body_a.0);
        let isl = if isl_a != ConstraintGraph::NO_ISLAND {
            isl_a
        } else {
            graph.island_of(m.body_b.0)
        };
        isl != ConstraintGraph::NO_ISLAND && sleep.is_island_frozen(isl)
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
    ///
    /// O7 dispatch fork over a contiguous group range — the SINGLE site that
    /// chooses the scalar oracle vs the AVX2 cohort kernel.
    ///
    /// - `simd == false` → [`solve_color`](Self::solve_color) over `span` (the
    ///   color/chunk's `[start, end)` slot range): BYTE-IDENTICAL to the committed
    ///   O6 path (the 0%-gate). `g_lo`/`g_hi` are ignored.
    /// - `simd == true` on an AVX2 build → [`solve_color_avx2`](Self::solve_color_avx2)
    ///   over the manifold-GROUP range `[g_lo, g_hi)` as 8-group cohorts: the
    ///   bit-exact WIDTH-ONLY path (Decision 2).
    /// - `simd == true` on a non-AVX2 build / Miri → falls back to
    ///   [`solve_color`](Self::solve_color) over `span` (one fallback = the oracle;
    ///   bit-identical, simpler — the design's "choose the latter").
    ///
    /// `span` and `[g_lo, g_hi)` MUST describe the same contiguous slot region
    /// (`span == (group_start[g_lo], group_start[g_hi])`) so the two paths solve the
    /// identical work — the caller (a whole color, or a parallel cohort-run chunk)
    /// upholds this.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn solve_color_dispatch(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        span: (usize, usize),
        g_lo: usize,
        g_hi: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        simd: bool,
    ) {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            if simd {
                // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees
                //   the executing CPU supports every AVX2 intrinsic the kernel uses
                //   (a non-AVX2 host cannot reach this branch — the `cfg` excludes it).
                //   The kernel documents its per-load bounds + disjoint-write invariants.
                unsafe {
                    Self::solve_color_avx2(
                        cols,
                        bodies_eff,
                        span,
                        g_lo,
                        g_hi,
                        bias_rate,
                        mass_coeff,
                        impulse_coeff,
                        bias_active,
                    );
                }
                return;
            }
        }
        // Flag off / non-AVX2 build / Miri: the byte-identical scalar oracle over
        // the whole span (the 0%-gate AND the SIMD non-AVX2 fallback).
        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
        let _ = (g_lo, g_hi, simd);
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        let _ = (g_lo, g_hi);
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

    /// AVX2 8-wide colored solve (Phase O7) — lane = one whole manifold-GROUP, a
    /// batch (cohort) = 8 body-disjoint groups of one color, BIT-IDENTICAL to
    /// [`solve_color`](Self::solve_color) over the same `[g_lo, g_hi)` groups.
    ///
    /// Solves the manifold-GROUP range `[g_lo, g_hi)` (groups indexing
    /// `cols.group_start`) as cohorts of up to 8 groups. Per cohort: GATHER-ONCE
    /// the 8 groups' two body pairs (inv_mass, inv_inertia, velocity) into stack
    /// SoA; a RANK loop `r = 0..max_width` where each ACTIVE lane (`width > r`)
    /// solves its group's point-`r` normal→friction with the A/B velocity
    /// REGISTER-CARRIED across ranks (so point `p` sees `p-1`'s update — the
    /// intra-group Gauss-Seidel coupling) and the per-rank impulse slots
    /// read-modify-written; SCATTER-ONCE the 16 body rows at cohort exit.
    ///
    /// # Bit-exactness (Decision 2, the PINNED invariant)
    ///
    /// Each lane runs its group's EXACT scalar `solve_color` op sequence (the same
    /// IEEE round-to-nearest `mul`/`add`/`sub`/`div`/`sqrt`, NO FMA, NO
    /// `rsqrt`/`rcp` — the `simd::*_x8` helpers mirror `contact.rs`/`math.rs`
    /// op-for-op). The 8 lanes write DISJOINT body rows + DISJOINT impulse slots
    /// (the O4 coloring invariant: distinct groups of a color touch disjoint
    /// dynamic bodies), so the parallel 8-lane evaluation equals solving the 8
    /// groups sequentially, which equals the scalar single-threaded colored solve,
    /// bit-for-bit. Masked exhausted / partial-cohort / static lanes compute
    /// garbage that every `blendv` / guarded scalar scatter discards (FP exceptions
    /// masked ⇒ Inf/NaN are inert DATA, never traps — Decision 3/4).
    ///
    /// # Safety
    ///
    /// The caller must guarantee AVX2 is available (the `cfg` + `target_feature`
    /// gate — a non-AVX2 host cannot link this path). `[g_lo, g_hi)` must be a valid
    /// contiguous group range (`g_hi <= cols.group_start.len() - 1`) whose groups'
    /// dynamic bodies are pairwise-disjoint (the O4 coloring invariant — upheld
    /// because the range is a color's groups, or a cohort-run within one color). All
    /// gathered body indices are `< bodies_eff.len()` (the build invariant).
    ///
    /// `span == [chunk_start, chunk_end)` is the worker's OWN slot span — the union
    /// of the `[g_lo, g_hi)` groups' contiguous slot runs (`chunk_start ==
    /// group_start[g_lo]`, `chunk_end == group_start[g_hi]`). C1 (the cross-worker
    /// READ-race fix): every column slot the kernel READS or WRITES MUST lie within
    /// `span`. The per-rank gather is therefore clamped to the LANE'S OWN GROUP last
    /// slot (never the global `cols.len()`), and an ABSENT lane (partial trailing
    /// cohort) gathers `chunk_start` — both provably in `span`. A foreign slot
    /// (`>= chunk_end`) belongs to the next concurrently-running worker, which writes
    /// it every rank, so reading it would be an unsynchronized cross-thread access
    /// (a data race) even though the value is `blendv`-discarded. Every gathered slot
    /// staying in `span` removes that race. The per-cohort scatter writes only own
    /// in-span slots (`g_base + r` for active lanes) and ≤ 16 distinct dynamic body
    /// rows; statics/sentinels are never written (the movable-blend guard), so a
    /// SHARED static row across cohorts is read-only.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[target_feature(enable = "avx2")]
    #[allow(clippy::too_many_arguments)]
    fn solve_color_avx2(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        span: (usize, usize),
        g_lo: usize,
        g_hi: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        use core::arch::x86_64::{
            _mm256_add_ps, _mm256_and_ps, _mm256_blendv_ps, _mm256_cmp_ps, _mm256_div_ps,
            _mm256_max_ps, _mm256_mul_ps, _mm256_set1_ps, _mm256_sqrt_ps, _mm256_sub_ps,
            _CMP_GT_OQ, _CMP_NEQ_OQ,
        };
        use crate::solver::simd::{apply_impulse_blend_x8, dot8, effective_mass_x8, pointvel_x8};

        const W: usize = 8;
        if g_lo >= g_hi {
            return;
        }
        debug_assert!(g_hi < cols.group_start.len(), "group range within the CSR");
        // C1: the worker's OWN slot span `[chunk_start, chunk_end)` — every gathered
        // slot must lie inside it (no foreign / cross-worker read). It is the union of
        // the `[g_lo, g_hi)` groups' contiguous slot runs.
        let (chunk_start, chunk_end) = span;
        debug_assert!(
            chunk_start == cols.group_start[g_lo] as usize
                && chunk_end == cols.group_start[g_hi] as usize
                && chunk_start < chunk_end,
            "invariant: span is exactly the [g_lo, g_hi) groups' slot run"
        );

        // ── Per-cohort stack scratch (zero heap; capacity-retained per call) ────
        // Per-lane (group) metadata, gathered once at cohort entry.
        let mut g_base = [0usize; W]; // slot base of each lane's group
        let mut g_last = [0usize; W]; // last slot the lane may gather (its own group's)
        let mut g_width = [0u32; W]; // each lane's group width (0 = absent lane)
        let mut ia = [0usize; W]; // body-A row index
        let mut ib = [0usize; W]; // body-B row index (== ia for a sentinel)
        let mut sent = [false; W]; // b_is_sentinel per lane
        // Per-lane A/B body-state gather (constants within the call): inv_mass,
        // inv_inertia (9), linear (3), angular (3).
        let mut a_invm_s = [0.0f32; W];
        let mut b_invm_s = [0.0f32; W];
        let mut a_ii_s = [[0.0f32; W]; 9];
        let mut b_ii_s = [[0.0f32; W]; 9];
        let mut a_lin_s = [[0.0f32; W]; 3];
        let mut a_ang_s = [[0.0f32; W]; 3];
        let mut b_lin_s = [[0.0f32; W]; 3];
        let mut b_ang_s = [[0.0f32; W]; 3];
        // a_static[lane] = 1.0 if A is NOT movable (inv_mass == 0). Always-dynamic
        // by manifold convention, but kept for scalar-exactness (the scalar guards
        // both sides — Decision 6 / residual-risk 4).
        let mut a_static_s = [0.0f32; W];
        // Per-rank geometry + impulse staging (re-gathered each rank, clamped).
        let mut ra_s = [[0.0f32; W]; 3];
        let mut rb_s = [[0.0f32; W]; 3];
        let mut n_s = [[0.0f32; W]; 3];
        let mut t1_s = [[0.0f32; W]; 3];
        let mut t2_s = [[0.0f32; W]; 3];
        let mut sep_s = [0.0f32; W];
        let mut fric_s = [0.0f32; W];
        let mut ni_s = [0.0f32; W];
        let mut ti1_s = [0.0f32; W];
        let mut ti2_s = [0.0f32; W];
        let mut active_s = [0.0f32; W];
        // Velocity scatter staging (written only at cohort exit).
        let mut out_a_lin = [[0.0f32; W]; 3];
        let mut out_a_ang = [[0.0f32; W]; 3];
        let mut out_b_lin = [[0.0f32; W]; 3];
        let mut out_b_ang = [[0.0f32; W]; 3];

        // Whole-call constants (bias_active hoisted OUTSIDE the loops for I-cache
        // compactness — it is loop-invariant, exactly as the scalar `if`).
        // SAFETY (target_feature): every intrinsic below is AVX2; this fn is
        //   `#[target_feature(enable = "avx2")]`-gated and only reached on an AVX2
        //   build (the `cfg` gate), so the CPU supports them.
        let zero = _mm256_set1_ps(0.0);
        let neg_one = _mm256_set1_ps(-1.0);
        let bias_rate_v = _mm256_set1_ps(bias_rate);
        let neg_max_bias = _mm256_set1_ps(-MAX_BIAS_VELOCITY);
        let mass_coeff_v = _mm256_set1_ps(mass_coeff);
        let impulse_coeff_v = _mm256_set1_ps(impulse_coeff);

        let mut cohort_lo = g_lo;
        while cohort_lo < g_hi {
            let cohort_hi = (cohort_lo + W).min(g_hi);
            let nlanes = cohort_hi - cohort_lo;
            debug_assert!((1..=W).contains(&nlanes), "cohort has 1..=8 lanes");

            // ── Gather-ONCE (cohort entry) ──────────────────────────────────────
            let mut max_width = 0u32;
            for lane in 0..W {
                if lane < nlanes {
                    let g = cohort_lo + lane;
                    let base = cols.group_start[g] as usize;
                    let width = cols.group_start[g + 1] as usize - base;
                    debug_assert!(width >= 1, "every built group has >= 1 point");
                    let s = base; // first slot of the group (the body pair is shared)
                    let lane_ia = cols.body_a[s] as usize;
                    let b_sent = cols.b_is_sentinel[s];
                    let lane_ib = cols.body_b[s] as usize;
                    // O3: B is only indexed when it is a real body (a sentinel reads
                    // IMMOVABLE_AT_REST, never `bodies_eff[lane_ib]`), mirroring the
                    // scalar oracle's conditional index — so only assert it then.
                    debug_assert!(lane_ia < bodies_eff.len());
                    debug_assert!(b_sent || lane_ib < bodies_eff.len());

                    g_base[lane] = base;
                    // Own-group last slot: the lane re-reads its own (in-span) slot
                    // once exhausted (rank r >= width), never a foreign worker's.
                    g_last[lane] = base + width - 1;
                    g_width[lane] = width as u32;
                    ia[lane] = lane_ia;
                    ib[lane] = lane_ib;
                    sent[lane] = b_sent;
                    max_width = max_width.max(width as u32);

                    // Body A (always the dynamic side by convention).
                    let ba = &bodies_eff[lane_ia];
                    a_invm_s[lane] = ba.inv_mass;
                    a_static_s[lane] = if is_dynamic_row(ba.inv_mass) { 0.0 } else { 1.0 };
                    Self::stage_body_state(ba, lane, &mut a_ii_s, &mut a_lin_s, &mut a_ang_s);

                    // Body B: a sentinel gathers IMMOVABLE_AT_REST (inv_mass 0,
                    // inv_inertia ZERO, velocity ZERO) so its lane is a value no-op
                    // and is never scattered (the sentinel guard at exit).
                    let bb = if b_sent { &IMMOVABLE_AT_REST } else { &bodies_eff[lane_ib] };
                    b_invm_s[lane] = bb.inv_mass;
                    Self::stage_body_state(bb, lane, &mut b_ii_s, &mut b_lin_s, &mut b_ang_s);
                } else {
                    // Absent lane (partial trailing cohort): permanently inactive
                    // (`width == 0` ⇒ `active = false` at every rank), every write
                    // discarded. Zero its gathered state so the masked arithmetic is
                    // harmless (no NaN propagation into a live lane — lanes are
                    // independent regardless).
                    //
                    // C1: an absent lane (no group in this cohort) gathers a
                    // SELF-OWNED, in-span slot — `chunk_start`, the worker's first
                    // slot — at every rank (`s = min(chunk_start + r, chunk_start) ==
                    // chunk_start`), NEVER slot 0 / the global len. Its value is
                    // discarded (width 0 ⇒ inactive at every rank), but the READ must
                    // stay inside `[chunk_start, chunk_end)` so it cannot touch a
                    // concurrently-running worker's slots.
                    g_base[lane] = chunk_start;
                    g_last[lane] = chunk_start;
                    g_width[lane] = 0;
                    ia[lane] = 0;
                    ib[lane] = 0;
                    sent[lane] = true; // never-scatter the absent B lane
                    a_invm_s[lane] = 0.0;
                    b_invm_s[lane] = 0.0;
                    a_static_s[lane] = 1.0;
                    for c in 0..9 {
                        a_ii_s[c][lane] = 0.0;
                        b_ii_s[c][lane] = 0.0;
                    }
                    for c in 0..3 {
                        a_lin_s[c][lane] = 0.0;
                        a_ang_s[c][lane] = 0.0;
                        b_lin_s[c][lane] = 0.0;
                        b_ang_s[c][lane] = 0.0;
                    }
                }
            }

            // Load the gather-once constants into registers (each `load1`/`load3`/
            // `load9` reads 8 `f32` from an in-bounds `[f32; 8]` stack buffer).
            let a_invm = load1(&a_invm_s);
            let b_invm = load1(&b_invm_s);
            let a_static = load1(&a_static_s);
            let a_ii = load9(&a_ii_s);
            let b_ii = load9(&b_ii_s);
            // not-sentinel mask as 1.0/0.0 → register (1.0 where B is a REAL body).
            let mut not_sent_f = [0.0f32; W];
            for lane in 0..W {
                not_sent_f[lane] = if sent[lane] { 0.0 } else { 1.0 };
            }
            let not_sent = _mm256_cmp_ps::<_CMP_NEQ_OQ>(load1(&not_sent_f), zero);

            // A/B velocity: REGISTER-CARRIED from here across the whole rank loop.
            let mut a_lin = load3(&a_lin_s);
            let mut a_ang = load3(&a_ang_s);
            let mut b_lin = load3(&b_lin_s);
            let mut b_ang = load3(&b_ang_s);

            // movable masks (constant within the cohort): A movable = inv_mass != 0;
            // B movable = !sentinel AND inv_mass != 0 (Decision 6 guard table).
            let a_movable = _mm256_cmp_ps::<_CMP_NEQ_OQ>(a_static, _mm256_set1_ps(1.0));
            let b_neq0 = _mm256_cmp_ps::<_CMP_NEQ_OQ>(b_invm, zero);
            let b_movable = _mm256_and_ps(not_sent, b_neq0);

            // ── Rank loop (register-carry velocity) ─────────────────────────────
            for r in 0..max_width as usize {
                // active = (g_width[lane] > r) as 1.0/0.0 → mask.
                for lane in 0..W {
                    active_s[lane] = if (g_width[lane] as usize) > r { 1.0 } else { 0.0 };
                }
                let active = _mm256_cmp_ps::<_CMP_NEQ_OQ>(load1(&active_s), zero);

                // Per-rank gather (C1 — own-span bound): slot `s = min(g_base + r,
                // g_last)` where `g_last` is the LANE'S OWN GROUP last slot (an absent
                // lane's `g_base == g_last == chunk_start`). An ACTIVE lane (rank r <
                // width) reads its exact rank-r slot; an EXHAUSTED lane re-reads its
                // own group's last (in-span) slot; an ABSENT lane reads `chunk_start`.
                // Every `s` therefore lies in `[chunk_start, chunk_end)` — never a
                // concurrently-running worker's slot (the cross-worker READ-race fix).
                for lane in 0..W {
                    let s = (g_base[lane] + r).min(g_last[lane]);
                    debug_assert!(
                        (chunk_start..chunk_end).contains(&s),
                        "C1: every gathered slot must lie within the worker's own span \
                         [{chunk_start}, {chunk_end}); got s={s} (lane={lane}, rank={r})"
                    );
                    ra_s[0][lane] = cols.ra_x[s];
                    ra_s[1][lane] = cols.ra_y[s];
                    ra_s[2][lane] = cols.ra_z[s];
                    rb_s[0][lane] = cols.rb_x[s];
                    rb_s[1][lane] = cols.rb_y[s];
                    rb_s[2][lane] = cols.rb_z[s];
                    n_s[0][lane] = cols.normal_x[s];
                    n_s[1][lane] = cols.normal_y[s];
                    n_s[2][lane] = cols.normal_z[s];
                    t1_s[0][lane] = cols.tangent1_x[s];
                    t1_s[1][lane] = cols.tangent1_y[s];
                    t1_s[2][lane] = cols.tangent1_z[s];
                    t2_s[0][lane] = cols.tangent2_x[s];
                    t2_s[1][lane] = cols.tangent2_y[s];
                    t2_s[2][lane] = cols.tangent2_z[s];
                    sep_s[lane] = cols.separation[s];
                    fric_s[lane] = cols.friction[s];
                    ni_s[lane] = cols.normal_impulse[s];
                    ti1_s[lane] = cols.tangent1_impulse[s];
                    ti2_s[lane] = cols.tangent2_impulse[s];
                }
                let ra = load3(&ra_s);
                let rb = load3(&rb_s);
                let n = load3(&n_s);
                let t1 = load3(&t1_s);
                let t2 = load3(&t2_s);
                let sep = load1(&sep_s);
                let fric = load1(&fric_s);
                let mut ni = load1(&ni_s);
                let mut ti1 = load1(&ti1_s);
                let mut ti2 = load1(&ti2_s);

                // Velocity-write masks (active AND movable), per Decision 6.
                let mask_a = _mm256_and_ps(active, a_movable);
                let mask_b = _mm256_and_ps(active, b_movable);

                // ── NORMAL solve (op-for-op vs the scalar, NO FMA) ──────────────
                let m_eff = effective_mass_x8(n, ra, rb, a_invm, a_ii, b_invm, b_ii);
                // vn = (pointvel(B,rb) - pointvel(A,ra)) · n.
                let pvb = pointvel_x8(b_lin, b_ang, rb);
                let pva = pointvel_x8(a_lin, a_ang, ra);
                let dvn = [
                    _mm256_sub_ps(pvb[0], pva[0]),
                    _mm256_sub_ps(pvb[1], pva[1]),
                    _mm256_sub_ps(pvb[2], pva[2]),
                ];
                let vn = dot8(dvn[0], dvn[1], dvn[2], n[0], n[1], n[2]);
                let lambda_n = ni;
                // bias_active hoisted: the whole d_lambda branch is a Rust `if`, not
                // a per-lane blend (matches the scalar loop-invariant `if`).
                let d_lambda = if bias_active {
                    // bias = max(bias_rate * sep, -MAX_BIAS_VELOCITY).
                    let bias = _mm256_max_ps(_mm256_mul_ps(bias_rate_v, sep), neg_max_bias);
                    // -massCoeff*mEff*(vn+bias) - impulseCoeff*lambda_n.
                    let vnb = _mm256_add_ps(vn, bias);
                    let neg_mc_meff = _mm256_mul_ps(_mm256_mul_ps(neg_one, mass_coeff_v), m_eff);
                    _mm256_sub_ps(
                        _mm256_mul_ps(neg_mc_meff, vnb),
                        _mm256_mul_ps(impulse_coeff_v, lambda_n),
                    )
                } else {
                    // -mEff * vn.
                    _mm256_mul_ps(_mm256_mul_ps(neg_one, m_eff), vn)
                };
                // new_lambda = max(lambda_n + d_lambda, 0); applied = new - old.
                let new_lambda = _mm256_max_ps(_mm256_add_ps(lambda_n, d_lambda), zero);
                let applied_n = _mm256_sub_ps(new_lambda, lambda_n);
                ni = new_lambda;
                // impulse = n * applied_n (vec3).
                let imp_n = [
                    _mm256_mul_ps(n[0], applied_n),
                    _mm256_mul_ps(n[1], applied_n),
                    _mm256_mul_ps(n[2], applied_n),
                ];
                // A gets -impulse, B gets +impulse (gated active AND movable).
                let neg_imp_n = [
                    _mm256_mul_ps(imp_n[0], neg_one),
                    _mm256_mul_ps(imp_n[1], neg_one),
                    _mm256_mul_ps(imp_n[2], neg_one),
                ];
                let (na_lin, na_ang) =
                    apply_impulse_blend_x8(a_lin, a_ang, ra, neg_imp_n, a_invm, a_ii, mask_a);
                a_lin = na_lin;
                a_ang = na_ang;
                let (nb_lin, nb_ang) =
                    apply_impulse_blend_x8(b_lin, b_ang, rb, imp_n, b_invm, b_ii, mask_b);
                b_lin = nb_lin;
                b_ang = nb_ang;

                // ── FRICTION solve (2-DOF coupled cone, Decision 3) ─────────────
                // max_friction = friction * ni (the JUST-written new normal impulse).
                let max_fric = _mm256_mul_ps(fric, ni);
                let m_eff_t1 = effective_mass_x8(t1, ra, rb, a_invm, a_ii, b_invm, b_ii);
                let m_eff_t2 = effective_mass_x8(t2, ra, rb, a_invm, a_ii, b_invm, b_ii);
                // RE-READ post-normal velocity (the scalar re-reads after the normal
                // apply too).
                let pvb2 = pointvel_x8(b_lin, b_ang, rb);
                let pva2 = pointvel_x8(a_lin, a_ang, ra);
                let dvt = [
                    _mm256_sub_ps(pvb2[0], pva2[0]),
                    _mm256_sub_ps(pvb2[1], pva2[1]),
                    _mm256_sub_ps(pvb2[2], pva2[2]),
                ];
                let vt1 = dot8(dvt[0], dvt[1], dvt[2], t1[0], t1[1], t1[2]);
                let vt2 = dot8(dvt[0], dvt[1], dvt[2], t2[0], t2[1], t2[2]);
                // new_t = ti - m_eff_t * vt (separate mul then sub — matches scalar).
                let mut new_t1 = _mm256_sub_ps(ti1, _mm256_mul_ps(m_eff_t1, vt1));
                let mut new_t2 = _mm256_sub_ps(ti2, _mm256_mul_ps(m_eff_t2, vt2));
                // The cone: len_sq = t1*t1 + t2*t2 (left-to-right); two-predicate mask
                // (len_sq > mf²) AND (len_sq > 0); UNCONDITIONAL scale = mf/sqrt(len_sq);
                // blendv-discard on unclamped lanes (Inf/NaN bit-irrelevant there).
                let len_sq = _mm256_add_ps(
                    _mm256_mul_ps(new_t1, new_t1),
                    _mm256_mul_ps(new_t2, new_t2),
                );
                let mf2 = _mm256_mul_ps(max_fric, max_fric);
                let cone = _mm256_and_ps(
                    _mm256_cmp_ps::<_CMP_GT_OQ>(len_sq, mf2),
                    _mm256_cmp_ps::<_CMP_GT_OQ>(len_sq, zero),
                );
                let scale = _mm256_div_ps(max_fric, _mm256_sqrt_ps(len_sq));
                new_t1 = _mm256_blendv_ps(new_t1, _mm256_mul_ps(new_t1, scale), cone);
                new_t2 = _mm256_blendv_ps(new_t2, _mm256_mul_ps(new_t2, scale), cone);
                let applied_t1 = _mm256_sub_ps(new_t1, ti1);
                let applied_t2 = _mm256_sub_ps(new_t2, ti2);
                ti1 = new_t1;
                ti2 = new_t2;
                // impulse = t1*applied_t1 + t2*applied_t2 (vec3; per-component
                // separate mul then add — matches `t1 * a1 + t2 * a2`).
                let imp_t = [
                    _mm256_add_ps(
                        _mm256_mul_ps(t1[0], applied_t1),
                        _mm256_mul_ps(t2[0], applied_t2),
                    ),
                    _mm256_add_ps(
                        _mm256_mul_ps(t1[1], applied_t1),
                        _mm256_mul_ps(t2[1], applied_t2),
                    ),
                    _mm256_add_ps(
                        _mm256_mul_ps(t1[2], applied_t1),
                        _mm256_mul_ps(t2[2], applied_t2),
                    ),
                ];
                let neg_imp_t = [
                    _mm256_mul_ps(imp_t[0], neg_one),
                    _mm256_mul_ps(imp_t[1], neg_one),
                    _mm256_mul_ps(imp_t[2], neg_one),
                ];
                let (na_lin2, na_ang2) =
                    apply_impulse_blend_x8(a_lin, a_ang, ra, neg_imp_t, a_invm, a_ii, mask_a);
                a_lin = na_lin2;
                a_ang = na_ang2;
                let (nb_lin2, nb_ang2) =
                    apply_impulse_blend_x8(b_lin, b_ang, rb, imp_t, b_invm, b_ii, mask_b);
                b_lin = nb_lin2;
                b_ang = nb_ang2;

                // ── Per-rank impulse SCATTER (gated by `active` only — matches the
                //    scalar UNCONDITIONAL impulse write, the velocity-vs-impulse
                //    asymmetry of Decision 6: impulse slots written for every LIVE
                //    point regardless of body movability). Via stack staging + a
                //    guarded scalar write so an inactive/clamped lane is a true
                //    NO-OP (no slot collision — Decision 4 resolution). ──────────
                store1(&mut ni_s, ni);
                store1(&mut ti1_s, ti1);
                store1(&mut ti2_s, ti2);
                for lane in 0..nlanes {
                    if (g_width[lane] as usize) > r {
                        let s = g_base[lane] + r; // active ⇒ exact in-range slot
                        cols.normal_impulse[s] = ni_s[lane];
                        cols.tangent1_impulse[s] = ti1_s[lane];
                        cols.tangent2_impulse[s] = ti2_s[lane];
                    }
                }
            }

            // ── Scatter-ONCE (cohort exit): A/B velocity registers → body rows ───
            // SAFETY: stores write 8 `f32` into in-bounds `[f32; 8]` stack buffers.
            store3(&a_lin, &mut out_a_lin);
            store3(&a_ang, &mut out_a_ang);
            store3(&b_lin, &mut out_b_lin);
            store3(&b_ang, &mut out_b_ang);
            // A is written for every lane (always-dynamic by convention; a STATIC-A
            // lane was masked off in `mask_a`, so its registers held the unchanged
            // gathered velocity — writing it back is a no-op of the gathered value,
            // but to be safe against a shared static A across cohorts we skip it).
            // SOUNDNESS: writing only MOVABLE rows keeps cohorts disjoint (a shared
            // static row is never written, matching the scalar `*_movable` guard).
            for lane in 0..nlanes {
                if a_static_s[lane] == 0.0 {
                    let b = &mut bodies_eff[ia[lane]];
                    b.linear_velocity = Vec3::new(out_a_lin[0][lane], out_a_lin[1][lane], out_a_lin[2][lane]);
                    b.angular_velocity = Vec3::new(out_a_ang[0][lane], out_a_ang[1][lane], out_a_ang[2][lane]);
                }
                // B written only when it is a real, movable dynamic body (not a
                // sentinel, not a static) — the disjoint-write soundness anchor.
                if !sent[lane] && is_dynamic_row(b_invm_s[lane]) {
                    let b = &mut bodies_eff[ib[lane]];
                    b.linear_velocity = Vec3::new(out_b_lin[0][lane], out_b_lin[1][lane], out_b_lin[2][lane]);
                    b.angular_velocity = Vec3::new(out_b_ang[0][lane], out_b_ang[1][lane], out_b_ang[2][lane]);
                }
            }

            cohort_lo = cohort_hi;
        }
    }

    /// Stages one body's `inv_inertia` (9 SoA columns), `linear_velocity` (3), and
    /// `angular_velocity` (3) into the cohort gather buffers at `lane` (the
    /// scalar-side gather half of [`solve_color_avx2`]). Pure scalar marshaling, no
    /// intrinsics.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[inline]
    fn stage_body_state(
        b: &BodyEffective,
        lane: usize,
        ii: &mut [[f32; 8]; 9],
        lin: &mut [[f32; 8]; 3],
        ang: &mut [[f32; 8]; 3],
    ) {
        let m = &b.inv_inertia.rows;
        ii[0][lane] = m[0].x;
        ii[1][lane] = m[0].y;
        ii[2][lane] = m[0].z;
        ii[3][lane] = m[1].x;
        ii[4][lane] = m[1].y;
        ii[5][lane] = m[1].z;
        ii[6][lane] = m[2].x;
        ii[7][lane] = m[2].y;
        ii[8][lane] = m[2].z;
        lin[0][lane] = b.linear_velocity.x;
        lin[1][lane] = b.linear_velocity.y;
        lin[2][lane] = b.linear_velocity.z;
        ang[0][lane] = b.angular_velocity.x;
        ang[1][lane] = b.angular_velocity.y;
        ang[2][lane] = b.angular_velocity.z;
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
    #[allow(clippy::too_many_arguments)]
    fn solve_all_colors(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        parallel: bool,
        simd: bool,
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
                    simd,
                );
            } else {
                let start = cols.color_offsets[c] as usize;
                let end = cols.color_offsets[c + 1] as usize;
                // O7 dispatch fork (the 0%-gate): `simd == false` runs the byte-
                // identical scalar oracle `solve_color`; `simd == true` runs the
                // AVX2 cohort kernel over the color's manifold-GROUP range (the
                // bit-exact width-only path). The non-parallel SIMD path solves the
                // WHOLE color's groups as cohorts on the calling thread.
                let g_lo = cols.color_group_start[c] as usize;
                let g_hi = cols.color_group_start[c + 1] as usize;
                Self::solve_color_dispatch(
                    cols,
                    bodies_eff,
                    (start, end),
                    g_lo,
                    g_hi,
                    bias_rate,
                    mass_coeff,
                    impulse_coeff,
                    bias_active,
                    simd,
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
    ///
    /// # O7 cohort-snapping (Decision 7)
    ///
    /// When `simd`, the chunk boundaries are SNAPPED to cohort (8-group) boundaries
    /// so every task solves only full-width-8 cohorts (the last cohort of the COLOR
    /// may be partial — handled by the masked kernel; the last cohort of a TASK is
    /// always whole). Each task routes through
    /// [`solve_color_dispatch`](Self::solve_color_dispatch) with its chunk's group
    /// range, so a worker runs [`solve_color_avx2`](Self::solve_color_avx2) over its
    /// cohorts. Cohorts within a color are body-disjoint (each is 8 disjoint groups;
    /// distinct cohorts are pairwise disjoint), so cross-worker disjointness — and
    /// thus the {1, N}×{simd} bit-identity — is unchanged from O6.
    #[allow(clippy::too_many_arguments)]
    fn solve_color_parallel(
        cols: &mut ContactColumns,
        bodies_eff: &mut [BodyEffective],
        color: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        simd: bool,
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
            // Inline on the calling thread, routed through the O7 dispatch fork:
            // `simd` runs `solve_color_avx2` over the whole color's groups (as
            // cohorts), else the scalar oracle over the span — both bit-identical to
            // the parallel split.
            Self::solve_color_dispatch(
                cols,
                bodies_eff,
                span,
                g_lo,
                g_hi,
                bias_rate,
                mass_coeff,
                impulse_coeff,
                bias_active,
                simd,
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
                    //
                    // O7 cohort-snapping (Decision 7): when `simd`, advance in steps
                    // of `COHORT` (8 groups) and clamp at `g_hi`, so every chunk
                    // boundary falls on a cohort boundary (a multiple-of-8 group
                    // offset from `g_lo`) — every task thus solves only full-width-8
                    // cohorts (plus the color's single possibly-partial trailing
                    // cohort, on whichever task owns the last group). The step keeps
                    // the chunk a contiguous group range = a contiguous slot span,
                    // the shape `solve_color_avx2` consumes. Bit-identity is
                    // chunk-shape-independent (cohorts are pairwise body-disjoint), so
                    // snapping is a pure perf knob.
                    let step = if simd { COHORT } else { 1 };
                    let mut chunk_g_hi = (chunk_g_lo + step).min(g_hi);
                    while chunk_g_hi < g_hi {
                        // SAFETY: `chunk_g_hi <= g_hi`, a valid `group_start` index.
                        let so_far = unsafe { ptrs.group_start_at(chunk_g_hi) } - chunk_start;
                        if so_far >= target {
                            break;
                        }
                        chunk_g_hi = (chunk_g_hi + step).min(g_hi);
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
                    debug_assert!(
                        !simd || (chunk_g_lo - g_lo).is_multiple_of(COHORT),
                        "invariant: a SIMD chunk's lo boundary is a cohort (8-group) boundary"
                    );

                    let task_g_lo = chunk_g_lo;
                    let task_g_hi = chunk_g_hi;
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
                        //   O7: when `simd`, the worker runs `solve_color_avx2` over
                        //   its cohort range `[task_g_lo, task_g_hi)` — a cohort packs
                        //   8 disjoint groups, so distinct workers' cohort-runs still
                        //   touch DISJOINT dynamic rows + DISJOINT impulse slots
                        //   (union of disjoint groups), and statics/sentinels remain
                        //   never-written (the kernel's movable-blend guard). The
                        //   disjointness argument is thus UNCHANGED from the scalar
                        //   chunk dispatch above.
                        let cols_mut = unsafe { ptrs.columns() };
                        let bodies_mut = unsafe { ptrs.bodies() };
                        Self::solve_color_dispatch(
                            cols_mut,
                            bodies_mut,
                            (chunk_start, chunk_end),
                            task_g_lo,
                            task_g_hi,
                            bias_rate,
                            mass_coeff,
                            impulse_coeff,
                            bias_active,
                            simd,
                        );
                    });

                    chunk_g_lo = chunk_g_hi;
                }
            });
        });

        // PAR-fallback: no pool attached → run the color single-threaded, routed
        // through the O7 dispatch fork so `simd` still widens (over the whole
        // color's groups as cohorts) and `!simd` is BYTE-IDENTICAL to O5 (the same
        // `solve_color` over the whole color span).
        if dispatched.is_none() {
            Self::solve_color_dispatch(
                cols,
                bodies_eff,
                span,
                g_lo,
                g_hi,
                bias_rate,
                mass_coeff,
                impulse_coeff,
                bias_active,
                simd,
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

    /// Write-back variant for O8 sleeping: identical to [`write_back`](Self::write_back)
    /// but SKIPS slept rows (an `awake_rows` bit that is clear), so a frozen body is
    /// never flagged `touched` and [`physics_apply`](crate::systems::physics_apply)
    /// leaves its live component byte-untouched.
    ///
    /// IM-1: this changes only WHICH rows are flagged touched (slept rows are not) —
    /// it does NOT change the row count gather/apply walk, so the desync assert is
    /// unaffected.
    fn write_back_awake(&self, scratch: &mut SolverScratch, sleep: &IslandSleep) {
        for row in 0..self.bodies.len() {
            if !sleep.is_row_awake(row) {
                continue;
            }
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
        // Sleeping OFF (or no resource): the byte-identical O6/O7 colored path.
        self.solve_colored_inner(config, manifolds, graph, scratch, None);
    }

    /// The colored solve with O8 sleeping (plan O8 / Decision 5) — the entry the
    /// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage calls
    /// when `PhysicsConfig::sleeping` is on.
    ///
    /// Identical to [`solve_colored`](Self::solve_colored) but threads the
    /// [`IslandSleep`] state: slept islands skip ONLY their SOLVE + INTEGRATE work
    /// (the gather is untouched — IM-1), and the energy / debounce / sleep transition
    /// is advanced after the solve for next frame.
    pub fn solve_colored_sleeping(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
        sleep: &mut IslandSleep,
    ) {
        self.solve_colored_inner(config, manifolds, graph, scratch, Some(sleep));
    }

    /// Shared body of the colored solve — `sleep == None` is the byte-identical
    /// O6/O7 path (the 0%-gate), `sleep == Some(_)` adds the O8 solve+integrate skip
    /// for slept islands (gather stays full — IM-1).
    fn solve_colored_inner(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
        mut sleep: Option<&mut IslandSleep>,
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

        // O8 phase 1 (BEFORE the solve): apply the wake conditions and build the
        // body→awake mask from last frame's sleep flags. This decides which islands
        // skip the solve + integrate THIS frame. A read-only borrow of the resource
        // for the duration of the build/solve (`asleep` / `awake_rows` are read);
        // the post-solve `end_step` reborrows mutably.
        let n_rows = scratch.bodies.len();
        let sleeping_active = sleep.is_some();
        if let Some(sleep) = sleep.as_mut() {
            sleep.begin_step(graph, n_rows);
        }
        // An immutable view used by `build_columns` (SOLVE skip) + the integrate
        // freeze; `None` when sleeping is off so the path is byte-identical.
        let sleep_view: Option<&IslandSleep> = sleep.as_deref();

        self.build_bodies(&scratch.bodies);
        self.build_columns(manifolds, graph, &scratch.bodies, sleep_view);

        // O8 integrate-freeze (INTEGRATE half): capture the pre-solve hot state of
        // every slept-island body so the per-substep integrate (which streams the
        // WHOLE array — the O1 SIMD kernels are NOT per-lane masked) can be UNDONE for
        // slept rows after the loop. This freezes a slept body's position / rotation /
        // velocity without touching the audited integrate kernels. `frozen` is
        // capacity-reused (empty when sleeping is off — the byte-identical path).
        self.frozen.clear();
        if let Some(sleep) = sleep_view {
            for (row, b) in scratch.bodies.iter().enumerate() {
                if b.inv_mass == 0.0 {
                    // Static rows are no-ops to the integrate kernels (the
                    // `inv_mass != 0` guard) — no need to snapshot them.
                    continue;
                }
                if !sleep.is_row_awake(row) {
                    self.frozen.push((row as u32, *b));
                }
            }
        }

        let soft = SoftCoefficients::new(config.contact_hertz, config.contact_damping, h);
        let gravity = config.gravity;
        // O1: gates the integrate / inertia kernels (gravity, position integrate,
        // inertia refresh).
        let use_simd = config.simd;
        // O7: gates the cohort-batched colored CONTACT SOLVE — a SEPARATE flag from
        // O1's `simd` so the solve widen has independent A/B + rollback. Default OFF
        // ⇒ the scalar `solve_color` oracle (the O6 0%-gate, byte-identical).
        let use_simd_solve = config.simd_solve;
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
                use_simd_solve,
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
                    use_simd_solve,
                );
            }
        }

        // Post-loop restitution (ONCE, velocity-only, bias-free).
        Self::apply_restitution(&mut self.columns, &mut self.bodies);

        // IM-2b: store converged impulses in canonical order, then swap.
        self.store_and_swap();

        // O8 integrate-freeze RESTORE: undo the integrate on slept rows by restoring
        // their captured pre-solve hot state into `scratch.bodies` (position /
        // rotation / velocities) — so a slept island advances by exactly nothing. The
        // matching `BodyEffective` velocity (which `write_back` would copy out) is also
        // restored so the slept body keeps its frozen velocity. Then `write_back` is
        // told to SKIP slept rows, so `physics_apply` leaves the live component
        // untouched (frozen) — and the gather-walked-every-row IM-1 invariant holds.
        if sleeping_active {
            for &(row, snap) in &self.frozen {
                let r = row as usize;
                scratch.bodies[r] = snap;
                let eff = &mut self.bodies[r];
                eff.linear_velocity = snap.linear_velocity;
                eff.angular_velocity = snap.angular_velocity;
            }
        }

        // Write the solved velocities back and flag integrated DYNAMIC rows. With
        // sleeping on, slept rows are skipped (their `awake_rows` bit is clear), so the
        // frozen rows are never flagged touched — `physics_apply` leaves them be.
        if let Some(sleep) = sleep.as_deref() {
            self.write_back_awake(scratch, sleep);
        } else {
            self.write_back(scratch);
        }

        // O8 phase 2 (AFTER the solve): accumulate this frame's per-island energy from
        // the (post-restore) body velocities and advance the debounce / sleep
        // transition for next frame. The mutable reborrow is sound: `sleep_view` (the
        // immutable view) is dead after the freeze capture.
        if let Some(sleep) = sleep.as_mut() {
            sleep.end_step(&scratch.bodies, graph, config.sleep_threshold, config.sleep_frames);
        }
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
        solver.build_columns(&manifolds, &graph, &bodies, None);
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
            solver.build_columns(&manifolds, &graph, &bodies, None);
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
        solver.build_columns(&manifolds, &graph, &bodies, None);
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

    // ── O7 SIMD-batched colored solve: dev smoke tests ───────────────────────
    //
    // The exhaustive 1000-scene differential + proptest + criterion are the
    // tester's job. These dev tests assert the two INVIOLABLE properties:
    //   (1) bit-exact `solve_color_avx2 == solve_color` over a colored scene with
    //       ragged ranks (a multi-point box manifold mixed with width-1 groups
    //       across the 8-lane boundary), incl. mixed cone activation (+avx2 only);
    //   (2) width-only: `solve_colored(simd=true) == solve_colored(simd=false)` over
    //       a full step (on non-AVX2 / Miri both arms ARE the scalar oracle, so the
    //       check holds trivially; under +avx2 it proves the widened path matches
    //       the O5/O6 scalar colored result bit-for-bit).

    /// A dynamic body view from a `BodyState` (mirrors `build_bodies`' per-row map),
    /// for the direct-kernel differential (used only by the +avx2 differential).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn eff_of(b: &BodyState) -> BodyEffective {
        BodyEffective {
            inv_mass: b.inv_mass,
            inv_inertia: b.inv_inertia,
            linear_velocity: b.linear_velocity,
            angular_velocity: b.angular_velocity,
        }
    }

    /// Builds a colored scene whose columns cross the 8-group cohort boundary with
    /// RAGGED widths: `n_floor` width-1 spheres on a shared static floor (one color,
    /// since they share only the static floor → all body-disjoint dynamic rows) plus
    /// one width-4 box-box manifold (a separate dynamic pair). Returns
    /// `(bodies, manifolds)`.
    fn ragged_colored_scene(n_floor: u32) -> (Vec<BodyState>, Vec<Manifold>) {
        let mut bodies = Vec::new();
        // Spheres 0..n_floor, each penetrating a shared floor, spread along x so the
        // narrowphase keeps them distinct dynamic bodies.
        for i in 0..n_floor {
            // A non-trivial inertia + a small spin so the angular term + friction
            // cone are exercised non-vacuously.
            let mut b = dyn_sphere(Vec3::new(i as f32 * 3.0, 0.6, 0.0), 1.0, 0.7, 0.0);
            b.inv_inertia = Mat3::from_diagonal(Vec3::new(1.5, 1.5, 1.5));
            b.linear_velocity = Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1);
            b.angular_velocity = Vec3::new(0.05, -0.1, 0.2);
            bodies.push(b);
        }
        // Two dynamic boxes for the width-4 manifold.
        let mut box_a = dyn_sphere(Vec3::new(-5.0, 10.0, 0.0), 1.0, 0.6, 0.0);
        box_a.inv_inertia = Mat3::from_diagonal(Vec3::new(1.2, 0.9, 1.1));
        box_a.linear_velocity = Vec3::new(1.0, 0.0, -0.3);
        box_a.angular_velocity = Vec3::new(0.1, 0.2, -0.15);
        let mut box_b = dyn_sphere(Vec3::new(-3.0, 10.0, 0.0), 1.0, 0.6, 0.0);
        box_b.inv_inertia = Mat3::from_diagonal(Vec3::new(0.8, 1.3, 1.0));
        box_b.linear_velocity = Vec3::new(-1.0, 0.0, 0.3);
        box_b.angular_velocity = Vec3::new(-0.2, 0.05, 0.1);
        let box_a_row = bodies.len() as u32;
        bodies.push(box_a);
        let box_b_row = bodies.len() as u32;
        bodies.push(box_b);
        // The shared static floor (last row).
        let floor_row = bodies.len() as u32;
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

        let mut manifolds = Vec::new();
        for i in 0..n_floor {
            manifolds.push(manifold(
                i,
                floor_row,
                Vec3::new(0.0, -1.0, 0.0),
                -0.2,
                Vec3::new(i as f32 * 3.0, 0.0, 0.0),
            ));
        }
        // Width-4 box-box manifold: A → B along +x, deep overlap.
        manifolds.push(box_manifold(
            box_a_row,
            box_b_row,
            Vec3::new(1.0, 0.0, 0.0),
            -0.3,
            Vec3::new(-4.0, 10.0, 0.0),
            4,
        ));
        (bodies, manifolds)
    }

    /// Captures the full body + impulse-column bit state after solving each color's
    /// groups with the supplied per-color kernel. Returns `(body_bits,
    /// impulse_bits)`. Used only by the +avx2 differential.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn body_impulse_bits(bodies: &[BodyEffective], cols: &ContactColumns) -> (Vec<u32>, Vec<u32>) {
        let body_bits = bodies
            .iter()
            .flat_map(|b| {
                [
                    b.linear_velocity.x.to_bits(),
                    b.linear_velocity.y.to_bits(),
                    b.linear_velocity.z.to_bits(),
                    b.angular_velocity.x.to_bits(),
                    b.angular_velocity.y.to_bits(),
                    b.angular_velocity.z.to_bits(),
                ]
            })
            .collect();
        let impulse_bits = (0..cols.len())
            .flat_map(|i| {
                [
                    cols.normal_impulse[i].to_bits(),
                    cols.tangent1_impulse[i].to_bits(),
                    cols.tangent2_impulse[i].to_bits(),
                ]
            })
            .collect();
        (body_bits, impulse_bits)
    }

    /// Test 1 (INVIOLABLE-1): `solve_color_avx2 == solve_color` bit-exact over a
    /// ragged colored scene (width-1 floor groups crossing the 8-lane boundary + a
    /// width-4 box manifold ⇒ exhausted lanes at high ranks), for both
    /// `bias_active ∈ {true, false}`. AVX2-only (the kernel is `cfg`-gated); on a
    /// non-AVX2 build the dispatch IS the scalar oracle so the property is vacuous —
    /// the always-compiled width-only test below covers that build.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn simd_solve_bits_match_scalar() {
        // 11 floor spheres + 1 box pair ⇒ 12 width-1 groups + 1 width-4 group; the
        // floor spheres land in one color (≥ 9 groups ⇒ crosses the 8-cohort
        // boundary with a partial trailing cohort), the box pair in its own
        // group(s). Ragged widths in one cohort ⇒ exhausted-lane coverage.
        let (bodies, manifolds) = ragged_colored_scene(11);
        let graph = build_graph(&bodies, &manifolds);

        let soft = SoftCoefficients::new(
            PhysicsConfig::default().contact_hertz,
            PhysicsConfig::default().contact_damping,
            (1.0 / 60.0) / 4.0,
        );

        for bias_active in [true, false] {
            // Build columns once via the solver's own build, then snapshot the
            // pristine pre-solve state for the two arms.
            let mut solver = ColoredSoftStepSolver::default();
            solver.build_bodies(&bodies);
            solver.build_columns(&manifolds, &graph, &bodies, None);

            let pristine_cols_ni: Vec<f32> = solver.columns.normal_impulse.clone();
            let pristine_cols_t1: Vec<f32> = solver.columns.tangent1_impulse.clone();
            let pristine_cols_t2: Vec<f32> = solver.columns.tangent2_impulse.clone();
            let pristine_bodies: Vec<BodyEffective> = bodies.iter().map(eff_of).collect();

            let n_colors = solver.columns.color_offsets.len() - 1;

            // ── Scalar arm ──────────────────────────────────────────────────
            let mut cols_scalar = ContactColumns::default();
            clone_columns(&solver.columns, &mut cols_scalar);
            let mut bodies_scalar = pristine_bodies.clone();
            for c in 0..n_colors {
                let start = cols_scalar.color_offsets[c] as usize;
                let end = cols_scalar.color_offsets[c + 1] as usize;
                ColoredSoftStepSolver::solve_color(
                    &mut cols_scalar,
                    &mut bodies_scalar,
                    (start, end),
                    soft.bias_rate,
                    soft.mass_coeff,
                    soft.impulse_coeff,
                    bias_active,
                );
            }

            // ── SIMD arm (re-seed the impulse columns to the pristine state) ──
            let mut cols_simd = ContactColumns::default();
            clone_columns(&solver.columns, &mut cols_simd);
            cols_simd.normal_impulse.clone_from(&pristine_cols_ni);
            cols_simd.tangent1_impulse.clone_from(&pristine_cols_t1);
            cols_simd.tangent2_impulse.clone_from(&pristine_cols_t2);
            let mut bodies_simd = pristine_bodies.clone();
            for c in 0..n_colors {
                let g_lo = cols_simd.color_group_start[c] as usize;
                let g_hi = cols_simd.color_group_start[c + 1] as usize;
                let span = (
                    cols_simd.color_offsets[c] as usize,
                    cols_simd.color_offsets[c + 1] as usize,
                );
                // SAFETY: the test target is gated `target_feature = "avx2"`, so the
                //   host running these tests supports AVX2; the group range is a
                //   color's own (body-disjoint) groups, and `span` is exactly that
                //   range's slot run (the kernel's own-span contract).
                unsafe {
                    ColoredSoftStepSolver::solve_color_avx2(
                        &mut cols_simd,
                        &mut bodies_simd,
                        span,
                        g_lo,
                        g_hi,
                        soft.bias_rate,
                        soft.mass_coeff,
                        soft.impulse_coeff,
                        bias_active,
                    );
                }
            }

            let (b_scalar, i_scalar) = body_impulse_bits(&bodies_scalar, &cols_scalar);
            let (b_simd, i_simd) = body_impulse_bits(&bodies_simd, &cols_simd);
            assert_eq!(
                b_scalar, b_simd,
                "O7 body velocity bits must match scalar (bias_active={bias_active})"
            );
            assert_eq!(
                i_scalar, i_simd,
                "O7 impulse column bits must match scalar (bias_active={bias_active})"
            );
        }
    }

    /// Clones the columns needed by the per-color kernels into `dst` (a fresh
    /// `ContactColumns`). Only the columns `solve_color`/`solve_color_avx2` read or
    /// write are copied; the rest stay empty (unused by the kernels). Used only by
    /// the +avx2 differential.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn clone_columns(src: &ContactColumns, dst: &mut ContactColumns) {
        dst.ra_x.clone_from(&src.ra_x);
        dst.ra_y.clone_from(&src.ra_y);
        dst.ra_z.clone_from(&src.ra_z);
        dst.rb_x.clone_from(&src.rb_x);
        dst.rb_y.clone_from(&src.rb_y);
        dst.rb_z.clone_from(&src.rb_z);
        dst.normal_x.clone_from(&src.normal_x);
        dst.normal_y.clone_from(&src.normal_y);
        dst.normal_z.clone_from(&src.normal_z);
        dst.tangent1_x.clone_from(&src.tangent1_x);
        dst.tangent1_y.clone_from(&src.tangent1_y);
        dst.tangent1_z.clone_from(&src.tangent1_z);
        dst.tangent2_x.clone_from(&src.tangent2_x);
        dst.tangent2_y.clone_from(&src.tangent2_y);
        dst.tangent2_z.clone_from(&src.tangent2_z);
        dst.separation.clone_from(&src.separation);
        dst.friction.clone_from(&src.friction);
        dst.normal_impulse.clone_from(&src.normal_impulse);
        dst.tangent1_impulse.clone_from(&src.tangent1_impulse);
        dst.tangent2_impulse.clone_from(&src.tangent2_impulse);
        dst.body_a.clone_from(&src.body_a);
        dst.body_b.clone_from(&src.body_b);
        dst.b_is_sentinel.clone_from(&src.b_is_sentinel);
        dst.color_offsets.clone_from(&src.color_offsets);
        dst.group_start.clone_from(&src.group_start);
        dst.color_group_start.clone_from(&src.color_group_start);
    }

    /// Test 2 (width-only / 0%-gate proxy): `solve_colored(simd=true)` produces a
    /// full-step body snapshot bit-identical to `solve_colored(simd=false)`. On a
    /// non-AVX2 build both arms run the scalar oracle (so the equality is the
    /// structural 0%-gate); under +avx2 it proves the widened cohort kernel
    /// reproduces the O5/O6 scalar colored result bit-for-bit over a multi-substep
    /// step incl. the multi-point box manifold.
    #[test]
    fn simd_solve_width_only_matches_scalar_step() {
        let (bodies, manifolds) = ragged_colored_scene(11);

        let run_step = |simd_solve: bool| -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                simd_solve,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.bodies = bodies.clone();
            scratch.touched.reset(scratch.bodies.len());
            let graph = build_graph(&scratch.bodies, &manifolds);
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
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
        };

        assert_eq!(
            run_step(false),
            run_step(true),
            "width-only: simd_solve=true must be bit-identical to the scalar colored result"
        );
    }

    // ── O1 (regression-pin): cone / degenerate adversarial differential ──────
    //
    // Test 1c/1d build a SINGLE one-color, one-cohort `ContactColumns` BY HAND so
    // every lane's geometry / impulse seed / body state is exact, forcing the
    // adversarial friction-cone + degenerate paths to fire NON-VACUOUSLY, then
    // assert `solve_color_avx2 == solve_color` bit-for-bit. The non-vacuity counts
    // come from `cone_probe`, a single-slot replay of the EXACT scalar op sequence
    // (the authoritative oracle for "did this lane clamp / was len_sq zero /
    // denormal"). A splitmix64 proptest then sweeps random cohort shapes.

    /// One built group spec for a hand-rolled single-color cohort: a body pair
    /// (`ia`, `ib`/sentinel) and its contact points. Each point carries explicit
    /// geometry, friction, separation, and an impulse seed.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone)]
    struct GroupSpec {
        ia: u32,
        ib: u32,
        sentinel: bool,
        /// `(ra, rb, normal, t1, t2, separation, friction, seed_ni, seed_t1, seed_t2)`.
        points: Vec<PointSpec>,
    }

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone, Copy)]
    struct PointSpec {
        ra: Vec3,
        rb: Vec3,
        normal: Vec3,
        t1: Vec3,
        t2: Vec3,
        separation: f32,
        friction: f32,
        seed: (f32, f32, f32),
    }

    /// Builds a single-COLOR, single-cohort (`groups.len() <= 8`) `ContactColumns`
    /// from the group specs, appending groups in order with the C1 CSR
    /// (`group_start` / `color_group_start` / `color_offsets`). Body-disjointness of
    /// the groups is the CALLER's responsibility (the cohort kernel's precondition).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn build_cohort_columns(groups: &[GroupSpec]) -> ContactColumns {
        let mut cols = ContactColumns::default();
        cols.color_offsets.push(0);
        cols.group_start.push(0);
        cols.color_group_start.push(0);
        let mut warm_key = 0u64;
        for g in groups {
            for (p, ps) in g.points.iter().enumerate() {
                cols.push_point(
                    ps.ra,
                    ps.rb,
                    ps.normal,
                    ps.t1,
                    ps.t2,
                    ps.separation,
                    ps.friction,
                    0.0, // restitution (the kernels do not read it)
                    ps.seed,
                    g.ia,
                    g.ib,
                    g.sentinel,
                    warm_key,
                    0.0,
                );
                cols.canonical.push((cols.len() - 1) as u32);
                let _ = p;
                warm_key = warm_key.wrapping_add(1);
            }
            cols.group_start.push(cols.len() as u32);
        }
        cols.color_offsets.push(cols.len() as u32);
        cols.color_group_start.push((cols.group_start.len() - 1) as u32);
        cols
    }

    /// Replays the EXACT scalar `solve_color` friction-cone evaluation for ONE slot
    /// against the pristine pre-solve state, reporting `(clamped, zero_cone,
    /// denorm_len_sq)`. The kernel is bit-identical to `solve_color`, so this is the
    /// authoritative non-vacuity oracle for that slot. `len_sq == 0` ⇒ `zero_cone`;
    /// `0 < len_sq < f32::MIN_POSITIVE` ⇒ `denorm_len_sq`; the scalar clamp branch
    /// (`len_sq > mf² && len_sq > 0`) firing ⇒ `clamped`.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn cone_probe(
        cols: &ContactColumns,
        bodies: &[BodyEffective],
        slot: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) -> (bool, bool, bool) {
        let ra = cols.ra(slot);
        let rb = cols.rb(slot);
        let normal = cols.normal(slot);
        let t1 = cols.tangent1(slot);
        let t2 = cols.tangent2(slot);
        let ia = cols.body_a[slot] as usize;
        let b_sent = cols.b_is_sentinel[slot];
        let ib = cols.body_b[slot] as usize;
        let friction = cols.friction[slot];
        let separation = cols.separation[slot];
        let bb = if b_sent { IMMOVABLE_AT_REST } else { bodies[ib] };
        let ba = bodies[ia];

        // Normal solve (to obtain the new normal impulse the cone uses).
        let m_eff = effective_mass(normal, ra, rb, &ba, &bb);
        let vn = (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal);
        let bias = if bias_active {
            (bias_rate * separation).max(-MAX_BIAS_VELOCITY)
        } else {
            0.0
        };
        let lambda_n = cols.normal_impulse[slot];
        let d_lambda = if bias_active {
            -mass_coeff * m_eff * (vn + bias) - impulse_coeff * lambda_n
        } else {
            -m_eff * vn
        };
        let new_lambda = (lambda_n + d_lambda).max(0.0);

        // Friction solve (no body mutation needed — single-point group, the cone
        // reads only the post-normal velocity; a single-point group's normal apply
        // does change velocity, so re-derive from a local copy).
        let mut ba_m = ba;
        let mut bb_m = bb;
        let applied_n = new_lambda - lambda_n;
        let imp = normal * applied_n;
        if is_dynamic_row(ba_m.inv_mass) {
            ba_m.apply_impulse(ra, imp * -1.0);
        }
        if !b_sent && is_dynamic_row(bb_m.inv_mass) {
            bb_m.apply_impulse(rb, imp);
        }
        let max_friction = friction * new_lambda;
        let m_eff_t1 = effective_mass(t1, ra, rb, &ba_m, &bb_m);
        let m_eff_t2 = effective_mass(t2, ra, rb, &ba_m, &bb_m);
        let dv = bb_m.point_velocity(rb) - ba_m.point_velocity(ra);
        let (vt1, vt2) = (dv.dot(t1), dv.dot(t2));
        let new_t1 = cols.tangent1_impulse[slot] - m_eff_t1 * vt1;
        let new_t2 = cols.tangent2_impulse[slot] - m_eff_t2 * vt2;
        let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
        let clamped = len_sq > max_friction * max_friction && len_sq > 0.0;
        let zero_cone = len_sq == 0.0;
        let denorm = len_sq > 0.0 && len_sq < f32::MIN_POSITIVE;
        (clamped, zero_cone, denorm)
    }

    /// Solves the single color of `cols` with the scalar oracle and with the AVX2
    /// cohort kernel (each on a fresh clone seeded to the same pristine state), and
    /// asserts the body + impulse bits match bit-for-bit. Returns the per-slot
    /// `(clamped, zero_cone, denorm)` counts from the scalar probe for non-vacuity.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn assert_cohort_differential(
        cols: &ContactColumns,
        bodies: &[BodyEffective],
        bias_active: bool,
    ) -> (usize, usize, usize) {
        let soft = SoftCoefficients::new(
            PhysicsConfig::default().contact_hertz,
            PhysicsConfig::default().contact_damping,
            (1.0 / 60.0) / 4.0,
        );

        // Non-vacuity counts from the pristine state (the probe is read-only).
        let (mut clamped, mut zero_cone, mut denorm) = (0usize, 0usize, 0usize);
        for s in 0..cols.len() {
            let (c, z, d) = cone_probe(
                cols,
                bodies,
                s,
                soft.bias_rate,
                soft.mass_coeff,
                soft.impulse_coeff,
                bias_active,
            );
            clamped += c as usize;
            zero_cone += z as usize;
            denorm += d as usize;
        }

        let g_lo = cols.color_group_start[0] as usize;
        let g_hi = cols.color_group_start[1] as usize;
        let span = (cols.color_offsets[0] as usize, cols.color_offsets[1] as usize);

        // Scalar arm.
        let mut cols_scalar = ContactColumns::default();
        clone_columns(cols, &mut cols_scalar);
        let mut bodies_scalar = bodies.to_vec();
        ColoredSoftStepSolver::solve_color(
            &mut cols_scalar,
            &mut bodies_scalar,
            span,
            soft.bias_rate,
            soft.mass_coeff,
            soft.impulse_coeff,
            bias_active,
        );

        // SIMD arm.
        let mut cols_simd = ContactColumns::default();
        clone_columns(cols, &mut cols_simd);
        let mut bodies_simd = bodies.to_vec();
        // SAFETY: the test target is `target_feature = "avx2"`-gated, so the host
        //   supports AVX2; `[g_lo, g_hi)` is the single color's body-disjoint groups
        //   and `span` is exactly that group range's slot run (the own-span contract).
        unsafe {
            ColoredSoftStepSolver::solve_color_avx2(
                &mut cols_simd,
                &mut bodies_simd,
                span,
                g_lo,
                g_hi,
                soft.bias_rate,
                soft.mass_coeff,
                soft.impulse_coeff,
                bias_active,
            );
        }

        let (b_scalar, i_scalar) = body_impulse_bits(&bodies_scalar, &cols_scalar);
        let (b_simd, i_simd) = body_impulse_bits(&bodies_simd, &cols_simd);
        assert_eq!(b_scalar, b_simd, "cohort differential: body bits (bias_active={bias_active})");
        assert_eq!(i_scalar, i_simd, "cohort differential: impulse bits (bias_active={bias_active})");
        (clamped, zero_cone, denorm)
    }

    /// A dynamic `BodyEffective` with a diagonal inertia and the given velocity.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn dyn_eff(inv_mass: f32, inertia_diag: f32, lin: Vec3, ang: Vec3) -> BodyEffective {
        BodyEffective {
            inv_mass,
            inv_inertia: Mat3::from_diagonal(Vec3::new(inertia_diag, inertia_diag, inertia_diag)),
            linear_velocity: lin,
            angular_velocity: ang,
        }
    }

    /// Test 1c (+avx2 only): a single cohort with a cone-CLAMPED lane, an unclamped
    /// lane, a `len_sq == 0` zero-tangent lane, and a denormal-`len_sq` lane, all
    /// body-disjoint. Asserts `solve_color_avx2 == solve_color` bit-for-bit AND that
    /// the clamp / zero-cone / denormal paths each fire (non-vacuity).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn cone_adversarial_differential_test_1c() {
        // Build 5 body-disjoint single-point groups (one cohort), bodies 0..10.
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);
        let mk = |ia: u32,
                  ib: u32,
                  friction: f32,
                  seed_ni: f32,
                  seed_t1: f32,
                  seed_t2: f32,
                  ra: Vec3|
         -> GroupSpec {
            GroupSpec {
                ia,
                ib,
                sentinel: false,
                points: vec![PointSpec {
                    ra,
                    rb: ra,
                    normal: n,
                    t1,
                    t2,
                    separation: -0.2,
                    friction,
                    seed: (seed_ni, seed_t1, seed_t2),
                }],
            }
        };

        // Lane 0 — cone-CLAMPED: large pre-seeded tangent impulse + a small normal
        // cap (friction·λn small) ⇒ len_sq ≫ mf² ⇒ clamp fires.
        let g0 = mk(0, 1, 0.1, 0.05, 5.0, 5.0, Vec3::new(0.3, 0.0, 0.1));
        // Lane 1 — UNCLAMPED: tiny tangent seed, generous friction cap ⇒ inside cone.
        let g1 = mk(2, 3, 2.0, 2.0, 1e-4, 1e-4, Vec3::new(-0.2, 0.0, 0.2));
        // Lane 2 — ZERO-tangent (`len_sq == 0`): zero friction AND zero tangent seed
        // with zero tangential velocity ⇒ new_t1 == new_t2 == 0 ⇒ len_sq == 0.
        let g2 = mk(4, 5, 0.0, 1.0, 0.0, 0.0, Vec3::ZERO);
        // Lane 3 — DENORMAL len_sq: a tiny tangent seed (subnormal-squared) with zero
        // tangential velocity ⇒ new_t stays the seed ⇒ len_sq ≈ seed² is subnormal.
        let tiny = 1e-22f32; // tiny² ≈ 1e-44 < f32::MIN_POSITIVE (≈ 1.18e-38)
        let g3 = mk(6, 7, 5.0, 0.0, tiny, 0.0, Vec3::ZERO);
        // Lane 4 — a second clamped lane on a sentinel body B (static surface).
        let mut g4 = mk(8, 9, 0.2, 0.1, 4.0, -3.0, Vec3::new(0.1, 0.0, -0.3));
        g4.sentinel = true;
        g4.ib = u32::MAX;

        let groups = vec![g0, g1, g2, g3, g4];
        let cols = build_cohort_columns(&groups);
        // 10 real bodies; spins so the angular term is non-vacuous. Lane-2 (g2)
        // bodies are zero-velocity so its tangent stays exactly zero.
        let bodies: Vec<BodyEffective> = (0..10)
            .map(|i| {
                if (4..=5).contains(&i) {
                    dyn_eff(1.0, 1.5, Vec3::ZERO, Vec3::ZERO)
                } else if (6..=7).contains(&i) {
                    // Lane-3 bodies zero-velocity too so its tangent stays the seed.
                    dyn_eff(1.0, 1.5, Vec3::ZERO, Vec3::ZERO)
                } else {
                    dyn_eff(
                        1.0,
                        1.5,
                        Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.15),
                        Vec3::new(0.05, -0.1, 0.2),
                    )
                }
            })
            .collect();

        let mut total_clamped = 0;
        let mut total_zero = 0;
        let mut total_denorm = 0;
        for bias_active in [true, false] {
            let (c, z, d) = assert_cohort_differential(&cols, &bodies, bias_active);
            total_clamped += c;
            total_zero += z;
            total_denorm += d;
        }
        eprintln!(
            "test_1c non-vacuity: clamped={total_clamped} zero_cone={total_zero} denorm={total_denorm}"
        );
        assert!(
            total_clamped > 0 && total_zero > 0 && total_denorm > 0,
            "non-vacuity: cone clamp ({total_clamped}), zero-cone ({total_zero}), and denormal \
             len_sq ({total_denorm}) lanes must each fire across the two bias modes"
        );
    }

    /// Test 1d (+avx2 only): a single cohort mixing a static-A lane (`inv_mass == 0`
    /// on body A — the `*_movable` guard side), a sentinel-B lane, and a `k <= 0`
    /// degenerate lane (both bodies static ⇒ `effective_mass == 0`). Asserts
    /// `solve_color_avx2 == solve_color` bit-for-bit; non-vacuity asserts the cone
    /// fires on the live lane.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn degenerate_lane_differential_test_1d() {
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);
        let pt = |friction: f32, seed: (f32, f32, f32), ra: Vec3| PointSpec {
            ra,
            rb: ra,
            normal: n,
            t1,
            t2,
            separation: -0.25,
            friction,
            seed,
        };

        // Lane 0 — STATIC body A (inv_mass 0): the `ia_movable == false` guard side.
        let g0 = GroupSpec {
            ia: 0,
            ib: 1,
            sentinel: false,
            points: vec![pt(0.5, (0.1, 0.2, -0.1), Vec3::new(0.2, 0.0, 0.1))],
        };
        // Lane 1 — SENTINEL body B: body B is IMMOVABLE_AT_REST, never indexed; a
        // live dynamic A with a clamp-forcing tangent seed.
        let g1 = GroupSpec {
            ia: 2,
            ib: u32::MAX,
            sentinel: true,
            points: vec![pt(0.05, (0.05, 6.0, 6.0), Vec3::new(-0.1, 0.0, 0.3))],
        };
        // Lane 2 — DEGENERATE k<=0: both bodies static (inv_mass 0, inertia ZERO) ⇒
        // effective_mass returns 0 ⇒ a no-op solve.
        let g2 = GroupSpec {
            ia: 3,
            ib: 4,
            sentinel: false,
            points: vec![pt(0.5, (0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.2))],
        };

        let groups = vec![g0, g1, g2];
        let cols = build_cohort_columns(&groups);
        // Bodies: 0 static-A, 1 dynamic, 2 dynamic (sentinel lane's A), 3+4 static.
        let bodies = vec![
            // 0: static A (inv_mass 0 ⇒ inertia ZERO to match the build invariant).
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::new(1.0, -0.5, 0.2), angular_velocity: Vec3::new(0.1, 0.0, -0.1) },
            // 1: dynamic B.
            dyn_eff(1.0, 1.2, Vec3::new(-0.3, 0.4, 0.1), Vec3::new(-0.05, 0.1, 0.0)),
            // 2: dynamic A (sentinel lane) with a fast tangential slide ⇒ cone fires.
            dyn_eff(1.0, 1.0, Vec3::new(2.0, -1.0, -1.5), Vec3::new(0.2, -0.1, 0.3)),
            // 3, 4: both static (degenerate k<=0 lane).
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::ZERO, angular_velocity: Vec3::ZERO },
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::ZERO, angular_velocity: Vec3::ZERO },
        ];

        let mut total_clamped = 0;
        for bias_active in [true, false] {
            let (c, _z, _d) = assert_cohort_differential(&cols, &bodies, bias_active);
            total_clamped += c;
        }
        eprintln!("test_1d non-vacuity: clamped={total_clamped}");
        assert!(
            total_clamped > 0,
            "non-vacuity: the sentinel-B live lane's friction cone must clamp at least once"
        );
    }

    /// A splitmix64 PRNG (deterministic, no deps) for the cohort-shape proptest.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    struct SplitMix64(u64);

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    impl SplitMix64 {
        fn next_u64(&mut self) -> u64 {
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

    /// O1 proptest (+avx2 only): random cohort shapes (group count 1..=32, width
    /// 1..=MAX_CONTACT_POINTS, masses incl. statics + sentinels, denormal-scale
    /// velocities) must be `solve_color_avx2 == solve_color` bit-for-bit, AND the
    /// cone clamp + zero-cone paths must fire non-vacuously across the corpus.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn cohort_shape_proptest_bit_exact_and_non_vacuous() {
        use crate::math::MAX_CONTACT_POINTS;
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);

        let mut rng = SplitMix64(0x0BAD_F00D_DEAD_BEEF);
        let mut corpus_clamped = 0usize;
        let mut corpus_zero = 0usize;

        for _ in 0..200 {
            let n_groups = rng.range(1, 33) as usize; // 1..=32 ⇒ multi-cohort
            let mut groups: Vec<GroupSpec> = Vec::with_capacity(n_groups);
            // Body rows: each group owns 2 disjoint dynamic rows (or 1 + sentinel).
            let mut bodies: Vec<BodyEffective> = Vec::with_capacity(n_groups * 2);
            for _gi in 0..n_groups {
                let ia = bodies.len() as u32;
                // Body A: mostly dynamic, sometimes static (the *_movable guard).
                let a_static = rng.f01() < 0.15;
                bodies.push(if a_static {
                    BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(&mut rng), angular_velocity: rand_vel(&mut rng) }
                } else {
                    dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(&mut rng), rand_vel(&mut rng))
                });
                let sentinel = rng.f01() < 0.25;
                let ib = if sentinel {
                    u32::MAX
                } else {
                    let row = bodies.len() as u32;
                    let b_static = rng.f01() < 0.15;
                    bodies.push(if b_static {
                        BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(&mut rng), angular_velocity: rand_vel(&mut rng) }
                    } else {
                        dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(&mut rng), rand_vel(&mut rng))
                    });
                    row
                };
                let width = rng.range(1, MAX_CONTACT_POINTS as u32 + 1) as usize;
                let mut points = Vec::with_capacity(width);
                for _ in 0..width {
                    // Occasionally a denormal-scale tangent seed + zero friction.
                    let denorm = rng.f01() < 0.1;
                    let zero_fric = rng.f01() < 0.1;
                    let seed_scale = if denorm { 1e-22 } else { 4.0 };
                    points.push(PointSpec {
                        ra: rand_vel(&mut rng) * 0.3,
                        rb: rand_vel(&mut rng) * 0.3,
                        normal: n,
                        t1,
                        t2,
                        separation: -(rng.f01() * 0.5),
                        friction: if zero_fric { 0.0 } else { rng.f01() * 2.0 },
                        seed: (
                            rng.f01() * 0.5,
                            (rng.f01() - 0.5) * seed_scale,
                            (rng.f01() - 0.5) * seed_scale,
                        ),
                    });
                }
                groups.push(GroupSpec { ia, ib, sentinel, points });
            }

            // build_cohort_columns packs ALL groups into ONE color (multi-cohort
            // when n_groups > 8); the kernel solves them as 8-group cohorts.
            let cols = build_cohort_columns(&groups);
            for bias_active in [true, false] {
                let (c, z, _d) = assert_cohort_differential(&cols, &bodies, bias_active);
                corpus_clamped += c;
                corpus_zero += z;
            }
        }
        eprintln!("proptest non-vacuity: clamped={corpus_clamped} zero_cone={corpus_zero}");
        assert!(
            corpus_clamped > 0 && corpus_zero > 0,
            "non-vacuity over the random corpus: clamp ({corpus_clamped}) and zero-cone \
             ({corpus_zero}) paths must both fire"
        );
    }

    /// A bounded random velocity for the proptest.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn rand_vel(rng: &mut SplitMix64) -> Vec3 {
        Vec3::new(
            (rng.f01() - 0.5) * 4.0,
            (rng.f01() - 0.5) * 4.0,
            (rng.f01() - 0.5) * 4.0,
        )
    }

    // ── O8 sleeping sanity tests ─────────────────────────────────────────────
    //
    // These build the bodies + per-step manifolds by hand and drive
    // `solve_colored_sleeping` directly (NO schedule / threadpool), so they run
    // native and under Miri. The exhaustive determinism / oscillation / criterion
    // suite is the tester's job.

    /// Drives the colored solver with O8 sleeping for `steps` fixed steps, returning
    /// `(final Y positions, the IslandSleep state)`. `cfg_mut` tweaks the config (e.g.
    /// the sleep threshold / frame count). The manifolds are re-derived from the
    /// current positions each step (the narrowphase stand-in), so a settled stack
    /// keeps producing its resting-floor contacts.
    fn run_sleeping(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
        cfg_mut: impl Fn(&mut PhysicsConfig),
    ) -> (Vec<f32>, IslandSleep) {
        let mut cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            ..PhysicsConfig::default()
        };
        cfg_mut(&mut cfg);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.bodies = bodies;
        scratch.touched.reset(scratch.bodies.len());
        let mut sleep = IslandSleep::with_capacity(scratch.bodies.len(), scratch.bodies.len());

        for _ in 0..steps {
            let manifolds = build_manifolds(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        let ys = scratch.bodies.iter().map(|b| b.position.y).collect();
        (ys, sleep)
    }

    /// A dynamic sphere resting on a static floor settles, then the island sleeps
    /// after `sleep_frames` consecutive low-energy frames (the headline O8 gate).
    #[test]
    fn dropped_body_settles_then_sleeps() {
        // Sphere just above a static floor; a short debounce so the test is brisk.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.05, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, 0.0, 0.0)),
        ];
        // The narrowphase stand-in: emit a floor contact whenever the sphere dips into
        // the floor (separation < 0), keyed (sphere, floor).
        let build = |bs: &[BodyState]| {
            let y = bs[0].position.y;
            let sep = y - 1.0; // sphere radius 1, floor surface at y = 1.0.
            if sep < 0.0 {
                vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), sep, Vec3::new(0.0, 1.0, 0.0))]
            } else {
                vec![]
            }
        };
        // Settle for many frames with a short 8-frame debounce, then keep stepping so
        // the debounce elapses.
        let (ys, sleep) = run_sleeping(bodies, build, 200, |c| c.sleep_frames = 8);

        // The sphere rests on the floor (did not sink far through it, did not fly off).
        assert!(
            (ys[0] - 1.0).abs() < 0.1,
            "sphere should rest near the floor surface (y ≈ 1.0), got {}",
            ys[0]
        );
        // The sphere's row is latched asleep (it settled, the debounce elapsed).
        assert!(
            sleep.is_row_asleep(0),
            "the settled body row must be latched asleep after the debounce"
        );
    }

    /// A slept island stays frozen across steps — its body neither drifts nor
    /// accumulates gravity (the integrate-skip gate). The floor contact is dropped
    /// while asleep, but the body must not fall.
    #[test]
    fn slept_body_is_frozen_no_drift() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, 0.0, 0.0)),
        ];
        // Resting-floor contact every step (sphere exactly on the surface).
        let build = |_bs: &[BodyState]| {
            vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.001, Vec3::new(0.0, 1.0, 0.0))]
        };
        let (ys, sleep) = run_sleeping(bodies, build, 100, |c| c.sleep_frames = 4);
        assert!(sleep.is_row_asleep(0), "the resting body row must be latched asleep");
        // A frozen body neither drifts down (gravity skipped) nor pops up.
        assert!(
            (ys[0] - 1.0).abs() < 1.0e-3,
            "a slept body must stay frozen at its rest Y, got {}",
            ys[0]
        );
    }

    /// **The real-pipeline wake-on-merge gate (the rewritten C1/C2 test).** A faller
    /// (a new awake body bringing a NEW contact) wakes a slept pile the SAME frame the
    /// contact appears — validated through the REAL solve, NOT a stale-graph artifact.
    ///
    /// Both arms drive `solve_colored_sleeping` step-by-step, re-deriving the manifolds
    /// AND the graph from the SAME current positions every frame (so `begin_step` sees
    /// exactly the graph the solve uses — the bug the old test cheated around). A pile
    /// of two spheres on a floor settles + latches asleep; then a faller is dropped onto
    /// it. The asserted behaviour: on the frame the faller's contact first appears, the
    /// pile's rows are ACTIVE (awake, not frozen) and the contact is resolved — no
    /// mid-air freeze, no penetration-stick.
    #[test]
    fn faller_wakes_slept_pile_same_frame_no_penetration() {
        // Pile: two stacked dynamic spheres (radius 1) resting on a static floor.
        // floor top surface at y = 1; sphere 0 centre at y ≈ 1; sphere 1 at y ≈ 3.
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // row 0 (bottom)
                dyn_sphere(Vec3::new(0.0, 3.0, 0.0), 1.0, 0.5, 0.0), // row 1 (top)
                static_body(Vec3::new(0.0, 0.0, 0.0)),               // row 2 (floor)
                dyn_sphere(Vec3::new(0.0, 30.0, 0.0), 1.0, 0.5, 0.0), // row 3 (faller, far above)
            ]
        };
        // Narrowphase stand-in: floor↔bottom, bottom↔top, top↔faller — each emitted
        // only while penetrating (separation < 0). Sphere radius 1 ⇒ centres touch at
        // distance 2; floor surface at y = 1.
        let build = |bs: &[BodyState]| {
            let mut ms = Vec::new();
            // floor contact for the bottom sphere.
            let sep_floor = bs[0].position.y - 1.0;
            if sep_floor < 0.0 {
                ms.push(manifold(0, 2, Vec3::new(0.0, -1.0, 0.0), sep_floor, bs[0].position));
            }
            // bottom↔top sphere-sphere.
            let d01 = bs[1].position.y - bs[0].position.y;
            if d01 - 2.0 < 0.0 {
                ms.push(manifold(0, 1, Vec3::new(0.0, 1.0, 0.0), d01 - 2.0, bs[0].position));
            }
            // top↔faller sphere-sphere (the NEW contact that must wake the pile).
            let d13 = bs[3].position.y - bs[1].position.y;
            if d13 - 2.0 < 0.0 {
                ms.push(manifold(1, 3, Vec3::new(0.0, 1.0, 0.0), d13 - 2.0, bs[1].position));
            }
            ms
        };

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 6,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(4);
        scratch.bodies = make();
        // Park the faller out of the simulation (no gravity reaches it until we drop
        // it) by zeroing its inv_mass for the settle phase: an inv_mass==0 row is not
        // an island node, so it cannot perturb the pile's sleep.
        scratch.bodies[3].inv_mass = 0.0;
        let mut sleep = IslandSleep::with_capacity(4, 4);

        // Settle phase: step until the pile latches asleep (bottom + top rows).
        for _ in 0..120 {
            let manifolds = build(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        assert!(
            sleep.is_row_asleep(0) && sleep.is_row_asleep(1),
            "the pile rows must latch asleep before the faller arrives"
        );
        let pile_top_y_before = scratch.bodies[1].position.y;

        // Drop the faller: give it mass + place it just above the top sphere so its
        // contact appears within a couple of steps.
        scratch.bodies[3].inv_mass = 1.0;
        scratch.bodies[3].position.y = 5.0; // touches the top sphere (centre y≈3) soon.

        // Step until the faller's contact first appears, then assert the pile woke that
        // SAME frame: its rows are awake (active), the contact was solved, and the pile
        // is not penetrated through.
        let mut woke_frame = None;
        for frame in 0..30 {
            let manifolds = build(&scratch.bodies);
            let faller_contact = manifolds.iter().any(|m| {
                (m.body_a.0 == 1 && m.body_b.0 == 3) || (m.body_a.0 == 3 && m.body_b.0 == 1)
            });
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);

            if faller_contact {
                // The frame the new contact appears: the pile's rows MUST be active
                // (awake) this same frame — wake-on-merge. They share an island with
                // the awake faller (row 3), so none of {0,1,3} may be frozen.
                assert!(
                    sleep.is_row_awake(0) && sleep.is_row_awake(1) && sleep.is_row_awake(3),
                    "the pile + faller rows must be ACTIVE the frame the new contact appears \
                     (wake-on-merge), not frozen"
                );
                woke_frame = Some(frame);
                break;
            }
        }
        assert!(
            woke_frame.is_some(),
            "the faller must produce a contact with the pile within the step budget"
        );

        // No penetration-stick: keep stepping; the faller must come to rest ABOVE the
        // top sphere (it cannot pass through a now-active pile).
        for _ in 0..60 {
            let manifolds = build(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        assert!(
            scratch.bodies[3].position.y > scratch.bodies[1].position.y,
            "the faller must rest ABOVE the top sphere, not sink through it: faller y={}, top y={}",
            scratch.bodies[3].position.y,
            scratch.bodies[1].position.y
        );
        // The pile did not get shoved through the floor by the impact.
        assert!(
            scratch.bodies[1].position.y < pile_top_y_before + 0.5,
            "the pile must absorb the faller near its rest height, not be launched: \
             top y={}, was {}",
            scratch.bodies[1].position.y,
            pile_top_y_before
        );
    }

    /// `wake_all` clears every row's latch on the next `begin_step` (wake condition
    /// (i)/(iii) — explicit / config-change wake), so no island can be frozen.
    #[test]
    fn wake_all_wakes_every_row() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.5, 1.0, 0.0), 1.0, 0.5, 0.0),
        ];
        let ms = vec![manifold(0, 1, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.25, 1.0, 0.0))];
        let graph = build_graph(&bodies, &ms);
        let isl = graph.island_of(0);

        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&graph, bodies.len());
        // Latch both rows of the island asleep, then confirm the island is frozen.
        sleep.force_sleep_row(0);
        sleep.force_sleep_row(1);
        sleep.begin_step(&graph, bodies.len());
        assert!(
            sleep.is_island_frozen(isl),
            "an island whose every row is latched must be frozen"
        );

        // wake_all clears the latch, so the island is active again next frame.
        sleep.wake_all();
        sleep.begin_step(&graph, bodies.len());
        assert!(
            !sleep.is_island_frozen(isl) && !sleep.is_row_asleep(0) && !sleep.is_row_asleep(1),
            "wake_all must wake every row (no island frozen)"
        );
    }

    /// Topology change is row-keyed and cannot spuriously freeze a moving island (C3).
    /// A two-row island latches asleep; then the manifold set splits it into two
    /// singleton islands AND a brand-new awake row joins one of them. The row latch
    /// follows the BODY, not the volatile island id, so: the unperturbed singleton
    /// stays frozen (its row is still latched), and the singleton that gained the new
    /// awake row is ACTIVE (no spurious freeze of a now-moving partition).
    #[test]
    fn topology_split_is_row_keyed_no_spurious_freeze() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // row 0
            dyn_sphere(Vec3::new(0.5, 1.0, 0.0), 1.0, 0.5, 0.0), // row 1
            dyn_sphere(Vec3::new(0.6, 1.0, 0.0), 1.0, 0.5, 0.0), // row 2 (new awake)
        ];
        // Frame A: rows 0+1 form one island; row 2 is its own singleton.
        let ms_a = vec![manifold(0, 1, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.25, 1.0, 0.0))];
        let graph_a = build_graph(&bodies, &ms_a);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&graph_a, bodies.len());
        // Latch rows 0 and 1 asleep (the resting pair); leave row 2 awake.
        sleep.force_sleep_row(0);
        sleep.force_sleep_row(1);

        // Frame B: the manifold set SPLITS — 0 alone, and 1+2 now coupled (a new awake
        // contact). Island ids are re-derived; the row latch is what carries.
        let ms_b = vec![manifold(1, 2, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.55, 1.0, 0.0))];
        let graph_b = build_graph(&bodies, &ms_b);
        sleep.begin_step(&graph_b, bodies.len());

        let isl0 = graph_b.island_of(0);
        let isl12 = graph_b.island_of(1);
        // Row 0 alone: still latched ⇒ its singleton island is frozen.
        assert!(
            sleep.is_island_frozen(isl0) && !sleep.is_row_awake(0),
            "the undisturbed latched row must stay frozen across the split"
        );
        // Rows 1+2: row 2 is awake (never latched), so their merged island is ACTIVE —
        // no spurious freeze of a partition that gained a moving row.
        assert!(
            isl0 != isl12,
            "the split must put row 0 in a different island from rows 1+2"
        );
        assert!(
            !sleep.is_island_frozen(isl12) && sleep.is_row_awake(1) && sleep.is_row_awake(2),
            "an island that gained an awake row must be ACTIVE (no C3 spurious freeze)"
        );
    }

    /// Sleeping with a threshold of `0` (an island can NEVER drop below it) is
    /// byte-identical to the sleeping-OFF colored solve — nothing ever sleeps, so the
    /// solve + integrate are never skipped (the 0%-gate at the value level).
    #[test]
    fn sleeping_that_never_sleeps_matches_sleeping_off() {
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 2.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.0, 4.0, 0.0), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, 0.0, 0.0)),
            ]
        };
        // A simple stacking narrowphase: floor contact + sphere-sphere contact.
        let build = |bs: &[BodyState]| {
            let mut ms = Vec::new();
            let y0 = bs[0].position.y;
            if y0 - 1.0 < 0.0 {
                ms.push(manifold(0, 2, Vec3::new(0.0, -1.0, 0.0), y0 - 1.0, Vec3::new(0.0, 1.0, 0.0)));
            }
            let d = bs[1].position.y - bs[0].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(0, 1, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[0].position));
            }
            ms
        };

        // Sleeping OFF reference.
        let ys_off = run(make(), build, 30);
        // Sleeping ON but threshold 0 → nothing ever sleeps.
        let (ys_on, sleep) = run_sleeping(make(), build, 30, |c| c.sleep_threshold = 0.0);

        assert!(
            !sleep.is_row_asleep(0) && !sleep.is_row_asleep(1),
            "with threshold 0 no row may latch asleep"
        );
        // Bit-identical (threshold-0 sleeping never skips solve/integrate).
        assert_eq!(
            ys_off.len(),
            ys_on.len(),
            "the two runs must produce the same body count"
        );
        for (off, on) in ys_off.iter().zip(ys_on.iter()) {
            assert_eq!(
                off.to_bits(),
                on.to_bits(),
                "sleeping-ON-but-never-sleeps must be BIT-identical to sleeping-OFF"
            );
        }
    }

    // ── O8 TESTER GATES (the re-review's deferred formal-gate list) ───────────
    //
    // These extend the dev's in-module sanity tests to the FORMAL gates: a larger
    // settled+slept stack hit by a faller (gate 1), rest==rest to ε (gate 2),
    // run-to-run bit-determinism on a sleep+WAKE scene (gate 3), the 0%-gate at
    // SCALE over the O6/O7 random corpus (gate 4), the no-oscillation debounce
    // proptest (gate 5), and topology-churn no-spurious-freeze (gate 7). They reuse
    // the in-module helpers (`dyn_sphere`/`static_body`/`manifold`/`build_graph`/
    // `run`/`run_sleeping`/`random_scene`) and the `#[cfg(test)]` `is_row_asleep`
    // hook, so they run native AND under Miri (no schedule / threadpool).

    use crate::resources::DEFAULT_SLEEP_THRESHOLD;

    /// A full body snapshot (position + rotation + velocities, bit-exact) of every
    /// row — the load-bearing comparand for the determinism / rest-to-ε gates.
    #[derive(Clone, PartialEq)]
    struct Snap {
        position: Vec3,
        rotation: Quat,
        linear_velocity: Vec3,
        angular_velocity: Vec3,
    }

    fn snap(bodies: &[BodyState]) -> Vec<Snap> {
        bodies
            .iter()
            .map(|b| Snap {
                position: b.position,
                rotation: b.rotation,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
            })
            .collect()
    }

    /// Bit-exact equality of two snapshots (every f32 component compared by `to_bits`).
    fn snaps_bit_equal(a: &[Snap], b: &[Snap]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).all(|(x, y)| {
            let v = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            let q = |r: Quat| [r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()];
            v(x.position) == v(y.position)
                && q(x.rotation) == q(y.rotation)
                && v(x.linear_velocity) == v(y.linear_velocity)
                && v(x.angular_velocity) == v(y.angular_velocity)
        })
    }

    /// Drives the colored solver with O8 sleeping, returning the FULL final body
    /// snapshot (not just Y). `sleeping` toggles the O8 path; `cfg_mut` tweaks the
    /// rest of the config. Mirrors `run_sleeping` but exposes the whole state so the
    /// determinism / rest-to-ε gates can compare every field, and lets the caller
    /// drop the sleeping flag (for the rest==rest reference arm).
    fn run_snap(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
        sleeping: bool,
        cfg_mut: impl Fn(&mut PhysicsConfig),
    ) -> Vec<Snap> {
        let mut cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping,
            ..PhysicsConfig::default()
        };
        cfg_mut(&mut cfg);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.bodies = bodies;
        scratch.touched.reset(scratch.bodies.len());
        let mut sleep = IslandSleep::with_capacity(scratch.bodies.len(), scratch.bodies.len());

        for _ in 0..steps {
            let manifolds = build_manifolds(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            if sleeping {
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            } else {
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        }
        snap(&scratch.bodies)
    }

    /// A vertical stack of `n` dynamic spheres (radius 1) resting on a static floor:
    /// centres at y = 1, 3, 5, …; the floor is the last row. Returns the bodies.
    fn vertical_stack(n: usize) -> Vec<BodyState> {
        let mut bodies = Vec::with_capacity(n + 1);
        for i in 0..n {
            bodies.push(dyn_sphere(Vec3::new(0.0, 1.0 + 2.0 * i as f32, 0.0), 1.0, 0.5, 0.0));
        }
        bodies.push(static_body(Vec3::new(0.0, 0.0, 0.0)));
        bodies
    }

    /// The per-step narrowphase stand-in for a vertical stack of `n` dynamic spheres
    /// on a floor (floor is row `n`): floor↔bottom + each adjacent sphere pair, each
    /// emitted only while penetrating (separation < 0).
    fn stack_manifolds(bs: &[BodyState], n: usize) -> Vec<Manifold> {
        let floor = n as u32;
        let mut ms = Vec::new();
        let sep_floor = bs[0].position.y - 1.0;
        if sep_floor < 0.0 {
            ms.push(manifold(0, floor, Vec3::new(0.0, -1.0, 0.0), sep_floor, bs[0].position));
        }
        for i in 0..n.saturating_sub(1) {
            let d = bs[i + 1].position.y - bs[i].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(
                    i as u32,
                    (i + 1) as u32,
                    Vec3::new(0.0, 1.0, 0.0),
                    d - 2.0,
                    bs[i].position,
                ));
            }
        }
        ms
    }

    /// **Gate 1 — a larger settled+slept stack hit by a faller wakes the SAME frame,
    /// the faller does not freeze mid-air, and no body penetrates beyond one
    /// narrowphase margin.** Beyond the dev's 2-sphere test: a 6-sphere stack on a
    /// floor (rows 0..=5, floor row 6, faller row 7). Built from the SAME per-frame
    /// manifolds the solve sees (the honest pipeline). The stack settles + latches,
    /// then a faller is dropped onto the top; the frame its contact appears the whole
    /// merged island is ACTIVE (no frozen row), and no resting body sinks through.
    #[test]
    fn larger_slept_stack_wakes_same_frame_no_penetration() {
        const N: usize = 6;
        let faller = (N + 1) as u32;
        let make = || {
            let mut b = vertical_stack(N); // rows 0..N dyn, row N floor
            b.push(dyn_sphere(Vec3::new(0.0, 60.0, 0.0), 1.0, 0.5, 0.0)); // row N+1 faller
            b
        };
        // Narrowphase: the stack contacts (rows 0..N + floor) plus a top↔faller
        // contact when the faller penetrates the top sphere (row N-1).
        let build = |bs: &[BodyState]| {
            let mut ms = stack_manifolds(bs, N);
            let top = (N - 1) as u32;
            let d = bs[faller as usize].position.y - bs[top as usize].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(top, faller, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[top as usize].position));
            }
            ms
        };

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 6,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(N + 2);
        scratch.bodies = make();
        // Park the faller (inv_mass 0 = not an island node) so it cannot perturb the
        // pile's settle; un-park it once the pile is asleep.
        scratch.bodies[faller as usize].inv_mass = 0.0;
        let mut sleep = IslandSleep::with_capacity(N + 2, N + 2);

        for _ in 0..400 {
            let manifolds = build(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        // The whole stack must be latched asleep before the faller arrives.
        for r in 0..N {
            assert!(
                sleep.is_row_asleep(r),
                "stack row {r} must latch asleep before the faller (settle failed)"
            );
        }
        // Resting heights of the slept stack — used to bound penetration after impact.
        let rest_y: Vec<f32> = (0..N).map(|r| scratch.bodies[r].position.y).collect();

        // Drop the faller onto the top sphere.
        scratch.bodies[faller as usize].inv_mass = 1.0;
        scratch.bodies[faller as usize].position.y = 5.0; // just above the top sphere (centre ~11)?
        // Place it a touch above the actual top so the contact appears within a few steps.
        scratch.bodies[faller as usize].position.y = scratch.bodies[N - 1].position.y + 2.5;

        let mut woke = false;
        for _ in 0..40 {
            let manifolds = build(&scratch.bodies);
            let faller_contact = manifolds.iter().any(|m| {
                (m.body_a.0 == (N - 1) as u32 && m.body_b.0 == faller)
                    || (m.body_a.0 == faller && m.body_b.0 == (N - 1) as u32)
            });
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);

            if faller_contact {
                // Wake-on-merge: the merged island (stack rows + faller) is ACTIVE the
                // SAME frame — every member row awake, none frozen.
                for r in 0..N {
                    assert!(
                        sleep.is_row_awake(r),
                        "stack row {r} must be ACTIVE the frame the faller's contact appears (wake-on-merge)"
                    );
                }
                assert!(
                    sleep.is_row_awake(faller as usize),
                    "the faller must be awake (it never slept; it must not freeze mid-air)"
                );
                woke = true;
                break;
            }
            // While the faller is still falling (no contact yet) it must NOT be frozen.
            assert!(
                sleep.is_row_awake(faller as usize),
                "the faller must not freeze mid-air before it touches the pile"
            );
        }
        assert!(woke, "the faller must reach the pile within the step budget");

        // Settle the impact and assert no penetration-stick: no resting body sank
        // more than one narrowphase margin (1.0) below its pre-impact rest height, and
        // adjacent spheres keep their ~2.0 centre spacing (no inter-penetration > 1).
        for _ in 0..120 {
            let manifolds = build(&scratch.bodies);
            let graph = build_graph(&scratch.bodies, &manifolds);
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        for (r, &rest) in rest_y.iter().enumerate() {
            assert!(
                scratch.bodies[r].position.y > rest - 1.0,
                "stack row {r} sank through the pile under impact: y={}, rest was {rest}",
                scratch.bodies[r].position.y,
            );
        }
        for i in 0..N - 1 {
            let gap = scratch.bodies[i + 1].position.y - scratch.bodies[i].position.y;
            assert!(
                gap > 1.0,
                "adjacent stack spheres {i}/{} penetrated > one margin: gap={gap}",
                i + 1
            );
        }
        // The faller came to rest ABOVE the top sphere (did not tunnel through).
        assert!(
            scratch.bodies[faller as usize].position.y > scratch.bodies[N - 1].position.y,
            "the faller tunnelled through the pile: faller y={}, top y={}",
            scratch.bodies[faller as usize].position.y,
            scratch.bodies[N - 1].position.y
        );
    }

    /// **Gate 2 — a settled stack's resting state with sleeping ON == with sleeping
    /// OFF, to a small ε.** Sleeping must not change the settled configuration, only
    /// stop integrating it. A 4-sphere stack settles for many frames under both
    /// configs; the final positions must match within ε (the slept arm freezes the
    /// converged rest pose; the awake arm keeps micro-integrating it — they agree to ε).
    #[test]
    fn rest_state_with_sleeping_equals_without_to_epsilon() {
        const N: usize = 4;
        let make = || vertical_stack(N);
        let build = move |bs: &[BodyState]| stack_manifolds(bs, N);
        // Use the default debounce-friendly threshold so the slept arm actually sleeps.
        let off = run_snap(make(), build, 600, false, |c| c.sleep_frames = 30);
        let on = run_snap(make(), build, 600, true, |c| c.sleep_frames = 30);

        const EPS: f32 = 1.0e-2;
        for r in 0..N {
            let dy = (on[r].position.y - off[r].position.y).abs();
            assert!(
                dy < EPS,
                "row {r} rest Y differs sleeping ON vs OFF beyond ε: on={}, off={}, |Δ|={dy}",
                on[r].position.y,
                off[r].position.y
            );
        }
    }

    /// **Gate 3 — run-to-run BIT-determinism on a sleep+WAKE scene.** A scene that
    /// settles → sleeps → is woken by a faller → re-settles, run N independent times,
    /// must produce bit-identical final body snapshots. The in-module tests do not
    /// loop runs — this is the load-bearing determinism gate (every f32 by `to_bits`).
    #[test]
    fn sleep_then_wake_scene_is_run_to_run_bit_deterministic() {
        const N: usize = 4;
        let floor = N as u32;
        let faller = (N + 1) as u32;
        let make = || {
            let mut b = vertical_stack(N);
            b.push(dyn_sphere(Vec3::new(0.0, 40.0, 0.0), 1.0, 0.5, 0.0)); // faller
            b
        };
        let build = move |bs: &[BodyState]| {
            let mut ms = stack_manifolds(bs, N);
            let top = (N - 1) as u32;
            let d = bs[faller as usize].position.y - bs[top as usize].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(top, faller, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[top as usize].position));
            }
            let _ = floor;
            ms
        };

        // One full sleep+wake trajectory: settle (faller parked) → drop faller →
        // re-settle. Returns the final snapshot.
        let trajectory = || {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                sleeping: true,
                sleep_frames: 6,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(N + 2);
            scratch.bodies = make();
            scratch.bodies[faller as usize].inv_mass = 0.0;
            let mut sleep = IslandSleep::with_capacity(N + 2, N + 2);
            // settle phase
            for _ in 0..200 {
                let manifolds = build(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            }
            // wake phase: drop the faller
            scratch.bodies[faller as usize].inv_mass = 1.0;
            scratch.bodies[faller as usize].position.y = scratch.bodies[N - 1].position.y + 2.5;
            for _ in 0..200 {
                let manifolds = build(&scratch.bodies);
                let graph = build_graph(&scratch.bodies, &manifolds);
                scratch.touched.reset(scratch.bodies.len());
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            }
            snap(&scratch.bodies)
        };

        let baseline = trajectory();
        for run_idx in 1..8 {
            let again = trajectory();
            assert!(
                snaps_bit_equal(&baseline, &again),
                "sleep+wake scene was NOT run-to-run bit-deterministic on run {run_idx}"
            );
        }
    }

    /// **Gate 4 — the 0%-gate at SCALE.** With sleeping=false the colored solve must
    /// be BYTE-identical to the pre-O8 colored path (`solve_colored` / `build_columns(None)`)
    /// across the O6/O7 random-scene corpus, not just a 3-body scene. Here: drive each
    /// random scene through `solve_colored_inner(.., None)` (the live path) vs the
    /// explicit `solve_colored` entry; both must produce bit-identical body state. The
    /// stronger claim — sleeping=ON-but-never-sleeps == sleeping=OFF — is also checked
    /// per scene (threshold 0 ⇒ no freeze, so the O8 path must be byte-identical).
    #[test]
    fn zero_gate_at_scale_sleeping_off_byte_identical_on_random_corpus() {
        proptest!(ProptestConfig::with_cases(300), |(seed in any::<u64>())| {
            let (bodies, manifolds, graph) = random_scene(seed);
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                ..PhysicsConfig::default()
            };

            // Arm A: the byte-untouched O6/O7 path (sleep == None).
            let mut solver_a = ColoredSoftStepSolver::default();
            let mut scratch_a = SolverScratch::with_capacity(bodies.len());
            scratch_a.bodies = bodies.clone();
            scratch_a.touched.reset(scratch_a.bodies.len());
            solver_a.solve_colored(&cfg, &manifolds, &graph, &mut scratch_a);
            let after_off = snap(&scratch_a.bodies);

            // Arm B: the O8 path with threshold 0 (nothing can sleep ⇒ no freeze) —
            // must be byte-identical to arm A (sleeping bookkeeping changes nothing).
            let cfg_on = PhysicsConfig {
                sleeping: true,
                sleep_threshold: 0.0,
                ..cfg
            };
            let mut solver_b = ColoredSoftStepSolver::default();
            let mut scratch_b = SolverScratch::with_capacity(bodies.len());
            scratch_b.bodies = bodies.clone();
            scratch_b.touched.reset(scratch_b.bodies.len());
            let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
            solver_b.solve_colored_sleeping(&cfg_on, &manifolds, &graph, &mut scratch_b, &mut sleep);
            let after_on = snap(&scratch_b.bodies);

            prop_assert!(
                snaps_bit_equal(&after_off, &after_on),
                "0%-gate at scale FAILED for seed {}: sleeping-off result != sleeping-on-but-never-sleeps",
                seed
            );
        });
    }

    /// **Gate 5 — no wake/sleep oscillation.** A body hovering near the threshold must
    /// not flap asleep/awake every frame; the integer debounce must hold. A proptest
    /// over near-threshold per-island energies: drive `begin_step`/`end_step` directly
    /// with a synthetic body velocity sampled around `sleep_threshold` and count latch
    /// TRANSITIONS over many frames — the count must be bounded (no per-frame flapping).
    #[test]
    fn no_sleep_wake_oscillation_near_threshold() {
        proptest!(ProptestConfig::with_cases(200), |(
            speed_bits in 0u32..=40u32,    // index into a near-threshold speed table
            frames_seed in any::<u64>(),
        )| {
            // A single dynamic body, no contacts, in its own singleton island so its
            // island energy is exactly its own |v|².
            let threshold = DEFAULT_SLEEP_THRESHOLD; // 1e-4
            let debounce: u16 = 8;
            // A speed² straddling the threshold: below for `speed_bits` even, above for
            // odd — deterministically alternating around the boundary to bait flapping.
            let base = threshold * 0.5; // safely below
            let above = threshold * 2.0; // safely above
            let mut rng = Lcg(frames_seed ^ (speed_bits as u64));

            let body = dyn_sphere(Vec3::new(0.0, 5.0, 0.0), 1.0, 0.5, 0.0);
            // Single-row graph: a manifold to a (added) static floor so the dyn body
            // forms an island. Use a 2-body world (dyn + static) and one contact.
            let bodies = vec![body, static_body(Vec3::ZERO)];
            let ms = vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.01, Vec3::ZERO)];
            let graph = build_graph(&bodies, &ms);
            let mut sleep = IslandSleep::with_capacity(2, 2);

            // Manually feed end_step a body whose speed² we control, then begin_step,
            // and count how many times the row's latch CHANGES state across frames.
            let mut bs = bodies.clone();
            let mut transitions = 0usize;
            let mut prev_asleep = false;
            for f in 0..200 {
                sleep.begin_step(&graph, bs.len());
                // Choose this frame's speed: a low-bias random walk that mostly stays
                // below threshold but occasionally pops above (the near-threshold case).
                let pop = rng.f01() < 0.15; // 15% of frames spike above threshold
                let v2 = if pop { above } else { base * rng.f01().max(0.01) };
                let speed = v2.sqrt();
                bs[0].linear_velocity = Vec3::new(speed, 0.0, 0.0);
                bs[0].angular_velocity = Vec3::ZERO;
                sleep.end_step(&bs, &graph, threshold, debounce);
                let now = sleep.is_row_asleep(0);
                if f > 0 && now != prev_asleep {
                    transitions += 1;
                }
                prev_asleep = now;
            }
            // With a debounce of 8 frames a body cannot flap each frame: every
            // sleep→wake costs 1 frame (an above-threshold spike) and every wake→sleep
            // costs ≥ debounce frames. Over 200 frames with ~15% spikes the transition
            // count must be far below the no-debounce worst case (~200). A debounce that
            // works keeps it bounded by roughly 2× the number of spike clusters.
            prop_assert!(
                transitions <= 60,
                "near-threshold latch oscillated {} times over 200 frames (debounce broken)",
                transitions
            );
        });
    }

    /// **Gate 5b — a body steadily AT rest (just below threshold every frame) latches
    /// exactly once and never flaps.** The clean no-oscillation case: 0 transitions
    /// after the single sleep latch.
    #[test]
    fn steady_below_threshold_latches_once_no_flap() {
        let bodies = vec![dyn_sphere(Vec3::new(0.0, 5.0, 0.0), 1.0, 0.5, 0.0), static_body(Vec3::ZERO)];
        let ms = vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.01, Vec3::ZERO)];
        let graph = build_graph(&bodies, &ms);
        let mut sleep = IslandSleep::with_capacity(2, 2);
        let threshold = DEFAULT_SLEEP_THRESHOLD;
        let debounce: u16 = 8;

        let mut bs = bodies.clone();
        bs[0].linear_velocity = Vec3::ZERO; // exactly at rest, always below threshold
        let mut transitions = 0usize;
        let mut prev = false;
        for f in 0..100 {
            sleep.begin_step(&graph, bs.len());
            sleep.end_step(&bs, &graph, threshold, debounce);
            let now = sleep.is_row_asleep(0);
            if f > 0 && now != prev {
                transitions += 1;
            }
            prev = now;
        }
        assert_eq!(transitions, 1, "a steadily-resting body must latch asleep exactly ONCE (no flap)");
        assert!(sleep.is_row_asleep(0), "the resting body must end latched asleep");
    }

    /// **Gate 9 probe — does a dense resting pile actually latch asleep, and after
    /// how many frames?** This mirrors the criterion `sleeping` bench's `pile_scene`
    /// (a grid of sphere columns on a floor with vertical + lateral contacts) and
    /// reports the slept-row fraction over time. If a dense, lateral-contact pile
    /// does NOT sleep, the bench's `mostly_settled_sleeping_on` arm measures the
    /// AWAKE path (no skip) and the headline-win claim is vacuous — so this is the
    /// load-bearing diagnostic behind the criterion result.
    #[test]
    fn dense_resting_pile_sleeps_diagnostic() {
        // A small pile (4 columns × 3 high) — the chromatic shape of the bench scene
        // at a Miri/native-cheap size.
        let n_columns = 4u32;
        let height = 3u32;
        let n_dyn = (n_columns * height) as usize;
        let mut bodies: Vec<BodyState> = Vec::with_capacity(n_dyn + 1);
        for col in 0..n_columns {
            for h in 0..height {
                let x = col as f32 * 1.05;
                let y = 0.5 + h as f32 * 0.99;
                bodies.push(dyn_sphere(Vec3::new(x, y, 0.0), 1.0, 0.5, 0.0));
            }
        }
        let floor_row = n_dyn as u32;
        bodies.push(static_body(Vec3::new(0.0, -50.0, 0.0)));

        // The bench's FIXED-anchor manifolds: contacts are re-emitted every step at the
        // ORIGINAL rest anchors regardless of how the bodies move (the bench reuses one
        // prebuilt manifold set + graph — it does NOT re-run narrowphase).
        let row_of = |col: u32, h: u32| col * height + h;
        let mut fixed = Vec::new();
        for col in 0..n_columns {
            for h in 0..height {
                let r = row_of(col, h);
                if h == 0 {
                    fixed.push(manifold(r, floor_row, Vec3::new(0.0, -1.0, 0.0), -0.001, bodies[r as usize].position));
                } else {
                    let below = row_of(col, h - 1);
                    fixed.push(manifold(below, r, Vec3::new(0.0, 1.0, 0.0), -0.001, bodies[r as usize].position));
                }
                if col + 1 < n_columns {
                    let right = row_of(col + 1, h);
                    fixed.push(manifold(r, right, Vec3::new(1.0, 0.0, 0.0), -0.001, bodies[r as usize].position));
                }
            }
        }
        let graph = build_graph(&bodies, &fixed);

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 4,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.bodies = bodies;
        let mut sleep = IslandSleep::with_capacity(scratch.bodies.len(), scratch.bodies.len());

        let mut first_all_asleep = None;
        for frame in 0..400 {
            scratch.touched.reset(scratch.bodies.len());
            solver.solve_colored_sleeping(&cfg, &fixed, &graph, &mut scratch, &mut sleep);
            let asleep = (0..n_dyn).filter(|&r| sleep.is_row_asleep(r)).count();
            if asleep == n_dyn && first_all_asleep.is_none() {
                first_all_asleep = Some(frame);
            }
        }
        let asleep_final = (0..n_dyn).filter(|&r| sleep.is_row_asleep(r)).count();
        eprintln!(
            "dense_pile_diagnostic: {asleep_final}/{n_dyn} rows asleep after 400 frames; \
             first all-asleep frame = {first_all_asleep:?}"
        );
        // The diagnostic gate: a dense resting pile MUST eventually sleep, else the
        // bench measures the awake path. (If this fails, the criterion headline-win
        // arm is vacuous — report the slept-row count.)
        assert_eq!(
            asleep_final, n_dyn,
            "a dense resting pile did not fully sleep ({asleep_final}/{n_dyn}); the criterion \
             mostly-settled arm would measure the AWAKE path"
        );
    }

    /// **Gate 7 — topology-change no-spurious-freeze (the C3 regression gate, at
    /// scale).** Random merge/split sequences: a body that should be active is never
    /// frozen because of a stale latch. The row-keyed latch must survive island
    /// renumbering. Drives `begin_step` over a sequence of random manifold sets over a
    /// fixed body set, latching/waking rows, and asserts the freeze decision is ALWAYS
    /// a pure function of the per-row latch — an island is frozen IFF every member row
    /// is latched, never otherwise.
    #[test]
    fn topology_churn_freeze_is_pure_function_of_row_latch() {
        proptest!(ProptestConfig::with_cases(300), |(seed in any::<u64>())| {
            let mut rng = Lcg(seed ^ 0x5DEE_CE66_D1CE_4B27);
            // A fixed set of dynamic bodies that we re-island with random manifolds.
            let n_dyn = rng.range(2, 9) as usize; // 2..=8 dynamic bodies
            let mut bodies: Vec<BodyState> = (0..n_dyn)
                .map(|i| dyn_sphere(Vec3::new(i as f32 * 0.3, 1.0, 0.0), 1.0, 0.5, 0.0))
                .collect();
            bodies.push(static_body(Vec3::ZERO));
            let n_rows = bodies.len();

            let mut sleep = IslandSleep::with_capacity(n_rows, n_rows);

            // Run several frames; each frame re-derive a random manifold set (random
            // merges/splits), randomly latch/wake rows via the energy path, then assert
            // the per-island freeze decision matches the pure predicate over the rows.
            for _frame in 0..20 {
                // Random contact set over the dynamic bodies (random merges/splits).
                let n_contacts = rng.range(0, (n_dyn * 2) as u32) as usize;
                let mut ms = Vec::with_capacity(n_contacts);
                for _ in 0..n_contacts {
                    let a = rng.range(0, n_dyn as u32);
                    let mut b = rng.range(0, n_dyn as u32);
                    if b == a {
                        b = (a + 1) % n_dyn as u32;
                    }
                    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                    ms.push(manifold(lo, hi, Vec3::new(1.0, 0.0, 0.0), -0.01, bodies[lo as usize].position));
                }
                let graph = build_graph(&bodies, &ms);
                sleep.begin_step(&graph, n_rows);

                // The per-island freeze decision must be EXACTLY: island frozen iff
                // every member dynamic row is latched asleep (and the island is non-empty).
                let n_islands = graph.n_islands() as usize;
                let mut member_count = vec![0usize; n_islands];
                let mut all_asleep = vec![true; n_islands];
                for row in 0..n_dyn {
                    let isl = graph.island_of(row as u32);
                    if isl == ConstraintGraph::NO_ISLAND {
                        continue;
                    }
                    member_count[isl as usize] += 1;
                    if !sleep.is_row_asleep(row) {
                        all_asleep[isl as usize] = false;
                    }
                }
                for isl in 0..n_islands {
                    let expect_frozen = member_count[isl] > 0 && all_asleep[isl];
                    // An island with no members is `frozen_islands[isl] == true` by the
                    // resize default but has no rows, so no row reports awake/frozen via it.
                    if member_count[isl] > 0 {
                        prop_assert_eq!(
                            sleep.is_island_frozen(isl as u32),
                            expect_frozen,
                            "spurious/missing freeze for island {} on frame {} (seed {}): \
                             members={}, all_asleep={}",
                            isl, _frame, seed, member_count[isl], all_asleep[isl]
                        );
                    }
                    // Every awake row's island must NOT be frozen (C3): no member of an
                    // active partition is frozen because of a stale latch.
                    for row in 0..n_dyn {
                        if graph.island_of(row as u32) == isl as u32 && !sleep.is_row_asleep(row) {
                            prop_assert!(
                                sleep.is_row_awake(row),
                                "C3: awake row {} was frozen by a stale latch (seed {}, frame {})",
                                row, seed, _frame
                            );
                        }
                    }
                }

                // Randomly latch / wake some rows for the next frame (drive churn).
                for row in 0..n_dyn {
                    if rng.f01() < 0.5 {
                        sleep.force_sleep_row(row);
                    }
                }
            }
        });
    }
}
