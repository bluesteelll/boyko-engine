//! GOLDEN-EDSL P0 (SSAO pilot), step 3 — the `derived == hand` proof.
//!
//! `docs/GOLDEN-EDSL-MIGRATION-PLAN.md` opens with the CIRCULARITY GUARDRAIL: an oracle
//! derived from the same eDSL AST as the shader cannot catch a bug in that AST, so the
//! derivation is trustworthy only while it is INDEPENDENTLY anchored to real GPU output, and
//! a hand mirror is never deleted in the step that adds its derived replacement. This file
//! is the "land alongside, prove agreement" leg of that pattern: it asserts that the
//! eDSL-DERIVED SSAO oracle (`golden_ssao_attributes_derived`, the generic
//! `boyko_shaderdsl::ssao::ssao_estimate_body_params` instantiated over `EvalCf` behind a
//! transcription of the hand-written seam glue) reproduces the HAND mirror
//! (`golden_ssao_attributes`) BIT-FOR-BIT — `f32::to_bits` equality, never a tolerance —
//! over the SAME inputs the owner's GPU golden feeds it.
//!
//! It is NOT the GPU anchor. The anchor stays `tests/sdf_gbuffer_hybrid.rs`'s
//! `ssao_variants_match_host` (device, `boot_render_or_skip`), which pins the GPU readback
//! against the HAND mirror; this file is device-free and runs everywhere. Step 4 (the owner's
//! `scripts/golden.ps1 -Check`) and step 5 (retiring the hand mirror) are NOT part of it.
//!
//! Inputs, in order of fidelity to the GPU golden:
//!   1. the three P4b ORTHO scenes at the golden 64x64 extent under ALL three quality presets
//!      (the exact `ssao_variants_match_host` inputs);
//!   2. synthetic ORTHO fixtures that reach every pixel — border rows/columns (the
//!      `reconstruct` bounds fallback), a checkerboard mask (the non-lit-tap skip in every
//!      neighbourhood), a single-lit seam, and an all-non-lit image (the early return);
//!   3. a PERSPECTIVE camera (the `pix_radius` clamp arm the ORTHO scenes never take);
//!   4. OFF-TABLE synthetic `SsaoParams` — see below, they are what makes `radius`, `strength`
//!      and `eps` falsifiable at all.
//!
//! Every assertion carries permanent POSITIVE CONTROLS — lit-pixel and occluded-pixel counts
//! that must be non-zero, and per-FIELD proofs that the oracles actually read `params` — so the
//! gate cannot pass from an empty comparison or from a derived oracle that silently substitutes
//! a module const for a `params` field (this repository's catalogued failure mode: the gate that
//! could not fail). Which control covers which field is NOT uniform, and the distinction is
//! load-bearing:
//!
//!   * `slices` and `steps` are covered by `presets_are_distinguishable_by_both_oracles`,
//!     because the three `SSAO_PARAMS` rows differ in exactly those two fields.
//!   * `radius`, `strength` and `eps` are IDENTICAL in all three rows (`0.5`, `2.5`, `1.0e-4`),
//!     so no preset-driven leg can see them: a derived oracle that read the module const for
//!     any of the three would pass every preset leg. They are falsified only by
//!     `derived_matches_hand_mirror_under_off_table_params`, which drives both oracles with a
//!     synthetic `SsaoParams` that is off the table in all five fields (both oracles take
//!     `&SsaoParams`, so nothing about the table constrains what may be fed) and additionally
//!     proves, field by field, that the fixture makes each of the five observable.

mod common;

use boyko_rhi_vulkan::compute::{
    CompositeCamera, DEFAULT_LIGHT_DIR, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS,
    MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SSAO_PARAMS, SSAO_QUALITY_HIGH, SSAO_QUALITY_LOW,
    SSAO_QUALITY_MEDIUM, SSAO_VIEWT_BG, SdfEdit, SsaoParams, mesh_depth_for_z, pixel_world_xy,
    sdf_op,
};
use boyko_rhi_vulkan::goldens::{
    GoldenMaterial, MarcherAttributes, golden_gbuffer, golden_ssao_attributes,
    golden_ssao_attributes_derived,
};
use common::{MESH_Z, QUAD_X_MAX, QUAD_X_MIN, QUAD_Y_MAX, QUAD_Y_MIN};

// ---- P4b scene fixtures ------------------------------------------------------------------
//
// Transcriptions of `tests/sdf_gbuffer_hybrid.rs`'s private `crater` / `box_csg` /
// `smooth_union` / `mesh_covers_pixel` / `expected_mesh_depth`. That binary's items are
// private to it and the file is frozen for this step; hoisting them into `tests/common` is
// step-5 work (the retirement commit that also swaps its call sites to the derived oracle).

/// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// A box CSG scene (a box unioned, exercising the box primitive + the mesh occlusion).
fn box_csg() -> Vec<SdfEdit> {
    vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
}

/// A smooth-union scene (two spheres blended), exercising the smooth-min path.
fn smooth_union() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
    ]
}

/// The three P4b scenes in the GPU golden's order, tagged for the failure message.
fn p4b_scenes() -> [(&'static str, Vec<SdfEdit>); 3] {
    [("crater_csg", crater()), ("box_csg", box_csg()), ("smooth_union", smooth_union())]
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint.
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU produces: the constant inside the quad, the clear outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        MESH_DEPTH_CLEAR
    }
}

/// The Stage-1 host G-buffer for one P4b scene, EXACTLY as `ssao_variants_match_host` builds
/// it: the default material table, the quad mesh depth, the golden extent, ORTHO, `omega = 1`,
/// shadows + AO armed, the default light.
fn p4b_gbuffer(edits: &[SdfEdit]) -> Vec<MarcherAttributes> {
    golden_gbuffer(
        edits,
        &[GoldenMaterial::default()],
        expected_mesh_depth,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
        1.0,
        LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        DEFAULT_LIGHT_DIR,
    )
}

// ---- Synthetic fixtures ------------------------------------------------------------------

/// The center pixel of the golden extent (the synthetic fixtures are shaped around it).
const CX: i32 = (SDF_IMG_W / 2) as i32;
const CY: i32 = (SDF_IMG_H / 2) as i32;

/// The constant octahedral normal every synthetic pixel carries (mirrors the lib's
/// `ssao_gather_tests` fixture: the gather decodes only the CENTER pixel's normal).
const OCT_RG: [u8; 2] = [128, 128];

/// Builds a golden-extent synthetic G-buffer from a per-pixel `(lit, view_t)` field.
fn synthetic_gbuffer<F: Fn(i32, i32) -> (bool, f32)>(field: F) -> Vec<MarcherAttributes> {
    let mut gbuf = Vec::with_capacity((SDF_IMG_W as usize) * (SDF_IMG_H as usize));
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let (lit, view_t) = field(px as i32, py as i32);
            gbuf.push(MarcherAttributes {
                base_rgb: [0, 0, 0],
                oct_rg: OCT_RG,
                mat_id: 0,
                shadow: 255,
                ao: 255,
                mask: if lit { 1 } else { 0 },
                view_t: if lit { view_t } else { SSAO_VIEWT_BG },
            });
        }
    }
    gbuf
}

/// (a) A V-valley crevice: `view_t` shrinks with the radius from the center, so every
/// neighbour rises above every center's tangent plane — strongly occluded everywhere.
fn crevice_gbuffer() -> Vec<MarcherAttributes> {
    synthetic_gbuffer(|x, y| {
        let dx = (x - CX) as f32;
        let dy = (y - CY) as f32;
        (true, 1.5 - 0.01 * (dx * dx + dy * dy).sqrt())
    })
}

/// (b) A checkerboard of lit / non-lit pixels on a flat depth: the non-lit-tap skip fires in
/// EVERY neighbourhood, on top of the crevice-free flat geometry.
fn checkerboard_gbuffer() -> Vec<MarcherAttributes> {
    synthetic_gbuffer(|x, y| ((x + y) % 2 == 0, 1.5))
}

/// (b') A checkerboard over the crevice depth: the mask skip AND a raised horizon, so the
/// occluded-pixel control is satisfiable while every neighbourhood still carries skipped taps.
fn checkerboard_crevice_gbuffer() -> Vec<MarcherAttributes> {
    synthetic_gbuffer(|x, y| {
        let dx = (x - CX) as f32;
        let dy = (y - CY) as f32;
        ((x + y) % 2 == 0, 1.5 - 0.01 * (dx * dx + dy * dy).sqrt())
    })
}

/// (c) A single lit center pixel on an all-background image: every tap is a seam skip.
fn seam_gbuffer() -> Vec<MarcherAttributes> {
    synthetic_gbuffer(|x, y| (x == CX && y == CY, 1.5))
}

/// (d) An all-non-lit image: the early-return path at every pixel.
fn dark_gbuffer() -> Vec<MarcherAttributes> {
    synthetic_gbuffer(|_x, _y| (false, 0.0))
}

/// The three presets in table order, tagged.
fn presets() -> [(&'static str, &'static SsaoParams); 3] {
    [
        ("low", &SSAO_PARAMS[SSAO_QUALITY_LOW]),
        ("medium", &SSAO_PARAMS[SSAO_QUALITY_MEDIUM]),
        ("high", &SSAO_PARAMS[SSAO_QUALITY_HIGH]),
    ]
}

/// Asserts `hand.to_bits() == derived.to_bits()` at EVERY pixel of a golden-extent
/// `gbuf`, and returns `(lit, occluded)` — the number of lit center pixels and the number of
/// pixels whose AO is strictly below `1.0` — so the caller can assert the comparison was not
/// empty. `tag` names the fixture / preset in the failure message.
fn assert_bit_equal_all_pixels(
    gbuf: &[MarcherAttributes],
    camera: CompositeCamera,
    params: &SsaoParams,
    tag: &str,
) -> (u64, u64) {
    let mut lit = 0u64;
    let mut occluded = 0u64;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let hand = golden_ssao_attributes(gbuf, px, py, SDF_IMG_W, SDF_IMG_H, camera, params);
            let derived =
                golden_ssao_attributes_derived(gbuf, px, py, SDF_IMG_W, SDF_IMG_H, camera, params);
            assert_eq!(
                hand.to_bits(),
                derived.to_bits(),
                "[{tag}] pixel ({px},{py}): the eDSL-DERIVED SSAO oracle is not bit-identical \
                 to the hand mirror: hand = {hand} ({:#010x}), derived = {derived} ({:#010x})",
                hand.to_bits(),
                derived.to_bits()
            );
            let idx = (py * SDF_IMG_W + px) as usize;
            if gbuf[idx].mask > 0 && gbuf[idx].view_t < SSAO_VIEWT_BG {
                lit += 1;
            }
            if hand < 1.0 {
                occluded += 1;
            }
        }
    }
    (lit, occluded)
}

/// Whether two oracle images (one call per pixel under `a` and under `b`) differ anywhere.
fn images_differ<O: Fn(u32, u32, &SsaoParams) -> f32>(
    oracle: &O,
    a: &SsaoParams,
    b: &SsaoParams,
) -> bool {
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            if oracle(px, py, a).to_bits() != oracle(px, py, b).to_bits() {
                return true;
            }
        }
    }
    false
}

#[test]
fn derived_matches_hand_mirror_over_p4b_scenes_all_presets() {
    // The GPU golden's exact inputs: 3 scenes x 3 presets x 64x64. Controls: every scene has
    // lit pixels, and every (scene, preset) has at least one occluded pixel — otherwise the
    // horizon path was never compared.
    for (scene, edits) in p4b_scenes() {
        let gbuf = p4b_gbuffer(&edits);
        for (quality, params) in presets() {
            let tag = format!("p4b {scene} / {quality}");
            let (lit, occluded) =
                assert_bit_equal_all_pixels(&gbuf, CompositeCamera::Ortho, params, &tag);
            assert!(lit > 0, "[{tag}] control: the scene lit no pixel — nothing was compared");
            assert!(
                occluded > 0,
                "[{tag}] control: no pixel is occluded — the horizon path was never compared"
            );
            println!("[{tag}] bit-identical over {} px: lit = {lit}, occluded = {occluded}",
                SDF_IMG_W * SDF_IMG_H);
        }
    }
}

#[test]
fn derived_matches_hand_mirror_on_border_and_seam_fixtures() {
    // ORTHO synthetic fixtures over ALL pixels, so px == 0 / py == 0 / px == 63 / py == 63
    // exercise the `reconstruct` bounds fallback under every preset.
    let fixtures: [(&str, Vec<MarcherAttributes>, bool); 5] = [
        ("crevice", crevice_gbuffer(), true),
        ("checkerboard", checkerboard_gbuffer(), false),
        ("checkerboard_crevice", checkerboard_crevice_gbuffer(), true),
        ("seam", seam_gbuffer(), false),
        ("dark", dark_gbuffer(), false),
    ];
    for (name, gbuf, expect_occluded) in &fixtures {
        for (quality, params) in presets() {
            let tag = format!("fixture {name} / {quality}");
            let (lit, occluded) =
                assert_bit_equal_all_pixels(gbuf, CompositeCamera::Ortho, params, &tag);
            if *name == "dark" {
                assert_eq!(lit, 0, "[{tag}] control: the dark fixture must light no pixel");
                assert_eq!(occluded, 0, "[{tag}] control: the dark fixture must occlude nothing");
            } else {
                assert!(lit > 0, "[{tag}] control: the fixture lit no pixel");
            }
            if *expect_occluded {
                assert!(
                    occluded > 0,
                    "[{tag}] control: no pixel is occluded — the horizon path was never compared"
                );
            }
            println!("[{tag}] bit-identical: lit = {lit}, occluded = {occluded}");
        }
    }
}

#[test]
fn derived_matches_hand_mirror_under_perspective_camera() {
    // The PERSPECTIVE `pix_radius` arm (the clamped `R * (h/2) / (z * tan)` band) on the
    // crevice fixture, all presets, all pixels.
    let camera = CompositeCamera::Perspective {
        eye: [0.0, 0.0, 3.0],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov: 1.0,
        aspect: 1.0,
    };
    let gbuf = crevice_gbuffer();
    for (quality, params) in presets() {
        let tag = format!("perspective crevice / {quality}");
        let (lit, occluded) = assert_bit_equal_all_pixels(&gbuf, camera, params, &tag);
        assert!(lit > 0, "[{tag}] control: the fixture lit no pixel");
        assert!(
            occluded > 0,
            "[{tag}] control: no pixel is occluded — the horizon path was never compared"
        );
        println!("[{tag}] bit-identical: lit = {lit}, occluded = {occluded}");
    }
}

#[test]
fn presets_are_distinguishable_by_both_oracles() {
    // The permanent control that the `params` plumbing is LIVE in BOTH oracles: on the
    // crater scene, Low vs Medium and Medium vs High must differ at some pixel for the hand
    // mirror AND, separately, for the derived oracle. A derived oracle that silently used the
    // Medium consts would pass the Medium leg of the equivalence test and fail here.
    let gbuf = p4b_gbuffer(&crater());
    let hand = |px: u32, py: u32, params: &SsaoParams| {
        golden_ssao_attributes(&gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, params)
    };
    let derived = |px: u32, py: u32, params: &SsaoParams| {
        golden_ssao_attributes_derived(
            &gbuf, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, params,
        )
    };
    let low = &SSAO_PARAMS[SSAO_QUALITY_LOW];
    let medium = &SSAO_PARAMS[SSAO_QUALITY_MEDIUM];
    let high = &SSAO_PARAMS[SSAO_QUALITY_HIGH];

    assert!(images_differ(&hand, low, medium), "hand mirror: Low == Medium at every pixel");
    assert!(images_differ(&hand, medium, high), "hand mirror: Medium == High at every pixel");
    assert!(
        images_differ(&derived, low, medium),
        "derived oracle: Low == Medium at every pixel — its `params` plumbing is dead"
    );
    assert!(
        images_differ(&derived, medium, high),
        "derived oracle: Medium == High at every pixel — its `params` plumbing is dead"
    );
}

// ---- Off-table params --------------------------------------------------------------------
//
// The three `SSAO_PARAMS` rows differ ONLY in `slices` and `steps`; `radius` (0.5),
// `strength` (2.5) and `eps` (1.0e-4) are identical in all three. So no preset-driven leg can
// falsify those three fields: a derived oracle that read the module const for any of them
// would agree with the hand mirror on every row of the table. Both oracles take a plain
// `&SsaoParams`, so the gate closes that hole by driving them with a synthetic row that is off
// the table in ALL FIVE fields.

/// The off-table base row. Every field differs from every `SSAO_PARAMS` row, and `eps` is
/// deliberately LARGE ENOUGH TO BIND: `elev = dot(delta,N) / max(length(delta), eps)` only
/// reads `eps` when `length(delta) < eps`, and the golden fixtures' tap deltas are ~1.0e-2 to
/// ~3.5e-1 world units, so the table's `1.0e-4` is never the max. A small off-table `eps`
/// would therefore be UNOBSERVABLE and the leg would silently not cover the field.
/// `slices` must divide `SSAO_ROT_N` (16) for even slice spacing — both oracles `debug_assert`
/// it — so 4 and 8 are the off-table choices.
const OFF_TABLE: SsaoParams =
    SsaoParams { radius: 0.35, slices: 4, steps: 5, strength: 1.7, eps: 0.5 };

/// The five single-field variations of [`OFF_TABLE`], each tagged by the field it moves. They
/// are the per-field OBSERVABILITY control: if varying a field alone changes neither oracle's
/// image on the fixture, the equivalence assertion at that field is blind and the leg says so.
fn off_table_field_variations() -> [(&'static str, SsaoParams); 5] {
    [
        ("radius", SsaoParams { radius: 0.62, ..OFF_TABLE }),
        ("slices", SsaoParams { slices: 8, ..OFF_TABLE }),
        ("steps", SsaoParams { steps: 3, ..OFF_TABLE }),
        ("strength", SsaoParams { strength: 3.1, ..OFF_TABLE }),
        ("eps", SsaoParams { eps: 1.0e-3, ..OFF_TABLE }),
    ]
}

#[test]
fn derived_matches_hand_mirror_under_off_table_params() {
    // Fixtures with real occlusion under BOTH camera arms, so the horizon path (the only
    // consumer of `radius`/`eps`) and the complement (the only consumer of `strength`) are
    // both exercised at off-table scalars.
    let p4b = p4b_gbuffer(&crater());
    let crevice = crevice_gbuffer();
    let perspective = CompositeCamera::Perspective {
        eye: [0.0, 0.0, 3.0],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov: 1.0,
        aspect: 1.0,
    };
    let cases: [(&str, &[MarcherAttributes], CompositeCamera); 3] = [
        ("p4b crater_csg ortho", &p4b, CompositeCamera::Ortho),
        ("crevice ortho", &crevice, CompositeCamera::Ortho),
        ("crevice perspective", &crevice, perspective),
    ];

    // (1) The equivalence itself, at the off-table base row and at every single-field
    //     variation of it — 6 rows x 3 fixtures x 64x64 pixels.
    let mut rows: Vec<(String, SsaoParams)> = vec![("base".to_string(), OFF_TABLE)];
    for (field, params) in off_table_field_variations() {
        rows.push((format!("off-table {field}"), params));
    }
    for (case, gbuf, camera) in cases {
        for (row, params) in &rows {
            let tag = format!("{case} / {row}");
            let (lit, occluded) = assert_bit_equal_all_pixels(gbuf, camera, params, &tag);
            assert!(lit > 0, "[{tag}] control: the fixture lit no pixel — nothing was compared");
            assert!(
                occluded > 0,
                "[{tag}] control: no pixel is occluded — the horizon path was never compared, \
                 so `radius`/`eps`/`strength` could not have been read"
            );
            println!("[{tag}] bit-identical: lit = {lit}, occluded = {occluded}");
        }
    }

    // (2) The per-field OBSERVABILITY control, on the crevice fixture under ORTHO: moving each
    //     field ALONE must change BOTH oracles' images. Without this, (1) could be comparing
    //     two oracles that both ignore the same field.
    let hand = |px: u32, py: u32, params: &SsaoParams| {
        golden_ssao_attributes(
            &crevice, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, params,
        )
    };
    let derived = |px: u32, py: u32, params: &SsaoParams| {
        golden_ssao_attributes_derived(
            &crevice, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, params,
        )
    };
    for (field, varied) in off_table_field_variations() {
        assert!(
            images_differ(&hand, &OFF_TABLE, &varied),
            "hand mirror: moving `{field}` alone changed no pixel — the off-table leg is blind \
             to that field, so its equivalence assertion proves nothing about it"
        );
        assert!(
            images_differ(&derived, &OFF_TABLE, &varied),
            "derived oracle: moving `{field}` alone changed no pixel — its `params.{field}` \
             plumbing is dead (a module const substituted for the field)"
        );
        println!("[off-table] `{field}` is observable in BOTH oracles");
    }
}
