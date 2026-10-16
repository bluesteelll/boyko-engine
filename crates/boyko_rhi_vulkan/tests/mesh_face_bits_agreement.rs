//! **RD6 / D-DS — H6: one face-bit layout, three spellings.**
//!
//! The mesh-face bits ride three carriers (the per-batch host word, the raster push word, the VB
//! instance row's flags) and are spelled in three places: `boyko_rhi::enums` (the host deriving
//! and selecting through them), `boyko_shaderdsl::face` (the generator the shaders come from) and
//! the generated `static const uint MESH_FACE_*` span committed in each opaque producer source.
//! A drift between any two would cull or flip on the wrong bit with every other gate green, so
//! this file pins all three equal — and pins the bits clear of the two bits the push word already
//! uses (bit 0 the instanced arm, bit 1 the VB visible indirection) and of the VB flags' bit 0
//! (occlusion culling).

use std::path::PathBuf;

/// The three producer sources the `mesh_face_consts` span is spliced into.
const PRODUCERS: [&str; 3] = ["forward_opaque.fs.hlsl", "gbuffer_mrt.fs.hlsl", "vb_geom_fetch.hlsli"];

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: {} is committed: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The value of `static const uint <name> = <expr>;` in `src`, where `<expr>` is either `Nu` or
/// `1u << <OTHER>` (the only two shapes the generated span uses).
fn hlsl_const(src: &str, name: &str) -> u32 {
    let prefix = format!("static const uint {name} = ");
    let line = src
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("`{prefix}…` is missing"));
    let expr = line[prefix.len()..].trim_end_matches(';').trim();
    if let Some(shift) = expr.strip_prefix("1u << ") {
        1u32 << hlsl_const(src, shift)
    } else {
        expr.trim_end_matches('u').parse().unwrap_or_else(|e| panic!("`{expr}` is not a uint literal: {e}"))
    }
}

#[test]
fn rhi_edsl_and_hlsl_face_bits_agree() {
    use boyko_rhi::enums::{MESH_FACE_BITS, MESH_FACE_CLASS_SHIFT, MESH_FACE_DOUBLE_SIDED, MESH_FACE_MIRRORED};
    use boyko_shaderdsl::face;

    assert_eq!(MESH_FACE_DOUBLE_SIDED, face::MESH_FACE_DOUBLE_SIDED, "boyko_rhi vs the eDSL: double-sided");
    assert_eq!(MESH_FACE_MIRRORED, face::MESH_FACE_MIRRORED, "boyko_rhi vs the eDSL: mirrored");
    assert_eq!(MESH_FACE_DOUBLE_SIDED, 1 << face::MESH_FACE_DOUBLE_SIDED_SHIFT);
    assert_eq!(MESH_FACE_MIRRORED, 1 << face::MESH_FACE_MIRRORED_SHIFT);
    assert_eq!(MESH_FACE_CLASS_SHIFT, face::MESH_FACE_DOUBLE_SIDED_SHIFT, "the class index starts at the double-sided bit");
    assert_eq!(MESH_FACE_BITS & 0b11, 0, "the face bits stay clear of the push word's bits 0 and 1");
    assert_eq!(MESH_FACE_BITS & 1, 0, "the face bits stay clear of the VB flags' occlusion bit");

    for name in PRODUCERS {
        let src = source(name);
        assert_eq!(hlsl_const(&src, "MESH_FACE_DOUBLE_SIDED"), MESH_FACE_DOUBLE_SIDED, "{name}: double-sided");
        assert_eq!(hlsl_const(&src, "MESH_FACE_MIRRORED"), MESH_FACE_MIRRORED, "{name}: mirrored");
    }
}

/// The parser above must be able to see a wrong literal (anti-vacuity).
#[test]
fn hlsl_const_reads_a_drifted_literal() {
    let drifted = "static const uint MESH_FACE_DOUBLE_SIDED_SHIFT = 5u;\n\
                   static const uint MESH_FACE_DOUBLE_SIDED = 1u << MESH_FACE_DOUBLE_SIDED_SHIFT;\n";
    assert_eq!(hlsl_const(drifted, "MESH_FACE_DOUBLE_SIDED"), 1 << 5);
    assert_ne!(hlsl_const(drifted, "MESH_FACE_DOUBLE_SIDED"), boyko_rhi::enums::MESH_FACE_DOUBLE_SIDED);
}
