//! Physics O11 SP4 — COLORED-PARALLEL soft-body solve.
//!
//! Parallelizes the per-substep XPBD projection sweeps (SP1 distance, SP2
//! volume/tet, SP3 self-collision) of a single large [`SoftBody`] across the engine
//! threadpool using per-constraint-type graph colorings, so that within one color
//! every dynamic endpoint is write-disjoint. The shipped SERIAL path (SP1–SP3) stays
//! byte-identical (the SP4 C1 guard, proved in `soft_colored_sp4_baseline`); the new
//! colored path is run-to-run bit-deterministic and `{1, N}`-worker bit-identical.
//!
//! # Determinism boundary (INVIOLABLE — identical to SP1–SP3)
//!
//! Exact `mul`/`add`/`sub`/`div`/`sqrt`; NO FMA/`mul_add`/rsqrt/rcp/`Vec3::normalize`,
//! no fast-math, NO atomics in the reduction. Coloring changes only the
//! inter-constraint VISIT ORDER, which is value-equivalent because same-color
//! constraints are write-disjoint on dynamic rows (the C2 disjointness lemma). The
//! single dynamic predicate is [`is_dynamic_row`] — the SAME predicate the coloring
//! and the C1 write-guard share, so a row the solve writes is exactly a row the
//! coloring marked dynamic (anti-drift). The colored visit order DIFFERS from SP1's
//! authoring order, so SP4 has its OWN colored oracle (`{1, N}` bit-identity +
//! run-to-run), NOT an SP1-golden match.
//!
//! # The SINGLE leaf kernels (W3-A)
//!
//! The projection math is NOT duplicated: this module calls the SAME
//! [`project_distance`] / [`project_volume`] / [`project_self_pair`] / [`sweep`] /
//! [`build_hash`] definitions the serial path uses (widened to `pub(in
//! crate::soft)`). Duplicating the hardest determinism math is the drift hazard
//! C1/W3-A avoid. Only the DRIVER passes (predict, sdf-collide, velocity) are
//! duplicated here (simple `for i in 0..n` SoA loops) so the serial
//! [`step_body`](crate::soft::solver::step_body) is left LITERALLY untouched (C4).

use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_threadpool::try_with_active_pool;

use crate::math::Vec3;
use crate::resources::PhysicsConfig;
use crate::sdf_query::SdfField;
use crate::soft::collide::collide_sdf;
use crate::soft::component::SoftBody;
use crate::soft::self_collision::{build_hash, project_self_pair, project_self_pair_raw, sweep};
use crate::soft::solver::{
    REST_CLAMP_EPS, SoftCols, StepParams, project_distance_raw, project_volume_raw,
    step_body_serial, step_params,
};
use crate::solver::contact::is_dynamic_row;

/// Bit width of one occupancy word (a `u64` per-color particle bitset cell). Mirrors
/// the rigid `OCC_WORD_BITS` (resources.rs).
const OCC_WORD_BITS: usize = 64;

/// Minimum total slots a color must hold to amortize a `pool.scope` dispatch — below
/// it the color is solved INLINE on the calling thread (BIT-IDENTICAL to the
/// parallel split: within a color the constraints touch disjoint dynamic rows, so
/// inline == 1-worker == N-worker). Mirrors the rigid `MIN_PARALLEL_SLOTS_PER_COLOR`
/// (colored.rs:219) — kept equal so both colored surfaces share the threshold.
const MIN_PARALLEL_SLOTS_PER_COLOR: u32 = 256;

/// Emit MORE, work-balanced chunks than lanes so the Chase-Lev work-stealing pool
/// equalizes the lanes (an idle lane steals the next chunk). A pure, bench-tunable
/// perf knob: bit-identity is chunk-COUNT- AND chunk-SHAPE-independent (distinct
/// chunks touch disjoint dynamic rows), so this changes only WHERE work runs, never
/// the bits. Mirrors the rigid `CHUNKS_PER_WORKER` (colored.rs:248).
const CHUNKS_PER_WORKER: usize = 6;

/// Reads bit `p & 63` of word `p >> 6` in the color row starting at `base`.
#[inline]
fn occ_get(occ: &[u64], base: usize, p: u32) -> bool {
    let word = base + (p as usize >> 6);
    (occ[word] >> (p & 63)) & 1 != 0
}

/// Sets bit `p & 63` of word `p >> 6` in the color row starting at `base`.
#[inline]
fn occ_set(occ: &mut [u64], base: usize, p: u32) {
    let word = base + (p as usize >> 6);
    occ[word] |= 1u64 << (p & 63);
}

/// Greedy first-fit graph coloring of one constraint set over PARTICLE rows (SP4 D2,
/// ported from the rigid `ConstraintGraph::color_manifolds`, resources.rs:1910-1966).
///
/// Particle-indexed (vs the rigid body-indexed), arity-generalized to {2, 4} (vs the
/// rigid 2-body manifold), with [`is_dynamic_row`] occupancy on the [`SoftBody`]
/// `inv_mass` column. NO union-find / islanding (the soft body is one connected
/// component). Produces the CSR `color_start` / `color_items`: color `c`'s
/// constraint indices are `color_start[c]..color_start[c + 1]`, in ascending
/// constraint-index order within a color (a stable counting sort — D4/W3#6).
///
/// # Diff from the rigid source (W3 table)
///
/// 1. arity {2, 4} — the 4-arity free-test is the conjunction over all four
///    vertices in fixed order; a pinned vertex short-circuits its conjunct and is
///    never `occ_set` (so pinned rows impose no occupancy — ground is shared).
/// 2. `is_dynamic_row(SoftBody.inv_mass[p])` (vs body inv_mass) — the SAME
///    predicate, a different column, routed identically by the C1 guard + coloring.
/// 3. occupancy words over PARTICLE count (`n.div_ceil(64)`) — pure sizing.
/// 5. dedicated `chosen` / `cursor` scratch (no union-find reuse) — pure scratch.
/// 6. counting-sort CSR, stable, ascending within a color — identical determinism.
/// 7. fixed visit order = `0..m` (distance) / `0..k` (volume) / SP3 emission order
///    (self) — greedy first-fit is a pure function of the fixed visit order.
#[derive(Default)]
pub struct ParticleColorGraph {
    /// Per-color particle bitset matrix, addressed `color_occ[c * words + (p >> 6)]`,
    /// bit `p & 63`. Reused (cleared, never realloc'd) — the occupancy scratch. The
    /// `words` stride is recomputed per coloring from `n` (each `color_*` entry point
    /// derives it locally), so it is not carried as state.
    color_occ: Vec<u64>,
    /// Number of colors produced this coloring (`color_start.len() == n_colors + 1`).
    n_colors: usize,
    /// Per-constraint chosen color (scratch, reused as the counting-sort source).
    chosen: Vec<u32>,
    /// CSR offsets: `color_start[c]..[c + 1]` indexes `color_items`
    /// (`len == n_colors + 1`).
    color_start: Vec<u32>,
    /// CSR values: constraint indices grouped by color, ascending within a color.
    color_items: Vec<u32>,
    /// Counting-sort cursor scratch (a working copy of `color_start`).
    cursor: Vec<u32>,
    /// Debug-only "seen this color" particle bitset scratch for the coloring re-scan,
    /// reused (resized + zeroed per color) so the in-debug invariant check is itself
    /// ZERO per-step alloc in steady state (the C3b contract holds in debug too).
    /// Never touched in release (the re-scan is `cfg(debug_assertions)`).
    seen_scratch: Vec<u64>,
}

impl ParticleColorGraph {
    /// Reserves the coloring buffers for up to `n` particles and `m` constraints (no
    /// later realloc in steady state).
    fn reserve(&mut self, n: usize, m: usize) {
        let words = n.div_ceil(OCC_WORD_BITS);
        self.color_occ.reserve(words);
        self.chosen.reserve(m);
        self.color_start.reserve(m + 1);
        self.color_items.reserve(m);
        self.cursor.reserve(m + 1);
    }

    /// Number of colors produced by the last coloring.
    #[inline]
    pub fn n_colors(&self) -> usize {
        self.n_colors
    }

    /// The constraint indices of color `c` (the CSR slice, ascending constraint
    /// index).
    #[inline]
    pub fn color(&self, c: usize) -> &[u32] {
        let lo = self.color_start[c] as usize;
        let hi = self.color_start[c + 1] as usize;
        &self.color_items[lo..hi]
    }

    /// Total slots in color `c` (its CSR span width) — the parallel-threshold metric.
    #[inline]
    pub fn color_span(&self, c: usize) -> u32 {
        self.color_start[c + 1] - self.color_start[c]
    }

    /// Colors a 2-arity constraint set (distance edges or self-collision pairs):
    /// constraint `i` (`0..m`, fixed order) has endpoints `endpoint(i) = (a, b)`.
    ///
    /// Takes an INDEX CLOSURE (not an allocated `&[(u32, u32)]`) so the distance pass
    /// can color the body's SoA `c_a`/`c_b` columns directly — zero per-frame alloc
    /// (the C3b steady-state contract). `inv_mass[p]` decides dynamic occupancy via
    /// [`is_dynamic_row`]. A both-pinned constraint occupies no color (its conjuncts
    /// both short-circuit) and lands in color 0 — harmless: the solve's per-endpoint
    /// guard skips both writes.
    pub fn color_constraints_2<E>(&mut self, m: usize, inv_mass: &[f32], n: usize, endpoint: E)
    where
        E: Fn(usize) -> (u32, u32),
    {
        let words = n.div_ceil(OCC_WORD_BITS);
        self.begin(words, m);
        for i in 0..m {
            let (a, b) = endpoint(i);
            let a_dyn = is_dynamic_row(inv_mass[a as usize]);
            let b_dyn = is_dynamic_row(inv_mass[b as usize]);
            let color = self.find_free_color_2(words, a, b, a_dyn, b_dyn);
            let base = color * words;
            if a_dyn {
                occ_set(&mut self.color_occ, base, a);
            }
            if b_dyn {
                occ_set(&mut self.color_occ, base, b);
            }
            self.chosen.push(color as u32);
        }
        self.finish_csr(m);
        self.debug_assert_coloring_2(m, inv_mass, words, &endpoint);
    }

    /// Colors a 4-arity constraint set (volume tets): tet `i` (`0..k`, fixed order)
    /// has vertices `vert(i) = (v0, v1, v2, v3)`.
    ///
    /// Takes an INDEX CLOSURE so the volume pass colors the body's SoA `t*` columns
    /// directly (zero per-frame alloc). The free-test is the conjunction over all four
    /// vertices in fixed order; a pinned vertex short-circuits and is never `occ_set`.
    /// Distinct vertices are guaranteed by the constructor
    /// (`SoftBodyError::DegenerateTet`).
    pub fn color_constraints_4<V>(&mut self, k: usize, inv_mass: &[f32], n: usize, vert: V)
    where
        V: Fn(usize) -> (u32, u32, u32, u32),
    {
        let words = n.div_ceil(OCC_WORD_BITS);
        self.begin(words, k);
        for i in 0..k {
            let (v0, v1, v2, v3) = vert(i);
            let d0 = is_dynamic_row(inv_mass[v0 as usize]);
            let d1 = is_dynamic_row(inv_mass[v1 as usize]);
            let d2 = is_dynamic_row(inv_mass[v2 as usize]);
            let d3 = is_dynamic_row(inv_mass[v3 as usize]);
            let color = self.find_free_color_4(words, (v0, v1, v2, v3), (d0, d1, d2, d3));
            let base = color * words;
            if d0 {
                occ_set(&mut self.color_occ, base, v0);
            }
            if d1 {
                occ_set(&mut self.color_occ, base, v1);
            }
            if d2 {
                occ_set(&mut self.color_occ, base, v2);
            }
            if d3 {
                occ_set(&mut self.color_occ, base, v3);
            }
            self.chosen.push(color as u32);
        }
        self.finish_csr(k);
        self.debug_assert_coloring_4(k, inv_mass, words, &vert);
    }

    /// Resets the per-coloring state for a fresh coloring of `m` constraints over
    /// `words`-word color rows (capacity reused — clear, never realloc in steady
    /// state).
    fn begin(&mut self, _words: usize, m: usize) {
        self.color_occ.clear();
        self.n_colors = 0;
        self.chosen.clear();
        self.chosen.reserve(m);
    }

    /// Finds the lowest color where both dynamic endpoints are free, appending a new
    /// (zeroed) color row when none exists.
    #[inline]
    fn find_free_color_2(
        &mut self,
        words: usize,
        a: u32,
        b: u32,
        a_dyn: bool,
        b_dyn: bool,
    ) -> usize {
        let mut color = 0usize;
        loop {
            if color >= self.n_colors {
                self.color_occ.resize(self.color_occ.len() + words, 0);
                self.n_colors += 1;
            }
            let base = color * words;
            let free = (!a_dyn || !occ_get(&self.color_occ, base, a))
                && (!b_dyn || !occ_get(&self.color_occ, base, b));
            if free {
                return color;
            }
            color += 1;
        }
    }

    /// 4-arity variant: the free-test is the conjunction over all four vertices in
    /// fixed order.
    #[inline]
    fn find_free_color_4(
        &mut self,
        words: usize,
        v: (u32, u32, u32, u32),
        d: (bool, bool, bool, bool),
    ) -> usize {
        let mut color = 0usize;
        loop {
            if color >= self.n_colors {
                self.color_occ.resize(self.color_occ.len() + words, 0);
                self.n_colors += 1;
            }
            let base = color * words;
            let free = (!d.0 || !occ_get(&self.color_occ, base, v.0))
                && (!d.1 || !occ_get(&self.color_occ, base, v.1))
                && (!d.2 || !occ_get(&self.color_occ, base, v.2))
                && (!d.3 || !occ_get(&self.color_occ, base, v.3));
            if free {
                return color;
            }
            color += 1;
        }
    }

    /// Counting-sort the per-constraint `chosen` colors into the CSR `color_start` /
    /// `color_items` (stable → ascending constraint index within a color, D4/W3#6).
    fn finish_csr(&mut self, m: usize) {
        let n_colors = self.n_colors;
        self.color_start.clear();
        self.color_start.resize(n_colors + 1, 0);
        for &c in &self.chosen {
            self.color_start[c as usize + 1] += 1;
        }
        for c in 0..n_colors {
            self.color_start[c + 1] += self.color_start[c];
        }
        debug_assert_eq!(
            self.color_start[n_colors] as usize, m,
            "invariant: every constraint colored"
        );
        self.color_items.clear();
        self.color_items.resize(m, 0);
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.color_start[..n_colors]);
        for (ci, &c) in self.chosen.iter().enumerate() {
            let slot = self.cursor[c as usize] as usize;
            self.color_items[slot] = ci as u32;
            self.cursor[c as usize] += 1;
        }
    }

    /// Debug-only re-scan of the 2-arity coloring invariant: within every color, no
    /// two constraints share a dynamic particle (W3 `debug_assert_coloring`). Compiles
    /// to nothing in release; reuses [`Self::seen_scratch`] so it is itself zero
    /// per-step alloc in steady state (debug).
    #[inline]
    fn debug_assert_coloring_2<E>(&mut self, _m: usize, inv_mass: &[f32], words: usize, endpoint: &E)
    where
        E: Fn(usize) -> (u32, u32),
    {
        if cfg!(debug_assertions) {
            let words = words.max(1);
            self.seen_scratch.clear();
            self.seen_scratch.resize(words, 0);
            // Split the borrows: the CSR is read-only, `seen_scratch` is `&mut`.
            let starts = &self.color_start;
            let items = &self.color_items;
            let seen = &mut self.seen_scratch;
            for c in 0..self.n_colors {
                seen.iter_mut().for_each(|w| *w = 0);
                let lo = starts[c] as usize;
                let hi = starts[c + 1] as usize;
                for &ci in &items[lo..hi] {
                    let (a, b) = endpoint(ci as usize);
                    for &p in &[a, b] {
                        if is_dynamic_row(inv_mass[p as usize]) {
                            mark_seen(seen, p, c);
                        }
                    }
                }
            }
        }
    }

    /// Debug-only re-scan of the 4-arity coloring invariant (reuses
    /// [`Self::seen_scratch`]).
    #[inline]
    fn debug_assert_coloring_4<V>(&mut self, _k: usize, inv_mass: &[f32], words: usize, vert: &V)
    where
        V: Fn(usize) -> (u32, u32, u32, u32),
    {
        if cfg!(debug_assertions) {
            let words = words.max(1);
            self.seen_scratch.clear();
            self.seen_scratch.resize(words, 0);
            let starts = &self.color_start;
            let items = &self.color_items;
            let seen = &mut self.seen_scratch;
            for c in 0..self.n_colors {
                seen.iter_mut().for_each(|w| *w = 0);
                let lo = starts[c] as usize;
                let hi = starts[c + 1] as usize;
                for &ci in &items[lo..hi] {
                    let (v0, v1, v2, v3) = vert(ci as usize);
                    for &p in &[v0, v1, v2, v3] {
                        if is_dynamic_row(inv_mass[p as usize]) {
                            mark_seen(seen, p, c);
                        }
                    }
                }
            }
        }
    }
}

/// Asserts particle `p` is not yet seen in color `c`, then marks it (the in-debug
/// coloring re-scan helper).
#[inline]
fn mark_seen(seen: &mut [u64], p: u32, c: usize) {
    let w = p as usize >> 6;
    let bit = 1u64 << (p & 63);
    debug_assert_eq!(
        seen[w] & bit,
        0,
        "coloring invariant: color {c} reuses dynamic particle {p}"
    );
    seen[w] |= bit;
}

/// The SP4 colored-solve scratch (a `Resource`): the per-constraint-type colorings +
/// the per-substep self-collision pair list.
///
/// All buffers are reserve-sized at first use from the body's `n` / `m` / `k` /
/// `self_table_size` and reused thereafter (the rigid steady-state-zero /
/// growth-frame-realloc alloc contract, resources.rs:1946-1949): the common case
/// never reallocs, but a denser-than-reserved substep may resize-grow. The zero-alloc
/// gate asserts STEADY STATE (after a warm-up window), NOT first-N-frames (C3b).
#[derive(Resource, Default)]
pub struct SoftColorScratch {
    /// Distance-constraint coloring (immutable topology → colored once per frame,
    /// reused across substeps, D3).
    distance: ParticleColorGraph,
    /// Volume-tet coloring (immutable topology → colored once per frame, D3).
    volume: ParticleColorGraph,
    /// Self-collision pair coloring (position-dependent → recolored every substep,
    /// D3/C3).
    self_pairs: ParticleColorGraph,
    /// The per-substep self-collision pair list in SP3 emission order (C3a). Cleared
    /// + refilled each substep (capacity reused).
    pair_list: Vec<(u32, u32)>,
    /// `true` once the distance/volume colorings are computed for the current frame
    /// (so they are not recolored per substep — D3). Reset at the top of each
    /// `step_body_colored`.
    topology_colored: bool,
    /// DEBUG anti-vacuity counter: number of colors solved through the PARALLEL
    /// `pool.scope` dispatch (i.e. crossing `MIN_PARALLEL_SLOTS_PER_COLOR` AND
    /// emitting >1 chunk) since the last
    /// [`reset_parallel_counter`](Self::reset_parallel_counter). The `{1, N}` oracle
    /// asserts this is `>= 1` so the parallel path is non-vacuously exercised (W1).
    /// Maintained only under `cfg(debug_assertions)`.
    parallel_color_count: usize,
}

impl SoftColorScratch {
    /// Reserves all coloring + pair buffers for a body of `n` particles, `m` distance
    /// constraints, `k` tets, and `pair_cap` expected self-collision pairs (no later
    /// realloc in steady state).
    fn reserve(&mut self, n: usize, m: usize, k: usize, pair_cap: usize) {
        self.distance.reserve(n, m);
        self.volume.reserve(n, k);
        self.self_pairs.reserve(n, pair_cap);
        self.pair_list.reserve(pair_cap);
    }

    /// The DISTANCE coloring (test hook for the `{1, N}` oracle + disjointness gate).
    #[inline]
    pub fn distance_graph(&self) -> &ParticleColorGraph {
        &self.distance
    }

    /// The VOLUME coloring (test hook).
    #[inline]
    pub fn volume_graph(&self) -> &ParticleColorGraph {
        &self.volume
    }

    /// The SELF-COLLISION pair coloring (test hook).
    #[inline]
    pub fn self_pairs_graph(&self) -> &ParticleColorGraph {
        &self.self_pairs
    }

    /// The PARALLEL-dispatched color count since the last reset (DEBUG anti-vacuity
    /// hook for the `{1, N}` oracle — W1). Always `0` in release.
    #[inline]
    pub fn parallel_color_count(&self) -> usize {
        self.parallel_color_count
    }

    /// Resets the DEBUG parallel-dispatch counter (call before a step the `{1, N}`
    /// oracle measures).
    #[inline]
    pub fn reset_parallel_counter(&mut self) {
        self.parallel_color_count = 0;
    }
}

/// `Send` + `Sync`-marked PER-COLUMN raw bases of the live [`SoftBody`] + the colored
/// CSR, dispatched into the per-color worker closures (SP4 D5/W4, mirroring the rigid
/// [`ColorSolvePtrs`], colored.rs:270-335).
///
/// The W3-A soundness fix: a worker NEVER reborrows a whole `&mut SoftBody`. It
/// instead carries the per-column base pointers ([`SoftCols`]) and the leaf cores
/// ([`project_distance_raw`] / [`project_volume_raw`] / [`project_self_pair_raw`])
/// touch a column ELEMENT via `*base.add(p)`, which under Tree-Borrows retags ONLY
/// that element — not the whole allocation. Two workers writing the C2-disjoint
/// dynamic rows of one color thus form no overlapping protectors (the previous
/// `&mut *body` + `body.pos_x[a]` reborrowed the WHOLE `pos_x` buffer per worker — the
/// Miri-TB data race this removes).
///
/// Raw pointers are `!Send`/`!Sync` by default; this wrapper lets a worker task
/// capture them. The fields are **private** and reached only through the `&self`
/// accessor methods so a closure capturing the wrapper captures the WHOLE struct —
/// never an inner `*mut`/`*const` directly (Rust 2021+ disjoint capture would
/// otherwise see the bare pointer field and reject the closure as `!Send`). The CSR is
/// read via a RAW pointer (never a `&[u32]` borrow into the scratch live across
/// `scope.spawn`) — the Phase-9.3c bare-pointer discipline.
#[derive(Copy, Clone)]
struct SoftColorPtrs {
    /// The live body's per-column raw bases — the `*_raw` cores write ONLY the
    /// per-constraint dynamic elements through these (never a whole-buffer reborrow).
    cols: SoftCols,
    /// Base of the coloring CSR `color_items` — read as a RAW pointer (never a
    /// `&[u32]` borrow into the scratch) so no shared reference into the scratch is
    /// live while a worker writes through `cols` (TB-clean, 9.3c).
    color_items: *const u32,
}

impl SoftColorPtrs {
    /// The per-column raw bases (a `Copy` of [`SoftCols`]) handed to the `*_raw` cores.
    ///
    /// Forms NO reference of any kind — it returns the bare base pointers by value, so
    /// a worker writing through them retags only the elements its constraint names
    /// (never the whole column). Soundness rests on cross-worker DISJOINTNESS (the C2
    /// lemma); see the per-spawn SAFETY block.
    #[inline]
    fn cols(&self) -> SoftCols {
        self.cols
    }

    /// Reads `color_items[i]` via the raw pointer (no `&` borrow into the scratch).
    ///
    /// # Safety
    /// `i` must be a valid index into the live `color_items` column. Upheld by the
    /// dispatcher, which reads only indices within the color's `[lo, hi)` range.
    #[inline]
    unsafe fn color_item_at(&self, i: usize) -> usize {
        // SAFETY: `i` is in range per the method contract; `color_items` is the live
        //   base of the scratch CSR. A plain `*const u32` read forms no reference into
        //   the scratch, so it never conflicts with a worker's per-element `pos_*`
        //   writes through `cols` (the Tree-Borrows discipline, 9.3c).
        unsafe { *self.color_items.add(i) as usize }
    }
}

// SAFETY: the pointers name the live `SoftBody`'s columns + the `SoftColorScratch` CSR
//   borrowed for the whole `pool.scope` frame, whose Drop blocks (work-stealing join)
//   until every worker that captured the wrapper has completed — so both pointees
//   outlive every task body. The soundness of the concurrent per-element `pos_*`
//   writes rests entirely on DISJOINTNESS, stated in full in the per-spawn SAFETY
//   block: within a color the chunks write pairwise-disjoint DYNAMIC particle rows
//   (the C2 coloring lemma), and a SHARED PINNED row (`is_dynamic_row == false`) is
//   NEVER written (the C1 per-endpoint guard in the leaf cores). The immutable
//   topology + `inv_mass` columns are read-only shared across all workers, and — the
//   W3-A fix — a worker NEVER forms a `&mut SoftBody` or a `&mut [f32]` spanning a
//   column: every access is a per-element `*base.add(p)` through `SoftCols`, so the
//   disjoint-element writes never form overlapping Tree-Borrows protectors. The
//   wrapper has no interior mutability, so a shared `&` to it (the outer `pool.scope`
//   closure's capture across the spawn loop) is trivially safe — hence both `Send`
//   (cross-thread move into a task) and `Sync` (shared by the loop) hold.
unsafe impl Send for SoftColorPtrs {}
unsafe impl Sync for SoftColorPtrs {}

/// Advances every opted-in [`SoftBody`] by one COLORED-PARALLEL XPBD step (SP4) —
/// the sibling of [`physics_soft_step`](crate::soft::physics_soft_step), registered
/// in its place by
/// [`add_physics_soft_colored`](crate::plugin::add_physics_soft_colored) (the two
/// never both run).
///
/// Early-returns when [`PhysicsConfig::soft_body`] is `false` (the soft 0%-gate).
/// When [`PhysicsConfig::soft_body_colored`] is `false` it runs the SERIAL
/// [`step_body`](crate::soft::solver::step_body) per body (byte-identical to
/// `physics_soft_step` — the SP4 0%-gate); when `true` it runs the colored projection
/// per body via [`step_body_colored`].
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam` read via a
// `&*` reborrow — the same false-positive the rigid + serial soft systems document.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_soft_step_colored(
    mut query: Query<&mut SoftBody>,
    cfg: Res<PhysicsConfig>,
    field: Res<SdfField>,
    mut scratch: ResMut<SoftColorScratch>,
) {
    if !cfg.soft_body {
        // The 0%-gate: an un-opted world does no soft-body work.
        return;
    }
    let field = &*field;
    let p = step_params(&cfg);
    let scratch = &mut *scratch;
    for body in query.iter_mut() {
        if cfg.soft_body_colored {
            step_body_colored(body, field, &p, &cfg, scratch);
        } else {
            // The SP4 0%-gate: behave exactly like the serial `physics_soft_step`.
            step_body_serial(body, field, &p, &cfg);
        }
    }
}

/// One full COLORED-PARALLEL XPBD advance of a single soft body (`substeps`
/// substeps) — the C4 interleaving sibling of
/// [`step_body`](crate::soft::solver::step_body).
///
/// Reproduces `step_body`'s EXACT per-substep order inside one `for _ in 0..substeps`
/// loop: predict → distance(colored) → volume(colored) → self-collision →
/// SDF-collide → velocity. Coupling is OUT OF SCOPE for SP4 (the colored soft step is
/// the non-coupling path; the coupling boundary stays serial, IM-1). The DRIVER
/// passes (predict, sdf-collide, velocity) are DUPLICATED here as simple SoA loops so
/// the serial `step_body` is left literally untouched (C4); only the projection LEAF
/// kernels are shared (C1 proves them byte-preserving serially).
fn step_body_colored(
    body: &mut SoftBody,
    field: &SdfField,
    p: &StepParams,
    cfg: &PhysicsConfig,
    scratch: &mut SoftColorScratch,
) {
    let n = body.particle_count();
    let m = body.constraint_count();
    let k = body.tet_count();
    debug_assert!(body.pos_x.len() == n && body.pos_y.len() == n && body.pos_z.len() == n);
    debug_assert!(body.inv_mass.len() == n);
    debug_assert!(body.c_a.len() == m && body.c_b.len() == m);
    debug_assert!(
        body.t0.len() == k && body.t1.len() == k && body.t2.len() == k && body.t3.len() == k
    );
    debug_assert!(
        body.particle_radius >= 0.0,
        "invariant: particle_radius must be >= 0"
    );

    // Reserve the coloring buffers once from this body's dimensions (steady-state
    // zero-alloc; may grow on a denser substep — the C3b alloc contract). The particle
    // count loosely bounds the expected self-collision pair count.
    scratch.reserve(n, m, k, n);
    // Color the IMMUTABLE distance/volume topology ONCE per frame, reused across
    // substeps (D3).
    color_topology_once(body, scratch);

    let radius = body.particle_radius;
    let gh = p.gravity * p.h;
    let h = p.h;
    let inv_h = p.inv_h;
    let visc = 1.0 - cfg.soft_damping;
    let rest_clamp = cfg.soft_rest_clamp;
    let clamp_sq = REST_CLAMP_EPS * REST_CLAMP_EPS;
    let self_collision_iters = cfg.self_collision_iters;
    let self_colored = cfg.soft_self_collision_colored;

    for _ in 0..p.substeps {
        // Predict — DUPLICATED from `step_body` (driver pass, simple SoA loop).
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                body.vel_x[i] += gh.x;
                body.vel_y[i] += gh.y;
                body.vel_z[i] += gh.z;
                body.prev_x[i] = body.pos_x[i];
                body.prev_y[i] = body.pos_y[i];
                body.prev_z[i] = body.pos_z[i];
                body.pos_x[i] += body.vel_x[i] * h;
                body.pos_y[i] += body.vel_y[i] * h;
                body.pos_z[i] += body.vel_z[i] * h;
            } else {
                body.prev_x[i] = body.pos_x[i];
                body.prev_y[i] = body.pos_y[i];
                body.prev_z[i] = body.pos_z[i];
            }
        }

        // Distance projection — colored, ONE GS pass (color-by-color), via the SHARED
        // `project_distance` leaf (the C1 guard is already inside it).
        solve_distance_colored(body, scratch, h);

        // Volume projection — colored, ONE GS pass, via the SHARED `project_volume`.
        solve_volume_colored(body, scratch, h);

        // Self-collision — build the hash ONCE per substep and sweep `iters` times
        // (C4: mirrors `resolve_self_collision`). Either serial (the SP4 default) or
        // colored (emit/color/solve color-by-color against the single hash).
        resolve_self_collision_colored(body, scratch, self_collision_iters, radius, self_colored);

        // SDF collision — DUPLICATED from `step_body`'s non-coupling arm (driver pass;
        // one-sided, no reaction).
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                let pos = Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i]);
                let pos = collide_sdf(field, pos, radius);
                body.pos_x[i] = pos.x;
                body.pos_y[i] = pos.y;
                body.pos_z[i] = pos.z;
            }
        }

        // Velocity update — DUPLICATED from `step_body`'s non-coupling arm (driver
        // pass: `(pos - prev) * inv_h`, then the SP2 D5 viscous scale + optional rest
        // clamp, fixed order). No coupling baseline (SP4 is the non-coupling path).
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                let mut vx = (body.pos_x[i] - body.prev_x[i]) * inv_h;
                let mut vy = (body.pos_y[i] - body.prev_y[i]) * inv_h;
                let mut vz = (body.pos_z[i] - body.prev_z[i]) * inv_h;
                vx *= visc;
                vy *= visc;
                vz *= visc;
                if rest_clamp && (vx * vx + vy * vy + vz * vz) < clamp_sq {
                    vx = 0.0;
                    vy = 0.0;
                    vz = 0.0;
                }
                body.vel_x[i] = vx;
                body.vel_y[i] = vy;
                body.vel_z[i] = vz;
            }
        }
    }

    // Reset the per-frame topology-colored latch so the NEXT frame recolors (the
    // topology is immutable WITHIN a frame's substeps; the frame boundary clears the
    // latch defensively — a body could be replaced between frames).
    scratch.topology_colored = false;
}

/// Colors the immutable distance + volume topology ONCE per frame (D3): zips the
/// body's constraint columns into endpoint/vertex tuples and runs the per-type
/// colorer. Idempotent within a frame (the `topology_colored` latch).
fn color_topology_once(body: &SoftBody, scratch: &mut SoftColorScratch) {
    if scratch.topology_colored {
        return;
    }
    let n = body.particle_count();
    // Distance edges `(c_a[i], c_b[i])` in fixed order `0..m` — colored straight from
    // the SoA columns via an index closure (no per-frame Vec, the C3b contract).
    let m = body.constraint_count();
    let ca = &body.c_a;
    let cb = &body.c_b;
    scratch
        .distance
        .color_constraints_2(m, &body.inv_mass, n, |i| (ca[i], cb[i]));
    // Volume tets `(t0, t1, t2, t3)[i]` in fixed order `0..k` — likewise from the SoA
    // columns.
    let k = body.tet_count();
    let (t0, t1, t2, t3) = (&body.t0, &body.t1, &body.t2, &body.t3);
    scratch
        .volume
        .color_constraints_4(k, &body.inv_mass, n, |i| (t0[i], t1[i], t2[i], t3[i]));
    scratch.topology_colored = true;
}

/// Solves the distance-constraint coloring color-by-color (0..n_colors sequentially,
/// the scope-Drop barrier between), each color parallel above the threshold else
/// inline (D5). Calls the SHARED raw core [`project_distance_raw`] per constraint
/// through [`SoftCols`] (per-element access — no whole-buffer reborrow on the parallel
/// path).
fn solve_distance_colored(body: &mut SoftBody, scratch: &mut SoftColorScratch, h: f32) {
    let parallel_count = &mut scratch.parallel_color_count;
    let graph = &scratch.distance;
    for c in 0..graph.n_colors() {
        dispatch_color(body, graph, c, parallel_count, move |cols, ci| {
            // SAFETY: `cols` names the live body's column bases; `ci` is a valid
            //   constraint index (a `color_items` value), so its endpoints index valid
            //   rows. On the parallel path the dispatcher invokes this only on this
            //   color's constraints with disjoint dynamic endpoints across workers (C2);
            //   serially the caller holds the unique `&mut SoftBody`.
            unsafe { project_distance_raw(cols, ci, h) };
        });
    }
}

/// Solves the volume-tet coloring color-by-color via the SHARED raw core
/// [`project_volume_raw`] through [`SoftCols`].
fn solve_volume_colored(body: &mut SoftBody, scratch: &mut SoftColorScratch, h: f32) {
    let parallel_count = &mut scratch.parallel_color_count;
    let graph = &scratch.volume;
    for c in 0..graph.n_colors() {
        dispatch_color(body, graph, c, parallel_count, move |cols, ci| {
            // SAFETY: `cols` names the live body's column bases; `ci` is a valid tet
            //   index (a `color_items` value), so its vertices index valid rows. On the
            //   parallel path the dispatcher invokes this only on this color's tets with
            //   disjoint dynamic vertices across workers (C2); serially the caller holds
            //   the unique `&mut SoftBody`.
            unsafe { project_volume_raw(cols, ci, h) };
        });
    }
}

/// The COLORED self-collision pass (C4): build the spatial hash ONCE per substep,
/// then sweep `iters` times. Each sweep either projects serially (the SP4 default,
/// byte-identical to the serial pass on a one-body world) or — when `colored` — emits
/// the SP3-ordered pair set against the single hash, colors it, and solves it
/// color-by-color.
fn resolve_self_collision_colored(
    body: &mut SoftBody,
    scratch: &mut SoftColorScratch,
    iters: usize,
    radius: f32,
    colored: bool,
) {
    if iters == 0 || radius <= 0.0 {
        return;
    }
    let n = body.particle_count();
    if n < 2 {
        return;
    }
    let table = body.self_table_size();
    let cell = 2.0 * radius;
    let inv_cell = 1.0 / cell;

    // C4: ONE hash build per substep, reused across the `iters` sweeps (mirrors
    // `resolve_self_collision`, self_collision.rs:136-141).
    build_hash(body, table, inv_cell);

    for _ in 0..iters {
        if colored {
            // Emit the SP3-ordered pair list against the single hash (C3a), color it
            // (recolored every sweep — the pair set is position-dependent), then solve
            // color-by-color via the SHARED `project_self_pair` leaf.
            scratch.pair_list.clear();
            {
                let pairs = &mut scratch.pair_list;
                sweep(body, table, inv_cell, |_body, i, j| {
                    pairs.push((i as u32, j as u32));
                });
            }
            // Color the emitted pairs straight from `pair_list` via an index closure
            // (disjoint borrows of the two scratch fields — `self_pairs` `&mut`, the
            // `pair_list` `&`).
            {
                let pairs = &scratch.pair_list;
                let pm = pairs.len();
                scratch
                    .self_pairs
                    .color_constraints_2(pm, &body.inv_mass, n, |i| pairs[i]);
            }
            let n_colors = scratch.self_pairs.n_colors();
            for c in 0..n_colors {
                solve_self_color(body, scratch, c, cell);
            }
        } else {
            // Serial sweep over the single hash (the SP4 default self-collision arm).
            sweep(body, table, inv_cell, |body, i, j| {
                project_self_pair(body, i, j, cell);
            });
        }
    }
}

/// Solves one self-collision color: maps each colored PAIR index through the pair
/// list to `(i, j)` and projects via the SHARED [`project_self_pair`]. Inline below
/// the threshold, else parallel CSR chunks (the same `dispatch_color` discipline).
fn solve_self_color(body: &mut SoftBody, scratch: &mut SoftColorScratch, color: usize, cell: f32) {
    // Borrow-split: the pair list is read-only inside the per-constraint closure, the
    // counter is `&mut`. Take a raw `*const (u32, u32)` to the pair list so the
    // closure (which crosses `scope.spawn`) holds no `&[...]` borrow into the scratch
    // (TB-clean, 9.3c); reads stay in-bounds (`slot < pair_list.len()`, the colorer's
    // invariant).
    let pairs_ptr = scratch.pair_list.as_ptr();
    let pairs_len = scratch.pair_list.len();
    let parallel_count = &mut scratch.parallel_color_count;
    let graph = &scratch.self_pairs;
    let pl = PairListPtr {
        ptr: pairs_ptr,
        len: pairs_len,
    };
    dispatch_color(body, graph, color, parallel_count, move |cols, slot| {
        // SAFETY: `slot` is a pair index `< pairs_len` (the colorer only emits
        //   indices into the pair list it colored), and the pair list outlives the
        //   `pool.scope` frame (it is the scratch's own Vec, untouched during the
        //   solve). A `*const` read forms no reference into the scratch.
        let (i, j) = unsafe { pl.get(slot) };
        // SAFETY: `cols` names the live body's column bases; `i`/`j` are valid particle
        //   rows (the sweep emitted them). On the parallel path the dispatcher invokes
        //   this only on this color's pairs with disjoint dynamic endpoints across
        //   workers (C2); serially the caller holds the unique `&mut SoftBody`.
        unsafe { project_self_pair_raw(cols, i as usize, j as usize, cell) };
    });
}

/// `Send` + `Sync` raw view of the read-only self-collision pair list, captured by
/// the per-constraint closure (a `*const` so no `&[...]` borrow into the scratch is
/// live across `scope.spawn` — the 9.3c discipline).
#[derive(Copy, Clone)]
struct PairListPtr {
    ptr: *const (u32, u32),
    len: usize,
}

impl PairListPtr {
    /// Reads pair `slot`.
    ///
    /// # Safety
    /// `slot < self.len`, and the pair list must outlive the read. Upheld by the
    /// caller (the colorer only emits in-range indices; the list lives in the scratch
    /// for the whole solve).
    #[inline]
    unsafe fn get(&self, slot: usize) -> (u32, u32) {
        debug_assert!(slot < self.len, "invariant: pair index in range");
        // SAFETY: `slot < len` and the pointee outlives the read (method contract).
        unsafe { *self.ptr.add(slot) }
    }
}

// SAFETY: `PairListPtr` is a read-only view of a `Vec<(u32, u32)>` that lives in the
//   `SoftColorScratch` for the whole `pool.scope` frame (Drop joins all workers before
//   the scratch is touched again). No worker writes it. So sharing it across tasks
//   (`Sync`) and moving it into a task (`Send`) is sound.
unsafe impl Send for PairListPtr {}
unsafe impl Sync for PairListPtr {}

/// Dispatches one color: inline below `MIN_PARALLEL_SLOTS_PER_COLOR`, else
/// work-balanced contiguous CSR chunks across the threadpool (D5, mirroring the rigid
/// `solve_color_parallel`, colored.rs:1778-1897).
///
/// `solve_one(cols, ci)` projects the constraint whose CSR `color_items` value is `ci`
/// (a constraint / pair index) through the per-column raw bases [`SoftCols`] — a
/// per-element `*base.add(p)` access, NEVER a `&mut SoftBody` or a `&mut [f32]`
/// spanning a column (the W3-A soundness discipline). The inline path walks the
/// color's CSR slice in ascending order against `SoftCols` built once from the unique
/// `&mut SoftBody`; the parallel path spawns one task per contiguous CSR chunk, each
/// resolving its chunk's `color_items` indices through `solve_one` against the SAME
/// `SoftCols` carried by the `Send`/`Sync` [`SoftColorPtrs`] (W4). Colors are solved
/// 0..n_colors sequentially by the caller with the scope-Drop barrier between (no
/// atomics).
fn dispatch_color<F>(
    body: &mut SoftBody,
    graph: &ParticleColorGraph,
    color: usize,
    parallel_count: &mut usize,
    solve_one: F,
) where
    F: Fn(SoftCols, usize) + Send + Sync + Copy,
{
    let lo = graph.color_start[color] as usize;
    let hi = graph.color_start[color + 1] as usize;
    if lo == hi {
        return;
    }
    let span = graph.color_span(color);

    // W1 min-work threshold: a SMALL color does not amortize a `pool.scope` dispatch,
    // so solve it INLINE on the calling thread — BIT-IDENTICAL to the parallel split
    // (within a color the constraints touch disjoint dynamic rows ⇒ inline == 1-worker
    // == N-worker), so it changes only WHERE the color is solved. The inline solve goes
    // through the SAME `SoftCols` per-element cores as the parallel split (byte-equal:
    // `body.pos_x[a]` and `*pos_x.add(a)` name the same storage), so the unique `&mut
    // SoftBody` here yields the bases once and the cores write only per-element.
    if span < MIN_PARALLEL_SLOTS_PER_COLOR {
        let cols = SoftCols::from_body(body);
        for &ci in graph.color(color) {
            solve_one(cols, ci as usize);
        }
        return;
    }

    // Grab the ambient pool (set by `Schedule::run`'s `install` frame). With no pool
    // attached (ad-hoc / no-scheduler call / Miri), fall back to the inline solve so
    // the result still matches the single-threaded path exactly.
    let dispatched = try_with_active_pool(|pool| {
        // Emit MORE, work-balanced chunks than lanes so the work-stealing pool
        // equalizes the lanes. The dispatcher lane work-steals too, so the lane pool
        // is `num_threads + 1`. Bit-identity is chunk-count/shape-independent (a pure
        // perf knob).
        let lanes = pool.num_threads() + 1;
        let total = hi - lo;
        let n_chunks = (lanes * CHUNKS_PER_WORKER).clamp(1, total);
        let target = total.div_ceil(n_chunks).max(1);

        // Anti-vacuity (W1 (i)): this color crossed the threshold AND will dispatch
        // through the parallel scope. Counted UNCONDITIONALLY — one increment per
        // parallel-dispatched color per step (NOT per constraint, so off the hot
        // inner loop and perf-negligible; it touches only a scratch field, never the
        // solve result, so the 0%-gate holds). The count must be real in RELEASE so
        // the `{1, N}` bit-identity oracle is non-vacuous there: were it debug-only,
        // a `cargo test --release` run of the oracle could pass even if BOTH worker
        // counts silently fell back to the inline path — hiding an SP4 regression in
        // the exact (optimized, multi-worker) profile where the data race bites.
        *parallel_count += 1;

        // Send + Sync per-column raw bases of the live body + the CSR base. Each worker
        // writes only its chunk's DISJOINT dynamic rows per element (the C2 lemma), so
        // the per-element aliasing is sound — and a whole-buffer reborrow is now
        // un-typeable on this path (see the per-spawn SAFETY block).
        let ptrs = SoftColorPtrs {
            cols: SoftCols::from_body(body),
            color_items: graph.color_items.as_ptr(),
        };

        pool.scope(|scope| {
            let mut chunk_lo = lo;
            while chunk_lo < hi {
                let chunk_hi = (chunk_lo + target).min(hi);
                debug_assert!(chunk_lo < chunk_hi, "invariant: a CSR chunk is non-empty");
                let task_lo = chunk_lo;
                let task_hi = chunk_hi;
                scope.spawn(move || {
                    // SAFETY (cross-worker disjoint aliasing — the SP4 D5 soundness
                    //   argument, mirroring the rigid per-spawn block colored.rs:1935):
                    //   - `ptrs` names the live `SoftBody`'s columns + the
                    //     `SoftColorScratch` CSR borrowed for the whole `dispatch_color`
                    //     frame; `pool.scope`'s Drop blocks (work-stealing join) until
                    //     every spawned task completes, so both pointees outlive every
                    //     task body — no use-after-free, no escape past the borrow.
                    //   - PER-ELEMENT access (the W3-A fix) + DISJOINTNESS make the
                    //     concurrent `pos_*` writes sound:
                    //     * A worker writes columns ONLY through the `*_raw` cores, which
                    //       do `*pos_*.add(p) += ..` per element — under Tree-Borrows that
                    //       retags ONLY element `p`, NOT the whole `pos_*` buffer. No
                    //       worker forms a `&mut SoftBody` or a `&mut [f32]` spanning a
                    //       column, so disjoint-element writes never overlap protectors
                    //       (the whole-buffer reborrow that was the prior Miri-TB race is
                    //       now un-typeable on this path).
                    //     * Each task resolves ONLY the CSR sub-range `[task_lo,
                    //       task_hi)` of THIS color's `color_items`; distinct chunks
                    //       have non-overlapping CSR ranges (they partition the color's
                    //       constraints), and within ONE color each DYNAMIC particle
                    //       belongs to at most one constraint (the C2 greedy first-fit
                    //       coloring lemma), so distinct chunks WRITE disjoint dynamic
                    //       `pos_*` ELEMENTS — no two workers write the same element.
                    //     * A SHARED PINNED row (`is_dynamic_row == false`) is NEVER
                    //       written: the C1 per-endpoint guard inside every leaf core
                    //       skips the write for any `inv_mass == 0.0` row (a write that
                    //       was already a value no-op), so a pinned element two same-color
                    //       constraints share is read-only across workers (the Phase
                    //       9.3c foreign-write failure mode this avoids).
                    //     * The immutable topology columns (`c_a`/`c_b`/`c_rest`/
                    //       `c_compliance`/`t*`) and `inv_mass` are read ONLY (the leaf
                    //       cores never write them), so the concurrent reads through the
                    //       `*const` bases alias them read-only — sound.
                    //   Therefore the per-task per-element `pos_*` writes touch only
                    //   provably-disjoint elements across workers (plus read-only shared
                    //   columns) — no UB.
                    let cols = ptrs.cols();
                    for s in task_lo..task_hi {
                        // SAFETY: `s` is within `[lo, hi)` of the live `color_items`.
                        let ci = unsafe { ptrs.color_item_at(s) };
                        solve_one(cols, ci);
                    }
                });
                chunk_lo = chunk_hi;
            }
        });
    });

    if dispatched.is_none() {
        // No pool attached → run the color inline (BIT-IDENTICAL to the parallel
        // split; the result still matches the single-threaded path). The unique `&mut
        // SoftBody` yields the column bases once; the cores write only per-element.
        let cols = SoftCols::from_body(body);
        for &ci in graph.color(color) {
            solve_one(cols, ci as usize);
        }
    }
    // `parallel_count` is read under debug only; silence the unused-var lint in
    // release builds where the increment is `cfg`'d out.
    let _ = parallel_count;
}
