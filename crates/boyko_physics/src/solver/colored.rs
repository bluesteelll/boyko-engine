//! The colored TGS-Soft solver (Phase O5, Decision 7) — a SEPARATE
//! [`RigidSolver`] that solves contacts in graph-COLOR order over the
//! cohort-shaped constraint tables of [`CohortColumns`] (L11 C2): every 8-group
//! cohort of a color is one aligned head of per-manifold constants plus one
//! block per rank of per-point data, the layout the [O7] AVX2 kernel loads as
//! vectors, the scalar oracle walks lane by lane, and the [O6] parallel
//! dispatch cuts by group exactly as it cut the per-point columns before.
//!
//! [O6]: https://github.com/bluesteelll/boyko-engine
//! [O7]: https://github.com/bluesteelll/boyko-engine
//!
//! # Why a separate solver (Decision 7)
//!
//! The reference [`SoftStepSolver`](super::SoftStepSolver) — its AoS
//! `PointConstraint` layout and the manifold-order `solve_velocities`
//! Gauss-Seidel sweep — is **byte-untouched** and stays in the tree as the
//! reference oracle. Since 2026-09-18 (owner decision) THIS solver is the default
//! world's ([`DefaultRigidSolver`](super::DefaultRigidSolver)), with the O7 AVX2
//! cohort kernel on (`PhysicsConfig::simd_solve`). Every `add_physics_*::<S>`
//! entry wires it when `S` is this type: the constraint graph plus a distinct
//! [`physics_solve_colored`](crate::systems::physics_solve_colored) stage that
//! stands in for the generic solve. A world wired with the reference solver is
//! unaffected.
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
//! [`group_start`](CohortColumns::group_start) +
//! [`color_group_start`](CohortColumns::color_group_start) CSR records the
//! per-manifold group boundaries so a consumer can enumerate, per color, the
//! groups and each group's slot range. (O5 is single-threaded; it solves a
//! color's contiguous SoA span slot-by-slot in index order, which absorbs the
//! intra-group coupling for free — the group CSR is additive data O5 does not
//! consume.)
//!
//! # The warm store: one record per manifold, in stream order (L11 C1)
//!
//! The velocity accumulation is order-independent within a color, and so is the
//! warm store: each manifold's converged impulses go to `recs[mi]` of the write
//! side of a double-buffered [`WarmRecords`] — a write by the manifold index, whose
//! layout is a pure function of the stream and never of the color layout or (in
//! [O6]) the thread count. The next step finds each manifold's source record by
//! a merge-join over the previous stream's ordinals (`warm_records::plan_sources`,
//! D2) and matches points by feature id inside the record, through the one lookup
//! routine the B1 carry also uses (`WarmLookup::seed`, ruling W1). No hash, no
//! per-point key, no table refill. `warm_records.rs` states Lemma W, the argument
//! that every seed and hit count equals the per-point table's it replaced.

use std::marker::PhantomData;
use std::ptr;

use boyko_diag::{zone, zone_enabled};
use boyko_ecs::ecs::core::component::scratch::{ScratchColumn, ScratchSolveView};
use boyko_macros::Resource as ResourceDerive;
use boyko_threadpool::try_with_active_pool;

use super::contact::{
    BodyEffective, effective_inv_mass, effective_mass, is_dynamic_row, tangent_basis,
};
use super::simd;
// O2: the soft constants, the immovable-surface view, and the soft-coefficient
// derivation are SHARED from the reference solver — a single source of truth so
// the colored kernel cannot drift from `soft_step.rs` (the byte-untouched
// 0%-gate reference). These are `pub(crate)` re-uses (visibility-widened only,
// no value/layout change to `soft_step.rs`).
use super::soft_step::{IMMOVABLE_AT_REST, MAX_BIAS_VELOCITY, RESTITUTION_THRESHOLD, SoftCoefficients};
use super::warm_records::{
    self, WarmIndex, WarmLookup, WarmRecord, WarmRecords, WarmRun, ord, point_fid,
};
use super::RigidSolver;
use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::math::{Mat3, Vec3};
use crate::profiling::{
    PHYS_COLOR_NARROW, PHYS_COLOR_WIDE, PHYS_GRAVITY, PHYS_INTEGRATE, PHYS_PASS_BIASED,
    PHYS_PASS_RELAX, PHYS_RESTITUTION, PHYS_SLEEP_BEGIN, PHYS_SLEEP_END, PHYS_SLEEP_FREEZE,
    PHYS_SLOTS_NARROW, PHYS_SLOTS_WIDE, PHYS_SOLVE_BUILD, PHYS_STORE, PHYS_WARM_APPLY,
    PHYS_WRITE_BACK, counter,
};
use crate::resources::{
    BodyState, ConstraintGraph, IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
};
use crate::row_identity::{RemapCursor, RowIdentity, RowRemap, WarmSeedStats};
use crate::scratch_ids::{
    body_eff_colored_id, colored_frozen_rows_id, contact_column_id, register_scratch_layouts,
    scratch_reserve_rows, warm_table_id,
};
use crate::sleep_sets::{HeldSolve, RowCls, SleepSets};

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

/// Loads one aligned `[f32; 8]` row of a cohort head or rank block into a
/// `__m256` (the O7 cohort kernel's aligned row load, L11 C2).
///
/// # Safety
///
/// AVX2-gated (the `cfg` + `target_feature`); `row` must point at a live,
/// 32 B-aligned `[f32; 8]` — every row of a [`CohortHead`] / [`RankBlock`] is, by
/// the structs' `align(64)` layout — that no other thread writes concurrently.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
unsafe fn load_row(row: *const [f32; COHORT]) -> core::arch::x86_64::__m256 {
    use core::arch::x86_64::_mm256_load_ps;
    debug_assert!((row as usize).is_multiple_of(32), "invariant: an aligned row load");
    // SAFETY: the caller passes a live, 32 B-aligned row; the aligned load reads its
    //   8 `f32`.
    unsafe { _mm256_load_ps(row.cast::<f32>()) }
}

/// Loads three aligned `[f32; 8]` rows (a `Vec3` per lane) into a `[__m256; 3]`
/// register triple.
///
/// # Safety
///
/// As [`load_row`]: AVX2-gated; `rows` is a live, 32 B-aligned `[[f32; 8]; 3]`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
unsafe fn load_rows3(rows: *const [[f32; COHORT]; 3]) -> [core::arch::x86_64::__m256; 3] {
    // SAFETY: each row of a live `[[f32; 8]; 3]` is a live, 32 B-aligned row.
    unsafe {
        [
            load_row(&raw const (*rows)[0]),
            load_row(&raw const (*rows)[1]),
            load_row(&raw const (*rows)[2]),
        ]
    }
}

/// Stores a `__m256` into one aligned `[f32; 8]` row of a rank block (the O7
/// cohort kernel's impulse store).
///
/// # Safety
///
/// AVX2-gated; `row` must point at a live, 32 B-aligned `[f32; 8]` this thread
/// owns (a SIMD worker's cuts are whole cohorts, so no other worker touches it).
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
unsafe fn store_row(row: *mut [f32; COHORT], reg: core::arch::x86_64::__m256) {
    use core::arch::x86_64::_mm256_store_ps;
    debug_assert!((row as usize).is_multiple_of(32), "invariant: an aligned row store");
    // SAFETY: the caller passes a live, 32 B-aligned, exclusively-owned row; the
    //   aligned store writes its 8 `f32`.
    unsafe { _mm256_store_ps(row.cast::<f32>(), reg) }
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
///
/// `pub(crate)` for one reader outside this file: the profiling zones class a color as
/// wide or narrow by this same floor ([`crate::profiling::WIDE_COLOR_MIN_SLOTS`]).
pub(crate) const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;

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

/// Minimum SLOTS a dispatched chunk must carry, so a chunk pays for its own
/// boxed closure.
///
/// ⚠ WITHOUT THIS, MORE WORKERS MAKE THE STEP SLOWER, AND THAT IS MEASURED. The
/// chunk count was `lanes * CHUNKS_PER_WORKER` clamped only by the GROUP count —
/// a function of the machine and of the partition, and of nothing about the work
/// inside a chunk. A color that just clears
/// [`MIN_PARALLEL_SLOTS_PER_COLOR`] (256 slots) therefore split into
/// `16 * 6 = 96` chunks on a 16-lane pool: about 2.7 slots of work per boxed
/// closure. Adding lanes added allocations against fixed work, so the Jolt-parity
/// pyramid measured 0.82x at 8 workers and 0.56x at 16 — NEGATIVE scaling.
///
/// The two existing constants do not cover this. `MIN_PARALLEL_SLOTS_PER_COLOR`
/// decides WHETHER a color dispatches at all; nothing decided HOW FINELY it was
/// then cut. The kernel's own row driver has had exactly this floor since it was
/// written (`BatchingStrategy::min_batch_size`) — this is the physics dispatcher
/// catching up with it.
///
/// Bit-identity is unaffected: the `{1, N}` property holds for ANY partition
/// (distinct chunks touch disjoint dynamic bodies, and the warm store is a write by
/// manifold index — a pure function of the stream, never of the partition), so this
/// changes only WHERE work runs, never the values.
///
/// # ⚠ The value is 64 because 256 was MEASURED to be better on one scene and to
/// # BREAK another — a single global constant cannot serve both
///
/// Swept on the Jolt-parity pyramid (`benches/jolt_parity_pyramid.rs`), W = 8 / 16:
///
/// | floor |  W8 ms | W16 ms |
/// |------:|-------:|-------:|
/// |    32 |  22.87 |  29.67 |
/// |    64 |  21.60 |  26.99 |
/// |   128 |  20.93 |  25.13 |
/// |   256 |  20.14 |  20.98 |
///
/// Monotone, still descending at 256 — so on that scene alone the answer is
/// "raise it". Raising it to 256 turns
/// `many_disjoint_pairs_are_one_wide_color_and_must_dispatch` RED: that scene is
/// ONE colour of 500 slots, `500 / 256 == 1` chunk, and the single-chunk
/// short-circuit below then sends it inline — undoing the whole measured 1.95x
/// win of the P2 gate-metric fix.
///
/// The two scenes want opposite things and both are legitimate. A pyramid is many
/// NARROW colours, so its cost is per-wave dispatch and it wants coarse chunks; a
/// debris pile is ONE WIDE colour, so its cost is idle lanes and it wants fine
/// ones. No single integer is right for both, which is the concrete case for
/// making the cut policy a per-call-site OBJECT rather than a global constant —
/// argued elsewhere in this campaign, measured here.
///
/// 64 is therefore chosen as the largest value that keeps BOTH gates green, not
/// as the optimum of either. It captures most of the pyramid win (26.99 vs 35.26
/// ms unfloored at W = 16) without refusing the wide-colour dispatch.
const MIN_SLOTS_PER_CHUNK: usize = 64;

/// O7 SIMD cohort width: the number of body-disjoint manifold-GROUPS packed into
/// one AVX2 batch (one group per lane = 8 lanes per `__m256`).
///
/// Under the parallel + SIMD path the chunk boundaries are SNAPPED to multiples of
/// this (Decision 7) so every dispatched task solves only full-width cohorts (the
/// last cohort of a COLOR may be partial — the masked kernel handles it). This is a
/// fixed SIMD-width constant, NOT a perf knob: it MUST equal the AVX2 lane count
/// the [`solve_color_avx2`](ColoredSoftStepSolver::solve_color_avx2) kernel uses.
const COHORT: usize = 8;

/// The cohort-shaped constraint layout (L11 D5): today's cohorts, materialised.
///
/// A cohort is an 8-group window counted from a color's first group — the same
/// window every path has walked since O7 (the serial loop, the inline path and the
/// `COHORT`-stepped parallel cuts), so cohort membership never depends on the
/// worker count. C2 stores each cohort as one [`CohortHead`] (the eight lanes'
/// per-manifold constants) plus one [`RankBlock`] per rank (the eight lanes'
/// point-`r` data), with a cold [`CohortCold`] and a cold `vn0` row per rank
/// beside them. The AVX2 kernel loads a rank as ten aligned vectors instead of
/// gathering 160 scalars; the scalar oracle, the warm apply, the restitution pass
/// and the store walk the same blocks lane by lane in group-major order, which is
/// the slot order the per-point columns had. Padding lanes (`lane >= nlanes`) and
/// the ranks past a lane's width are zero in every field, so the layout bytes are
/// a pure function of the stream (G3).
///
/// Every reader indexes by `(cohort, lane, rank)`: group `g` of color `c` is lane
/// `(g - color_group_start[c]) % 8` of cohort `color_cohort_start[c] + (g -
/// color_group_start[c]) / 8` ([`ColorCtx`]).
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CohortHead {
    /// Per-lane contact normal (A → B), three SoA rows.
    n: [[f32; COHORT]; 3],
    /// Per-lane first friction tangent, three SoA rows. The second tangent is derived
    /// where it is read as `n × t1` — the same op [`tangent_basis`] uses.
    t1: [[f32; COHORT]; 3],
    /// Per-lane combined friction coefficient (`max(µa, µb)`, the reference rule).
    friction: [f32; COHORT],
    /// Per-lane dense body-A row.
    body_a: [u32; COHORT],
    /// Per-lane dense body-B row; `== body_a` on a sentinel lane, whose body B is
    /// [`IMMOVABLE_AT_REST`] and never a row.
    body_b: [u32; COHORT],
    /// Per-lane live point count; `0` on a padding lane.
    width: [u8; COHORT],
    /// Index of the cohort's first [`RankBlock`] (and its first `vn0` row).
    rank_base: u32,
    /// The cohort's rank count: the widest lane's point count.
    depth: u8,
    /// Live lanes, `1..=8`.
    nlanes: u8,
    /// Bit `lane` set on an SDF lane (body B immovable); clear on a padding lane.
    sentinel: u8,
    /// Zero.
    _p: u8,
    /// Zero, to the fifth line.
    _pad: [u8; 16],
}

const _: () = assert!(
    size_of::<CohortHead>() == 320 && align_of::<CohortHead>() == 64,
    "a cohort head is exactly five cache lines"
);

impl CohortHead {
    /// A head with every field zero: the value a padding cohort would hold.
    const ZERO: Self = Self {
        n: [[0.0; COHORT]; 3],
        t1: [[0.0; COHORT]; 3],
        friction: [0.0; COHORT],
        body_a: [0; COHORT],
        body_b: [0; COHORT],
        width: [0; COHORT],
        rank_base: 0,
        depth: 0,
        nlanes: 0,
        sentinel: 0,
        _p: 0,
        _pad: [0; 16],
    };

    /// Lane `l`'s contact normal.
    #[inline]
    fn normal(&self, l: usize) -> Vec3 {
        Vec3::new(self.n[0][l], self.n[1][l], self.n[2][l])
    }

    /// Lane `l`'s first friction tangent.
    #[inline]
    fn tangent1(&self, l: usize) -> Vec3 {
        Vec3::new(self.t1[0][l], self.t1[1][l], self.t1[2][l])
    }

    /// Whether lane `l` is an SDF lane (body B immovable, never a row).
    #[inline]
    fn is_sentinel(&self, l: usize) -> bool {
        (self.sentinel >> l) & 1 != 0
    }
}

/// One cohort's cold per-lane data (D5): read once per step by the restitution
/// pass and the store, never by a sweep. 64 B.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CohortCold {
    /// Per-lane combined restitution coefficient (`max(ea, eb)`).
    restitution: [f32; COHORT],
    /// Per-lane manifold index: the store's record index. `0` on a padding lane.
    mi: [u32; COHORT],
}

const _: () = assert!(
    size_of::<CohortCold>() == 64 && align_of::<CohortCold>() == 64,
    "a cohort's cold data is one cache line"
);

impl CohortCold {
    /// Every field zero.
    const ZERO: Self = Self {
        restitution: [0.0; COHORT],
        mi: [0; COHORT],
    };
}

/// One rank of one cohort (D5): the eight lanes' point-`r` anchors, separation and
/// accumulated impulses, each an aligned `[f32; 8]` row. 320 B, one owner per
/// cohort under the SIMD kernel (which writes the three impulse rows as vectors);
/// scalar workers may share a block and then write distinct lane elements through
/// raw projections. Padding lanes and the ranks past a lane's width are zero.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RankBlock {
    /// Body-A anchor offset (world frame), three SoA rows.
    ra: [[f32; COHORT]; 3],
    /// Body-B anchor offset (world frame), three SoA rows; zero on a sentinel lane.
    rb: [[f32; COHORT]; 3],
    /// Signed separation at gather time (negative = penetrating).
    sep: [f32; COHORT],
    /// Accumulated normal impulse `λn ≥ 0` (warm-seeded).
    ni: [f32; COHORT],
    /// Accumulated tangent impulse along `t1` (warm-seeded).
    ti1: [f32; COHORT],
    /// Accumulated tangent impulse along `t2` (warm-seeded).
    ti2: [f32; COHORT],
}

const _: () = assert!(
    size_of::<RankBlock>() == 320 && align_of::<RankBlock>() == 64,
    "a rank block is exactly five cache lines"
);

impl RankBlock {
    /// Every field zero.
    const ZERO: Self = Self {
        ra: [[0.0; COHORT]; 3],
        rb: [[0.0; COHORT]; 3],
        sep: [0.0; COHORT],
        ni: [0.0; COHORT],
        ti1: [0.0; COHORT],
        ti2: [0.0; COHORT],
    };

    /// Lane `l`'s body-A anchor.
    #[inline]
    fn ra(&self, l: usize) -> Vec3 {
        Vec3::new(self.ra[0][l], self.ra[1][l], self.ra[2][l])
    }

    /// Lane `l`'s body-B anchor.
    #[inline]
    fn rb(&self, l: usize) -> Vec3 {
        Vec3::new(self.rb[0][l], self.rb[1][l], self.rb[2][l])
    }

    /// Zeroes lane `l` of every row.
    #[inline]
    fn clear_lane(&mut self, l: usize) {
        for c in 0..3 {
            self.ra[c][l] = 0.0;
            self.rb[c][l] = 0.0;
        }
        self.sep[l] = 0.0;
        self.ni[l] = 0.0;
        self.ti1[l] = 0.0;
        self.ti2[l] = 0.0;
    }
}

/// A manifold's part in the step (P-a of D4): its live point count, and whether its
/// island is frozen this step (B1). Written in stream order, read by the layout
/// pass in color order and by the store's second pass in manifold order. A manifold
/// is laid out — solved — iff it is not frozen and has a live point.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManifoldTag {
    /// Live points, `0..=MAX_CONTACT_POINTS`.
    count: u8,
    /// [`Self::FROZEN`] and [`Self::SRC_RESTORE`], or zero.
    flags: u8,
}

impl ManifoldTag {
    /// The manifold's island is frozen this step: not solved, its record carried.
    const FROZEN: u8 = 1;
    /// L10 (design 08 A3′, the plan's E1): the manifold's warm run names positions of the
    /// restore source — the kept records of the islands restored this step — not of the read
    /// side. Set by P-a's search; the P-c seed and the D3 carry pick their source by it.
    const SRC_RESTORE: u8 = 1 << 1;

    /// Whether the manifold is frozen this step.
    #[inline]
    fn frozen(self) -> bool {
        self.flags & Self::FROZEN != 0
    }

    /// Whether the manifold's warm run lies in L10's restore source.
    #[inline]
    fn src_restore(self) -> bool {
        self.flags & Self::SRC_RESTORE != 0
    }

    /// Whether the manifold was laid out into a cohort and solved.
    #[inline]
    fn solved(self) -> bool {
        !self.frozen() && self.count != 0
    }
}

/// A step's warm sources as the P-c seed and the D3 carry read them: every manifold's run
/// (`plan`), and the lookup its tag names — the read side, or L10's restore source for a
/// manifold P-a marked [`SRC_RESTORE`](ManifoldTag::SRC_RESTORE) (design 08 A3′, the plan's
/// E1). One branch per manifold, none per point.
#[derive(Clone, Copy)]
struct WarmSources<'a> {
    /// The read side's lookup.
    lookup: WarmLookup<'a>,
    /// The restore source's lookup, on a step that has one.
    restore: Option<WarmLookup<'a>>,
    /// Every manifold's run.
    plan: &'a [WarmRun],
    /// Every manifold's tag.
    tags: &'a [ManifoldTag],
}

impl<'a> WarmSources<'a> {
    /// Manifold `mi`'s lookup and run.
    #[inline]
    fn of(self, mi: usize) -> (WarmLookup<'a>, WarmRun) {
        let lookup = match self.restore {
            Some(restore) if self.tags[mi].src_restore() => restore,
            _ => self.lookup,
        };
        (lookup, self.plan[mi])
    }
}

/// A color's place in the group and cohort tables: its first group index and its
/// first cohort index. Group `g` of the color is lane `(g - g_base) % 8` of cohort
/// `k_base + (g - g_base) / 8` — today's cohort, an 8-group window from the color's
/// first group, on every path.
#[derive(Clone, Copy, Debug)]
struct ColorCtx {
    /// The color's first group (`color_group_start[c]`).
    g_base: usize,
    /// The color's first cohort (`color_cohort_start[c]`).
    k_base: usize,
}

impl ColorCtx {
    /// The `(cohort, lane)` of group `g` of this color.
    #[inline]
    fn lane_of(self, g: usize) -> (usize, usize) {
        let d = g - self.g_base;
        (self.k_base + d / COHORT, d % COHORT)
    }

    /// The cohort range `[k_lo, k_hi)` of the cohort-aligned group range `[g_lo,
    /// g_hi)` of this color: `g_lo` is a cohort boundary (the color's first group,
    /// or a `COHORT`-stepped cut from it), and `g_hi` is one or the color's end.
    #[inline]
    fn cohorts_of(self, g_lo: usize, g_hi: usize) -> (usize, usize) {
        debug_assert!(
            (g_lo - self.g_base).is_multiple_of(COHORT),
            "invariant: a SIMD group range starts on a cohort boundary"
        );
        (
            self.k_base + (g_lo - self.g_base) / COHORT,
            self.k_base + (g_hi - self.g_base).div_ceil(COHORT),
        )
    }
}

/// The colored solver's anti-vacuity counters (L11 C0): what the G1 identity gate
/// reads to show that a pinned scene exercised the path it claims to. Crate-internal;
/// outside `cfg(test)` this is a zero-sized type whose methods are empty, so the
/// step carries no counting cost in a shipping build (the `WalkCounters` pattern in
/// `row_identity.rs`: one impl whose counting statements are `#[cfg(test)]`-gated,
/// never a `not(test)` predicate).
///
/// Every count is cumulative over the solver's lifetime; a scene asserts a delta over
/// its steps. `backward_searches` and `cold_index_steps` are D2's own counts (C1):
/// what `warm_records::plan_sources` reports of the merge-join it ran.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SetupCounters {
    /// Points of frozen manifolds whose record the store carried (B1 hits): the sum of
    /// [`WarmSeedStats::carry_hits`] over the steps.
    #[cfg(test)]
    pub(crate) carry_hits: u64,
    /// Lookups on a strict read side that the cursor could not gallop forward to (a
    /// descent of the translated ordinals), served by a backward binary search.
    #[cfg(test)]
    pub(crate) backward_searches: u64,
    /// Per-point lookups that found no stored point: an untranslatable pair (a flipped
    /// order, a `Reset`), a pair the previous stream did not hold, or a feature id its
    /// record does not carry.
    #[cfg(test)]
    pub(crate) misses: u64,
    /// Steps that looked up a read side whose ordinals were NOT strictly increasing
    /// (an unsorted stream, or a duplicate pair) and built D2's cold sorted index.
    #[cfg(test)]
    pub(crate) cold_index_steps: u64,
    /// Manifolds met with two live points sharing a (masked) feature id, solved or
    /// carried: the last-write-wins case Lemma W's in-record scan must reproduce.
    #[cfg(test)]
    pub(crate) duplicate_fids_met: u64,
    /// Solves run with warm start disabled (no lookup, no store, no stamp).
    #[cfg(test)]
    pub(crate) disabled_steps: u64,
    /// Points the restitution pass applied an impulse to (both of its gates passed).
    #[cfg(test)]
    pub(crate) restitution_applied: u64,
}

impl SetupCounters {
    /// Adds the store's carry hits of one step.
    #[inline]
    fn carry(&mut self, hits: u32) {
        #[cfg(test)]
        {
            self.carry_hits += u64::from(hits);
        }
        // The only other reader of `hits` is the gated statement above.
        let _ = hits;
    }

    /// Adds one manifold's per-point lookup misses.
    #[inline]
    fn miss(&mut self, points: u32) {
        #[cfg(test)]
        {
            self.misses += u64::from(points);
        }
        // The only other reader of `points` is the gated statement above.
        let _ = points;
    }

    /// Counts a manifold with two live points sharing a masked feature id.
    #[inline]
    fn duplicate_fids(&mut self, m: &Manifold) {
        #[cfg(test)]
        {
            let n = m.count as usize;
            let dup = (0..n).any(|p| (p + 1..n).any(|q| point_fid(m, p) == point_fid(m, q)));
            self.duplicate_fids_met += u64::from(dup);
        }
        // The only other reader of `m` is the gated statement above.
        let _ = m;
    }

    /// Counts one solve; `warm_start_enabled` false is a disabled step.
    #[inline]
    fn step(&mut self, warm_start_enabled: bool) {
        #[cfg(test)]
        {
            self.disabled_steps += u64::from(!warm_start_enabled);
        }
        // The only other reader of the flag is the gated statement above.
        let _ = warm_start_enabled;
    }

    /// Counts one point the restitution pass applied to.
    #[inline]
    fn restitution(&mut self) {
        #[cfg(test)]
        {
            self.restitution_applied += 1;
        }
    }

    /// Adds what one step's `plan_sources` counted: its backward searches, and whether
    /// it built the cold index.
    #[inline]
    fn plan(&mut self, counts: warm_records::PlanCounts) {
        #[cfg(test)]
        {
            self.backward_searches += u64::from(counts.backward_searches);
            self.cold_index_steps += u64::from(counts.cold_index);
        }
        // The only other reader of `counts` is the gated block above.
        let _ = counts;
    }
}

/// FNV-1a 64 offset basis (the runner's pose-hash function, reused for the C0 digest).
#[cfg(test)]
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64 prime.
#[cfg(test)]
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Folds one `u32` into an FNV-1a 64 hash, byte by byte, little-endian.
#[cfg(test)]
#[inline]
fn fnv_u32(mut h: u64, v: u32) -> u64 {
    for b in v.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Folds one `u64` into an FNV-1a 64 hash, byte by byte, little-endian.
#[cfg(test)]
#[inline]
fn fnv_u64(h: u64, v: u64) -> u64 {
    // Truncation is the point: the low and high halves, in that order.
    fnv_u32(fnv_u32(h, v as u32), (v >> 32) as u32)
}

/// Folds a `u32` slice, length first, into an FNV-1a 64 hash.
#[cfg(test)]
fn fnv_u32s(mut h: u64, vs: &[u32]) -> u64 {
    h = fnv_u32(h, vs.len() as u32);
    for &v in vs {
        h = fnv_u32(h, v);
    }
    h
}

/// Folds a [`WarmSeedStats`] into an FNV-1a 64 hash, field by field in declaration order.
#[cfg(test)]
fn fnv_warm_stats(mut h: u64, s: &WarmSeedStats) -> u64 {
    for v in [
        s.manifolds,
        s.translated,
        s.points,
        s.point_hits,
        s.carry_points,
        s.carry_hits,
    ] {
        h = fnv_u32(h, v);
    }
    fnv_u64(h, s.remap_resets)
}

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
struct ColorSolvePtrs<'a> {
    /// Per-element bodies access via the committed [`ScratchSolveView`] — Copy,
    /// Send+Sync, row-ptr-ONLY. There is NO whole-buffer `&mut [BodyEffective]`
    /// reborrow path (the SP4 structural fix): a worker reaches a body row only
    /// through [`ScratchSolveView::row_ptr`], which yields one typed `*mut
    /// BodyEffective` per DISTINCT index its color owns.
    bodies: ScratchSolveView<'a, BodyEffective>,
    /// The worker-facing cohort solve view (P2; L11 C2): `Copy + Send + Sync`, raw
    /// bases ONLY. This REPLACES the deleted `cols: *mut ContactColumns` +
    /// `columns()` (`&mut *self.cols`) whole-struct reborrow — the rigid
    /// Tree-Borrows race surface. A worker reaches a block element solely through
    /// a raw projection of `view.block(i)` (or, owning the whole cohort, an aligned
    /// row load / store), so no `&mut` ever spans another worker's lane and the
    /// whole-buffer reborrow is un-typeable on the worker path.
    view: CohortSolveView<'a>,
}

// SAFETY: `ColorSolvePtrs` is `Send + Sync` because its only shared-mutable state
//   is reached PER ELEMENT through `view: CohortSolveView` and `bodies:
//   ScratchSolveView`, both of which expose mutation only as a single-lane raw
//   write or an owned cohort's row store (never a whole-buffer `&mut [_]` reborrow
//   — the P2/P1 structural fix). The C2 coloring invariant guarantees two workers
//   in one parallel step never share a lane (distinct groups => distinct lanes =>
//   distinct body pairs), so concurrent impulse writes target disjoint addresses
//   and concurrent reads never alias a concurrent write; a SHARED static body
//   (`inv_mass == 0`) is never written (the `*_movable` guard in `solve_color`),
//   so it is read-only across workers. No `&mut` ever spans more than one element,
//   so no overlapping unique TB protector is ever created. The bases are
//   address-stable (the backing `ComponentPool` reservations never realloc-move)
//   and finalized BEFORE any view is built (the B4 re-create-before-view-live
//   discipline in `solve_colored_inner`), so they cannot dangle while a view is
//   live. The wrapper has no interior mutability, so a shared `&` to it (the outer
//   `pool.scope` closure's capture across the spawn loop) is trivially safe —
//   hence both `Send` (cross-thread move into a task) and `Sync` (shared by the
//   loop) hold.
unsafe impl Send for ColorSolvePtrs<'_> {}
unsafe impl Sync for ColorSolvePtrs<'_> {}

// ── BodyEffective row access through the ScratchSolveView (mirror 1) ─────────────
//
// These three helpers are the ONLY way the colored kernels reach a body row now
// that `bodies` is a `ScratchColumn` (no whole-buffer `&mut [BodyEffective]`). Each
// is one `view.row_ptr(i)` = `base + i*stride` (a typed `*mut BodyEffective`, no
// cast, no bounds compare) followed by a deref — asm-identical to the prior
// `bodies_eff[i]` slice index minus the slice's bounds-check branch (the index `i`
// comes from `cols.body_a/body_b`, so the slice form could not elide its panic
// branch). The SHARED invariant (stated once here, relied on by every call):
//
//   * `i < view.len()` — every `i` is a gathered body row (`body_a[s]`/`body_b[s]`),
//     which `build_columns` validated against the gathered body count, so it indexes
//     a live `[0, len)` element (debug-asserted inside `row_ptr`).
//   * In the PARALLEL path the caller writes only the DISTINCT rows its color owns
//     (the O4 coloring invariant); two workers never derive `&mut` to the same row,
//     so no aliasing across threads. In the SERIAL path there is one thread.
//   * `BodyEffective: Copy` ⇒ no drop glue on the raw bytes.

/// Shared reference to body row `i` via the solve view.
///
/// # Safety contract (caller): see the module-level helper invariant above — `i <
/// len` and no concurrent writer of row `i`.
#[inline]
fn body_ref<'a>(view: ScratchSolveView<'a, BodyEffective>, i: usize) -> &'a BodyEffective {
    // SAFETY: `i < view.len()` (a gathered body row — the build invariant), so
    //   `row_ptr(i)` is the live `i`-th element on the column's address-stable base.
    //   This shared read never coincides with a write of row `i` (serial: one
    //   thread; parallel: the coloring grants row `i` to one worker only).
    unsafe { &*view.row_ptr(i) }
}

/// Copy of body row `i` via the solve view (a non-aliasing value read of a `Copy`).
///
/// # Safety contract (caller): see the module-level helper invariant above.
#[inline]
fn body_copy(view: ScratchSolveView<'_, BodyEffective>, i: usize) -> BodyEffective {
    // SAFETY: as `body_ref` — `i < len`, address-stable base; `BodyEffective: Copy`
    //   so the read is a non-moving byte copy with no drop/aliasing hazard.
    unsafe { *view.row_ptr(i) }
}

/// Exclusive reference to body row `i` via the solve view (the per-element write
/// surface — replaces the deleted whole-buffer `&mut [BodyEffective]`).
///
/// # Safety contract (caller): see the module-level helper invariant above —
/// crucially, NO other worker writes row `i` concurrently (the coloring
/// distinct-index invariant), so the derived `&mut` never aliases another `&mut`.
#[inline]
fn body_mut<'a>(view: ScratchSolveView<'a, BodyEffective>, i: usize) -> &'a mut BodyEffective {
    // SAFETY: `i < view.len()` (a gathered body row); `row_ptr(i)` is the live `i`-th
    //   element on the address-stable base. The caller guarantees row `i` is written
    //   by at most ONE worker (the O4 coloring distinct-index invariant — serial: one
    //   thread), so this `&mut` is unique. `BodyEffective: Copy` ⇒ no drop glue.
    unsafe { &mut *view.row_ptr(i) }
}

/// Worker-facing, color-disjoint cohort solve view (audit Stage P — P2; L11 C2).
///
/// `Copy + Send + Sync`: the scheduler hands a COPY to each parallel worker. It
/// exposes the cohort tables the colored kernels touch on the worker path as raw
/// bases ONLY — a `&CohortHead` per cohort (never written during a solve) and a
/// `*mut RankBlock` per rank block, reached per element through raw projections.
/// There is NO whole-buffer `&mut [_]` / slice path, so the `&mut *self.cols`
/// whole-struct reborrow that caused the rigid Tree-Borrows race is un-typeable
/// from this view (the P2 structural fix, mirroring the body [`ScratchSolveView`]).
///
/// # SAFETY / soundness (`Send + Sync`)
///
/// Soundness rests on the C2 coloring disjointness invariant: any two lanes touched
/// concurrently by distinct workers belong to distinct groups of one color and
/// therefore to distinct dynamic bodies. A SIMD worker owns whole cohorts (its
/// cuts are `COHORT`-stepped), so its aligned vector loads and stores of a block's
/// rows touch no other worker's data. A scalar worker owns whole groups, which may
/// share a cohort with another worker's groups: it reads its lanes' head constants
/// through a shared `&CohortHead` (no worker writes a head during a solve) and
/// reads and writes its lanes' block elements ONLY through raw `&raw` projections
/// (`(*block).ni[lane]`), so no `&`/`&mut` ever spans a block and two workers never
/// materialize overlapping references. The `group_start` CSR is read-only.
///
/// PROVENANCE: every base is a raw write-capable base derived from
/// [`ScratchColumn::solve_base`] (i.e. `ComponentPool::buffer_ptr().cast_mut()`,
/// provenance-preserving, NO `&[_]` interposed). The block base therefore carries
/// WRITE provenance, so the per-element and per-row writes are Tree-Borrows-clean
/// (the C1 fix; the prior `as_read_slice().as_ptr().cast_mut()` branded them
/// Frozen / SharedReadOnly — UB to write through). The read-only bases are
/// `*const _` reborrows of the same write-capable raw base, which is sound to read
/// through. The bases are address-stable (backed by `ComponentPool` reservations
/// that never realloc-move) and captured AFTER the last build-time grow (the B4
/// re-create-before-view-live discipline), so they cannot dangle while a view is
/// live.
#[derive(Clone, Copy)]
struct CohortSolveView<'a> {
    /// Read-only cohort heads.
    heads: *const CohortHead,
    /// Worker-mutable rank blocks (the impulse rows are written; the rest read).
    blocks: *mut RankBlock,
    /// Read-only per-group point CSR (the dispatcher cuts work-balanced chunks by it).
    group_start: *const u32,
    /// Cohort count (the exclusive ceiling of `heads`).
    n_heads: usize,
    /// Rank block count (the exclusive ceiling of `blocks`).
    n_blocks: usize,
    /// Binds the view to the column borrow so it cannot outlive a refill / regrow.
    _marker: PhantomData<&'a ()>,
}

// SAFETY: see the struct-level soundness note — the C2 coloring disjointness
//   invariant + per-lane raw projections on the scalar path + whole-cohort ownership
//   on the SIMD path + address-stable bases. No `&mut` ever spans more than one
//   element, so concurrent worker access never aliases.
unsafe impl Send for CohortSolveView<'_> {}
// SAFETY: see the `Send` impl above — shared access yields per-element reads /
//   per-lane writes only (no whole-buffer slice), and disjoint groups mean
//   concurrent accessors target disjoint memory.
unsafe impl Sync for CohortSolveView<'_> {}

impl<'a> CohortSolveView<'a> {
    /// Reads `group_start[g]` (the per-group point CSR boundary). Used by the
    /// dispatcher to cut work-balanced chunks.
    ///
    /// # Safety
    /// `g` must index the live `group_start` column (`g <= n_groups`); upheld by
    /// the dispatcher, which reads only group indices within the color's
    /// `[g_lo, g_hi]` range.
    #[inline]
    unsafe fn group_start_at(&self, g: usize) -> u32 {
        // SAFETY: `g` is in range per the method contract; `group_start` is the live
        //   base of the `group_start` ScratchColumn. A plain `*const u32` read forms
        //   no reference spanning the column, so it never conflicts with a worker's
        //   per-element impulse write (the Tree-Borrows discipline).
        unsafe { *self.group_start.add(g) }
    }

    /// Cohort `k`'s head, shared: no worker writes a head during a solve, so the
    /// reference never aliases a write.
    ///
    /// # Safety
    /// `k < n_heads` — a cohort of the color or chunk being solved.
    #[inline]
    unsafe fn head(&self, k: usize) -> &'a CohortHead {
        debug_assert!(k < self.n_heads, "invariant: a solved cohort is a built cohort");
        // SAFETY: `k < n_heads` (the contract), so `heads + k` is the live `k`-th head
        //   on the address-stable base; heads are written only by the single-threaded
        //   build, before any view is live, so this shared read aliases no write.
        unsafe { &*self.heads.add(k) }
    }

    /// Rank block `i`, raw: the caller projects single lanes (`(*block).ni[lane]`,
    /// scalar workers) or whole rows (aligned vector loads, the SIMD worker that
    /// owns the cohort), never a reference to the whole block.
    ///
    /// # Safety
    /// `i < n_blocks` — a rank of a cohort of the color or chunk being solved.
    #[inline]
    unsafe fn block(&self, i: usize) -> *mut RankBlock {
        debug_assert!(i < self.n_blocks, "invariant: a solved rank is a built rank");
        // SAFETY: `i < n_blocks` (the contract), so `blocks + i` is the live `i`-th
        //   block on the address-stable, write-capable base.
        unsafe { self.blocks.add(i) }
    }
}

/// Lane `l` of a block's three anchor rows, read through raw projections (the
/// scalar worker path: no reference to the block is formed).
///
/// # Safety
/// `rows` is a live `[[f32; 8]; 3]` of a built block and `l < 8`; no other worker
/// writes lane `l` of it (anchors are never written during a solve).
#[inline]
unsafe fn lane_vec3(rows: *const [[f32; COHORT]; 3], l: usize) -> Vec3 {
    debug_assert!(l < COHORT);
    // SAFETY: each projection names one live `f32` inside the row array (the
    //   contract); `ptr::read` of a `Copy` value forms no reference.
    unsafe {
        Vec3::new(
            ptr::read(&raw const (*rows)[0][l]),
            ptr::read(&raw const (*rows)[1][l]),
            ptr::read(&raw const (*rows)[2][l]),
        )
    }
}

/// The colored solver's per-step constraint state in cohort-shaped **AoSoA** form
/// (L11 D5): one [`CohortHead`] and one [`CohortCold`] per cohort, one
/// [`RankBlock`] and one cold `vn0` row per (cohort, rank), plus the per-manifold
/// plan and the four CSRs that place a color in the tables.
///
/// # Layout
///
/// Cohorts are numbered in color order: color `c`'s cohorts are
/// `color_cohort_start[c] .. color_cohort_start[c + 1]`, its groups
/// `color_group_start[c] .. color_group_start[c + 1]`, and group `g` of the color
/// is lane `(g - color_group_start[c]) % 8` of cohort
/// `color_cohort_start[c] + (g - color_group_start[c]) / 8`. A cohort's ranks are
/// `rank_base .. rank_base + depth` in `blocks` / `rank_cold`. `group_start` is the per-group POINT prefix
/// (`len == n_groups + 1`) the dispatcher cuts work-balanced chunks by, and
/// `color_offsets` the per-color point prefix (`len == n_colors + 1`) the
/// whole-solve dispatch gate and the profiling zones read — both exactly the CSRs
/// the per-point columns kept, so the dispatch structure is unchanged (D6).
///
/// # The build (D4)
///
/// Three passes: P-a in stream order writes the warm keys, the source runs
/// (`plan`) and the per-manifold `tags`; P-b walks the color CSR and lays the
/// cohorts out — the CSRs, each cohort's rank table and lane manifolds — sizing
/// `heads` / `cold` to a bound it then cuts, and `blocks` / `rank_cold` to their
/// exact rank count; P-c fills every cohort from its manifolds (the head lanes,
/// the blocks, the seeds, the lazy `vn0`), zeroing padding. Columns are `resize`d,
/// never pushed: growth fills once, and every entry is then overwritten.
///
/// # The warm store's view (L11 C1/C2)
///
/// The store walks cohorts and writes each solved manifold's converged impulses
/// into its record by `cold.mi` (`warm_records.rs`), then walks `tags` for the
/// frozen manifolds' carry (B1) and the empty ones. Neither depends on the worker
/// count: the layout is a pure function of the stream and the graph.
///
/// The columns are kernel-native [`ScratchColumn`]s (audit Stage P — P2): a
/// `ScratchColumn`'s data base is ADDRESS-STABLE across an in-place grow, the
/// property `std::Vec` lacks (the SP4/rigid race root cause). The worker-facing
/// [`CohortSolveView`] hands out raw bases only, so the parallel solve never
/// reborrows a whole buffer.
struct CohortColumns {
    /// One head per cohort, in color order (HOT: five lines per cohort per sweep).
    heads: ScratchColumn<CohortHead>,
    /// One block per (cohort, rank), cohorts in color order (HOT: the sweeps' data).
    blocks: ScratchColumn<RankBlock>,
    /// One cold record per cohort (restitution, manifold indices).
    cold: ScratchColumn<CohortCold>,
    /// One cold `vn0` row per (cohort, rank): the gather-time relative normal
    /// APPROACH velocity of each lane's point, computed only where the lane's
    /// restitution can read it (D9) and zero elsewhere.
    rank_cold: ScratchColumn<[f32; COHORT]>,
    /// Per-manifold-index warm source (L11 D2, ruling W1): the run of the previous
    /// stream's records whose key is this manifold's translated pair, written by
    /// `plan_sources` in stream order, read by the seed of every solved point and by
    /// the store's carry. `WarmRun::MISS` when nothing was looked up.
    plan: ScratchColumn<WarmRun>,
    /// Per-manifold-index tag (P-a): the live point count and the frozen flag.
    tags: ScratchColumn<ManifoldTag>,
    /// CSR color point offsets: `color_offsets[c + 1] - color_offsets[c]` is color
    /// `c`'s point count (`len == n_colors + 1`).
    color_offsets: ScratchColumn<u32>,
    /// CSR manifold-group point boundaries in solve order (C1): group `g` holds
    /// `group_start[g + 1] - group_start[g]` points (`len == n_groups + 1`). One
    /// group per laid-out manifold.
    group_start: ScratchColumn<u32>,
    /// Per-color CSR into the groups (C1): color `c`'s groups are
    /// `color_group_start[c] .. color_group_start[c + 1]` (`len == n_colors + 1`).
    color_group_start: ScratchColumn<u32>,
    /// Per-color CSR into the cohorts (C2): color `c`'s cohorts are
    /// `color_cohort_start[c] .. color_cohort_start[c + 1]` (`len == n_colors + 1`).
    color_cohort_start: ScratchColumn<u32>,
}

impl CohortColumns {
    /// Builds the 10 cohort columns, each on its own band id, reserving room for
    /// `contacts` points (clamped up to the kernel's adaptive per-element budget so
    /// a freshly-built solver never regrows in steady state). A cohort has at least
    /// one point and a rank block at least one, so `contacts` bounds both counts.
    ///
    /// The layouts are registered idempotently before any `ScratchColumn::new`
    /// reads them (see [`register_scratch_layouts`]).
    fn with_capacity(contacts: usize) -> Self {
        // Self-register the band BEFORE any `ScratchColumn::new` reads a synthetic
        // id's layout — idempotent (write-once `OnceLock`), so calling it here AND in
        // the solver constructor costs one branch each after the first. Makes this
        // constructor safe for ANY caller (tests build `CohortColumns` directly).
        register_scratch_layouts();
        // Reserve at least the contact count, but never below the kernel's adaptive
        // per-element budget (a pure-address-space reservation, demand-committed —
        // a generous ceiling costs nothing and removes the per-step grow-cap hazard).
        let rows = |stride: usize| contacts.max(scratch_reserve_rows(stride));
        // `k` walks the band in struct field order (see `register_contact_column_layouts`).
        let cols = Self {
            heads: ScratchColumn::new(contact_column_id(0), rows(size_of::<CohortHead>())),
            blocks: ScratchColumn::new(contact_column_id(1), rows(size_of::<RankBlock>())),
            cold: ScratchColumn::new(contact_column_id(2), rows(size_of::<CohortCold>())),
            rank_cold: ScratchColumn::new(contact_column_id(3), rows(size_of::<[f32; COHORT]>())),
            plan: ScratchColumn::new(contact_column_id(4), rows(size_of::<WarmRun>())),
            tags: ScratchColumn::new(contact_column_id(5), rows(size_of::<ManifoldTag>())),
            color_offsets: ScratchColumn::new(contact_column_id(6), rows(size_of::<u32>())),
            group_start: ScratchColumn::new(contact_column_id(7), rows(size_of::<u32>())),
            color_group_start: ScratchColumn::new(contact_column_id(8), rows(size_of::<u32>())),
            color_cohort_start: ScratchColumn::new(contact_column_id(9), rows(size_of::<u32>())),
        };
        // The kernel's aligned vector loads need 32 B rows; the structs are `align(64)`
        // and the pool's base plus its cache-line stagger keeps that alignment.
        debug_assert!(
            (cols.heads.solve_base() as usize).is_multiple_of(64)
                && (cols.blocks.solve_base() as usize).is_multiple_of(64),
            "invariant: the cohort tables are 64 B-aligned"
        );
        cols
    }

    /// Number of contact points laid out (the sum over colors).
    #[inline]
    fn len(&self) -> usize {
        self.color_offsets().last().map_or(0, |&n| n as usize)
    }

    /// Freezes the built tables into the worker-facing [`CohortSolveView`] (B4
    /// re-create-before-view-live: called AFTER the last build-time grow, so the
    /// captured raw bases are stable for every worker's lifetime).
    ///
    /// Every base is derived from the kernel's write-capable raw base accessor
    /// [`ScratchColumn::solve_base`] — a provenance-preserving `*mut T` taken from
    /// `ComponentPool::buffer_ptr` with NO `&[T]` interposed. The block base
    /// (worker-mutable) therefore carries WRITE provenance; the read-only bases are
    /// `*const _` reborrows of the same write-capable raw base, sound to read
    /// through. Concurrent writes are non-aliasing by the C2 coloring invariant.
    fn solve_view(&self) -> CohortSolveView<'_> {
        CohortSolveView {
            heads: self.heads.solve_base().cast_const(),
            // Worker-mutable: write-capable provenance (NOT Frozen).
            blocks: self.blocks.solve_base(),
            group_start: self.group_start.solve_base().cast_const(),
            n_heads: self.heads.len(),
            n_blocks: self.blocks.len(),
            _marker: PhantomData,
        }
    }

    /// The CSR color point offsets as a read slice.
    #[inline]
    fn color_offsets(&self) -> &[u32] {
        self.color_offsets.as_read_slice()
    }

    /// Point count of the WIDEST color — the whole-solve dispatch metric.
    ///
    /// This is the same quantity `solve_all_colors` compares against
    /// [`MIN_PARALLEL_SLOTS_PER_COLOR`] per color, taken over the widest color:
    /// if it does not clear the floor, no color does, and the whole-solve
    /// dispatch is pure loss. If it DOES clear the floor, at least one color is
    /// worth dispatching and the whole-solve gate must let the step through.
    ///
    /// It replaces `ConstraintGraph::max_island_constraints` as the gate metric.
    /// That one measured ISLAND size, on the premise that a small largest island
    /// cannot yield a wide color — but a color is a set of BODY-DISJOINT
    /// manifolds, and manifolds in DIFFERENT islands are always body-disjoint,
    /// so island size bounds color width from ABOVE only in the degenerate
    /// single-island case and bounds it from below not at all. `n` disjoint
    /// pairs are `n` islands of one manifold each AND one color of `n` slots.
    ///
    /// Zero for an empty partition: `color_offsets` is then empty or a lone
    /// sentinel, `windows(2)` yields nothing, and the gate correctly refuses.
    #[inline]
    fn widest_color_slots(&self) -> u32 {
        self.color_offsets()
            .windows(2)
            .map(|w| w[1] - w[0])
            .max()
            .unwrap_or(0)
    }

    /// `(wide, narrow)` contact-point totals over this step's colors, classed by the
    /// same `MIN_PARALLEL_SLOTS_PER_COLOR` floor as the per-color profiling zones.
    ///
    /// Read only by the armed profiler's slot counters, once per step, so it is out
    /// of line rather than folded into the build.
    #[cold]
    fn slots_by_width(&self) -> (u64, u64) {
        self.color_offsets().windows(2).fold((0, 0), |(wide, narrow), w| {
            let slots = w[1] - w[0];
            if slots < MIN_PARALLEL_SLOTS_PER_COLOR {
                (wide, narrow + u64::from(slots))
            } else {
                (wide + u64::from(slots), narrow)
            }
        })
    }

    /// The per-group point CSR (`group_start`) as a read slice.
    #[inline]
    fn group_start(&self) -> &[u32] {
        self.group_start.as_read_slice()
    }

    /// The per-color CSR into the groups as a read slice.
    #[inline]
    fn color_group_start(&self) -> &[u32] {
        self.color_group_start.as_read_slice()
    }

    /// The per-color CSR into the cohorts as a read slice.
    #[inline]
    fn color_cohort_start(&self) -> &[u32] {
        self.color_cohort_start.as_read_slice()
    }

    /// Color `c`'s place in the group and cohort tables (the tests' lane lookup).
    #[cfg(test)]
    #[inline]
    fn color_ctx(&self, c: usize) -> ColorCtx {
        ColorCtx {
            g_base: self.color_group_start()[c] as usize,
            k_base: self.color_cohort_start()[c] as usize,
        }
    }

    /// The per-manifold warm-source runs as a read slice.
    #[inline]
    fn plan(&self) -> &[WarmRun] {
        self.plan.as_read_slice()
    }

    /// The per-manifold tags as a read slice.
    #[inline]
    fn tags(&self) -> &[ManifoldTag] {
        self.tags.as_read_slice()
    }

    /// The cohort heads as a read slice (the single-threaded passes).
    #[inline]
    fn heads(&self) -> &[CohortHead] {
        self.heads.as_read_slice()
    }

    /// The rank blocks as a read slice (the single-threaded passes).
    #[inline]
    fn blocks(&self) -> &[RankBlock] {
        self.blocks.as_read_slice()
    }

    /// Sizes `plan` and `tags` to exactly `n` manifolds for P-a to write by index
    /// (fills only on growth; every entry is then overwritten).
    fn manifold_fill(&mut self, n: usize) {
        self.plan.build_view().resize(n, WarmRun::MISS);
        self.tags.build_view().resize(n, ManifoldTag::default());
    }

    /// P-b (D4): lays the cohorts out over the graph's color CSR from the tags P-a
    /// wrote, in color order. Writes the four CSRs and every cohort's rank table —
    /// `rank_base`, `depth`, `nlanes`, the per-lane `width` and `cold.mi`, the zero
    /// padding bytes — and sizes `blocks` / `rank_cold` to the exact rank count. The
    /// lane data the fill (P-c) writes is untouched here.
    ///
    /// `heads` / `cold` are sized to a bound (one cohort per eight manifolds plus one
    /// partial cohort per color) and cut to the cohort count afterwards — the
    /// sized-then-written idiom; the fill on growth happens once.
    ///
    /// Returns the laid-out manifold count.
    fn layout(&mut self, graph: &ConstraintGraph) -> u32 {
        let Self {
            heads,
            blocks,
            cold,
            rank_cold,
            tags,
            color_offsets,
            group_start,
            color_group_start,
            color_cohort_start,
            ..
        } = self;
        let n_colors = graph.n_colors() as usize;
        let tags = tags.as_read_slice();
        let bound = tags.len().div_ceil(COHORT) + n_colors;

        let mut heads_view = heads.build_view();
        heads_view.resize(bound, CohortHead::ZERO);
        let mut cold_view = cold.build_view();
        cold_view.resize(bound, CohortCold::ZERO);
        let heads = heads_view.as_mut_slice();
        let cold = cold_view.as_mut_slice();

        let mut color_offsets = color_offsets.build_view();
        let mut group_start = group_start.build_view();
        let mut color_group_start = color_group_start.build_view();
        let mut color_cohort_start = color_cohort_start.build_view();
        color_offsets.clear();
        group_start.clear();
        color_group_start.clear();
        color_cohort_start.clear();
        color_offsets.push(0);
        group_start.push(0);
        color_group_start.push(0);
        color_cohort_start.push(0);

        let mut k = 0usize;
        let mut ranks = 0u32;
        let mut points = 0u32;
        let mut groups = 0u32;
        let mut seeded = 0u32;
        for c in 0..n_colors {
            let mut lane = 0usize;
            // The graph yields the color's manifold indices in ascending order (D4),
            // so lanes ascend in manifold index within a cohort and cohorts within a
            // color: group-major order over the cohorts is the slot order.
            for &mi in graph.color(c as u32) {
                let tag = tags[mi as usize];
                if !tag.solved() {
                    continue;
                }
                if lane == 0 {
                    // Open a cohort: its rank table starts at the running rank count;
                    // the lane fields the fill does not write are zeroed here.
                    let head = &mut heads[k];
                    head.rank_base = ranks;
                    head.depth = 0;
                    head.width = [0; COHORT];
                    head._p = 0;
                    head._pad = [0; 16];
                    cold[k].mi = [0; COHORT];
                }
                heads[k].width[lane] = tag.count;
                heads[k].depth = heads[k].depth.max(tag.count);
                cold[k].mi[lane] = mi;
                points += u32::from(tag.count);
                group_start.push(points);
                groups += 1;
                seeded += 1;
                lane += 1;
                if lane == COHORT {
                    heads[k].nlanes = COHORT as u8;
                    ranks += u32::from(heads[k].depth);
                    k += 1;
                    lane = 0;
                }
            }
            if lane != 0 {
                heads[k].nlanes = lane as u8;
                ranks += u32::from(heads[k].depth);
                k += 1;
            }
            color_offsets.push(points);
            color_group_start.push(groups);
            color_cohort_start.push(k as u32);
        }
        heads_view.truncate(k);
        cold_view.truncate(k);
        blocks.build_view().resize(ranks as usize, RankBlock::ZERO);
        rank_cold.build_view().resize(ranks as usize, [0.0; COHORT]);
        seeded
    }

    /// L11 C0: the FNV-1a 64 digest of the built layout's LOGICAL per-point values in
    /// slot order — the three warm seeds, and `vn0` masked to `0.0` wherever the
    /// restitution pass would never read it (`restitution <= 0.0`; a NaN coefficient
    /// keeps it, the exact complement D9/O3 rule) — then the group and colour CSRs,
    /// each length-first. Layout-free by construction: C2 re-derives the same values
    /// from the cohort blocks (cohorts in color order, lanes, then ranks, is the slot
    /// order the per-point columns had), so the C0 pins reproduce, and a changed
    /// seed, mask rule or CSR boundary moves the digest. Taken right after the
    /// build, before any sweep touches the impulse rows.
    #[cfg(test)]
    fn setup_digest(&self) -> u64 {
        let blocks = self.blocks();
        let vn0 = self.rank_cold.as_read_slice();
        let mut h = fnv_u32(FNV_OFFSET, self.len() as u32);
        for (head, cold) in self.heads().iter().zip(self.cold.as_read_slice()) {
            let rank_base = head.rank_base as usize;
            for (l, &width) in head.width[..head.nlanes as usize].iter().enumerate() {
                for (blk, vn) in blocks[rank_base..].iter().zip(&vn0[rank_base..]).take(width as usize) {
                    h = fnv_u32(h, blk.ni[l].to_bits());
                    h = fnv_u32(h, blk.ti1[l].to_bits());
                    h = fnv_u32(h, blk.ti2[l].to_bits());
                    let masked = if cold.restitution[l] <= 0.0 { 0.0 } else { vn[l] };
                    h = fnv_u32(h, masked.to_bits());
                }
            }
        }
        h = fnv_u32s(h, self.group_start());
        h = fnv_u32s(h, self.color_group_start());
        fnv_u32s(h, self.color_offsets())
    }
}

/// G3's debug half: every padding lane of `head` / `cold` and every padding rank of
/// the cohort's blocks and `vn0` rows is zero, so the layout bytes are a pure
/// function of the stream. Checked after each cohort's fill in debug builds; the
/// release build trusts the fill.
#[cfg(debug_assertions)]
fn debug_assert_padding_zero(
    head: &CohortHead,
    cold: &CohortCold,
    blocks: &[RankBlock],
    vn0: &[[f32; COHORT]],
) {
    let nlanes = head.nlanes as usize;
    for l in 0..COHORT {
        let width = head.width[l] as usize;
        if l >= nlanes {
            assert!(
                width == 0
                    && (0..3).all(|c| head.n[c][l].to_bits() == 0 && head.t1[c][l].to_bits() == 0)
                    && head.friction[l].to_bits() == 0
                    && head.body_a[l] == 0
                    && head.body_b[l] == 0
                    && !head.is_sentinel(l)
                    && cold.restitution[l].to_bits() == 0
                    && cold.mi[l] == 0,
                "invariant: padding lane {l} of a cohort is zero in every head field"
            );
        }
        for (blk, vn) in blocks.iter().zip(vn0).skip(width) {
            assert!(
                (0..3).all(|c| blk.ra[c][l].to_bits() == 0 && blk.rb[c][l].to_bits() == 0)
                    && blk.sep[l].to_bits() == 0
                    && blk.ni[l].to_bits() == 0
                    && blk.ti1[l].to_bits() == 0
                    && blk.ti2[l].to_bits() == 0
                    && vn[l].to_bits() == 0,
                "invariant: the ranks past lane {l}'s width are zero in every row"
            );
        }
    }
    assert!(head._p == 0 && head._pad == [0; 16], "invariant: the head's padding bytes are zero");
}

/// The colored TGS-Soft rigid-body solver (Phase O5, Decision 7).
///
/// A `Resource` owning its [`CohortColumns`] tables + the double-buffered
/// per-manifold warm store. Solves contacts in color order (a Gauss-Seidel sweep across
/// colors) over the columns, single-threaded in O5. Like the reference
/// [`SoftStepSolver`](super::SoftStepSolver) it
/// [`owns_integration`](RigidSolver::owns_integration), so the pipeline's
/// `physics_integrate` is gated off.
#[derive(ResourceDerive)]
pub struct ColoredSoftStepSolver {
    /// Per-body solver view, parallel to `scratch.bodies` — refreshed each
    /// substep so the world inverse inertia tracks the advancing orientation.
    /// Backed by a [`ScratchColumn`] (engine-owned, ADDRESS-STABLE base) instead
    /// of a `std::Vec` parallel-data-system (audit Stage P, the race-fix column):
    /// the colored workers reach each body row through a `ScratchSolveView`'s
    /// per-element `row_ptr` (no whole-buffer reborrow), and the base never
    /// realloc-moves across a refill-grow — the property `std::Vec` lacked that
    /// caused the SP4 colored-solve data race.
    bodies: ScratchColumn<BodyEffective>,
    /// The cohort tables, in color order (rebuilt each solve, reused).
    columns: CohortColumns,
    /// The two sides of the warm store (L11 D1): `warm[warm_cur]` holds the previous
    /// step's records and is read this step; the other side is written by
    /// `plan_sources` (the keys) and the store (the records), then becomes current.
    warm: [WarmRecords; 2],
    /// Which side of `warm` is read this step (`0` or `1`).
    warm_cur: u8,
    /// The cold sorted index over the read side's keys, built by `plan_sources` on a
    /// step whose read side is not strictly increasing (D2; hand-built streams only).
    warm_index: WarmIndex,
    /// Whether warm-starting is active (production default `true`).
    warm_start_enabled: bool,
    /// O8 integrate-freeze scratch: the pre-solve `(row, BodyState)` snapshot of each
    /// slept body, captured before the substep loop and restored after — so a slept
    /// island's bodies are NOT integrated (their hot state is frozen) without masking
    /// the per-lane O1 SIMD integrate kernels. Capacity-reused (cleared each step);
    /// empty when sleeping is off (the byte-identical O6/O7 path). Backed by a
    /// [`ScratchColumn`] (audit Stage 4).
    frozen: ScratchColumn<(u32, BodyState)>,
    /// The warm store's place in the gather sequence, stamped where the read side
    /// is swapped in (defect A, interim; U7 deletes it).
    warm_cursor: RemapCursor,
    /// The last solve's warm-start lookup diagnostic (defect A, interim; U7 deletes it).
    warm_stats: WarmSeedStats,
    /// Solves that ran past the no-dynamic-body early return. Diagnostic (defect A,
    /// interim).
    solved_steps: u64,
    /// Solves that took the no-awake fast path (L10 C3a). Diagnostic.
    fast_path_steps: u64,
    /// The G1 anti-vacuity counters (L11 C0). Zero-sized outside `cfg(test)`.
    counters: SetupCounters,
    /// The last solve's setup digest (L11 C0): the built columns' logical values,
    /// then the step's [`WarmSeedStats`], folded after the store. Test-only.
    #[cfg(test)]
    step_digest: u64,
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
        register_scratch_layouts();
        let reserve = bodies.max(scratch_reserve_rows(size_of::<BodyEffective>()));
        Self {
            bodies: ScratchColumn::new(body_eff_colored_id(), reserve),
            columns: CohortColumns::with_capacity(contacts),
            // One record per manifold; a manifold has at least one point in every
            // stream the narrowphase emits, so `contacts` bounds the manifold count
            // there, and the reserve floor covers the hand-built streams that do not.
            warm: [
                WarmRecords::with_capacity(warm_table_id(4), warm_table_id(2), contacts),
                WarmRecords::with_capacity(warm_table_id(5), warm_table_id(3), contacts),
            ],
            warm_cur: 0,
            warm_index: WarmIndex::with_capacity(warm_table_id(6), contacts),
            warm_start_enabled: true,
            frozen: ScratchColumn::new(
                colored_frozen_rows_id(),
                bodies.max(scratch_reserve_rows(size_of::<(u32, BodyState)>())),
            ),
            warm_cursor: RemapCursor::default(),
            warm_stats: WarmSeedStats::default(),
            solved_steps: 0,
            fast_path_steps: 0,
            counters: SetupCounters::default(),
            #[cfg(test)]
            step_digest: 0,
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

    /// Diagnostic: the last solve's warm-start lookup statistics — how many manifolds
    /// were seeded, how many of their lookup keys resolved to rows of the previous
    /// gather, and the warm table cursor's `Reset` count (defect A, interim).
    #[inline]
    pub fn warm_seed_stats(&self) -> WarmSeedStats {
        self.warm_stats
    }

    /// Diagnostic: the number of solves that ran past the no-dynamic-body early return
    /// (defect A, interim). A step with no simulated dynamic body returns before the
    /// count, so an unchanged value across a step shows that the early return ran.
    #[inline]
    pub fn solved_steps(&self) -> u64 {
        self.solved_steps
    }

    /// Diagnostic: the number of solves that took the no-awake fast path (L10 C3a) — sleeping
    /// on, no dynamic row awake and no contact point laid out — which run no substep, no
    /// restitution pass and no freeze capture or restore, and leave every value as the full
    /// path would. A structural witness: a reader derives the same count from the world.
    #[inline]
    pub fn fast_path_steps(&self) -> u64 {
        self.fast_path_steps
    }

    /// Diagnostic (L10's bit-identity gate, design 04 "What is compared", 06 §7): calls
    /// `f(key, fid, seed)` for every point of every logical manifold of `manifolds` — the
    /// stream's, whose records the last store wrote, and the held store's, whose records
    /// `sets` keeps — with the seed the next step's lookup of that manifold finds for the
    /// point's feature id: the last stored point with that id (Lemma W), `None` on a miss and
    /// with warm start off. `key` is the manifold's ordinal in the rows of the last step. Stream
    /// manifolds first, in stream order, then the kept ones by ordinal. O(points · log
    /// manifolds) on a strict store.
    pub fn for_each_warm_seed(
        &self,
        manifolds: &Manifolds,
        sets: &SleepSets,
        mut f: impl FnMut(u64, u16, Option<[f32; 3]>),
    ) {
        let read = &self.warm[usize::from(self.warm_cur)];
        let (keys, recs) = (read.keys(), read.recs());
        let lookup = |key: u64, fid: u16| -> Option<[f32; 3]> {
            if !self.warm_start_enabled {
                return None;
            }
            if read.strict() {
                let i = keys.partition_point(|&k| k < key);
                (keys.get(i) == Some(&key)).then(|| recs[i].seed_of(fid)).flatten()
            } else {
                (0..keys.len()).rev().filter(|&i| keys[i] == key).find_map(|i| recs[i].seed_of(fid))
            }
        };
        for m in manifolds.solver_manifolds() {
            let key = ord(m.body_a.0, m.body_b.0);
            for p in 0..usize::from(m.count) {
                let fid = point_fid(m, p);
                f(key, fid, lookup(key, fid));
            }
        }
        let held = manifolds.held.view();
        let kept_rec = sets.kept_records();
        for &slot in held.order() {
            let m = held.manifold(slot);
            let key = ord(m.body_a.0, m.body_b.0);
            for p in 0..usize::from(m.count) {
                let fid = point_fid(&m, p);
                let seed = if self.warm_start_enabled {
                    kept_rec.get(slot as usize).and_then(|r| r.seed_of(fid))
                } else {
                    None
                };
                f(key, fid, seed);
            }
        }
    }

    /// Whether warm-starting is active: L10's sleep epoch reads it in the broadphase (design
    /// 04 D5), since a toggle changes what a held island's records would be under `Off`.
    #[inline]
    pub(crate) fn warm_start_enabled(&self) -> bool {
        self.warm_start_enabled
    }

    /// L10's D-H drain (ruling on rev 2.3, open question 2): the warm records of the islands a
    /// flush restored on a step whose mode is `Off` join the read side before the solve, which
    /// searches the read side alone on that path, so every lookup of the step finds what
    /// `Off`'s store holds. Merged into the idle write side, which then becomes the read side;
    /// the warm cursor's stamp is unchanged, since the drained keys are in the rows the read
    /// side is keyed in. Called by the colored broadphase, never by the solve. A disabled
    /// solver reads no record, and takes none.
    pub(crate) fn drain_restore(&mut self, restore: &WarmRecords) {
        if !self.warm_start_enabled || restore.keys().is_empty() {
            return;
        }
        let (read, write) = Self::warm_sides(&mut self.warm, self.warm_cur);
        write.merge_of(read, restore);
        self.warm_cur ^= 1;
    }

    /// L10 A1′ (design 06 A1, 08 A1′): every kept manifold the broadphase moved in this step
    /// takes the record `Off`'s store writes for it at this step — its points carried from the
    /// read side through the run of its previous-step pair, by the D2 search and the D3 carry a
    /// frozen stream manifold takes — and its hits join the held total. Runs before the swap,
    /// while the read side still holds the previous step's records, on the full path and the
    /// fast path alike (O10). With warm start off or a Reset, every run misses: an empty record
    /// with no hit, as `Off`'s carry would drop every point. `remap` is how the warm store's
    /// cursor classifies this gather (the build's classification, not yet stamped).
    fn capture_moved_in(&self, remap: RowRemap<'_>, held: &mut HeldSolve<'_>) {
        let range = held.capture.clone();
        if range.is_empty() {
            return;
        }
        let mut kept_rec = held.kept_rec.build_view();
        let recs = kept_rec.as_mut_slice();
        if !self.warm_start_enabled {
            recs[range].fill(WarmRecord::EMPTY);
            return;
        }
        let read = &self.warm[usize::from(self.warm_cur)];
        let lookup = WarmLookup { read, index: &self.warm_index };
        let mut cursor = 0usize;
        for (i, slot) in range.enumerate() {
            let m = held.held.manifold(slot as u32);
            let run = match remap.manifold_pair(&m) {
                Some((la, lb)) if read.strict() => read.find(&mut cursor, ord(la, lb)).run,
                Some((la, lb)) => self.warm_index.run(ord(la, lb)),
                None => WarmRun::MISS,
            };
            // Design 08 OQ3: the run is the previous-stream index the move-in copied the
            // manifold from (debug builds hand the list over; a strict side's run is the index).
            if let Some(&entry) = held.sources.get(i) {
                debug_assert_eq!(entry >> 32, slot as u64, "invariant: the list is in capture order");
                debug_assert!(
                    run == WarmRun::MISS || !read.strict() || u64::from(run.lo) == entry & 0xFFFF_FFFF,
                    "invariant: a moved-in manifold's run is the stream index it was copied from"
                );
            }
            let (rec, hits) = lookup.carry(run, &m, usize::from(m.count));
            debug_assert_eq!(
                u32::from(rec.count()),
                hits,
                "invariant: the carry compacts its hits, so a record's count is its hit count"
            );
            recs[slot] = rec;
            *held.held_warm += hits;
        }
    }

    /// The G1 anti-vacuity counters, cumulative since construction (L11 C0).
    #[cfg(test)]
    #[inline]
    pub(crate) fn setup_counters(&self) -> SetupCounters {
        self.counters
    }

    /// The last solve's setup digest (L11 C0): `0` before the first solve that built
    /// columns; unchanged by a solve that took the no-dynamic-body early return.
    #[cfg(test)]
    #[inline]
    pub(crate) fn step_digest(&self) -> u64 {
        self.step_digest
    }

    /// Rebuilds the per-body solver views from the gather snapshot (mirrors the
    /// reference `build_bodies`).
    ///
    /// `cls` is L10's classification of the step (empty when the sleep-skip did not classify
    /// it): a row it marks `HELD` gets the effective inverse mass `0` (design 04 D8, ruling
    /// W1), read from the same function over the same flag as the graph's colouring predicate,
    /// so the write guards, gravity, the integrate, the refresh and the write-back leave a held
    /// row as the colouring does.
    fn build_bodies(&mut self, bodies: &[BodyState], cls: &[RowCls]) {
        debug_assert!(
            cls.is_empty() || cls.len() >= bodies.len(),
            "invariant: a classification covers every row"
        );
        let mut view = self.bodies.build_view();
        view.clear();
        for (row, b) in bodies.iter().enumerate() {
            let held = cls.get(row).is_some_and(|c| c.is_held());
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
            // L10 D8 (ruling W1): the graph's colouring predicate reads the same function
            // over the same flag, so the write guards and the colouring cannot disagree.
            view.push(BodyEffective {
                inv_mass: effective_inv_mass(b.inv_mass, held),
                inv_inertia: b.inv_inertia,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
            });
        }
    }

    /// Builds the cohort layout ([`CohortColumns`]) from the graph's color CSR,
    /// warm-SEEDS each point, and captures the lazy `vn0` (L11 D4: plan → layout →
    /// fill).
    ///
    /// P-a, in stream order: this step's ordinals go to the warm store's write side
    /// and every manifold's source run to `plan` (`warm_records::plan_sources`), then
    /// every manifold's live point count and frozen flag to `tags`. P-b, over the
    /// color CSR: [`CohortColumns::layout`] assigns each solved manifold a lane of a
    /// cohort (8-group windows from each color's first group — today's cohorts) and
    /// sizes the tables. P-c, per cohort: [`fill_cohort`](Self::fill_cohort) writes
    /// the lane constants, the rank blocks, the seeds by feature id, the lazy `vn0`
    /// and the zero padding.
    ///
    /// Mirrors the reference `build_constraints` per-point math: anchors relative
    /// to each body's gather center, the degeneracy-safe tangent basis, and the
    /// seeded accumulated impulses (zero on a miss / when disabled).
    ///
    /// No per-step heap allocation: all scratch is capacity-reused.
    ///
    /// `restore` is L10's restore warm source for the step (design 06 A3), searched after a
    /// miss of the read side. Returns its searches and hits.
    fn build_columns(
        &mut self,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        bodies: &[BodyState],
        sleep: Option<&IslandSleep>,
        remap: RowRemap<'_>,
        restore: Option<&WarmRecords>,
    ) -> (u32, u32) {
        if let RowRemap::Rows(prev_row) = remap {
            debug_assert_eq!(
                prev_row.len(),
                bodies.len(),
                "invariant: the warm remap maps exactly the gathered rows"
            );
        }
        // Disjoint-field borrows: `columns` is written while `bodies` / the warm
        // read side are read. Destructure `self` so the borrow checker sees the
        // fields are distinct (a re-borrow alias through a method call would not).
        let Self {
            columns: cols,
            bodies: bodies_eff,
            warm,
            warm_cur,
            warm_index,
            warm_start_enabled,
            warm_cursor,
            warm_stats,
            counters,
            ..
        } = self;
        let n = manifolds.len();

        // P-a (D4): stream order. The write side takes this step's ordinals; every
        // manifold's warm source run goes to `plan`. The read side is `warm[cur]`,
        // the write side the other one — two elements of one array, borrowed apart.
        // B4: every grow / refill happens here and in P-b, single-threaded, BEFORE
        // any `solve_view()` captures a base — so no worker can see a moving base.
        let (read, write) = Self::warm_sides(warm, *warm_cur);
        cols.manifold_fill(n);
        if *warm_start_enabled {
            write.resize(n);
        }

        // P-a, the tags: the live point count and the freeze decision, made here once
        // per manifold. A manifold whose island is FROZEN this step is not laid out —
        // it never enters a cohort, so it is never solved — the IM-1-safe solve skip
        // (gather stays full; only the solve is elided); the store carries its record
        // instead (B1). `sleep == None` (sleeping off) takes the byte-identical O6/O7
        // path. The island is the manifold's dynamic side (the same resolution the
        // graph build uses). The tags are written before the source search below, which
        // adds L10's `SRC_RESTORE` to the manifolds whose run it finds in `restore`.
        //
        // Warm-seed diagnostic (defect A): the carried count is taken only on a step
        // whose rows changed; the unchanged step does no per-manifold work for it.
        let carried_rows = *warm_start_enabled && matches!(remap, RowRemap::Rows(_));
        let mut carried = 0u32;
        let mut points = 0u32;
        // Live points of the manifolds frozen this step (B1): the carry set's size.
        let mut frozen_points = 0u32;
        {
            let mut tags = cols.tags.build_view();
            let tags = tags.as_mut_slice();
            for (m, tag) in manifolds.iter().zip(tags) {
                let frozen = sleep.is_some_and(|s| Self::manifold_frozen(m, graph, s));
                *tag = ManifoldTag {
                    count: m.count,
                    flags: if frozen { ManifoldTag::FROZEN } else { 0 },
                };
                if tag.solved() {
                    points += u32::from(m.count);
                    if carried_rows && remap.manifold_pair(m).is_some() {
                        carried += 1;
                    }
                } else if frozen && *warm_start_enabled && m.count != 0 {
                    frozen_points += u32::from(m.count);
                    counters.duplicate_fids(m);
                }
            }
        }
        // P-a, the sources: this step's ordinals to the write side, every manifold's warm run
        // to `plan` — from the read side, or (L10, design 06 A3) from the restore source
        // when the read side misses, which marks the tag `SRC_RESTORE`.
        let plan_counts = {
            let mut tags = cols.tags.build_view();
            let tags = tags.as_mut_slice();
            warm_records::plan_sources_restored(
                manifolds,
                remap,
                *warm_start_enabled,
                read,
                write,
                warm_index,
                cols.plan.build_view().as_mut_slice(),
                restore,
                |mi| tags[mi].flags |= ManifoldTag::SRC_RESTORE,
            )
        };
        counters.plan(plan_counts);
        let index: &WarmIndex = warm_index;
        let lookup = WarmLookup { read, index };
        let restore_lookup = restore.map(|read| WarmLookup { read, index });

        // P-b (D4): the layout, in color order.
        let seeded = cols.layout(graph);
        debug_assert_eq!(points, cols.len() as u32, "invariant: the tags and the layout agree on the point count");

        // P-c (D4): the fill, per cohort. The `BodyEffective` rows the fill reads are
        // a SINGLE-THREADED read here (the build runs before any parallel dispatch);
        // take one read slice of the solver's body column (a distinct field of
        // `self`). The write side's records take each solved manifold's shape now,
        // while its manifold is in cache; the store writes the impulses (D3).
        let bodies_eff = bodies_eff.as_read_slice();
        let mut point_hits = 0u32;
        // The build views commit their lengths on drop, so the fill's borrows end here.
        {
            let sources = WarmSources {
                lookup,
                restore: restore_lookup,
                plan: cols.plan.as_read_slice(),
                tags: cols.tags.as_read_slice(),
            };
            let mut recs_view = write.recs_mut();
            let mut recs_w = if *warm_start_enabled { Some(recs_view.as_mut_slice()) } else { None };
            let mut heads = cols.heads.build_view();
            let mut cold = cols.cold.build_view();
            let mut blocks = cols.blocks.build_view();
            let mut vn0 = cols.rank_cold.build_view();
            let blocks = blocks.as_mut_slice();
            let vn0 = vn0.as_mut_slice();
            for (head, cold) in heads.as_mut_slice().iter_mut().zip(cold.as_mut_slice()) {
                let lo = head.rank_base as usize;
                let hi = lo + head.depth as usize;
                point_hits += Self::fill_cohort(
                    head,
                    cold,
                    &mut blocks[lo..hi],
                    &mut vn0[lo..hi],
                    manifolds,
                    bodies,
                    bodies_eff,
                    sources,
                    recs_w.as_deref_mut(),
                    counters,
                );
                #[cfg(debug_assertions)]
                debug_assert_padding_zero(head, cold, &blocks[lo..hi], &vn0[lo..hi]);
            }
        }

        let translated = match remap {
            _ if !*warm_start_enabled => 0,
            RowRemap::Identity => seeded,
            RowRemap::Rows(_) => carried,
            RowRemap::Reset => 0,
        };
        *warm_stats = WarmSeedStats {
            manifolds: seeded,
            translated,
            points,
            point_hits,
            carry_points: frozen_points,
            // Counted by the store, which runs the carry after the solve.
            carry_hits: 0,
            remap_resets: warm_cursor.resets(),
        };

        debug_assert_eq!(
            cols.group_start().len() as u32,
            *cols.color_group_start().last().unwrap_or(&0) + 1,
            "invariant: the per-color group CSR must tile every manifold-group exactly once"
        );
        debug_assert_eq!(
            cols.heads.len() as u32,
            *cols.color_cohort_start().last().unwrap_or(&0),
            "invariant: the per-color cohort CSR must tile every cohort exactly once"
        );
        (plan_counts.restore_searches, plan_counts.restore_hits)
    }

    /// The read side and the write side of the warm store: `warm[cur]` is read this
    /// step, the other side written.
    #[inline]
    fn warm_sides(warm: &mut [WarmRecords; 2], cur: u8) -> (&WarmRecords, &mut WarmRecords) {
        let [side0, side1] = warm;
        if cur == 0 { (side0, side1) } else { (side1, side0) }
    }

    /// P-c (D4): fills one cohort from its manifolds — a pure function of the cohort's
    /// rank table (P-b), the manifolds, the bodies and the warm read side, so the
    /// cohorts could be filled in any order or in parallel (C4).
    ///
    /// Per live lane: the manifold constants go into the head (the normal, the first
    /// tangent, the friction, the body rows, the sentinel bit) and the cold record
    /// (the restitution); the write side's record takes the manifold's shape (its
    /// feature ids and point count); per point, the rank block takes the anchors,
    /// the separation and the seed — the last stored point of the manifold's run
    /// carrying its feature id (Lemma W), zero on a miss or when nothing was looked
    /// up — and the cold row its `vn0`, computed only where the restitution pass can
    /// read it (`!(restitution <= 0.0)`, D9/O3) and zero elsewhere. Every padding
    /// lane and every rank past a lane's width is written as zero.
    ///
    /// `blocks` / `vn0` are the cohort's own `depth` rows. `recs_w` is `None` with
    /// warm start disabled (no record is written). `sources` holds every manifold's run and
    /// the lookup its tag names (the read side, or L10's restore source). Returns the lanes'
    /// point hits.
    // The ten parameters are the per-cohort slice of every table the fill reads
    // or writes, split so the C4 parallel fill can hand each worker its own cohort
    // rows without a shared `&mut CohortColumns`; a parameter struct would be built
    // once per cohort per step for no reader.
    #[allow(clippy::too_many_arguments)]
    fn fill_cohort(
        head: &mut CohortHead,
        cold: &mut CohortCold,
        blocks: &mut [RankBlock],
        vn0: &mut [[f32; COHORT]],
        manifolds: &[Manifold],
        bodies: &[BodyState],
        bodies_eff: &[BodyEffective],
        sources: WarmSources<'_>,
        mut recs_w: Option<&mut [WarmRecord]>,
        counters: &mut SetupCounters,
    ) -> u32 {
        let depth = head.depth as usize;
        let nlanes = head.nlanes as usize;
        debug_assert!(
            blocks.len() == depth && vn0.len() == depth && (1..=COHORT).contains(&nlanes),
            "invariant: a cohort's fill sees exactly its ranks and 1..=8 lanes"
        );
        let mut sentinel = 0u8;
        let mut hits = 0u32;
        for l in 0..COHORT {
            if l >= nlanes {
                // Padding lane: zero in every field.
                for c in 0..3 {
                    head.n[c][l] = 0.0;
                    head.t1[c][l] = 0.0;
                }
                head.friction[l] = 0.0;
                head.body_a[l] = 0;
                head.body_b[l] = 0;
                cold.restitution[l] = 0.0;
                for (blk, vn) in blocks.iter_mut().zip(vn0.iter_mut()) {
                    blk.clear_lane(l);
                    vn[l] = 0.0;
                }
                continue;
            }
            let mi = cold.mi[l] as usize;
            let m = &manifolds[mi];
            let width = head.width[l] as usize;
            debug_assert!(
                width == m.count as usize && 1 <= width && width <= depth,
                "invariant: a lane's width is its manifold's live point count"
            );
            let ia = m.body_a.0 as usize;
            let b_is_sentinel = m.body_b == SDF_SENTINEL;
            let ib = if b_is_sentinel { ia } else { m.body_b.0 as usize };
            let normal = m.normal;
            let (t1, _) = tangent_basis(normal);

            // Combined material coefficients (the reference `max` rule). For a
            // sentinel, `ib == ia`, so this resolves to A's own material — the same
            // convention the reference uses.
            let friction = bodies[ia].friction.max(bodies[ib].friction);
            let restitution = bodies[ia].restitution.max(bodies[ib].restitution);

            let pa = bodies[ia].position;
            let pb = if b_is_sentinel { Vec3::ZERO } else { bodies[ib].position };

            head.n[0][l] = normal.x;
            head.n[1][l] = normal.y;
            head.n[2][l] = normal.z;
            head.t1[0][l] = t1.x;
            head.t1[1][l] = t1.y;
            head.t1[2][l] = t1.z;
            head.friction[l] = friction;
            head.body_a[l] = ia as u32;
            head.body_b[l] = ib as u32;
            sentinel |= u8::from(b_is_sentinel) << l;
            cold.restitution[l] = restitution;

            // D9: `vn0` is read only by the restitution pass, and only where the
            // coefficient is not `<= 0` — the exact complement of its skip (a NaN
            // coefficient keeps the value). It is dead elsewhere, so a zero there
            // changes no value.
            // The written form IS the rule: `restitution > 0.0` would differ on a NaN
            // coefficient, which the consumer does not skip (review O3).
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            let lazy_vn0 = !(restitution <= 0.0);
            let ba = &bodies_eff[ia];
            let bb = if b_is_sentinel { &IMMOVABLE_AT_REST } else { &bodies_eff[ib] };

            let (lookup, run) = sources.of(mi);
            if let Some(recs) = recs_w.as_deref_mut() {
                recs[mi].set_shape(m);
            }
            let mut lane_hits = 0u32;
            for (r, (blk, vn)) in blocks.iter_mut().zip(vn0.iter_mut()).enumerate() {
                if r >= width {
                    blk.clear_lane(l);
                    vn[l] = 0.0;
                    continue;
                }
                let cp = &m.points[r];
                let ra = cp.anchor_a - pa;
                let rb = if b_is_sentinel { Vec3::ZERO } else { cp.anchor_b - pb };
                blk.ra[0][l] = ra.x;
                blk.ra[1][l] = ra.y;
                blk.ra[2][l] = ra.z;
                blk.rb[0][l] = rb.x;
                blk.rb[1][l] = rb.y;
                blk.rb[2][l] = rb.z;
                blk.sep[l] = cp.separation;

                // The seed: the last stored point of the manifold's run carrying this
                // feature id (Lemma W), zero on a miss or when nothing was looked up
                // (an empty run).
                let seed = match lookup.seed(run, point_fid(m, r)) {
                    Some(seed) => {
                        lane_hits += 1;
                        seed
                    }
                    None => [0.0; 3],
                };
                blk.ni[l] = seed[0];
                blk.ti1[l] = seed[1];
                blk.ti2[l] = seed[2];

                // Gather-time relative normal approach velocity (B−A on the normal).
                vn[l] = if lazy_vn0 {
                    (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
                } else {
                    0.0
                };
            }
            hits += lane_hits;
            if recs_w.is_some() {
                counters.miss(width as u32 - lane_hits);
                counters.duplicate_fids(m);
            }
        }
        head.sentinel = sentinel;
        hits
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

    /// Whether no dynamic row (`inv_mass != 0`, the rows the freeze capture snapshots) is awake
    /// this step: the first condition of the no-awake fast path (L10 C3a).
    #[inline]
    fn no_awake_dynamic_row(sleep: &IslandSleep, bodies: &[BodyState]) -> bool {
        !bodies
            .iter()
            .enumerate()
            .any(|(row, b)| is_dynamic_row(b.inv_mass) && sleep.is_row_awake(row))
    }

    /// Applies every contact point's seeded accumulated impulse to both bodies'
    /// velocities (the warm-start apply, run once per substep after gravity) — the
    /// D7 dispatch fork, the SINGLE site that chooses the apply's shape.
    ///
    /// - `simd == false`, or a non-AVX2 build →
    ///   [`warm_apply_scalar`](Self::warm_apply_scalar), the group-major oracle.
    /// - `simd == true` on an AVX2 build →
    ///   [`warm_apply_avx2`](Self::warm_apply_avx2) over the same cohorts, eight
    ///   lanes at a time and bit-identical (L11 C3, D7).
    ///
    /// The flag is [`PhysicsConfig::simd_solve`] — the same one that forks the
    /// sweep kernel, so a step never mixes the two shapes, and the `simd_solve`
    /// arms of every G1 scene gate this fork. Both paths are SERIAL (C3): the apply
    /// runs on the calling thread with no worker live, before the first sweep's
    /// dispatch.
    #[inline]
    fn warm_start_apply(
        cols: &CohortColumns,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        simd: bool,
    ) {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        {
            if simd {
                // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees the
                //   executing CPU supports every AVX2 intrinsic the apply uses (a non-AVX2
                //   host cannot reach this branch — the `cfg` excludes it). `cols` is the
                //   cohort table `build_columns` filled this step, so every head's
                //   `rank_base + depth` is in bounds and every head and block is live and
                //   64 B-aligned; no worker is live here, so this thread alone reads and
                //   writes the body rows the apply touches.
                unsafe { Self::warm_apply_avx2(cols, bodies_eff) };
                return;
            }
        }
        // Flag off / non-AVX2 build: the group-major oracle.
        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
        let _ = simd;
        Self::warm_apply_scalar(cols, bodies_eff);
    }

    /// The scalar warm-start apply — the ORACLE the AVX2 apply is gated against
    /// (G4), and the path a `simd_solve == false` step takes.
    ///
    /// Mirrors the reference `warm_start_apply`, but reads the cohort tables
    /// group-major — cohorts in color order, lanes, then ranks — which is the slot
    /// order the per-point columns had, so every body's add sequence is unchanged.
    /// The apply is a pure accumulation onto velocities and, within a color, the
    /// bodies are disjoint; across colors the seed is independent of order.
    fn warm_apply_scalar(cols: &CohortColumns, bodies_eff: ScratchSolveView<'_, BodyEffective>) {
        let blocks = cols.blocks();
        for head in cols.heads() {
            let rank_base = head.rank_base as usize;
            for l in 0..head.nlanes as usize {
                let normal = head.normal(l);
                let t1 = head.tangent1(l);
                let t2 = normal.cross(t1);
                // Guard the write on MOVABILITY, exactly as `solve_color` does — see the
                // `ia_movable` / `ib_movable` rationale there for the full proof. Two
                // properties, and only the first is about this function today:
                //
                // * BIT-IDENTICAL. A static / kinematic row has `inv_mass == 0` AND
                //   `inv_inertia == Mat3::ZERO`, so both halves of `apply_impulse` are a
                //   value no-op on it for every finite impulse. Skipping the write cannot
                //   change a bit.
                // * LOAD-BEARING THE MOMENT THIS RUNS IN PARALLEL. The coloring marks only
                //   DYNAMIC bodies, so two manifold-groups in one color may SHARE a static
                //   body (a ground floor as `body_b`). Unguarded, a color-partitioned
                //   parallel apply has every lane writing that one shared static row — a
                //   data race that is VALUE-identical and therefore invisible to the
                //   `{1, N}` bit oracles. The guard is what makes each worker's writes
                //   disjoint, and it must land BEFORE the parallel dispatch, not with it.
                //
                // `is_dynamic_row` is the same predicate the O4 coloring uses, so the two
                // cannot drift into disagreeing about which rows are shared. It is the
                // IEEE `inv_mass != 0.0`, so a `-0.0` row is immovable on this path and on
                // the AVX2 one (review O1, gated by
                // `negative_zero_inv_mass_row_is_immovable_in_both_warm_applies`).
                let ia = head.body_a[l] as usize;
                let ia_movable = is_dynamic_row(body_ref(bodies_eff, ia).inv_mass);
                let ib = head.body_b[l] as usize;
                let ib_movable =
                    !head.is_sentinel(l) && is_dynamic_row(body_ref(bodies_eff, ib).inv_mass);
                for blk in &blocks[rank_base..rank_base + head.width[l] as usize] {
                    let impulse = normal * blk.ni[l] + t1 * blk.ti1[l] + t2 * blk.ti2[l];
                    if ia_movable {
                        body_mut(bodies_eff, ia).apply_impulse(blk.ra(l), impulse * -1.0);
                    }
                    if ib_movable {
                        body_mut(bodies_eff, ib).apply_impulse(blk.rb(l), impulse);
                    }
                }
            }
        }
    }

    /// The 8-lane warm-start apply (L11 C3, D7): one cohort at a time, serial,
    /// under `simd_solve`. BIT-IDENTICAL to
    /// [`warm_apply_scalar`](Self::warm_apply_scalar).
    ///
    /// # Why the bits are the same
    ///
    /// * **The impulse associates left to right.** The scalar
    ///   `normal * ni + t1 * ti1 + t2 * ti2` is `((n·λn + t1·λt1) + t2·λt2)` per
    ///   component, and each product is a separate `mul` then `add` here (NO FMA —
    ///   the module's no-FMA invariant, censused by
    ///   `colored_has_no_fma_or_approx_callsites`). Mutation M7 re-associates it and
    ///   is recorded red.
    /// * **`t2 = n × t1`** via [`cross8`](super::simd::cross8), op for op the
    ///   `Vec3::cross` the scalar calls and the one `tangent_basis` derived the
    ///   stored tangent with.
    /// * **The register carry is the scalar's read-modify-write.** The A/B
    ///   velocities are gathered once per cohort, carried across the ranks and
    ///   scattered at cohort exit. Within a color the manifold-groups are
    ///   body-disjoint (the O4 invariant), so lane `l`'s two rows are touched by no
    ///   other lane of the cohort; each lane therefore sees exactly its own adds, in
    ///   ascending rank order, which is the scalar's sequence for that body. The
    ///   scatter lands before the next cohort's gather, so the cross-cohort order is
    ///   the scalar's too — and cohorts are walked in color order on both paths.
    /// * **The mask is `active ∧ movable`** (review O1): `active = width[l] > r`
    ///   reproduces the scalar's `r < width[l]` rank bound, and `movable` is the
    ///   IEEE `inv_mass != 0.0` of [`is_dynamic_row`] — an
    ///   `_mm256_cmp_ps::<_CMP_NEQ_OQ>` against zero, NOT a bit test, so a `-0.0`
    ///   `inv_mass` row is immovable here exactly as it is on the scalar path.
    ///   [`apply_impulse_blend_x8`](super::simd::apply_impulse_blend_x8) blends, so
    ///   a masked-off lane keeps its original velocity bits.
    ///
    /// # O2 — a padding lane reads no body row
    ///
    /// A lane `>= nlanes` is skipped by the gather (its staged state is written
    /// zero, never read from a row) and by the scatter. Its `width` is 0, so
    /// `active` is false at every rank and every blended write is discarded. The
    /// fill zeroes a padding lane's `body_a`, so gathering it would read row 0 — a
    /// real, dynamic row the solve writes. That read is value-identical (the lane is
    /// masked), which is why only the contract, the Miri case and mutation M12 can
    /// see it.
    ///
    /// # Safety
    ///
    /// The caller must guarantee AVX2 (the `cfg` + `target_feature` gate), that
    /// `cols` is a fully built cohort table — every head's `rank_base + depth`
    /// within `blocks`, every head and block live and 64 B-aligned — and that no
    /// other thread reads or writes the body rows of `bodies_eff` while this runs
    /// (C3 is serial: the apply owns them).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[target_feature(enable = "avx2")]
    unsafe fn warm_apply_avx2(
        cols: &CohortColumns,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
    ) {
        use core::arch::x86_64::{
            _CMP_NEQ_OQ, _mm_loadl_epi64, _mm256_add_ps, _mm256_and_ps, _mm256_castsi256_ps,
            _mm256_cmp_ps, _mm256_cmpgt_epi32, _mm256_cvtepu8_epi32, _mm256_mul_ps,
            _mm256_set1_epi32, _mm256_set1_ps,
        };
        use crate::solver::simd::{apply_impulse_blend_x8, cross8};

        const W: usize = COHORT;

        // ── Per-cohort stack scratch (zero heap; the kernel's gather-once shape) ──
        let mut ia = [0usize; W]; // body-A row index
        let mut ib = [0usize; W]; // body-B row index (== ia for a sentinel)
        let mut sent = [false; W]; // sentinel per lane (padding lanes: false)
        let mut a_invm_s = [0.0f32; W];
        let mut b_invm_s = [0.0f32; W];
        let mut a_ii_s = [[0.0f32; W]; 9];
        let mut b_ii_s = [[0.0f32; W]; 9];
        let mut a_lin_s = [[0.0f32; W]; 3];
        let mut a_ang_s = [[0.0f32; W]; 3];
        let mut b_lin_s = [[0.0f32; W]; 3];
        let mut b_ang_s = [[0.0f32; W]; 3];
        // a_static[lane] = 1.0 when A is NOT movable (`inv_mass == 0.0`, IEEE — so a
        // `-0.0` row lands here). Always-dynamic by manifold convention, but kept for
        // scalar-exactness: the scalar guards both sides.
        let mut a_static_s = [0.0f32; W];
        let mut not_sent_f = [0.0f32; W];
        // Velocity scatter staging (written only at cohort exit).
        let mut out_a_lin = [[0.0f32; W]; 3];
        let mut out_a_ang = [[0.0f32; W]; 3];
        let mut out_b_lin = [[0.0f32; W]; 3];
        let mut out_b_ang = [[0.0f32; W]; 3];

        // SAFETY (target_feature): every intrinsic below is AVX2; this fn is
        //   `#[target_feature(enable = "avx2")]`-gated and only reached on an AVX2
        //   build (the `cfg` gate), so the CPU supports them.
        let zero = _mm256_set1_ps(0.0);
        let one = _mm256_set1_ps(1.0);
        let neg_one = _mm256_set1_ps(-1.0);

        let blocks = cols.blocks();
        for head in cols.heads() {
            let nlanes = head.nlanes as usize;
            let depth = head.depth as usize;
            let rank_base = head.rank_base as usize;
            debug_assert!((1..=W).contains(&nlanes), "cohort has 1..=8 lanes");
            debug_assert!(
                rank_base + depth <= blocks.len(),
                "invariant: the cohort's ranks are built"
            );

            // ── Gather-ONCE (cohort entry) ──────────────────────────────────────
            for lane in 0..W {
                // Review O2: a padding lane (`lane >= nlanes`) reads NO body row.
                let b_sent = head.is_sentinel(lane);
                sent[lane] = b_sent;
                not_sent_f[lane] = if b_sent { 0.0 } else { 1.0 };
                if lane < nlanes {
                    let lane_ia = head.body_a[lane] as usize;
                    let lane_ib = head.body_b[lane] as usize;
                    // A sentinel reads IMMOVABLE_AT_REST, never `bodies_eff[lane_ib]`,
                    // mirroring the scalar's short-circuited `!is_sentinel(l) && …`, so
                    // only assert B's row when it is a real body.
                    debug_assert!(lane_ia < bodies_eff.len());
                    debug_assert!(b_sent || lane_ib < bodies_eff.len());
                    ia[lane] = lane_ia;
                    ib[lane] = lane_ib;

                    let ba = body_ref(bodies_eff, lane_ia);
                    a_invm_s[lane] = ba.inv_mass;
                    a_static_s[lane] = if is_dynamic_row(ba.inv_mass) { 0.0 } else { 1.0 };
                    Self::stage_body_state(ba, lane, &mut a_ii_s, &mut a_lin_s, &mut a_ang_s);

                    let bb =
                        if b_sent { &IMMOVABLE_AT_REST } else { body_ref(bodies_eff, lane_ib) };
                    b_invm_s[lane] = bb.inv_mass;
                    Self::stage_body_state(bb, lane, &mut b_ii_s, &mut b_lin_s, &mut b_ang_s);
                } else {
                    ia[lane] = 0;
                    ib[lane] = 0;
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

            let a_invm = load1(&a_invm_s);
            let b_invm = load1(&b_invm_s);
            let a_ii = load9(&a_ii_s);
            let b_ii = load9(&b_ii_s);
            // movable masks (constant within the cohort), the kernel's guard table:
            // A movable = `inv_mass != 0.0`; B movable = !sentinel AND `inv_mass != 0.0`.
            let not_sent = _mm256_cmp_ps::<_CMP_NEQ_OQ>(load1(&not_sent_f), zero);
            let a_movable = _mm256_cmp_ps::<_CMP_NEQ_OQ>(load1(&a_static_s), one);
            let b_movable = _mm256_and_ps(not_sent, _mm256_cmp_ps::<_CMP_NEQ_OQ>(b_invm, zero));

            // The head's per-lane constants: aligned rows of the 64 B-aligned head.
            // SAFETY: `head` is a live `&CohortHead`; its `n` / `t1` rows sit at offsets
            //   0 / 96, both 32 B-aligned.
            let (n, t1) = unsafe { (load_rows3(&raw const head.n), load_rows3(&raw const head.t1)) };
            let t2 = cross8(n[0], n[1], n[2], t1[0], t1[1], t1[2]);
            // The per-lane widths, widened to 8 × i32 for the per-rank `active` compare.
            // SAFETY: `head.width` is a live 8 B array; `_mm_loadl_epi64` reads 8 bytes
            //   with no alignment requirement.
            let width = unsafe { _mm256_cvtepu8_epi32(_mm_loadl_epi64(head.width.as_ptr().cast())) };

            // A/B velocity: REGISTER-CARRIED across the whole rank loop.
            let mut a_lin = load3(&a_lin_s);
            let mut a_ang = load3(&a_ang_s);
            let mut b_lin = load3(&b_lin_s);
            let mut b_ang = load3(&b_ang_s);

            // ── Rank loop (register-carry velocity) ─────────────────────────────
            for r in 0..depth {
                // SAFETY: `rank_base + r < rank_base + depth <= blocks.len()` (asserted
                //   above), so this is a built block of THIS cohort; `blocks` is a shared
                //   slice of the 64 B-aligned column, so each row sits at offset
                //   0 / 96 / 224 / 256 / 288 of a 64 B-aligned block and the aligned
                //   vector loads below are in bounds and aligned. The apply never writes
                //   a block, and no worker is live.
                let blk: *const RankBlock = &blocks[rank_base + r];
                // active = width[lane] > r as a lane mask (an integer compare, cast) —
                // the scalar's `r < width[l]` rank bound.
                let active =
                    _mm256_castsi256_ps(_mm256_cmpgt_epi32(width, _mm256_set1_epi32(r as i32)));
                let mask_a = _mm256_and_ps(active, a_movable);
                let mask_b = _mm256_and_ps(active, b_movable);

                // Nine aligned loads: the rank's two anchors and three impulse rows.
                // SAFETY: see the block's contract above.
                let (ra, rb, ni, ti1, ti2) = unsafe {
                    (
                        load_rows3(&raw const (*blk).ra),
                        load_rows3(&raw const (*blk).rb),
                        load_row(&raw const (*blk).ni),
                        load_row(&raw const (*blk).ti1),
                        load_row(&raw const (*blk).ti2),
                    )
                };

                // impulse = ((n·λn + t1·λt1) + t2·λt2), component by component: the
                // scalar `normal * ni + t1 * ti1 + t2 * ti2` associates left to right.
                // Separate mul then add, NO FMA. (Mutation M7 re-associates this.)
                let imp = [
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(n[0], ni), _mm256_mul_ps(t1[0], ti1)),
                        _mm256_mul_ps(t2[0], ti2),
                    ),
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(n[1], ni), _mm256_mul_ps(t1[1], ti1)),
                        _mm256_mul_ps(t2[1], ti2),
                    ),
                    _mm256_add_ps(
                        _mm256_add_ps(_mm256_mul_ps(n[2], ni), _mm256_mul_ps(t1[2], ti1)),
                        _mm256_mul_ps(t2[2], ti2),
                    ),
                ];
                // A gets -impulse, B gets +impulse (the scalar's `impulse * -1.0`).
                let neg_imp = [
                    _mm256_mul_ps(imp[0], neg_one),
                    _mm256_mul_ps(imp[1], neg_one),
                    _mm256_mul_ps(imp[2], neg_one),
                ];
                let (na_lin, na_ang) =
                    apply_impulse_blend_x8(a_lin, a_ang, ra, neg_imp, a_invm, a_ii, mask_a);
                a_lin = na_lin;
                a_ang = na_ang;
                let (nb_lin, nb_ang) =
                    apply_impulse_blend_x8(b_lin, b_ang, rb, imp, b_invm, b_ii, mask_b);
                b_lin = nb_lin;
                b_ang = nb_ang;
            }

            // ── Scatter-ONCE (cohort exit): A/B velocity registers → body rows ───
            // Only MOVABLE lanes BELOW `nlanes` are written (D7): a static / sentinel
            // row is never touched (its register held the unchanged gathered value
            // anyway, since `mask_*` masked it off), and a padding lane is not a lane —
            // its `body_a` is the fill's zero, i.e. row 0. Mutation M12 drops both
            // halves of that guard and is recorded red.
            // SAFETY: the stores write 8 `f32` into in-bounds `[f32; 8]` stack buffers.
            store3(&a_lin, &mut out_a_lin);
            store3(&a_ang, &mut out_a_ang);
            store3(&b_lin, &mut out_b_lin);
            store3(&b_ang, &mut out_b_ang);
            for lane in 0..nlanes {
                if a_static_s[lane] == 0.0 {
                    let b = body_mut(bodies_eff, ia[lane]);
                    b.linear_velocity =
                        Vec3::new(out_a_lin[0][lane], out_a_lin[1][lane], out_a_lin[2][lane]);
                    b.angular_velocity =
                        Vec3::new(out_a_ang[0][lane], out_a_ang[1][lane], out_a_ang[2][lane]);
                }
                if !sent[lane] && is_dynamic_row(b_invm_s[lane]) {
                    let b = body_mut(bodies_eff, ib[lane]);
                    b.linear_velocity =
                        Vec3::new(out_b_lin[0][lane], out_b_lin[1][lane], out_b_lin[2][lane]);
                    b.angular_velocity =
                        Vec3::new(out_b_ang[0][lane], out_b_ang[1][lane], out_b_ang[2][lane]);
                }
            }
        }
    }

    /// Solves the normal + coupled-friction impulses for one COLOR's manifold-group
    /// range once (one Gauss-Seidel sweep over the groups) — the per-color kernel
    /// (Phase O5, Decision 7) over the cohort layout (L11 C2).
    ///
    /// `[g_lo, g_hi)` is the color's (or a chunk's) group range and `ctx` places the
    /// color in the cohort tables. POOL-AGNOSTIC and ORDER-INDEPENDENT ACROSS THE
    /// MANIFOLD-GROUPS of the color: each manifold-group in a color touches dynamic
    /// bodies no OTHER group in the color touches (the O4 coloring invariant is
    /// manifold-group granular), so the velocity result does not depend on the
    /// order the GROUPS are visited — the property [O6] relies on to dispatch a
    /// color's groups across threads (and [O7] to pack adjacent groups into a lane).
    /// The ≥2 points of a SINGLE manifold-group share BOTH bodies, so they are
    /// order-coupled and must be solved together, sequentially, on one thread /
    /// lane — never split across workers/lanes. The scalar oracle visits each group's
    /// ranks in ascending order, which solves the group's points in sequence.
    ///
    /// [O6]: https://github.com/bluesteelll/boyko-engine
    ///
    /// The numerical kernel MIRRORS the reference
    /// [`solve_velocities`](super::SoftStepSolver) per-contact math: the soft
    /// normal solve (`dλ = -massCoeff·mEff·(vn + bias) - impulseCoeff·λ`, the
    /// `max(0)` clamp, the `max(-MAX_BIAS_VELOCITY)` bias clamp), then the 2-DOF
    /// coupled Coulomb friction CONE (`|λt| ≤ µ·λn`, exact `sqrt`), reading the
    /// cohort tables instead of the AoS `PointConstraint`. SCALAR here ([O7] widens
    /// this to AVX2 over the same tables).
    ///
    /// [O7]: https://github.com/bluesteelll/boyko-engine
    ///
    /// O7 dispatch fork over a contiguous group range — the SINGLE site that
    /// chooses the scalar oracle vs the AVX2 cohort kernel.
    ///
    /// - `simd == false` → [`solve_color`](Self::solve_color) over the groups
    ///   `[g_lo, g_hi)`: BYTE-IDENTICAL to the committed O6 path (the 0%-gate).
    /// - `simd == true` on an AVX2 build → [`solve_color_avx2`](Self::solve_color_avx2)
    ///   over the cohorts of `[g_lo, g_hi)` (`ctx.cohorts_of`): the bit-exact
    ///   WIDTH-ONLY path (Decision 2). The range must start on a cohort boundary —
    ///   the color's first group, or a `COHORT`-stepped cut from it — which the
    ///   caller (a whole color, or a parallel cohort-run chunk) upholds.
    /// - `simd == true` on a non-AVX2 build → falls back to
    ///   [`solve_color`](Self::solve_color) (one fallback = the oracle;
    ///   bit-identical, simpler — the design's "choose the latter").
    ///
    /// The gate is `cfg(all(target_arch = "x86_64", target_feature = "avx2"))` with
    /// no `not(miri)` term, on purpose: under Miri the fork follows the Miri build's
    /// own target features, so an AVX2 Miri build exercises the `unsafe` kernel
    /// rather than a scalar stand-in.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn solve_color_dispatch(
        view: CohortSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        ctx: ColorCtx,
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
                let (k_lo, k_hi) = ctx.cohorts_of(g_lo, g_hi);
                // SAFETY: the `target_feature = "avx2"` compile-time gate guarantees
                //   the executing CPU supports every AVX2 intrinsic the kernel uses
                //   (a non-AVX2 host cannot reach this branch — the `cfg` excludes it).
                //   `[k_lo, k_hi)` are built cohorts of one color (the caller's range),
                //   whose lanes are body-disjoint; the kernel documents its per-load
                //   bounds + disjoint-write invariants.
                unsafe {
                    Self::solve_color_avx2(
                        view,
                        bodies_eff,
                        k_lo,
                        k_hi,
                        bias_rate,
                        mass_coeff,
                        impulse_coeff,
                        bias_active,
                    );
                }
                return;
            }
        }
        // Flag off / non-AVX2 build: the scalar oracle over the groups (the
        // `simd_solve == false` path AND the SIMD non-AVX2 fallback).
        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
        let _ = simd;
        Self::solve_color(
            view,
            bodies_eff,
            ctx,
            g_lo,
            g_hi,
            bias_rate,
            mass_coeff,
            impulse_coeff,
            bias_active,
        );
    }

    /// The scalar per-color kernel over the cohort layout: group-major (lane outer,
    /// rank inner) over the groups `[g_lo, g_hi)` of the color `ctx` places — the
    /// slot order of the per-point columns, op for op. Worker-safe: reads its lanes'
    /// constants through the shared head and its lanes' block elements through raw
    /// projections, and writes its lanes' impulse elements the same way (a scalar
    /// chunk may share a cohort with another worker's chunk, never a lane).
    ///
    /// [O7]: https://github.com/bluesteelll/boyko-engine
    #[allow(clippy::too_many_arguments)]
    fn solve_color(
        view: CohortSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        ctx: ColorCtx,
        g_lo: usize,
        g_hi: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        for g in g_lo..g_hi {
            let (k, l) = ctx.lane_of(g);
            // SAFETY: `g` is a built group of the color, so `(k, l)` is a live lane of a
            //   built cohort (`k < n_heads`); heads are never written during a solve.
            let head = unsafe { view.head(k) };
            debug_assert!(l < head.nlanes as usize, "invariant: a group maps to a live lane");
            let normal = head.normal(l);
            let t1 = head.tangent1(l);
            let t2 = normal.cross(t1);
            let ia = head.body_a[l] as usize;
            let b_is_sentinel = head.is_sentinel(l);
            let ib = head.body_b[l] as usize;
            let friction = head.friction[l];

            // Snapshot body B (the immovable surface for an SDF contact).
            let bb_view = || -> BodyEffective {
                if b_is_sentinel { IMMOVABLE_AT_REST } else { body_copy(bodies_eff, ib) }
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
            let ia_movable = is_dynamic_row(body_ref(bodies_eff, ia).inv_mass);
            let ib_movable = !b_is_sentinel && is_dynamic_row(body_ref(bodies_eff, ib).inv_mass);

            let rank_base = head.rank_base as usize;
            for r in 0..head.width[l] as usize {
                // SAFETY: `rank_base + r < rank_base + depth <= n_blocks` (a built rank
                //   of this cohort); every access below is a raw projection of lane `l`,
                //   which this worker owns (the group is its own) — no reference to the
                //   block is formed, so another worker's writes to its own lanes of the
                //   same block never alias.
                let blk = unsafe { view.block(rank_base + r) };
                // SAFETY: as above — single-lane raw reads of live `f32`s.
                let (ra, rb, separation) = unsafe {
                    (
                        lane_vec3(&raw const (*blk).ra, l),
                        lane_vec3(&raw const (*blk).rb, l),
                        ptr::read(&raw const (*blk).sep[l]),
                    )
                };

                // ── Normal solve ───────────────────────────────────────────────
                let m_eff = {
                    let ba = body_copy(bodies_eff, ia);
                    let bb = bb_view();
                    effective_mass(normal, ra, rb, &ba, &bb)
                };
                let vn = {
                    let ba = body_ref(bodies_eff, ia);
                    let bb = bb_view();
                    (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
                };
                let bias = if bias_active {
                    (bias_rate * separation).max(-MAX_BIAS_VELOCITY)
                } else {
                    0.0
                };
                // SAFETY: a single-lane raw read of the live impulse element this
                //   worker owns.
                let lambda_n = unsafe { ptr::read(&raw const (*blk).ni[l]) };
                let d_lambda = if bias_active {
                    -mass_coeff * m_eff * (vn + bias) - impulse_coeff * lambda_n
                } else {
                    -m_eff * vn
                };
                let new_lambda = (lambda_n + d_lambda).max(0.0);
                // The AVX2 kernel's `_mm256_max_ps(sum, +0)` returns `+0` on a `±0` tie,
                // while this `max` leaves the sign of such a tie unspecified; they agree
                // bit-for-bit because the tie cannot occur. λ starts at `+0` and is always
                // a `max(.., 0)` output, and in round-to-nearest a sum is `-0` only when
                // both addends are `-0`, so by induction the clamp never sees `-0`.
                debug_assert!(
                    new_lambda.to_bits() != 0x8000_0000,
                    "invariant: the normal-impulse clamp never yields -0 (the SIMD ±0-tie proof)"
                );
                let applied_n = new_lambda - lambda_n;
                // SAFETY: a single-lane raw write of the impulse element this worker
                //   owns (the C2 coloring invariant grants lane `l` of this cohort to AT
                //   MOST ONE worker); the base carries WRITE provenance
                //   (`ScratchColumn::solve_base`), never a Frozen `&[_]` reborrow.
                unsafe { ptr::write(&raw mut (*blk).ni[l], new_lambda) };
                {
                    let impulse = normal * applied_n;
                    if ia_movable {
                        body_mut(bodies_eff, ia).apply_impulse(ra, impulse * -1.0);
                    }
                    if ib_movable {
                        body_mut(bodies_eff, ib).apply_impulse(rb, impulse);
                    }
                }

                // ── Friction solve (2-DOF coupled cone) ────────────────────────
                // `new_lambda` IS the value just stored at this lane's normal impulse,
                // so `friction * new_lambda` is bit-identical to re-reading the element
                // and removes a redundant load.
                let max_friction = friction * new_lambda;
                let m_eff_t1 = {
                    let ba = body_copy(bodies_eff, ia);
                    let bb = bb_view();
                    effective_mass(t1, ra, rb, &ba, &bb)
                };
                let m_eff_t2 = {
                    let ba = body_copy(bodies_eff, ia);
                    let bb = bb_view();
                    effective_mass(t2, ra, rb, &ba, &bb)
                };
                let (vt1, vt2) = {
                    let ba = body_ref(bodies_eff, ia);
                    let bb = bb_view();
                    let dv = bb.point_velocity(rb) - ba.point_velocity(ra);
                    (dv.dot(t1), dv.dot(t2))
                };
                // Read each tangent impulse ONCE (both uses read the same pre-write value);
                // bit-identical to the prior double-read, no reload between the two uses.
                // SAFETY: single-lane raw reads of the live impulse elements this worker
                //   owns.
                let (lambda_t1, lambda_t2) = unsafe {
                    (ptr::read(&raw const (*blk).ti1[l]), ptr::read(&raw const (*blk).ti2[l]))
                };
                let mut new_t1 = lambda_t1 - m_eff_t1 * vt1;
                let mut new_t2 = lambda_t2 - m_eff_t2 * vt2;
                let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
                if len_sq > max_friction * max_friction && len_sq > 0.0 {
                    let scale = max_friction / len_sq.sqrt();
                    new_t1 *= scale;
                    new_t2 *= scale;
                }
                let applied_t1 = new_t1 - lambda_t1;
                let applied_t2 = new_t2 - lambda_t2;
                // SAFETY: as the normal-impulse write — single-lane, single-owner,
                //   write-capable base.
                unsafe {
                    ptr::write(&raw mut (*blk).ti1[l], new_t1);
                    ptr::write(&raw mut (*blk).ti2[l], new_t2);
                }
                {
                    let impulse = t1 * applied_t1 + t2 * applied_t2;
                    if ia_movable {
                        body_mut(bodies_eff, ia).apply_impulse(ra, impulse * -1.0);
                    }
                    if ib_movable {
                        body_mut(bodies_eff, ib).apply_impulse(rb, impulse);
                    }
                }
            }
        }
    }

    /// AVX2 8-wide colored solve (Phase O7) over the cohort layout (L11 C2) — lane
    /// = one whole manifold-GROUP, a cohort = 8 body-disjoint groups of one color,
    /// BIT-IDENTICAL to [`solve_color`](Self::solve_color) over the same groups.
    ///
    /// Solves the cohorts `[k_lo, k_hi)`. Per cohort: GATHER-ONCE the live lanes'
    /// two body pairs (inv_mass, inv_inertia, velocity) into stack SoA and load the
    /// head's constants (`n`, `t1`, the friction; `t2 = n × t1` in registers, the
    /// same op `tangent_basis` uses); a RANK loop `r = 0..depth` where each ACTIVE
    /// lane (`width > r`) solves its group's point-`r` normal→friction with the A/B
    /// velocity REGISTER-CARRIED across ranks (so point `p` sees `p-1`'s update —
    /// the intra-group Gauss-Seidel coupling) and the rank block's impulse rows
    /// read-modify-written as vectors; SCATTER-ONCE the 16 body rows at cohort exit.
    ///
    /// # Bit-exactness (Decision 2, the PINNED invariant)
    ///
    /// Each lane runs its group's EXACT scalar `solve_color` op sequence (the same
    /// IEEE round-to-nearest `mul`/`add`/`sub`/`div`/`sqrt`, NO FMA, NO
    /// `rsqrt`/`rcp` — the `simd::*_x8` helpers mirror `contact.rs`/`math.rs`
    /// op-for-op). The 8 lanes write DISJOINT body rows + DISJOINT impulse elements
    /// (the O4 coloring invariant: distinct groups of a color touch disjoint
    /// dynamic bodies), so the parallel 8-lane evaluation equals solving the 8
    /// groups sequentially, which equals the scalar single-threaded colored solve,
    /// bit-for-bit. Masked exhausted / padding / static lanes compute on the zero
    /// padding that every `blendv` / guarded scalar scatter discards (FP exceptions
    /// masked ⇒ Inf/NaN are inert DATA, never traps — Decision 3/4); the impulse
    /// store is `blendv(old, new, active)`, so padding stays zero.
    ///
    /// # Safety
    ///
    /// The caller must guarantee AVX2 is available (the `cfg` + `target_feature`
    /// gate — a non-AVX2 host cannot link this path). `[k_lo, k_hi)` must be built
    /// cohorts of ONE color (`k_hi <= n_heads`) whose lanes' dynamic bodies are
    /// pairwise-disjoint (the O4 coloring invariant — upheld because the range is a
    /// color's cohorts, or a cohort-run within one color). All gathered body indices
    /// are `< bodies_eff.len()` (the build invariant).
    ///
    /// A SIMD worker OWNS its cohorts: the dispatcher's cuts are `COHORT`-stepped,
    /// so every head and block the kernel loads or stores belongs to `[k_lo, k_hi)`
    /// and to no concurrently-running worker — there is no foreign read (the old
    /// per-point gather's clamped-slot trick is gone with the per-point columns).
    /// Lanes `>= nlanes` read NO body row (review O2): their gathered state is zero,
    /// so a padding lane never touches a row another chunk of the color may be
    /// writing. The per-cohort scatter writes ≤ 16 distinct dynamic body rows;
    /// statics/sentinels are never written (the movable-blend guard), so a SHARED
    /// static row across cohorts is read-only.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[target_feature(enable = "avx2")]
    #[allow(clippy::too_many_arguments)]
    fn solve_color_avx2(
        view: CohortSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        k_lo: usize,
        k_hi: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        use core::arch::x86_64::{
            _CMP_GT_OQ, _CMP_NEQ_OQ, _mm_loadl_epi64, _mm256_add_ps, _mm256_and_ps,
            _mm256_blendv_ps, _mm256_castsi256_ps, _mm256_cmp_ps, _mm256_cmpgt_epi32,
            _mm256_cvtepu8_epi32, _mm256_div_ps, _mm256_max_ps, _mm256_mul_ps,
            _mm256_set1_epi32, _mm256_set1_ps, _mm256_sqrt_ps, _mm256_sub_ps,
        };
        use crate::solver::simd::{
            apply_impulse_blend_x8, cross8, dot8, effective_mass_x8, pointvel_x8,
        };

        const W: usize = COHORT;
        debug_assert!(k_hi <= view.n_heads, "invariant: the cohort range is built");

        // ── Per-cohort stack scratch (zero heap; capacity-retained per call) ────
        // Per-lane (group) metadata, gathered once at cohort entry.
        let mut ia = [0usize; W]; // body-A row index
        let mut ib = [0usize; W]; // body-B row index (== ia for a sentinel)
        let mut sent = [false; W]; // sentinel per lane (padding lanes: false)
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
        let mut not_sent_f = [0.0f32; W];
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

        for k in k_lo..k_hi {
            // SAFETY: `k < k_hi <= n_heads` (the caller's contract): a built cohort of
            //   the color, never written during a solve.
            let head = unsafe { view.head(k) };
            let nlanes = head.nlanes as usize;
            let depth = head.depth as usize;
            let rank_base = head.rank_base as usize;
            debug_assert!((1..=W).contains(&nlanes), "cohort has 1..=8 lanes");
            debug_assert!(rank_base + depth <= view.n_blocks, "invariant: the cohort's ranks are built");

            // ── Gather-ONCE (cohort entry) ──────────────────────────────────────
            for lane in 0..W {
                // Review O2: a padding lane (`lane >= nlanes`) reads NO body row —
                // its gathered state is zero, so it never touches a row another chunk
                // of this color may be writing. Permanently inactive (`width == 0`
                // ⇒ `active = false` at every rank), every write discarded.
                let b_sent = head.is_sentinel(lane);
                sent[lane] = b_sent;
                not_sent_f[lane] = if b_sent { 0.0 } else { 1.0 };
                if lane < nlanes {
                    let lane_ia = head.body_a[lane] as usize;
                    let lane_ib = head.body_b[lane] as usize;
                    // O3: B is only indexed when it is a real body (a sentinel reads
                    // IMMOVABLE_AT_REST, never `bodies_eff[lane_ib]`), mirroring the
                    // scalar oracle's conditional index — so only assert it then.
                    debug_assert!(lane_ia < bodies_eff.len());
                    debug_assert!(b_sent || lane_ib < bodies_eff.len());
                    ia[lane] = lane_ia;
                    ib[lane] = lane_ib;

                    // Body A (always the dynamic side by convention).
                    let ba = body_ref(bodies_eff, lane_ia);
                    a_invm_s[lane] = ba.inv_mass;
                    a_static_s[lane] = if is_dynamic_row(ba.inv_mass) { 0.0 } else { 1.0 };
                    Self::stage_body_state(ba, lane, &mut a_ii_s, &mut a_lin_s, &mut a_ang_s);

                    // Body B: a sentinel gathers IMMOVABLE_AT_REST (inv_mass 0,
                    // inv_inertia ZERO, velocity ZERO) so its lane is a value no-op
                    // and is never scattered (the sentinel guard at exit).
                    let bb = if b_sent { &IMMOVABLE_AT_REST } else { body_ref(bodies_eff, lane_ib) };
                    b_invm_s[lane] = bb.inv_mass;
                    Self::stage_body_state(bb, lane, &mut b_ii_s, &mut b_lin_s, &mut b_ang_s);
                } else {
                    ia[lane] = 0;
                    ib[lane] = 0;
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
            let not_sent = _mm256_cmp_ps::<_CMP_NEQ_OQ>(load1(&not_sent_f), zero);

            // The head's per-lane constants: aligned rows of the 64 B-aligned head.
            // SAFETY: `head` is a live `&CohortHead`; its `n` / `t1` / `friction` rows
            //   sit at offsets 0 / 96 / 192, all 32 B-aligned.
            let (n, t1, fric) = unsafe {
                (
                    load_rows3(&raw const head.n),
                    load_rows3(&raw const head.t1),
                    load_row(&raw const head.friction),
                )
            };
            // t2 = n × t1, op-for-op `Vec3::cross` (`cross8`), the same op
            // `tangent_basis` derives the stored per-point tangent with.
            let t2 = cross8(n[0], n[1], n[2], t1[0], t1[1], t1[2]);
            // The per-lane widths, widened to 8 × i32 for the per-rank `active` compare.
            // SAFETY: `head.width` is a live 8 B array; `_mm_loadl_epi64` reads 8 bytes
            //   with no alignment requirement.
            let width = unsafe { _mm256_cvtepu8_epi32(_mm_loadl_epi64(head.width.as_ptr().cast())) };

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
            for r in 0..depth {
                // SAFETY: `rank_base + r < rank_base + depth <= n_blocks` (asserted
                //   above): a built block of THIS cohort, which this worker owns whole
                //   (the `COHORT`-stepped cuts); its rows are 32 B-aligned (offsets
                //   0 / 96 / 192 / 224 / 256 / 288 of a 64 B-aligned block), so the
                //   aligned vector loads and stores below are in bounds and aligned, and
                //   no other worker touches the block.
                let blk = unsafe { view.block(rank_base + r) };
                // active = width[lane] > r as a lane mask (an integer compare, cast).
                let active =
                    _mm256_castsi256_ps(_mm256_cmpgt_epi32(width, _mm256_set1_epi32(r as i32)));

                // Ten aligned loads: the rank's anchors, separation and impulses.
                // SAFETY: see the block's contract above.
                let (ra, rb, sep, old_ni, old_ti1, old_ti2) = unsafe {
                    (
                        load_rows3(&raw const (*blk).ra),
                        load_rows3(&raw const (*blk).rb),
                        load_row(&raw const (*blk).sep),
                        load_row(&raw const (*blk).ni),
                        load_row(&raw const (*blk).ti1),
                        load_row(&raw const (*blk).ti2),
                    )
                };

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
                let lambda_n = old_ni;
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
                let ni = new_lambda;
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
                // max_friction = friction * ni (the JUST-computed new normal impulse).
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
                let mut new_t1 = _mm256_sub_ps(old_ti1, _mm256_mul_ps(m_eff_t1, vt1));
                let mut new_t2 = _mm256_sub_ps(old_ti2, _mm256_mul_ps(m_eff_t2, vt2));
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
                let applied_t1 = _mm256_sub_ps(new_t1, old_ti1);
                let applied_t2 = _mm256_sub_ps(new_t2, old_ti2);
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

                // ── Per-rank impulse STORE (gated by `active` only — matches the
                //    scalar UNCONDITIONAL impulse write, the velocity-vs-impulse
                //    asymmetry of Decision 6: impulse elements written for every LIVE
                //    point regardless of body movability). A `blendv` keeps an
                //    inactive lane's old value — zero on a padding lane or rank — so
                //    the store is one aligned vector per row and padding stays zero. ──
                // SAFETY: see the block's contract — an owned, aligned block row.
                unsafe {
                    store_row(&raw mut (*blk).ni, _mm256_blendv_ps(old_ni, ni, active));
                    store_row(&raw mut (*blk).ti1, _mm256_blendv_ps(old_ti1, new_t1, active));
                    store_row(&raw mut (*blk).ti2, _mm256_blendv_ps(old_ti2, new_t2, active));
                }
            }

            // ── Scatter-ONCE (cohort exit): A/B velocity registers → body rows ───
            // SAFETY: stores write 8 `f32` into in-bounds `[f32; 8]` stack buffers.
            store3(&a_lin, &mut out_a_lin);
            store3(&a_ang, &mut out_a_ang);
            store3(&b_lin, &mut out_b_lin);
            store3(&b_ang, &mut out_b_ang);
            // A is written for every live lane (always-dynamic by convention; a
            // STATIC-A lane was masked off in `mask_a`, so its registers held the
            // unchanged gathered velocity — writing it back is a no-op of the gathered
            // value, but to be safe against a shared static A across cohorts we skip
            // it). SOUNDNESS: writing only MOVABLE rows keeps cohorts disjoint (a
            // shared static row is never written, matching the scalar `*_movable`
            // guard). Padding lanes are never scattered.
            for lane in 0..nlanes {
                if a_static_s[lane] == 0.0 {
                    let b = body_mut(bodies_eff, ia[lane]);
                    b.linear_velocity = Vec3::new(out_a_lin[0][lane], out_a_lin[1][lane], out_a_lin[2][lane]);
                    b.angular_velocity = Vec3::new(out_a_ang[0][lane], out_a_ang[1][lane], out_a_ang[2][lane]);
                }
                // B written only when it is a real, movable dynamic body (not a
                // sentinel, not a static) — the disjoint-write soundness anchor.
                if !sent[lane] && is_dynamic_row(b_invm_s[lane]) {
                    let b = body_mut(bodies_eff, ib[lane]);
                    b.linear_velocity = Vec3::new(out_b_lin[0][lane], out_b_lin[1][lane], out_b_lin[2][lane]);
                    b.angular_velocity = Vec3::new(out_b_ang[0][lane], out_b_ang[1][lane], out_b_ang[2][lane]);
                }
            }
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
    /// - `false` (O5): each color's groups are solved in ascending order on the
    ///   calling thread — BYTE-IDENTICAL to the committed O5 colored solve (the O6
    ///   0%-gate). This is the path taken when [`PhysicsConfig::parallel_solve`] is
    ///   off, when the step's widest color is under `MIN_PARALLEL_SLOTS_PER_COLOR`,
    ///   OR when the running thread's [`ThreadPool`](boyko_threadpool::ThreadPool)
    ///   is absent or has one worker (the whole-step gate in `solve_colored_inner`).
    /// - `true` (O6): each color's manifold-GROUPS are partitioned into disjoint
    ///   worker chunks and dispatched across the ambient pool via `pool.scope`; the
    ///   scope-Drop join is the barrier BEFORE the next color (color `c + 1` may read
    ///   bodies color `c` wrote). See [`solve_color_parallel`](Self::solve_color_parallel).
    ///
    /// The single-threaded order within a group's ranks is preserved in BOTH paths
    /// (a worker solves its chunk's groups in ascending order, exactly as O5 does),
    /// and distinct groups in a color touch DISJOINT dynamic bodies, so the
    /// parallel result is bit-identical to the sequential one for any worker count
    /// (see [`solve_color_parallel`](Self::solve_color_parallel)).
    #[allow(clippy::too_many_arguments)]
    fn solve_all_colors(
        cols: &CohortColumns,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        parallel: bool,
        simd: bool,
    ) {
        // B4 re-create-before-view-live: the tables are FROZEN by now (the last
        // build-time grow happened in `build_columns`, before the substep loop), so
        // the worker-facing `CohortSolveView` captures stable raw bases that cannot
        // dangle while a view is live. The view is `Copy` — each worker gets a copy.
        let view = cols.solve_view();
        let color_offsets = cols.color_offsets();
        let color_group_start = cols.color_group_start();
        let color_cohort_start = cols.color_cohort_start();
        let n_colors = color_offsets.len().saturating_sub(1);
        for c in 0..n_colors {
            // Profiling: one span per color, on this (the calling) thread and never inside a
            // worker's chunk task, classed by the inline floor's own predicate so the class
            // does not depend on `parallel` or on the worker count.
            let _color_zone =
                if color_offsets[c + 1] - color_offsets[c] < MIN_PARALLEL_SLOTS_PER_COLOR {
                    zone!(PHYS_COLOR_NARROW)
                } else {
                    zone!(PHYS_COLOR_WIDE)
                };
            // The color's place in the tables: its first group and first cohort.
            let ctx = ColorCtx {
                g_base: color_group_start[c] as usize,
                k_base: color_cohort_start[c] as usize,
            };
            let g_hi = color_group_start[c + 1] as usize;
            if parallel {
                Self::solve_color_parallel(
                    cols,
                    view,
                    bodies_eff,
                    c,
                    ctx,
                    g_hi,
                    bias_rate,
                    mass_coeff,
                    impulse_coeff,
                    bias_active,
                    simd,
                );
            } else {
                // O7 dispatch fork (the 0%-gate): `simd == false` runs the byte-
                // identical scalar oracle `solve_color`; `simd == true` runs the
                // AVX2 cohort kernel over the color's cohorts (the bit-exact
                // width-only path). The non-parallel SIMD path solves the WHOLE
                // color's cohorts on the calling thread.
                Self::solve_color_dispatch(
                    view,
                    bodies_eff,
                    ctx,
                    ctx.g_base,
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
    /// # Granularity (C1): MANIFOLD-GROUP, never point
    ///
    /// Dispatch is at MANIFOLD-GROUP granularity, NOT point granularity. A color's
    /// groups are `g in color_group_start[c]..color_group_start[c + 1]`, each group
    /// `g` holds `group_start[g + 1] - group_start[g]` points, and a color's groups
    /// occupy a CONTIGUOUS block of `group_start` (the layout appends them in order),
    /// so a chunk of consecutive groups is a contiguous lane run of the color's
    /// cohorts — the shape [`solve_color`](Self::solve_color) consumes. All points
    /// of one manifold-group stay on ONE worker (they share both bodies and are
    /// order-coupled — they MUST be solved sequentially within the group).
    ///
    /// # Chunk count + balance (the work-stealing perf knob)
    ///
    /// The color is split into `num_threads() * `[`CHUNKS_PER_WORKER`] chunks
    /// (capped at the group count), balanced by total POINT count rather than group
    /// count — the dispatch loop walks groups accumulating points and cuts a chunk
    /// once its run reaches the per-chunk point quota. The lane count is
    /// `num_threads()`, never `+ 1` (KE16 App-1): the thread that calls `pool.scope`
    /// on the production route IS one of the W workers. Emitting MORE, smaller,
    /// work-balanced chunks than lanes lets the Chase-Lev work-STEALING pool
    /// equalize the lanes (an idle lane steals the next chunk), which removes the
    /// load imbalance of O6's ORIGINAL coarse split (one chunk per lane, historical
    /// — see [`CHUNKS_PER_WORKER`]). The chunk count + shape are
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
    ///   `BodyEffective` rows (and pairwise-disjoint impulse lanes) — no shared
    ///   write, no data race, no atomics needed.
    /// - **Order-independent per body:** a body's converged velocity is the result
    ///   of solving its one group's points in sequence; which worker runs that group
    ///   and in what order the groups are visited cannot change that result.
    /// - **Within-group order preserved:** a worker solves its chunk's groups' ranks
    ///   in ascending order, exactly as O5 does, so each group's order-coupled
    ///   points are solved in the SAME sequence as single-threaded.
    /// - **Barrier between colors:** the scope-Drop join completes color `c` before
    ///   color `c + 1` starts (cross-color Gauss-Seidel order is fixed).
    /// - **Worker-count-independent warm store:** after the solve each manifold's
    ///   converged impulses are written to the warm store's record for its MANIFOLD
    ///   INDEX ([`store_and_swap`](Self::store_and_swap), L11 D3) — a layout that is
    ///   a pure function of the stream, so next step's seeds do not depend on the
    ///   dispatch / thread count.
    ///
    /// Hence the per-body result — and the full body snapshot — is BIT-FOR-BIT
    /// identical to the single-threaded colored solve for ANY worker count. Any
    /// deviation from bit-identity is a bug (a shared write, a non-disjoint chunk, a
    /// missing barrier, or a float-reduction-order dependence).
    ///
    /// # O7 cohort-snapping (Decision 7)
    ///
    /// When `simd`, the chunk boundaries are SNAPPED to cohort (8-group) boundaries
    /// so every task solves only whole cohorts (the last cohort of the COLOR may be
    /// partial — handled by the masked kernel; the last cohort of a TASK is always
    /// whole). Each task routes through
    /// [`solve_color_dispatch`](Self::solve_color_dispatch) with its chunk's group
    /// range, so a worker runs [`solve_color_avx2`](Self::solve_color_avx2) over its
    /// cohorts and OWNS their heads and blocks. Cohorts within a color are
    /// body-disjoint (each is 8 disjoint groups; distinct cohorts are pairwise
    /// disjoint), so cross-worker disjointness — and thus the {1, N}×{simd}
    /// bit-identity — is unchanged from O6.
    #[allow(clippy::too_many_arguments)]
    fn solve_color_parallel(
        cols: &CohortColumns,
        view: CohortSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        color: usize,
        ctx: ColorCtx,
        g_hi: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        simd: bool,
    ) {
        // The color's manifold-group range (indices into `group_start`).
        let color_offsets = cols.color_offsets();
        let g_lo = ctx.g_base;
        let n_groups = g_hi - g_lo;
        if n_groups == 0 {
            return;
        }

        let span = (color_offsets[color] as usize, color_offsets[color + 1] as usize);

        // W1 min-work threshold: a SMALL color does not amortize a `pool.scope`
        // dispatch (a boxed shared frame + a boxed closure per spawn), so solve it
        // INLINE on the calling thread — the EXACT `solve_color` over the whole
        // color the no-pool / `parallel_solve == false` path uses. This is
        // BIT-IDENTICAL to the parallel split (within a color the groups touch
        // disjoint dynamic bodies ⇒ inline == 1-worker == N-worker), so it changes
        // only WHERE the color is solved, never the bits. The metric is the color's
        // total point count (the span width). Bounding `scope` to large colors keeps
        // the residual scope allocation at the justified, threshold-bounded
        // parallelism cost (one scope per genuinely-parallel unit, the `par_iter`
        // per-dispatch cost class) instead of one-per-tiny-color.
        let color_slots = (span.1 - span.0) as u32;
        if color_slots < MIN_PARALLEL_SLOTS_PER_COLOR {
            // Inline on the calling thread, routed through the O7 dispatch fork:
            // `simd` runs `solve_color_avx2` over the whole color's cohorts, else the
            // scalar oracle over its groups — both bit-identical to the parallel split.
            Self::solve_color_dispatch(
                view,
                bodies_eff,
                ctx,
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
            // chunk). KE16 App-1: the lane pool is `num_threads()`, never `+ 1` — on
            // the production route (`Schedule::run`'s `install` frame) the thread
            // that calls `pool.scope` IS one of the W workers, so counting it as an
            // extra lane over-chunked by a lane's worth; the `+ 1` described the
            // BENCH route, where an external joiner is an extra lane whose share is
            // measured, not counted. Target `num_threads() * CHUNKS_PER_WORKER`
            // chunks, capped at the group count (a chunk is always ≥ 1 WHOLE
            // manifold-group — never split a group's order-coupled points across
            // lanes, the C1 invariant). At least one chunk always (`max(1)`).
            // Bit-identity is chunk-COUNT- AND chunk-SHAPE-independent (the {1, N}
            // property holds for ANY partition — distinct chunks touch disjoint
            // dynamic bodies), so this is a pure, bench-tunable perf knob, never a
            // value change.
            let lanes = pool.num_threads();
            // Work-bounded, not just lane-bounded: a chunk must carry at least
            // `MIN_SLOTS_PER_CHUNK` points, or the split costs more in boxed
            // closures than the extra lane returns. See the const for the measured
            // failure this floor removes.
            let by_lanes = lanes * CHUNKS_PER_WORKER;
            let by_work = ((span.1 - span.0) / MIN_SLOTS_PER_CHUNK).max(1);
            let n_chunks = by_lanes.min(by_work).clamp(1, n_groups);

            // A ONE-chunk dispatch spawns a single task and then waits for it —
            // strictly worse than running the color on this thread, since the
            // caller is a worker and would otherwise be doing the work itself.
            // `MIN_PARALLEL_SLOTS_PER_COLOR` cannot express this: it is a floor on
            // the COLOR, while whether a color yields more than one chunk depends
            // on the work floor and the group count too. Refuse here and let the
            // caller take the inline path.
            if n_chunks < 2 {
                return false;
            }

            // Balance by total POINT count (work), not group count: groups vary in
            // width (1..=MAX_CONTACT_POINTS points), so an equal-GROUP split is
            // work-imbalanced. `target` is the per-chunk point quota; the dispatch
            // loop walks groups accumulating points and cuts a chunk once its run
            // reaches `target` (a contiguous group range). Computed from the CSR with
            // NO per-step Vec of chunk bounds (W2: the chunk boundaries are derived on
            // the fly, alloc-free).
            let total_slots = span.1 - span.0;
            let target = total_slots.div_ceil(n_chunks).max(1);

            // Send + Sync wrapper: the cohort SOLVE VIEW (Copy, raw bases only) + the
            // bodies SOLVE VIEW (Copy, row-ptr-only). Each worker writes only its
            // chunk's DISJOINT impulse lanes and DISJOINT body rows, reaching every
            // block element via a raw projection and every body row via
            // `bodies.row_ptr(i)` — NO whole-buffer reborrow on any path (the P2 + P1
            // structural fix; the prior `cols: *mut ContactColumns` + `columns()`
            // `&mut *self.cols` whole-struct reborrow that caused the rigid
            // Tree-Borrows race is DELETED). The `group_start` CSR base rides inside
            // `view`, read via `view.group_start_at` — no `&[u32]` borrow into `cols`
            // is ever held across the scope (TB-clean, Phase 9.3c discipline).
            let ptrs = ColorSolvePtrs {
                bodies: bodies_eff,
                view,
            };

            // One chunk cut: `(group lo, group hi)`.
            type ColorChunkCut = (usize, usize);

            // The cut walk as a lazy iterator. The boundaries are data-dependent
            // (they follow the CSR's point runs), so they are derived on the fly as
            // the dispatch loop consumes them — never materialized into a per-step
            // Vec of chunk bounds (W2), and never available in closed form.
            let cuts = || {
                let mut chunk_g_lo = g_lo;
                core::iter::from_fn(move || -> Option<ColorChunkCut> {
                    if chunk_g_lo >= g_hi {
                        return None;
                    }
                    // The chunk's first group's first point. Read via the view's raw
                    // `group_start` base (no `&` borrow into `cols`).
                    // SAFETY: `chunk_g_lo` is within `[g_lo, g_hi)`, a valid index
                    //   into the live `group_start` column.
                    let chunk_start = unsafe { ptrs.view.group_start_at(chunk_g_lo) } as usize;

                    // Grow the chunk by WHOLE groups until its accumulated point run
                    // reaches the per-chunk quota `target` (work-balanced) or the
                    // color's last group is consumed. A group is never split: the
                    // chunk boundary always falls on a group index, so every point of
                    // one manifold-group stays on ONE lane (C1). Always includes ≥ 1
                    // group (the first), so it makes progress.
                    //
                    // O7 cohort-snapping (Decision 7): when `simd`, advance in steps
                    // of `COHORT` (8 groups) and clamp at `g_hi`, so every chunk
                    // boundary falls on a cohort boundary (a multiple-of-8 group
                    // offset from `g_lo`) — every task thus solves only whole
                    // cohorts (plus the color's single possibly-partial trailing
                    // cohort, on whichever task owns the last group) and OWNS their
                    // heads and blocks. Bit-identity is chunk-shape-independent
                    // (cohorts are pairwise body-disjoint), so snapping is a pure
                    // perf knob.
                    let step = if simd { COHORT } else { 1 };
                    let mut chunk_g_hi = (chunk_g_lo + step).min(g_hi);
                    while chunk_g_hi < g_hi {
                        // SAFETY: `chunk_g_hi <= g_hi`, a valid `group_start` index.
                        let so_far =
                            unsafe { ptrs.view.group_start_at(chunk_g_hi) } as usize - chunk_start;
                        if so_far >= target {
                            break;
                        }
                        chunk_g_hi = (chunk_g_hi + step).min(g_hi);
                    }

                    debug_assert!(
                        chunk_g_lo < chunk_g_hi && chunk_g_hi <= g_hi,
                        "invariant: a group-chunk is non-empty and lies within the color"
                    );
                    debug_assert!(
                        !simd || (chunk_g_lo - g_lo).is_multiple_of(COHORT),
                        "invariant: a SIMD chunk's lo boundary is a cohort (8-group) boundary"
                    );

                    let cut = (chunk_g_lo, chunk_g_hi);
                    chunk_g_lo = chunk_g_hi;
                    Some(cut)
                })
            };

            pool.scope(|scope| {
                // The chunk task: a `Fn` over a cut that hands back that chunk's
                // body, one spawn per cut. Every capture is `Copy` (the solve
                // views, the color context and the step scalars), so each body owns
                // its copies and borrows nothing.
                let task = move |cut: ColorChunkCut| {
                    let (task_g_lo, task_g_hi) = cut;
                    move || {
                        // DISJOINTNESS (the O6 + P2 soundness argument — why the
                        // concurrent per-element accesses are race- and TB-clean):
                        //   - `ptrs` carries only `Copy` solve views (`CohortSolveView`
                        //     + body `ScratchSolveView`) whose raw bases name columns
                        //     borrowed for the whole `solve_color_parallel` frame;
                        //     `pool.scope`'s Drop blocks (work-stealing join) until every
                        //     spawned task completes, so every base outlives every task —
                        //     no use-after-free, no escape past the borrow, and (B4) no
                        //     regrow moves a base while a view is live.
                        //   - This chunk solves ONLY the groups `[task_g_lo, task_g_hi)`
                        //     and writes ONLY those groups' lanes of the impulse rows
                        //     (a raw single-element projection on the scalar path; an
                        //     owned whole cohort's rows on the SIMD path). Distinct chunks
                        //     have non-overlapping group ranges (they partition the
                        //     color's groups), so no two workers write the same element
                        //     and no `&mut`/store ever spans another worker's lane.
                        //   - Within ONE color, each DYNAMIC body belongs to at most one
                        //     manifold-group (the O4 coloring invariant), so distinct
                        //     chunks' groups touch DISJOINT dynamic body rows — no two
                        //     workers `apply_impulse` to the same dynamic `BodyEffective`
                        //     (each reached per-element via `bodies.row_ptr`, never a
                        //     whole-buffer reborrow).
                        //   - A SHARED static body (a ground floor several groups in this
                        //     color reference) is NEVER WRITTEN: the `*_movable` guard in
                        //     `solve_color` skips the `apply_impulse` for any
                        //     `inv_mass == 0` row (already a value no-op), so a shared
                        //     static row is read-only across workers. Sentinel body B is
                        //     likewise never written (`IMMOVABLE_AT_REST`, a local copy).
                        //   - The tables + bodies a chunk READS are its own lanes' head
                        //     constants (never written during a solve), its own lanes'
                        //     block elements, its own rows plus shared read-only static
                        //     rows, and the read-only `group_start` CSR, so no chunk reads
                        //     an element another chunk is writing. A padding lane of a
                        //     partial cohort reads no body row at all (review O2).
                        //   The DELETED `cols: *mut ContactColumns` + `columns()`
                        //   (`&mut *self.cols`) whole-struct reborrow — the rigid TB race
                        //   surface — is gone and un-typeable from `ptrs`. O7: when
                        //   `simd`, the worker runs `solve_color_avx2` over its cohorts
                        //   (`ctx.cohorts_of`) — a cohort packs 8 disjoint groups, so
                        //   distinct workers' cohort-runs still touch DISJOINT dynamic
                        //   rows + DISJOINT blocks, statics/sentinels never-written; the
                        //   disjointness argument is UNCHANGED from the scalar chunk
                        //   dispatch above.
                        Self::solve_color_dispatch(
                            ptrs.view,
                            ptrs.bodies,
                            ctx,
                            task_g_lo,
                            task_g_hi,
                            bias_rate,
                            mass_coeff,
                            impulse_coeff,
                            bias_active,
                            simd,
                        );
                    }
                };

                for cut in cuts() {
                    scope.spawn(task(cut));
                }
            });
            true
        });

        // PAR-fallback: no pool attached → run the color single-threaded, routed
        // through the O7 dispatch fork so `simd` still widens (over the whole
        // color's cohorts) and `!simd` is BYTE-IDENTICAL to O5 (the same
        // `solve_color` over the whole color's groups).
        // `None` = no pool attached; `Some(false)` = a pool was attached but the
        // color did not split into two or more chunks, so dispatching it would have
        // been one task and a wait. Both take the inline path.
        if dispatched != Some(true) {
            Self::solve_color_dispatch(
                view,
                bodies_eff,
                ctx,
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
    /// Walks the cohorts group-major in color order — the slot order (the bodies
    /// within a color are disjoint; cross-color the result is order-fixed). A
    /// zero-restitution contact is skipped, and only there is `vn0` unread (D9).
    fn apply_restitution(
        cols: &mut CohortColumns,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        counters: &mut SetupCounters,
    ) {
        // Single-threaded (run after the parallel solve has joined), so direct
        // `&mut` access to the blocks is sound — `cold` / `rank_cold` are ST-only
        // columns absent from the worker-facing view.
        let CohortColumns { heads, blocks, cold, rank_cold, .. } = cols;
        let heads = heads.as_read_slice();
        let cold = cold.as_read_slice();
        let vn0 = rank_cold.as_read_slice();
        let mut blocks = blocks.build_view();
        let blocks = blocks.as_mut_slice();
        for (head, cold) in heads.iter().zip(cold) {
            let rank_base = head.rank_base as usize;
            for l in 0..head.nlanes as usize {
                let restitution = cold.restitution[l];
                if restitution <= 0.0 {
                    continue;
                }
                let normal = head.normal(l);
                let ia = head.body_a[l] as usize;
                let b_is_sentinel = head.is_sentinel(l);
                let ib = head.body_b[l] as usize;
                let bb_view = || -> BodyEffective {
                    if b_is_sentinel { IMMOVABLE_AT_REST } else { body_copy(bodies_eff, ib) }
                };
                for r in 0..head.width[l] as usize {
                    let vn0 = vn0[rank_base + r][l];
                    if vn0 > -RESTITUTION_THRESHOLD {
                        continue;
                    }
                    counters.restitution();
                    let blk = &mut blocks[rank_base + r];
                    let ra = blk.ra(l);
                    let rb = blk.rb(l);
                    let m_eff = {
                        let ba = body_copy(bodies_eff, ia);
                        let bb = bb_view();
                        effective_mass(normal, ra, rb, &ba, &bb)
                    };
                    let vn = {
                        let ba = body_ref(bodies_eff, ia);
                        let bb = bb_view();
                        (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
                    };
                    let v_target = -restitution * vn0;
                    let d_lambda = m_eff * (v_target - vn);
                    let lambda_n = blk.ni[l];
                    let new_lambda = (lambda_n + d_lambda).max(0.0);
                    // Same `±0` invariant as `solve_color`'s clamp: this impulse is the
                    // next step's warm seed, which the AVX2 kernel clamps with a
                    // `+0`-on-tie `max`.
                    debug_assert!(
                        new_lambda.to_bits() != 0x8000_0000,
                        "invariant: the restitution clamp never yields -0 (the SIMD ±0-tie proof)"
                    );
                    let applied = new_lambda - lambda_n;
                    blk.ni[l] = new_lambda;
                    let impulse = normal * applied;
                    // Same movability guard as `solve_color` / `warm_start_apply` — see
                    // the `ia_movable` rationale in `solve_color`. Bit-identical today (an
                    // impulse on an immovable row is a value no-op), and the precondition
                    // for ever running this pass color-partitioned: without it, lanes
                    // sharing a static `body_b` would all write the same row.
                    if is_dynamic_row(body_ref(bodies_eff, ia).inv_mass) {
                        body_mut(bodies_eff, ia).apply_impulse(ra, impulse * -1.0);
                    }
                    if !b_is_sentinel && is_dynamic_row(body_ref(bodies_eff, ib).inv_mass) {
                        body_mut(bodies_eff, ib).apply_impulse(rb, impulse);
                    }
                }
            }
        }
    }

    /// The store (L11 D3): writes every manifold's record into the warm store's
    /// write side by manifold index, then swaps the sides.
    ///
    /// Two passes. The cohorts first, in color order: each live lane's converged
    /// impulses go into its manifold's record (`cold.mi`) — the record's shape (its
    /// feature ids and point count) was written by the fill, so only the impulse
    /// values are touched; the writes land on one ascending stream of 64 B records
    /// per color. Then the tags, in manifold order: a manifold FROZEN this step (B1)
    /// carries — each of its live points looked up by feature id in the read side
    /// through the run `plan_sources` found for it, the hits compacted into the
    /// record, a miss dropping its point (a pair whose rows cannot be translated, a
    /// `Reset`, a feature id that changed) — so the island wakes with the impulses
    /// it froze with; a manifold with no live point stores an empty record. The
    /// layout is a pure function of the stream and the solved values, never of the
    /// color layout or (in [O6]) the thread count, and the write side's keys were
    /// written in stream order by `plan_sources`.
    ///
    /// After the swap it stamps the warm cursor with `rows`: the read side has just
    /// been written from this step's cohorts and carried records, keyed by this
    /// gather's rows, and this is its only writer (defect A, interim). A disabled
    /// solver neither stores nor stamps.
    ///
    /// `restore` is L10's restore warm source (design 06 A3): a frozen manifold that P-a
    /// marked `SRC_RESTORE` carries from it (design 08 A3′, mutation N28).
    ///
    /// [O6]: https://github.com/bluesteelll/boyko-engine
    fn store_and_swap(
        &mut self,
        rows: &RowIdentity,
        manifolds: &[Manifold],
        restore: Option<&WarmRecords>,
    ) {
        if !self.warm_start_enabled {
            return;
        }
        let cols = &self.columns;
        let (read, write) = Self::warm_sides(&mut self.warm, self.warm_cur);
        let index = &self.warm_index;
        let tags = cols.tags();
        let plan = cols.plan();
        let sources = WarmSources {
            lookup: WarmLookup { read, index },
            restore: restore.map(|read| WarmLookup { read, index }),
            plan,
            tags,
        };
        debug_assert!(
            tags.len() == manifolds.len() && plan.len() == manifolds.len(),
            "invariant: tags and plan hold one row per manifold of this step"
        );
        let blocks = cols.blocks();
        let mut carry_hits = 0u32;
        {
            let mut recs_w = write.recs_mut();
            let recs_w = recs_w.as_mut_slice();
            debug_assert_eq!(recs_w.len(), manifolds.len(), "invariant: one record per manifold");
            for (head, cold) in cols.heads().iter().zip(cols.cold.as_read_slice()) {
                let rank_base = head.rank_base as usize;
                for l in 0..head.nlanes as usize {
                    let rec = &mut recs_w[cold.mi[l] as usize];
                    debug_assert_eq!(
                        usize::from(rec.count()),
                        head.width[l] as usize,
                        "invariant: the fill wrote the solved record's shape"
                    );
                    for (p, blk) in blocks[rank_base..rank_base + head.width[l] as usize].iter().enumerate() {
                        rec.set_impulses(p, [blk.ni[l], blk.ti1[l], blk.ti2[l]]);
                    }
                }
            }
            for (mi, (m, &tag)) in manifolds.iter().zip(tags).enumerate() {
                if tag.solved() {
                    continue;
                }
                recs_w[mi] = if tag.frozen() && tag.count != 0 {
                    debug_assert_eq!(
                        tag.count,
                        m.count,
                        "invariant: a frozen tag holds its manifold's live-point count"
                    );
                    let (lookup, run) = sources.of(mi);
                    let (rec, hits) = lookup.carry(run, m, usize::from(tag.count));
                    carry_hits += hits;
                    rec
                } else {
                    WarmRecord::EMPTY
                };
            }
        }
        self.warm_stats.carry_hits = carry_hits;
        self.counters.carry(carry_hits);
        debug_assert!(
            carry_hits <= self.warm_stats.carry_points,
            "invariant: the carry keeps at most the frozen manifolds' live points"
        );
        self.warm_cur ^= 1;
        self.warm_cursor.stamp(rows);
    }

    /// Writes the solved velocities back into the gather snapshot and flags every
    /// integrated DYNAMIC row touched (mirrors the reference `write_back`).
    fn write_back(&self, scratch: &mut SolverScratch) {
        let eff = self.bodies.as_read_slice();
        let n = eff.len();
        let mut snap_view = scratch.bodies.build_view();
        let snapshot = snap_view.as_mut_slice();
        for row in 0..n {
            if snapshot[row].simulated && is_dynamic_row(eff[row].inv_mass) {
                snapshot[row].linear_velocity = eff[row].linear_velocity;
                snapshot[row].angular_velocity = eff[row].angular_velocity;
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
        let eff = self.bodies.as_read_slice();
        let n = eff.len();
        let mut snap_view = scratch.bodies.build_view();
        let snapshot = snap_view.as_mut_slice();
        for row in 0..n {
            if !sleep.is_row_awake(row) {
                continue;
            }
            if snapshot[row].simulated && is_dynamic_row(eff[row].inv_mass) {
                snapshot[row].linear_velocity = eff[row].linear_velocity;
                snapshot[row].angular_velocity = eff[row].angular_velocity;
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
    /// restitution pass, the warm store's write by manifold index
    /// ([`store_and_swap`](Self::store_and_swap), L11 D3), and the write-back. The
    /// integrate / inertia kernels are the SAME `simd::*` helpers the reference
    /// calls (no duplicated inertia math).
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
        self.solve_colored_inner(config, manifolds, graph, scratch, None, None);
    }

    /// The colored solve with O8 sleeping (plan O8 / Decision 5) — the entry the
    /// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage calls
    /// when `PhysicsConfig::sleeping` is on.
    ///
    /// Identical to [`solve_colored`](Self::solve_colored) but threads the
    /// [`IslandSleep`] state: slept islands skip ONLY their SOLVE + INTEGRATE work
    /// (the gather is untouched — IM-1), and the energy / debounce / sleep transition
    /// is advanced after the solve for next frame.
    ///
    /// A step on which no dynamic row is awake and no contact point is laid out takes the
    /// no-awake fast path (L10 C3a, [`fast_path_steps`](Self::fast_path_steps)): it skips the
    /// substep loop, the restitution pass and the freeze capture and restore, which would change
    /// no state, and still builds the columns (the store's keys and tags), carries every frozen
    /// manifold's record, swaps the store and advances the sleep state. Its values are the full
    /// path's, bit for bit.
    pub fn solve_colored_sleeping(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
        sleep: &mut IslandSleep,
    ) {
        self.solve_colored_inner(config, manifolds, graph, scratch, Some(sleep), None);
    }

    /// [`solve_colored_sleeping`](Self::solve_colored_sleeping) with L10's sleep-skip (design
    /// 04 A6, 06 A1–A3, 08 A1′): the rows the broadphase holds get the effective inverse mass
    /// `0` (D8), P-a searches the restore source after the read side (A3), and the kept
    /// manifolds moved in this step take their warm records before the swap (A1′); the logical
    /// [`WarmSeedStats`] add the held store's points and hits (E6′). The
    /// [`physics_solve_colored`](crate::systems::physics_solve_colored) stage's sleeping arm.
    pub(crate) fn solve_colored_held(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
        sleep: &mut IslandSleep,
        held: HeldSolve<'_>,
    ) {
        self.solve_colored_inner(config, manifolds, graph, scratch, Some(sleep), Some(held));
    }

    /// Shared body of the colored solve — `sleep == None` is the byte-identical
    /// O6/O7 path (the 0%-gate), `sleep == Some(_)` adds the O8 solve+integrate skip
    /// for slept islands (gather stays full — IM-1), and `held == Some(_)` L10's
    /// sleep-skip on top of it.
    fn solve_colored_inner(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        graph: &ConstraintGraph,
        scratch: &mut SolverScratch,
        mut sleep: Option<&mut IslandSleep>,
        mut held: Option<HeldSolve<'_>>,
    ) {
        let substeps = config.substeps.max(1);
        let h = config.dt / substeps as f32;

        // Defect A (interim): re-key the sleep latch to this gather's rows BEFORE the early
        // return below, so a transient step with no simulated dynamic body does not leave
        // the latch keyed by an older gather and force a wake on the next step.
        if let Some(sleep) = sleep.as_mut() {
            sleep.rekey_rows(&scratch.rows);
        }

        // O1: degenerate early-return BEFORE any build/alloc. In solver-owned mode
        // a free dynamic body must keep falling, so the only valid skip is a world
        // with no dynamic body to integrate at all — then there is nothing to
        // integrate, build, or write back. Hoisting it above `build_bodies` /
        // `build_columns` makes an idle / all-static world do zero build work.
        let has_dynamic = scratch
            .bodies()
            .iter()
            .any(|b| b.simulated && is_dynamic_row(b.inv_mass));
        if !has_dynamic {
            return;
        }
        self.solved_steps += 1;
        self.counters.step(self.warm_start_enabled);

        // O8 phase 1 (BEFORE the solve): apply the wake conditions and build the
        // body→awake mask from last frame's sleep flags. This decides which islands
        // skip the solve + integrate THIS frame. A read-only borrow of the resource
        // for the duration of the build/solve (`asleep` / `awake_rows` are read);
        // the post-solve `end_step` reborrows mutably.
        let n_rows = scratch.bodies_len();
        let sleeping_active = sleep.is_some();
        if let Some(sleep) = sleep.as_mut() {
            let _z = zone!(PHYS_SLEEP_BEGIN);
            sleep.begin_step(graph, n_rows);
            // L10 A1.1 (design 04 D9): the awake mask is now this gather's; the next
            // broadphase's resting test trusts it only with this stamp.
            sleep.stamp_mask(&scratch.rows);
        }
        // An immutable view used by `build_columns` (SOLVE skip) + the integrate
        // freeze; `None` when sleeping is off so the path is byte-identical.
        let sleep_view: Option<&IslandSleep> = sleep.as_deref();

        {
            let _z = zone!(PHYS_SOLVE_BUILD);
            let cls: &[RowCls] = held.as_ref().map_or(&[], |h| h.cls);
            self.build_bodies(scratch.bodies(), cls);
            // Ruling W1: the write guards read the effective inverse mass of the flag the
            // colouring read, so a held row is immovable to both.
            debug_assert!(
                cls.iter().zip(self.bodies.as_read_slice()).all(|(c, e)| !c.is_held() || e.inv_mass == 0.0),
                "invariant: a held row's effective inverse mass is 0 (design 04 D8, ruling W1)"
            );
            // Warm start is classified only while it is enabled: a disabled solver never
            // reads or stores the records, so `Identity` is a placeholder that takes no
            // per-manifold branch, and its cursor never counts a phantom `Reset`.
            let warm_remap = if self.warm_start_enabled {
                self.warm_cursor.remap(&scratch.rows)
            } else {
                RowRemap::Identity
            };
            let restore = held.as_ref().and_then(|h| h.restore);
            let (searches, hits) =
                self.build_columns(manifolds, graph, scratch.bodies(), sleep_view, warm_remap, restore);
            if let Some(h) = held.as_mut() {
                h.rules.restore_rec_searches += u64::from(searches);
                h.rules.restore_rec_hits += u64::from(hits);
            }
        }
        // L11 C0: the setup digest is taken here, over the seeds the sweeps have not
        // yet touched; the step's warm stats are folded in after the store.
        #[cfg(test)]
        {
            self.step_digest = self.columns.setup_digest();
        }
        // Profiling: this step's slot totals by color class, the denominators of the
        // per-color spans. One pass over the color CSR, and only while armed.
        if zone_enabled!(PHYS_SLOTS_WIDE) {
            let (wide, narrow) = self.columns.slots_by_width();
            counter!(PHYS_SLOTS_WIDE, wide);
            counter!(PHYS_SLOTS_NARROW, narrow);
        }

        // L10 C3a (design 06 D-A, 08 A1′/O10): the no-awake fast path. With sleeping on, no
        // dynamic row awake and no point laid out, the freeze capture, the substep loop, the
        // restitution pass and the freeze restore change no state: every row the loop integrates
        // is a frozen row the restore puts back (with its velocity), no cohort is swept, and the
        // write-back skips every frozen row. So they are skipped, and what does change state
        // still runs, in order: the build above (P-a's write-side keys, the tags, the empty
        // layout), the store's carry and swap and the cursor's stamp, the write-back and
        // `end_step`. A manifold with no dynamic side is solved on every step, so a layout with a
        // point keeps the full path.
        let fast = self.columns.len() == 0
            && sleep_view.is_some_and(|sleep| Self::no_awake_dynamic_row(sleep, scratch.bodies()));
        self.fast_path_steps += u64::from(fast);
        if let Some(h) = held.as_mut()
            && !h.cls.is_empty()
        {
            h.stats.fast_path = u32::from(fast);
        }

        // O8 integrate-freeze (INTEGRATE half): capture the pre-solve hot state of
        // every slept-island body so the per-substep integrate (which streams the
        // WHOLE array — the O1 SIMD kernels are NOT per-lane masked) can be UNDONE for
        // slept rows after the loop. This freezes a slept body's position / rotation /
        // velocity without touching the audited integrate kernels. `frozen` is
        // capacity-reused (empty when sleeping is off — the byte-identical path).
        if !fast {
            let mut frozen = self.frozen.build_view();
            frozen.clear();
            if let Some(sleep) = sleep_view {
                let _z = zone!(PHYS_SLEEP_FREEZE);
                let eff = self.bodies.as_read_slice();
                for (row, b) in scratch.bodies().iter().enumerate() {
                    if eff[row].inv_mass == 0.0 {
                        // Static rows are no-ops to the integrate kernels (the
                        // `inv_mass != 0` guard) — no need to snapshot them. So are the rows
                        // L10 holds (design 04 D8: their effective inverse mass is `0`).
                        continue;
                    }
                    if !sleep.is_row_awake(row) {
                        frozen.push((row as u32, *b));
                    }
                }
            }
        }

        let soft = SoftCoefficients::new(config.contact_hertz, config.contact_damping, h);
        let gravity = config.gravity;
        // O1: gates the integrate / inertia kernels (gravity, position integrate,
        // inertia refresh).
        let use_simd = config.simd;
        // O7: gates the cohort-batched colored CONTACT SOLVE — a SEPARATE flag from
        // O1's `simd` so the solve widen has independent A/B + rollback. Default ON
        // since 2026-09-18; `false` selects the scalar `solve_color` oracle, which
        // produces the same bits.
        let use_simd_solve = config.simd_solve;
        // O6: parallel per-color dispatch when opted in. The result is bit-identical
        // to the single-threaded colored solve for any worker count (disjoint-body
        // groups + a warm store written by manifold index); when off it is
        // BYTE-IDENTICAL to O5.
        //
        // P2 whole-solve dispatch gate: even with `parallel_solve` opted in, a step
        // whose WIDEST COLOR cannot clear the solver's own per-color
        // `MIN_PARALLEL_SLOTS_PER_COLOR` floor has no color worth a `pool.scope`, so
        // the whole dispatch is pure loss. Force the byte-identical single-threaded
        // path and skip the ambient-pool probe + the per-color span checks every pass.
        // This changes only WHERE the colored solve runs, NEVER the bits: the inline
        // path is the SAME `solve_color` the `parallel == false` fallback uses, and
        // the {1, N}-worker bit-identity property makes the parallel path equal to it.
        //
        // ⚠ THE METRIC WAS WRONG UNTIL NOW, AND IT WAS WRONG IN THE EXPENSIVE
        // DIRECTION. It used `graph.max_island_constraints() >=
        // LARGE_ISLAND_CONSTRAINTS`, on the stated premise that "the largest color is
        // bounded by the largest island's manifold count". It is not. A color is a set
        // of BODY-DISJOINT manifolds, and manifolds in DIFFERENT islands are always
        // body-disjoint — so `n` disjoint pairs are `n` islands of ONE manifold each
        // AND a single color of `n` slots. Island size bounds color width from below
        // not at all, and the old gate therefore forced the single-threaded path on
        // precisely the most parallel scenes the solver can be handed: every
        // many-pile, many-debris, many-ragdoll world. Regression-gated by
        // `many_disjoint_pairs_are_one_wide_color_and_must_dispatch`.
        //
        // The new metric is the quantity the per-color floor already compares against,
        // maximised over colors — read off the `color_offsets` CSR that
        // `build_columns` (above) has already filled, so it stays a single pass over
        // `n_colors + 1` u32s and no new state.
        //
        // L4 lanes term: a pool of ONE worker has nothing to parallelise with, yet the
        // per-color cut (`lanes × CHUNKS_PER_WORKER` chunks, `solve_color_parallel`)
        // still yields ≥ 2 chunks at `lanes == 1` and would open a `pool.scope` for
        // every wide color of every pass. Decided once per step here — one thread-local
        // read, never per color — so with `parallel_solve` on by default a W=1 world
        // runs exactly the path of `parallel_solve == false`. `num_threads()`, never
        // `+ 1`, for the reason `BroadphaseGrid::build_parallel` gives (KE16 App-1).
        // No pool attached ⇒ `None` ⇒ inline, as the per-color probe already did.
        // Gated red-first by `one_worker_parallel_solve_takes_the_inline_path`.
        let parallel = !fast
            && config.parallel_solve
            && self.columns.widest_color_slots() >= MIN_PARALLEL_SLOTS_PER_COLOR
            && try_with_active_pool(|pool| pool.num_threads() >= 2) == Some(true);

        // L10 C3a: the fast path runs no substep.
        let passes = if fast { 0 } else { substeps };
        for _ in 0..passes {
            // (1) Gravity integrate DYNAMIC bodies (shared O1 kernel). Single-
            // threaded — the BodyEffective build view's mut slice (no parallel
            // access in the integrate kernels).
            {
                let _z = zone!(PHYS_GRAVITY);
                let mut view = self.bodies.build_view();
                simd::apply_gravity(view.as_mut_slice(), scratch.bodies(), gravity, h, use_simd);
            }

            // (2) Warm-start apply. The colored sweeps reach bodies through the body
            // SOLVE VIEW — single-threaded here, parallel in `solve_all_colors`; the
            // body view is the SAME surface either way (the P1 structural fix: no
            // whole-buffer reborrow on any body path). The cohort tables are read
            // through shared slices: no worker is live. `use_simd_solve` forks the
            // apply's SHAPE only (C3, D7): the 8-lane apply is bit-identical to the
            // group-major oracle, and it is the same flag the sweep kernel forks on,
            // so a step never mixes the two.
            {
                let _z = zone!(PHYS_WARM_APPLY);
                Self::warm_start_apply(&self.columns, self.bodies.solve_view(), use_simd_solve);
            }

            // (3)+(4) Soft normal + friction sweep ACROSS colors (Gauss-Seidel).
            {
                let _z = zone!(PHYS_PASS_BIASED);
                Self::solve_all_colors(
                    &self.columns,
                    self.bodies.solve_view(),
                    soft.bias_rate,
                    soft.mass_coeff,
                    soft.impulse_coeff,
                    true,
                    parallel,
                    use_simd_solve,
                );
            }

            // (5) Position integrate (scalar — the reference's MEASURED-SCALAR
            // choice for the AoS `BodyState`) then refresh the world inertia. The
            // BodyEffective read slice + the BodyState mut slice are distinct
            // ScratchColumns (no borrow conflict).
            {
                let _z = zone!(PHYS_INTEGRATE);
                {
                    let mut snap_view = scratch.bodies.build_view();
                    simd::position_integrate(
                        self.bodies.as_read_slice(),
                        snap_view.as_mut_slice(),
                        h,
                        false,
                    );
                }
                {
                    let mut view = self.bodies.build_view();
                    simd::refresh_inertia(view.as_mut_slice(), scratch.bodies(), use_simd);
                }
            }

            // (6) Relax: re-solve bias-free to remove soft-bias energy.
            for _ in 0..config.relax_iterations {
                let _z = zone!(PHYS_PASS_RELAX);
                Self::solve_all_colors(
                    &self.columns,
                    self.bodies.solve_view(),
                    soft.bias_rate,
                    soft.mass_coeff,
                    soft.impulse_coeff,
                    false,
                    parallel,
                    use_simd_solve,
                );
            }
        }

        // Post-loop restitution (ONCE, velocity-only, bias-free); not on the fast path, which
        // lays out no point.
        if !fast {
            let _z = zone!(PHYS_RESTITUTION);
            Self::apply_restitution(
                &mut self.columns,
                self.bodies.solve_view(),
                &mut self.counters,
            );
        }

        // D3: store every manifold's record by index, carry the frozen manifolds'
        // points (B1), then swap the sides.
        {
            let _z = zone!(PHYS_STORE);
            // L10 A1′ (design 08 O10): the move-in capture, before the swap, on both paths.
            if let Some(h) = held.as_mut() {
                let remap = self.warm_cursor.peek(&scratch.rows);
                self.capture_moved_in(remap, h);
            }
            let restore = held.as_ref().and_then(|h| h.restore);
            self.store_and_swap(&scratch.rows, manifolds, restore);
            // L10 E6′ (design 06): the logical carry adds the held store's points and hits —
            // `Off` carries the same frozen manifolds' records every step with the same hits.
            if let Some(h) = held.as_ref()
                && self.warm_start_enabled
            {
                self.warm_stats.carry_points += h.held.live_points();
                self.warm_stats.carry_hits += *h.held_warm;
            }
        }
        #[cfg(test)]
        {
            self.step_digest = fnv_warm_stats(self.step_digest, &self.warm_stats);
        }

        // O8 integrate-freeze RESTORE: undo the integrate on slept rows by restoring
        // their captured pre-solve hot state into `scratch.bodies` (position /
        // rotation / velocities) — so a slept island advances by exactly nothing. The
        // matching `BodyEffective` velocity (which `write_back` would copy out) is also
        // restored so the slept body keeps its frozen velocity. Then `write_back` is
        // told to SKIP slept rows, so `physics_apply` leaves the live component
        // untouched (frozen) — and the gather-walked-every-row IM-1 invariant holds. The fast
        // path captured nothing and integrated nothing, so it has nothing to restore.
        if sleeping_active && !fast {
            let _z = zone!(PHYS_SLEEP_FREEZE);
            let mut snap_view = scratch.bodies.build_view();
            let snapshot = snap_view.as_mut_slice();
            let mut eff_view = self.bodies.build_view();
            let eff_rows = eff_view.as_mut_slice();
            for &(row, snap) in self.frozen.as_read_slice() {
                let r = row as usize;
                snapshot[r] = snap;
                let eff = &mut eff_rows[r];
                eff.linear_velocity = snap.linear_velocity;
                eff.angular_velocity = snap.angular_velocity;
            }
        }

        // Write the solved velocities back and flag integrated DYNAMIC rows. With
        // sleeping on, slept rows are skipped (their `awake_rows` bit is clear), so the
        // frozen rows are never flagged touched — `physics_apply` leaves them be.
        {
            let _z = zone!(PHYS_WRITE_BACK);
            if let Some(sleep) = sleep.as_deref() {
                self.write_back_awake(scratch, sleep);
            } else {
                self.write_back(scratch);
            }
        }

        // O8 phase 2 (AFTER the solve): accumulate this frame's per-island energy from
        // the (post-restore) body velocities and advance the debounce / sleep
        // transition for next frame. The mutable reborrow is sound: `sleep_view` (the
        // immutable view) is dead after the freeze capture.
        if let Some(sleep) = sleep.as_mut() {
            let _z = zone!(PHYS_SLEEP_END);
            sleep.end_step(scratch.bodies(), graph, config.sleep_threshold, config.sleep_frames);
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
#[path = "colored_tests.rs"]
mod tests;
