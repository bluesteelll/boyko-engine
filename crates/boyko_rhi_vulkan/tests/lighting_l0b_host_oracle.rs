//! Lighting L0b host-oracle (CPU-only — NO GPU required).
//!
//! Proves the L0b point/spot RESOLVE path the GPU golden compares against:
//! - the host `GoldenLight::point` / `GoldenLight::spot` bakes mirror
//!   `boyko_render::light::{from_point, from_spot}` (`I = Φ/(4π)` / `I = Φ/(2π(1−cos))`),
//! - the table oracle `golden_deferred_resolve_table` reconstructs the surface world
//!   position `P = ro + rd * view_t` (the `gViewT` lane) and applies the range cull +
//!   smooth windowed inverse-square attenuation + (spot) the O2 cone falloff, and
//! - the 0%-gate: a table with ZERO point/spot lights produces byte-identical output to
//!   the L0a directional/sky path (the point/spot loop body never runs).
//!
//! This file boots NO Vulkan context — it is a pure host-math regression the developer
//! runs as part of the non-GPU gate (the GPU golden runs separately on the 3060).

use core::f32::consts::PI;

use boyko_rhi_vulkan::compute::{
    depth_to_t, golden_deferred_resolve_table, golden_marcher_attributes, sdf_op, CompositeCamera,
    GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes, SdfEdit, LIGHTING_FLAG_AO,
    LIGHTING_FLAG_SHADOWS, MESH_DEPTH_CLEAR, PBR_SKY_DIFFUSE, SDF_IMG_H, SDF_IMG_W,
};

/// The ray origin/dir used to reconstruct `P = ro + rd * view_t` in the L0b path. `rd` is
/// unit (the shared ray-gen contract), so `view_t` is the true world distance.
const RO: [f32; 3] = [0.0, 0.0, 2.0];
const RD: [f32; 3] = [0.0, 0.0, -1.0];

/// A lit (mask == 1) attribute with a chosen `view_t`, so the L0b path reconstructs a
/// known surface `P`. The oct normal decodes to roughly +Z (toward the camera).
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

fn materials() -> Vec<GoldenMaterial> {
    vec![GoldenMaterial::default()]
}

#[test]
fn point_bakes_phi_over_4pi_into_the_color_lane() {
    let phi = 100.0_f32;
    let g = GoldenLight::point([1.0, 2.0, 3.0], [1.0, 0.5, 0.25], phi, 10.0);
    let i = phi / (4.0 * PI);
    let eps = 1e-4;
    assert!((g.color_cone[0] - i).abs() < eps, "R = color.r * I");
    assert!((g.color_cone[1] - 0.5 * i).abs() < eps, "G = color.g * I");
    assert!((g.color_cone[2] - 0.25 * i).abs() < eps, "B = color.b * I");
    assert_eq!([g.pos_range[0], g.pos_range[1], g.pos_range[2]], [1.0, 2.0, 3.0]);
    assert_eq!(g.pos_range[3], 10.0, "range stored raw in pos_range.w");
}

#[test]
fn spot_bakes_phi_over_2pi_one_minus_cos_outer() {
    let phi = 200.0_f32;
    let outer = 30.0_f32;
    let g = GoldenLight::spot([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], phi, 5.0, 15.0, outer);
    let cos_outer = outer.to_radians().cos();
    let i = phi / (2.0 * PI * (1.0 - cos_outer));
    assert!((g.color_cone[0] - i).abs() < 1e-3, "expected I={i}, got {}", g.color_cone[0]);
}

#[test]
fn zero_point_spot_table_is_byte_identical_to_l0a() {
    // 0%-gate: a table with only directional + sky lights (point_spot_count == 0) must
    // produce the SAME packed color whether or not the L0b loop is present — the point/spot
    // loop `[l0a_count .. light_count)` is empty, so it never runs.
    let mats = materials();
    // L0a-only table: 2 no-P lights (a directional + a sky), 0 point/spot.
    let header_l0a = GoldenLightHeader::new(2, 0, 1.0);
    let lights_l0a = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::sky(PBR_SKY_DIFFUSE, PBR_SKY_DIFFUSE),
    ];
    for &view_t in &[0.0_f32, 0.5, 1.0, 2.0, 1.0e30] {
        for &mask in &[1u8, 0u8] {
            let mut a = lit_attrs(view_t);
            a.mask = mask;
            // With point_spot_count == 0 the L0b loop is empty; the result equals the
            // L0a-only resolve regardless of `view_t` (the gViewT read is never consumed).
            let got = golden_deferred_resolve_table(a, RO, RD, &mats, &header_l0a, &lights_l0a);
            // Re-run with a DIFFERENT view_t: with no point/spot lights, `view_t` must not
            // change the output (proves the read-under-no-light invariance).
            let mut a2 = a;
            a2.view_t = if view_t == 0.0 { 99.0 } else { 0.0 };
            let got2 = golden_deferred_resolve_table(a2, RO, RD, &mats, &header_l0a, &lights_l0a);
            assert_eq!(got, got2, "view_t must not affect a zero-point/spot table");
        }
    }
}

#[test]
fn a_point_light_brightens_the_lit_pixel_within_range() {
    // A point light placed near the reconstructed surface P = ro + rd*1.0 = (0,0,1) must
    // add radiance to the lit pixel vs the directional-only baseline.
    let mats = materials();
    let attrs = lit_attrs(1.0); // P = (0, 0, 1)

    // Baseline: one directional only (no point/spot).
    let header_dir = GoldenLightHeader::new(1, 0, 1.0);
    let lights_dir = vec![GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
    let baseline = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_dir, &lights_dir);

    // + a bright point light just in front of P, well inside its range.
    let header_pt = GoldenLightHeader::new(1, 1, 1.0);
    let lights_pt = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.5], [1.0, 1.0, 1.0], 5000.0, 10.0),
    ];
    let with_point = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_pt, &lights_pt);

    assert_ne!(with_point, baseline, "a point light inside range must change the lit pixel");
}

#[test]
fn a_point_light_outside_range_is_culled() {
    // A point light farther than its `range` from the surface contributes NOTHING (the
    // range cull `d2 > range2` `continue`s) — the lit pixel equals the directional baseline.
    let mats = materials();
    let attrs = lit_attrs(1.0); // P = (0, 0, 1)

    let header_dir = GoldenLightHeader::new(1, 0, 1.0);
    let lights_dir = vec![GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
    let baseline = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_dir, &lights_dir);

    // A point light at distance 100 with range 1.0 — far outside the cull sphere.
    let header_pt = GoldenLightHeader::new(1, 1, 1.0);
    let lights_pt = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 100.0, 1.0], [1.0, 1.0, 1.0], 5000.0, 1.0),
    ];
    let culled = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_pt, &lights_pt);

    assert_eq!(culled, baseline, "an out-of-range point light must be culled (no contribution)");
}

#[test]
fn a_spot_light_outside_its_cone_contributes_nothing() {
    // A spot light whose axis points AWAY from the surface (the surface is outside the
    // cone) contributes nothing: the cone term `saturate((cosA - cos_outer)/...)` is 0.
    let mats = materials();
    let attrs = lit_attrs(1.0); // P = (0, 0, 1)

    let header_dir = GoldenLightHeader::new(1, 0, 1.0);
    let lights_dir = vec![GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
    let baseline = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_dir, &lights_dir);

    // A spot at (0,0,1.5) but aiming +Y (away from P at (0,0,1), which is along -Z from it).
    // `dir` is the to-light axis; the surface->light dir reversed (-l) must fall outside the
    // narrow cone, so the cone falloff is zero.
    let header_sp = GoldenLightHeader::new(1, 1, 1.0);
    let lights_sp = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::spot([0.0, 0.0, 1.5], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0], 5000.0, 10.0, 5.0, 10.0),
    ];
    let outside_cone = golden_deferred_resolve_table(attrs, RO, RD, &mats, &header_sp, &lights_sp);

    assert_eq!(outside_cone, baseline, "a surface outside the spot cone gets no spot contribution");
}

#[test]
fn p_reconstruction_uses_ro_plus_rd_times_view_t() {
    // The L0b path reconstructs `P = ro + rd * view_t`. A point light placed slightly in
    // FRONT of the surface (so `NoL > 0` and `d2 > 0`, avoiding the degenerate light-at-
    // surface case) is in range for one `view_t` but culled for another, so changing
    // `view_t` must change the lit result (proves P depends on view_t).
    let mats = materials();

    // ro=(0,0,2), rd=(0,0,-1): view_t==1 -> P==(0,0,1), view_t==0 -> P==(0,0,2). The light
    // sits at (0,0,1.4): distance 0.4 from P(view_t==1) (in range 0.5, lit), 0.6 from
    // P(view_t==0) (> range 0.5, culled).
    let header_pt = GoldenLightHeader::new(1, 1, 1.0);
    let lights_pt = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.4], [1.0, 1.0, 1.0], 5000.0, 0.5),
    ];

    // view_t == 1 -> P == (0,0,1): light at distance 0.4 < range 0.5 (in range, NoL > 0).
    let near = golden_deferred_resolve_table(lit_attrs(1.0), RO, RD, &mats, &header_pt, &lights_pt);
    // view_t == 0 -> P == (0,0,2): light at distance 0.6 > range 0.5 (culled).
    let far = golden_deferred_resolve_table(lit_attrs(0.0), RO, RD, &mats, &header_pt, &lights_pt);

    assert_ne!(near, far, "P must depend on view_t (ro + rd * view_t)");
}

#[test]
fn every_real_pixel_writes_a_finite_or_sentinel_view_t() {
    // C2 EXACTLY-ONCE coverage (host mirror of the full-frame gViewT golden): for every
    // real pixel the marcher attribute oracle assigns `view_t` exactly once — the REAL
    // marched `t` (finite, > 0) on the SDF-lit arm (mask == 1), and (with NO mesh fed here)
    // the `1.0e30` sentinel on the pure-background arm. NO pixel is left with an
    // uninitialized / NaN lane. (The mesh arm's `view_t == t_mesh` is covered by
    // `mesh_owned_pixel_carries_t_mesh_not_sentinel`.)
    let mats = vec![GoldenMaterial::default()];
    let scene: Vec<SdfEdit> = vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ];
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let mut sdf_hits = 0u64;
    let mut bg_hits = 0u64;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            // No mesh: the clear depth makes every pixel SDF-or-background.
            let a = golden_marcher_attributes(
                &scene, &mats, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                CompositeCamera::Ortho, 1.0, flags, [0.0, 0.0, 1.0],
            );
            assert!(a.view_t.is_finite() || a.view_t == 1.0e30, "view_t must never be NaN");
            if a.mask == 1 {
                assert!(
                    a.view_t.is_finite() && a.view_t > 0.0 && a.view_t < 1.0e30,
                    "SDF-lit pixel ({px},{py}) must carry a finite marched t, got {}",
                    a.view_t
                );
                sdf_hits += 1;
            } else {
                assert_eq!(
                    a.view_t, 1.0e30,
                    "non-lit pixel ({px},{py}) must carry the 1.0e30 sentinel, got {}",
                    a.view_t
                );
                bg_hits += 1;
            }
        }
    }
    assert!(sdf_hits > 0, "the scene must produce at least one SDF-lit pixel");
    assert!(bg_hits > 0, "the scene must produce at least one background pixel");
}

#[test]
fn mesh_owned_pixel_carries_t_mesh_not_sentinel() {
    // Render P7/P5-r1b UNLOCK: a mesh-covered pixel the SDF does NOT win is raster-owned, and
    // the marcher now stores the MESH surface ray-t `t_mesh` (= `depth_to_t(mesh_depth)`) into
    // gViewT instead of the old `1.0e30` sentinel — so the deferred resolve reconstructs the
    // real mesh `P` (in-range point/spot lighting) AND the SSAO pass processes the mesh pixel.
    // The host mirror `golden_marcher_attributes` must emit the SAME value (host == GPU).
    let mats = vec![GoldenMaterial::default()];
    // An EMPTY scene (no edits) — the SDF can never win, so EVERY mesh-covered pixel is
    // raster-owned and a pixel with the clear depth is pure background.
    let scene: Vec<SdfEdit> = Vec::new();
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;

    // A finite mesh depth strictly less than the far-plane clear -> `has_mesh == true`.
    let mesh_depth = 0.5_f32;
    assert!(mesh_depth < MESH_DEPTH_CLEAR, "the probe depth must be in front of the clear");
    let expected_t = depth_to_t(mesh_depth);

    let mesh = golden_marcher_attributes(
        &scene, &mats, mesh_depth, 12, 17, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
        flags, [0.0, 0.0, 1.0],
    );
    assert_eq!(
        mesh.view_t, expected_t,
        "a mesh-owned pixel must carry t_mesh = depth_to_t(mesh_depth) = {expected_t}, got {}",
        mesh.view_t
    );
    assert_eq!(mesh.mask, 1, "the raster-PBR mesh producer is mask == 1");
    assert!(mesh.view_t.is_finite(), "t_mesh must be finite (it drives P reconstruction + SSAO)");

    // The same scene at the CLEAR depth (no mesh, SDF empty -> miss) keeps the `1.0e30` sentinel.
    let bg = golden_marcher_attributes(
        &scene, &mats, MESH_DEPTH_CLEAR, 12, 17, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
        flags, [0.0, 0.0, 1.0],
    );
    assert_eq!(bg.mask, 0, "a pure-background pixel is mask == 0");
    assert_eq!(
        bg.view_t, 1.0e30,
        "a pure-background pixel must keep the 1.0e30 sentinel, got {}",
        bg.view_t
    );
}
