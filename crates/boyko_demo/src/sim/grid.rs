//! Uniform spatial grid for neighbor queries (plan §6.4 / D11).
//!
//! A bounded world box plus a fixed cell size turns "find my neighbors" from
//! O(n^2) into O(n + cells). The grid is a flat **CSR** (compressed-sparse-row)
//! structure — two `Vec<u32>`s, NOT a `HashMap` and NOT a `Vec` per cell — built
//! by a counting sort each frame:
//!
//! * `cell_starts[c .. c+1]` is the half-open slice of `entity_idx` holding the
//!   indices of the items in cell `c` (CSR offsets, length `cell_count + 1`).
//! * `entity_idx` holds item indices (rows into the caller's position array)
//!   grouped by cell.
//!
//! Both vectors are sized once (the world box + cell size fix `cell_count`, and
//! the item count is the boid population) and **refilled** in place each frame —
//! no per-frame allocation (plan principle 5 / §11.2). The grid indexes whatever
//! position array the caller passes (the boids index a pre-tick snapshot, D12),
//! so it has no dependency on the ECS or any component type.

use boyko_macros::Resource;

/// A uniform CSR spatial grid over a centered square world box (plan §6.4).
///
/// Indexes a flat array of 2D positions: [`rebuild`](Self::rebuild) bins each
/// position into a cell by a counting sort, then [`for_each_neighbor`](Self::for_each_neighbor)
/// walks the 3x3 cell block around a query point. Sized once via
/// [`new`](Self::new); refilled each frame with no allocation.
#[derive(Resource)]
pub struct SpatialGrid {
    /// World half-extent on each axis; the box is `[-half, half]^2`.
    half_extent: f32,
    /// Edge length of one (square) cell in world units. Set to the neighbor
    /// radius so a 3x3 block covers every point within one radius.
    cell_size: f32,
    /// Number of cells per axis. The grid is `dim x dim` cells.
    dim: u32,
    /// CSR offsets, length `dim*dim + 1`. `cell_starts[c]..cell_starts[c+1]`
    /// indexes [`entity_idx`](Self::entity_idx) for cell `c`. Sized once.
    cell_starts: Vec<u32>,
    /// Item indices (rows into the caller's position array) grouped by cell.
    /// Length grows to the item count on rebuild; capacity is reused.
    entity_idx: Vec<u32>,
    /// Per-cell write cursor reused by the scatter pass (same length as
    /// `cell_starts`). A persistent buffer instead of a per-frame clone, so
    /// `rebuild` allocates nothing in steady state (plan §11.2).
    cursor: Vec<u32>,
}

impl SpatialGrid {
    /// Builds a grid over the box `[-half_extent, half_extent]^2` with cells of
    /// edge `cell_size` (the neighbor radius), pre-sized for up to `max_items`.
    ///
    /// `cell_starts` is sized to the (fixed) cell count and `entity_idx` reserves
    /// `max_items`, so steady-state rebuilds never reallocate. `cell_size` is
    /// clamped to a small positive floor so a degenerate (zero/negative) radius
    /// from the UI cannot produce a zero `dim` or a divide-by-zero.
    pub fn new(half_extent: f32, cell_size: f32, max_items: usize) -> Self {
        let cell_size = cell_size.max(MIN_CELL_SIZE);
        let dim = Self::dim_for(half_extent, cell_size);
        let cell_count = (dim as usize) * (dim as usize);
        Self {
            half_extent,
            cell_size,
            dim,
            cell_starts: vec![0; cell_count + 1],
            entity_idx: Vec::with_capacity(max_items),
            cursor: vec![0; cell_count + 1],
        }
    }

    /// Cells per axis for a box half-extent and cell size: `ceil(2*half / cell)`,
    /// floored at 1 so the grid always has at least one cell.
    #[inline]
    fn dim_for(half_extent: f32, cell_size: f32) -> u32 {
        let span = 2.0 * half_extent;
        ((span / cell_size).ceil() as u32).max(1)
    }

    /// Resizes the grid for a new cell size (e.g. the UI changed the boid
    /// radius), reusing the existing allocations where possible.
    ///
    /// Recomputes `dim` and, only if the cell count changed, resizes
    /// `cell_starts`. A no-op when the rounded `dim` is unchanged, so dragging
    /// the radius slider within one cell-count band costs nothing.
    pub fn set_cell_size(&mut self, cell_size: f32) {
        let cell_size = cell_size.max(MIN_CELL_SIZE);
        let dim = Self::dim_for(self.half_extent, cell_size);
        self.cell_size = cell_size;
        if dim != self.dim {
            self.dim = dim;
            let cell_count = (dim as usize) * (dim as usize);
            self.cell_starts.resize(cell_count + 1, 0);
            self.cursor.resize(cell_count + 1, 0);
        }
    }

    /// Number of cells along each axis.
    #[inline]
    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// Maps a world position to its cell coordinates, clamped to `[0, dim)` on
    /// each axis so a point on or just past the boundary still lands in an edge
    /// cell (the integrator keeps positions in-box, but clamping makes the grid
    /// robust to a stray out-of-box point).
    #[inline]
    fn cell_coords(&self, x: f32, y: f32) -> (u32, u32) {
        let last = self.dim - 1;
        // Shift the origin to the box corner so coordinates are non-negative.
        let gx = ((x + self.half_extent) / self.cell_size) as i32;
        let gy = ((y + self.half_extent) / self.cell_size) as i32;
        let cx = gx.clamp(0, last as i32) as u32;
        let cy = gy.clamp(0, last as i32) as u32;
        (cx, cy)
    }

    /// Linear cell index from cell coordinates (row-major: `y*dim + x`).
    #[inline]
    fn cell_index(&self, cx: u32, cy: u32) -> usize {
        (cy as usize) * (self.dim as usize) + (cx as usize)
    }

    /// Rebuilds the grid from a contiguous `positions` slice by a counting sort
    /// (plan §6.4), allocation-free in steady state.
    ///
    /// `positions[i]` is the world position of item `i`; after this call,
    /// [`for_each_neighbor`](Self::for_each_neighbor) yields item indices into
    /// this same array. Convenience wrapper over [`rebuild_with`](Self::rebuild_with).
    pub fn rebuild(&mut self, positions: &[[f32; 2]]) {
        self.rebuild_with(positions.len(), |i| positions[i]);
    }

    /// Rebuilds the grid from `count` items whose positions are produced by
    /// `pos_of(i)` (plan §6.4), allocation-free in steady state.
    ///
    /// Lets the caller bin positions stored AoS inside a larger record (e.g. the
    /// boids' `BoidState { pos, vel }`) without materializing a separate
    /// positions array. Four passes: clear histogram, count per cell, prefix-sum
    /// into CSR offsets, scatter indices. O(n + cells). After this call,
    /// [`for_each_neighbor`](Self::for_each_neighbor) yields item indices in
    /// `0..count`.
    pub fn rebuild_with<F: Fn(usize) -> [f32; 2]>(&mut self, count: usize, pos_of: F) {
        let cell_count = (self.dim as usize) * (self.dim as usize);
        debug_assert_eq!(self.cell_starts.len(), cell_count + 1);

        // Pass 1: clear the histogram (reuse the allocation).
        self.cell_starts.fill(0);

        // Pass 2: count items per cell. Store the count for cell `c` at
        // `cell_starts[c + 1]` so the prefix sum below turns it directly into the
        // exclusive-start offset for cell `c`.
        for i in 0..count {
            let p = pos_of(i);
            let (cx, cy) = self.cell_coords(p[0], p[1]);
            let c = self.cell_index(cx, cy);
            self.cell_starts[c + 1] += 1;
        }

        // Pass 3: prefix-sum into CSR start offsets. After this,
        // `cell_starts[c]` is the first slot of cell `c` and `cell_starts[c+1]`
        // its end; `cell_starts[cell_count] == count`.
        for c in 0..cell_count {
            self.cell_starts[c + 1] += self.cell_starts[c];
        }

        // Pass 4: scatter item indices into their cell's slice. The persistent
        // `cursor` buffer walks a per-cell write head initialized to each cell's
        // start (copied from the CSR offsets — an in-place overwrite, no alloc).
        // `entity_idx` reuses its capacity, sized to the item count.
        self.entity_idx.clear();
        self.entity_idx.resize(count, 0);
        self.cursor.copy_from_slice(&self.cell_starts);
        for i in 0..count {
            let p = pos_of(i);
            let (cx, cy) = self.cell_coords(p[0], p[1]);
            let c = self.cell_index(cx, cy);
            let slot = self.cursor[c] as usize;
            self.entity_idx[slot] = i as u32;
            self.cursor[c] += 1;
        }

        debug_assert_eq!(self.cell_starts[cell_count] as usize, count);
        debug_assert_eq!(self.entity_idx.len(), count);
    }

    /// Calls `f(item_index)` for every item in the 3x3 cell block centered on the
    /// cell containing `(x, y)` (plan §6.4 neighbor query).
    ///
    /// The 3x3 block at cell edge = the neighbor radius guarantees every item
    /// within one radius of the query point is visited (plus some in adjacent
    /// cells, which the caller distance-filters). Edge cells simply skip
    /// out-of-range neighbor columns/rows. Borrows nothing mutably — the boids'
    /// `par_iter` force pass calls this read-only from each worker.
    #[inline]
    pub fn for_each_neighbor<F: FnMut(u32)>(&self, x: f32, y: f32, mut f: F) {
        let (cx, cy) = self.cell_coords(x, y);
        let last = self.dim - 1;
        let x0 = cx.saturating_sub(1);
        let x1 = (cx + 1).min(last);
        let y0 = cy.saturating_sub(1);
        let y1 = (cy + 1).min(last);

        for ny in y0..=y1 {
            for nx in x0..=x1 {
                let c = self.cell_index(nx, ny);
                let start = self.cell_starts[c] as usize;
                let end = self.cell_starts[c + 1] as usize;
                // SAFETY-free: `start <= end <= entity_idx.len()` by CSR
                // construction (monotonic offsets, last == len), so the slice is
                // always in bounds.
                for &item in &self.entity_idx[start..end] {
                    f(item);
                }
            }
        }
    }
}

/// Smallest cell edge the grid accepts, in world units. Guards against a
/// zero/negative neighbor radius from the UI producing a zero `dim` or a
/// divide-by-zero in [`SpatialGrid::cell_coords`].
const MIN_CELL_SIZE: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects the item indices the 3x3 neighbor walk yields for a query point,
    /// sorted for stable comparison.
    fn neighbors(grid: &SpatialGrid, x: f32, y: f32) -> Vec<u32> {
        let mut out = Vec::new();
        grid.for_each_neighbor(x, y, |i| out.push(i));
        out.sort_unstable();
        out
    }

    /// CSR invariant: after a rebuild, the last offset equals the item count and
    /// `entity_idx` holds exactly that many indices (every item placed once).
    #[test]
    fn csr_offsets_cover_all_items() {
        let mut grid = SpatialGrid::new(100.0, 10.0, 16);
        let positions = [
            [-90.0, -90.0],
            [0.0, 0.0],
            [90.0, 90.0],
            [5.0, -5.0],
            [-5.0, 5.0],
        ];
        grid.rebuild(&positions);

        let cell_count = (grid.dim() as usize) * (grid.dim() as usize);
        assert_eq!(
            grid.cell_starts[cell_count] as usize,
            positions.len(),
            "the final CSR offset must equal the item count"
        );
        assert_eq!(
            grid.entity_idx.len(),
            positions.len(),
            "every item must be scattered into entity_idx exactly once"
        );
    }

    /// A point's own cell is returned by its neighbor query, and a point far away
    /// (more than one cell distant) is NOT — the 3x3 block is local.
    #[test]
    fn neighbor_query_is_local() {
        // cell = 10 over [-100, 100] -> 20 cells/axis. Two points 100 units apart
        // are ~10 cells apart, far outside any shared 3x3 block.
        let mut grid = SpatialGrid::new(100.0, 10.0, 8);
        let positions = [[0.0, 0.0], [50.0, 50.0]];
        grid.rebuild(&positions);

        // Querying at item 0's position returns item 0 but not the distant item 1.
        let near = neighbors(&grid, 0.0, 0.0);
        assert!(near.contains(&0), "the query point's own cell is included");
        assert!(
            !near.contains(&1),
            "an item ~10 cells away is outside the 3x3 neighborhood"
        );

        // Querying at item 1's position returns item 1 but not item 0.
        let far = neighbors(&grid, 50.0, 50.0);
        assert!(far.contains(&1), "item 1 is in its own cell's neighborhood");
        assert!(!far.contains(&0), "item 0 is too far to be a neighbor of item 1");
    }

    /// Items in adjacent cells ARE returned (the 3x3 block spans the neighbors),
    /// confirming the query is not collapsed to a single cell.
    #[test]
    fn neighbor_query_spans_adjacent_cells() {
        let mut grid = SpatialGrid::new(100.0, 10.0, 8);
        // Three points within one cell-width of the origin: same cell, and the
        // two diagonally-adjacent cells. All must appear in a query at origin.
        let positions = [[0.0, 0.0], [9.0, 9.0], [-9.0, -9.0]];
        grid.rebuild(&positions);

        let got = neighbors(&grid, 0.0, 0.0);
        assert_eq!(
            got,
            vec![0, 1, 2],
            "all three within one cell of the origin are 3x3 neighbors"
        );
    }

    /// `set_cell_size` resizes the CSR offset buffer when the cell count changes,
    /// and a subsequent rebuild still satisfies the CSR invariant — the
    /// allocation-stable resize path the UI radius slider drives.
    #[test]
    fn resize_then_rebuild_is_consistent() {
        let mut grid = SpatialGrid::new(100.0, 10.0, 8);
        grid.set_cell_size(5.0); // doubles dim -> larger cell_starts
        let positions = [[0.0, 0.0], [1.0, 1.0], [-50.0, 50.0]];
        grid.rebuild(&positions);

        let cell_count = (grid.dim() as usize) * (grid.dim() as usize);
        assert_eq!(grid.cell_starts.len(), cell_count + 1, "offsets resized");
        assert_eq!(
            grid.cell_starts[cell_count] as usize,
            positions.len(),
            "CSR invariant holds after a resize + rebuild"
        );
    }

    /// An out-of-box position is clamped into an edge cell rather than panicking
    /// (defensive: the integrator keeps boids in-box, but a stray point must not
    /// index out of bounds).
    #[test]
    fn out_of_box_position_clamps() {
        let mut grid = SpatialGrid::new(100.0, 10.0, 4);
        // Well outside [-100, 100] on both axes.
        let positions = [[1_000.0, -1_000.0]];
        grid.rebuild(&positions);
        // The single item must still be findable via a query at the same point.
        let got = neighbors(&grid, 1_000.0, -1_000.0);
        assert!(got.contains(&0), "a clamped out-of-box item is still indexed");
    }
}
