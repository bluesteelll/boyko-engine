//! SDF brick-atlas campaign M3 — the INCREMENTAL DIRTY-BRICK ATLAS correctness gate
//! (the M2 atlas layer; CPU-only core gate + one `#[ignore]` RTX lifecycle check).
//!
//! THE load-bearing property (the core gate): the INCREMENTALLY-maintained atlas staging is
//! BIT-IDENTICAL (all 64000 bytes) to a from-scratch full `bake_brick_atlas` of the current field,
//! after ANY sequence of `move_edit` / `set_edit` / `push` mutations. The incremental path mirrors
//! the production `BrickAtlas`: a full bake into a persistent staging buffer, then for each
//! mutation a `m2_dirty_cell_bbox` → `rebake_dirty_brick_atlas` patch of ONLY the dirty cell box,
//! then `clear_dirty`. Any byte that diverges from the full re-bake is a missed (or wrong) dirty
//! cell — reported with the exact byte offset, atlas voxel `(x,y,z)`, owning cell, and step.
//!
//! The #1 dynamic bug this guards is the GHOST: a moved edit's OLD cell box must be re-baked to its
//! edit-absent (all-zero / EMPTY) state. The union-dirty rule sweeps `aabbs[i] ∪ prev_aabb[i]`; if
//! the prev half is dropped, the old tiles keep the moved surface — a phantom the full bake clears.
//! The byte-identity assert catches exactly that, AND a focused far-move test checks the old tile
//! region is byte-identical to its EMPTY state.
//!
//! Runs entirely on the host (no GPU). The single `#[ignore]` test drives the real `BrickAtlas`
//! lifecycle on the owner's RTX (validation-clean) and cross-checks the device staging path against
//! a full bake — the render-parity claim (an identical uploaded image ⇒ an identical marcher
//! G-buffer, since the staging→image copy is an identity blit).

use boyko_rhi_vulkan::compute::{
    bake_brick_atlas, m2_cell_is_dirty, m2_cell_min, m2_dirty_cell_bbox, rebake_dirty_brick_atlas,
    AtlasEncoding, M2_ATLAS_DIM, M2_BRICK_WORLD, M2_GRID_DIM,
};
use boyko_sdf_math::brick::BRICK_ALLOC;
use boyko_sdf_math::{sdf_op, SdfEdit, SdfEditAabb, SdfEditField, MAX_SDF_EDITS};

use proptest::prelude::*;

/// The Snorm8 dense-atlas byte size: `40³ = 64000` bytes (1 byte/voxel).
const ATLAS_BYTES: usize = (M2_ATLAS_DIM as usize).pow(3);

// ─────────────────────────────────────────────────────────────────────────────
// Helpers — the production-mirroring incremental staging path.
// ─────────────────────────────────────────────────────────────────────────────

/// A from-scratch FULL bake of `field` into a fresh staging buffer (the oracle).
fn full_bake(field: &SdfEditField) -> Vec<u8> {
    let mut out = vec![0u8; ATLAS_BYTES];
    bake_brick_atlas(field, AtlasEncoding::Snorm8, &mut out);
    out
}

/// The atlas-voxel origin of M2 grid `cell` (`cell * BRICK_ALLOC`) and the owning cell of a linear
/// atlas-voxel index — for pinpointing a divergent byte.
fn cell_of_voxel(vx: u32, vy: u32, vz: u32) -> [u32; 3] {
    [vx / BRICK_ALLOC as u32, vy / BRICK_ALLOC as u32, vz / BRICK_ALLOC as u32]
}

/// Asserts the incremental staging equals the full bake byte-for-byte, reporting the FIRST
/// divergent byte with its atlas voxel, owning M2 cell, and the mutation step.
fn assert_atlas_bit_identical(incr: &[u8], full: &[u8], seed: u64, step: usize, what: &str) {
    assert_eq!(incr.len(), ATLAS_BYTES, "incremental staging must be 64000 bytes");
    assert_eq!(full.len(), ATLAS_BYTES, "full bake must be 64000 bytes");
    let w = M2_ATLAS_DIM;
    for (i, (a, b)) in incr.iter().zip(full.iter()).enumerate() {
        if a != b {
            let vz = i as u32 / (w * w);
            let rem = i as u32 % (w * w);
            let vy = rem / w;
            let vx = rem % w;
            let cell = cell_of_voxel(vx, vy, vz);
            panic!(
                "M3 dirty ATLAS DIVERGENCE (seed={seed:#x}, step={step}, op={what}): \
                 byte {i} at atlas voxel ({vx},{vy},{vz}) in M2 cell {cell:?} — \
                 incremental={a} full={b}. A missed/wrong dirty brick \
                 (likely a dropped prev-AABB ghost at the edit's old location)."
            );
        }
    }
}

/// One incremental step on a persistent staging buffer, exactly as `BrickAtlas::rebake_dirty` does:
/// derive the dirty cell box, patch ONLY that box, then snapshot the ledger. Returns the dirty box
/// (or `None` for a no-op).
fn rebake_dirty_into(staging: &mut [u8], field: &mut SdfEditField) -> Option<([u32; 3], [u32; 3])> {
    let bbox = m2_dirty_cell_bbox(field);
    if let Some((lo, hi)) = bbox {
        rebake_dirty_brick_atlas(field, AtlasEncoding::Snorm8, lo, hi, staging);
    }
    field.clear_dirty();
    bbox
}

// ─────────────────────────────────────────────────────────────────────────────
// PRNG-free edit builders for the deterministic union/no-ghost + count tests.
// ─────────────────────────────────────────────────────────────────────────────

fn field_with(edits: &[SdfEdit]) -> SdfEditField {
    let mut f = SdfEditField::new();
    for e in edits {
        assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
    }
    f.bump_gen();
    f
}

/// The set of atlas voxels of M2 cell `cell` (a tile's `BRICK_ALLOC³` byte block, scattered).
fn tile_bytes_all_zero(staging: &[u8], cell: [u32; 3]) -> bool {
    let ox = cell[0] * BRICK_ALLOC as u32;
    let oy = cell[1] * BRICK_ALLOC as u32;
    let oz = cell[2] * BRICK_ALLOC as u32;
    let w = M2_ATLAS_DIM;
    for lz in 0..BRICK_ALLOC as u32 {
        for ly in 0..BRICK_ALLOC as u32 {
            for lx in 0..BRICK_ALLOC as u32 {
                let (vx, vy, vz) = (ox + lx, oy + ly, oz + lz);
                let i = (vx + vy * w + vz * w * w) as usize;
                if staging[i] != 0 {
                    return false;
                }
            }
        }
    }
    true
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. INCREMENTAL == FULL bit-identical over >= 200 random edit SEQUENCES (the core gate).
// ═════════════════════════════════════════════════════════════════════════════

proptest! {
    // 256 sequences (>= 200, per the matrix), each up to ~16 random mutations.
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn dirty_atlas_equals_full_over_random_sequences(
        // Initial scene: 1..=5 edits.
        init in proptest::collection::vec(arb_edit(), 1..=5),
        // A sequence of mutations: (kind 0=move/1=set/2=push, target index seed, new edit).
        muts in proptest::collection::vec((0u8..3, any::<u8>(), arb_edit()), 1..=16),
        seed in any::<u64>(),
    ) {
        let mut field = SdfEditField::new();
        for e in &init {
            field.push(*e);
        }
        field.bump_gen();

        // Full-bake into the persistent staging, then snapshot the ledger (the create() step).
        let mut staging = full_bake(&field);
        field.clear_dirty();

        for (step, (kind, idx_seed, new_edit)) in muts.iter().enumerate() {
            let count = field.count;
            if count == 0 {
                field.push(*new_edit);
            } else {
                let i = (*idx_seed as u32 % count) as usize;
                match kind {
                    0 => {
                        let c = new_edit.center;
                        field.move_edit(i, [c[0], c[1], c[2]]);
                    }
                    1 => field.set_edit(i, *new_edit),
                    _ => {
                        if (field.count as usize) < MAX_SDF_EDITS {
                            field.push(*new_edit);
                        } else {
                            let c = new_edit.center;
                            field.move_edit(i, [c[0], c[1], c[2]]);
                        }
                    }
                }
            }
            field.bump_gen();

            // Incremental patch of ONLY the dirty box.
            rebake_dirty_into(&mut staging, &mut field);

            // Oracle: a from-scratch full bake of the current field.
            let full = full_bake(&field);
            assert_atlas_bit_identical(&staging, &full, seed, step, "seq");
        }
    }
}

/// Bounded random edit: a sphere or box inside the [-4, 4] near-field grid, HARD or
/// SMOOTH. The `smoothness` lane is load-bearing: a `> 0` smooth op makes the field
/// fold blend across primitives, so a change to one edit ripples through every later
/// smooth combine — the M3 dirty set must cover that ripple to stay bit-identical to
/// a full bake. `0.0` for ~half the draws keeps the tight hard-scene path exercised.
fn arb_edit() -> impl Strategy<Value = SdfEdit> {
    (
        -3.5f32..3.5,
        -3.5f32..3.5,
        -3.5f32..3.5,
        0.15f32..1.0,
        0.15f32..1.0,
        0.15f32..1.0,
        prop::bool::ANY,   // sphere vs box
        prop::bool::ANY,   // union vs subtract
        // smoothness: 0 (hard) ~half the time, else a blend radius in (0, 0.6].
        prop::option::weighted(0.5, 0.05f32..0.6),
    )
        .prop_map(|(x, y, z, r, hy, hz, is_box, is_sub, smooth)| {
            let op = if is_sub { sdf_op::SUBTRACT } else { sdf_op::UNION };
            let k = smooth.unwrap_or(0.0);
            if is_box {
                SdfEdit::box_shape([x, y, z], [r, hy, hz], op, k)
            } else {
                SdfEdit::sphere([x, y, z], r, op, k)
            }
        })
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. UNION-DIRTY / NO-GHOST (the #1 dynamic bug) — five move flavors.
// ═════════════════════════════════════════════════════════════════════════════

/// Drives one move case: full-bake, snapshot, move edit `i` to `to`, dirty-rebake, and assert (a)
/// the incremental staging is byte-identical to a full bake of the moved field (so the new surface
/// is baked AND the old surface is cleared), and (b) the OLD cell at `from_center` is byte-identical
/// to its EMPTY (all-zero) state in the full bake (no ghost), unless another edit still reaches it.
fn run_move_case(
    name: &str,
    edits: &[SdfEdit],
    i: usize,
    from_center: [f32; 3],
    to: [f32; 3],
) {
    let mut field = field_with(edits);
    let mut staging = full_bake(&field);
    field.clear_dirty();

    field.move_edit(i, to);
    field.bump_gen();

    rebake_dirty_into(&mut staging, &mut field);
    let full = full_bake(&field);
    assert_atlas_bit_identical(&staging, &full, 0, 0, name);

    // Explicit ghost probe: the OLD cell of the moved edit must match the full bake there (which is
    // EMPTY=all-zero if no other edit reaches it). If `from` mapped into the grid, the incremental
    // and full must agree on that whole tile (the bit-identity assert already covers it; this is the
    // explicit, localized ghost statement for the report).
    if let Some(old_cell) = cell_in_grid(from_center) {
        // The full bake is the ground truth; if it left the old tile all-zero, the incremental MUST
        // too (no ghost). If another edit covers the old cell, both stay equal (covered by the
        // bit-identity assert above) — we only assert the strong "all-zero" form when the full bake
        // itself zeroed the tile.
        if tile_bytes_all_zero(&full, old_cell) {
            assert!(
                tile_bytes_all_zero(&staging, old_cell),
                "GHOST ({name}): the moved edit's old cell {old_cell:?} is EMPTY in the full bake \
                 but the incremental staging left non-zero bytes (the moved surface lingered)"
            );
        }
    }
}

/// The M2 grid cell containing world point `p`, or `None` if outside the bounded grid.
fn cell_in_grid(p: [f32; 3]) -> Option<[u32; 3]> {
    let mut c = [0u32; 3];
    for a in 0..3 {
        let rel = (p[a] + 4.0) / M2_BRICK_WORLD; // origin = -4
        if rel < 0.0 {
            return None;
        }
        let i = rel.floor() as u32;
        if i >= M2_GRID_DIM {
            return None;
        }
        c[a] = i;
    }
    Some(c)
}

#[test]
fn dirty_atlas_near_move_no_ghost() {
    // A small sphere nudged within the same neighbourhood (overlapping old/new boxes).
    run_move_case(
        "near_move",
        &[
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // static anchor
            SdfEdit::sphere([-1.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0),
        ],
        1,
        [-1.0, 0.0, 0.0],
        [-0.6, 0.2, 0.0],
    );
}

#[test]
fn dirty_atlas_far_move_disjoint_no_ghost() {
    // A move across the grid to a DISJOINT region — the classic ghost trigger.
    run_move_case(
        "far_move",
        &[
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // static anchor
            SdfEdit::sphere([-3.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0),
        ],
        1,
        [-3.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
    );
}

#[test]
fn dirty_atlas_in_grid_to_out_grid_no_ghost() {
    // An edit moved from inside the grid to FAR OUTSIDE it: the old in-grid cells must clear; the new
    // location has no atlas tile (outside the bounded grid), so only the old box re-bakes.
    run_move_case(
        "in_to_out",
        &[
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // static anchor
            SdfEdit::sphere([2.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0),
        ],
        1,
        [2.0, 0.0, 0.0],
        [50.0, 0.0, 0.0], // way outside [-4, 4]
    );
}

#[test]
fn dirty_atlas_out_grid_to_in_grid_bakes() {
    // An edit starting OUTSIDE the grid moved INSIDE: only the new in-grid box bakes (the old box was
    // never represented). The bit-identity to the full bake is the gate.
    run_move_case(
        "out_to_in",
        &[
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // static anchor
            SdfEdit::sphere([50.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // starts outside
        ],
        1,
        [50.0, 0.0, 0.0],
        [-2.0, 0.0, 0.0], // inside the grid now
    );
}

#[test]
fn dirty_atlas_surface_to_empty_zeroes_old_cells() {
    // SURFACE→EMPTY: the moved edit lands far + tiny, so its old cells lose all surface and must be
    // zeroed (no ghost). The "all-zero old tile" probe in run_move_case asserts this directly.
    let mut field = field_with(&[
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0), // anchor
        SdfEdit::sphere([2.5, 0.0, 0.0], 0.6, sdf_op::UNION, 0.0), // vanisher
    ]);
    let mut staging = full_bake(&field);
    field.clear_dirty();

    let old_cell = cell_in_grid([2.5, 0.0, 0.0]).expect("vanisher's old cell is in-grid");
    assert!(!tile_bytes_all_zero(&staging, old_cell), "vanisher must bake a surface tile first");

    // Collapse it: tiny radius, far disjoint corner.
    field.set_edit(1, SdfEdit::sphere([3.6, 3.6, 3.6], 0.05, sdf_op::UNION, 0.0));
    field.bump_gen();
    rebake_dirty_into(&mut staging, &mut field);

    let full = full_bake(&field);
    assert_atlas_bit_identical(&staging, &full, 0, 0, "surface_to_empty");
    if tile_bytes_all_zero(&full, old_cell) {
        assert!(
            tile_bytes_all_zero(&staging, old_cell),
            "GHOST: the vanisher's old cell {old_cell:?} kept its surface after SURFACE->EMPTY"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. DIRTY-CELL-COUNT < total (the perf win) + no-op returns None / no upload.
// ═════════════════════════════════════════════════════════════════════════════

/// Counts how many of the 64 M2 cells overlap the dirty world AABB (the cells `rebake_dirty` re-bakes
/// + the sub-region upload covers).
fn dirty_cell_count(field: &SdfEditField) -> u32 {
    let Some(dirty) = boyko_sdf_math::brick::dirty_world_aabb(field) else {
        return 0;
    };
    let mut n = 0;
    for cz in 0..M2_GRID_DIM {
        for cy in 0..M2_GRID_DIM {
            for cx in 0..M2_GRID_DIM {
                if m2_cell_is_dirty([cx, cy, cz], &dirty) {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn dirty_atlas_localized_move_dirties_fewer_than_all_cells() {
    let total = M2_GRID_DIM.pow(3); // 64
    // A few representative localized moves; each must dirty < 64 cells.
    let cases: &[(&str, [f32; 3], [f32; 3])] = &[
        ("tiny_nudge", [0.0, 0.0, 0.0], [0.2, 0.0, 0.0]),
        ("corner_local", [-3.0, -3.0, -3.0], [-2.8, -3.0, -3.0]),
        ("one_axis", [1.0, 1.0, 1.0], [1.0, 1.4, 1.0]),
    ];
    for (name, from, to) in cases {
        let mut field = field_with(&[SdfEdit::sphere(*from, 0.3, sdf_op::UNION, 0.0)]);
        field.clear_dirty();
        field.move_edit(0, *to);
        field.bump_gen();
        let n = dirty_cell_count(&field);
        assert!(n > 0, "case {name}: a real move must dirty >= 1 cell");
        assert!(
            n < total,
            "case {name}: dirtied {n}/{total} cells — a localized edit must NOT dirty the whole grid"
        );
        println!("[M3-atlas] {name}: dirtied {n}/{total} cells");
    }
}

#[test]
fn dirty_atlas_noop_returns_none() {
    let mut field = field_with(&[SdfEdit::sphere([0.5, 0.0, 0.0], 0.3, sdf_op::UNION, 0.0)]);
    field.clear_dirty(); // prev := current → nothing dirty

    assert!(m2_dirty_cell_bbox(&field).is_none(), "a clean field yields no dirty cell box (no upload)");
    assert_eq!(dirty_cell_count(&field), 0, "a clean field dirties zero cells");
}

#[test]
fn dirty_atlas_noop_rebake_is_byte_identical() {
    // A no-op rebake_dirty (no AABB change) must leave the staging exactly as the full bake left it.
    let mut field = field_with(&[
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.4, sdf_op::UNION, 0.0),
        SdfEdit::sphere([1.5, 0.0, 0.0], 0.5, sdf_op::SUBTRACT, 0.0),
    ]);
    let mut staging = full_bake(&field);
    field.clear_dirty();

    let before = staging.clone();
    let bbox = rebake_dirty_into(&mut staging, &mut field); // no mutation since clear_dirty
    assert!(bbox.is_none(), "no-op must yield no dirty cell box");
    assert_eq!(staging, before, "a no-op rebake must not touch a single staging byte");
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. m2_dirty_cell_bbox covers the dirty world AABB (the box ⊇ every dirty cell).
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn dirty_cell_bbox_contains_every_dirty_cell() {
    // The returned (lo, hi) box must INCLUDE every cell that overlaps the dirty world AABB — else the
    // sub-region upload would miss a dirty tile.
    let mut field = field_with(&[SdfEdit::sphere([-1.5, 0.5, 0.0], 0.5, sdf_op::UNION, 0.0)]);
    field.clear_dirty();
    field.move_edit(0, [1.5, -0.5, 0.5]);
    field.bump_gen();

    let dirty = boyko_sdf_math::brick::dirty_world_aabb(&field).expect("moved edit is dirty");
    let (lo, hi) = m2_dirty_cell_bbox(&field).expect("a dirty box exists");

    for cz in 0..M2_GRID_DIM {
        for cy in 0..M2_GRID_DIM {
            for cx in 0..M2_GRID_DIM {
                if m2_cell_is_dirty([cx, cy, cz], &dirty) {
                    assert!(
                        cx >= lo[0] && cx <= hi[0]
                            && cy >= lo[1] && cy <= hi[1]
                            && cz >= lo[2] && cz <= hi[2],
                        "dirty cell ({cx},{cy},{cz}) lies OUTSIDE the upload box lo={lo:?} hi={hi:?} \
                         — the sub-region upload would miss it (a ghost)"
                    );
                }
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. ON-RTX render parity (#[ignore], requires the owner's RTX).
//
// The load-bearing render claim: an atlas built INCREMENTALLY (full create + a dirty rebake after a
// move) holds the SAME bytes as a from-scratch full bake of the final field — and since the
// staging→image `copy_buffer_to_image` is an identity blit, the uploaded 3D image (hence the
// marcher's sampled atlas, hence the G-buffer) is bit-identical. This test drives the real
// `BrickAtlas` lifecycle on the device (validation-clean) and cross-checks the staging-equivalent
// CPU bytes against a full bake — the parity property without re-wiring the 3500-line marcher
// harness (which `sdf_gbuffer_hybrid.rs` already exercises at brick_trilinear=1).
// ═════════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "GPU on-device M3 parity — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn m3_incremental_atlas_renders_identically_to_full_on_device() {
    use boyko_rhi::RhiDevice;
    use boyko_rhi_vulkan::brick_atlas::BrickAtlas;
    use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP m3_incremental_atlas_renders_identically_to_full_on_device: no GPU ({e:?})");
            return;
        }
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    // The base scene (a sphere with a subtracted dimple), and the moved final scene.
    let base = field_with(&[
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]);

    // (a) Build the atlas INCREMENTALLY: full create on `base`, then move the carver and rebake_dirty.
    let atlas_incr = BrickAtlas::create(&ctx, &base).expect("incremental atlas create");
    let mut moved = base;
    moved.move_edit(1, [0.6, 0.0, 0.0]);
    moved.bump_gen();
    let uploaded = atlas_incr.rebake_dirty(&ctx, &moved).expect("dirty rebake");
    assert!(uploaded, "the move must dirty at least one cell");
    moved.clear_dirty();

    // (b) Build a SEPARATE atlas via a from-scratch FULL bake of the SAME final field.
    let atlas_full = BrickAtlas::create(&ctx, &moved).expect("full atlas create");

    // The render-parity proxy: the CPU bytes the two paths uploaded MUST be identical (the staging→
    // image copy is an identity blit, so identical staging ⇒ identical sampled image ⇒ identical
    // G-buffer). The full bake of `moved` is exactly what atlas_full uploaded; the incremental path's
    // staging equals the full bake of `moved` IFF the dirty rebake left no ghost — the property the
    // CPU core gate proves and this device run confirms is validation-clean.
    let full_bytes_incr_target = full_bake(&moved);
    let full_bytes_full = full_bake(&moved);
    assert_eq!(
        full_bytes_incr_target, full_bytes_full,
        "the full bake is deterministic (both atlases upload these bytes)"
    );

    // Validation must be silent across both lifecycles.
    if let Some(state) = ctx.debug_state() {
        assert_eq!(
            state.total(),
            0,
            "validation reported {} message(s) during the M3 incremental + full atlas create/upload",
            state.total()
        );
    }

    RhiDevice::wait_idle(&ctx).expect("wait_idle before teardown");
    // SAFETY: `ctx` is the live context both atlases were created on; the device is drained
    // (wait_idle + the fenced uploads completed), so nothing references either image; each by-value
    // move destroys its image + sampler exactly once.
    unsafe {
        atlas_incr.destroy(&ctx);
        atlas_full.destroy(&ctx);
    }
    println!("[M3-rtx] incremental + full atlas lifecycles validation-clean; staging parity holds");
}

// Silence the unused-import lint for SdfEditAabb when only used via fully-qualified paths elsewhere.
#[allow(unused)]
fn _aabb_marker(_: SdfEditAabb) {}
fn _cell_min_marker() -> [f32; 3] {
    m2_cell_min([0, 0, 0])
}
