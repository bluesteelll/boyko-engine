//! Lighting L0a host-oracle 0%-gate (CPU-only — NO GPU required).
//!
//! Proves the W1 byte-identity contract: the table-driven host oracle
//! [`golden_deferred_resolve_table`] fed the DEGENERATE light table (one directional
//! dir = +Z / white / illuminance 1.0, one sky with `sky == ground == (0.10,0.10,0.12)`,
//! exposure 1.0) reproduces the constant-path [`golden_deferred_resolve`]
//! BYTE-FOR-BYTE across a sweep of synthetic G-buffer attributes + materials + view rays.
//! Because `0.0 + x == x` and `x * 1.0 == x` are exact and the sky `lerp` folds when
//! `sky == ground`, the degenerate fold is bit-exact (no tolerance). Also asserts the
//! exposure multiply is identity at 1.0.
//!
//! This file boots NO Vulkan context — it is a pure host-math regression the developer
//! runs as part of the non-GPU gate (the GPU golden runs separately on the 3060).

use boyko_rhi_vulkan::compute::{
    golden_deferred_resolve, golden_deferred_resolve_table, GoldenLight, GoldenLightHeader,
    GoldenMaterial, MarcherAttributes, PBR_SKY_DIFFUSE,
};

/// The ray origin passed on the L0a-only path: point/spot lights are absent, so the
/// table oracle never reconstructs `P` from `ro` — any value is fine.
const RO_ZERO: [f32; 3] = [0.0, 0.0, 0.0];

/// Builds the degenerate 2-entry table (the 0%-gate anchor): a directional matching the
/// old `LIGHT_DIR` / `LIGHT_COLOR`, and a sky with `sky == ground == PBR_SKY_DIFFUSE`.
fn degenerate_table(exposure: f32) -> (GoldenLightHeader, Vec<GoldenLight>) {
    let header = GoldenLightHeader::new(2, 0, exposure);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::sky(PBR_SKY_DIFFUSE, PBR_SKY_DIFFUSE),
    ];
    (header, lights)
}

/// A small sweep of synthetic G-buffer attributes covering the SDF-lit branch + the
/// mask == 0 pass-through arms.
fn sweep_attrs() -> Vec<MarcherAttributes> {
    let mut v = Vec::new();
    for &mask in &[1u8, 0u8] {
        for &shadow in &[0u8, 128u8, 255u8] {
            for &ao in &[0u8, 200u8, 255u8] {
                for &(ox, oy) in &[(128u8, 128u8), (200u8, 60u8), (10u8, 240u8)] {
                    v.push(MarcherAttributes {
                        base_rgb: [180, 120, 90],
                        oct_rg: [ox, oy],
                        mat_id: 0,
                        shadow,
                        ao,
                        mask,
                        // L0a sweep: no point/spot lights, so view_t is never consumed
                        // (the sentinel; the read-under-mask gate would ignore it anyway).
                        view_t: 1.0e30,
                    });
                }
            }
        }
    }
    v
}

fn sweep_rays() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.0, -1.0],
        [0.2, 0.1, -0.97],
        [-0.3, 0.25, -0.92],
        [0.05, -0.4, -0.91],
    ]
}

fn materials() -> Vec<GoldenMaterial> {
    vec![
        GoldenMaterial::default(),
        GoldenMaterial::new([0.9, 0.1, 0.1, 1.0], 1.0, 0.3, 0.5, [0.0, 0.0, 0.0]),
        GoldenMaterial::new([0.2, 0.6, 0.9, 1.0], 0.0, 0.8, 0.5, [0.02, 0.0, 0.05]),
    ]
}

#[test]
fn degenerate_table_is_byte_identical_to_the_constant_path() {
    let (header, lights) = degenerate_table(1.0);
    let mats = materials();
    let mut compared = 0usize;
    for attrs in sweep_attrs() {
        for &rd in &sweep_rays() {
            // The constant-path oracle (the existing bit-exact source of truth) uses
            // material id 0 internally; sweep every material by forcing the id.
            for (mid, _m) in mats.iter().enumerate() {
                let mut a = attrs;
                a.mat_id = mid as u16;
                let want = golden_deferred_resolve(a, rd, &mats);
                // L0a path: no point/spot lights, so `ro` is unused by the loop body.
                let got = golden_deferred_resolve_table(a, RO_ZERO, rd, &mats, &header, &lights);
                assert_eq!(
                    got, want,
                    "L0a 0%-gate: table path 0x{got:08X} != constant path 0x{want:08X} \
                     (mask={}, shadow={}, ao={}, mat={mid})",
                    a.mask, a.shadow, a.ao
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 0, "the sweep must compare at least one pixel");
}

#[test]
fn exposure_is_identity_at_one() {
    // exposure == 1.0 must leave the accumulated radiance unchanged (x * 1.0 == x).
    let (h1, l1) = degenerate_table(1.0);
    let mats = materials();
    let attrs = MarcherAttributes {
        base_rgb: [200, 150, 100],
        oct_rg: [140, 120],
        mat_id: 0,
        shadow: 255,
        ao: 255,
        mask: 1,
        view_t: 1.0e30,
    };
    let rd = [0.1, 0.05, -0.99];
    let lit = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &h1, &l1);
    // Same as the constant path (already covered above) — re-asserted as the identity pin.
    assert_eq!(lit, golden_deferred_resolve(attrs, rd, &mats));
}

#[test]
fn exposure_above_one_brightens_a_lit_pixel() {
    // A non-identity exposure must change a LIT pixel (sanity that exposure is wired).
    let mats = materials();
    let attrs = MarcherAttributes {
        base_rgb: [200, 150, 100],
        oct_rg: [140, 120],
        mat_id: 0,
        shadow: 255,
        ao: 255,
        mask: 1,
        view_t: 1.0e30,
    };
    let rd = [0.0, 0.0, -1.0];
    let (h1, l1) = degenerate_table(1.0);
    let (h2, l2) = degenerate_table(4.0);
    let base = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &h1, &l1);
    let bright = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &h2, &l2);
    // A lit pixel below saturation gets brighter under exposure 4× (or clamps to white).
    assert!(bright != base || base == 0x00FF_FFFF, "exposure must affect the lit pixel");
}

#[test]
fn mask_zero_passes_base_through_under_any_table() {
    // The pass-through arm ignores the table entirely (the resolve's 0%-gate).
    let (header, lights) = degenerate_table(7.0); // exposure irrelevant on this arm
    let mats = materials();
    let attrs = MarcherAttributes {
        base_rgb: [33, 77, 211],
        oct_rg: [128, 128],
        mat_id: 0,
        shadow: 0,
        ao: 0,
        mask: 0,
        view_t: 1.0e30,
    };
    let rd = [0.0, 0.0, -1.0];
    let got = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &header, &lights);
    assert_eq!(got, golden_deferred_resolve(attrs, rd, &mats));
}
