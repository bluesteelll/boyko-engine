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

/// The result of comparing a rendered readback against a golden reference within a
/// pixel-space bounding box.
///
/// Bbox-scoped on purpose: a small, localized effect (a denoise pass over a penumbra, a
/// moved shadow edge) must never be averaged into invisibility across the full frame. A
/// whole-image mean once hid a real 6847-px effect behind a 0.12 average and cost ~8 debug
/// cycles chasing a phantom "no-op". Always report BOTH the whole-frame diff AND the
/// in-bbox diff so an effect that lives in a rectangle cannot be masked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BboxDiff {
    /// Largest single-channel absolute difference (`0..=255`) seen inside the bbox.
    pub worst_delta: i32,
    /// Count of texels with ANY RGB channel differing by more than the tolerance.
    pub changed_texels: u32,
    /// Total texels inspected inside the (image-clamped) bbox.
    pub total_texels: u32,
}

/// Compares two tightly-packed RGBA byte buffers (`w x h`, 4 bytes/texel) inside the
/// half-open pixel rect `(x0, y0, x1, y1)` = `[x0, x1) x [y0, y1)`, clamped to the image.
/// Alpha is ignored (RGB only). Returns the worst per-channel delta and the number of
/// texels exceeding `tol`.
///
/// Use alongside a whole-frame diff, never instead of one: the point is to surface a
/// localized change that a frame-wide aggregate would flatten. The bbox is the effect's
/// expected footprint (e.g. the penumbra region for a shadow-denoise change).
pub fn diff_in_bbox(
    golden: &[u8],
    readback: &[u8],
    w: u32,
    h: u32,
    rect: (u32, u32, u32, u32),
    tol: i32,
) -> BboxDiff {
    debug_assert!(golden.len() >= (w * h * 4) as usize, "invariant: golden buffer is w*h*4 bytes");
    debug_assert!(readback.len() >= (w * h * 4) as usize, "invariant: readback buffer is w*h*4 bytes");
    let (x0, y0, x1, y1) = rect;
    let x0 = x0.min(w);
    let x1 = x1.min(w);
    let y0 = y0.min(h);
    let y1 = y1.min(h);

    let mut worst = 0i32;
    let mut changed = 0u32;
    let mut total = 0u32;
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let base = ((y * w + x) * 4) as usize;
            let mut texel_changed = false;
            for c in 0..3 {
                let d = (golden[base + c] as i32 - readback[base + c] as i32).abs();
                if d > worst {
                    worst = d;
                }
                if d > tol {
                    texel_changed = true;
                }
            }
            if texel_changed {
                changed += 1;
            }
            total += 1;
            x += 1;
        }
        y += 1;
    }
    BboxDiff { worst_delta: worst, changed_texels: changed, total_texels: total }
}
