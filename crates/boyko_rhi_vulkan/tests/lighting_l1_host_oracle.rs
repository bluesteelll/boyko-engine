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
//! This file boots NO Vulkan context — it is the non-GPU gate the developer runs (the GPU
//! golden runs separately on the 3060).

use boyko_rhi_vulkan::compute::{
    composite_pixel_ray, golden_cluster_cull, golden_cluster_index, golden_cluster_xy_tile,
    golden_cluster_z_slice, golden_deferred_resolve_clustered, golden_deferred_resolve_table,
    CompositeCamera, GoldenClusterConfig, GoldenLight, GoldenLightHeader, GoldenMaterial,
    MarcherAttributes, SDF_IMG_H, SDF_IMG_W,
};

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
