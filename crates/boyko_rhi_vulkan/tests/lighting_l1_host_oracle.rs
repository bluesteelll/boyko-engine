//! Lighting L1 host-oracle (CPU-only — NO GPU required).
//!
//! Proves the L1 clustered froxel-cull RESOLVE path the GPU golden compares against:
//! - the cluster linearization `(x,y,z) <-> index` round-trips and agrees host/shader
//!   (`golden_cluster_index`, the ONE linearization both cull-write + resolve-read use);
//! - the exp-Z slice math (`golden_cluster_z_slice`) inverts the slice distribution
//!   `view_z(k) = near*(far/near)^(k/dim_z)`;
//! - the host sphere-vs-AABB cull (`golden_cluster_cull`) keeps every light whose sphere
//!   intersects a froxel's world AABB and drops the rest (no false drop under the cap);
//! - **the load-bearing L1 golden**: the clustered resolve
//!   (`golden_deferred_resolve_clustered`) produces the SAME packed color as the
//!   brute-force `golden_deferred_resolve_table` for a multi-light scene (the cull is exact
//!   for the test lights), and the L1-OFF header path is byte-identical to L0b.
//!
//! It also carries the provenance of `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.3's published
//! occupancy table (VB-P1e rung HP): `cluster_cull_occupancy_profile_matches_the_published_table`
//! and `cluster_cull_rejection_ratio_at_n512_matches_the_headline_claim` drive the same
//! `golden_cluster_cull` oracle above with the VB-P1d bench camera / light rig
//! (`crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`) to pin the measured froxel occupancy
//! the hierarchical-cull design (§2 onward) is built on, replacing a session-ephemeral scratch
//! probe that was never committed to the repository.
//!
//! This file boots NO Vulkan context — it is the non-GPU gate the developer runs (the GPU
//! golden runs separately on the 3060).

use boyko_rhi_vulkan::compute::{composite_pixel_ray, CompositeCamera, SDF_IMG_H, SDF_IMG_W};
use boyko_rhi_vulkan::goldens::{golden_cluster_cull, golden_cluster_index, golden_cluster_xy_tile, golden_cluster_z_slice, golden_deferred_resolve_clustered, golden_deferred_resolve_table, GoldenClusterConfig, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes};

/// The ortho ray-gen the resolve uses: `ro=(0,0,2)`, `rd=(0,0,-1)`, so `view_z == view_t`
/// and `P = ro + rd * view_t = (u*HE, v*HE, 2 - view_t)`.
const RO: [f32; 3] = [0.0, 0.0, 2.0];
const RD: [f32; 3] = [0.0, 0.0, -1.0];

/// A cluster config whose exp-Z near/far span the ortho scene's view-z (= ray param `t`):
/// surfaces sit near world z = 0 (camera at z = 2), so `t ≈ 2`. near=0.25, far=4.0 keeps the
/// froxel slices concentrated over the scene's depth band.
fn cfg() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.25,
        z_far: 4.0,
    }
}

fn materials() -> Vec<GoldenMaterial> {
    vec![GoldenMaterial::default()]
}

/// A lit (mask == 1) attribute with a chosen `view_t` and an oct-encoded ~+Z normal.
fn lit_attrs(view_t: f32) -> MarcherAttributes {
    MarcherAttributes {
        base_rgb: [200, 200, 200],
        oct_rg: [128, 128], // oct (0.5, 0.5) -> +Z normal
        mat_id: 0,
        shadow: 255,
        ao: 255,
        mask: 1,
        view_t,
    }
}

/// The bench's square render target (`vb_p1d_cull_shade_bench.rs`'s own `CameraRig`, aspect 1.0).
const IMG: u32 = 512;

/// `boyko_render::light::INDEX_LIST_CAP`. The vulkan crate cannot depend on `boyko_render`
/// (see [`GoldenClusterConfig`]'s own doc comment), so this mirrors the constant rather than
/// importing it.
const INDEX_LIST_CAP: u32 = 16384;

/// The bench rig's own `DEFAULT_N_PS` (`vb_p1d_cull_shade_bench.rs:68`) — [`light_position`]'s
/// volume-scale factor is relative to this fixed baseline, not to the swept `n_ps`.
const BENCH_DEFAULT_N_PS: u32 = 14;

/// The published §1.3 table, pinned exactly: `(n_ps, total_indices, non_empty_froxels,
/// max_per_froxel)`. These are MEASURED values, not derived — a mismatch means the host oracle
/// or the bench rig changed since the table was published, which is exactly what this test
/// exists to catch. Do not edit these literals to make a failing run pass.
const PUBLISHED_TABLE: [(u32, usize, usize, usize); 8] = [
    (8, 789, 514, 3),
    (14, 1239, 543, 5),
    (32, 1916, 557, 10),
    (64, 2063, 364, 15),
    (128, 1654, 143, 24),
    (256, 2072, 115, 40),
    (512, 2597, 85, 64),
    (1024, 2709, 55, 109),
];

/// `ClusterConfig::default()` mirrored as a [`GoldenClusterConfig`]: 16x9x24 = 3456 froxels,
/// `MAX_LIGHTS_PER_CLUSTER` 256, `z_near` 0.1, `z_far` 50.0. Distinct from this file's own
/// [`cfg`] (ortho-tuned, `z_near` 0.25 / `z_far` 4.0): this fixture is the VB-P1d bench's
/// PERSPECTIVE scene and must not be merged with the resolve fixtures above.
fn vb_p1d_bench_cluster_cfg() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.1,
        z_far: 50.0,
    }
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The VB-P1d bench camera (`vb_p1d_cull_shade_bench.rs:235-254`): eye `(0, 1.1, 7.8)` looking
/// at `(0, 0.55, 0)`, `fov_y` 52 degrees, aspect 1.0.
fn camera() -> CompositeCamera {
    let eye = [0.0, 1.1, 7.8];
    let fwd = norm([0.0 - eye[0], 0.55 - eye[1], 0.0 - eye[2]]);
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    CompositeCamera::Perspective {
        eye,
        forward: fwd,
        right,
        up,
        tan_half_fov: (52.0_f32 * core::f32::consts::PI / 180.0 / 2.0).tan(),
        aspect: 1.0,
    }
}

/// Verbatim mirror of `vb_p1d_cull_shade_bench.rs::light_position` (`:124-137`).
fn light_position(i: u32, n: u32) -> [f32; 3] {
    let scale = (f64::from(n) / f64::from(BENCH_DEFAULT_N_PS)).max(1.0).cbrt() as f32;
    let half_x = 4.5 * scale;
    let y_min = 0.3;
    let y_span = 3.3 * scale;
    let z_min = -2.0 * scale;
    let z_span = 6.0 * scale;

    let t = f64::from(i);
    let fx = (t * 0.618_033_988_75).fract() as f32;
    let fy = (t * 0.381_966_011_25).fract() as f32;
    let fz = (t * 0.236_067_977_5).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

/// Verbatim mirror of `vb_p1d_cull_shade_bench.rs::light_range` (`:142-144`).
fn light_range(i: u32) -> f32 {
    1.2 + ((f64::from(i) * 0.142_857).fract() as f32) * 0.8
}

/// Builds the bench's light table for `n_ps` point/spot lights: 2 global `l0a` lights (a
/// directional + a sky, matching `setup`'s own sun + sky count — [`golden_cluster_cull`] never
/// inspects `l0a`-indexed lights, so only their COUNT matters here, not their values), followed
/// by `n_ps` point/spot lights placed by [`light_position`]/[`light_range`], every 4th
/// (`i % 4 == 3`) a spot aimed straight down, the rest points.
fn lights_for(n_ps: u32) -> Vec<GoldenLight> {
    let mut lights = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    for i in 0..n_ps {
        let p = light_position(i, n_ps);
        let r = light_range(i);
        if i % 4 == 3 {
            lights.push(GoldenLight::spot(p, [0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 65.0, r, 15.0, 30.0));
        } else {
            lights.push(GoldenLight::point(p, [1.0, 1.0, 1.0], 65.0, r));
        }
    }
    lights
}

/// Runs the host cull oracle for `n_ps` point/spot lights at the VB-P1d bench rig / fixture
/// above, returning `(total_indices, non_empty_froxels, max_per_froxel)`.
fn occupancy_at(n_ps: u32) -> (usize, usize, usize) {
    let c = vb_p1d_bench_cluster_cfg();
    let cam = camera();
    let lights = lights_for(n_ps);
    let header = GoldenLightHeader::new(2, n_ps, 1.0);
    let grid = golden_cluster_cull(IMG, IMG, cam, &c, &header, &lights);
    let total: usize = grid.iter().map(Vec::len).sum();
    let non_empty = grid.iter().filter(|cell| !cell.is_empty()).count();
    let max_per_froxel = grid.iter().map(Vec::len).max().unwrap_or(0);
    (total, non_empty, max_per_froxel)
}

#[test]
fn cluster_index_is_a_bijection_with_z_innermost() {
    let c = cfg();
    // Z innermost: incrementing z by 1 increments the index by 1.
    assert_eq!(golden_cluster_index(0, 0, 0, c.dim_x, c.dim_z), 0);
    assert_eq!(golden_cluster_index(0, 0, 1, c.dim_x, c.dim_z), 1);
    assert_eq!(golden_cluster_index(1, 0, 0, c.dim_x, c.dim_z), c.dim_z);
    assert_eq!(golden_cluster_index(0, 1, 0, c.dim_x, c.dim_z), c.dim_x * c.dim_z);
    assert_eq!(
        golden_cluster_index(c.dim_x - 1, c.dim_y - 1, c.dim_z - 1, c.dim_x, c.dim_z),
        c.cluster_count() - 1
    );
    // Bijection: every froxel maps to a distinct index in [0, COUNT).
    let mut seen = vec![false; c.cluster_count() as usize];
    for y in 0..c.dim_y {
        for x in 0..c.dim_x {
            for z in 0..c.dim_z {
                let idx = golden_cluster_index(x, y, z, c.dim_x, c.dim_z) as usize;
                assert!(!seen[idx], "linearization collision at ({x},{y},{z})");
                seen[idx] = true;
            }
        }
    }
    assert!(seen.iter().all(|&s| s));
}

#[test]
fn exp_z_slice_inverts_the_distribution() {
    let c = cfg();
    let scale = c.z_scale();
    let bias = c.z_bias();
    // The boundary view-z at slice k maps back to slice k (round-trip).
    for k in 0..c.dim_z {
        let view_z = c.z_near * (c.z_far / c.z_near).powf(k as f32 / c.dim_z as f32);
        // Use the midpoint of slice [k, k+1) so the floor lands on k unambiguously.
        let view_z_mid = c.z_near * (c.z_far / c.z_near).powf((k as f32 + 0.5) / c.dim_z as f32);
        assert_eq!(golden_cluster_z_slice(view_z_mid, &c), k, "slice {k} midpoint");
        let _ = (view_z, scale, bias);
    }
    // Below near clamps to slice 0; above far clamps to the last slice.
    assert_eq!(golden_cluster_z_slice(c.z_near * 0.5, &c), 0);
    assert_eq!(golden_cluster_z_slice(c.z_far * 2.0, &c), c.dim_z - 1);
    // A non-positive (sentinel/behind) view-z clamps to slice 0.
    assert_eq!(golden_cluster_z_slice(0.0, &c), 0);
    assert_eq!(golden_cluster_z_slice(-1.0, &c), 0);
}

#[test]
fn cull_keeps_an_in_range_light_and_drops_an_out_of_range_one() {
    let c = cfg();
    // One point light at world (0,0,0) (center of the ortho view), range 3.0 — its sphere
    // intersects froxels along the central column. A second far-away point at (100,100,0)
    // range 0.1 — its sphere intersects NO froxel.
    let header = GoldenLightHeader::new_clustered(0, 2, 1.0, &c);
    let lights = vec![
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0),
        GoldenLight::point([100.0, 100.0, 0.0], [1.0, 1.0, 1.0], 100.0, 0.1),
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights);
    assert_eq!(grid.len(), c.cluster_count() as usize);

    // Light 0 (in range) must appear in at least one froxel; light 1 (far) in none.
    let mut light0_seen = 0u32;
    let mut light1_seen = 0u32;
    for cell in &grid {
        for &i in cell {
            if i == 0 {
                light0_seen += 1;
            }
            if i == 1 {
                light1_seen += 1;
            }
        }
    }
    assert!(light0_seen > 0, "an in-range point light must land in at least one froxel");
    assert_eq!(light1_seen, 0, "an out-of-range point light must be in NO froxel");
}

#[test]
fn cull_directional_and_sky_are_global_never_in_a_froxel() {
    // The no-`P` front block (directionals + sky) is GLOBAL: the cull never appends those
    // indices to any froxel (the resolve always loops them outside the cluster path).
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(2, 1, 1.0, &c);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0), // index 0 (l0a)
        GoldenLight::sky([0.1, 0.1, 0.12], [0.1, 0.1, 0.12]),            // index 1 (l0a)
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0), // index 2 (point)
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights);
    for cell in &grid {
        for &i in cell {
            assert!(i >= 2, "a froxel must only carry point/spot indices (>= l0a_count), got {i}");
        }
    }
}

#[test]
fn cull_keeps_a_shadow_flagged_and_an_atlas_slotted_punctual() {
    // HOST-ORACLE mask invariant (CPU-only — does NOT exercise the GPU `cluster_cull.hlsl`).
    // `golden_cluster_cull` culls by `GoldenLight::kind()`, which masks off bit 16
    // (`casts_sdf_shadow`) and bits 17..21 (the atlas slot) before the POINT/SPOT comparison —
    // this pins that a shadow-flagged point and an atlas-slotted spot are therefore treated as
    // their BASE kind and survive the cull, landing in the froxel whose world AABB contains
    // them, EXACTLY the masking VB-P1-0 added to `cluster_cull.hlsl` (`light_kind()`, mirrored
    // 1:1 here). The GPU shader's masked-kind byte content is separately pinned by
    // `cluster_cull_spv_sync.rs`; the end-to-end "flagged lights survive on hardware" proof is
    // VB-P1b's `vb_mesh_froxel` equality golden.
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(0, 2, 1.0, &c);
    let lights = vec![
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0).with_sdf_shadow(),
        GoldenLight::spot([0.2, 0.2, 0.5], [0.0, 0.0, 1.0], [1.0, 1.0, 0.5], 3000.0, 3.0, 20.0, 35.0)
            .with_atlas_slot(4),
    ];
    assert!(lights[0].casts_sdf_shadow(), "index 0 must carry the P6 R1 shadow flag (bit 16)");
    assert_eq!(lights[1].atlas_slot(), 4, "index 1 must carry a real atlas slot (bits 17..21)");
    assert!(lights[1].casts_sdf_shadow(), "a real atlas slot also sets the shadow flag (bit 16)");

    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights);
    assert_eq!(grid.len(), c.cluster_count() as usize);

    let mut shadow_flagged_point_seen = 0u32;
    let mut atlas_slotted_spot_seen = 0u32;
    for cell in &grid {
        for &i in cell {
            if i == 0 {
                shadow_flagged_point_seen += 1;
            }
            if i == 1 {
                atlas_slotted_spot_seen += 1;
            }
        }
    }
    assert!(
        shadow_flagged_point_seen > 0,
        "the shadow-flagged point must SURVIVE the host-oracle cull: `GoldenLight::kind()` masks \
         off bit 16 before the LIGHT_KIND_POINT comparison, so the flag never perturbs the kind \
         classification"
    );
    assert!(
        atlas_slotted_spot_seen > 0,
        "the atlas-slotted spot must SURVIVE the host-oracle cull: `GoldenLight::kind()` masks \
         off bits 17..21 before the LIGHT_KIND_SPOT comparison, so the slot never perturbs the \
         kind classification"
    );
}

#[test]
fn clustered_resolve_off_is_byte_identical_to_brute_force() {
    // The L1 0%-gate: a header with clusters DISABLED makes `golden_deferred_resolve_clustered`
    // delegate to the brute-force table resolve — byte-identical to L0b for every pixel.
    let c = cfg();
    let mats = materials();
    // clusters_enabled == false (plain L0 header).
    let header = GoldenLightHeader::new(1, 1, 1.0);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.5], [1.0, 1.0, 1.0], 5000.0, 10.0),
    ];
    let grid: Vec<Vec<u32>> = vec![Vec::new(); c.cluster_count() as usize]; // unused on OFF
    for &view_t in &[0.5_f32, 1.0, 1.5] {
        let attrs = lit_attrs(view_t);
        let (ro, rd) = (RO, RD);
        let want = golden_deferred_resolve_table(attrs, ro, rd, &mats, &header, &lights);
        let got = golden_deferred_resolve_clustered(
            attrs, 32, 32, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &mats, &header, &lights, &c,
            &grid,
        );
        assert_eq!(got, want, "L1 OFF must be byte-identical to the brute-force resolve");
    }
}

#[test]
fn clustered_resolve_equals_brute_force_for_a_multi_light_scene() {
    // THE load-bearing L1 golden: the CLUSTERED resolve (looping only the pixel's froxel
    // lights) must produce the SAME image as the brute-force resolve (looping all lights) —
    // because the cull is EXACT for the test scene (no light wrongly dropped, all under the
    // cap). Tested per pixel across the whole frame.
    let c = cfg();
    let mats = materials();
    // A multi-light scene: 1 directional (global) + several point/spot spread through the
    // view. The clustered header carries the exp-Z factors.
    let header = GoldenLightHeader::new_clustered(1, 4, 1.0, &c);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        // Point lights at world positions across the view (z near 0, the surface band), each
        // with a generous range so several froxels keep them.
        GoldenLight::point([-0.5, 0.3, 0.0], [1.0, 0.2, 0.2], 2000.0, 2.0),
        GoldenLight::point([0.5, -0.3, 0.0], [0.2, 1.0, 0.2], 2000.0, 2.0),
        GoldenLight::point([0.0, 0.0, 0.2], [0.2, 0.2, 1.0], 2000.0, 2.5),
        GoldenLight::spot([0.2, 0.2, 0.5], [0.0, 0.0, 1.0], [1.0, 1.0, 0.5], 3000.0, 3.0, 20.0, 35.0),
    ];

    // Build the cull grid ONCE for the whole frame (as the GPU cull pass does).
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights);

    // Compare the clustered vs brute-force resolve per pixel. Since the cull is geometric +
    // exact (the froxel AABB conservatively encloses the pixel's world point) and the
    // per-light sum is table-ordered in both, the results match bit-for-bit.
    let l0b_header = GoldenLightHeader::new(1, 4, 1.0); // same lights, clusters OFF (brute force)
    let mut compared = 0u64;
    let mut lit_pixels = 0u64;
    for py in (0..SDF_IMG_H).step_by(3) {
        for px in (0..SDF_IMG_W).step_by(3) {
            let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            // A surface at world z ≈ 0 -> view_t ≈ 2 (camera at z = 2). Use view_t = 2.0 so
            // the reconstructed P sits in the scene's lit band where the lights live.
            let attrs = lit_attrs(2.0);
            let brute = golden_deferred_resolve_table(attrs, ro, rd, &mats, &l0b_header, &lights);
            let clustered = golden_deferred_resolve_clustered(
                attrs, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &mats, &header,
                &lights, &c, &grid,
            );
            assert_eq!(
                clustered, brute,
                "clustered resolve != brute force at ({px},{py}) — the cull dropped an in-range light"
            );
            compared += 1;
            if clustered != golden_deferred_resolve_table(attrs, ro, rd, &mats, &l0b_header, &[lights[0]]) {
                lit_pixels += 1;
            }
        }
    }
    assert!(compared > 0);
    assert!(lit_pixels > 0, "the multi-light scene must light at least one pixel beyond the directional");
}

#[test]
fn pixel_maps_to_a_unique_froxel_and_the_cull_set_is_a_superset_of_in_range() {
    // No false drop under the cap (the property-style L1 invariant): for a pixel mapped to
    // its froxel, EVERY light whose bounding sphere contains the pixel's reconstructed P is
    // present in that froxel's cull set (the froxel AABB encloses P, so a sphere reaching P
    // reaches the AABB). Checked against a brute scan of the point/spot block.
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(0, 3, 1.0, &c);
    let lights = vec![
        GoldenLight::point([-0.3, 0.2, 0.0], [1.0, 1.0, 1.0], 100.0, 1.5),
        GoldenLight::point([0.4, -0.1, 0.1], [1.0, 1.0, 1.0], 100.0, 1.0),
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 2.5),
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights);

    for py in (0..SDF_IMG_H).step_by(7) {
        for px in (0..SDF_IMG_W).step_by(7) {
            let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let view_t = 2.0_f32; // P at world z ≈ 0
            let p = [ro[0] + rd[0] * view_t, ro[1] + rd[1] * view_t, ro[2] + rd[2] * view_t];
            let view_z = view_t; // ortho
            let (tx, ty) = golden_cluster_xy_tile(px, py, SDF_IMG_W, SDF_IMG_H, &c);
            let zsl = golden_cluster_z_slice(view_z, &c);
            let cluster = golden_cluster_index(tx, ty, zsl, c.dim_x, c.dim_z) as usize;
            let set = &grid[cluster];
            // Every light whose sphere contains P must be in the froxel's cull set.
            for (i, l) in lights.iter().enumerate() {
                let pos = [l.pos_range[0], l.pos_range[1], l.pos_range[2]];
                let r = l.pos_range[3];
                let d2 = (pos[0] - p[0]).powi(2) + (pos[1] - p[1]).powi(2) + (pos[2] - p[2]).powi(2);
                if d2 <= r * r {
                    assert!(
                        set.contains(&(i as u32)),
                        "no false drop: light {i} reaches P at ({px},{py}) but is absent from froxel {cluster}"
                    );
                }
            }
        }
    }
}

/// Pins §1.3's occupancy table exactly, and re-asserts the cap non-saturation property §6's
/// byte-identity discharge depends on.
#[test]
fn cluster_cull_occupancy_profile_matches_the_published_table() {
    let c = vb_p1d_bench_cluster_cfg();
    assert_eq!(
        c.cluster_count(),
        3456,
        "invariant: the ClusterConfig::default() mirror must stay 16x9x24 — every literal below \
         was measured against exactly this froxel count"
    );

    for &(n_ps, expected_total, expected_non_empty, expected_max) in &PUBLISHED_TABLE {
        let (total, non_empty, max_per_froxel) = occupancy_at(n_ps);

        assert_eq!(
            (total, non_empty, max_per_froxel),
            (expected_total, expected_non_empty, expected_max),
            "N_ps={n_ps}: occupancy drifted from docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md §1.3's \
             published table (total_indices/non_empty_froxels/max_per_froxel) — either the host \
             oracle (`golden_cluster_cull`) or the bench rig (camera / `light_position` / \
             `light_range`) changed since the table was measured. Do not adjust these literals to \
             match a new run; §6/§7/§10 of the plan are anchored on the published numbers"
        );

        assert!(
            total < INDEX_LIST_CAP as usize,
            "N_ps={n_ps}: total_indices ({total}) must stay under INDEX_LIST_CAP \
             ({INDEX_LIST_CAP}) — the plan's byte-identity argument for the hierarchical arm \
             depends on this: once the flat cull's global InterlockedAdd saturates the cap, claim \
             order decides which froxel loses its tail, and the flat/hierarchical arms are no \
             longer comparable byte-for-byte"
        );
        assert!(
            max_per_froxel < c.max_lights_per_cluster as usize,
            "N_ps={n_ps}: max_per_froxel ({max_per_froxel}) must stay under \
             max_lights_per_cluster ({}) — the O2 per-froxel clamp-and-drop must never trigger on \
             this rig, or the two arms diverge for the same claim-order reason as the \
             INDEX_LIST_CAP check above",
            c.max_lights_per_cluster
        );
    }
}

/// Pins §1.3's headline claim: at `N_ps=512` the cull is dominated by rejection work — under
/// 0.2 % of the `froxel_count * N_ps` pair tests actually succeed.
#[test]
fn cluster_cull_rejection_ratio_at_n512_matches_the_headline_claim() {
    let c = vb_p1d_bench_cluster_cfg();
    let n_ps = 512_u32;
    let (total, _non_empty, _max_per_froxel) = occupancy_at(n_ps);

    let pair_tests = u64::from(c.cluster_count()) * u64::from(n_ps);
    let accept_ratio = total as f64 / pair_tests as f64;

    assert!(
        accept_ratio < 0.002,
        "§1.3's headline claim: at N_ps=512 the cull performs {pair_tests} froxel x light pair \
         tests and only {total} succeed (accept_ratio={accept_ratio:.5}, must be < 0.2%) — the \
         pass is meant to be >99.8% pure rejection work, which is the entire justification for a \
         hierarchical level that rejects whole blocks of froxels against whole ranges of lights"
    );
}
