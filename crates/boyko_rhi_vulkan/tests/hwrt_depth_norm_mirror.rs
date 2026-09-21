//! Lane fix/hwrt-shadow-ray-origin — the host↔shader text tripwire for the HWRT resolve's
//! raster-owned-pixel producer test (CPU, no GPU).
//!
//! `deferred_pbr.hlsl`'s `#if HWRT` block decides whether a pixel is RASTER-owned (its `gViewT`
//! is the Euclidean distance the JITTERED raster wrote, so the shadow-ray origin must be
//! re-placed on the raster's jittered ray) or SDF-owned (marched on the b5 ray, exact already)
//! with the bit-exact test `gViewT == md * HWRT_MESH_DEPTH_T_MAX` against the raster depth texel
//! `md` — both producers (`sdf_gbuffer_composite.hlsl`'s `t_mesh = md * mesh_norm`,
//! `viewt_from_depth.comp.hlsl`'s `md * pc.mesh_norm`) write exactly `md * MESH_DEPTH_T_MAX` for
//! a mesh-covered pixel under PERSPECTIVE, a power-of-two multiply that is exact in fp32, and an
//! SDF hit has `t < t_mesh` strictly. The shader spells the normaliser as its OWN `static const`
//! (`ray_gen.hlsli`-style: the HLSL cannot read `compute.rs`), so the value has no machine link
//! to the host's `MESH_DEPTH_T_MAX` other than THIS test: a host retune of the normaliser with
//! the shader constant left behind would silently classify every raster pixel as SDF-owned and
//! re-open the false-shadow defect with no other symptom. Same shape as `cluster_grid_read_bound`'s
//! source grep + `instanced_vs_host_mirror`'s sync pin. Runs unconditionally under
//! `cargo test -p boyko_rhi_vulkan` (no window, no device).

use std::path::PathBuf;

use boyko_rhi_vulkan::compute::{MESH_DEPTH_CLEAR, MESH_DEPTH_T_MAX};

/// The committed `deferred_pbr.hlsl` source with `\r\n` normalised to `\n`: the tree has no
/// `.gitattributes`, so a `core.autocrlf` checkout (the owner's) reads the file with CRLF endings
/// while a Linux checkout reads LF, and the line-terminated needle below must match on both.
fn deferred_pbr_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders").join("deferred_pbr.hlsl");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The shader's `HWRT_MESH_DEPTH_T_MAX` equals the host's `MESH_DEPTH_T_MAX` (64), spelled
/// exactly once, inside the `#if HWRT` block.
#[test]
fn hwrt_mesh_depth_t_max_mirrors_the_host_normaliser() {
    let src = deferred_pbr_source();
    let needle = format!("static const float HWRT_MESH_DEPTH_T_MAX = {MESH_DEPTH_T_MAX:.1};");
    let n = src.matches(&needle).count();
    assert_eq!(
        n, 1,
        "deferred_pbr.hlsl must spell `{needle}` exactly once (found {n}): the HWRT resolve's \
         raster-owned producer test `gViewT == md * HWRT_MESH_DEPTH_T_MAX` mirrors the host's \
         compute::MESH_DEPTH_T_MAX ({MESH_DEPTH_T_MAX}), and a mismatch classifies every raster \
         pixel as SDF-owned (the shadow-ray origin fix silently disarmed)"
    );
    // The constant must sit inside the `#if HWRT` block (before its `#endif`) so the software
    // `.spv` never sees it — the byte-identity discipline of every HWRT-only declaration.
    let hwrt_start = src.find("#if HWRT\n").expect("deferred_pbr.hlsl has an `#if HWRT` block");
    let pos = src.find(&needle).expect("found above");
    let block_end = src[hwrt_start..].find("#endif").map(|i| hwrt_start + i).expect("the block closes");
    assert!(
        hwrt_start < pos && pos < block_end,
        "HWRT_MESH_DEPTH_T_MAX must be declared inside the first `#if HWRT` block (the byte-identity gate)"
    );
}

/// The shader's `HWRT_DEPTH_CLEAR` equals the host's `MESH_DEPTH_CLEAR` (1.0): the `md < clear`
/// half of the producer test — a texel at the clear sentinel has no mesh under it.
#[test]
fn hwrt_depth_clear_mirrors_the_host_sentinel() {
    let src = deferred_pbr_source();
    let needle = format!("static const float HWRT_DEPTH_CLEAR      = {MESH_DEPTH_CLEAR:.1};");
    assert_eq!(
        src.matches(&needle).count(),
        1,
        "deferred_pbr.hlsl must spell `{needle}` exactly once: the raster-owned producer test's \
         `md < HWRT_DEPTH_CLEAR` mirrors compute::MESH_DEPTH_CLEAR ({MESH_DEPTH_CLEAR})"
    );
}

/// The producer test itself is spelled with the mirrored constants (not a literal `64.0`), and
/// the two trace arms take `P_shadow` as their origin — deleting either reopens the defect
/// without a compile error.
#[test]
fn the_trace_origin_is_the_producer_tested_p_shadow() {
    let src = deferred_pbr_source();
    assert_eq!(
        src.matches("(view_t == md * HWRT_MESH_DEPTH_T_MAX)").count(),
        1,
        "the raster-owned producer test must compare gViewT against md * HWRT_MESH_DEPTH_T_MAX"
    );
    assert_eq!(
        src.matches("shadow_ray.Origin = P_shadow + n * SHADOW_RAY_BIAS;").count(),
        2,
        "both tracing arms (RESOLVE_INLINE + VIS) must cast from P_shadow, not P"
    );
    assert_eq!(
        src.matches("shadow_ray.Origin = P + n * SHADOW_RAY_BIAS;").count(),
        0,
        "no tracing arm may cast from the b5-reconstructed P"
    );
}
