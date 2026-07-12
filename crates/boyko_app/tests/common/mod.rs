//! Shared test fixtures for `boyko_app`'s PBR / textured-PBR windowed-eval scenes
//! (mirrors `boyko_render/tests/common/mod.rs`'s `mod common;` crate-test pattern —
//! this directory is NOT auto-discovered as its own integration-test binary).
//!
//! The CANONICAL [`uv_sphere`] / [`floor_plane`] mesh builders, migrated here from
//! `pbr_material_showcase.rs` / `textured_smoke.rs`'s formerly-duplicated local
//! copies (grooming item H). `pbr_showcase.rs` / `grand_showcase_2mat.rs` /
//! `grand_showcase_mvpm.rs` keep their own local, PINNED-GOLDEN `uv_sphere` copies
//! verbatim (see the NOTE comment above each) — do NOT migrate those without a
//! golden re-bless.

#![allow(dead_code)]

use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;

/// Generates a UV-sphere (`stacks` x `slices`) of `radius`, centered at the origin,
/// with outward per-vertex normals, a uniform `color`, spherical UVs (`u =
/// theta/2pi`, `v = phi/pi`), and a generated tangent basis (required for the
/// textured pipeline's tangent-space normal mapping).
///
/// Skips the degenerate pole-fan triangles: at both poles the row collapses to a
/// single 3D point (duplicated vertices carrying distinct `u`), so one of the two
/// triangles per quad has a zero-length edge (zero 3D area). Emitting it poisons the
/// pole-ring tangent basis (dark/smeared pole); a robust UV-sphere builder omits it
/// (mirrors Bevy's sphere primitive) — `generate_tangents`'s own `GEO_AREA_EPS_SQ`
/// guard is the belt to this suspenders.
pub fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let u = j as f32 / slices as f32;
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            // Skip the degenerate pole-fan triangles — see this fn's doc.
            if i != 0 {
                idx.extend_from_slice(&[a, b, a + 1]);
            }
            if i != stacks - 1 {
                idx.extend_from_slice(&[a + 1, b, b + 1]);
            }
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// A large flat floor quad on the XZ plane at y=0, up-facing normal, uniform color, a
/// planar UV, and a generated tangent basis (required for the textured pipeline's
/// tangent-space normal mapping).
pub fn floor_plane(half: f32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let n = [0.0, 1.0, 0.0];
    let mut verts = vec![
        Vertex::new([-half, 0.0, -half], n, color),
        Vertex::new([half, 0.0, -half], n, color),
        Vertex::new([half, 0.0, half], n, color),
        Vertex::new([-half, 0.0, half], n, color),
    ];
    verts[0].uv = [0.0, 0.0];
    verts[1].uv = [1.0, 0.0];
    verts[2].uv = [1.0, 1.0];
    verts[3].uv = [0.0, 1.0];
    let idx = vec![0, 1, 2, 0, 2, 3];
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}
