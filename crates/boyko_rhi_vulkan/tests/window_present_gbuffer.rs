//! Render **P1c GPU gate** (scaffold) — the FIRST IMAGE-BASED HYBRID FRAME ON SCREEN.
//! The P1b shared-depth marcher (a real GPU-rasterized mesh's depth written into a
//! D32_SFLOAT IMAGE, SAMPLED directly by the SDF compute marcher, which STORES the
//! FINAL composite into an R8G8B8A8 ALBEDO storage image) is driven ON SCREEN and the
//! ALBEDO is present-blit (fullscreen-sample) into the windowed swapchain image, with
//! the validation layer as the soundness oracle. On ONE presented frame — before
//! present — the swapchain image is copied back into a host-visible staging buffer and
//! golden-asserted against the host composite truth ([`golden_composite_pixel`]).
//!
//! # The P1c graduation: image-based, NO depth→buffer copy
//!
//! This is the on-screen counterpart of the P1b OFFSCREEN driver
//! (`tests/sdf_gbuffer_hybrid.rs::run_gbuffer_hybrid`). Where the packed on-screen path
//! (`window_present_hybrid.rs`) copies the rasterized depth into a shared buffer
//! (`copy_image_to_buffer(depth)`) and the marcher reads it as a packed buffer, the P1c
//! path SAMPLES the depth IMAGE and STORES the composite into the ALBEDO IMAGE — the
//! per-frame depth→buffer copy is GONE. The single depth
//! `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier replaces the packed
//! path's depth copy + its two transfer barriers; the descriptor sets are written ONCE
//! at [`Renderer::render_gbuffer_frame`]'s first sync (NO per-frame
//! `vkUpdateDescriptorSets`).
//!
//! Determinism (INVIOLABLE): the marcher (field eval + ray-gen + lighting + the albedo
//! composite) is BYTE-UNTOUCHED from P1b (the verbatim `sdf_gbuffer_composite_spirv`),
//! so the on-screen image-composite golden equals the packed on-screen composite within
//! `+/-2/255` per channel — the SAME host golden the packed path uses.
//!
//! # 1:1 top-left present (WSI-clamp safe)
//!
//! The composite is rendered at its NATIVE 64×64 size; [`Renderer::render_gbuffer_frame`]
//! present-blits it 1:1 in the swapchain image's TOP-LEFT (it clamps the present
//! viewport/scissor to `min(swapchain_extent, present_extent)`), so the per-texel golden
//! is exact regardless of a WSI `current_extent` clamp, as long as the swapchain is at
//! least 64×64 (the top-left sub-rect fits). The G-buffer + marcher dispatch are sized
//! to the 64×64 composite, NOT the (possibly wider) swapchain extent.
//!
//! # The discriminator texels (picked host-side, BEFORE any GPU run)
//!
//! The same three rung-10/P1b regions: a mesh-occludes-SDF texel (`MESH_COLOR`), an SDF
//! lit texel, and a background texel — each asserted color-close to
//! [`golden_composite_pixel`] within `+/-2/255`, accounting for the swapchain being
//! `B8G8R8A8` (the readback bytes are then BGRA; the golden is RGBA byte order).
//!
//! # SCAFFOLD STATUS — the GPU run is the TESTER's
//!
//! This file compiles + [`Renderer::render_gbuffer_frame`] records the full P1c stream,
//! but the golden GPU assertion is gated behind a graceful boot/WSI/format SKIP and a
//! `#[cfg(windows)]` (it needs a real RTX-3060 windowed device). The tester: run it on
//! the GPU, confirm the presented swapchain image matches the rung-10/P1b hybrid golden
//! within `+/-2/255`, confirm — by recording inspection — that NO
//! `copy_image_to_buffer(depth)` and NO per-frame `vkUpdateDescriptorSets` are in the
//! stream, and confirm validation + sync-validation are clean.

#![cfg(windows)]

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::enums::{AddressMode, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc, BufferUsage, ComputePipelineDesc,
    Format, GraphicsPipelineDesc, MemoryLocation, MipMode, PrimitiveTopology, RhiDevice,
    SamplerDesc, ShaderStage, VertexAttribute, VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::compute::{
    B5_CAMERA_UBO_BYTES_M4, COMPOSITE_PUSH_CONSTANT_BYTES, CoarseMode, CompositePushConstants,
    EDITLIST_BUFFER_WORDS, GOLDEN_LIGHT_HEADER_BASE_WORDS, GoldenLight, GoldenLightHeader,
    LOCAL_SIZE_X, M2_GRID_PARAMS_OFFSET,
    MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_TRACE_T_MAX,
    SDF_VIEW_HALF_EXTENT, SdfEdit, TILE_BOUND_BYTES, CompositeCamera,
    encode_edit_list, deferred_pbr_spirv, golden_composite_pixel_ex, golden_deferred_resolve,
    golden_marcher_attributes, composite_pixel_ray, GoldenMaterial, DEFAULT_MARCHER_OMEGA,
    LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, DEFAULT_LIGHT_DIR, mesh_depth_for_z,
    sdf_gbuffer_composite_spirv, sdf_op, sdf_ssao_spirv_variant, sdf_tile_cull_spirv,
    tile_grid_extent,
    // Render P7-Q2: the SSAO quality-variant indices the ladder showcase selects between.
    SSAO_QUALITY_LOW, SSAO_QUALITY_MEDIUM, SSAO_QUALITY_HIGH,
};
use boyko_rhi_vulkan::brick_atlas::BrickClipmap;
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D,
};
use boyko_rhi_vulkan::swapchain::{
    BrickActivation, GBUFFER_MVP_BYTES, GBufferFrame, GBufferScene, Renderer, SsaoActivation,
    Surface, Swapchain,
};
use boyko_rhi_vulkan::window::{CapturedMsg, Window};

use boyko_sdf_math::brick::{BRICK_LEVELS, PointerGrid};

/// The composite extent — the size the G-buffer is allocated at, the marcher dispatches
/// at, and the depth-prepass rasterizes at, present-blit 1:1 into the swapchain's top-left.
///
/// DECOUPLED from the frozen `SDF_IMG_W`/`SDF_IMG_H` (64×64): the ORTHO camera maps
/// `u, v ∈ [-1, 1]` → world `[-SDF_VIEW_HALF_EXTENT, +SDF_VIEW_HALF_EXTENT]` *regardless of
/// resolution*, so a larger extent keeps the SAME framing (the r=0.5 sphere stays centered,
/// occupying the central ~half of the view, with the mesh quad over the left part) and only
/// raises the sample density. The golden is recomputed at this extent via the extent-aware
/// `golden_*` oracles (`golden_composite_pixel_ex` / `golden_marcher_attributes`), so it
/// re-blesses automatically — the frozen field, the offscreen tests, and the brick path are
/// all untouched (this test simply marches the same field at a finer grid). 512×512 is large
/// enough for the owner to evaluate the brick-ON vs analytic A/B by eye; the whole sphere is
/// visible with margin and the occluding quad is clearly distinguishable.
const COMPOSITE_W: u32 = 512;
const COMPOSITE_H: u32 = 512;

/// The window's client size — the swapchain is created at the composite extent so the
/// 1:1 top-left present fills the whole window. The WSI may clamp it wider/narrower.
const WIDTH: u32 = COMPOSITE_W;
const HEIGHT: u32 = COMPOSITE_H;

/// Total pixel count (the marcher's dispatch element count; the shader bounds `idx < count`).
const PIXELS: u32 = COMPOSITE_W * COMPOSITE_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10 / P1b): DXC
/// `mad`/`fma` rounding + the float→UNORM store + the sample round-trip make a bit-exact
/// match brittle; `+/-2/255` still proves the lit SDF / flat mesh / background apart.
const CHANNEL_TOL: i32 = 2;

/// The mesh quad's constant world Z (strictly between the sphere surface and the camera,
/// so the mesh occludes the SDF where they overlap). Mirrors the packed/P1b `MESH_Z`.
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint (the left part of the view in x, full y), so the
/// sphere straddles the quad edge. Mirrors the packed/P1b footprint.
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// The brick-cache activation state the present STARTS in. `true` boots brick-ON (empty-skip +
/// trilinear/cubic surface cache + the 3-level clip-map) so the owner sees the activated path
/// immediately; the 'B' key flips it live for an A/B comparison against the analytic marcher. Flip
/// this to `false` to boot in the analytic (OFF) state instead. The brick path is RTX-verified
/// byte-identical to analytic in this small origin-centered scene, so the on-screen image must look
/// IDENTICAL either way (the toggle proves the brick render == analytic, just faster).
const BRICK_START_ON: bool = true;

/// Win32 `WM_KEYDOWN` (`0x0100`) — the message the toggle watches for in the captured input ring
/// (matched numerically; `boyko_rhi_vulkan::window`'s OS constants are private, but the renderer
/// captures the verbatim `(msg, wparam, lparam)` triple, and `wparam` is the virtual-key code).
const WM_KEYDOWN: u32 = 0x0100;

/// The virtual-key code for the 'B' key (`0x42`) — the brick A/B toggle.
const VK_B: usize = 0x42;

/// The mesh-raster G-buffer color format (Render P5-r0). MUST equal the recorder's
/// `GBUFFER_FORMAT` (`R8G8B8A8_UNORM`) so the mesh-MRT producer pipeline's 3 declared
/// color formats match the bound albedo/normal/material attachments.
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// One vertex: a `Float32x3` position (offset 0), a `Float32x3` world normal (offset 12),
/// and a `Float32x4` color (offset 24). `#[repr(C)]` for the exact 40-byte stride. The
/// per-vertex normal feeds the mesh-MRT producer's G-buffer normal target (multi-object
/// meshes carry real face normals — the +Z constant the VS used to bake is gone).
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 40, "Vertex must be tightly packed at 40 bytes");

/// The MVP byte size (a `float4x4`). Must equal [`GBUFFER_MVP_BYTES`].
const MVP_BYTES: u32 = GBUFFER_MVP_BYTES as u32;

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // `'static` blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER vertex SPIR-V (`gbuffer_mrt.vs.spv`).
/// Vertex layout: position (loc 0, offset 0) + color (loc 1, offset 24) + per-vertex world
/// normal (loc 2, offset 12); passes the LINEAR color + the per-vertex normal through.
static MRT_VS_SPV: SpirvBlob<1480> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.vs.spv"
)));

/// Render P5-r0: the mesh-MRT G-buffer PRODUCER fragment SPIR-V (`gbuffer_mrt.fs.spv`):
/// writes albedo/normal/material as 3 MRT in the marcher's exact encoding (mask=1) + the
/// marcher-aligned `SV_Depth` (euclidean under perspective, axial under ortho).
static MRT_FS_SPV: SpirvBlob<2252> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer_mrt.fs.spv"
)));

/// The committed rung-5 fullscreen vertex SPIR-V (`fullscreen_sample.vs.spv`): a
/// fullscreen triangle generating positions + UVs from `SV_VertexID`.
static SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed rung-5 fullscreen fragment SPIR-V (`fullscreen_sample.fs.spv`): samples
/// the bound `Texture2D` + `SamplerState` at the UV and outputs it.
static SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.fs.spv"
)));

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Maps the swapchain's `i32` `VkFormat` to "readback bytes are BGRA" (skips an
/// unsupported / SRGB format). Identical to the packed on-screen test.
fn swapchain_readback_is_bgra(vk_format: i32) -> Option<bool> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(true),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(false),
        _ => None,
    }
}

/// The orthographic MVP push for the mesh-MRT vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose). Maps a fronto-parallel world vertex so the rasterized
/// `SV_Position.z` is the AXIAL `(CAM_Z - worldZ) / T_MAX` — the depth the fragment writes
/// back unchanged under ortho (`cam_mode == 0`), byte-identical to step 1. The trailing
/// 16 bytes are the `cam_eye` push field: `[0, 0, 0, 0]` (mode 0 = ortho; the eye is
/// unused since the ortho fragment keeps `SV_Position.z`). Mirrors the packed/P1b convention.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    let mt: [f32; 16] = [
        1.0 / h, 0.0,      0.0,          0.0,
        0.0,     -1.0 / h, 0.0,          0.0,
        0.0,     0.0,      -1.0 / tmax,  0.0,
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    // `[0u8; MVP_BYTES]` leaves the trailing cam_eye (bytes 64..80) at [0,0,0,0] => mode 0.
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The PERSPECTIVE MVP push (`proj * view`, column-major) FOLLOWED by the `cam_eye`
/// float4 (`xyz = eye`, `w = 1.0` = perspective mode), for the mesh-MRT vertex shader.
///
/// The 80-byte layout MUST match `gbuffer_mrt.vs.hlsl`'s `{ float4x4 mvp; float4 cam_eye }`
/// push. The `proj * view` is built from the SAME eye / basis / fov / aspect the marcher's
/// perspective ray-gen (`ray_gen.hlsli`) + `CompositePushConstants::perspective` use, so a
/// mesh vertex projects to the SAME pixel the marcher's ray through that pixel reaches at
/// that world point (screen-space alignment is the load-bearing requirement).
///
/// Convention matched to the marcher (`ray_gen.hlsli` PERSPECTIVE arm):
///   * `view`  : a right-handed look-along-`forward` frame. The marcher builds the ray
///     direction as `forward + right*(ndc_x*aspect*tan) + up*(ndc_y*tan)` and marches from
///     `eye` along it; the equivalent view matrix rows are `right`, `up`, `-forward` with
///     the eye translation, mapping a world point to camera space where camera looks down
///     `-z_cam` (`z_cam = -dot(forward, P - eye)`, the positive depth in front).
///   * `proj`  : maps camera `x_cam / (z_cam * aspect * tan)` and `y_cam / (z_cam * tan)`
///     to clip x/y (the inverse of the marcher's NDC->dir scaling). The marcher flips
///     NDC-y (`ndc_y = -(...)`), so the projection negates the camera-up axis to land a
///     `+y` world point in the upper half of the image, matching the ortho `-1/h` row.
///   * depth   : Vulkan clip `z ∈ [0, w]`; the EXACT clip-z is IRRELEVANT to correctness
///     here because the FRAGMENT overwrites depth via `SV_Depth = length(eye_rel)/T_MAX`.
///     A simple `z_clip = z_cam`, `w_clip = z_cam` (=> SV_Position.z = 1, unused) keeps the
///     vertex in front of the near plane (`z_cam > 0`) so it is not clipped.
///
/// The mesh and the marcher therefore agree in screen x/y; the per-pixel mesh depth comes
/// from the fragment's euclidean `length(cam_eye - P)`, NOT from this matrix's z.
#[rustfmt::skip]
fn perspective_mvp_bytes(
    eye: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    fov_y_radians: f32,
    aspect: f32,
) -> [u8; MVP_BYTES as usize] {
    let tan = (fov_y_radians * 0.5).tan();
    // view: world -> camera. Rows are the basis; the camera looks down -forward, so
    // z_cam = -dot(forward, P - eye) = +depth in front. (right, up, forward) is right-handed.
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (rx, ry, rz) = (right[0], right[1], right[2]);
    let (ux, uy, uz) = (up[0], up[1], up[2]);
    let (fx, fy, fz) = (forward[0], forward[1], forward[2]);
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = dot(forward, eye); // forward·eye; the in-front view depth is z_cam = forward·(P-eye) = forward·P - tz
    // proj * view, ROW-MAJOR math rows below; uploaded column-major (transposed) to match
    // `ortho_mvp_bytes`. clip.x = x_cam/(aspect*tan); clip.y = -y_cam/tan (flip to match the
    // marcher's `ndc_y = -(...)`); clip.z = clip.w = z_cam = forward·(P-eye) (POSITIVE in front, so
    // the perspective divide is well-defined). `forward` points INTO the scene (= the marcher's ray
    // direction `rd` in `ray_gen.hlsli`), so the basis row is `+forward`, NOT `-forward` — a flipped
    // sign here warps every vertex by a depth-dependent amount (`-2·forward·P`), which a flat quad
    // survives but a multi-depth cube cracks into black wedges (BUG fixed: was `[-fx,-fy,-fz,-tz]`).
    let sx = 1.0 / (aspect * tan);
    let sy = -1.0 / tan;
    // pv row r = proj_scale_r · view_row_r  (view_row = [basis | translation]).
    let pv: [[f32; 4]; 4] = [
        [sx * rx, sx * ry, sx * rz, sx * tx], // clip.x
        [sy * ux, sy * uy, sy * uz, sy * ty], // clip.y (flipped)
        [fx,      fy,      fz,      -tz     ], // clip.z = z_cam = forward·(P-eye) = forward·P - tz
        [fx,      fy,      fz,      -tz     ], // clip.w = z_cam (perspective divide)
    ];
    // Upload COLUMN-MAJOR: out[col*4 + row] holds pv[row][col] (the verified transpose).
    let mut out = [0u8; MVP_BYTES as usize];
    for col in 0..4 {
        for row in 0..4 {
            let b = pv[row][col].to_le_bytes();
            out[(col * 4 + row) * 4..(col * 4 + row) * 4 + 4].copy_from_slice(&b);
        }
    }
    // cam_eye push field (bytes 64..80): xyz = eye, w = 1.0 (perspective mode).
    let cam_eye = [eye[0], eye[1], eye[2], 1.0_f32];
    for (i, f) in cam_eye.iter().enumerate() {
        out[64 + i * 4..64 + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The mesh quad as two triangles spanning the world-XY footprint at world Z [`MESH_Z`].
/// The quad faces the camera (`+Z`), so every vertex carries the outward normal `[0, 0, 1]`.
fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let n = [0.0_f32, 0.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], normal: n, color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], normal: n, color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], normal: n, color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], normal: n, color: c };
    [bl, br, tr, bl, tr, tl]
}

/// Emits one mesh quad face as two CCW triangles `(a, b, c)` + `(a, c, d)`, every vertex
/// carrying the supplied outward world `normal` `n` and `color`. `corners` are the four
/// quad corners in CCW order as seen from the `+n` side (matching [`quad_vertices`]'s
/// `bl, br, tr, tl` winding for the `+Z` face). Culling is OFF (`rhi_impl.rs`), so the
/// winding is cosmetic, but it is kept consistent for correctness.
fn mesh_quad(corners: [[f32; 3]; 4], n: [f32; 3], color: [f32; 4]) -> [Vertex; 6] {
    let [a, b, c, d] = corners;
    let v = |p: [f32; 3]| Vertex { position: p, normal: n, color };
    [v(a), v(b), v(c), v(a), v(c), v(d)]
}

/// A solid axis-aligned mesh box centered at `center` with per-axis half-extents `half`,
/// as 6 faces × 2 triangles = 36 vertices. Each face carries its outward axis normal
/// (`±X`, `±Y`, `±Z`), with its 4 corners ordered CCW as seen from outside the box. The
/// per-vertex normals feed the G-buffer normal target so the box's faces shade distinctly.
fn mesh_box(center: [f32; 3], half: [f32; 3], color: [f32; 4]) -> Vec<Vertex> {
    let [cx, cy, cz] = center;
    let [hx, hy, hz] = half;
    let (x0, x1) = (cx - hx, cx + hx);
    let (y0, y1) = (cy - hy, cy + hy);
    let (z0, z1) = (cz - hz, cz + hz);

    // Each face lists its 4 corners CCW from the outward normal's side.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        // +Z (front): looking toward -Z, CCW = bl, br, tr, tl in the +Z plane.
        ([0.0, 0.0, 1.0], [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]),
        // -Z (back): looking toward +Z, CCW winds the opposite way in X.
        ([0.0, 0.0, -1.0], [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]]),
        // +X (right): looking toward -X, CCW in the +X plane.
        ([1.0, 0.0, 0.0], [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]]),
        // -X (left): looking toward +X.
        ([-1.0, 0.0, 0.0], [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]]),
        // +Y (top): looking toward -Y.
        ([0.0, 1.0, 0.0], [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]]),
        // -Y (bottom): looking toward +Y.
        ([0.0, -1.0, 0.0], [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]]),
    ];

    let mut verts = Vec::with_capacity(36);
    for (n, corners) in faces {
        verts.extend_from_slice(&mesh_quad(corners, n, color));
    }
    verts
}

/// The ORTHO world-XY of pixel `(px, py)`'s ray at the COMPOSITE extent — the
/// extent-aware mirror of `compute::pixel_world_xy` (which is frozen to 64×64). The
/// arithmetic is byte-identical to the shader's / `composite_ray`'s ORTHO arm (`u`/`v`
/// → `* SDF_VIEW_HALF_EXTENT`), just parameterized on the live extent so the discriminator
/// picking + mesh-coverage host model track the 512×512 dispatch the marcher runs.
fn composite_pixel_world_xy(px: u32, py: u32) -> [f32; 2] {
    let u = (((px as f32) + 0.5) / (COMPOSITE_W as f32)) * 2.0 - 1.0;
    let v = -((((py as f32) + 0.5) / (COMPOSITE_H as f32)) * 2.0 - 1.0);
    [u * SDF_VIEW_HALF_EXTENT, v * SDF_VIEW_HALF_EXTENT]
}

/// Whether the SDF field is hit at pixel `(px, py)` IGNORING the mesh, at the COMPOSITE
/// extent. The extent-aware mirror of `compute::editlist_pixel_hits` (frozen to 64×64):
/// it asks the extent-aware marcher oracle for the attributes with NO mesh
/// (`mesh_depth == MESH_DEPTH_CLEAR`, so `t_mesh == +inf`) — then `mask == 1` is exactly
/// a pure SDF geometry hit. Lighting flags are irrelevant to the hit test.
fn composite_sdf_hits(edits: &[SdfEdit], px: u32, py: u32) -> bool {
    let materials = [GoldenMaterial::default()];
    let attrs = golden_marcher_attributes(
        edits,
        &materials,
        MESH_DEPTH_CLEAR,
        px,
        py,
        COMPOSITE_W,
        COMPOSITE_H,
        CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA,
        0,
        DEFAULT_LIGHT_DIR,
    );
    attrs.mask == 1
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint
/// (the rasterizer's covered-pixel set, host-computable from the SAME camera mapping).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = composite_pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU is expected to produce: the constant inside the
/// quad, the clear value outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// The base-sphere SDF scene (one union sphere, origin, r=0.5) — the recognizable SDF
/// body the mesh occludes (the packed/P1b `sphere_scene`).
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (the CPU seeds
/// the edit-list header before submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is at least `words.len() * 4` bytes inside the persistent
        // host-coherent mapping; `dst + i` for `i < words.len()` is in-bounds. No GPU
        // work is in flight yet (the present loop follows), so the host write is
        // unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// PBR MVP-2: the std430 word-packing of a ONE-element material table holding the engine
/// default material (mid-gray dielectric: base 0.8/0.8/0.8/1, metallic 0, roughness 0.5,
/// reflectance 0.5, flags 0, emissive 0). 12 words = 48 B (mirrors `MaterialGpu`'s 3 vec4
/// lanes). The windowed scene's edits carry no material id, so every SDF hit picks id 0.
const DEFAULT_MATERIAL_TABLE: [u32; 12] = [
    0x3F4CCCCD, 0x3F4CCCCD, 0x3F4CCCCD, 0x3F800000, // base_color: 0.8, 0.8, 0.8, 1.0
    0x00000000, 0x3F000000, 0x3F000000, 0x00000000, // mrr: metallic 0, rough 0.5, refl 0.5, flags 0
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // emissive: 0, 0, 0, 0
];

/// Lighting L0a: the std430 word-packing of the DEGENERATE light table — the 0%-gate
/// anchor that reproduces the resolve's old compiled-in `LIGHT_DIR`/`LIGHT_COLOR`/`SKY_*`
/// constants byte-for-byte. Layout `[LightHeaderGpu (16 words) || GpuLight[2] (24 words)]`
/// = 40 words = 160 B (mirrors `boyko_render::light` + `light_table.hlsli`):
///
/// - header: light_count 2, exposure 1.0, l0a_count 2 (1 dir + 1 sky), point_spot 0,
///   sky_diffuse/sky_spec = (0.10,0.10,0.12) (carried; the L0a resolve drives ambient
///   from the sky entity, these are unused by the resolve), cluster params 0.
/// - element 0 (DIRECTIONAL, kind 0): dir (0,0,1), range +inf, color (1,1,1) — matches
///   the old `LIGHT_DIR` / `LIGHT_COLOR` (illuminance 1.0).
/// - element 1 (SKY, kind 3): ground (0.10,0.10,0.12) in the pos lane, sky (0.10,0.10,0.12)
///   in the color lane — `sky == ground` ⇒ the hemisphere `lerp` folds to the old `SKY_*`.
const DEGENERATE_LIGHT_TABLE: [u32; 40] = [
    // --- LightHeaderGpu (16 words) ---
    0x00000002, 0x3F800000, 0x00000002, 0x00000000, // count 2, exposure 1.0, l0a 2, ps 0
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // sky_diffuse 0.10,0.10,0.12, pad
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // sky_spec    0.10,0.10,0.12, pad
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // cluster_params (zero in L0)
    // --- GpuLight[0] DIRECTIONAL (12 words) ---
    0x00000000, 0x00000000, 0x3F800000, 0x00000000, // dir_kind: (0,0,1), kind 0
    0x00000000, 0x00000000, 0x00000000, 0x7F800000, // pos_range: (0,0,0), range +inf
    0x3F800000, 0x3F800000, 0x3F800000, 0x00000000, // color_cone: (1,1,1), cone 0
    // --- GpuLight[1] SKY (12 words) ---
    0x00000000, 0x00000000, 0x00000000, 0x00000003, // dir_kind: (0,0,0), kind 3 (SKY)
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // pos_range: ground 0.10,0.10,0.12, r 0
    0x3DCCCCCD, 0x3DCCCCCD, 0x3DF5C28F, 0x00000000, // color_cone: sky 0.10,0.10,0.12, cone 0
];

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Decodes one readback texel into `[r, g, b]`, applying the swapchain channel order.
fn readback_rgb(texel: [u8; 4], is_bgra: bool) -> [i32; 3] {
    if is_bgra {
        [texel[2] as i32, texel[1] as i32, texel[0] as i32]
    } else {
        [texel[0] as i32, texel[1] as i32, texel[2] as i32]
    }
}

/// `true` if a readback texel agrees with a golden packed `0xAABBGGRR` within
/// `CHANNEL_TOL` per RGB channel (swapchain-byte-order-aware).
fn readback_close(texel: [u8; 4], golden: u32, is_bgra: bool) -> bool {
    let g = readback_rgb(texel, is_bgra);
    let w = unpack_rgb(golden);
    (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Asserts a readback texel agrees with a golden packed color (swapchain-order-aware).
fn assert_readback_close(texel: [u8; 4], golden: u32, is_bgra: bool, label: &str) {
    assert!(
        readback_close(texel, golden, is_bgra),
        "{label}: readback {texel:02x?} (bgra={is_bgra}) != golden {golden:#010x} -> {:?} within +/-{CHANNEL_TOL}",
        unpack_rgb(golden),
    );
}

/// `true` if two golden packed colors agree within `CHANNEL_TOL` per RGB channel.
fn goldens_close(a: u32, b: u32) -> bool {
    let x = unpack_rgb(a);
    let y = unpack_rgb(b);
    (0..3).all(|c| (x[c] - y[c]).abs() <= CHANNEL_TOL)
}

/// The byte index of texel `(x, y)` in a tightly-packed 4-byte/texel readback of a
/// `w`-wide image.
fn texel_base(x: u32, y: u32, w: u32) -> usize {
    ((y * w + x) * 4) as usize
}

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)` at the COMPOSITE
/// extent (using the extent-aware hit/coverage host models).
fn find_texel(edits: &[SdfEdit], pred: impl Fn(bool, bool) -> bool) -> Option<(u32, u32)> {
    for py in 0..COMPOSITE_H {
        for px in 0..COMPOSITE_W {
            let hit = composite_sdf_hits(edits, px, py);
            let covered = mesh_covers_pixel(px, py);
            if pred(hit, covered) {
                return Some((px, py));
            }
        }
    }
    None
}

/// Writes an `w × h` RGBA byte buffer as a 32-bpp top-down BI_RGB .bmp at `path`
/// (RGBA → the BMP's BGRA channel order; no row flip — `biHeight` is negative). Mirrors
/// the `boyko_render` test screenshot writer so the dump opens in any image viewer. The
/// caller passes an already-RGBA-normalized buffer (the swapchain R/B swap applied), so
/// the two dumps are byte-comparable regardless of the swapchain's native channel order.
fn write_bmp(path: &str, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(
        rgba.len(),
        (w * h * 4) as usize,
        "invariant: BMP body is w*h*4 bytes"
    );
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    std::fs::write(path, &buf)
}

/// Normalizes a swapchain readback (BGRA when `is_bgra`, else RGBA) into a contiguous
/// RGBA buffer — applying the SAME R/B handling as the golden assertion
/// ([`readback_rgb`]), so the two brick-ON / brick-OFF dumps are color-correct AND
/// byte-comparable to each other.
fn readback_to_rgba(readback: &[u8], w: u32, h: u32, is_bgra: bool) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (dst, src) in out.chunks_exact_mut(4).zip(readback.chunks_exact(4)) {
        let texel = [src[0], src[1], src[2], src[3]];
        let rgb = readback_rgb(texel, is_bgra);
        dst[0] = rgb[0] as u8;
        dst[1] = rgb[1] as u8;
        dst[2] = rgb[2] as u8;
        dst[3] = src[3];
    }
    out
}

/// The fixed dump path for the brick-ON (empty-skip + trilinear + clip-map) frame.
const BRICK_ON_BMP: &str = r"C:\Users\flint\AppData\Local\Temp\brick_on.bmp";
/// The fixed dump path for the brick-OFF (analytic marcher) frame.
const BRICK_OFF_BMP: &str = r"C:\Users\flint\AppData\Local\Temp\brick_off.bmp";

#[test]
fn windowed_gbuffer_composite_present_is_validation_clean_and_renders_composite() {
    // Open the window first — the surface borrows its HWND/HINSTANCE and must be
    // destroyed before it.
    let mut window = match Window::open("boyko_rhi_vulkan gbuffer window", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());
    // Validation is the soundness oracle, NOT a render-output dependency: a context
    // booted with `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW
    // process on this box) still drives the pixel gate. The `state.total() == 0`
    // oracle below self-gates on `validation_enabled()`.
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel gate still runs");
    }
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: swapchain creation failed ({e:?})");
            return;
        }
    };
    assert!(swapchain.image_count() >= 1, "swapchain must expose >= 1 image");
    println!(
        "swapchain: {} images, extent {}x{}, format {}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height,
        swapchain.format()
    );

    // The composite present-blits 1:1 in the swapchain image's TOP-LEFT COMPOSITE_W×COMPOSITE_H
    // sub-rect. A WSI clamp WIDER is fine (the rest stays clear); SMALLER in either dimension
    // clips the composite → a graceful SKIP (the rung-11 handling).
    if swapchain.extent().width < COMPOSITE_W || swapchain.extent().height < COMPOSITE_H {
        eprintln!(
            "SKIP windowed_gbuffer_present: swapchain extent {}x{} is smaller than the {}x{} \
             composite, so the top-left 1:1 sub-rect does not fit",
            swapchain.extent().width,
            swapchain.extent().height,
            COMPOSITE_W,
            COMPOSITE_H
        );
        return;
    }

    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!(
            "SKIP windowed_gbuffer_present: swapchain format {} (e.g. {VK_FORMAT_B8G8R8A8_SRGB} SRGB) \
             has no host-decodable UNORM byte order",
            swapchain.format()
        );
        return;
    };

    // The present-blit pipeline declares the swapchain's color format (W2-b).
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP windowed_gbuffer_present: swapchain format has no basic-slice Format variant");
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    let device: &VulkanContext = &ctx;
    let sdf = sphere_scene();

    // --- Pick the three discriminator texels host-side, BEFORE any GPU run. ---
    let (ax, ay) = find_texel(&sdf, |hit, covered| hit && covered)
        .expect("invariant: some pixel must be over BOTH the sphere and the quad (mesh-occludes-SDF)");
    let (bx, by) = find_texel(&sdf, |hit, covered| hit && !covered)
        .expect("invariant: some pixel must be over the sphere but NOT the quad (SDF)");
    let (dx, dy) = find_texel(&sdf, |hit, covered| !hit && !covered)
        .expect("invariant: some pixel must be over neither (background)");

    let depth_at = |px, py| expected_mesh_depth(px, py);
    // The mesh-occludes (a) + background (d) texels are mask == 0 PASS-THROUGH arms — the
    // resolve emits `base` byte-identically (the 0%-gate), so the old inline composite is
    // still the truth there. PBR MVP-2 only changed the SDF-LIT arm.
    // Live-computed at the COMPOSITE extent via the extent-aware ORTHO oracle, so the golden
    // re-blesses automatically at 512×512 (the frozen 64×64 `golden_composite_pixel` is the
    // `_ex` forwarder at `(SDF_IMG_W, SDF_IMG_H)`; here we forward at `(COMPOSITE_W, COMPOSITE_H)`).
    // Render P5: a_want (mesh-occludes) is now a RASTER-PBR producer (mask == 1) — computed below
    // alongside b_want via the PBR oracle, NOT the old flat MESH_COLOR pass-through.
    let d_want =
        golden_composite_pixel_ex(&sdf, depth_at(dx, dy), dx, dy, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    // The SDF-LIT texel (b) is now FULL Cook-Torrance (the owner-acknowledged behavioral
    // change, PBR plan call F), NOT the old `base*vis` composite — so its golden comes from
    // the PBR oracle (`golden_deferred_resolve ∘ golden_marcher_attributes`) with the SAME
    // marcher params the windowed present uses (lighting ON, default light, DEFAULT omega).
    let materials = [GoldenMaterial::default()];
    let b_flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let b_attrs = golden_marcher_attributes(
        &sdf, &materials, depth_at(bx, by), bx, by, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA, b_flags, DEFAULT_LIGHT_DIR,
    );
    let (_, b_rd) = composite_pixel_ray(bx, by, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    let b_want = golden_deferred_resolve(b_attrs, b_rd, &materials);

    // Render P5: the mesh-occludes (a) texel is a raster-PBR producer (mask == 1) — model it
    // through the SAME PBR oracle as the SDF-lit texel (golden_marcher_attributes' has_mesh arm
    // emits the raster mesh attrs; golden_deferred_resolve runs full Cook-Torrance).
    let (_, a_rd) = composite_pixel_ray(ax, ay, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho);
    let a_attrs = golden_marcher_attributes(
        &sdf, &materials, depth_at(ax, ay), ax, ay, COMPOSITE_W, COMPOSITE_H, CompositeCamera::Ortho,
        DEFAULT_MARCHER_OMEGA, b_flags, DEFAULT_LIGHT_DIR,
    );
    let a_want = golden_deferred_resolve(a_attrs, a_rd, &materials);
    assert!(
        !goldens_close(a_want, b_want),
        "invariant: the raster-PBR mesh and the SDF lit color must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(a_want, d_want),
        "invariant: MESH_COLOR and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(b_want, d_want),
        "invariant: the SDF lit color and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );

    // === Build the P1c on-screen G-buffer scene's STATIC inputs (the GBufferScene). ===

    // The ONE SDF edit authority (principle 0): the field every brick resource bakes from. Built
    // once from the same `sdf` edits the marcher's edit-list carries, so the brick cache mirrors the
    // analytic field exactly (no parallel field store). `bump_gen()` marks it dirty-baked.
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in &sdf {
            assert!(f.push(*e), "windowed scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };

    // SDF brick-atlas campaign — the WINDOWED ACTIVATION. The full 3-level clip-map, baked from the
    // authority and centered at the WORLD ORIGIN (NOT the camera): the demo scene is small + fixed
    // (the sphere lives in ~[-0.5, 0.5]³, well inside level 0's [-4, 4]³ box), and the camera ORBITS
    // a fixed scene rather than translating through the world, so an origin-centered clip-map is
    // STATIC — no per-frame re-center (the toroidal camera-follow is campaign M5). Level 0 covers the
    // whole scene, so `brick_levels = 3` and `= 1` render the same here; the full 3-level path is
    // used to exercise exactly what the owner asked for (empty-skip + trilinear + clip-map LOD).
    //
    // `BrickClipmap::create` bakes every level's atlas + seeds every level's pointer grid + ends each
    // upload in SHADER_READ (the offscreen barrier discipline), so the cache is sample-ready before
    // the first present. The scene is static (the orbit moves only the camera, which is NOT in the
    // field), so no per-frame rebake is needed — a one-time startup bake suffices (an edit loop would
    // call `rebake_dirty_all` on the authority's `gen` change; there is none here).
    let clipmap = BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0])
        .expect("M4 brick clip-map (windowed activation) — create + bake every level + upload");

    // Level 0's empty-skip grid geometry (the marcher's `lvl == 0` arm indexes binding 9 with it).
    // The clip-map's level-0 grid IS the fine `default_near_field` (`16³ @ 0.5`, origin `[-4,-4,-4]`)
    // — see `brick_atlas::level_empty_skip_grid` — so the activation's `with_brick` uniforms come
    // from `PointerGrid::default_near_field` to match the bound binding-9 SSBO + the host oracle.
    let level0_grid = PointerGrid::default_near_field();
    let brick_on = BrickActivation {
        grid_origin: level0_grid.origin,
        grid_dims: level0_grid.dims,
        brick_world: level0_grid.brick_world,
        levels: BRICK_LEVELS as u32,
    };

    // The edit-list StorageBuffer (binding 0), host-seeded ONCE. Over-allocated to the
    // full `EDITLIST_BUFFER_WORDS` (the encoder debug-asserts that size); the marcher
    // only reads the header + edit array.
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // The camera/extent UNIFORM buffer (binding 5), host-seeded ONCE at the golden 64×64
    // ORTHO extent (the composite size — NOT the swapchain extent) for bit-exact rays.
    //
    // SDF brick-atlas M4 (clip-map LOD, Slice C): the b5 UBO is sized to `B5_CAMERA_UBO_BYTES_M4`
    // (224 B) — the 80-byte camera block + the `BRICK_LEVELS`-level `M4GridParams` array tail at
    // `M2_GRID_PARAMS_OFFSET` (80). The widened marcher cbuffer declares 224 B. The tail holds the
    // ACTIVATED clip-map's baked per-level params (`clipmap.params()`); the brick-ON 'B' toggle reads
    // them across all 3 levels, the OFF path (`brick_levels = 1`) reads only level 0.
    let camera_uniform = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H);
        assert_eq!(pc.count, PIXELS);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B (offset of the M4 tail)");
        // The M4 array tail at offset 80: the clip-map's baked per-level snapped origins (the values
        // the level atlases were baked at — `M4GridParams::camera_centered([0,0,0])`). The marcher's
        // clip-map ladder reads `m2_levels[0..brick_levels]` from here; on the brick-ON path (the 'B'
        // toggle, `brick_levels = 3`) it samples real per-level params, and on the OFF path
        // (`brick_levels = 1`) it reads only level 0 — which, origin-centered, equals the M2 near-field.
        let m4 = *clipmap.params();
        let m4_bytes = m4.as_ubo_bytes();
        debug_assert_eq!(M2_GRID_PARAMS_OFFSET + m4_bytes.len(), B5_CAMERA_UBO_BYTES_M4);
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
        // 80-byte camera block is written at offset 0 and the (224-80)-byte M4 tail at offset 80 —
        // together exactly 224 in-bounds bytes, disjoint. No GPU work is in flight yet (the present
        // loop follows), so the writes are unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
            core::ptr::copy_nonoverlapping(
                m4_bytes.as_ptr(),
                mapped.as_ptr().add(M2_GRID_PARAMS_OFFSET),
                m4_bytes.len(),
            );
        }
    }

    // P4b: the coarse-cull tile StorageBuffer (vocab binding 6), sized to the full tile
    // grid at the COMPOSITE extent (NOT the swapchain extent — the marcher dispatches +
    // the camera UBO `count` are sized to the 64×64 composite). The windowed path runs
    // the marcher with the coarse cull gated OFF (coarse_enabled=0), so its contents are
    // never read — but the marcher shader DECLARES binding 6, so a VALID descriptor must
    // be bound. Allocated once; bound (borrowed) into the vocabulary set; never written.
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // PBR MVP-2: the material table SSBO (vocab binding 7 + resolve binding 4). The windowed
    // scene's edits carry no material id (center.w == 0), so every SDF hit picks material 0 —
    // the default mid-gray dielectric. One 48-B element (12 words; mirrors MaterialGpu).
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer (vocab binding 7 / resolve binding 4)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // The brick bindings 9..=14 are the ACTIVATED clip-map's REAL per-level resources (created above
    // from the authority): level `L`'s empty-skip pointer grid at @9/@11/@13 and its brick atlas +
    // sampler at @10/@12/@14. This REPLACES the prior "single atlas duplicated at every level slot"
    // OFF scaffold with the genuine 3-level cache — the SAME binding discipline the offscreen
    // RTX-verified `run_gbuffer_hybrid_m4` uses. The descriptors are static (the clip-map is baked
    // once + origin-centered, never re-snapped), so they are written ONCE into the vocabulary set;
    // the per-frame 'B' toggle flips only the push gates. On the OFF push (`brick_levels = 1`) the
    // marcher reads only level 0's bindings (9/10) — bound-but-unread above that.

    // Lighting L0a: the light table SSBO (resolve binding 6). For this test the table is
    // seeded host-visible with the DEGENERATE table (the 0%-gate anchor); a production
    // path would mint it DEVICE-LOCAL (TRANSFER_DST | STORAGE) and seed via
    // `upload_initial`, then re-upload on-change via the async recorder (rung L0-r0). The
    // resolve reads the header (count + exposure) + the table.
    let light_table_bytes = (DEGENERATE_LIGHT_TABLE.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table storage buffer (resolve binding 6)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }
    // The host-coherent STAGING source for the async re-upload (rung L0-r0). Seeded with
    // the SAME degenerate table; the windowed present path runs with `light_dirty == false`
    // (the static-scene 0%-gate: the recorder records NO copy), so this is the dormant
    // source kept valid for the dirty-frame path.
    let light_staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table staging buffer (rung L0-r0)");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }

    // The mesh quad's vertex buffer (host-visible).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes;
        // `vertices` is a distinct stack array of `vertex_bytes` bytes; the write
        // completes before any submit references the buffer (host-coherent: no flush).
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    // The depth sampler (bound at vocab binding 1; ignored by the marcher's `.Load`).
    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
    // The present-blit sampler (nearest/clamp for a 1:1 albedo sample).
    let present_sampler = RhiDevice::create_sampler(
        device,
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
        },
    )
    .expect("present nearest/clamp sampler");

    // The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0): rung-3 vertex
    // layout + 64-byte VERTEX MVP + 3 G-buffer color formats + a declared depth format.
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes; the production
            // `record_gbuffer` binds albedo/normal/material as the 3 MRT attachments.
            color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
            blend: None,
        },
    )
    .expect("depth-prepass graphics pipeline");

    // The P1b marcher: the vocabulary bind-group layout + the compute pipeline.
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    // P4b: binding 6 = the coarse-cull tile StorageBuffer. The marcher shader DECLARES
    // it unconditionally, so the layout must carry it (and a valid buffer must be bound)
    // even though the windowed path runs with the coarse cull gated OFF (coarse_enabled=0).
    let vocab_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @8 (the marcher stores the surface `t`).
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // M1: the empty-skip `PointerGrid` SSBO @9. The recompiled marcher SPIR-V statically
        // references `StructuredBuffer<uint> PointerGrid : register(t9)` inside the
        // runtime-gated empty-skip branch (DXC does NOT dead-strip it despite `brick_enabled`),
        // so the layout MUST declare binding 9 — a VALID StorageBuffer descriptor must be bound
        // even though the windowed path runs the skip OFF (`brick_enabled == 0`), or
        // `vkCreateComputePipelines` / `vkCmdDispatch` trip VUID-…-layout-07988 / -08114.
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // M2: the brick-atlas combined image+sampler @10. The recompiled marcher SPIR-V
        // statically references `Texture3D BrickAtlas : register(t10)` +
        // `SamplerState BrickSampler : register(s10)` (collapsed to ONE combined descriptor by
        // DXC) inside the runtime-gated `brick_trilinear` branch (NOT dead-stripped despite the
        // gate), so the layout MUST declare binding 10 — a VALID combined image+sampler must be
        // bound even though the windowed path runs the trilinear path OFF (`brick_trilinear == 0`,
        // bound-but-unread, byte-identical output), or the layout VUIDs trip (the M1 binding-9
        // lesson at the next slot).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        // M4 clip-map LOD (Slice C): the LEVEL-1 + LEVEL-2 brick bindings. The recompiled marcher
        // SPIR-V statically references `PointerGrid1`@t11 + `BrickAtlas1`@t12 + `PointerGrid2`@t13 +
        // `BrickAtlas2`@t14 inside the runtime level branch-ladder, so the layout MUST declare all four
        // — bound-but-unread on the windowed OFF/N=1 path (`brick_levels == 1` takes only the lvl==0 arm).
        // 6 brick bindings total (9..=14) under the 16-binding cap.
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
    let marcher = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
        },
    )
    .expect("P1b G-buffer marcher compute pipeline");

    // The deferred RESOLVE (`deferred_pbr.comp`): 8 bindings (≤ 12) { gAlbedo @0, gNormal
    // @1, gMaterial @2, lit @3 (STORAGE images), the material SSBO @4, the camera UBO @5,
    // the Lighting-L0 light table SSBO @6, the Lighting-L0b gViewT STORAGE image @7 }. The
    // resolve reads the extent + the per-pixel view direction from the camera UBO, the
    // lights from the table (L0a), and (L0b) `gViewT` to reconstruct `P` for point/spot.
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
    let resolve_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Lighting L0b: the gViewT STORAGE image @7 (the resolve reads it under `mask == 1`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        // Lighting L1: the ClusterGrid @8 + LightIndexList @9 SSBOs (read on the cluster path;
        // L1 is OFF here, so they bind the light table as a harmless valid placeholder).
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // P6 R1: the SDF edit-list `Buf` SSBO @10 (the resolve's `sdf_soft_shadow_ranged`
        // analytic march reads it read-only; the SAME buffer the marcher binds @0).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (the resolve reads it under
        // `ssao_mode != 0`; OFF here, so it is a bound-but-unread descriptor). The production
        // `GBufferTargets` binds the SSAO image at @11, so the resolve layout MUST declare it or
        // bind-group creation trips the entry-count check (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
    let resolve_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            // The resolve shader pushes NO constants, but `create_compute_pipeline` requires
            // a non-empty (multiple-of-4) push range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
        },
    )
    .expect("deferred resolve compute pipeline");

    // The present-blit: one COMBINED_IMAGE_SAMPLER layout + the fullscreen-sample pipeline.
    let present_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::CombinedImageSampler,
                stage: ShaderStage::FRAGMENT,
            }],
        },
    )
    .expect("present-blit bind-group layout (one COMBINED_IMAGE_SAMPLER)");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&present_layout),
            blend: None,
        },
    )
    .expect("present-blit fullscreen-sample pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its
    // pipeline is created (the pipeline holds its own compiled code); each is destroyed
    // exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    let mvp = ortho_mvp_bytes();
    let mut scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_uniform: &camera_uniform,
        tiles_buffer: &tiles_buffer,
        // Brick bindings 9..=14: the ACTIVATED clip-map's REAL per-level resources. Level 0's grid +
        // atlas at @9/@10, level 1 at @11/@12, level 2 at @13/@14 — the genuine 3-level cache (NOT the
        // old "level-0 duplicated" OFF scaffold). The marcher samples level `L`'s grid/atlas on the
        // ON push; on the OFF push (`brick_levels = 1`) it reads only level 0 (9/10), with 11..14
        // bound-but-unread.
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        // Static-scene 0%-gate: the table is seeded once (host-visible above); no
        // on-change re-upload this run, so the recorder records NO copy/barrier (the
        // command stream is byte-identical to before L0-r0).
        light_dirty: false,
        // Lighting L1 is OFF for the on-screen demo (no cluster cull wired): the cull
        // pipeline + cluster SSBOs are absent, so the recorder skips the cull pass entirely
        // and the resolve's `clusters_enabled` header gate (0) loops the flat table — the L1
        // OFF / 0%-gate. The resolve set's @8/@9 bind the light table as a harmless valid
        // placeholder (never read on the OFF path; see GBufferTargets::create).
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The brick A/B toggle's STARTING state (flipped live by the 'B' key in the present loop).
        // `Some(brick_on)` boots the empty-skip + trilinear/cubic surface cache + 3-level clip-map ON;
        // `None` boots the analytic (OFF) path. RTX-verified byte-identical in this scene, so either
        // start looks the same on screen.
        brick: if BRICK_START_ON { Some(brick_on) } else { None },
        // P0 coarse tile-cull: OFF for this existing golden present (the 0%-gate — NO coarse
        // dispatch / barrier recorded, `coarse_enabled == 0`, byte-identical to the pre-P0 stream).
        // The dedicated `p0_windowed_coarse_cull_matches_uncull` test drives the ON vs OFF readback.
        coarse: None,
        // The on-screen present's coarse-cull mode (a don't-care here since `coarse == None`):
        // `EmptySkipOnly` is the lit-transparent on-screen cull (EMPTY-skip only, no `near_t` seed).
        coarse_mode: CoarseMode::EmptySkipOnly,
        // The on-screen demo renders with soft shadows (A1) + AO (A2) — its existing lighting
        // validation is unchanged (byte-identical push to the pre-`lighting_flags`-field stream).
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The legacy head-on shadow direction (`[0,0,1]`) — byte-identical to the pre-`light_dir`-
        // field marcher push (this golden present asserts the existing stream).
        light_dir: DEFAULT_LIGHT_DIR,
        // Render P7: SSAO OFF (the default) — NO SSAO pass recorded, byte-identical to the pre-P7
        // stream (the 0%-gate). These golden/cull-comparison presents assert the existing stream.
        ssao: None,
    };

    // The composite's native size — drives the G-buffer alloc + the 1:1 top-left present.
    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };

    // A host-visible staging buffer sized for one full swapchain image (4 B/texel).
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();

    let mut frame = GBufferFrame::new();

    // === DIAGNOSTIC: dump TWO comparable offscreen frames — brick ON vs brick OFF — so the
    //     orchestrator can open them side-by-side and confirm the brick-ON render matches the
    //     analytic (OFF) render (the owner reported the sphere seems to "disappear" toggling). ===
    //
    // Both frames are rendered from the IDENTICAL camera / scene / edit-list, differing ONLY by
    // `scene.brick` (Some(brick_on) vs None — the gate the marcher push carries). Each is read back
    // through the SAME staging buffer with the SAME R/B handling as the golden capture
    // (`readback_to_rgba`), then written as a 32-bpp BMP — so the two files are byte-comparable.
    //
    // COHERENCY: `render_gbuffer_frame` issues the swapchain→staging copy in the readback frame's
    // submit, but the host read is only coherent once that frame slot's fence has been WAITED again
    // (it is waited at the START of each `render_gbuffer_frame`). The engine keeps `FRAMES_IN_FLIGHT`
    // (== 2) slots, so rendering `DRAIN_FRAMES` (3 > 2) further frames after the readback frame
    // guarantees the readback slot's fence was re-waited — exactly the discipline the existing golden
    // relies on. A swapchain recreate (`Ok(false)`) on the readback frame skips that dump gracefully.
    const DRAIN_FRAMES: u32 = 3;
    // Captures `scene`/`renderer`/`swapchain`/`frame`/`window` mutably + the device/surface/staging
    // immutably; takes only the brick state + dump path. NLL ends these borrows after the last call,
    // before the interactive loop reuses them. (A free `fn` would have to name `BoundBuffer` +
    // re-thread eight references; the capturing closure keeps the call sites trivial.)
    let mut dump_brick_ab = |brick_state: Option<BrickActivation>, path: &str| {
        if !window.pump_events() {
            return; // The window was closed before the dump — skip it cleanly.
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE brick dump: extent changed before the dump frame — skipping {path}");
            return;
        }

        scene.brick = brick_state;
        let clear = [0.0_f32, 0.0, 0.0, 1.0];

        // The readback frame (requests the swapchain→staging copy).
        // SAFETY: identical contract to the interactive loop's `render_gbuffer_frame` below —
        // `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource is live;
        // `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover the
        // composite extent; `staging` is host-visible and ≥ one swapchain image in bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                &ctx, &surface, &mut swapchain, &scene, &mut frame,
                window.width(), window.height(), clear, present_extent, Some(&staging),
            )
        }
        .unwrap_or_else(|e| panic!("brick dump frame ({path}) failed: {e:?}"));
        if !presented {
            eprintln!("NOTE brick dump: swapchain recreated on the readback frame — skipping {path}");
            return;
        }
        let dump_extent = swapchain.extent();

        // Drain frames so the readback slot's fence is waited (staging becomes coherent).
        for _ in 0..DRAIN_FRAMES {
            if !window.pump_events() {
                break;
            }
            window.refresh_size();
            // SAFETY: same contract; no readback requested on the drain frames.
            let _ = unsafe {
                renderer.render_gbuffer_frame(
                    &ctx, &surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, None,
                )
            }
            .unwrap_or_else(|e| panic!("brick dump drain frame ({path}) failed: {e:?}"));
        }

        // Read back the staged swapchain image, normalize to RGBA, write the BMP.
        let w = dump_extent.width;
        let h = dump_extent.height;
        let byte_count = (w * h * 4) as usize;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let mut raw = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent bytes;
        // the readback frame's copy completed before this read (its slot fence was re-waited by the
        // drain frames above); `raw` is a distinct, non-overlapping alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
        let rgba = readback_to_rgba(&raw, w, h, is_bgra);
        match write_bmp(path, &rgba, w, h) {
            Ok(()) => println!("brick dump -> {path} ({w}x{h})"),
            Err(e) => eprintln!("NOTE brick dump: failed to write {path}: {e:?}"),
        }
    };

    dump_brick_ab(Some(brick_on), BRICK_ON_BMP);
    dump_brick_ab(None, BRICK_OFF_BMP);
    // The closure is not used again; NLL ends its `&mut scene`/`renderer`/`swapchain`/`frame`/`window`
    // borrows here, so the interactive loop below freely reuses them.

    // Restore the boot brick state for the interactive loop + the live golden capture.
    scene.brick = if BRICK_START_ON { Some(brick_on) } else { None };

    // --- Present the image-based composite; request the swapchain-image readback on ONE
    //     presented frame. The loop runs up to `MAX_FRAMES` (so CI / a headless run always
    //     terminates) but ALSO exits the moment the window is closed, so the owner can watch +
    //     toggle the brick path live and close the window to end the run. ---
    //
    // Brick A/B TOGGLE: each frame the captured input ring is drained; a 'B' WM_KEYDOWN flips
    // `scene.brick` between ON (`Some(brick_on)` — empty-skip + trilinear/cubic + 3-level clip-map)
    // and OFF (`None` — the analytic marcher). The gates live entirely in the per-frame marcher
    // push, so the flip costs nothing but a different push byte image — no re-record, no re-bind.
    // The owner confirms the brick render looks IDENTICAL to analytic (RTX-verified byte-identical
    // in this scene) and is faster (empty-space-skip).
    //
    // The frame cap. Under `cargo test` (CI / the tester) the loop must terminate fast + record the
    // golden, so the DEFAULT is a short bounded run (`CI_FRAMES`). The owner runs it interactively by
    // setting `BOYKO_WINDOW_FRAMES` (e.g. a large count) — then the loop runs that many frames (or
    // until the window is closed), long enough to watch + toggle the brick A/B live. Either way the
    // golden readback frame (`i == 2`) renders before the cap, in the `BRICK_START_ON` state (brick-ON
    // is byte-identical to analytic, so the +/-2/255 golden holds regardless of the start state).
    const CI_FRAMES: u32 = 5;
    let max_frames: u32 = std::env::var("BOYKO_WINDOW_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CI_FRAMES);
    let clear = [0.0_f32, 0.0, 0.0, 1.0];
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for i in 0..max_frames {
        if !window.pump_events() {
            break; // The window was closed — end the interactive run cleanly.
        }
        window.refresh_size();

        // Drain captured input; a 'B' key-down toggles the brick A/B state for the NEXT dispatch.
        window.drain_input(|msg| {
            if let CapturedMsg::Raw { msg: wm, wparam, .. } = msg
                && wm == WM_KEYDOWN
                && wparam == VK_B
            {
                scene.brick = match scene.brick {
                    Some(_) => None,
                    None => Some(brick_on),
                };
                println!(
                    "brick toggle -> {}",
                    if scene.brick.is_some() { "ON (empty-skip + trilinear + clip-map)" } else { "OFF (analytic)" }
                );
            }
        });

        // Request the readback on a single steady frame, only while the live extent
        // still matches the staging-buffer size (a resize simply skips the golden).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = i == 2 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // The golden discriminator-texel assertion (below) compares the readback against the ANALYTIC
        // host golden within ±2/255. The M1 empty-skip is verified ±2/255 of analytic, but the M2
        // trilinear+cubic SURFACE crossing is validated by the exact-CSG hit residual (M2_CREASE_EPS),
        // NOT by ±2/255 lit-color identity to the analytic marcher — the cubic can shift the surface
        // `t` (and thus the shaded color) slightly. So force the GOLDEN-CAPTURE frame to render OFF
        // (analytic), then restore the live brick state for the next frame. This keeps the CI golden
        // deterministic (analytic == analytic) while leaving the boot/interactive state owner-driven.
        let restore_brick = scene.brick;
        if want_readback {
            scene.brick = None;
        }

        // SAFETY: `ctx`/`surface`/`swapchain` are live + created on the same device as
        // `renderer`; every `scene` resource is live on this device; `edit_list` /
        // `camera_uniform` were host-seeded once + are never written again (the marcher
        // only reads them); `frame`'s targets are synced to `present_extent` by the call;
        // `scene.dispatch_group_count_x` + the camera UBO's `count` were sized to the
        // composite extent; a `Some(rb)` staging buffer is host-visible + `staging_size`
        // (>= one swapchain image) bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                &ctx,
                &surface,
                &mut swapchain,
                &scene,
                &mut frame,
                window.width(),
                window.height(),
                clear,
                present_extent,
                rb,
            )
        }
        .unwrap_or_else(|e| panic!("gbuffer present frame {i} failed: {e:?}"));

        // Restore the live brick state the golden frame may have forced OFF.
        scene.brick = restore_brick;

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean windowed image-based present records zero validation messages.
    // Gated on `validation_enabled()` so the composite pixel golden below still runs under
    // `BOYKO_DISABLE_VALIDATION` (no messenger is created when validation is off).
    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the windowed G-buffer present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // The golden: if a readback frame presented, the three discriminator texels must
    // match the host composite truth (swapchain byte-order-aware) — PROVING the
    // image-based composite reached the swapchain image with correct colors, equal to
    // the packed on-screen composite within +/-2/255.
    if readback_done {
        let w = readback_extent.width;
        let h = readback_extent.height;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let byte_count = (w * h * 4) as usize;
        let mut out = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (>= `byte_count`) mapped
        // host-coherent bytes; the readback frame's submit completed before this read
        // (the renderer fence-waits the frame slot at the START of each subsequent
        // `render_gbuffer_frame`, and frames followed frame 2, so frame 2's copy is
        // complete + coherent); `out` is a distinct, non-overlapping alloc.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), byte_count);
        }

        let read_texel = |px: u32, py: u32| -> [u8; 4] {
            let b = texel_base(px, py, w);
            [out[b], out[b + 1], out[b + 2], out[b + 3]]
        };

        let a_got = read_texel(ax, ay);
        let b_got = read_texel(bx, by);
        let d_got = read_texel(dx, dy);

        assert_readback_close(a_got, a_want, is_bgra, "mesh-occludes-SDF texel (raster-PBR)");
        assert_readback_close(b_got, b_want, is_bgra, "SDF texel (lit color)");
        assert_readback_close(d_got, d_want, is_bgra, "background texel");

        // The occlusion actually changed the on-screen pixel: the mesh-occludes-SDF
        // texel and the SDF texel are BOTH over the sphere, yet must differ.
        let a_rgb = readback_rgb(a_got, is_bgra);
        let b_rgb = readback_rgb(b_got, is_bgra);
        assert!(
            (0..3).any(|c| (a_rgb[c] - b_rgb[c]).abs() > CHANNEL_TOL),
            "the on-screen mesh-occluded texel {a_got:02x?} and the SDF-visible texel {b_got:02x?} \
             (both over the sphere) must differ — proving the image-based hybrid composite (not a clear) reached the screen"
        );
    } else {
        eprintln!(
            "NOTE windowed_gbuffer_present: no readback frame presented (swapchain kept recreating); \
             validation was still asserted clean across all frames"
        );
    }

    // Clean reverse-order teardown: renderer (waits idle) → the per-extent G-buffer
    // frame → the scene's static resources → swapchain → surface → window.
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no
    // submission references these resources; `ctx` is still alive; each is destroyed
    // exactly once, in reverse dependency order.
    unsafe {
        frame.destroy(&ctx);
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        // The brick clip-map (every level's atlas image + sampler + pointer-grid SSBO). The renderer
        // was dropped above (waits idle), so no submission still samples it; `ctx` is alive; the
        // by-value `destroy` moves each level's resources out once.
        clipmap.destroy(&ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        RhiDevice::destroy_buffer(device, camera_uniform);
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}

/// **Render P0 GPU gate — the windowed EMPTY-SKIP-ONLY coarse cull is LIT-TRANSPARENT.**
///
/// Drives the WINDOWED present path (the same `Renderer::render_gbuffer_frame` 3-pass) through a
/// swapchain-image readback TWICE from the IDENTICAL camera / scene / edit-list AT THE REAL ON-SCREEN
/// LIT FLAGS (`LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO`), differing ONLY by [`GBufferScene::coarse`]:
///
/// - `coarse = None` (the OFF / 0%-gate path): NO coarse dispatch + NO `tiles_buffer` barrier are
///   recorded, `coarse_enabled == 0` — the pre-P0 windowed command stream, byte-for-byte.
/// - `coarse = Some(&coarse_compute)` with `coarse_mode = EmptySkipOnly` (the lit-transparent ON
///   path): the P4b coarse-cull pass runs BEFORE the marcher (one invocation per 8×8 tile writes a
///   `TileBound` into vocab binding 6), and the marcher reads them with `coarse_enabled == 2` — it
///   SKIPS EMPTY tiles only, WITHOUT seeding `near_t` on the surface tiles.
///
/// # Why EmptySkipOnly (mode 2), not Full (mode 1)
///
/// The empty-tile skip is provably image-identical lit+unlit (an empty tile has no surface). The
/// FULL mode (1) additionally seeds the march at the tile's conservative `near_t` on a NON-empty
/// tile; fed into the B1 over-relaxed march that seed latches a different grazing tangent on the
/// silhouette (a shifted normal → a shifted AO/shadow), so the LIT cull-ON image gains a 16–32/255
/// rim — the FULL cull is NOT lit-transparent. EmptySkipOnly drops the seed, so it is transparent
/// UNDER LIGHTING by construction. This test asserts that: the ON readback MUST equal the OFF
/// readback within the goldens' per-channel tolerance ([`CHANNEL_TOL`], `+/-2/255`) AT THE LIT FLAGS
/// — proving the on-screen cull adds NO visible rim. (The FULL-mode image-transparency contract
/// remains the UNLIT offscreen golden `sdf_gbuffer_hybrid::p4b_cull_on_conservative_within_tol_of_cull_off`.)
///
/// The brick path is held OFF (`brick = None`) on BOTH frames so the comparison isolates the cull.
/// The test also asserts the validation layer is clean across the ON path (the recorder's new coarse
/// dispatch + barrier are sound).
///
/// `#[ignore]`: needs a real RTX windowed device. The orchestrator runs it on the GPU; CPU `cargo
/// test` skips it (the harness still compiles it, proving the OFF caller + the new `coarse` field +
/// the coarse-pipeline creation type-check).
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn p0_windowed_coarse_cull_matches_uncull() {
    let mut window = match Window::open("boyko_rhi_vulkan P0 coarse-cull window", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP p0_windowed_coarse_cull: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP p0_windowed_coarse_cull: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    // Validation is the soundness oracle, NOT a render-output dependency: a context
    // booted with `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW
    // process on this box) still drives the pixel gate. The `state.total() == 0`
    // oracle below self-gates on `validation_enabled()`.
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel gate still runs");
    }
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    // SAFETY: `window` outlives the surface (dropped after it below); its HWND/HINSTANCE are live
    // for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP p0_windowed_coarse_cull: surface creation failed ({e:?})");
            return;
        }
    };
    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP p0_windowed_coarse_cull: swapchain creation failed ({e:?})");
            return;
        }
    };

    if swapchain.extent().width < COMPOSITE_W || swapchain.extent().height < COMPOSITE_H {
        eprintln!(
            "SKIP p0_windowed_coarse_cull: swapchain extent {}x{} is smaller than the {}x{} composite",
            swapchain.extent().width,
            swapchain.extent().height,
            COMPOSITE_W,
            COMPOSITE_H
        );
        return;
    }

    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!("SKIP p0_windowed_coarse_cull: swapchain format has no host-decodable UNORM byte order");
        return;
    };
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP p0_windowed_coarse_cull: swapchain format has no basic-slice Format variant");
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");
    let device: &VulkanContext = &ctx;
    let sdf = sphere_scene();

    // --- The edit-list SSBO (binding 0), host-seeded ONCE. ---
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UBO (binding 5), host-seeded ONCE at the COMPOSITE ORTHO extent. The
    // M4 tail is zero here (brick is held OFF on both readback frames, so the marcher never reads
    // the per-level params; binding 9..=14 still need VALID descriptors below). ---
    let camera_uniform = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H);
        assert_eq!(pc.count, PIXELS);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes; the
        // 80-byte camera block is written at offset 0 (the M4 tail stays zero — brick is OFF). No
        // GPU work is in flight yet, so the host write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // --- The P4b coarse-cull tile StorageBuffer (vocab binding 6), sized to the full tile grid at
    // the COMPOSITE extent. On the OFF frame it is bound-but-unread; on the ON frame the coarse
    // pass WRITES it and the marcher READS it. ---
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // --- The PBR material table SSBO (vocab binding 7 + resolve binding 4). ---
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- The brick clip-map: the brick path is held OFF on both readback frames, but the marcher
    // SPIR-V statically references bindings 9..=14 past the runtime gate, so VALID descriptors must
    // be bound. The real clip-map supplies them (`brick = None` keeps them bound-but-unread). ---
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in &sdf {
            assert!(f.push(*e), "P0 cull scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };
    let clipmap = BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0])
        .expect("brick clip-map (P0 cull scene) — create + bake + upload");

    // --- The Lighting-L0 light table SSBO (resolve binding 6) + its staging source. ---
    let light_table_bytes = (DEGENERATE_LIGHT_TABLE.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }
    let light_staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("Lighting-L0 light table staging buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &DEGENERATE_LIGHT_TABLE);
    }

    // --- The mesh quad's vertex buffer. ---
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices` is a
        // distinct stack array of `vertex_bytes` bytes; the write completes before any submit.
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
    let present_sampler = RhiDevice::create_sampler(
        device,
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
        },
    )
    .expect("present nearest/clamp sampler");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            // Render P5-r0: 3 MRT color formats = the G-buffer RGBA8 lanes; the production
            // `record_gbuffer` binds albedo/normal/material as the 3 MRT attachments.
            color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
            blend: None,
        },
    )
    .expect("depth-prepass graphics pipeline");

    // --- The P1b marcher: the vocabulary layout + the marcher pipeline. The SAME layout is shared
    // by the coarse-cull pipeline below (the cull shader declares only a subset — valid). ---
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    let vocab_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
    let marcher = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
        },
    )
    .expect("P1b G-buffer marcher compute pipeline");

    // --- Render P0: the COARSE-CULL pipeline, created against the SAME vocabulary layout (the
    // offscreen `run_gbuffer_hybrid_ex` discipline — the cull shader declares only a subset of the
    // vocab bindings, so sharing the full layout is valid). ---
    let coarse_cs = RhiDevice::create_shader_module(device, sdf_tile_cull_spirv())
        .expect("P4b coarse-cull compute shader module");
    let coarse_compute = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &coarse_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
        },
    )
    .expect("P4b coarse-cull compute pipeline (shared vocab layout)");

    // --- The deferred RESOLVE pipeline. ---
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
    let resolve_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P6 R1: the deferred resolve binds the SDF edit-list `Buf` at binding 10 (the
        // sdf_soft_shadow_ranged march reads it). The production `record_gbuffer` binds it, so the
        // resolve layout MUST declare it or bind-group creation trips the entry-count check.
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (read under `ssao_mode != 0`; OFF
        // here, bound-but-unread). The production `GBufferTargets` binds the SSAO image at @11,
        // so the resolve layout MUST declare it (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
    let resolve_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
        },
    )
    .expect("deferred resolve compute pipeline");

    // --- The present-blit pipeline. ---
    let present_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::CombinedImageSampler,
                stage: ShaderStage::FRAGMENT,
            }],
        },
    )
    .expect("present-blit bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&present_layout),
            blend: None,
        },
    )
    .expect("present-blit fullscreen-sample pipeline");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its pipeline is
    // created; each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, coarse_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    let mvp = ortho_mvp_bytes();
    let mut scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_uniform: &camera_uniform,
        tiles_buffer: &tiles_buffer,
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        light_dirty: false,
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The brick path is held OFF on BOTH frames so the cull-on-vs-off comparison is isolated.
        brick: None,
        // The cull gate, flipped per readback frame below (None then Some(&coarse_compute)).
        coarse: None,
        // EmptySkipOnly (mode 2) — the LIT-TRANSPARENT cull: EMPTY-skip only, NO `near_t` seed.
        // The empty-tile skip is provably image-identical lit+unlit (an empty tile has no surface);
        // dropping the `near_t` seed on the few NON-empty surface tiles removes the grazing-silhouette
        // AO/shadow rim the FULL mode's seed latches (a shifted grazing tangent → a shifted normal →
        // a shifted AO/shadow). So this mode is transparent UNDER LIGHTING — which is exactly what
        // this test now proves (it renders at the real on-screen lit flags, NOT `0`).
        coarse_mode: CoarseMode::EmptySkipOnly,
        // Lighting ON — the REAL on-screen flags (A1 soft shadows + A2 AO). The previous P0 test set
        // `lighting_flags == 0` to dodge the FULL-mode `near_t` rim (the lit cull-transparency
        // invariant was un-shipped). EmptySkipOnly is lit-transparent BY CONSTRUCTION (no seed → no
        // rim), so the cull-ON vs cull-OFF comparison is now asserted at the real lit flags — proving
        // the on-screen cull adds NO visible rim.
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The legacy head-on shadow direction (`[0,0,1]`): the cull-ON vs cull-OFF comparison must
        // hold the marcher push fixed, so this stays the pre-`light_dir`-field default.
        light_dir: DEFAULT_LIGHT_DIR,
        // Render P7: SSAO OFF (the default) — NO SSAO pass recorded, byte-identical to the pre-P7
        // stream (the 0%-gate). These golden/cull-comparison presents assert the existing stream.
        ssao: None,
    };

    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();
    let mut frame = GBufferFrame::new();

    // Render ONE readback frame at the current `scene.coarse` state + drain so the staging buffer is
    // coherent, then copy it out as an RGBA frame (the SAME R/B normalization the goldens use). The
    // closure mirrors the existing `dump_brick_ab` readback/drain discipline (FRAMES_IN_FLIGHT==2,
    // 3 drain frames re-wait the readback slot's fence). Returns `None` if the swapchain recreated
    // (a resize), in which case the comparison is skipped gracefully.
    const DRAIN_FRAMES: u32 = 3;
    let coarse_pipeline_ref: &boyko_rhi_vulkan::rhi_impl::ComputePipeline = &coarse_compute;
    let mut readback_rgba = |cull_on: bool| -> Option<(Vec<u8>, u32, u32)> {
        if !window.pump_events() {
            return None;
        }
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE p0 cull: extent changed before the readback frame — skipping");
            return None;
        }

        scene.coarse = if cull_on { Some(coarse_pipeline_ref) } else { None };
        let clear = [0.0_f32, 0.0, 0.0, 1.0];

        // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene` resource
        // is live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO `count` cover
        // the composite extent; `staging` is host-visible and ≥ one swapchain image in bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                &ctx, &surface, &mut swapchain, &scene, &mut frame,
                window.width(), window.height(), clear, present_extent, Some(&staging),
            )
        }
        .unwrap_or_else(|e| panic!("p0 cull readback frame (cull_on={cull_on}) failed: {e:?}"));
        if !presented {
            eprintln!("NOTE p0 cull: swapchain recreated on the readback frame — skipping");
            return None;
        }
        let extent = swapchain.extent();

        for _ in 0..DRAIN_FRAMES {
            if !window.pump_events() {
                break;
            }
            window.refresh_size();
            // SAFETY: same contract; no readback requested on the drain frames.
            let _ = unsafe {
                renderer.render_gbuffer_frame(
                    &ctx, &surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, None,
                )
            }
            .unwrap_or_else(|e| panic!("p0 cull drain frame (cull_on={cull_on}) failed: {e:?}"));
        }

        let w = extent.width;
        let h = extent.height;
        let byte_count = (w * h * 4) as usize;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let mut raw = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent bytes;
        // the readback frame's copy completed before this read (its slot fence was re-waited by the
        // drain frames); `raw` is a distinct, non-overlapping alloc.
        unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
        Some((readback_to_rgba(&raw, w, h, is_bgra), w, h))
    };

    let off = readback_rgba(false);
    let on = readback_rgba(true);

    // The validation oracle: the ON path's new coarse dispatch + barrier are sound (zero messages
    // across all frames recorded by the two readbacks). Gated on `validation_enabled()` so the
    // pixel gate below still runs under `BOYKO_DISABLE_VALIDATION` (no messenger when off).
    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the P0 coarse-cull present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // The pixel gate: cull-ON must equal cull-OFF within +/-CHANNEL_TOL per RGB channel (the cull
    // is a PERF optimization — same surface, fewer marches). Both frames are already RGBA-normalized
    // (the swapchain R/B swap applied), so they are byte-comparable per channel.
    match (off, on) {
        (Some((off_rgba, ow, oh)), Some((on_rgba, nw, nh))) => {
            assert_eq!((ow, oh), (nw, nh), "cull-ON and cull-OFF readback extents must match");
            assert_eq!(
                off_rgba.len(),
                on_rgba.len(),
                "cull-ON and cull-OFF readback byte lengths must match"
            );
            let mut mismatches = 0usize;
            let mut worst = (0u32, 0u32, 0i32);
            for (i, (o, n)) in off_rgba.chunks_exact(4).zip(on_rgba.chunks_exact(4)).enumerate() {
                let mut bad = false;
                for c in 0..3 {
                    let d = (o[c] as i32 - n[c] as i32).abs();
                    if d > CHANNEL_TOL {
                        bad = true;
                        if d > worst.2 {
                            let px = (i as u32) % ow;
                            let py = (i as u32) / ow;
                            worst = (px, py, d);
                        }
                    }
                }
                if bad {
                    mismatches += 1;
                }
            }
            assert_eq!(
                mismatches, 0,
                "P0 coarse cull changed {mismatches} pixel(s) beyond +/-{CHANNEL_TOL} (worst delta \
                 {} at ({}, {})) — the cull must skip empty tiles only, NOT alter the surface",
                worst.2, worst.0, worst.1,
            );
            println!("p0_windowed_coarse_cull: cull-ON == cull-OFF across {ow}x{oh} (0 mismatches)");
        }
        _ => {
            eprintln!(
                "NOTE p0_windowed_coarse_cull: a readback frame did not present (swapchain kept \
                 recreating); validation was still asserted clean"
            );
        }
    }

    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no submission
    // references these resources; `ctx` is still alive; each is destroyed exactly once, in reverse
    // dependency order.
    unsafe {
        frame.destroy(&ctx);
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, coarse_compute);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        clipmap.destroy(&ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        RhiDevice::destroy_buffer(device, camera_uniform);
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}

// ============================================================================
// Engine showcase — a CRISP 512×512-NATIVE multi-light SDF-shadow screenshot.
//
// This drives the EXACT production windowed present (`Renderer::render_gbuffer_frame`,
// the same 3-pass raster-MRT → marcher → deferred-resolve → present-blit) at the native
// `COMPOSITE_W`×`COMPOSITE_H` (512×512) extent and dumps ONE true-resolution BMP (no
// upscale) so the owner can judge the render. Unlike the offscreen screenshot tests
// (hardwired to 64×64, then 8× upscaled → blocky), this is the windowed path that renders
// at the full composite extent.
//
// The scene is the P5 hybrid (a raster-PBR mesh + an SDF body) lit by P6 R1 multi-light
// SDF shadows: a neutral primary directional plus TWO shadow-flagged POINT casters of
// DISTINCT colors (a warm orange and a cool blue). The light table is `shadow_mode == 1`
// + NON-CLUSTERED — exactly the path `p6_r1_multi_light_sdf_shadows_match_oracle` validates
// offscreen, here re-seeded into the windowed `light_table` SSBO (no production-code change:
// the resolve reads `shadow_mode`/`casts_sdf_shadow` from the table header/elements, and the
// windowed `record_gbuffer` already binds the SDF edit-list at resolve binding 10, so the
// per-caster `sdf_soft_shadow_ranged` march runs on hardware).
// ============================================================================

/// The fixed dump path for the 512-native engine showcase frame.
const SHOWCASE_BMP: &str = r"D:\tmp\engine_showcase_512.bmp";

/// The fixed dump path for the 512-native engine SSAO showcase frame (the SAME scene with SSAO
/// ON, dumped under an SSAO-labelled path the orchestrator converts + shows the owner).
const SSAO_BMP: &str = r"D:\tmp\engine_ssao_512.bmp";

/// The showcase sun direction (`L`, the un-normalized "direction TO the light"): upper-LEFT and
/// slightly toward the camera, ~57° elevation. Used BOTH as the marcher's `scene.light_dir` (the
/// A1 cast-shadow march direction) AND the primary directional in [`showcase_light_table`] — they
/// MUST match so the shadow the marcher bakes into `gMaterial.r` lands where the resolve lights
/// from. With this `+y`-dominant `L` the up-facing floor is well-lit (NoL ≈ 0.85) and the
/// sphere/box throw a clear elongated shadow back-and-right across the floor, visible to the
/// down-looking camera.
const SHOWCASE_SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// The showcase SDF body: a clean, realistic studio scene — a wide flat **floor slab** with a
/// **sphere**, a **cube**, and a smaller **sphere** RESTING on it (each primitive's base sits at
/// the slab's top face, `y = -0.5`). The perspective camera ([`showcase_camera`]) looks down at the
/// floor from the front, the warm directional sun ([`SHOWCASE_SUN_DIR`]) rakes from the upper-left,
/// and the marcher's A1 soft shadow casts each body's shadow ACROSS the floor — the floor-and-cast-
/// shadow composition a head-on ortho twin-sphere scene cannot show. Mid-gray dielectric (material
/// slot 0) throughout, so the shape reads from lighting + shadow, not color.
fn showcase_sdf_scene() -> Vec<SdfEdit> {
    vec![
        // The floor: a wide thin slab centered below the origin; its top face is at y = -0.5.
        SdfEdit::box_shape([0.0, -1.0, 0.0], [5.0, 0.5, 4.0], sdf_op::UNION, 0.0),
        // The hero sphere, resting on the floor (center y = -0.5 + r), a touch toward the camera.
        SdfEdit::sphere([0.0, 0.0, 0.2], 0.50, sdf_op::UNION, 0.0),
        // A cube to the left, resting on the floor (center y = -0.5 + half).
        SdfEdit::box_shape([-1.30, -0.18, -0.40], [0.32, 0.32, 0.32], sdf_op::UNION, 0.0),
        // A smaller sphere to the right, resting on the floor.
        SdfEdit::sphere([1.30, -0.22, -0.30], 0.28, sdf_op::UNION, 0.0),
    ]
}

/// The showcase perspective camera: eye in FRONT and ABOVE the scene (`+Z`, `+Y`), looking DOWN at
/// the floor and the hero sphere — so the floor recedes, the bodies sit on it, and their cast
/// shadows are visible. 50° vertical FOV. The basis is the standard non-rolled right-handed frame
/// (`right = [1,0,0]`, `up = right × forward`). The whole scene sits within the marcher's
/// `SDF_TRACE_T_MAX` (≈10) ray range so the finite floor's far edge renders against the dark
/// background.
fn showcase_camera() -> CompositePushConstants {
    // eye = [0, 1.9, 4.0], target = [0, -0.15, -0.30] → forward = normalize(target - eye).
    let forward = [0.0_f32, -0.43035, -0.90266];
    let right = [1.0_f32, 0.0, 0.0];
    let up = [0.0_f32, 0.90266, -0.43035]; // right × forward (unit)
    CompositePushConstants::perspective(
        [0.0, 1.9, 4.0],
        forward,
        right,
        up,
        core::f32::consts::FRAC_PI_3 * 5.0 / 6.0, // 50° vertical FOV (π/3 · 5/6)
        COMPOSITE_W,
        COMPOSITE_H,
    )
}

/// The showcase raster mesh is DEGENERATE (zero-area): the realistic showcase is ALL-SDF — the
/// floor is an SDF slab marched by the perspective camera, so a raster floor (which the harness
/// projects with the ORTHO `ortho_mvp_bytes` MVP) would land in the wrong place and double the
/// floor. Six identical vertices ⇒ two zero-area triangles ⇒ NO fragments ⇒ the raster pass only
/// clears the depth attachment to far (`MESH_DEPTH_CLEAR`), so `has_mesh == false` for every pixel
/// and the marcher OWNS the whole frame (the SDF floor + bodies).
fn showcase_quad_vertices() -> Vec<Vertex> {
    let v = Vertex { position: [0.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0], color: [1.0, 1.0, 1.0, 1.0] };
    vec![v, v, v, v, v, v]
}

/// The showcase light table: a single warm-white directional **sun** (the PRIMARY directional —
/// its visibility reads the marcher's `gMaterial.r`, i.e. the A1 soft shadow the marcher marched
/// toward [`SHOWCASE_SUN_DIR`], so the bodies cast a real shadow across the floor) + a soft
/// **sky/hemisphere ambient** fill that lifts the shadowed floor off pure black and tints it
/// cool (sky) vs the warm sun — the warm-key/cool-fill contrast of a realistic render. NON-
/// CLUSTERED, `shadow_mode == 0` (no per-caster march — the cast shadow is the primary
/// directional's `gMaterial.r`). `l0a_count == 2` (sun + sky), `point_spot_count == 0`.
fn showcase_light_table() -> (GoldenLightHeader, Vec<GoldenLight>) {
    let header = GoldenLightHeader::new(2, 0, 1.0);
    let lights = vec![
        // The sun: warm white, raking from the upper-left ([`SHOWCASE_SUN_DIR`] — the SAME vector
        // as `scene.light_dir`, so the marched cast shadow matches the lit direction). Illuminance
        // tuned so the mid-gray floor lights to ~0.7 without clipping to white.
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.96, 0.90], 2.8),
        // Sky/hemisphere ambient: a cool-blue sky over a warm-dark ground, so the cast shadow is a
        // readable cool gray (not black) and the contact AO still darkens it.
        GoldenLight::sky([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]),
    ];
    (header, lights)
}

/// Packs a `GoldenLightHeader` + `GoldenLight[]` into the std430 light-table SSBO word stream
/// (`[header (16 words) || GpuLight[] (12 words each)]`) the resolve reads at binding 6.
/// Host mirror of `boyko_render::light`'s packing; identical to the offscreen test's
/// `pack_light_table`.
fn pack_showcase_light_table(header: &GoldenLightHeader, lights: &[GoldenLight]) -> Vec<u32> {
    let mut words = vec![0u32; GOLDEN_LIGHT_HEADER_BASE_WORDS + lights.len() * 12];
    let lanes = [
        header.counts_exposure,
        header.sky_diffuse,
        header.sky_spec,
        header.cluster_params,
    ];
    for (li, lane) in lanes.iter().enumerate() {
        for (c, &v) in lane.iter().enumerate() {
            words[li * 4 + c] = v.to_bits();
        }
    }
    for (i, l) in lights.iter().enumerate() {
        let base = GOLDEN_LIGHT_HEADER_BASE_WORDS + i * 12;
        for (c, &v) in l.dir_kind.iter().enumerate() {
            words[base + c] = v.to_bits();
        }
        for (c, &v) in l.pos_range.iter().enumerate() {
            words[base + 4 + c] = v.to_bits();
        }
        for (c, &v) in l.color_cone.iter().enumerate() {
            words[base + 8 + c] = v.to_bits();
        }
    }
    words
}

/// Reads back a BI_RGB BMP's `biWidth` / `biHeight` (`biHeight` is negative for the top-down
/// images [`write_bmp`] emits, so the magnitude is the height). Returns `None` if the file is
/// not a `"BM"` 54-byte-header BMP. Used to VERIFY the dumped showcase is 512×512 native.
fn read_bmp_dimensions(bytes: &[u8]) -> Option<(i32, i32)> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return None;
    }
    let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    Some((w, h.abs()))
}

/// The fixed dump path for the 512-native engine MESH-floor SSAO showcase frame (Render P7
/// Unlock-2): a flat RASTER MESH quad + an SDF sphere standing in front, SSAO ON — the mesh
/// (A2 == 1.0) visibly receives the sphere's contact AO, which the A2 SDF-march cannot give it.
const SSAO_MESH_BMP: &str = r"D:\tmp\engine_ssao_mesh_512.bmp";

/// Render P7-Q2 — the SSAO quality-LADDER BMP dump paths (the SAME mesh+SDF scene rendered with SSAO
/// OFF / Low / Medium / High, so the orchestrator converts + shows the owner the quality ladder).
const SSAO_LADDER_OFF_BMP: &str = r"D:\tmp\engine_ssao_off.bmp";
const SSAO_LADDER_LOW_BMP: &str = r"D:\tmp\engine_ssao_low.bmp";
const SSAO_LADDER_MEDIUM_BMP: &str = r"D:\tmp\engine_ssao_medium.bmp";
const SSAO_LADDER_HIGH_BMP: &str = r"D:\tmp\engine_ssao_high.bmp";

/// The fixed dump path for the ORTHO HYBRID-ROOM showcase (multi-object mesh + SDF, step 1 of
/// the hybrid-mesh-room build): a mesh backdrop wall + mesh cubes in front + SDF bodies casting
/// the marcher's analytic shadow/AO onto the mesh. The orchestrator runs the GPU test + converts.
const HYBRID_BMP: &str = r"D:\tmp\engine_hybrid_room.bmp";

/// Render Shadow Phase 1 — the capsule-character screenshot path: a coarse 6-capsule
/// humanoid (a character capsule-proxy) standing on the mesh floor in front of the back
/// wall, casting the marcher's analytic SDF shadow onto the mesh as a readable humanoid
/// silhouette. The orchestrator runs the GPU test + converts the BMP.
const CAPSULE_CHARACTER_BMP: &str = r"D:\tmp\engine_capsule_character.bmp";

/// Render Shadow Phase 3 — the Screen-Space Contact Shadows (SSCS) A/B screenshot paths: the
/// SAME capsule character feet-on-floor scene, dumped with `contact_shadow_mode` OFF (the A/B
/// reference + the 0%-gate visual proof) and ON (the contact-shadow tightening visible where
/// the feet meet the floor). The orchestrator runs the GPU test + converts the BMPs.
const CONTACT_SHADOW_OFF_BMP: &str = r"D:\tmp\engine_contact_shadow_off.bmp";
const CONTACT_SHADOW_ON_BMP: &str = r"D:\tmp\engine_contact_shadow.bmp";

/// The per-showcase variable scene: the SDF edit list, the marcher/resolve camera push, the light
/// table (header + elements), and the RASTER MESH (vertices + MVP). The shared [`run_showcase_dump`]
/// body holds everything else (pipelines, barriers, the dump tail) constant. Built by the per-test
/// builders so the all-SDF perspective showcase and the ORTHO mesh-floor SSAO showcase share ONE
/// recorder/dump body without duplicating its ~400 lines.
struct ShowcaseConfig {
    /// The SDF edit list (the marcher field + the resolve per-caster shadow march).
    sdf: Vec<SdfEdit>,
    /// The marcher + resolve + SSAO camera push (perspective for the all-SDF showcase; ORTHO for
    /// the mesh-floor showcase, whose `md * T_MAX == t_mesh` decode the raster MVP below matches).
    camera: CompositePushConstants,
    /// The light table (already `ssao_mode`-armed by the builder).
    light_header: GoldenLightHeader,
    /// The light table elements (directional + sky [+ point/spot]).
    light_elems: Vec<GoldenLight>,
    /// The raster mesh vertices (DEGENERATE zero-area for the all-SDF showcase; a real floor quad
    /// for the mesh-floor showcase; an arbitrary multi-object mesh for the hybrid room). A `Vec`
    /// so any vertex count is supported — the draw is length-driven (`vertex_count == len`).
    vertices: Vec<Vertex>,
    /// The raster MVP push (the ORTHO `ortho_mvp_bytes` — its `(CAM_Z - z)/T_MAX` depth is the
    /// convention the marcher's `t_mesh = md * T_MAX` ownership/gViewT decode reconstructs exactly).
    mvp: [u8; MVP_BYTES as usize],
    /// Render P7-Q2 — the SSAO state: `Some(quality)` records the SSAO pass binding that variant's
    /// pre-compiled `.spv` (an `SSAO_QUALITY_*` index) AND arms `ssao_mode == 1`; `None` is SSAO OFF
    /// (no SSAO pass recorded — `scene.ssao = None` — AND `ssao_mode == 0`, the byte-identical 0%-gate
    /// reference for the ladder's `_off` frame). The builder sets the `ssao_mode` on `light_header`.
    ssao_quality: Option<usize>,
}

/// The default all-SDF perspective showcase config (the historical [`run_showcase_dump`] scene):
/// the SDF floor + bodies, the down-looking [`showcase_camera`], the multi-light table, and the
/// DEGENERATE zero-area raster mesh (so the marcher owns the whole frame). `ssao_quality`:
/// `Some(SSAO_QUALITY_*)` arms the SSAO pass at that variant; `None` is SSAO OFF (the 0%-gate
/// reference — `ssao_mode == 0`, no SSAO pass).
fn showcase_config(ssao_quality: Option<usize>) -> ShowcaseConfig {
    let (light_header, light_elems) = showcase_light_table();
    let ssao_mode = if ssao_quality.is_some() { 1 } else { 0 };
    ShowcaseConfig {
        sdf: showcase_sdf_scene(),
        camera: showcase_camera(),
        light_header: light_header.with_ssao_mode(ssao_mode),
        light_elems,
        vertices: showcase_quad_vertices(),
        mvp: ortho_mvp_bytes(),
        ssao_quality,
    }
}

/// **Engine showcase — a CRISP 512×512-NATIVE multi-light SDF-shadow screenshot.**
///
/// Renders the production windowed present (the raster-PBR mesh + the SDF twin-sphere body)
/// at the native 512×512 composite extent, lit by 1 directional + 2 shadow-flagged colored
/// point casters (`shadow_mode == 1`, NON-CLUSTERED), reads back the 512 frame, and writes a
/// TRUE 512×512 24-bit BMP to [`SHOWCASE_BMP`] — NO upscaling. Verifies the dumped BMP header
/// is 512×512. The orchestrator converts it to PNG + opens it for the owner.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshot is the
/// deliverable, not a golden assertion.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the screenshot"]
fn engine_showcase_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine showcase 512",
        SHOWCASE_BMP,
        showcase_config(Some(SSAO_QUALITY_MEDIUM)),
    );
}

/// **Engine SSAO showcase — the SAME crisp 512×512-native scene WITH SSAO ON, dumped to
/// [`SSAO_BMP`].** Identical to [`engine_showcase_512_screenshot_dump`] (the showcase already
/// arms `ssao_mode == 1` + `scene.ssao = Some(..)`, so the SSAO contact-crease darkening is in the
/// frame) — this sibling writes the SSAO-labelled BMP path the orchestrator converts + shows the
/// owner for the SSAO A/B visual sign-off.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the SSAO screenshot"]
fn engine_ssao_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine SSAO 512",
        SSAO_BMP,
        showcase_config(Some(SSAO_QUALITY_MEDIUM)),
    );
}

/// **Render P7 Unlock-2 — engine MESH-FLOOR SSAO showcase (the visual).** A REAL RASTER MESH quad
/// floor (the ORTHO [`quad_vertices`] at `MESH_Z == 1.0`, whose A2 `gMaterial.g` == 1.0 — the
/// raster has no analytic SDF AO) + an SDF sphere standing IN FRONT of it ([`mesh_ssao_sphere`],
/// near pole at `z == 1.55 > MESH_Z`), lit + SSAO ON. The sphere casts CONTACT AO onto the mesh
/// around its silhouette — darkening the A2 SDF-march CANNOT produce on the mesh (its A2 == 1.0,
/// so SSAO is its only AO). Dumps a TRUE 512×512 BMP to [`SSAO_MESH_BMP`].
///
/// This is the ORTHO camera (matching the offscreen non-vacuity gate
/// `ssao_darkens_mesh_near_sdf_occluder`), so the raster MVP's `(CAM_Z - z)/T_MAX` depth is exactly
/// the convention the marcher's `t_mesh = md * T_MAX` ownership + gViewT decode reconstructs — no
/// perspective-MVP-vs-ray-gen alignment is needed. (The full PERSPECTIVE mesh-floor MVP is deferred:
/// the marcher decodes mesh depth as `md * T_MAX`, a LINEAR-in-ray-distance convention a standard
/// perspective projection's nonlinear NDC depth does not satisfy, so a perspective mesh floor would
/// need a custom depth-writing VS or a marcher decode change — both out of this pass's scope. The
/// ORTHO mesh floor delivers the same mesh-receives-SSAO visual with an exactly aligned gate.)
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the mesh-floor SSAO screenshot"]
fn engine_ssao_mesh_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine SSAO mesh floor 512",
        SSAO_MESH_BMP,
        mesh_ssao_config(Some(SSAO_QUALITY_MEDIUM)),
    );
}

/// **Render P7-Q2 — engine SSAO QUALITY-LADDER screenshot dump (the visual oracle).** Renders the
/// SAME mesh+SDF SSAO scene ([`mesh_ssao_config`] — the raster mesh quad floor + the SDF sphere in
/// front) FOUR times and dumps a TRUE 512×512 BMP per quality so the orchestrator converts + shows
/// the owner the ladder:
///   - [`SSAO_LADDER_OFF_BMP`] — SSAO OFF (`scene.ssao = None`, `ssao_mode == 0`, the 0%-gate frame).
///   - [`SSAO_LADDER_LOW_BMP`] — the Low variant pipeline (2×3×2 = 12 taps).
///   - [`SSAO_LADDER_MEDIUM_BMP`] — the Medium variant (2×4×2 = 16 taps; == today's shipped path).
///   - [`SSAO_LADDER_HIGH_BMP`] — the High variant (3×6×2 = 36 taps).
///
/// Each is a fresh windowed render (`run_showcase_dump` boots + tears down its own device per call),
/// so the ladder is four independent frames the owner compares side-by-side (the mesh contact-AO ring
/// sharpens / spreads with the tap budget; OFF is the no-AO baseline).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the SSAO quality ladder"]
fn engine_ssao_ladder_off_dump() {
    // ONE window/context per process: a windowed boot only survives the FIRST showcase dump in a
    // process (later boots hit "swapchain kept recreating"), so each ladder rung is its OWN test —
    // the orchestrator runs them in separate processes.
    run_showcase_dump("boyko_engine SSAO ladder OFF", SSAO_LADDER_OFF_BMP, mesh_ssao_config(None));
}

/// SSAO ladder rung — LOW (2x3). See [`engine_ssao_ladder_off_dump`] for the one-per-process note.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_low_dump() {
    run_showcase_dump("boyko_engine SSAO ladder LOW", SSAO_LADDER_LOW_BMP, mesh_ssao_config(Some(SSAO_QUALITY_LOW)));
}

/// SSAO ladder rung — MEDIUM (2x4, == today). See [`engine_ssao_ladder_off_dump`].
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_medium_dump() {
    run_showcase_dump("boyko_engine SSAO ladder MEDIUM", SSAO_LADDER_MEDIUM_BMP, mesh_ssao_config(Some(SSAO_QUALITY_MEDIUM)));
}

/// SSAO ladder rung — HIGH (3x6). See [`engine_ssao_ladder_off_dump`].
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU"]
fn engine_ssao_ladder_high_dump() {
    run_showcase_dump("boyko_engine SSAO ladder HIGH", SSAO_LADDER_HIGH_BMP, mesh_ssao_config(Some(SSAO_QUALITY_HIGH)));
}

/// **Hybrid-room screenshot dump (step 1 of the hybrid-mesh-room build).** Renders the ORTHO
/// hybrid room ([`hybrid_room_config`]: an arbitrary multi-object mesh — a backdrop wall + 3 cubes
/// with per-vertex face normals — plus several SDF bodies standing in front so the marcher's
/// analytic shadows + AO fall on the mesh) at the native 512×512 composite extent and writes a
/// TRUE 512 BMP to [`HYBRID_BMP`]. Proves the multi-object-mesh + per-vertex-normal infra on the
/// PROVEN ORTHO path (the orchestrator adds the perspective camera in step 2).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1`.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the hybrid-room screenshot"]
fn engine_hybrid_room_512_screenshot_dump() {
    run_showcase_dump("boyko_engine hybrid room 512", HYBRID_BMP, hybrid_room_config());
}

/// The unlock-2 SDF occluder sphere for the mesh-floor showcase: ONE sphere standing in FRONT of
/// the mesh quad (`MESH_Z == 1.0`) — center at `+Z`, near pole at `z == 1.55 > MESH_Z`, so the SDF
/// wins ownership where it covers and the mesh stands elsewhere (the SAME geometry as the offscreen
/// `ssao_darkens_mesh_near_sdf_occluder` gate, lifted to the 512 composite).
fn mesh_ssao_sphere() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.95], 0.60, sdf_op::UNION, 0.0)]
}

/// The ORTHO mesh-floor SSAO showcase config (Render P7 Unlock-2): the real raster mesh quad floor,
/// the SDF sphere in front, the ORTHO camera, and the SSAO-armed showcase light table.
/// `ssao_quality`: `Some(SSAO_QUALITY_*)` arms the SSAO pass at that variant; `None` is SSAO OFF.
fn mesh_ssao_config(ssao_quality: Option<usize>) -> ShowcaseConfig {
    let (light_header, light_elems) = showcase_light_table();
    let ssao_mode = if ssao_quality.is_some() { 1 } else { 0 };
    ShowcaseConfig {
        sdf: mesh_ssao_sphere(),
        camera: CompositePushConstants::ortho(COMPOSITE_W, COMPOSITE_H),
        light_header: light_header.with_ssao_mode(ssao_mode),
        light_elems,
        // A REAL floor quad (NOT the degenerate all-SDF mesh): its A2 == 1.0, so the contact AO is
        // pure SSAO. The ORTHO MVP lands it exactly where the marcher's `t_mesh` decode expects.
        vertices: quad_vertices().to_vec(),
        mvp: ortho_mvp_bytes(),
        ssao_quality,
    }
}

// === The 3D hybrid room — PERSPECTIVE step-2 named consts (orchestrator-tunable). ===
// All positions are world-space, y-up. The mesh floor (y = 0) + back wall (z = -4) +
// 2 mesh cubes RESTING on the floor form the room; 3 SDF bodies rest on the floor in
// front and cast the marcher's analytic shadow/AO onto the mesh.

/// The room camera EYE (world). Above + in front, looking down into the room.
const ROOM_CAM_EYE: [f32; 3] = [0.0, 3.2, 4.5];
/// The room camera LOOK-AT target (world).
const ROOM_CAM_TARGET: [f32; 3] = [0.0, 0.8, -1.5];
/// The room camera vertical FOV (radians) — 50°.
const ROOM_CAM_FOV_Y: f32 = 50.0 * core::f32::consts::PI / 180.0;
/// The room camera right-handed basis, precomputed from EYE/TARGET (verified orthonormal by
/// the `debug_assert!` in [`room_camera`]). forward = normalize(target - eye);
/// right = normalize(cross(forward, +Y)); up = cross(right, forward).
const ROOM_CAM_FORWARD: [f32; 3] = [0.0, -0.371391, -0.928477];
const ROOM_CAM_RIGHT: [f32; 3] = [1.0, 0.0, 0.0];
const ROOM_CAM_UP: [f32; 3] = [0.0, 0.928477, -0.371391];

/// The 2 mesh cubes resting on the floor: center / half-extent / color. Each bottom face sits a
/// hair (0.01) ABOVE the floor plane (y=0) — a coplanar bottom would Z-fight the floor under
/// `LESS` with no depth bias (the jagged contact line). 0.01 is sub-pixel at this camera distance.
const ROOM_CUBE_A: ([f32; 3], [f32; 3], [f32; 4]) =
    ([-1.6, 0.51, -1.5], [0.5, 0.5, 0.5], [0.80, 0.34, 0.28, 1.0]); // warm terracotta
const ROOM_CUBE_B: ([f32; 3], [f32; 3], [f32; 4]) =
    ([1.4, 0.36, -2.2], [0.35, 0.35, 0.35], [0.28, 0.46, 0.78, 1.0]); // cool blue

/// The 3 SDF bodies resting on the floor (center.y = radius / half-height): a hero sphere,
/// a smaller sphere, and a box. HARD unions (4th arg `0.0` = SMOOTHNESS, mid-gray material 0).
const ROOM_SDF_SPHERE_A: ([f32; 3], f32) = ([0.0, 0.7, -1.0], 0.7);
const ROOM_SDF_SPHERE_B: ([f32; 3], f32) = ([1.5, 0.5, -0.5], 0.5);
const ROOM_SDF_BOX: ([f32; 3], [f32; 3]) = ([-1.2, 0.51, 0.2], [0.5, 0.5, 0.5]); // bottom 0.01 above the floor: a coplanar bottom Z-fought the mesh floor (the "strange front shadow")

/// The mesh floor + wall colors (neutral grays; the wall a touch different so the corner reads).
const ROOM_FLOOR_COLOR: [f32; 4] = [0.55, 0.55, 0.57, 1.0];
const ROOM_WALL_COLOR: [f32; 4] = [0.45, 0.46, 0.50, 1.0];

/// The room camera push: a PERSPECTIVE [`CompositePushConstants`] matching the
/// [`perspective_mvp_bytes`] the raster mesh uses (same eye / basis / fov / aspect). The
/// `debug_assert!` guards the precomputed basis (unit, orthogonal, right-handed) against an
/// edit of the EYE/TARGET consts that forgets to recompute the basis.
fn room_camera() -> CompositePushConstants {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (f, r, u) = (ROOM_CAM_FORWARD, ROOM_CAM_RIGHT, ROOM_CAM_UP);
    // The precomputed `forward` must equal normalize(TARGET - EYE) (guards an EYE/TARGET edit
    // that forgets to recompute), and the basis must be orthonormal + right-handed.
    let raw = [
        ROOM_CAM_TARGET[0] - ROOM_CAM_EYE[0],
        ROOM_CAM_TARGET[1] - ROOM_CAM_EYE[1],
        ROOM_CAM_TARGET[2] - ROOM_CAM_EYE[2],
    ];
    let inv_len = 1.0 / dot(raw, raw).sqrt();
    let fwd = [raw[0] * inv_len, raw[1] * inv_len, raw[2] * inv_len];
    debug_assert!(
        (f[0] - fwd[0]).abs() < 1e-3
            && (f[1] - fwd[1]).abs() < 1e-3
            && (f[2] - fwd[2]).abs() < 1e-3
            && (dot(f, f) - 1.0).abs() < 1e-3
            && (dot(r, r) - 1.0).abs() < 1e-3
            && (dot(u, u) - 1.0).abs() < 1e-3
            && dot(f, r).abs() < 1e-3
            && dot(f, u).abs() < 1e-3
            && dot(r, u).abs() < 1e-3,
        "invariant: ROOM_CAM_* basis must be orthonormal + match normalize(TARGET - EYE) \
         (recompute the basis consts after editing EYE/TARGET)"
    );
    CompositePushConstants::perspective(
        ROOM_CAM_EYE,
        ROOM_CAM_FORWARD,
        ROOM_CAM_RIGHT,
        ROOM_CAM_UP,
        ROOM_CAM_FOV_Y,
        COMPOSITE_W,
        COMPOSITE_H,
    )
}

/// The 3D-room SDF bodies (step 2, PERSPECTIVE): a sphere + a smaller sphere + a box, each
/// RESTING on the mesh floor (`center.y == radius / half-height`), standing in the room so the
/// marcher's analytic soft shadow + contact AO fall on the mesh floor, wall, and cubes. HARD
/// unions (4th arg `0.0` is SMOOTHNESS, not a material — the bodies are mid-gray material 0).
fn hybrid_room_sdf_scene() -> Vec<SdfEdit> {
    // SHADOW-CASTER PROXY for a mesh cube: a HARD-union SDF box at the cube's exact center,
    // shrunk by `PROXY_MARGIN` per axis. It is NEVER rendered — the raster mesh cube (larger) wins
    // the marcher ownership at every shared pixel (the mesh surface is `PROXY_MARGIN` in FRONT of
    // the proxy), so the visible cube stays polygonal. But the proxy IS in the FROZEN field, so
    // every OTHER surface's analytic soft-shadow march toward the light hits it → the mesh cube
    // casts a clean SDF shadow onto the floor/wall. This is the MAX-PERF mesh-shadow path in an
    // SDF-first engine: it piggybacks the already-running march (+1 edit each) instead of adding a
    // whole separate shadow-map pass. The `SHADOW_NORMAL_BIAS` lift keeps the cube's OWN lit faces
    // clear of self-shadow (the march starts outside the proxy and travels away from it).
    const PROXY_MARGIN: f32 = 0.02;
    let proxy = |center: [f32; 3], half: [f32; 3]| {
        SdfEdit::box_shape(
            center,
            [half[0] - PROXY_MARGIN, half[1] - PROXY_MARGIN, half[2] - PROXY_MARGIN],
            sdf_op::UNION,
            0.0,
        )
    };
    vec![
        SdfEdit::sphere(ROOM_SDF_SPHERE_A.0, ROOM_SDF_SPHERE_A.1, sdf_op::UNION, 0.0),
        SdfEdit::sphere(ROOM_SDF_SPHERE_B.0, ROOM_SDF_SPHERE_B.1, sdf_op::UNION, 0.0),
        SdfEdit::box_shape(ROOM_SDF_BOX.0, ROOM_SDF_BOX.1, sdf_op::UNION, 0.0),
        // Invisible shadow-caster proxies under the 2 mesh cubes (≤ MAX_SDF_EDITS: 5 edits total).
        proxy(ROOM_CUBE_A.0, ROOM_CUBE_A.1),
        proxy(ROOM_CUBE_B.0, ROOM_CUBE_B.1),
    ]
}

/// The 3D-room MESH geometry (step 2, PERSPECTIVE): a horizontal FLOOR quad at y = 0 (outward
/// normal `+Y`), a vertical BACK-WALL quad at z = -4 (outward normal `+Z`), and 2 mesh CUBES
/// resting on the floor (distinct positions / sizes / colors). All concatenated into one
/// `Vec<Vertex>` — the draw is length-driven. The cubes + floor + wall carry real per-vertex
/// face normals for the G-buffer; the SDF bodies ([`hybrid_room_sdf_scene`]) shadow them.
fn hybrid_room_mesh() -> Vec<Vertex> {
    let mut verts = Vec::new();

    // Floor: a horizontal quad at y = 0 spanning x[-3,3] z[-4,1], outward normal +Y. Corners
    // CCW as seen from +Y (above): looking down the -Y axis.
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, 1.0], [3.0, 0.0, 1.0], [3.0, 0.0, -4.0], [-3.0, 0.0, -4.0]],
        [0.0, 1.0, 0.0],
        ROOM_FLOOR_COLOR,
    ));

    // Back wall: a vertical quad at z = -4 spanning x[-3,3] y[0,4], outward normal +Z. Corners
    // CCW as seen from +Z (in front of the wall).
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, -4.0], [3.0, 0.0, -4.0], [3.0, 4.0, -4.0], [-3.0, 4.0, -4.0]],
        [0.0, 0.0, 1.0],
        ROOM_WALL_COLOR,
    ));

    // 2 cubes resting on the floor (bottom at y = 0).
    verts.extend(mesh_box(ROOM_CUBE_A.0, ROOM_CUBE_A.1, ROOM_CUBE_A.2));
    verts.extend(mesh_box(ROOM_CUBE_B.0, ROOM_CUBE_B.1, ROOM_CUBE_B.2));

    verts
}

/// **Hybrid-room showcase — a PERSPECTIVE 3D room (step 2 of the hybrid-mesh-room build).**
/// A real 3D room: a mesh FLOOR + BACK WALL + 2 mesh cubes ([`hybrid_room_mesh`]) under a
/// PERSPECTIVE camera ([`room_camera`], matched by the [`perspective_mvp_bytes`] raster MVP),
/// with 3 SDF bodies ([`hybrid_room_sdf_scene`]) resting on the floor that cast the marcher's
/// analytic SHADOWS + AO onto the mesh. Analytic path: `ssao_quality: None`, `lighting_flags`
/// SHADOWS|AO (set by [`run_showcase_dump`]'s shared body), 1 directional sun ([`SHOWCASE_SUN_DIR`])
/// + 1 dim sky.
fn hybrid_room_config() -> ShowcaseConfig {
    let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(0);
    let lights = vec![
        // The sun: the recorder's hardcoded `SHOWCASE_SUN_DIR` so the marcher's shadow march
        // matches the resolve's primary directional. Strong illuminance.
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        // A dim neutral sky/hemisphere fill so the shadowed floor reads off pure black.
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];
    ShowcaseConfig {
        sdf: hybrid_room_sdf_scene(),
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: hybrid_room_mesh(),
        mvp: perspective_mvp_bytes(
            ROOM_CAM_EYE,
            ROOM_CAM_FORWARD,
            ROOM_CAM_RIGHT,
            ROOM_CAM_UP,
            ROOM_CAM_FOV_Y,
            COMPOSITE_W as f32 / COMPOSITE_H as f32,
        ),
        ssao_quality: None,
    }
}

// === Render Shadow Phase 1 — the capsule-character proxy demo. ===

/// The capsule cap radius for the character limbs (a coarse humanoid; the torso uses a
/// thicker radius below). Small enough that 6 capsules read as a stick-figure silhouette.
const CHAR_LIMB_RADIUS: f32 = 0.09;
/// The torso cap radius — thicker than the limbs so the body reads as a trunk.
const CHAR_TORSO_RADIUS: f32 = 0.16;
/// The head cap radius.
const CHAR_HEAD_RADIUS: f32 = 0.17;

/// A COARSE 6-capsule humanoid character proxy standing on the floor (feet at y = 0),
/// rooted at world `root` (the feet midpoint), scaled to total `height`, facing the
/// `facing` heading (radians about +Y; 0 faces +Z toward the camera). An ASYMMETRIC pose
/// — one leg forward / one back, one arm out / one down — so the cast SDF shadow reads
/// unmistakably as a humanoid rather than a blob.
///
/// The 6 capsules: torso (hip→shoulder), head (neck→crown), left+right legs (hip→foot),
/// left+right arms (shoulder→hand). HARD unions, smoothness 0.0 (a crisp silhouette).
/// `≤ MAX_SDF_EDITS` (6 edits) with room to spare.
fn character_capsules(root: [f32; 3], height: f32, facing: f32) -> Vec<SdfEdit> {
    // Proportions as fractions of `height` (a coarse 7.5-head canon, simplified).
    let hip_y = 0.50 * height; // pelvis
    let shoulder_y = 0.82 * height;
    let neck_y = 0.84 * height;
    let crown_y = 1.00 * height;
    let foot_y = 0.0; // on the floor
    let hand_y = 0.42 * height;

    // The facing basis in the xz-plane: `fwd` is the heading, `side` is its right-hand
    // perpendicular. The asymmetric pose offsets are expressed in (side, fwd) and rotated
    // into world xz so the whole figure turns with `facing`.
    let (s, c) = facing.sin_cos();
    // fwd = (sin, cos), side = (cos, -sin)  (right-handed about +Y, 0 -> +Z).
    let place = |side: f32, fwd: f32| -> [f32; 2] { [side * c + fwd * s, -side * s + fwd * c] };

    // Lateral half-stance (hips/shoulders), the forward/back leg split, and the arm reach.
    let hip_dx = 0.10 * height;
    let shoulder_dx = 0.17 * height;
    let leg_fwd = 0.14 * height; // right leg forward, left leg back (asymmetric stride)
    let arm_out = 0.26 * height; // right arm raised out to the side; left arm hangs down

    let p = |side: f32, fwd: f32, y: f32| -> [f32; 3] {
        let xz = place(side, fwd);
        [root[0] + xz[0], root[1] + y, root[2] + xz[1]]
    };

    // Hips and shoulders.
    let hip_l = p(-hip_dx, 0.0, hip_y);
    let hip_r = p(hip_dx, 0.0, hip_y);
    let hip_c = p(0.0, 0.0, hip_y);
    let sh_l = p(-shoulder_dx, 0.0, shoulder_y);
    let sh_r = p(shoulder_dx, 0.0, shoulder_y);

    vec![
        // Torso: hip center -> shoulder center (thick).
        SdfEdit::capsule(hip_c, p(0.0, 0.0, shoulder_y), CHAR_TORSO_RADIUS, sdf_op::UNION, 0.0),
        // Head: neck -> crown.
        SdfEdit::capsule(p(0.0, 0.0, neck_y), p(0.0, 0.0, crown_y), CHAR_HEAD_RADIUS, sdf_op::UNION, 0.0),
        // Right leg: hip -> foot, planted FORWARD (the asymmetric stride).
        SdfEdit::capsule(hip_r, p(hip_dx, leg_fwd, foot_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Left leg: hip -> foot, planted BACK.
        SdfEdit::capsule(hip_l, p(-hip_dx, -leg_fwd, foot_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Right arm: shoulder -> hand, raised OUT to the side (reads as a wave).
        SdfEdit::capsule(sh_r, p(shoulder_dx + arm_out, 0.0, shoulder_y + 0.06 * height), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
        // Left arm: shoulder -> hand, hanging DOWN.
        SdfEdit::capsule(sh_l, p(-shoulder_dx, 0.04 * height, hand_y), CHAR_LIMB_RADIUS, sdf_op::UNION, 0.0),
    ]
}

/// The capsule-character backdrop mesh: just the FLOOR quad (y = 0) + the BACK-WALL quad
/// (z = -4), NO cubes — so the SDF scene is the 6 character capsules ALONE (no shadow
/// proxies needed) and the humanoid shadow falls on a clean floor/wall. Reuses the proven
/// `hybrid_room_mesh` floor/wall geometry (the cubes are intentionally dropped).
fn capsule_character_mesh() -> Vec<Vertex> {
    let mut verts = Vec::new();
    // Floor: y = 0, outward normal +Y (CCW from above), spanning x[-3,3] z[-4,1].
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, 1.0], [3.0, 0.0, 1.0], [3.0, 0.0, -4.0], [-3.0, 0.0, -4.0]],
        [0.0, 1.0, 0.0],
        ROOM_FLOOR_COLOR,
    ));
    // Back wall: z = -4, outward normal +Z, spanning x[-3,3] y[0,4].
    verts.extend_from_slice(&mesh_quad(
        [[-3.0, 0.0, -4.0], [3.0, 0.0, -4.0], [3.0, 4.0, -4.0], [-3.0, 4.0, -4.0]],
        [0.0, 0.0, 1.0],
        ROOM_WALL_COLOR,
    ));
    verts
}

/// The capsule-character showcase config (Render Shadow Phase 1): the 6-capsule humanoid
/// proxy standing on the mesh floor under the [`room_camera`] perspective, lit by the
/// showcase sun + a dim sky, casting an analytic SDF shadow onto the floor/wall mesh.
/// Analytic path (`ssao_quality: None`); HARD unions (smoothness 0.0). The 6 capsules are
/// ≤ MAX_SDF_EDITS with room to spare (no cube proxies — the figure stands on a clean floor).
///
/// `contact_shadow` arms Render Shadow Phase 3's Screen-Space Contact Shadows (`with_contact_
/// shadow_mode` — header word 7 bit 1). `false` is the byte-identical 0%-gate (the SSCS march
/// block never runs); `true` tightens the shadow where the feet meet the floor.
fn capsule_character_config(contact_shadow: bool) -> ShowcaseConfig {
    let header = GoldenLightHeader::new(2, 0, 1.0)
        .with_ssao_mode(0)
        .with_contact_shadow_mode(contact_shadow);
    let lights = vec![
        GoldenLight::directional(SHOWCASE_SUN_DIR, [1.0, 0.97, 0.92], 3.0),
        GoldenLight::sky([0.05, 0.05, 0.05], [0.05, 0.05, 0.05]),
    ];
    // The humanoid stands a little behind the room center, facing the camera (+Z) so its
    // front is lit and the shadow rakes back/aside onto the floor and wall.
    let character = character_capsules([0.0, 0.0, -1.0], 1.8, 0.0);
    debug_assert!(
        character.len() <= boyko_sdf_math::MAX_SDF_EDITS,
        "invariant: the character must fit the edit-list budget"
    );
    ShowcaseConfig {
        sdf: character,
        camera: room_camera(),
        light_header: header,
        light_elems: lights,
        vertices: capsule_character_mesh(),
        mvp: perspective_mvp_bytes(
            ROOM_CAM_EYE,
            ROOM_CAM_FORWARD,
            ROOM_CAM_RIGHT,
            ROOM_CAM_UP,
            ROOM_CAM_FOV_Y,
            COMPOSITE_W as f32 / COMPOSITE_H as f32,
        ),
        ssao_quality: None,
    }
}

/// **Render Shadow Phase 1 — the capsule-character screenshot dump (the visual oracle).**
/// Renders the 6-capsule humanoid proxy ([`character_capsules`]) standing on the mesh
/// floor+wall ([`capsule_character_mesh`]) under the [`room_camera`] perspective, lit by the
/// showcase sun + a dim sky, and dumps a TRUE 512×512 24-bit BMP to [`CAPSULE_CHARACTER_BMP`].
/// The deliverable is the cast SDF shadow reading as a humanoid silhouette on the floor.
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshot is the
/// deliverable, not a golden assertion.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator runs it on the GPU to dump the capsule-character screenshot"]
fn engine_capsule_character_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine capsule character 512",
        CAPSULE_CHARACTER_BMP,
        capsule_character_config(false),
    );
}

/// **Render Shadow Phase 3 — Screen-Space Contact Shadows A/B screenshot dump (the visual
/// oracle).** The SAME capsule character feet-on-floor scene as
/// [`engine_capsule_character_512_screenshot_dump`], rendered TWICE: once with
/// `contact_shadow_mode` OFF (dumped to [`CONTACT_SHADOW_OFF_BMP`] — the A/B reference AND the
/// 0%-gate visual proof, since the SSCS march block is structurally skipped) and once with it ON
/// (dumped to [`CONTACT_SHADOW_ON_BMP`] — the contact-shadow tightening where the feet meet the
/// floor). Two windowed renders (each boots + tears down its own device).
///
/// `#[ignore]`: needs a real RTX windowed device. Run with `BOYKO_DISABLE_VALIDATION=1` so the
/// (broken-on-this-box) validation layer does not crash the process; the screenshots are the
/// deliverable, not a golden assertion.
/// `#[ignore]`: needs a real RTX windowed device. SPLIT into two ONE-render-per-process tests —
/// a second windowed render in the same process trips the swapchain-recreate path and never dumps.
#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator dumps the contact-shadow OFF screenshot"]
fn engine_contact_shadow_off_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine contact shadow OFF 512",
        CONTACT_SHADOW_OFF_BMP,
        capsule_character_config(false),
    );
}

#[test]
#[ignore = "needs a real RTX windowed device; the orchestrator dumps the contact-shadow ON screenshot"]
fn engine_contact_shadow_on_512_screenshot_dump() {
    run_showcase_dump(
        "boyko_engine contact shadow ON 512",
        CONTACT_SHADOW_ON_BMP,
        capsule_character_config(true),
    );
}

/// The shared 512×512-native multi-light SDF-shadow + SSAO showcase dump body. `window_title` is
/// the window caption; `bmp_path` is the TRUE 512×512 24-bit BMP destination (no upscale); `cfg`
/// supplies the variable scene (SDF edits, camera, light table, raster mesh + MVP). SSAO is ON (the
/// `cfg` builder arms `ssao_mode == 1`; `scene.ssao = Some(..)` records the pass that writes it).
fn run_showcase_dump(window_title: &str, bmp_path: &str, cfg: ShowcaseConfig) {
    let mut window = match Window::open(window_title, WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP engine_showcase_512: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP engine_showcase_512: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — showcase dump still runs");
    }
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    // SAFETY: `window` outlives the surface (dropped after it below); its HWND/HINSTANCE are
    // live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP engine_showcase_512: surface creation failed ({e:?})");
            return;
        }
    };
    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP engine_showcase_512: swapchain creation failed ({e:?})");
            return;
        }
    };

    if swapchain.extent().width < COMPOSITE_W || swapchain.extent().height < COMPOSITE_H {
        eprintln!(
            "SKIP engine_showcase_512: swapchain extent {}x{} is smaller than the {}x{} composite",
            swapchain.extent().width,
            swapchain.extent().height,
            COMPOSITE_W,
            COMPOSITE_H
        );
        return;
    }

    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!("SKIP engine_showcase_512: swapchain format has no host-decodable UNORM byte order");
        return;
    };
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP engine_showcase_512: swapchain format has no basic-slice Format variant");
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");
    let device: &VulkanContext = &ctx;
    let sdf = &cfg.sdf;

    // --- The edit-list SSBO (binding 0), host-seeded ONCE. The resolve binds the SAME buffer
    // at binding 10 for the per-caster shadow march. ---
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UBO (binding 5), host-seeded ONCE at the COMPOSITE PERSPECTIVE extent
    // ([`showcase_camera`] — a down-looking front camera so the SDF floor + bodies + their cast
    // shadows read as a 3D scene). The M4 tail stays zero (brick is held OFF for the showcase —
    // the analytic marcher is the crisp reference path; bindings 9..=14 still need VALID
    // descriptors below). ---
    let camera_uniform = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: B5_CAMERA_UBO_BYTES_M4 as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("camera uniform buffer");
    {
        let pc = &cfg.camera;
        assert_eq!(pc.count, PIXELS);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        debug_assert_eq!(bytes.len(), M2_GRID_PARAMS_OFFSET, "camera block must be 80 B");
        // SAFETY: `mapped` points to `B5_CAMERA_UBO_BYTES_M4` (224) mapped host-coherent bytes;
        // the 80-byte camera block is written at offset 0 (the M4 tail stays zero — brick OFF).
        // No GPU work is in flight yet, so the host write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // --- The P4b coarse-cull tile StorageBuffer (vocab binding 6), bound-but-unread (the
    // showcase runs the marcher with the coarse cull gated OFF). ---
    let (tw, th) = tile_grid_extent(COMPOSITE_W, COMPOSITE_H);
    let tiles_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (tw as u64) * (th as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("P4b coarse-cull tile-bound storage buffer (vocab binding 6)");

    // --- The PBR material table SSBO (vocab binding 7 + resolve binding 4): the default
    // mid-gray dielectric (the showcase edits carry no material id ⇒ every SDF hit picks 0). ---
    let material_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("PBR material table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- The brick clip-map: brick is held OFF for the showcase, but the marcher SPIR-V
    // statically references bindings 9..=14 past the runtime gate, so VALID descriptors must be
    // bound. The real clip-map (baked from the SAME authority field) supplies them. ---
    let field = {
        use boyko_sdf_math::SdfEditField;
        let mut f = SdfEditField::new();
        for e in sdf {
            assert!(f.push(*e), "showcase scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    };
    let clipmap = BrickClipmap::create(&ctx, &field, [0.0, 0.0, 0.0])
        .expect("brick clip-map (showcase scene) — create + bake + upload");

    // --- The Lighting light table SSBO (resolve binding 6): the SHOWCASE multi-light shadow
    // table (`shadow_mode == 1`, NON-CLUSTERED) + its staging source. Render P7: the `cfg` builder
    // already ARMED `ssao_mode == 1` (header word 11) so the resolve combines the SSAO term
    // (`scene.ssao = Some(..)` records the SSAO pass that writes it). ---
    let light_header = cfg.light_header;
    let light_elems = &cfg.light_elems;
    let light_words = pack_showcase_light_table(&light_header, light_elems);
    let light_table_bytes = (light_words.len() as u64) * 4;
    let light_table = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("showcase light table storage buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_table)
            .expect("host-visible light table is mapped");
        write_words(mapped, &light_words);
    }
    let light_staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: light_table_bytes,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("showcase light table staging buffer");
    {
        let mapped = RhiDevice::buffer_mapped_ptr(device, &light_staging)
            .expect("host-visible light staging is mapped");
        write_words(mapped, &light_words);
    }

    // --- The mesh's vertex buffer (the showcase floor / hybrid-room geometry). ---
    let vertices = cfg.vertices;
    // `vertices` is a `Vec`, so the byte length is the slice's footprint (NOT `size_of_val`
    // of the `Vec` handle, which is the 24-byte struct, not the heap buffer).
    let vertex_bytes = core::mem::size_of_val(vertices.as_slice()) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`'s heap
        // buffer is a distinct `vertex_bytes`-byte region (`vertex_bytes == len * stride`); the
        // write completes before any submit.
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
    let present_sampler = RhiDevice::create_sampler(
        device,
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
        },
    )
    .expect("present nearest/clamp sampler");

    // --- The mesh-MRT G-buffer producer graphics pipeline (Render P5-r0). ---
    let vs = RhiDevice::create_shader_module(device, MRT_VS_SPV.as_words())
        .expect("mesh-MRT vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MRT_FS_SPV.as_words())
        .expect("mesh-MRT fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
    ];
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT, RASTER_COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
            blend: None,
        },
    )
    .expect("mesh-MRT graphics pipeline");

    // --- The P1b marcher: the vocabulary layout + the marcher pipeline. ---
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    let vocab_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 12, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 13, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 14, count: 1, kind: DescriptorKind::CombinedImageSampler, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
    let marcher = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
        },
    )
    .expect("P1b G-buffer marcher compute pipeline");

    // --- The deferred RESOLVE pipeline (binds the light table @6 + the SDF edit-list @10 for
    // the per-caster shadow march). ---
    let resolve_cs = RhiDevice::create_shader_module(device, deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");
    let resolve_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 8, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 9, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // P6 R1: the SDF edit-list `Buf` @10 (the `sdf_soft_shadow_ranged` march reads it).
        BindGroupLayoutEntry { binding: 10, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // Render P7: the SSAO term `gSsao` STORAGE image @11 (read under `ssao_mode != 0`; OFF
        // here, bound-but-unread). The production `GBufferTargets` binds the SSAO image at @11,
        // so the resolve layout MUST declare it (the P6 R1 binding-10 discipline).
        BindGroupLayoutEntry { binding: 11, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &resolve_entries },
    )
    .expect("deferred resolve bind-group layout");
    let resolve_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
        },
    )
    .expect("deferred resolve compute pipeline");

    // --- The present-blit pipeline. ---
    let present_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::CombinedImageSampler,
                stage: ShaderStage::FRAGMENT,
            }],
        },
    )
    .expect("present-blit bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&present_layout),
            blend: None,
        },
    )
    .expect("present-blit fullscreen-sample pipeline");

    // --- Render P7: the SSAO compute pass (dedicated 5-binding set { gNormal @0, gMaterial @1,
    // gViewT @2 (R), the `ssao` out @3 (W), camera UBO @4 }). It gathers a horizon-based AO factor
    // from the G-buffer and stores it into the `ssao` lane the resolve combines under `ssao_mode
    // != 0` (armed via `light_header.with_ssao_mode(1)` above). `GBufferTargets` writes the
    // `ssao_set` against THIS layout, pointing at the per-extent G-buffer + `ssao` images. ---
    // Render P7-Q2: bind the SELECTED quality variant's pre-compiled `.spv` (Mechanism C). When SSAO
    // is OFF (`cfg.ssao_quality == None`) the pipeline is still created (and destroyed) — harmless —
    // but `scene.ssao` below is set to `None`, so the recorder records NO SSAO pass (the 0%-gate).
    let ssao_variant = cfg.ssao_quality.unwrap_or(SSAO_QUALITY_MEDIUM);
    let ssao_cs = RhiDevice::create_shader_module(device, sdf_ssao_spirv_variant(ssao_variant))
        .expect("Render P7 SSAO compute shader module");
    let ssao_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let ssao_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &ssao_entries },
    )
    .expect("Render P7 SSAO bind-group layout");
    let ssao_pipeline = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &ssao_cs,
            entry: c"main",
            // The SSAO shader pushes NO constant (camera is the UBO @4), but the create contract
            // requires a non-empty (multiple-of-4) range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&ssao_layout),
        },
    )
    .expect("Render P7 SSAO compute pipeline");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its pipeline
    // is created; each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, ssao_cs);
        RhiDevice::destroy_shader_module(device, resolve_cs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    let mvp = cfg.mvp;
    let scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_uniform: &camera_uniform,
        tiles_buffer: &tiles_buffer,
        pointer_grid: clipmap.grid_buffer(0),
        atlas: clipmap.atlas(0).texture(),
        atlas_sampler: clipmap.sampler(0),
        level_grids: [clipmap.grid_buffer(1), clipmap.grid_buffer(2)],
        level_atlases: [clipmap.atlas(1).texture(), clipmap.atlas(2).texture()],
        level_atlas_samplers: [clipmap.sampler(1), clipmap.sampler(2)],
        depth_sampler: &depth_sampler,
        material_table: &material_table,
        light_table: &light_table,
        light_staging: &light_staging,
        light_upload_bytes: light_table_bytes,
        light_dirty: false,
        // L1 cluster cull OFF (NON-CLUSTERED): the frozen `cluster_cull.hlsl` drops a
        // shadow-flagged punctual, so the multi-light SDF-shadow path runs on the flat-table
        // (non-clustered) resolve — exactly `p6_r1_multi_light_sdf_shadows_match_oracle`'s path.
        cluster_cull: None,
        cull_layout: None,
        cluster_grid: None,
        light_index: None,
        light_index_alloc: None,
        cluster_cull_push: [0u8; 16],
        cluster_count: 0,
        resolve_pipeline: &resolve_pipeline,
        resolve_layout: &resolve_layout,
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
        // The analytic marcher (brick OFF) is the crisp reference path for the showcase.
        brick: None,
        coarse: None,
        coarse_mode: CoarseMode::EmptySkipOnly,
        // The real on-screen lit flags: A1 soft shadows + A2 AO. The marcher marches the A1 soft
        // shadow toward `light_dir` (the sun) into `gMaterial.r`, which the resolve's PRIMARY
        // directional consumes — so the sphere/box cast a real shadow ACROSS the SDF floor.
        lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        // The sun direction (`L`, direction TO the light) — MUST equal the primary directional in
        // `showcase_light_table` so the marched cast shadow lands where the resolve lights from.
        light_dir: SHOWCASE_SUN_DIR,
        // Render P7 / P7-Q2: SSAO is ON only when `cfg.ssao_quality` selected a variant — the
        // recorder then records the SSAO pass (BETWEEN the marcher→resolve barrier and the resolve)
        // that writes the `ssao` lane the resolve combines (`ssao_mode == 1`, armed on `light_header`
        // above), and the contact creases / floor-body junctions darken. `None` = SSAO OFF: NO SSAO
        // pass + `ssao_mode == 0` (the byte-identical 0%-gate `_off` reference for the quality ladder).
        ssao: cfg
            .ssao_quality
            .map(|_| SsaoActivation { pipeline: &ssao_pipeline, layout: &ssao_layout }),
    };

    let present_extent = VkExtent2D { width: COMPOSITE_W, height: COMPOSITE_H };
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();
    let mut frame = GBufferFrame::new();

    // Render ONE readback frame, then drain so the staging buffer is host-coherent (the same
    // FRAMES_IN_FLIGHT==2 / 3-drain discipline the existing windowed dumps use). The readback is
    // a 4-B/texel BGRA-or-RGBA copy of the FULL swapchain image; `readback_to_rgba` normalizes
    // the swapchain R/B order so the dumped BMP is color-correct.
    const DRAIN_FRAMES: u32 = 3;
    let clear = [0.04_f32, 0.05, 0.07, 1.0];

    let mut dumped: Option<(Vec<u8>, u32, u32)> = None;
    if !window.pump_events() {
        eprintln!("NOTE engine_showcase_512: window closed before the dump frame — skipping");
    } else {
        window.refresh_size();
        let live = swapchain.extent();
        if live.width != alloc_extent.width || live.height != alloc_extent.height {
            eprintln!("NOTE engine_showcase_512: extent changed before the dump frame — skipping");
        } else {
            // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share one device; every `scene`
            // resource is live; `present_extent` + `scene.dispatch_group_count_x` + the camera UBO
            // `count` cover the composite extent; `staging` is host-visible and ≥ one swapchain
            // image in bytes.
            let presented = unsafe {
                renderer.render_gbuffer_frame(
                    &ctx, &surface, &mut swapchain, &scene, &mut frame,
                    window.width(), window.height(), clear, present_extent, Some(&staging),
                )
            }
            .unwrap_or_else(|e| panic!("showcase readback frame failed: {e:?}"));

            if !presented {
                eprintln!("NOTE engine_showcase_512: swapchain recreated on the readback frame — skipping");
            } else {
                let extent = swapchain.extent();
                for _ in 0..DRAIN_FRAMES {
                    if !window.pump_events() {
                        break;
                    }
                    window.refresh_size();
                    // SAFETY: same contract; no readback requested on the drain frames.
                    let _ = unsafe {
                        renderer.render_gbuffer_frame(
                            &ctx, &surface, &mut swapchain, &scene, &mut frame,
                            window.width(), window.height(), clear, present_extent, None,
                        )
                    }
                    .unwrap_or_else(|e| panic!("showcase drain frame failed: {e:?}"));
                }

                let w = extent.width;
                let h = extent.height;
                let byte_count = (w * h * 4) as usize;
                let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
                    .expect("host-visible staging buffer is mapped");
                let mut raw = vec![0u8; byte_count];
                // SAFETY: `dst_ptr` points to `staging_size` (≥ `byte_count`) mapped host-coherent
                // bytes; the readback frame's copy completed before this read (its slot fence was
                // re-waited by the drain frames); `raw` is a distinct, non-overlapping alloc.
                unsafe { core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), raw.as_mut_ptr(), byte_count) };
                dumped = Some((readback_to_rgba(&raw, w, h, is_bgra), w, h));
            }
        }
    }

    if ctx.validation_enabled() {
        let state = ctx
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation layer reported {} message(s) during the showcase present — \
             see the [vk-validation] log",
            state.total()
        );
    }

    // Write the TRUE 512×512 BMP (no upscale — the composite is already native) + verify the
    // dumped dimensions are exactly 512×512.
    match dumped {
        Some((rgba, w, h)) => {
            assert_eq!(
                (w, h),
                (COMPOSITE_W, COMPOSITE_H),
                "the readback must be the native {COMPOSITE_W}x{COMPOSITE_H} composite (no upscale)"
            );
            write_bmp(bmp_path, &rgba, w, h)
                .unwrap_or_else(|e| panic!("failed to write {bmp_path}: {e:?}"));
            let bytes = std::fs::read(bmp_path)
                .unwrap_or_else(|e| panic!("failed to re-read {bmp_path} for header verification: {e:?}"));
            let (bw, bh) = read_bmp_dimensions(&bytes)
                .expect("the dumped showcase must be a valid BM 54-byte-header BMP");
            assert_eq!(
                (bw, bh),
                (COMPOSITE_W as i32, COMPOSITE_H as i32),
                "the dumped BMP header must report {COMPOSITE_W}x{COMPOSITE_H} native dimensions"
            );
            println!("engine showcase dump -> {bmp_path} ({bw}x{bh} native, multi-light SDF shadows + SSAO)");
        }
        None => {
            eprintln!(
                "NOTE engine_showcase_512: no readback frame presented (swapchain kept recreating); \
                 no BMP written"
            );
        }
    }

    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no submission
    // references these resources; `ctx` is still alive; each is destroyed exactly once, in reverse
    // dependency order.
    unsafe {
        frame.destroy(&ctx);
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, ssao_pipeline);
        RhiDevice::destroy_bind_group_layout(device, ssao_layout);
        RhiDevice::destroy_compute_pipeline(device, resolve_pipeline);
        RhiDevice::destroy_bind_group_layout(device, resolve_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, tiles_buffer);
        clipmap.destroy(&ctx);
        RhiDevice::destroy_buffer(device, light_staging);
        RhiDevice::destroy_buffer(device, light_table);
        RhiDevice::destroy_buffer(device, material_table);
        RhiDevice::destroy_buffer(device, camera_uniform);
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
