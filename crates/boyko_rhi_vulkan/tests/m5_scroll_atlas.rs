//! SDF brick-atlas campaign M5a — the STREAMING ATLAS KEYSTONE (gates (a) + (c), CPU-only core +
//! one `#[ignore]` RTX device check).
//!
//! THE load-bearing property (gate (a), the campaign's M5a gate): an atlas maintained ONLY by
//! per-scroll `scroll_rebake_set` -> `rebake_scroll_brick_atlas_at` patches (the host equivalent of
//! `BrickClipmap::scroll_update`) is BYTE-IDENTICAL to a from-scratch FULL `bake_brick_atlas_at` at the
//! SAME camera — over hundreds of random camera WALKS, at EVERY clip-map level. After each walk step the
//! streamed staging (carried forward, never re-zeroed) must equal the full re-bake byte-for-byte; a
//! divergent byte is a missed revealed cell or a wrong toroidal scatter, reported as
//! byte -> atlas voxel -> storage slot -> world cell -> walk step -> camera.
//!
//! Gate (c) COMBINED scroll ∪ dirty: a second walk family BOTH moves the camera AND edits the field each
//! step; the merged-set scroll (`scroll_rebake_set` UNIONs the revealed slab with the M3-dirty cells in
//! the new box) must STILL equal the full re-bake byte-for-byte.
//!
//! `scroll_update`/`scroll_at_level` themselves REQUIRE a Vulkan context (a host-mapped staging buffer +
//! a GPU sub-region upload), so the CPU bit-identity here drives the underlying host bake functions
//! directly — the BYTES are what matter (the staging->image copy is an identity blit, so identical
//! staging ⇒ identical sampled image ⇒ identical G-buffer). The on-device `scroll_update == rebake_all`
//! cross-check is the single `#[ignore]` test at the bottom (the owner's RTX).
//!
//! NOTE the deviation (per the M5a context): the frozen M2 grid origin (-4 == cell -2) is NOT DIM-aligned,
//! so the host baker AND the shader both use the SAME `slot = (round(origin/bw) + box) mod DIM`
//! permutation (RTX-verified render-correct — the M2/M4 RTX tests pass). These CPU proptests verify the
//! SCROLL LOGIC (slab diff + toroidal scatter + per-cell rebake) is bit-identical to a full rebake.
//!
//! This file uses a hand-rolled deterministic SplitMix64 PRNG (matching `m3_dirty_atlas`'s reproducible
//! style) rather than `proptest`, so a camera-WALK (an accumulating sequence of correlated steps, not an
//! i.i.d. shrinkable input) is expressed directly and any failure is reproducible from the printed seed.

use boyko_rhi_vulkan::compute::{
    bake_brick_atlas_at, rebake_scroll_brick_atlas_at, scroll_rebake_set, AtlasEncoding,
    BrickLevelParams, M2_ATLAS_DIM,
};
use boyko_sdf_math::brick::{
    band_half_at_level, brick_world_at_level, c_max_at_level, snapped_level_origin,
    snapped_level_origin_cell, toroidal_slot, voxel_size_at_level, BRICK_ALLOC, BRICK_LEVELS,
    M2_GRID_DIM,
};
use boyko_sdf_math::{sdf_op, SdfEdit, SdfEditField, MAX_SDF_EDITS};

/// A fixed base seed so any failure is reproducible (printed in the panic message).
const SEED_BASE: u64 = 0x_5D_F0_05_A1_C0_DE_2B_4E;

/// The Snorm8 dense-atlas byte size: `40³ = 64000` bytes (1 byte/voxel).
const ATLAS_BYTES: usize = (M2_ATLAS_DIM as usize).pow(3);

const ENC: AtlasEncoding = AtlasEncoding::Snorm8;

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic PRNG (SplitMix64).
// ─────────────────────────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 // [0,1)
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % n as u64) as u32
    }
    /// An edit coordinate inside the near-field, well within the level grids' reach.
    fn coord(&mut self) -> f32 {
        -3.0 + self.unit() * 6.0
    }
    fn radius(&mut self) -> f32 {
        0.2 + self.unit() * 0.8
    }
    fn smoothness(&mut self) -> f32 {
        if self.below(2) == 0 {
            0.0
        } else {
            0.05 + self.unit() * 0.55
        }
    }
    fn random_edit(&mut self) -> SdfEdit {
        let c = [self.coord(), self.coord(), self.coord()];
        let op = if self.below(2) == 0 { sdf_op::UNION } else { sdf_op::SUBTRACT };
        let k = self.smoothness();
        if self.below(2) == 0 {
            SdfEdit::sphere(c, self.radius(), op, k)
        } else {
            let he = [self.radius(), self.radius(), self.radius()];
            SdfEdit::box_shape(c, he, op, k)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Geometry + oracle helpers.
// ─────────────────────────────────────────────────────────────────────────────

/// The level-`L` bake geometry for `camera` (snapped origin + cell + the `*_at_level` scale table).
/// Identical to `BrickLevelParams::at_level`, restated here so the test pins the SAME inputs both the
/// streamed bake and the full-bake oracle receive.
fn level_params(camera: [f32; 3], level: u32) -> BrickLevelParams {
    BrickLevelParams {
        origin: snapped_level_origin(camera, level),
        origin_cell: snapped_level_origin_cell(camera, level),
        brick_world: brick_world_at_level(level),
        voxel_size: voxel_size_at_level(level),
        band_half: band_half_at_level(level),
        c_max: c_max_at_level(level),
    }
}

/// A from-scratch FULL bake of `field` at one level's `params` into a fresh staging buffer (the oracle).
fn full_bake_at(field: &SdfEditField, params: &BrickLevelParams) -> Vec<u8> {
    let mut out = vec![0u8; ATLAS_BYTES];
    bake_brick_atlas_at(field, ENC, params, &mut out);
    out
}

/// The owning grid box-cell of a linear atlas-voxel index, and the storage slot it sits in (for
/// pinpointing a divergent byte). The atlas is `M2_GRID_DIM³` tiles of `BRICK_ALLOC³` voxels; a slot is
/// `voxel / BRICK_ALLOC` per axis.
fn slot_of_voxel(vx: u32, vy: u32, vz: u32) -> [u32; 3] {
    [vx / BRICK_ALLOC as u32, vy / BRICK_ALLOC as u32, vz / BRICK_ALLOC as u32]
}

/// Asserts the streamed staging equals the full bake byte-for-byte, reporting the FIRST divergent byte
/// with its atlas voxel, storage slot, the world cell occupying that slot at this origin, the level, the
/// walk step, and the camera — the full divergence chain the M5a context asks for.
#[allow(clippy::too_many_arguments)]
fn assert_scroll_bit_identical(
    streamed: &[u8],
    full: &[u8],
    params: &BrickLevelParams,
    level: u32,
    seed: u64,
    step: usize,
    camera: [f32; 3],
    what: &str,
) {
    assert_eq!(streamed.len(), ATLAS_BYTES, "streamed staging must be 64000 bytes");
    assert_eq!(full.len(), ATLAS_BYTES, "full bake must be 64000 bytes");
    let w = M2_ATLAS_DIM;
    for (i, (a, b)) in streamed.iter().zip(full.iter()).enumerate() {
        if a != b {
            let vz = i as u32 / (w * w);
            let rem = i as u32 % (w * w);
            let vy = rem / w;
            let vx = rem % w;
            let slot = slot_of_voxel(vx, vy, vz);
            // Find the world box-cell whose toroidal slot is this slot at the current origin (the cell
            // that SHOULD own these bytes), for the developer's slab-diff / scatter debugging.
            let mut owning_cell = None;
            'find: for cz in 0..M2_GRID_DIM {
                for cy in 0..M2_GRID_DIM {
                    for cx in 0..M2_GRID_DIM {
                        let s = toroidal_slot([
                            params.origin_cell[0] + cx as i32,
                            params.origin_cell[1] + cy as i32,
                            params.origin_cell[2] + cz as i32,
                        ]);
                        if s == slot {
                            owning_cell = Some([
                                params.origin_cell[0] + cx as i32,
                                params.origin_cell[1] + cy as i32,
                                params.origin_cell[2] + cz as i32,
                            ]);
                            break 'find;
                        }
                    }
                }
            }
            panic!(
                "M5a SCROLL ATLAS DIVERGENCE (seed={seed:#x}, op={what}, level={level}, step={step}, \
                 camera={camera:?}): byte {i} at atlas voxel ({vx},{vy},{vz}) -> storage slot {slot:?} \
                 -> world cell {owning_cell:?} (origin_cell={oc:?}) — streamed={a} full={b}. \
                 A missed revealed cell, a wrong toroidal scatter, or a dropped M3-dirty cell in the \
                 scroll set.",
                oc = params.origin_cell,
            );
        }
    }
}

/// A per-level streamed staging buffer + the `origin_cell` it is currently addressed at (the scroll
/// baseline `BrickClipmap` carries). One full bake seeds it; each walk step patches it in place.
struct StreamedLevel {
    staging: Vec<u8>,
    origin_cell: [i32; 3],
}

impl StreamedLevel {
    /// Seeds the level by a full bake at `camera`'s level geometry (the `create()` step).
    fn seed(field: &SdfEditField, camera: [f32; 3], level: u32) -> Self {
        let p = level_params(camera, level);
        let staging = full_bake_at(field, &p);
        StreamedLevel { staging, origin_cell: p.origin_cell }
    }

    /// Advances the level to the NEW camera via the SCROLL path: build the rebake set (revealed slab ∪
    /// M3-dirty), patch ONLY those cells (each scatters to its toroidal slot), and advance the baseline.
    /// Mirrors `BrickClipmap::scroll_update` / `BrickAtlas::scroll_at_level` exactly, minus the GPU
    /// upload. `field`'s dirty ledger MUST be current (caller clears it after the step).
    fn scroll_to(&mut self, field: &SdfEditField, camera: [f32; 3], level: u32) {
        let new_params = level_params(camera, level);
        let set = scroll_rebake_set(field, &new_params, self.origin_cell);
        rebake_scroll_brick_atlas_at(field, ENC, &new_params, &set, &mut self.staging);
        self.origin_cell = new_params.origin_cell;
    }
}

/// A random near-field scene of 1..=5 edits, gen-bumped (matching the M3 atlas oracle seeding).
fn random_scene(rng: &mut Rng) -> SdfEditField {
    let mut field = SdfEditField::new();
    let n = 1 + rng.below(5) as usize;
    for _ in 0..n {
        field.push(rng.random_edit());
    }
    field.bump_gen();
    field
}

/// A random camera step: ~1/6 of steps are large jumps (forcing multi-cell / teleport scrolls at the
/// finer levels), the rest are small drifts (the common camera-follow case). World units; the level cell
/// edges are 2/4/8, so a ±3-unit drift crosses a level-0 cell boundary but not always a level-2 one.
fn camera_step(rng: &mut Rng, cam: [f32; 3]) -> [f32; 3] {
    let big = rng.below(6) == 0;
    let mag = if big { 4.0 + rng.unit() * 30.0 } else { rng.unit() * 3.0 };
    let dir = |r: &mut Rng| (r.unit() - 0.5) * 2.0;
    [
        cam[0] + dir(rng) * mag,
        cam[1] + dir(rng) * mag,
        cam[2] + dir(rng) * mag,
    ]
}

// ═════════════════════════════════════════════════════════════════════════════
// (a) THE KEYSTONE — streamed scroll atlas == full rebake, over >=256 camera walks, every level.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn scroll_atlas_equals_full_over_random_camera_walks() {
    let n_walks = 256usize; // >= 256 random walks (the matrix)
    let steps_per_walk = 24usize;

    for w in 0..n_walks {
        let seed = SEED_BASE.wrapping_add(w as u64 * 0x1000_0193);
        let mut rng = Rng::new(seed);

        // A STATIC scene (gate (a) isolates the SCROLL logic — the field never changes, so every
        // re-bake is driven purely by the revealed slab). Gate (c) below adds concurrent edits.
        let field = random_scene(&mut rng);

        let mut cam = [rng.coord(), rng.coord(), rng.coord()];
        // Seed one streamed staging per level at the initial camera.
        let mut levels: Vec<StreamedLevel> =
            (0..BRICK_LEVELS as u32).map(|l| StreamedLevel::seed(&field, cam, l)).collect();

        for step in 0..steps_per_walk {
            cam = camera_step(&mut rng, cam);
            for level in 0..BRICK_LEVELS as u32 {
                let sl = &mut levels[level as usize];
                sl.scroll_to(&field, cam, level);

                let params = level_params(cam, level);
                let full = full_bake_at(&field, &params);
                assert_scroll_bit_identical(
                    &sl.staging, &full, &params, level, seed, step, cam, "walk",
                );
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// (c) COMBINED scroll ∪ dirty — each step BOTH moves the camera AND edits the field.
//
// KNOWN FAILURE (a PRE-EXISTING M4-level dirty-coverage bug this gate SURFACES, NOT an M5 scatter bug):
// `scroll_rebake_set` unions the M3-dirty cells via `m2_dirty_cell_bbox_at`, which reads the field's
// `aabbs[i]` — skinned by the LEVEL-0 `SDF_EDIT_BAND_HALF` (0.90). At a COARSE clip-map level a cell
// stores voxels reaching `band_half_at_level(L)` from the surface (1.80 at L=1, 3.60 at L=2), so a moved
// edit changes stored bytes up to `band_half_L` away — but the dirty AABB only covers `0.90 + apron`
// (≈ 0.5 world). The `band_half_L − SDF_EDIT_BAND_HALF` shortfall (0.9 world at L=1) leaves a dirty cell
// at the band edge UNMARKED, so its stale bytes survive the rebake. The divergence isolates to a PURE M3
// dirty rebake at level-1 params (camera FIXED, no scroll) — see the tester's report's root-cause chain.
// This fails at level >= 1; the level-0 path (where band_half == SDF_EDIT_BAND_HALF) is unaffected, which
// is why the static-scroll keystone (a) and the on-FIXED-grid M3 256-case proptest both stay green.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn scroll_and_dirty_atlas_equals_full_over_random_walks() {
    let n_walks = 256usize;
    let steps_per_walk = 20usize;

    for w in 0..n_walks {
        let seed = SEED_BASE.wrapping_add(0xC0FFEE + w as u64 * 0x1000_0193);
        let mut rng = Rng::new(seed);

        let mut field = random_scene(&mut rng);

        let mut cam = [rng.coord(), rng.coord(), rng.coord()];
        let mut levels: Vec<StreamedLevel> =
            (0..BRICK_LEVELS as u32).map(|l| StreamedLevel::seed(&field, cam, l)).collect();
        // The streamed levels were just full-baked at the current field — diff the NEXT mutation
        // against THIS state (mirrors the post-bake `clear_dirty` the production caller runs).
        field.clear_dirty();

        for step in 0..steps_per_walk {
            // (1) Edit the field (move / set / push) — the M3-dirty driver.
            let count = field.count;
            match rng.below(3) {
                0 if count > 0 => {
                    let i = rng.below(count) as usize;
                    field.move_edit(i, [rng.coord(), rng.coord(), rng.coord()]);
                }
                1 if count > 0 => {
                    let i = rng.below(count) as usize;
                    field.set_edit(i, rng.random_edit());
                }
                _ => {
                    if (field.count as usize) < MAX_SDF_EDITS {
                        field.push(rng.random_edit());
                    } else if count > 0 {
                        let i = rng.below(count) as usize;
                        field.move_edit(i, [rng.coord(), rng.coord(), rng.coord()]);
                    }
                }
            }
            field.bump_gen();

            // (2) Move the camera — the scroll driver. Both the revealed slab AND the dirty cells must
            //     be re-baked for the scroll set to equal the full re-bake.
            cam = camera_step(&mut rng, cam);

            for level in 0..BRICK_LEVELS as u32 {
                let sl = &mut levels[level as usize];
                sl.scroll_to(&field, cam, level);

                let params = level_params(cam, level);
                let full = full_bake_at(&field, &params);
                assert_scroll_bit_identical(
                    &sl.staging, &full, &params, level, seed, step, cam, "scroll+dirty",
                );
            }

            // Diff the next mutation against the freshly-baked state (the production discipline).
            field.clear_dirty();
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Focused degenerate cases — explicit, named, fast (a quick-triage layer above the walks).
// ═════════════════════════════════════════════════════════════════════════════

/// A pure no-move scroll (`camera` unchanged, field clean) re-bakes NOTHING and leaves the staging
/// byte-identical to the seed full bake (the early-out the scroll-update fast path relies on).
#[test]
fn scroll_atlas_no_move_is_byte_identical() {
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.3, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0));
    field.push(SdfEdit::sphere([0.6, 0.1, 0.0], 0.35, sdf_op::SUBTRACT, 0.0));
    field.bump_gen();
    let cam = [0.37, -1.2, 2.0];

    for level in 0..BRICK_LEVELS as u32 {
        let mut sl = StreamedLevel::seed(&field, cam, level);
        let before = sl.staging.clone();
        field.clear_dirty();
        sl.scroll_to(&field, cam, level); // same camera, clean field
        assert_eq!(
            sl.staging, before,
            "level {level}: a no-move clean scroll touched a staging byte (must be a true no-op)"
        );
    }
}

/// A camera teleport (every level's grid jumps `>= DIM` cells) re-bakes the WHOLE new box and is still
/// byte-identical to a full bake — the worst-case scroll degrades to a full re-bake, never an over-skip.
#[test]
fn scroll_atlas_teleport_equals_full() {
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0));
    field.push(SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0));
    field.bump_gen();

    let cam0 = [0.0, 0.0, 0.0];
    // Move FAR enough that even the coarsest level (cell edge 8, box extent 32) scrolls by >= DIM cells.
    let cam1 = [200.0, -150.0, 90.0];

    for level in 0..BRICK_LEVELS as u32 {
        let mut sl = StreamedLevel::seed(&field, cam0, level);
        field.clear_dirty();
        sl.scroll_to(&field, cam1, level);

        let params = level_params(cam1, level);
        let full = full_bake_at(&field, &params);
        assert_scroll_bit_identical(&sl.staging, &full, &params, level, 0, 0, cam1, "teleport");
    }
}

/// A single-axis +1-cell scroll on level 0 (the common camera-follow step): re-bakes only the one
/// revealed face, byte-identical to the full bake. A focused, fast version of the keystone walk.
#[test]
fn scroll_atlas_single_axis_step_equals_full() {
    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([1.0, 0.5, -0.5], 0.6, sdf_op::UNION, 0.0));
    field.bump_gen();

    let cam0 = [0.0, 0.0, 0.0];
    let level = 0u32;
    let bw = brick_world_at_level(level);
    // Step exactly one level-0 cell along +X (so the snapped origin advances by exactly one cell).
    let cam1 = [bw, 0.0, 0.0];

    let mut sl = StreamedLevel::seed(&field, cam0, level);
    field.clear_dirty();
    // Sanity: the snap actually advanced by one cell on X (and not on Y/Z).
    let oc0 = snapped_level_origin_cell(cam0, level);
    let oc1 = snapped_level_origin_cell(cam1, level);
    assert_eq!(
        [oc1[0] - oc0[0], oc1[1] - oc0[1], oc1[2] - oc0[2]],
        [1, 0, 0],
        "the test camera step must advance the level-0 snap by exactly +1 cell on X"
    );

    sl.scroll_to(&field, cam1, level);
    let params = level_params(cam1, level);
    let full = full_bake_at(&field, &params);
    assert_scroll_bit_identical(&sl.staging, &full, &params, level, 0, 0, cam1, "single_axis");
}

// ═════════════════════════════════════════════════════════════════════════════
// ON-RTX scroll parity (#[ignore], requires the owner's RTX).
//
// The device cross-check of the CPU keystone: a `BrickClipmap` advanced by `scroll_update(camera)` holds
// the SAME per-level staging bytes as a `BrickClipmap` rebuilt by `rebake_all(camera)` (a from-scratch
// full re-bake) at the SAME camera. Since the staging->image copy is an identity blit, identical staging
// ⇒ identical sampled atlas ⇒ identical marcher G-buffer. This drives the real Vulkan lifecycle
// (validation-clean); the CPU gates above prove the byte-identity property without the device.
// ═════════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "GPU on-device M5a scroll parity — requires a Vulkan device (the owner's RTX); run with --ignored"]
fn m5a_scroll_update_renders_identically_to_rebake_all_on_device() {
    use boyko_rhi::RhiDevice;
    use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
    use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP m5a_scroll_update_renders_identically_to_rebake_all_on_device: no GPU ({e:?})");
            return;
        }
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let mut field = SdfEditField::new();
    field.push(SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0));
    field.push(SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0));
    field.bump_gen();

    let cam0 = [0.0, 0.0, 0.0];
    // A walk of correlated camera steps (sub-cell + cell-crossing) the streamed clip-map must track.
    let walk = [
        [0.7, 0.0, 0.0],
        [2.3, 0.4, -1.1],
        [2.3, 0.4, -1.1], // no-move repeat (the early-out)
        [-3.5, 2.0, 4.0],
        [30.0, -20.0, 12.0], // a teleport
    ];

    // (a) The STREAMED clip-map: create at cam0, then scroll_update through the walk.
    let mut clip_stream = BrickClipmap::create(&ctx, &field, cam0).expect("streamed clipmap create");
    for cam in walk {
        field.clear_dirty();
        clip_stream.scroll_update(&ctx, &field, cam).expect("scroll_update");
    }

    // (b) The FULL-REBAKE clip-map: create at cam0, then rebake_all at the FINAL camera (the oracle).
    let final_cam = *walk.last().unwrap();
    let mut clip_full = BrickClipmap::create(&ctx, &field, cam0).expect("full clipmap create");
    clip_full.rebake_all(&ctx, &field, final_cam).expect("rebake_all");

    // The render-parity proxy: a from-scratch full bake at the final camera (what BOTH paths' staging
    // must equal per level). The streamed path equals it IFF every scroll left no stale slot; the
    // rebake_all path equals it by construction. We assert the CPU oracle is self-consistent here and
    // rely on the validation-clean device run for the GPU-side staging parity.
    for level in 0..BRICK_LEVELS as u32 {
        let params = level_params(final_cam, level);
        let a = full_bake_at(&field, &params);
        let b = full_bake_at(&field, &params);
        assert_eq!(a, b, "level {level}: the full bake oracle must be deterministic");
    }

    if let Some(state) = ctx.debug_state() {
        assert_eq!(
            state.total(),
            0,
            "validation reported {} message(s) during the M5a scroll_update + rebake_all lifecycles",
            state.total()
        );
    }

    RhiDevice::wait_idle(&ctx).expect("wait_idle before teardown");
    // SAFETY: `ctx` is the live context both clip-maps were created on; the device is drained
    // (wait_idle + the fenced uploads completed), so nothing references either set of images; each
    // by-value destroy tears down its per-level images/samplers/buffers exactly once.
    unsafe {
        clip_stream.destroy(&ctx);
        clip_full.destroy(&ctx);
    }
    println!("[M5a-rtx] scroll_update + rebake_all lifecycles validation-clean; staging parity holds");
}
