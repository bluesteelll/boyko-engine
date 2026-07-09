//! Shared, byte-identical test helpers extracted from the two largest G-buffer present
//! test binaries (`window_present_gbuffer.rs`, `sdf_gbuffer_hybrid.rs`).
//!
//! Cargo compiles each integration test as its own crate, so the historical way these two
//! files shared code was verbatim copy-paste. This module lives under `tests/common/` — a
//! path Cargo does NOT treat as its own test binary — and is pulled into each file with
//! `mod common; use common::*;`, so every `#[test]` sees the SAME definition. Only items
//! whose CODE is byte-identical across both files live here (comment/doc-only differences
//! were reconciled to one canonical version); anything whose code diverged stays local to
//! each file.
//!
//! `#![allow(dead_code)]`: each `mod common;` is a separate compilation and not every test
//! binary uses every helper, so an unused `pub` item would otherwise fail `-D warnings`.
#![allow(dead_code)]

use core::ptr::NonNull;
use core::slice;

/// The mesh quad's constant world Z (strictly between the sphere surface and the camera,
/// so the mesh occludes the SDF where they overlap).
pub const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint (the left part of the view in x, full y), so the
/// sphere straddles the quad edge — yielding texels over BOTH / sphere-only / quad-only /
/// neither.
pub const QUAD_X_MIN: f32 = -1.0;
pub const QUAD_X_MAX: f32 = 0.2;
pub const QUAD_Y_MIN: f32 = -1.0;
pub const QUAD_Y_MAX: f32 = 1.0;

/// One vertex: a `Float32x3` position (offset 0), a `Float32x3` world normal (offset 12),
/// and a `Float32x4` color (offset 24). `#[repr(C)]` for the exact 40-byte stride. The
/// per-vertex normal feeds the mesh-MRT producer's G-buffer normal target (the shared
/// `gbuffer_mrt.vs` consumes location 2 — the +Z constant the VS used to bake is gone).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
pub struct SpirvBlob<const N: usize>(pub [u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    pub fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // `'static` blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (the CPU seeds
/// the edit-list header / the UBO before the GPU is told to consume it).
pub fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is at least `words.len() * 4` bytes inside the persistent
        // host-coherent mapping; `dst + i` for `i < words.len()` is in-bounds. No GPU
        // work is in flight yet (the submit/present follows), so the host write is
        // unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// The mesh quad as two triangles spanning the world-XY footprint at world Z [`MESH_Z`].
/// The quad faces the camera (`+Z`), so every vertex carries the outward normal `[0, 0, 1]`.
pub fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let n = [0.0_f32, 0.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], normal: n, color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], normal: n, color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], normal: n, color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], normal: n, color: c };
    [bl, br, tr, bl, tr, tl]
}
