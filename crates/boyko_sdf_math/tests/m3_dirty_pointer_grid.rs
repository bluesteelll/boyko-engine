//! SDF brick-atlas campaign M3 — the INCREMENTAL DIRTY POINTER-GRID correctness gate
//! (M1 layer, CPU-only, no GPU).
//!
//! THE load-bearing property: `build_dirty_pointer_grid` (re-classify ONLY the dirty cells of an
//! already-built grid) produces a result BIT-IDENTICAL to `build_pointer_grid` (full re-classify of
//! every cell) over the same authority — after ANY sequence of `move_edit` / `set_edit` / `push`
//! mutations. A missed (or over-eager) dirty cell shows up here as a per-cell divergence with the
//! exact `(ix, iy, iz)`, the two classes, and the mutation step that produced it.
//!
//! The #1 dynamic bug this guards is the GHOST: a moved edit's OLD cells must be re-classified to
//! their edit-absent state (the union-dirty rule sweeps `aabbs[i] ∪ prev_aabb[i]`). If the prev-AABB
//! half of the sweep is dropped, the old cells keep the moved edit's `Surface` label — a phantom the
//! full re-classify would have cleared. The bit-identity assert catches exactly that.
//!
//! `boyko_sdf_math` is a TRUE LEAF (zero third-party deps), so this file uses a hand-rolled
//! deterministic SplitMix64 PRNG instead of `proptest` — preserving the leaf invariant while still
//! sweeping hundreds of randomized edit sequences. Failures are 100% reproducible from the printed
//! seed.

use boyko_sdf_math::brick::{
    build_dirty_pointer_grid, build_pointer_grid, dirty_world_aabb, PointerGrid,
};
use boyko_sdf_math::{sdf_op, BrickClass, SdfEdit, SdfEditField, MAX_SDF_EDITS};

/// A fixed base seed so any failure is reproducible (printed in the panic message).
const SEED_BASE: u64 = 0x5D_F0_03_17_2A_4B_6C_8E;

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic PRNG (SplitMix64) — keeps `boyko_sdf_math` a zero-dep leaf.
// ─────────────────────────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        // SplitMix64 — a tiny, well-mixed, deterministic generator.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 // [0,1)
    }
    /// A coordinate in `[-3.0, 3.0]` (inside the [-4, 4] near-field grid extent).
    fn coord(&mut self) -> f32 {
        -3.0 + self.unit() * 6.0
    }
    fn radius(&mut self) -> f32 {
        0.2 + self.unit() * 0.8 // [0.2, 1.0]
    }
    /// A blend radius: `0.0` (hard) ~half the time, else `(0.05, 0.6]` (smooth). A
    /// smooth op makes the field fold blend across primitives, so a change to one edit
    /// ripples through every later smooth combine — the M3 dirty set must cover that.
    fn smoothness(&mut self) -> f32 {
        if self.below(2) == 0 {
            0.0
        } else {
            0.05 + self.unit() * 0.55
        }
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % n as u64) as u32
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn default_grid() -> PointerGrid {
    PointerGrid::default_near_field()
}

fn random_edit(rng: &mut Rng) -> SdfEdit {
    let c = [rng.coord(), rng.coord(), rng.coord()];
    let op = if rng.below(2) == 0 { sdf_op::UNION } else { sdf_op::SUBTRACT };
    let k = rng.smoothness();
    if rng.below(2) == 0 {
        SdfEdit::sphere(c, rng.radius(), op, k)
    } else {
        let he = [rng.radius(), rng.radius(), rng.radius()];
        SdfEdit::box_shape(c, he, op, k)
    }
}

/// A FULL classify into a fresh buffer (the from-scratch oracle).
fn full_grid(field: &SdfEditField, grid: &PointerGrid) -> Vec<u32> {
    let mut out = vec![0u32; grid.cell_count()];
    build_pointer_grid(field, grid, &mut out);
    out
}

/// Asserts `incr == full` cell-by-cell, reporting the FIRST divergence with full context.
fn assert_grid_bit_identical(
    incr: &[u32],
    full: &[u32],
    grid: &PointerGrid,
    seed: u64,
    step: usize,
    what: &str,
) {
    assert_eq!(incr.len(), full.len(), "grid length mismatch");
    let w = grid.dims[0];
    let h = grid.dims[1];
    for (idx, (a, b)) in incr.iter().zip(full.iter()).enumerate() {
        if a != b {
            let iz = idx as u32 / (w * h);
            let rem = idx as u32 % (w * h);
            let iy = rem / w;
            let ix = rem % w;
            panic!(
                "M3 dirty pointer-grid DIVERGENCE (seed={seed:#x}, step={step}, op={what}): \
                 cell ({ix},{iy},{iz}) incremental={a} full={b} — \
                 a missed/wrong dirty cell (likely a dropped prev-AABB ghost)"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. INCREMENTAL == FULL bit-identical over random mutation sequences (the core gate).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dirty_pointer_grid_equals_full_over_random_sequences() {
    let grid = default_grid();
    let n_seeds = 240usize; // >= 200 sequences (per the matrix)
    let steps_per_seed = 24usize;

    for s in 0..n_seeds {
        let seed = SEED_BASE.wrapping_add(s as u64 * 0x1000_0001);
        let mut rng = Rng::new(seed);

        // Seed an initial scene of 1..=6 edits, full-bake, snapshot the dirty ledger.
        let mut field = SdfEditField::new();
        let init = 1 + rng.below(6) as usize;
        for _ in 0..init {
            field.push(random_edit(&mut rng));
        }
        field.bump_gen();

        let mut incr = vec![0u32; grid.cell_count()];
        build_pointer_grid(&field, &grid, &mut incr);
        field.clear_dirty(); // post-bake snapshot

        for step in 0..steps_per_seed {
            let kind = rng.below(3);
            let count = field.count;
            match kind {
                0 if count > 0 => {
                    let i = rng.below(count) as usize;
                    field.move_edit(i, [rng.coord(), rng.coord(), rng.coord()]);
                }
                1 if count > 0 => {
                    let i = rng.below(count) as usize;
                    field.set_edit(i, random_edit(&mut rng));
                }
                _ => {
                    if (field.count as usize) < MAX_SDF_EDITS {
                        field.push(random_edit(&mut rng));
                    } else if count > 0 {
                        let i = rng.below(count) as usize;
                        field.move_edit(i, [rng.coord(), rng.coord(), rng.coord()]);
                    }
                }
            }
            field.bump_gen();

            // Incremental: patch ONLY the dirty cells of the previously-built grid.
            build_dirty_pointer_grid(&field, &grid, &mut incr);

            // Oracle: a from-scratch full classify of the CURRENT field.
            let full = full_grid(&field, &grid);

            assert_grid_bit_identical(&incr, &full, &grid, seed, step, "seq");

            field.clear_dirty(); // diff the next mutation against the freshly-baked state.
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. UNION-DIRTY / NO-GHOST on the pointer grid (the #1 dynamic bug).
// ─────────────────────────────────────────────────────────────────────────────

/// Moves a single edit from a region A to a DISJOINT region B. The OLD region-A cells must be
/// re-classified to their edit-absent state (NO ghost) and the NEW region-B cells must now hold the
/// moved surface — both verified by bit-identity to a full re-bake of the moved field.
#[test]
fn dirty_pointer_grid_far_move_leaves_no_ghost() {
    let grid = default_grid();

    // One small sphere far in -X (region A) plus a static anchor sphere at the origin so the field
    // is non-trivial. After the move the sphere is far in +X (region B), disjoint from A.
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0)); // static anchor
    field.push(SdfEdit::sphere([-2.5, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0)); // the mover (A)
    field.bump_gen();

    let mut incr = vec![0u32; grid.cell_count()];
    build_pointer_grid(&field, &grid, &mut incr);
    field.clear_dirty();

    // Record which cells were Surface around region A before the move (the ghost candidates).
    let pre_a_surface: Vec<usize> = incr
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c == BrickClass::Surface as u32)
        .map(|(i, _)| i)
        .collect();
    assert!(!pre_a_surface.is_empty(), "region A must have surface cells before the move");

    // Move the sphere to +X (region B), disjoint from A.
    field.move_edit(1, [2.5, 0.0, 0.0]);
    field.bump_gen();

    build_dirty_pointer_grid(&field, &grid, &mut incr);
    let full = full_grid(&field, &grid);
    assert_grid_bit_identical(&incr, &full, &grid, 0, 0, "far_move");

    // Explicit ghost check: the cell at A's old center is no longer Surface unless the static anchor
    // still reaches it (it does not — the anchor is at the origin, A is at x=-2.5).
    let a_cell = grid_cell_of(&grid, [-2.5, 0.0, 0.0]);
    let a_idx = cell_index(&grid, a_cell);
    assert_eq!(
        incr[a_idx],
        full[a_idx],
        "ghost: A's old cell {a_cell:?} diverges from the full re-bake (the moved surface lingers)"
    );
    assert_ne!(
        incr[a_idx],
        BrickClass::Surface as u32,
        "ghost: A's old cell {a_cell:?} still holds the moved sphere's surface after a far move"
    );
}

/// SURFACE→EMPTY: shrinking an edit to a zero-radius point (so its band no longer crosses its old
/// cells) must re-classify those cells empty — no ghost surface left behind.
#[test]
fn dirty_pointer_grid_surface_to_empty_clears() {
    let grid = default_grid();

    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0)); // anchor
    field.push(SdfEdit::sphere([2.0, 0.0, 0.0], 0.6, sdf_op::UNION, 0.0)); // vanisher
    field.bump_gen();

    let mut incr = vec![0u32; grid.cell_count()];
    build_pointer_grid(&field, &grid, &mut incr);
    field.clear_dirty();

    // Replace the vanisher with a degenerate union sphere at a far, disjoint location AND radius ~0,
    // so its old cells lose all surface. (A SUBTRACT->nothing or a tiny far sphere both work; we use
    // a tiny far sphere so the "vanish at old location" is unambiguous.)
    field.set_edit(1, SdfEdit::sphere([3.5, 3.5, 3.5], 0.05, sdf_op::UNION, 0.0));
    field.bump_gen();

    build_dirty_pointer_grid(&field, &grid, &mut incr);
    let full = full_grid(&field, &grid);
    assert_grid_bit_identical(&incr, &full, &grid, 0, 0, "surface_to_empty");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. DIRTY-CELL-COUNT < total (the perf win) + no-op returns 0.
// ─────────────────────────────────────────────────────────────────────────────

/// A localized small-edit move dirties FEW cells of the 16³=4096-cell grid — not all of them.
#[test]
fn dirty_pointer_grid_localized_move_touches_fewer_than_total() {
    let grid = default_grid();
    let total = grid.cell_count() as u32;

    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 0.3, sdf_op::UNION, 0.0));
    field.bump_gen();

    let mut incr = vec![0u32; grid.cell_count()];
    build_pointer_grid(&field, &grid, &mut incr);
    field.clear_dirty();

    // A small nudge (0.3 world) of a small sphere.
    field.move_edit(0, [0.3, 0.0, 0.0]);
    field.bump_gen();
    let touched = build_dirty_pointer_grid(&field, &grid, &mut incr);

    assert!(touched > 0, "a real move must dirty at least one cell");
    assert!(
        touched < total,
        "a localized move dirtied {touched}/{total} cells — the incremental win is lost if it is all"
    );
    // Sanity: the incremental result still matches a full re-bake.
    let full = full_grid(&field, &grid);
    assert_grid_bit_identical(&incr, &full, &grid, 0, 0, "localized");
    println!("[M3-grid] localized move dirtied {touched}/{total} cells");
}

/// A `bump_gen` with NO geometry change (prev == current AABBs) yields ZERO dirty cells and an empty
/// dirty AABB (the no-op fast path: the prior grid is already current).
#[test]
fn dirty_pointer_grid_noop_touches_zero() {
    let grid = default_grid();

    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.5, 0.0, 0.0], 0.3, sdf_op::UNION, 0.0));
    field.bump_gen();

    let mut incr = vec![0u32; grid.cell_count()];
    build_pointer_grid(&field, &grid, &mut incr);
    field.clear_dirty(); // prev := current → nothing dirty

    assert!(dirty_world_aabb(&field).is_none(), "a clean field has no dirty world AABB");
    let touched = build_dirty_pointer_grid(&field, &grid, &mut incr);
    assert_eq!(touched, 0, "a no-op (no AABB change) must dirty zero cells");
}

// ─────────────────────────────────────────────────────────────────────────────
// Grid index helpers (cell of a world point + linear index).
// ─────────────────────────────────────────────────────────────────────────────

fn grid_cell_of(grid: &PointerGrid, p: [f32; 3]) -> [u32; 3] {
    let mut c = [0u32; 3];
    for a in 0..3 {
        let rel = (p[a] - grid.origin[a]) / grid.brick_world;
        let i = rel.floor().max(0.0) as u32;
        c[a] = i.min(grid.dims[a] - 1);
    }
    c
}

fn cell_index(grid: &PointerGrid, c: [u32; 3]) -> usize {
    let w = grid.dims[0];
    let h = grid.dims[1];
    (c[0] + c[1] * w + c[2] * w * h) as usize
}
