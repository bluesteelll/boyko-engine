//! Physics step resources — the preallocated, reused step buffers (plan D1/D4/
//! IM-1).
//!
//! Every `Vec` here is sized once and refilled each step (cleared, capacity
//! reused): the foundation does no per-step / per-manifold heap allocation
//! (principle 5). [`SolverScratch`] is the dense, row-indexed snapshot the
//! gather→solve→apply pipeline addresses by [`BodyIndex`]
//! — see [`crate::systems`].

use boyko_macros::Resource;
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
}

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
    /// Pre-filter candidate pairs (before the sphere-bound test), reused.
    candidates: Vec<(BodyIndex, BodyIndex)>,
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
        self.oversized.clear();
        self.candidates.clear();

        let n = bodies.len();
        if n == 0 {
            return;
        }

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
    }
}
