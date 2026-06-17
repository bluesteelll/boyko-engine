//! Phase-6 rung-10 acceptance test: SDF + MESH HYBRID COMPOSITE via a SHARED
//! DEPTH buffer (the SDF-doc §15.1 seam, the Slice-S2 occlusion-acceptance core).
//!
//! This proves the §15.1 shared-depth seam end-to-end, OFFSCREEN, on real Vulkan:
//! a REAL GPU-rasterized mesh's depth BOUNDS the GPU SDF sphere-trace march, so
//! the mesh and the SDF OCCLUDE EACH OTHER correctly, composited into one image.
//! It composes the proven rung-4 depth-attachment graphics path (rasterize a quad
//! into a D32_SFLOAT depth attachment) with the rung-9 edit-list compute
//! sphere-trace, joined by a NEW cross-stage hazard: depth-write → image→buffer
//! copy → compute-read over ONE shared storage buffer.
//!
//! # The deterministic scene (single source of truth in `compute.rs`)
//!
//! - **SDF**: a single UNION sphere at the origin, radius 0.5 (rung 9's
//!   `base_only`). Its nearest surface along the center ray is at `t ≈ CAM_Z - 0.5
//!   = 1.5`.
//! - **Mesh**: a fronto-parallel quad at constant world `Z = MESH_Z = 1.0`,
//!   covering a KNOWN screen sub-rectangle (the left part of the image in x, full
//!   y). `t_mesh = CAM_Z - MESH_Z = 1.0 < 1.5`, so OVER the sphere the mesh is IN
//!   FRONT (it occludes the SDF there).
//!
//! # The orthographic depth convention (the one subtle correctness point)
//!
//! The quad is rendered with an orthographic projection chosen so the stored depth
//! equals the normalized ray parameter `t / T_MAX` (`t = CAM_Z - worldZ`): near
//! plane `worldZ = CAM_Z` → depth 0, far plane `worldZ = CAM_Z - T_MAX` → depth 1,
//! Vulkan clip-space depth `[0, 1]`, the SAME y-flip the SDF camera uses. Because
//! the projection is orthographic there is no perspective divide, so depth is
//! exactly linear in `t` and the mapping is exact for a fronto-parallel surface.
//! The host mirror is `compute::depth_to_t` / `mesh_depth_for_z`.
//!
//! # The four discriminator texels (picked host-side, BEFORE any GPU run)
//!
//! - **A (mesh occludes SDF):** over BOTH the quad AND the sphere → `MESH_COLOR`.
//!   LOAD-BEARING: only `MESH_COLOR` if the depth bound actually clipped the march.
//! - **B (SDF shows):** over the sphere, NOT the quad → the SDF lit color.
//! - **C (mesh on background):** over the quad, NOT the sphere → `MESH_COLOR`.
//! - **D (background):** neither → `BACKGROUND`.
//!
//! Each is asserted against the host golden ([`golden_composite_pixel`]) within
//! the same `+/-2/255` per-channel tolerance as rung 9, plus a depth-region check
//! and pairwise-distinctness invariants.
//!
//! # The oracle (plan §6, mirrored from `sdf_editlist.rs` / `graphics_depth.rs`)
//!
//! Boots with validation enabled and asserts `debug_state().total() == 0` after
//! the run. A GPU-less / loader-less / validation-layer-less host makes
//! `VulkanContext::boot` return `Err`; the test skips gracefully.

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BarrierDesc, BufferBarrier, BufferDesc, BufferImageCopy, BufferUsage, ComputePipelineDesc,
    DepthAttachment, Format, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout,
    ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation, PrimitiveTopology, RenderArea,
    RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_BUFFER_WORDS, COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PIXEL_BASE_WORDS, LOCAL_SIZE_X,
    MESH_COLOR, MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_IMG_H, SDF_IMG_W, SDF_TRACE_T_MAX,
    SDF_VIEW_HALF_EXTENT, SdfEdit, editlist_pixel_hits, encode_edit_list, golden_composite_pixel,
    mesh_depth_for_z, pixel_world_xy, sdf_depth_composite_spirv, sdf_op,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Total pixel count (the compute push constant; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9): DXC
/// `mad`/`fma` rounding makes a bit-exact match brittle; `+/-2/255` still proves
/// the lit SDF surface / flat mesh / background colors apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// Tolerance on the depth-region check (a stored D32 float vs the host-computed
/// constant). A fronto-parallel ortho surface should be exact, but FMA in the MVP
/// multiply can move it by under one ULP at this magnitude; a tiny epsilon absorbs
/// that while a wrong mapping misses by `>= 0.05`.
const DEPTH_TOL: f32 = 1.0e-4;

/// The mesh quad's constant world Z. Chosen strictly between the sphere's nearest
/// surface (`worldZ = +0.5`, `t = 1.5`) and the camera (`worldZ = CAM_Z = 2.0`),
/// so OVER the sphere the mesh is in front (`t_mesh = 2.0 - 1.0 = 1.0 < 1.5`).
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint corners: the left part of the view in x
/// (`[-1.0, +0.2]`), full y (`[-1.0, +1.0]`). Mapped to pixels via
/// [`pixel_world_xy`], this covers roughly the left 60% of the image. The sphere
/// (radius 0.5, world-x in `[-0.5, +0.5]`) straddles the `x = +0.2` quad edge, so
/// there are pixels over BOTH (texel A), over the sphere only (texel B, `x` in
/// `(0.2, 0.5]`), over the quad only (texel C), and over neither (texel D).
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane; an uncovered pixel keeps
/// this, decoded host-side as "no mesh"). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// A throwaway color-attachment format. The graphics pipeline requires a non-empty
/// `color_formats`, so the quad is rendered into a 1-color + depth scope; only the
/// DEPTH result is consumed (the color image is never read back).
const COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset
/// 12), the rung-3/4 vertex layout reused. `#[repr(C)]` so the field layout is the
/// exact 28-byte stride the layout declares. The color is unused by the composite
/// (the mesh albedo is a constant), but the rung-3 fragment shader reads it.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// The MVP byte size (a `float4x4`).
const MVP_BYTES: u32 = 64;

// --- Committed rung-3 SPIR-V, reused unchanged for the mesh raster (the depth
//     comes from `gl_Position.z`, so the MVP vertex + a constant-color fragment
//     suffice — NO new graphics shader). Embedded at compile time. ---

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const
        // u32`; `N` is a 4-byte multiple (const-asserted above), so the blob is
        // exactly `N / 4` whole `u32` words; the `&self` borrow keeps the `'static`
        // blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`, 916 bytes), reused.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP
/// log) when no GPU / loader / validation layer / dynamic-rendering is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages (the GPU-half oracle).
fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the hybrid-composite run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// The orthographic MVP for the rung-3 vertex shader, uploaded COLUMN-MAJOR. It
/// maps a fronto-parallel world-space vertex to clip space so that, for `w == 1`
/// and viewport depth `[0, 1]`, the stored depth equals `t / T_MAX` with
/// `t = CAM_Z - worldZ`:
///
/// - `x_clip = world_x / HALF_EXTENT`     (world-x `[-H, +H]` → NDC `[-1, +1]`,
///   matching the SDF ray's `ro.x = u * HALF_EXTENT`).
/// - `y_clip = -world_y / HALF_EXTENT`    (Vulkan NDC y points DOWN; the minus
///   places +world_y at the top, matching the SDF camera's `v = -(...)` flip so the
///   raster footprint lines up pixel-for-pixel with the SDF rays — NOT double-
///   flipped, see [`pixel_world_xy`]).
/// - `z_clip = (CAM_Z - world_z) / T_MAX` (depth 0 at the near plane `worldZ =
///   CAM_Z`, depth 1 at the far plane `worldZ = CAM_Z - T_MAX`).
/// - `w_clip = 1`                         (orthographic — no perspective divide).
///
/// So the INTENDED transform `clip = M * p` (`p = (x, y, z, 1)`) has rows:
///   row0 = [1/H,    0,        0,        0      ]
///   row1 = [0,     -1/H,      0,        0      ]
///   row2 = [0,      0,       -1/T_MAX,  CAM_Z/T_MAX]
///   row3 = [0,      0,        0,        1      ]
///
/// # VERIFIED upload convention — the upload is the TRANSPOSE of `M` (column-major)
///
/// DXC lowers the rung-3 HLSL `mul(pc.mvp, float4(pos, 1))` to SPIR-V
/// `OpVectorTimesMatrix(p, mvp)` with the matrix decorated `RowMajor` (DXC's
/// documented default: HLSL `column_major` ↦ SPIR-V `RowMajor`). Under that
/// execution model the GPU computes the j-th clip component as
/// `clip[j] = dot(p, "GPU-column j")`, where "GPU-column j" reads the uploaded
/// floats at the stride-4 positions `{j, j+4, j+8, j+12}`. For that to equal the
/// intended `clip[j] = dot(M_row_j, p)`, "GPU-column j" must equal `M`'s row `j`,
/// i.e. `upload[r*4 + c] = M[c][r]`: the 16 floats are `M` TRANSPOSED, laid out
/// row-major. (Verified against the KNOWN-CORRECT rungs 3/4, whose MVPs are
/// DIAGONAL — transpose is a no-op there, which is exactly why a transposed upload
/// was invisible until this first ASYMMETRIC matrix, whose only off-diagonal term
/// `CAM_Z/T_MAX` is the one that breaks.)
///
/// A NAIVE row-major upload of `M` (the bug this replaces) makes the GPU compute
/// `Mᵀ * p`: for a world-Z vertex (`z = MESH_Z = 1`, `H = 1`, `T_MAX = 10`,
/// `CAM_Z = 2`) that yields `clip = (x, -y, -0.1, 1.2)` → `z_clip = -0.1 < 0`
/// fails Vulkan's near clip (`0 ≤ z_clip ≤ w_clip`) → the WHOLE quad is clipped →
/// no depth written. The correct transposed upload gives `clip = (x, -y, 0.1,
/// 1.0)`, stored depth `0.1` = [`mesh_depth_for_z`]`(MESH_Z)`. The depth-region
/// assertion (a covered texel reads `0.1`, not the `1.0` clear) is the CANARY for a
/// transpose — if a future MVP regresses to a naive row-major upload, that
/// assertion fails first. DO NOT "simplify" this to a row-major copy.
///
/// `mt` below is `Mᵀ` written out directly in upload order (each group of 4 is a
/// ROW of `Mᵀ` = a COLUMN of `M`), so the only off-diagonal term `CAM_Z/T_MAX`
/// moves from `M[2][3]` to `mt[3*4 + 2]` (the 15th float).
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    // Mᵀ in row-major upload order: mt[r*4 + c] = M[c][r]. Each group of 4 is a
    // COLUMN of `M`. The GPU's `clip[j] = dot(p, {mt[j], mt[j+4], mt[j+8],
    // mt[j+12]})` then evaluates the intended `clip = M * p`.
    let mt: [f32; 16] = [
        // M column 0
        1.0 / h, 0.0,      0.0,          0.0,
        // M column 1
        0.0,     -1.0 / h, 0.0,          0.0,
        // M column 2
        0.0,     0.0,      -1.0 / tmax,  0.0,
        // M column 3
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The mesh quad as two triangles (six vertices) spanning the world-XY footprint
/// `[QUAD_X_MIN, QUAD_X_MAX] × [QUAD_Y_MIN, QUAD_Y_MAX]` at constant world Z
/// [`MESH_Z`]. The per-vertex color is arbitrary (unused by the composite). The
/// two triangles are `(bl, br, tr)` and `(bl, tr, tl)` — a standard CCW quad.
fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], color: c };
    [bl, br, tr, bl, tr, tl]
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad's
/// world-XY footprint. For a fronto-parallel axis-aligned quad, the ray (a column
/// at fixed world-xy) hits the quad iff its world-xy lies within the corner rect —
/// so this predicate is exactly the rasterizer's covered-pixel set, host-computable
/// from the SAME camera mapping the golden uses ([`pixel_world_xy`]).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh DEPTH the GPU is expected to produce: the constant
/// [`mesh_depth_for_z`]`(MESH_Z)` inside the quad footprint, the clear value
/// (`1.0`) outside. This is the host model the depth-region assertion checks
/// against and the input to [`golden_composite_pixel`].
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// The base-sphere SDF scene (one union sphere, origin, r=0.5) — the rung-9
/// `base_only` field, reused as the recognizable SDF body the mesh occludes.
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (valid
/// before the submit — the CPU seeds the edit-list header here).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is `COMPOSITE_BUFFER_WORDS * 4` bytes inside the
        // persistent host-coherent mapping; `dst + i` for `i < words.len() <=
        // COMPOSITE_BUFFER_WORDS` is in-bounds. No GPU work is in flight yet (the
        // submit happens after this), so the host write is unsynchronized-safe.
        // `write_unaligned` tolerates the sub-allocated offset's alignment.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Reads `PIXELS` packed-RGBA `u32`s from the buffer's PIXEL region, valid only
/// after a fence-waited submit.
fn read_pixels(base: NonNull<u8>) -> Vec<u32> {
    let n = PIXELS as usize;
    let mut out = Vec::with_capacity(n);
    let base = base.as_ptr().cast::<u32>();
    for i in 0..n {
        // SAFETY: the buffer is `COMPOSITE_BUFFER_WORDS * 4` bytes inside the
        // persistent host-coherent mapping; `COMPOSITE_PIXEL_BASE_WORDS + i` for
        // `i < n` is in-bounds (`COMPOSITE_PIXEL_BASE_WORDS + n ==
        // COMPOSITE_BUFFER_WORDS`). A fence wait preceded this read, so the GPU
        // writes are complete + coherent.
        let v = unsafe { base.add(COMPOSITE_PIXEL_BASE_WORDS + i).read_unaligned() };
        out.push(v);
    }
    out
}

/// Reads `PIXELS` `f32` mesh-depth values from the buffer's DEPTH region (written
/// by the GPU image→buffer copy), valid only after a fence-waited submit.
fn read_depth(base: NonNull<u8>) -> Vec<f32> {
    let n = PIXELS as usize;
    let mut out = Vec::with_capacity(n);
    let base = base.as_ptr().cast::<u32>();
    for i in 0..n {
        // SAFETY: the DEPTH region is `[COMPOSITE_DEPTH_BASE_WORDS,
        // COMPOSITE_PIXEL_BASE_WORDS)` — `n` words, in-bounds. A fence wait
        // preceded this read (the depth-write → copy → compute chain completed), so
        // the bytes are the final D32_SFLOAT depths, complete + coherent. Any bit
        // pattern is a valid `u32`; `f32::from_bits` reinterprets it as the stored
        // depth float.
        let bits = unsafe { base.add(COMPOSITE_DEPTH_BASE_WORDS + i).read_unaligned() };
        out.push(f32::from_bits(bits));
    }
    out
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Asserts two packed colors agree within `CHANNEL_TOL` per RGB channel.
fn assert_color_close(got: u32, want: u32, label: &str) {
    let g = unpack_rgb(got);
    let w = unpack_rgb(want);
    for c in 0..3 {
        assert!(
            (g[c] - w[c]).abs() <= CHANNEL_TOL,
            "{label}: channel {c} off by {} (got {:#010x} -> {:?}, want {:#010x} -> {:?}, tol {CHANNEL_TOL})",
            (g[c] - w[c]).abs(),
            got,
            g,
            want,
            w,
        );
    }
}

/// `true` if two packed colors agree within `CHANNEL_TOL` per RGB channel.
fn colors_close(a: u32, b: u32) -> bool {
    let x = unpack_rgb(a);
    let y = unpack_rgb(b);
    (0..3).all(|c| (x[c] - y[c]).abs() <= CHANNEL_TOL)
}

/// Records + submits the full hybrid composite in ONE command buffer / ONE fenced
/// submit and returns `(pixels, depth)`: the packed-RGBA composite output and the
/// shared mesh-depth region. The flow is the §15.1 seam end-to-end:
///   raster quad → depth attachment → copy depth into the shared buffer →
///   transfer→compute barrier → SDF sphere-trace bounded by that depth.
fn run_hybrid(ctx: &VulkanContext, edits: &[SdfEdit]) -> (Vec<u32>, Vec<f32>) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // The ONE shared storage buffer (header + edit array + depth region + pixels).
    // `STORAGE | TRANSFER_DST`: the compute shader reads/writes it AND the depth
    // image→buffer copy writes its DEPTH region.
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (COMPOSITE_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("shared composite storage buffer");

    // Seed the edit-list header BEFORE submit (the depth + pixel regions are GPU-
    // written, so they are left zero here).
    {
        let mut header = vec![0u32; COMPOSITE_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // The depth image (D32_SFLOAT) — usage DEPTH_STENCIL_ATTACHMENT (rasterize into
    // it) | TRANSFER_SRC (copy it into the shared buffer).
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::TRANSFER_SRC,
        })
        .expect("offscreen depth texture");

    // A throwaway color attachment (the graphics pipeline requires a non-empty
    // color set; its result is never consumed).
    let color = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: COLOR_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT,
        })
        .expect("throwaway color texture");

    // The quad vertex buffer (host-visible).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = device
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    let vb_ptr = device
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the
    // buffer was created at exactly that size); `vertices` is a distinct,
    // non-overlapping stack array of `vertex_bytes` bytes; the write completes
    // before any submit references the buffer (host-coherent: no explicit flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // The mesh-raster (vertex + fragment) modules + the SDF composite (compute).
    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");
    let cs = device
        .create_shader_module(sdf_depth_composite_spirv())
        .expect("composite compute shader module");

    // The depth-testing graphics pipeline: rung-3 vertex layout + the 64-byte
    // VERTEX MVP push range + a declared depth_format (enables depth test/write +
    // compareOp LESS). Both attachment formats equal the images' formats (W2-b).
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 12, format: VertexFormat::Float32x4 },
    ];
    let gfx = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
        })
        .expect("depth-testing graphics pipeline");

    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: 4,
        })
        .expect("composite compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // --- Mesh raster pass: clear depth to the far plane, rasterize the quad. ---
    // Color: UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (the throwaway target).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &color,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    // Depth: UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL (early/late fragment-test stage +
    // depth-write access, DEPTH aspect).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    let full = RenderArea { x: 0, y: 0, width: SDF_IMG_W, height: SDF_IMG_H };
    let color_attachment = [RenderingAttachment {
        texture: &color,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: [0.0, 0.0, 0.0, 1.0],
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &color_attachment,
        depth: Some(DepthAttachment {
            texture: &depth,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: DEPTH_CLEAR,
        }),
    });
    encoder.bind_graphics_pipeline(&gfx);
    encoder.push_graphics_constants(&gfx, ShaderStage::VERTEX, 0, &ortho_mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: SDF_IMG_W as f32,
        height: SDF_IMG_H as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0); // two triangles = the quad
    encoder.end_rendering();

    // --- Depth → shared buffer. First a layout transition for the copy source. ---
    // Depth: DEPTH_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL. The depth WRITES
    // happen at the LATE_FRAGMENT_TESTS stage; the copy READS at TRANSFER. This
    // image barrier makes the depth-write available + visible to the transfer read
    // and transitions the layout (over-synchronized via both fragment-test stages).
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    // Copy the DEPTH aspect into the shared buffer's DEPTH region (one f32/pixel).
    let regions = [BufferImageCopy {
        buffer_offset: (COMPOSITE_DEPTH_BASE_WORDS as u64) * 4,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::DEPTH,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&depth, ImageLayout::TransferSrcOptimal, &buffer, &regions);

    // --- THE load-bearing transfer → compute hazard over the shared buffer. ---
    // The image→buffer copy WROTE the DEPTH region (TRANSFER stage, TRANSFER_WRITE);
    // the compute shader READS it (COMPUTE_SHADER stage, SHADER_READ). Raw Vulkan
    // inserts NOTHING automatically, so without this buffer barrier the dispatch
    // could read stale/partial depth. Superset-correct: src = full transfer write,
    // dst = full shader read.
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        buffers: &[BufferBarrier {
            buffer: &buffer,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        }],
    });

    // --- SDF composite compute pass: march bounded by the shared mesh depth. ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &PIXELS.to_ne_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let pixels = read_pixels(mapped);
    let depths = read_depth(mapped);
    assert_eq!(pixels.len(), PIXELS as usize);
    assert_eq!(depths.len(), PIXELS as usize);

    assert_validation_clean(ctx);

    // SAFETY: every resource below was created on `device` and is destroyed exactly
    // once; the last submission completed (fence-waited above), so none is in use
    // by the GPU. The depth/color textures tear down view → image → memory.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(compute);
        device.destroy_graphics_pipeline(gfx);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_texture(color);
        device.destroy_texture(depth);
        device.destroy_buffer(buffer);
    }

    (pixels, depths)
}

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)`. Returns
/// `None` if the scene has no such pixel (the test then fails with a clear
/// invariant message at the call site).
fn find_texel(edits: &[SdfEdit], pred: impl Fn(bool, bool) -> bool) -> Option<(u32, u32)> {
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let hit = editlist_pixel_hits(edits, px, py);
            let covered = mesh_covers_pixel(px, py);
            if pred(hit, covered) {
                return Some((px, py));
            }
        }
    }
    None
}

/// Rung 10 — the SDF+mesh hybrid composite via shared depth. A real rasterized
/// quad's depth bounds the SDF march so the two occlude each other:
/// - texel A (sphere ∧ quad) → MESH_COLOR (mesh in front, the depth bound clipped
///   the march — the load-bearing occlusion proof),
/// - texel B (sphere ∧ ¬quad) → the SDF lit color,
/// - texel C (¬sphere ∧ quad) → MESH_COLOR,
/// - texel D (¬sphere ∧ ¬quad) → BACKGROUND.
#[test]
fn sdf_mesh_hybrid_shared_depth() {
    let Some(ctx) = boot_or_skip("sdf_mesh_hybrid_shared_depth") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let sdf = sphere_scene();

    // Pick the four discriminator texels host-side, BEFORE any GPU run, so the
    // assertions are independent of the GPU and prove the composite is correct.
    let (ax, ay) = find_texel(&sdf, |hit, covered| hit && covered)
        .expect("invariant: some pixel must be over BOTH the sphere and the quad (texel A)");
    let (bx, by) = find_texel(&sdf, |hit, covered| hit && !covered)
        .expect("invariant: some pixel must be over the sphere but NOT the quad (texel B)");
    let (cx, cy) = find_texel(&sdf, |hit, covered| !hit && covered)
        .expect("invariant: some pixel must be over the quad but NOT the sphere (texel C)");
    let (dx, dy) = find_texel(&sdf, |hit, covered| !hit && !covered)
        .expect("invariant: some pixel must be over neither (texel D)");

    // The host-expected per-texel colors (the single source of truth). Texel A
    // proves occlusion ONLY because the depth bound clips the march: the sphere is
    // hit there (t≈1.5) but the mesh is nearer (t_mesh=1.0), so the golden returns
    // MESH_COLOR — and so must the GPU.
    let depth_at = |px, py| expected_mesh_depth(px, py);
    let a_want = golden_composite_pixel(&sdf, depth_at(ax, ay), ax, ay);
    let b_want = golden_composite_pixel(&sdf, depth_at(bx, by), bx, by);
    let c_want = golden_composite_pixel(&sdf, depth_at(cx, cy), cx, cy);
    let d_want = golden_composite_pixel(&sdf, depth_at(dx, dy), dx, dy);

    // Pairwise-distinct invariant: MESH_COLOR (texel A/C), the SDF lit color (B),
    // and BACKGROUND (D) must differ beyond the tolerance so the regions are
    // unambiguous. (A and C are both MESH_COLOR — they must AGREE.)
    let mesh_packed = boyko_rhi_vulkan::compute::pack_rgba(MESH_COLOR);
    assert_color_close(a_want, mesh_packed, "texel A golden == MESH_COLOR");
    assert_color_close(c_want, mesh_packed, "texel C golden == MESH_COLOR");
    assert!(
        !colors_close(a_want, b_want),
        "invariant: MESH_COLOR (texel A) and the SDF lit color (texel B) must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !colors_close(a_want, d_want),
        "invariant: MESH_COLOR (texel A) and BACKGROUND (texel D) must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !colors_close(b_want, d_want),
        "invariant: the SDF lit color (texel B) and BACKGROUND (texel D) must differ beyond +/-{CHANNEL_TOL}"
    );

    let (pixels, depths) = run_hybrid(&ctx, &sdf);
    let idx = |px: u32, py: u32| (py * SDF_IMG_W + px) as usize;

    // (a) Depth-region check — localizes any raster/ortho-MVP bug before the
    //     composite assertions. Inside the quad the stored depth must be the
    //     constant mesh depth; at the outside texel D it must be the clear (1.0).
    //     The covered `0.1` vs clear `1.0` are separated by a 0.9 margin, far
    //     wider than any FMA wobble — which is why the composite's strict `md <
    //     MESH_DEPTH_CLEAR` (`< 1.0`) sentinel is robust without an epsilon: a
    //     transposed MVP that left the depth at the `1.0` clear (the C1 bug) trips
    //     this check by ~0.9, not by one ULP.
    let want_mesh_depth = mesh_depth_for_z(MESH_Z);
    let got_a_depth = depths[idx(ax, ay)];
    assert!(
        (got_a_depth - want_mesh_depth).abs() <= DEPTH_TOL,
        "depth-region: covered texel A ({ax},{ay}) depth {got_a_depth} != expected {want_mesh_depth} (tol {DEPTH_TOL})"
    );
    let got_c_depth = depths[idx(cx, cy)];
    assert!(
        (got_c_depth - want_mesh_depth).abs() <= DEPTH_TOL,
        "depth-region: covered texel C ({cx},{cy}) depth {got_c_depth} != expected {want_mesh_depth} (tol {DEPTH_TOL})"
    );
    let got_d_depth = depths[idx(dx, dy)];
    assert!(
        (got_d_depth - DEPTH_CLEAR).abs() <= DEPTH_TOL,
        "depth-region: uncovered texel D ({dx},{dy}) depth {got_d_depth} != clear {DEPTH_CLEAR} (tol {DEPTH_TOL})"
    );

    // (b) Composite golden at the four texels (+/-2/255 per channel).
    let a_got = pixels[idx(ax, ay)];
    let b_got = pixels[idx(bx, by)];
    let c_got = pixels[idx(cx, cy)];
    let d_got = pixels[idx(dx, dy)];
    assert_color_close(a_got, a_want, "texel A (sphere ∧ quad → MESH occludes SDF)");
    assert_color_close(b_got, b_want, "texel B (sphere ∧ ¬quad → SDF lit)");
    assert_color_close(c_got, c_want, "texel C (¬sphere ∧ quad → MESH on background)");
    assert_color_close(d_got, d_want, "texel D (¬sphere ∧ ¬quad → BACKGROUND)");

    // (c) The occlusion actually changed the pixel: texel A (mesh occludes the
    //     sphere) and texel B (sphere shows) are BOTH over the sphere, yet must
    //     differ — A is the mesh, B is the lit SDF. If the depth bound had been
    //     ignored, A would be the SDF lit color too and this would fail.
    assert!(
        !colors_close(a_got, b_got),
        "the GPU mesh-occluded texel A and the SDF-visible texel B (both over the sphere) must differ — proving the shared depth bound clipped the march"
    );
    // texel A and texel C are both the mesh — they must agree.
    assert_color_close(a_got, c_got, "texel A vs texel C (both MESH_COLOR)");

    drop(ctx);
}
