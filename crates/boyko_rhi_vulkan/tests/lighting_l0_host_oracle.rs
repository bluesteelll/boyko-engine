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

use boyko_rhi_vulkan::compute::{pack_rgba, PBR_SKY_DIFFUSE};
use boyko_rhi_vulkan::goldens::{golden_deferred_resolve, golden_deferred_resolve_table, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes};

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

/// A table with NO sky light — a single directional in the L0a block, optionally one punctual.
/// The mask == 0 background arm then takes its `pack_rgba(base)` pass-through branch (there is no
/// SKY entry to paint), whatever the exposure or the punctual light: the resolve's
/// `golden_sky_background(..).unwrap_or_else(|| pack_rgba(base))` returns `None` when the scanned
/// L0a lights carry no `GOLDEN_LIGHT_KIND_SKY` entry.
fn no_sky_table(exposure: f32, with_punctual: bool) -> (GoldenLightHeader, Vec<GoldenLight>) {
    let mut lights = vec![GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0)];
    let punctual = if with_punctual { 1 } else { 0 };
    if with_punctual {
        lights.push(GoldenLight::point([0.0, 0.0, 1.5], [1.0, 0.9, 0.8], 4000.0, 6.0));
    }
    (GoldenLightHeader::new(1, punctual, exposure), lights)
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
fn mask_zero_renders_sky_when_present_else_passes_base_through() {
    // The post-PBR (commit 8e48f7f) contract for the mask == 0 background arm, mirroring
    // `deferred_pbr.hlsl`'s `if (has_sky) { sky } else { lit = base; }`:
    //   * NO sky light in the scanned L0a block → pass `base` through UNCHANGED, invariant to
    //     the directional/punctual lights AND to `header.exposure` (this arm consumes none of
    //     them — `golden_sky_background` returns `None`, and `pack_rgba(base)` is unscaled).
    //   * a sky light present → paint the analytic sky (scaled by `header.exposure`), NOT `base`.
    //
    // This SUPERSEDES the pre-PBR `mask_zero_passes_base_through_under_any_table`, whose premise
    // — base pass-through under ANY table — 8e48f7f made false in two ways: a sky table now
    // paints a sky (ignoring `base`), and `header.exposure` scales it (so it was NOT
    // table-independent). The genuine, still-true device-free invariant is the CONDITIONAL above.
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
    // The exact bytes the no-sky arm produces: `pack_rgba(attrs.base_rgb / 255)` — computed the
    // SAME way the resolve computes its `base`, so this is a byte-identity reference, not a proxy.
    let base_linear = [
        attrs.base_rgb[0] as f32 / 255.0,
        attrs.base_rgb[1] as f32 / 255.0,
        attrs.base_rgb[2] as f32 / 255.0,
    ];
    let base_passthrough = pack_rgba(base_linear);

    // (1) No sky light: `base` passes through unchanged, whatever the exposure or punctual light.
    for exposure in [1.0_f32, 4.0, 7.0] {
        for &with_punctual in &[false, true] {
            let (header, lights) = no_sky_table(exposure, with_punctual);
            let got = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &header, &lights);
            assert_eq!(
                got, base_passthrough,
                "mask == 0 with NO sky light must pass base through unchanged \
                 (exposure={exposure}, punctual={with_punctual}): got 0x{got:08X} \
                 != 0x{base_passthrough:08X}"
            );
        }
    }

    // (2) A sky light present: the arm PAINTS the sky, so it must differ from the base
    // pass-through. (The exact sky value's equivalence to the constant path is pinned separately
    // by `degenerate_table_is_byte_identical_to_the_constant_path`, which also covers mask == 0.)
    let (header, lights) = degenerate_table(1.0);
    let sky = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &header, &lights);
    assert_ne!(
        sky, base_passthrough,
        "a sky light must paint the analytic sky over the mask == 0 arm, not pass base through \
         (0x{sky:08X})"
    );
}
