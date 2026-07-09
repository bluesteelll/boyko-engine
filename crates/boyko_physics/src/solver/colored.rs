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

use std::marker::PhantomData;

use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchColumn, ScratchSolveView};
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
use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::math::{Mat3, Vec3};
use crate::resources::{
    BodyState, ConstraintGraph, IslandSleep, LARGE_ISLAND_CONSTRAINTS, PhysicsConfig, SolverScratch,
};
use crate::scratch_ids::{
    body_eff_colored_id, contact_column_id, register_scratch_layouts, scratch_reserve_rows,
};

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
struct ColorSolvePtrs<'a> {
    /// Per-element bodies access via the committed [`ScratchSolveView`] — Copy,
    /// Send+Sync, row-ptr-ONLY. There is NO whole-buffer `&mut [BodyEffective]`
    /// reborrow path (the SP4 structural fix): a worker reaches a body row only
    /// through [`ScratchSolveView::row_ptr`], which yields one typed `*mut
    /// BodyEffective` per DISTINCT index its color owns.
    bodies: ScratchSolveView<'a, BodyEffective>,
    /// The worker-facing contact-column solve view (P2): `Copy + Send + Sync`,
    /// per-element raw base + index ONLY. This REPLACES the deleted `cols: *mut
    /// ContactColumns` + `columns()` (`&mut *self.cols`) whole-struct reborrow —
    /// the rigid Tree-Borrows race surface. A worker reaches a contact column
    /// element solely through `view.<accessor>(i)` (`base.add(i)`), so no `&mut`
    /// ever spans more than one row and the whole-buffer reborrow is un-typeable
    /// on the worker path.
    view: ContactSolveView<'a>,
}

// SAFETY: `ColorSolvePtrs` is `Send + Sync` because its only shared-mutable state
//   is reached PER ELEMENT through `view: ContactSolveView` and `bodies:
//   ScratchSolveView`, both of which expose mutation only as a single-row
//   `base + index` write (never a whole-buffer `&mut [_]` reborrow — the P2/P1
//   structural fix). The C2 coloring invariant guarantees two workers in one
//   parallel step never share a contact row (distinct colors => distinct rows =>
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

/// Worker-facing, color-disjoint contact solve view (audit Stage P — P2).
///
/// `Copy + Send + Sync`: the scheduler hands a COPY to each parallel worker. It
/// exposes the contact columns the colored kernels touch on the worker path as
/// per-element raw `base + index` accessors ONLY — there is NO whole-buffer
/// `&mut [_]` / slice path, so the `&mut *self.cols` whole-struct reborrow that
/// caused the rigid Tree-Borrows race is un-typeable from this view (the P2
/// structural fix, mirroring the body [`ScratchSolveView`] from P1).
///
/// # SAFETY / soundness (`Send + Sync`)
///
/// Soundness rests on the C2 coloring disjointness invariant: any two contact
/// rows touched concurrently by distinct workers belong to distinct color bands
/// and therefore distinct rows. All mutation goes through `set_*_impulse*` which
/// writes a SINGLE row via `base + index` (no `&mut` ever spans more than one
/// element), so two workers never materialize overlapping `&mut`. Read accessors
/// form a value read from a single element only. The 24 read-only bases (geometry,
/// body indices, the friction coefficient, the per-group `group_start` CSR) are
/// never written by any worker, so concurrent reads are sound.
///
/// PROVENANCE: every base — the three worker-mutable impulse bases AND the
/// read-only ones — is a raw write-capable base derived from
/// [`ScratchColumn::solve_base`] (i.e. `ComponentPool::buffer_ptr().cast_mut()`,
/// provenance-preserving, NO `&[_]` interposed). The impulse `*mut f32` bases
/// therefore carry WRITE provenance, so the per-row `*base.add(i) = v` writes are
/// Tree-Borrows-clean (the C1 fix; the prior `as_read_slice().as_ptr().cast_mut()`
/// branded them Frozen / SharedReadOnly — UB to write through). The read-only
/// bases are `*const _` reborrows of the same write-capable raw base, which is
/// sound to read through. The bases are address-stable (backed by `ComponentPool`
/// reservations that never realloc-move) and captured AFTER the last build-time
/// grow (the B4 re-create-before-view-live discipline), so they cannot dangle
/// while a view is live.
#[derive(Clone, Copy)]
struct ContactSolveView<'a> {
    // ── read-only geometry bases (per contact-point slot) ──
    ra_x: *const f32,
    ra_y: *const f32,
    ra_z: *const f32,
    rb_x: *const f32,
    rb_y: *const f32,
    rb_z: *const f32,
    normal_x: *const f32,
    normal_y: *const f32,
    normal_z: *const f32,
    tangent1_x: *const f32,
    tangent1_y: *const f32,
    tangent1_z: *const f32,
    tangent2_x: *const f32,
    tangent2_y: *const f32,
    tangent2_z: *const f32,
    separation: *const f32,
    friction: *const f32,
    // ── read-only body indices / sentinel flag ──
    body_a: *const u32,
    body_b: *const u32,
    b_is_sentinel: *const bool,
    // ── worker-mutable impulse bases ──
    normal_impulse: *mut f32,
    tangent1_impulse: *mut f32,
    tangent2_impulse: *mut f32,
    // ── read-only per-group CSR base (worker AVX2 kernel navigates groups) ──
    group_start: *const u32,
    /// Slot count (the exclusive index ceiling for the per-slot accessors).
    len: usize,
    /// Binds the view to the column borrow so it cannot outlive a refill / regrow.
    _marker: PhantomData<&'a ()>,
}

// SAFETY: see the struct-level soundness note — the C2 coloring disjointness
//   invariant + per-element-only mutation + address-stable bases. No `&mut` ever
//   spans more than one element, so concurrent worker access never aliases.
unsafe impl Send for ContactSolveView<'_> {}
// SAFETY: see the `Send` impl above — shared access yields per-element reads /
//   single-row writes only (no whole-buffer slice), and disjoint color bands mean
//   concurrent accessors target disjoint memory.
unsafe impl Sync for ContactSolveView<'_> {}

impl<'a> ContactSolveView<'a> {
    /// The contact-point slot count.
    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    /// Reads `group_start[g]` (the per-group CSR boundary). Used by the AVX2 worker
    /// kernel to navigate the manifold-group ranges and by the dispatcher to cut
    /// work-balanced chunks.
    ///
    /// # Safety
    /// `g` must index the live `group_start` column (`g <= n_groups`); upheld by
    /// the dispatcher / kernel, which read only group indices within the color's
    /// `[g_lo, g_hi]` range.
    #[inline]
    unsafe fn group_start_at(&self, g: usize) -> u32 {
        // SAFETY: `g` is in range per the method contract; `group_start` is the live
        //   base of the `group_start` ScratchColumn. A plain `*const u32` read forms
        //   no reference spanning the column, so it never conflicts with a worker's
        //   per-element impulse write (the Tree-Borrows discipline).
        unsafe { *self.group_start.add(g) }
    }

    // ── scalar per-element reads (single-row `*base.add(i)` provenance) ──

    /// Reads slot `i`'s body-A anchor as a [`Vec3`].
    #[inline]
    fn ra(&self, i: usize) -> Vec3 {
        // SAFETY: `i < len` (the kernel iterates `[start, end) ⊆ [0, len)`); the
        //   three bases point at the live `i`-th element on address-stable columns.
        unsafe { Vec3::new(*self.ra_x.add(i), *self.ra_y.add(i), *self.ra_z.add(i)) }
    }

    /// Reads slot `i`'s body-B anchor as a [`Vec3`].
    #[inline]
    fn rb(&self, i: usize) -> Vec3 {
        // SAFETY: `i < len`; address-stable bases, live `i`-th element.
        unsafe { Vec3::new(*self.rb_x.add(i), *self.rb_y.add(i), *self.rb_z.add(i)) }
    }

    /// Reads slot `i`'s contact normal as a [`Vec3`].
    #[inline]
    fn normal(&self, i: usize) -> Vec3 {
        // SAFETY: `i < len`; address-stable bases, live `i`-th element.
        unsafe { Vec3::new(*self.normal_x.add(i), *self.normal_y.add(i), *self.normal_z.add(i)) }
    }

    /// Reads slot `i`'s first friction tangent as a [`Vec3`].
    #[inline]
    fn tangent1(&self, i: usize) -> Vec3 {
        // SAFETY: `i < len`; address-stable bases, live `i`-th element.
        unsafe {
            Vec3::new(*self.tangent1_x.add(i), *self.tangent1_y.add(i), *self.tangent1_z.add(i))
        }
    }

    /// Reads slot `i`'s second friction tangent as a [`Vec3`].
    #[inline]
    fn tangent2(&self, i: usize) -> Vec3 {
        // SAFETY: `i < len`; address-stable bases, live `i`-th element.
        unsafe {
            Vec3::new(*self.tangent2_x.add(i), *self.tangent2_y.add(i), *self.tangent2_z.add(i))
        }
    }

    // ── single-column scalar reads (the AVX2 SoA gather marshals these per lane;
    //    only the avx2-gated `solve_color_avx2` consumes them, so they are
    //    dead-code-allowed on a non-AVX2 build) ──

    /// Reads slot `i`'s `ra_x` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn ra_x(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.ra_x.add(i) }
    }
    /// Reads slot `i`'s `ra_y` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn ra_y(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.ra_y.add(i) }
    }
    /// Reads slot `i`'s `ra_z` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn ra_z(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.ra_z.add(i) }
    }
    /// Reads slot `i`'s `rb_x` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn rb_x(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.rb_x.add(i) }
    }
    /// Reads slot `i`'s `rb_y` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn rb_y(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.rb_y.add(i) }
    }
    /// Reads slot `i`'s `rb_z` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn rb_z(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.rb_z.add(i) }
    }
    /// Reads slot `i`'s `normal_x` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn normal_x(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.normal_x.add(i) }
    }
    /// Reads slot `i`'s `normal_y` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn normal_y(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.normal_y.add(i) }
    }
    /// Reads slot `i`'s `normal_z` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn normal_z(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.normal_z.add(i) }
    }
    /// Reads slot `i`'s `tangent1_x` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent1_x(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent1_x.add(i) }
    }
    /// Reads slot `i`'s `tangent1_y` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent1_y(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent1_y.add(i) }
    }
    /// Reads slot `i`'s `tangent1_z` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent1_z(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent1_z.add(i) }
    }
    /// Reads slot `i`'s `tangent2_x` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent2_x(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent2_x.add(i) }
    }
    /// Reads slot `i`'s `tangent2_y` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent2_y(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent2_y.add(i) }
    }
    /// Reads slot `i`'s `tangent2_z` component.
    #[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
    #[inline]
    fn tangent2_z(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.tangent2_z.add(i) }
    }

    /// Reads slot `i`'s signed separation.
    #[inline]
    fn separation(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.separation.add(i) }
    }

    /// Reads slot `i`'s combined friction coefficient.
    #[inline]
    fn friction(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.friction.add(i) }
    }

    /// Reads slot `i`'s dense body-A row index.
    #[inline]
    fn body_a(&self, i: usize) -> u32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.body_a.add(i) }
    }

    /// Reads slot `i`'s dense body-B row index.
    #[inline]
    fn body_b(&self, i: usize) -> u32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.body_b.add(i) }
    }

    /// Reads slot `i`'s sentinel flag (`true` = SDF contact, B is immovable).
    #[inline]
    fn b_is_sentinel(&self, i: usize) -> bool {
        // SAFETY: `i < len`; address-stable base, live `i`-th element.
        unsafe { *self.b_is_sentinel.add(i) }
    }

    // ── impulse reads (single-row) ──

    /// Reads slot `i`'s accumulated normal impulse.
    #[inline]
    fn normal_impulse(&self, i: usize) -> f32 {
        // SAFETY: `i < len`; address-stable base, live `i`-th element. A concurrent
        //   write by another worker targets a DISTINCT row (the C2 invariant).
        unsafe { *self.normal_impulse.add(i) }
    }

    /// Reads slot `i`'s accumulated first tangent impulse.
    #[inline]
    fn tangent1_impulse(&self, i: usize) -> f32 {
        // SAFETY: as `normal_impulse` — `i < len`, distinct-row concurrency.
        unsafe { *self.tangent1_impulse.add(i) }
    }

    /// Reads slot `i`'s accumulated second tangent impulse.
    #[inline]
    fn tangent2_impulse(&self, i: usize) -> f32 {
        // SAFETY: as `normal_impulse` — `i < len`, distinct-row concurrency.
        unsafe { *self.tangent2_impulse.add(i) }
    }

    // ── impulse writes (single-row, `base + index`, never spanning rows) ──

    /// Writes slot `i`'s accumulated normal impulse.
    #[inline]
    fn set_normal_impulse(&self, i: usize, v: f32) {
        // SAFETY: `i < len`; the C2 coloring invariant grants row `i` to AT MOST ONE
        //   worker, so this single-element write never aliases another worker's. The
        //   base is address-stable and carries WRITE provenance — it is the raw
        //   `ScratchColumn::solve_base` (`ComponentPool::buffer_ptr().cast_mut()`),
        //   never an `&[f32]`-Frozen reborrow — so `*base.add(i) = v` is TB-clean.
        unsafe { *self.normal_impulse.add(i) = v }
    }

    /// Writes slot `i`'s accumulated first tangent impulse.
    #[inline]
    fn set_tangent1_impulse(&self, i: usize, v: f32) {
        // SAFETY: as `set_normal_impulse` — `i < len`, single-owner row, write-capable
        //   `solve_base`-derived `*mut f32` (not a Frozen `&[f32]` reborrow).
        unsafe { *self.tangent1_impulse.add(i) = v }
    }

    /// Writes slot `i`'s accumulated second tangent impulse.
    #[inline]
    fn set_tangent2_impulse(&self, i: usize, v: f32) {
        // SAFETY: as `set_normal_impulse` — `i < len`, single-owner row, write-capable
        //   `solve_base`-derived `*mut f32` (not a Frozen `&[f32]` reborrow).
        unsafe { *self.tangent2_impulse.add(i) = v }
    }
}

/// Single-thread build / refill view over all 31 contact [`ScratchColumn`]s
/// (audit Stage P — P2).
///
/// `!Send` (`PhantomData<*mut ()>`): the build/push phase runs on ONE thread, so
/// holding `&mut` build views over whole columns is sound — no other thread is
/// live during build. This is the ONLY surface that pushes / fills the columns;
/// the worker-facing [`ContactSolveView`] deliberately lacks every whole-buffer
/// path (the structural SP4 fix).
struct ContactBuildView<'a> {
    ra_x: ScratchBuildView<'a, f32>,
    ra_y: ScratchBuildView<'a, f32>,
    ra_z: ScratchBuildView<'a, f32>,
    rb_x: ScratchBuildView<'a, f32>,
    rb_y: ScratchBuildView<'a, f32>,
    rb_z: ScratchBuildView<'a, f32>,
    normal_x: ScratchBuildView<'a, f32>,
    normal_y: ScratchBuildView<'a, f32>,
    normal_z: ScratchBuildView<'a, f32>,
    tangent1_x: ScratchBuildView<'a, f32>,
    tangent1_y: ScratchBuildView<'a, f32>,
    tangent1_z: ScratchBuildView<'a, f32>,
    tangent2_x: ScratchBuildView<'a, f32>,
    tangent2_y: ScratchBuildView<'a, f32>,
    tangent2_z: ScratchBuildView<'a, f32>,
    separation: ScratchBuildView<'a, f32>,
    friction: ScratchBuildView<'a, f32>,
    restitution: ScratchBuildView<'a, f32>,
    normal_impulse: ScratchBuildView<'a, f32>,
    tangent1_impulse: ScratchBuildView<'a, f32>,
    tangent2_impulse: ScratchBuildView<'a, f32>,
    body_a: ScratchBuildView<'a, u32>,
    body_b: ScratchBuildView<'a, u32>,
    b_is_sentinel: ScratchBuildView<'a, bool>,
    warm_key: ScratchBuildView<'a, u64>,
    vn_initial: ScratchBuildView<'a, f32>,
    _not_send: PhantomData<*mut ()>,
}

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
/// All columns are `clear()`-ed and refilled each build — capacity is reused, no
/// per-step alloc in steady state (W2). The columns are kernel-native
/// [`ScratchColumn`]s (audit Stage P — P2) instead of `std::Vec` side-stores: a
/// `ScratchColumn`'s data base is ADDRESS-STABLE across an in-place grow, which is
/// the property a `std::Vec` lacks (a realloc moves the base — the SP4/rigid race
/// root cause). The worker-facing [`ContactSolveView`] hands out per-element
/// `base + index` accessors ONLY, so the parallel solve never reborrows the whole
/// buffer.
struct ContactColumns {
    /// Body-A anchor offset (world frame), split into three SoA columns.
    ra_x: ScratchColumn<f32>,
    ra_y: ScratchColumn<f32>,
    ra_z: ScratchColumn<f32>,
    /// Body-B anchor offset (world frame), split into three SoA columns.
    rb_x: ScratchColumn<f32>,
    rb_y: ScratchColumn<f32>,
    rb_z: ScratchColumn<f32>,
    /// Contact normal (A → B), split into three SoA columns.
    normal_x: ScratchColumn<f32>,
    normal_y: ScratchColumn<f32>,
    normal_z: ScratchColumn<f32>,
    /// First friction tangent, split into three SoA columns.
    tangent1_x: ScratchColumn<f32>,
    tangent1_y: ScratchColumn<f32>,
    tangent1_z: ScratchColumn<f32>,
    /// Second friction tangent, split into three SoA columns.
    tangent2_x: ScratchColumn<f32>,
    tangent2_y: ScratchColumn<f32>,
    tangent2_z: ScratchColumn<f32>,
    /// Signed separation at gather time (negative = penetrating).
    separation: ScratchColumn<f32>,
    /// Combined friction coefficient (`max(µa, µb)`, the reference rule).
    friction: ScratchColumn<f32>,
    /// Combined restitution coefficient (`max(ea, eb)`, the reference rule).
    restitution: ScratchColumn<f32>,
    /// Accumulated normal impulse `λn ≥ 0` (warm-seeded).
    normal_impulse: ScratchColumn<f32>,
    /// Accumulated tangent impulse along `t1` (warm-seeded).
    tangent1_impulse: ScratchColumn<f32>,
    /// Accumulated tangent impulse along `t2` (warm-seeded).
    tangent2_impulse: ScratchColumn<f32>,
    /// Dense body-A row index.
    body_a: ScratchColumn<u32>,
    /// Dense body-B row index (an A-row placeholder when `b_is_sentinel`).
    body_b: ScratchColumn<u32>,
    /// `true` for an SDF contact (`body_b == SDF_SENTINEL`): body B is
    /// [`IMMOVABLE_AT_REST`], never `bodies[body_b]`.
    b_is_sentinel: ScratchColumn<bool>,
    /// Per-point warm-start key (`pack`/`pack_sdf` with this point's feature id).
    warm_key: ScratchColumn<u64>,
    /// Gather-time relative normal APPROACH velocity (B−A on the normal),
    /// captured before the first substep for the restitution pass.
    vn_initial: ScratchColumn<f32>,
    /// CSR color offsets: `color_offsets[c] .. color_offsets[c + 1]` is color
    /// `c`'s contiguous slot span (`len == n_colors + 1`).
    color_offsets: ScratchColumn<u32>,
    /// Slot indices in canonical `(manifold, point)` order (IM-2b warm store).
    canonical: ScratchColumn<u32>,
    /// CSR manifold-group boundaries in solve (slot) order (C1): group `g`'s slot
    /// run is `group_start[g] .. group_start[g + 1]` (`len == n_groups + 1`). One
    /// group per appended manifold with ≥1 live point.
    group_start: ScratchColumn<u32>,
    /// Per-color CSR into `group_start` (C1): color `c`'s manifold-groups are
    /// `color_group_start[c] .. color_group_start[c + 1]` (`len == n_colors + 1`),
    /// each value indexing `group_start`. Lets O6/O7 enumerate, per color, the
    /// groups and (via `group_start`) each group's slot range.
    color_group_start: ScratchColumn<u32>,
    /// Reused scratch: per-manifold-index base slot + live-point count, written as
    /// each manifold is appended in the build walk so the canonical order is
    /// recovered WITHOUT a second replay walk (W1/O1/O3 fold). `(u32::MAX, 0)` for
    /// a manifold absent from every color or with no live point. Capacity-reused
    /// (`clear()` + per-build `manifold_fill` refill), never `vec!` per step.
    manifold_base: ScratchColumn<(u32, u32)>,
}

impl ContactColumns {
    /// Builds the 31 contact columns, each on its own band id, reserving
    /// `contacts` rows (clamped up to the kernel's adaptive per-element budget so
    /// a freshly-built solver never regrows in steady state).
    ///
    /// The contact-column layouts are registered idempotently before any
    /// `ScratchColumn::new` reads them (see [`register_scratch_layouts`]).
    fn with_capacity(contacts: usize) -> Self {
        // Self-register the contact-column band BEFORE any `ScratchColumn::new` reads a
        // synthetic id's layout — idempotent (write-once `OnceLock`), so calling it here
        // AND in the solver constructor costs one branch each after the first. Makes this
        // constructor safe for ANY caller (tests build `ContactColumns` directly).
        register_scratch_layouts();
        // Reserve at least the contact count, but never below the kernel's adaptive
        // per-element budget (a pure-address-space reservation, demand-committed —
        // a generous ceiling costs nothing and removes the per-step grow-cap hazard).
        let f32_rows = contacts.max(scratch_reserve_rows(size_of::<f32>()));
        let u32_rows = contacts.max(scratch_reserve_rows(size_of::<u32>()));
        let bool_rows = contacts.max(scratch_reserve_rows(size_of::<bool>()));
        let u64_rows = contacts.max(scratch_reserve_rows(size_of::<u64>()));
        let pair_rows = contacts.max(scratch_reserve_rows(size_of::<(u32, u32)>()));

        // `k` walks the band in struct field order (see `register_contact_column_layouts`).
        let mut k = 0usize;
        let mut f32_col = || {
            let c = ScratchColumn::<f32>::new(contact_column_id(k), f32_rows);
            k += 1;
            c
        };
        Self {
            ra_x: f32_col(),
            ra_y: f32_col(),
            ra_z: f32_col(),
            rb_x: f32_col(),
            rb_y: f32_col(),
            rb_z: f32_col(),
            normal_x: f32_col(),
            normal_y: f32_col(),
            normal_z: f32_col(),
            tangent1_x: f32_col(),
            tangent1_y: f32_col(),
            tangent1_z: f32_col(),
            tangent2_x: f32_col(),
            tangent2_y: f32_col(),
            tangent2_z: f32_col(),
            separation: f32_col(),
            friction: f32_col(),
            restitution: f32_col(),
            normal_impulse: f32_col(),
            tangent1_impulse: f32_col(),
            tangent2_impulse: f32_col(),
            body_a: ScratchColumn::<u32>::new(contact_column_id(21), u32_rows),
            body_b: ScratchColumn::<u32>::new(contact_column_id(22), u32_rows),
            b_is_sentinel: ScratchColumn::<bool>::new(contact_column_id(23), bool_rows),
            warm_key: ScratchColumn::<u64>::new(contact_column_id(24), u64_rows),
            vn_initial: ScratchColumn::<f32>::new(contact_column_id(25), f32_rows),
            color_offsets: ScratchColumn::<u32>::new(contact_column_id(26), u32_rows),
            canonical: ScratchColumn::<u32>::new(contact_column_id(27), u32_rows),
            group_start: ScratchColumn::<u32>::new(contact_column_id(28), u32_rows),
            color_group_start: ScratchColumn::<u32>::new(contact_column_id(29), u32_rows),
            manifold_base: ScratchColumn::<(u32, u32)>::new(contact_column_id(30), pair_rows),
        }
    }

    /// Number of contact-point slots currently built.
    #[inline]
    fn len(&self) -> usize {
        self.separation.len()
    }

    /// Borrows the 25 push-filled columns as single-thread build views (the
    /// `manifold_base` / CSR columns are filled via their own helpers, not the
    /// per-row `push_row`). The borrows are DISJOINT-field, so the borrow checker
    /// permits 25 simultaneous `&mut` into one `&mut ContactColumns`.
    fn build_view(&mut self) -> ContactBuildView<'_> {
        ContactBuildView {
            ra_x: self.ra_x.build_view(),
            ra_y: self.ra_y.build_view(),
            ra_z: self.ra_z.build_view(),
            rb_x: self.rb_x.build_view(),
            rb_y: self.rb_y.build_view(),
            rb_z: self.rb_z.build_view(),
            normal_x: self.normal_x.build_view(),
            normal_y: self.normal_y.build_view(),
            normal_z: self.normal_z.build_view(),
            tangent1_x: self.tangent1_x.build_view(),
            tangent1_y: self.tangent1_y.build_view(),
            tangent1_z: self.tangent1_z.build_view(),
            tangent2_x: self.tangent2_x.build_view(),
            tangent2_y: self.tangent2_y.build_view(),
            tangent2_z: self.tangent2_z.build_view(),
            separation: self.separation.build_view(),
            friction: self.friction.build_view(),
            restitution: self.restitution.build_view(),
            normal_impulse: self.normal_impulse.build_view(),
            tangent1_impulse: self.tangent1_impulse.build_view(),
            tangent2_impulse: self.tangent2_impulse.build_view(),
            body_a: self.body_a.build_view(),
            body_b: self.body_b.build_view(),
            b_is_sentinel: self.b_is_sentinel.build_view(),
            warm_key: self.warm_key.build_view(),
            vn_initial: self.vn_initial.build_view(),
            _not_send: PhantomData,
        }
    }

    /// Freezes the built columns into the worker-facing [`ContactSolveView`] (B4
    /// re-create-before-view-live: called AFTER the last build-time grow, so the
    /// captured raw bases are stable for every worker's lifetime).
    ///
    /// Every base is derived from the kernel's write-capable raw base accessor
    /// [`ScratchColumn::solve_base`] — a provenance-preserving `*mut T` taken from
    /// `ComponentPool::buffer_ptr` with NO `&[T]` interposed. The three impulse
    /// columns (worker-mutable) therefore carry WRITE provenance: the previous
    /// `as_read_slice().as_ptr().cast_mut()` branded them Frozen / SharedReadOnly,
    /// which is Tree-Borrows-UB to write through (`set_*_impulse`). The read-only
    /// bases are stored as `*const _` reborrows of the same write-capable raw base
    /// — reading through a `*const` derived from a write-capable base is sound, and
    /// deriving every base from one accessor sidesteps any future Frozen-tag trap.
    /// Concurrent `*mut` writes are non-aliasing by the C2 coloring invariant (two
    /// workers never own the same slot).
    fn solve_view(&self) -> ContactSolveView<'_> {
        ContactSolveView {
            ra_x: self.ra_x.solve_base().cast_const(),
            ra_y: self.ra_y.solve_base().cast_const(),
            ra_z: self.ra_z.solve_base().cast_const(),
            rb_x: self.rb_x.solve_base().cast_const(),
            rb_y: self.rb_y.solve_base().cast_const(),
            rb_z: self.rb_z.solve_base().cast_const(),
            normal_x: self.normal_x.solve_base().cast_const(),
            normal_y: self.normal_y.solve_base().cast_const(),
            normal_z: self.normal_z.solve_base().cast_const(),
            tangent1_x: self.tangent1_x.solve_base().cast_const(),
            tangent1_y: self.tangent1_y.solve_base().cast_const(),
            tangent1_z: self.tangent1_z.solve_base().cast_const(),
            tangent2_x: self.tangent2_x.solve_base().cast_const(),
            tangent2_y: self.tangent2_y.solve_base().cast_const(),
            tangent2_z: self.tangent2_z.solve_base().cast_const(),
            separation: self.separation.solve_base().cast_const(),
            friction: self.friction.solve_base().cast_const(),
            body_a: self.body_a.solve_base().cast_const(),
            body_b: self.body_b.solve_base().cast_const(),
            b_is_sentinel: self.b_is_sentinel.solve_base().cast_const(),
            // Worker-mutable: write-capable provenance (NOT Frozen).
            normal_impulse: self.normal_impulse.solve_base(),
            tangent1_impulse: self.tangent1_impulse.solve_base(),
            tangent2_impulse: self.tangent2_impulse.solve_base(),
            group_start: self.group_start.solve_base().cast_const(),
            len: self.len(),
            _marker: PhantomData,
        }
    }

    /// Reads slot `i`'s body-A anchor as a [`Vec3`] (single-thread passes only —
    /// `apply_restitution` / `warm_start_apply`).
    #[inline]
    fn ra(&self, i: usize) -> Vec3 {
        let s = self.ra_x.as_read_slice();
        Vec3::new(s[i], self.ra_y.as_read_slice()[i], self.ra_z.as_read_slice()[i])
    }

    /// Reads slot `i`'s body-B anchor as a [`Vec3`] (single-thread passes only).
    #[inline]
    fn rb(&self, i: usize) -> Vec3 {
        Vec3::new(
            self.rb_x.as_read_slice()[i],
            self.rb_y.as_read_slice()[i],
            self.rb_z.as_read_slice()[i],
        )
    }

    /// Reads slot `i`'s contact normal as a [`Vec3`] (single-thread passes only).
    #[inline]
    fn normal(&self, i: usize) -> Vec3 {
        Vec3::new(
            self.normal_x.as_read_slice()[i],
            self.normal_y.as_read_slice()[i],
            self.normal_z.as_read_slice()[i],
        )
    }

    /// Reads slot `i`'s first friction tangent as a [`Vec3`] — used ONLY by the
    /// +avx2 `cone_probe` differential oracle, so it is gated to that exact build.
    #[cfg(all(test, target_arch = "x86_64", target_feature = "avx2"))]
    #[inline]
    fn tangent1(&self, i: usize) -> Vec3 {
        Vec3::new(
            self.tangent1_x.as_read_slice()[i],
            self.tangent1_y.as_read_slice()[i],
            self.tangent1_z.as_read_slice()[i],
        )
    }

    /// Reads slot `i`'s second friction tangent as a [`Vec3`] — used ONLY by the
    /// +avx2 `cone_probe` differential oracle, so it is gated to that exact build.
    #[cfg(all(test, target_arch = "x86_64", target_feature = "avx2"))]
    #[inline]
    fn tangent2(&self, i: usize) -> Vec3 {
        Vec3::new(
            self.tangent2_x.as_read_slice()[i],
            self.tangent2_y.as_read_slice()[i],
            self.tangent2_z.as_read_slice()[i],
        )
    }

    /// Reads slot `i`'s combined restitution coefficient (single-thread only —
    /// `apply_restitution`).
    #[inline]
    fn restitution(&self, i: usize) -> f32 {
        self.restitution.as_read_slice()[i]
    }

    /// Reads slot `i`'s gather-time approach velocity (single-thread only —
    /// `apply_restitution`).
    #[inline]
    fn vn_initial(&self, i: usize) -> f32 {
        self.vn_initial.as_read_slice()[i]
    }

    /// Reads slot `i`'s body-A row index (single-thread passes only).
    #[inline]
    fn body_a(&self, i: usize) -> u32 {
        self.body_a.as_read_slice()[i]
    }

    /// Reads slot `i`'s body-B row index (single-thread passes only).
    #[inline]
    fn body_b(&self, i: usize) -> u32 {
        self.body_b.as_read_slice()[i]
    }

    /// Reads slot `i`'s sentinel flag (single-thread passes only).
    #[inline]
    fn b_is_sentinel(&self, i: usize) -> bool {
        self.b_is_sentinel.as_read_slice()[i]
    }

    /// Reads slot `i`'s accumulated normal impulse (single-thread passes only).
    #[inline]
    fn normal_impulse(&self, i: usize) -> f32 {
        self.normal_impulse.as_read_slice()[i]
    }

    /// Reads slot `i`'s first tangent impulse (single-thread passes only).
    #[inline]
    fn tangent1_impulse(&self, i: usize) -> f32 {
        self.tangent1_impulse.as_read_slice()[i]
    }

    /// Reads slot `i`'s second tangent impulse (single-thread passes only).
    #[inline]
    fn tangent2_impulse(&self, i: usize) -> f32 {
        self.tangent2_impulse.as_read_slice()[i]
    }

    /// Reads slot `i`'s warm-start key (single-thread passes only — `store_and_swap`).
    #[inline]
    fn warm_key(&self, i: usize) -> u64 {
        self.warm_key.as_read_slice()[i]
    }

    /// Writes slot `i`'s normal impulse (single-thread passes only —
    /// `apply_restitution` runs after the parallel solve has joined).
    #[inline]
    fn set_normal_impulse(&mut self, i: usize, v: f32) {
        self.normal_impulse.build_view().as_mut_slice()[i] = v;
    }

    /// The CSR color-offsets column as a read slice.
    #[inline]
    fn color_offsets(&self) -> &[u32] {
        self.color_offsets.as_read_slice()
    }

    /// The per-group CSR (`group_start`) as a read slice.
    #[inline]
    fn group_start(&self) -> &[u32] {
        self.group_start.as_read_slice()
    }

    /// The per-color CSR into `group_start` as a read slice.
    #[inline]
    fn color_group_start(&self) -> &[u32] {
        self.color_group_start.as_read_slice()
    }

    /// The canonical-order slot list as a read slice.
    #[inline]
    fn canonical(&self) -> &[u32] {
        self.canonical.as_read_slice()
    }

    /// Resets the push-filled point columns AND the CSR columns for a fresh build,
    /// seeding the three CSRs with their leading `0`.
    fn begin_build(&mut self) {
        self.build_view().clear();
        self.color_offsets.build_view().clear();
        self.group_start.build_view().clear();
        self.color_group_start.build_view().clear();
        self.canonical.build_view().clear();
        self.color_offsets.build_view().push(0);
        self.group_start.build_view().push(0);
        self.color_group_start.build_view().push(0);
    }

    /// Appends a `color_offsets` CSR boundary.
    #[inline]
    fn push_color_offset(&mut self, v: u32) {
        self.color_offsets.build_view().push(v);
    }

    /// Appends a `group_start` CSR boundary.
    #[inline]
    fn push_group_start(&mut self, v: u32) {
        self.group_start.build_view().push(v);
    }

    /// Appends a `color_group_start` CSR boundary.
    #[inline]
    fn push_color_group_start(&mut self, v: u32) {
        self.color_group_start.build_view().push(v);
    }

    /// Appends a canonical-order slot index.
    #[inline]
    fn push_canonical(&mut self, v: u32) {
        self.canonical.build_view().push(v);
    }

    /// Reproduces the prior `manifold_base.resize(n, (u32::MAX, 0))` then runs the
    /// sparse manifold-start write (audit Stage P — P2, critic note O1).
    ///
    /// `manifold_fill` is a TRANSIENT refill stager reproducing the resize-sentinel
    /// pattern on the `manifold_base` [`ScratchColumn`] — NOT durable per-entity
    /// data. The column is `clear`-ed then refilled to exactly `n` rows of the
    /// sentinel `(u32::MAX, 0)`; the committed pages are capacity-reused (no free,
    /// no per-frame alloc in steady state). The caller (`build_columns`) then
    /// sparse-overwrites the manifold-start rows with `(base, count)`.
    fn manifold_fill(&mut self, n: usize) {
        let mut view = self.manifold_base.build_view();
        view.clear();
        // Refill the first `n` rows with the resize sentinel (the same value
        // `Vec::resize(n, (u32::MAX, 0))` produced) — a straight broadcast of the
        // sentinel pair, in-place on the address-stable column (grows the committed
        // frontier the first time, capacity-reused thereafter — zero steady-state
        // alloc).
        for _ in 0..n {
            view.push((u32::MAX, 0));
        }
    }

    /// Sparse-overwrites `manifold_base[mi]` with `(base, count)` (the manifold-start
    /// write run after [`manifold_fill`](Self::manifold_fill)).
    #[inline]
    fn set_manifold_base(&mut self, mi: usize, base: u32, count: u32) {
        self.manifold_base.build_view().as_mut_slice()[mi] = (base, count);
    }

    /// The `manifold_base` column as a read slice (the canonical-order emit reads it).
    #[inline]
    fn manifold_base(&self) -> &[(u32, u32)] {
        self.manifold_base.as_read_slice()
    }
}

impl ContactBuildView<'_> {
    /// Clears every push-filled column for a fresh build (capacity reused — the
    /// committed pages stay resident; `clear` is `len = 0` with no free).
    fn clear(&mut self) {
        self.ra_x.clear();
        self.ra_y.clear();
        self.ra_z.clear();
        self.rb_x.clear();
        self.rb_y.clear();
        self.rb_z.clear();
        self.normal_x.clear();
        self.normal_y.clear();
        self.normal_z.clear();
        self.tangent1_x.clear();
        self.tangent1_y.clear();
        self.tangent1_z.clear();
        self.tangent2_x.clear();
        self.tangent2_y.clear();
        self.tangent2_z.clear();
        self.separation.clear();
        self.friction.clear();
        self.restitution.clear();
        self.normal_impulse.clear();
        self.tangent1_impulse.clear();
        self.tangent2_impulse.clear();
        self.body_a.clear();
        self.body_b.clear();
        self.b_is_sentinel.clear();
        self.warm_key.clear();
        self.vn_initial.clear();
    }

    /// Appends one contact-point slot in lockstep across the 26 push-filled
    /// columns. Order is identical to the prior `Vec::push` order, so the byte
    /// layout is bit-for-bit identical.
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
    /// Backed by a [`ScratchColumn`] (engine-owned, ADDRESS-STABLE base) instead
    /// of a `std::Vec` parallel-data-system (audit Stage P, the race-fix column):
    /// the colored workers reach each body row through a `ScratchSolveView`'s
    /// per-element `row_ptr` (no whole-buffer reborrow), and the base never
    /// realloc-moves across a refill-grow — the property `std::Vec` lacked that
    /// caused the SP4 colored-solve data race.
    bodies: ScratchColumn<BodyEffective>,
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
        register_scratch_layouts();
        let reserve = bodies.max(scratch_reserve_rows(size_of::<BodyEffective>()));
        Self {
            bodies: ScratchColumn::new(body_eff_colored_id(), reserve),
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
        let mut view = self.bodies.build_view();
        view.clear();
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
            view.push(BodyEffective {
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
        // Reset the push-filled point columns + the three CSR columns, seeding each
        // CSR's leading `0` (B4: all grow / refill happens here, single-threaded,
        // BEFORE any `solve_view()` captures a base — so no worker can see a moving
        // base).
        cols.begin_build();

        // Reused per-manifold base map (no `vec!` per step): base slot + live count
        // recorded as each manifold is first appended, consumed below to emit the
        // canonical order in manifold index ascending order (D4). `manifold_fill`
        // reproduces `resize(n, (u32::MAX, 0))` on the address-stable column (O1).
        cols.manifold_fill(manifolds.len());

        // The BodyEffective rows read by the per-point build are a SINGLE-THREADED
        // read here (the build runs before any parallel dispatch); take one read
        // slice of the solver's body column. Disjoint from `cols` (distinct field).
        let bodies_eff = bodies_eff.as_read_slice();

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
                    cols.set_manifold_base(mi as usize, base, count);
                    cols.push_group_start(base + count);
                }
            }
            cols.push_color_offset(cols.len() as u32);
            // Color `c`'s manifold-groups end at the current `group_start` length
            // (the per-color CSR indexes into `group_start`).
            cols.push_color_group_start((cols.group_start().len() - 1) as u32);
        }

        // Canonical `(manifold, point)` order for the IM-2b warm store — emitted
        // from the base map recorded above, in ascending manifold index, WITHOUT a
        // second color→manifold→point replay walk (W1/O1/O3 fold). The base map is
        // read into a fixed-capacity stack-free pass; pull each `(base, count)` by
        // value so no `&[_]` borrow into `cols` is held across the `push_canonical`.
        let n_manifolds = cols.manifold_base().len();
        for mi in 0..n_manifolds {
            let (base, count) = cols.manifold_base()[mi];
            if base == u32::MAX {
                continue;
            }
            for p in 0..count {
                cols.push_canonical(base + p);
            }
        }
        debug_assert_eq!(
            cols.canonical().len(),
            cols.len(),
            "invariant: canonical order must cover every built contact-point slot exactly once"
        );
        debug_assert_eq!(
            cols.group_start().len() as u32,
            *cols.color_group_start().last().unwrap_or(&0) + 1,
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

        // One single-thread build view over the 26 push-filled columns for the
        // whole manifold's points (the CSR / `manifold_base` columns are filled by
        // the caller, not here).
        let mut view = cols.build_view();
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

            view.push_point(
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
    fn warm_start_apply(view: ContactSolveView<'_>, bodies_eff: ScratchSolveView<'_, BodyEffective>) {
        for i in 0..view.len() {
            let normal = view.normal(i);
            let t1 = view.tangent1(i);
            let t2 = view.tangent2(i);
            let impulse = normal * view.normal_impulse(i)
                + t1 * view.tangent1_impulse(i)
                + t2 * view.tangent2_impulse(i);
            let ia = view.body_a(i) as usize;
            body_mut(bodies_eff, ia).apply_impulse(view.ra(i), impulse * -1.0);
            if !view.b_is_sentinel(i) {
                body_mut(bodies_eff, view.body_b(i) as usize).apply_impulse(view.rb(i), impulse);
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
        view: ContactSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
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
                        view,
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
            view,
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
        view: ContactSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        span: (usize, usize),
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) {
        let (start, end) = span;
        for i in start..end {
            let ra = view.ra(i);
            let rb = view.rb(i);
            let normal = view.normal(i);
            let t1 = view.tangent1(i);
            let t2 = view.tangent2(i);
            let ia = view.body_a(i) as usize;
            let b_is_sentinel = view.b_is_sentinel(i);
            let ib = view.body_b(i) as usize;
            let friction = view.friction(i);
            let separation = view.separation(i);

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
            let lambda_n = view.normal_impulse(i);
            let d_lambda = if bias_active {
                -mass_coeff * m_eff * (vn + bias) - impulse_coeff * lambda_n
            } else {
                -m_eff * vn
            };
            let new_lambda = (lambda_n + d_lambda).max(0.0);
            let applied_n = new_lambda - lambda_n;
            view.set_normal_impulse(i, new_lambda);
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
            // `new_lambda` IS the value just stored at slot `i`'s normal impulse, so
            // `friction * new_lambda` is bit-identical to re-reading the column and
            // removes a redundant load.
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
            let lambda_t1 = view.tangent1_impulse(i);
            let lambda_t2 = view.tangent2_impulse(i);
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
            view.set_tangent1_impulse(i, new_t1);
            view.set_tangent2_impulse(i, new_t2);
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
        view: ContactSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
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
        // C1: the worker's OWN slot span `[chunk_start, chunk_end)` — every gathered
        // slot must lie inside it (no foreign / cross-worker read). It is the union of
        // the `[g_lo, g_hi)` groups' contiguous slot runs.
        let (chunk_start, chunk_end) = span;
        // SAFETY: `g_lo`/`g_hi` are within the color's group range `[g_lo, g_hi] <=
        //   n_groups` (the caller's contract), each a valid `group_start` index.
        debug_assert!(
            chunk_start == unsafe { view.group_start_at(g_lo) } as usize
                && chunk_end == unsafe { view.group_start_at(g_hi) } as usize
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
                    // SAFETY: `g`/`g + 1` index the live `group_start` CSR — `g` is in
                    //   `[cohort_lo, cohort_hi) ⊆ [g_lo, g_hi)` and `g + 1 <= g_hi <=
                    //   n_groups`, both valid CSR indices (the caller's contract).
                    let base = unsafe { view.group_start_at(g) } as usize;
                    let width = unsafe { view.group_start_at(g + 1) } as usize - base;
                    debug_assert!(width >= 1, "every built group has >= 1 point");
                    let s = base; // first slot of the group (the body pair is shared)
                    let lane_ia = view.body_a(s) as usize;
                    let b_sent = view.b_is_sentinel(s);
                    let lane_ib = view.body_b(s) as usize;
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
                    ra_s[0][lane] = view.ra_x(s);
                    ra_s[1][lane] = view.ra_y(s);
                    ra_s[2][lane] = view.ra_z(s);
                    rb_s[0][lane] = view.rb_x(s);
                    rb_s[1][lane] = view.rb_y(s);
                    rb_s[2][lane] = view.rb_z(s);
                    n_s[0][lane] = view.normal_x(s);
                    n_s[1][lane] = view.normal_y(s);
                    n_s[2][lane] = view.normal_z(s);
                    t1_s[0][lane] = view.tangent1_x(s);
                    t1_s[1][lane] = view.tangent1_y(s);
                    t1_s[2][lane] = view.tangent1_z(s);
                    t2_s[0][lane] = view.tangent2_x(s);
                    t2_s[1][lane] = view.tangent2_y(s);
                    t2_s[2][lane] = view.tangent2_z(s);
                    sep_s[lane] = view.separation(s);
                    fric_s[lane] = view.friction(s);
                    ni_s[lane] = view.normal_impulse(s);
                    ti1_s[lane] = view.tangent1_impulse(s);
                    ti2_s[lane] = view.tangent2_impulse(s);
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
                        view.set_normal_impulse(s, ni_s[lane]);
                        view.set_tangent1_impulse(s, ti1_s[lane]);
                        view.set_tangent2_impulse(s, ti2_s[lane]);
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
        cols: &ContactColumns,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        parallel: bool,
        simd: bool,
    ) {
        // B4 re-create-before-view-live: the columns are FROZEN by now (the last
        // build-time grow happened in `build_columns`, before the substep loop), so
        // the worker-facing `ContactSolveView` captures stable raw bases that cannot
        // dangle while a view is live. The view is `Copy` — each worker gets a copy.
        let view = cols.solve_view();
        let color_offsets = cols.color_offsets();
        let color_group_start = cols.color_group_start();
        let n_colors = color_offsets.len().saturating_sub(1);
        for c in 0..n_colors {
            if parallel {
                Self::solve_color_parallel(
                    cols,
                    view,
                    bodies_eff,
                    c,
                    bias_rate,
                    mass_coeff,
                    impulse_coeff,
                    bias_active,
                    simd,
                );
            } else {
                let start = color_offsets[c] as usize;
                let end = color_offsets[c + 1] as usize;
                // O7 dispatch fork (the 0%-gate): `simd == false` runs the byte-
                // identical scalar oracle `solve_color`; `simd == true` runs the
                // AVX2 cohort kernel over the color's manifold-GROUP range (the
                // bit-exact width-only path). The non-parallel SIMD path solves the
                // WHOLE color's groups as cohorts on the calling thread.
                let g_lo = color_group_start[c] as usize;
                let g_hi = color_group_start[c + 1] as usize;
                Self::solve_color_dispatch(
                    view,
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
        cols: &ContactColumns,
        view: ContactSolveView<'_>,
        bodies_eff: ScratchSolveView<'_, BodyEffective>,
        color: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
        simd: bool,
    ) {
        // The color's manifold-group range (indices into `group_start`).
        let color_group_start = cols.color_group_start();
        let color_offsets = cols.color_offsets();
        let g_lo = color_group_start[color] as usize;
        let g_hi = color_group_start[color + 1] as usize;
        let n_groups = g_hi - g_lo;
        if n_groups == 0 {
            return;
        }

        let span = (color_offsets[color] as usize, color_offsets[color + 1] as usize);

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
                view,
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

            // Send + Sync wrapper: the contact-column SOLVE VIEW (Copy, per-element
            // base+index only) + the bodies SOLVE VIEW (Copy, row-ptr-only). Each
            // worker writes only its chunk's DISJOINT impulse slots and DISJOINT body
            // rows, reaching every contact column element via `view.<accessor>(i)` and
            // every body row via `bodies.row_ptr(i)` — NO whole-buffer reborrow on any
            // path (the P2 + P1 structural fix; the prior `cols: *mut ContactColumns`
            // + `columns()` `&mut *self.cols` whole-struct reborrow that caused the
            // rigid Tree-Borrows race is DELETED). The `group_start` CSR base now rides
            // inside `view`, read via `view.group_start_at` — no `&[u32]` borrow into
            // `cols` is ever held across the scope (TB-clean, Phase 9.3c discipline).
            let ptrs = ColorSolvePtrs {
                bodies: bodies_eff,
                view,
            };

            pool.scope(|scope| {
                let mut chunk_g_lo = g_lo;
                while chunk_g_lo < g_hi {
                    // The chunk's first group's first slot. Read via the view's raw
                    // `group_start` base (no `&` borrow into `cols`).
                    // SAFETY: `chunk_g_lo` is within `[g_lo, g_hi)`, a valid index
                    //   into the live `group_start` column.
                    let chunk_start = unsafe { ptrs.view.group_start_at(chunk_g_lo) } as usize;

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
                        let so_far =
                            unsafe { ptrs.view.group_start_at(chunk_g_hi) } as usize - chunk_start;
                        if so_far >= target {
                            break;
                        }
                        chunk_g_hi = (chunk_g_hi + step).min(g_hi);
                    }

                    // The chunk's contiguous slot span ends at its last group's last
                    // slot.
                    // SAFETY: `chunk_g_hi` is within `(g_lo, g_hi]`, a valid index
                    //   into the live `group_start` column.
                    let chunk_end = unsafe { ptrs.view.group_start_at(chunk_g_hi) } as usize;
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
                        // DISJOINTNESS (the O6 + P2 soundness argument — why the
                        // concurrent per-element accesses are race- and TB-clean):
                        //   - `ptrs` carries only `Copy` solve views (`ContactSolveView`
                        //     + body `ScratchSolveView`) whose raw bases name columns
                        //     borrowed for the whole `solve_color_parallel` frame;
                        //     `pool.scope`'s Drop blocks (work-stealing join) until every
                        //     spawned task completes, so every base outlives every task —
                        //     no use-after-free, no escape past the borrow, and (B4) no
                        //     regrow moves a base while a view is live.
                        //   - This chunk solves ONLY slots `[chunk_start, chunk_end)` and
                        //     writes ONLY those slots' impulse columns via
                        //     `view.set_*_impulse*(s, _)` (a single-row `base + s` write).
                        //     Distinct chunks have non-overlapping slot ranges (they
                        //     partition the color's groups), so no two workers write the
                        //     same column element and no `&mut`/store ever spans more than
                        //     one row.
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
                        //   - The contact columns + bodies a chunk READS are its own
                        //     disjoint slots/rows plus shared read-only static rows + the
                        //     read-only `group_start` CSR, so no chunk reads an element
                        //     another chunk is writing.
                        //   The DELETED `cols: *mut ContactColumns` + `columns()`
                        //   (`&mut *self.cols`) whole-struct reborrow — the rigid TB race
                        //   surface — is gone and un-typeable from `ptrs`. O7: when
                        //   `simd`, the worker runs `solve_color_avx2` over its cohort
                        //   range `[task_g_lo, task_g_hi)` — a cohort packs 8 disjoint
                        //   groups, so distinct workers' cohort-runs still touch DISJOINT
                        //   dynamic rows + DISJOINT impulse slots, statics/sentinels
                        //   never-written; the disjointness argument is UNCHANGED from
                        //   the scalar chunk dispatch above.
                        Self::solve_color_dispatch(
                            ptrs.view,
                            ptrs.bodies,
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
                view,
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
    fn apply_restitution(cols: &mut ContactColumns, bodies_eff: ScratchSolveView<'_, BodyEffective>) {
        // Single-threaded (run after the parallel solve has joined), so direct
        // `&mut ContactColumns` per-element access is sound — `restitution` /
        // `vn_initial` are ST-only columns absent from the worker-facing view.
        for i in 0..cols.len() {
            if cols.restitution(i) <= 0.0 {
                continue;
            }
            let vn0 = cols.vn_initial(i);
            if vn0 > -RESTITUTION_THRESHOLD {
                continue;
            }
            let ra = cols.ra(i);
            let rb = cols.rb(i);
            let normal = cols.normal(i);
            let ia = cols.body_a(i) as usize;
            let b_is_sentinel = cols.b_is_sentinel(i);
            let ib = cols.body_b(i) as usize;
            let bb_view = || -> BodyEffective {
                if b_is_sentinel { IMMOVABLE_AT_REST } else { body_copy(bodies_eff, ib) }
            };
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
            let v_target = -cols.restitution(i) * vn0;
            let d_lambda = m_eff * (v_target - vn);
            let lambda_n = cols.normal_impulse(i);
            let new_lambda = (lambda_n + d_lambda).max(0.0);
            let applied = new_lambda - lambda_n;
            cols.set_normal_impulse(i, new_lambda);
            let impulse = normal * applied;
            body_mut(bodies_eff, ia).apply_impulse(ra, impulse * -1.0);
            if !b_is_sentinel {
                body_mut(bodies_eff, ib).apply_impulse(rb, impulse);
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
        for k in 0..cols.canonical().len() {
            let i = cols.canonical()[k] as usize;
            self.warm_write.insert(
                cols.warm_key(i),
                cols.normal_impulse(i),
                [cols.tangent1_impulse(i), cols.tangent2_impulse(i)],
            );
        }
        core::mem::swap(&mut self.warm_read, &mut self.warm_write);
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
            .bodies()
            .iter()
            .any(|b| b.simulated && is_dynamic_row(b.inv_mass));
        if !has_dynamic {
            return;
        }

        // O8 phase 1 (BEFORE the solve): apply the wake conditions and build the
        // body→awake mask from last frame's sleep flags. This decides which islands
        // skip the solve + integrate THIS frame. A read-only borrow of the resource
        // for the duration of the build/solve (`asleep` / `awake_rows` are read);
        // the post-solve `end_step` reborrows mutably.
        let n_rows = scratch.bodies_len();
        let sleeping_active = sleep.is_some();
        if let Some(sleep) = sleep.as_mut() {
            sleep.begin_step(graph, n_rows);
        }
        // An immutable view used by `build_columns` (SOLVE skip) + the integrate
        // freeze; `None` when sleeping is off so the path is byte-identical.
        let sleep_view: Option<&IslandSleep> = sleep.as_deref();

        self.build_bodies(scratch.bodies());
        self.build_columns(manifolds, graph, scratch.bodies(), sleep_view);

        // O8 integrate-freeze (INTEGRATE half): capture the pre-solve hot state of
        // every slept-island body so the per-substep integrate (which streams the
        // WHOLE array — the O1 SIMD kernels are NOT per-lane masked) can be UNDONE for
        // slept rows after the loop. This freezes a slept body's position / rotation /
        // velocity without touching the audited integrate kernels. `frozen` is
        // capacity-reused (empty when sleeping is off — the byte-identical path).
        self.frozen.clear();
        if let Some(sleep) = sleep_view {
            for (row, b) in scratch.bodies().iter().enumerate() {
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
        //
        // P2 large-island gate: even with `parallel_solve` opted in, a step whose
        // LARGEST island is below `LARGE_ISLAND_CONSTRAINTS` cannot produce a color
        // worth a `pool.scope` dispatch — the largest color is bounded by the largest
        // island's manifold count, so no color can clear the solver's per-color
        // `MIN_PARALLEL_SLOTS_PER_COLOR` dispatch floor. Force the byte-identical
        // single-threaded path so the whole solve skips the ambient-pool probe + the
        // per-color span checks every pass (the build_graph + dispatch overhead the
        // analysis flags as pure loss below the crossover). This changes only WHERE
        // the colored solve runs, NEVER the bits: the inline / single-threaded path is
        // the SAME `solve_color` the `parallel == false` fallback uses, and the
        // {1, N}-worker bit-identity property makes large-island parallel == this. The
        // metric is a single scalar compare on a count `build_graph` already folded in
        // (zero extra pass). `[ESTIMATE]` threshold — P10 calibrates it (see the const).
        let parallel =
            config.parallel_solve && graph.max_island_constraints() >= LARGE_ISLAND_CONSTRAINTS;

        for _ in 0..substeps {
            // (1) Gravity integrate DYNAMIC bodies (shared O1 kernel). Single-
            // threaded — the BodyEffective build view's mut slice (no parallel
            // access in the integrate kernels).
            {
                let mut view = self.bodies.build_view();
                simd::apply_gravity(view.as_mut_slice(), scratch.bodies(), gravity, h, use_simd);
            }

            // (2) Warm-start apply. The colored sweeps reach bodies through the body
            // SOLVE VIEW and the contacts through the contact SOLVE VIEW (per-element
            // base+index) — single-threaded here, parallel in `solve_all_colors`; the
            // views are the SAME surface either way (the P1/P2 structural fix: no
            // whole-buffer reborrow on any contact / body path).
            Self::warm_start_apply(self.columns.solve_view(), self.bodies.solve_view());

            // (3)+(4) Soft normal + friction sweep ACROSS colors (Gauss-Seidel).
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

            // (5) Position integrate (scalar — the reference's MEASURED-SCALAR
            // choice for the AoS `BodyState`) then refresh the world inertia. The
            // BodyEffective read slice + the BodyState mut slice are distinct
            // ScratchColumns (no borrow conflict).
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

            // (6) Relax: re-solve bias-free to remove soft-bias energy.
            for _ in 0..config.relax_iterations {
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

        // Post-loop restitution (ONCE, velocity-only, bias-free).
        Self::apply_restitution(&mut self.columns, self.bodies.solve_view());

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
            let mut snap_view = scratch.bodies.build_view();
            let snapshot = snap_view.as_mut_slice();
            let mut eff_view = self.bodies.build_view();
            let eff_rows = eff_view.as_mut_slice();
            for &(row, snap) in &self.frozen {
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
