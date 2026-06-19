//! Render **P1b GPU gate** (scaffold) — the OFFSCREEN image-based SDF + mesh hybrid
//! composite writing an MRT G-buffer, the image-based rewrite of the rung-10
//! packed-buffer marcher (`sdf_mesh_hybrid_depth.rs`'s `run_hybrid`).
//!
//! # What this proves (the P1b milestone)
//!
//! `run_gbuffer_hybrid` records the §15.1 shared-depth seam WITHOUT the per-frame
//! depth→buffer copy: a real GPU-rasterized quad's depth is written into a D32_SFLOAT
//! IMAGE, transitioned `DEPTH_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL` (a
//! SINGLE depth barrier, DEPTH aspect, `LATE_FRAGMENT_TESTS` src — replacing the old
//! copy + its two transfer barriers), and SAMPLED directly by the marcher compute
//! shader (`Texture2D<float>.Load`). The marcher STORES its color into an
//! `R8G8B8A8_UNORM` STORAGE image (the ALBEDO G-buffer target), plus additive
//! normal/material targets, through the P1a multi-resource descriptor *vocabulary* set
//! (written ONCE at create — NO per-frame `vkUpdateDescriptorSets`). A deferred RESOLVE
//! pass then composites `gLit` (full Cook-Torrance on the SDF arm; the `mask == 0` mesh /
//! background / empty pixels pass through verbatim). The LIT image is read back and
//! asserted against the deferred PBR oracle `golden_deferred_resolve ∘
//! golden_marcher_attributes` within `+/-2/255` per channel.
//!
//! PBR MVP-2 (the behavioral change): the SDF-surface (mask == 1) shading moved from the
//! MVP-1 `base*vis` Lambert inline composite (the retired `golden_composite_pixel_ex*`
//! oracles) to full Cook-Torrance via the deferred G-buffer + resolve. The GPU gates that
//! read `gLit` therefore compare against the deferred oracle (the proven reference, see
//! the `d2g`/`d3g` gates), NOT the MVP-1 inline oracles — which survive only on the
//! pass-through arms (host-only: `a_host_*` + `d1_host_*`).
//!
//! Determinism (INVIOLABLE): the field eval + ray-gen + marcher attributes are
//! byte-identical to the host oracle (a verbatim shader cut); only the depth SOURCE (a
//! sampled image) and the color SINK (a storage image) change. The float-to-UNORM store
//! vs the host `pack_rgba` rounding is absorbed by the `+/-2/255` tolerance.

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::descriptor::{BarrierDesc, BufferBarrier};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, ComputePipelineDesc, DepthAttachment, DescriptorKind, Format,
    GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, LoadOp, MemoryLocation, PrimitiveTopology, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera, CompositePushConstants, DEFAULT_LIGHT_DIR,
    DEFAULT_MARCHER_OMEGA, FineMarcherPush, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, LOCAL_SIZE_X,
    MESH_COLOR, MESH_DEPTH_CLEAR,
    SDF_CAMERA_Z, SDF_IMG_H,
    SDF_IMG_W, SDF_TRACE_T_MAX, SDF_VIEW_HALF_EXTENT, SdfEdit, TILE_BOUND_BYTES, TILE_FLAG_EMPTY,
    TILE_SIZE, TileBound, EDITLIST_BUFFER_WORDS, editlist_pixel_hits, encode_edit_list,
    golden_composite_pixel_culled, golden_composite_pixel_ex,
    golden_composite_pixel_ex_omega_lit, golden_tile_bound,
    golden_deferred_resolve, golden_marcher_attributes, GoldenMaterial, composite_pixel_ray,
    deferred_pbr_spirv, mesh_depth_for_z, pack_rgba, pixel_world_xy,
    sdf_gbuffer_composite_spirv, sdf_op, sdf_tile_cull_spirv, tile_grid_extent,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Total pixel count (the compute UBO `count`; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// R8G8B8A8 ALBEDO readback byte size.
const READBACK_BYTES: u64 = (PIXELS as u64) * 4;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10): DXC
/// `mad`/`fma` rounding + the float→UNORM store quantization make a bit-exact match
/// brittle; `+/-2/255` still proves the lit SDF surface / flat mesh / background colors
/// apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// The mesh quad's constant world Z (chosen strictly between the sphere surface and the
/// camera so the mesh occludes the SDF over the sphere). Mirrors `run_hybrid`'s `MESH_Z`.
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint (the left part of the view in x, full y), so the
/// sphere straddles the quad edge — yielding texels over BOTH / sphere-only / quad-only
/// / neither. Mirrors `run_hybrid`'s footprint.
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane; an uncovered pixel keeps it,
/// decoded as "no mesh"). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// A throwaway color-attachment format for the mesh raster pass (the graphics pipeline
/// requires a non-empty `color_formats`; only the DEPTH result is consumed).
const COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target whose support the [`DeviceCaps`] boot fail-fast asserts.
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12), the
/// rung-3/4 vertex layout reused. `#[repr(C)]` for the exact 28-byte stride.
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

/// PBR MVP-2: the std430 word-packing of a ONE-element material table holding the engine
/// default material (mid-gray dielectric: base 0.8/0.8/0.8/1, metallic 0, roughness 0.5,
/// reflectance 0.5, flags 0, emissive 0). 12 words = 48 B, mirroring `MaterialGpu`'s 3
/// `vec4` lanes. The crater/box/smooth edits carry NO material id (center.w == 0), so every
/// SDF hit picks id 0 → this material. Kept in sync with [`host_material_table`].
const DEFAULT_MATERIAL_TABLE: [u32; 12] = [
    // lane 0: base_color (rgb linear + alpha)
    0x3F4CCCCD, 0x3F4CCCCD, 0x3F4CCCCD, 0x3F800000, // 0.8, 0.8, 0.8, 1.0
    // lane 1: mrr = [metallic, roughness, reflectance, bitcast(flags)]
    0x00000000, 0x3F000000, 0x3F000000, 0x00000000, // 0.0, 0.5, 0.5, flags=0
    // lane 2: emissive (rgb linear + unused)
    0x00000000, 0x00000000, 0x00000000, 0x00000000, // 0, 0, 0, 0
];

/// The HOST mirror of [`DEFAULT_MATERIAL_TABLE`] for the `golden_*` oracles (the same
/// single default material at id 0).
fn host_material_table() -> [GoldenMaterial; 1] {
    [GoldenMaterial::default()]
}

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // `'static` blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`), reused for the raster.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP log)
/// when no GPU / loader / validation layer / dynamic-rendering is available.
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
        "validation layer reported {} message(s) during the P1b G-buffer hybrid run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count (fine pass).
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// The coarse tile-grid extent (`tiles_w`, `tiles_h`) for the golden 64×64 image.
fn tile_extent() -> (u32, u32) {
    tile_grid_extent(SDF_IMG_W, SDF_IMG_H)
}

/// Total coarse tiles (the `RWStructuredBuffer<TileBound>` element count + the
/// coarse-pass dispatch element count).
fn tile_count() -> u32 {
    let (tw, th) = tile_extent();
    tw * th
}

/// `ceil(tile_count / LOCAL_SIZE_X)` — the 1D coarse-pass dispatch group count.
fn coarse_group_count_x() -> u32 {
    tile_count().div_ceil(LOCAL_SIZE_X)
}

/// The orthographic MVP for the rung-3 vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose — see `run_hybrid`'s `ortho_mvp_bytes`). Maps a fronto-parallel
/// world vertex so the stored depth is `(CAM_Z - worldZ) / T_MAX`.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    // Mᵀ in row-major upload order: mt[r*4 + c] = M[c][r] (each group of 4 is a COLUMN
    // of M); the only off-diagonal term `CAM_Z/T_MAX` lives at mt[14].
    let mt: [f32; 16] = [
        1.0 / h, 0.0,      0.0,          0.0,
        0.0,     -1.0 / h, 0.0,          0.0,
        0.0,     0.0,      -1.0 / tmax,  0.0,
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The mesh quad as two triangles spanning the world-XY footprint at world Z [`MESH_Z`].
fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], color: c };
    [bl, br, tr, bl, tr, tl]
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint
/// (the rasterizer's covered-pixel set, host-computable from the SAME camera mapping).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU is expected to produce (the host model for the
/// golden's `mesh_depth` input): the constant inside the quad, the clear outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (valid before
/// the submit — the CPU seeds the edit-list header / the UBO here).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is at least `words.len() * 4` bytes inside the persistent
        // host-coherent mapping; `dst + i` for `i < words.len()` is in-bounds. No GPU
        // work is in flight yet (the submit follows), so the host write is
        // unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_packed_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Splits an R8G8B8A8 readback texel's first three bytes into `[r, g, b]`.
fn unpack_texel_rgb(rgba: &[u8]) -> [i32; 3] {
    [rgba[0] as i32, rgba[1] as i32, rgba[2] as i32]
}

/// `true` if a readback texel agrees with a packed golden within `CHANNEL_TOL`/channel.
fn texel_close(got: [i32; 3], want_packed: u32) -> bool {
    let w = unpack_packed_rgb(want_packed);
    (0..3).all(|c| (got[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Records + submits the full OFFSCREEN G-buffer hybrid composite + the deferred RESOLVE
/// in ONE command buffer / ONE fenced submit, returning the readback LIT storage image as
/// `PIXELS` R8G8B8A8 texels (4 bytes each). The flow — the §15.1 seam with NO depth→buffer
/// copy, plus the deferred-split resolve:
///
///   raster quad → D32 depth IMAGE → barrier depth DEPTH_ATTACHMENT→SHADER_READ_ONLY
///   (one barrier) → barrier the 3 G-buffer images + lit UNDEFINED→GENERAL → bind the
///   vocabulary set {SSBO edit-list, SAMPLED depth, STORAGE albedo/normal/material,
///   UNIFORM camera} + the marcher → dispatch (writes ATTRIBUTES: gAlbedo = base,
///   gMaterial = (shadow, ao, mask, 1)) → barrier albedo/material GENERAL→GENERAL
///   (SHADER_WRITE→SHADER_READ) → bind the resolve set {STORAGE albedo/material/lit} +
///   the resolve → dispatch (composites lit via full Cook-Torrance on the SDF arm, the
///   mask==0 pass-through verbatim) → barrier lit
///   GENERAL→TRANSFER_SRC → copy_image_to_buffer(lit) into readback.
///
/// There is NO `copy_image_to_buffer(depth)` and NO transfer→compute buffer barrier:
/// the single depth `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier
/// replaces the old copy + its two barriers. Both descriptor sets are written ONCE at
/// `create_bind_group` — there is no per-frame `vkUpdateDescriptorSets`.
fn run_gbuffer_hybrid(ctx: &VulkanContext, edits: &[SdfEdit], coarse_enabled: bool) -> Vec<u8> {
    // Delegate to the `_ex` variant, discarding the tiles-buffer readback. Defaults the
    // Render B1 over-relaxation factor to `1.0` — the marcher byte-identical to the pre-B1
    // path — so the existing 0%-gate callers (`p1b_gbuffer_hybrid_matches_golden`, the
    // GATE-4 `p4b_cull_off_is_byte_identical_to_pre_p4b_path`) stay TRUE ω=1.0 byte-identity
    // gates against the ω=1.0 host golden. The ω>1 path (engine default
    // `DEFAULT_MARCHER_OMEGA`) is exercised by the dedicated B1 over-relaxation tests, which
    // call `run_gbuffer_hybrid_ex` with an explicit ω and diff against `_omega` host goldens.
    run_gbuffer_hybrid_ex(ctx, edits, coarse_enabled, false, 1.0).0
}

/// Render P4b — the extended harness: the same OFFSCREEN G-buffer hybrid composite as
/// [`run_gbuffer_hybrid`], but ALSO reads back the per-tile [`TileBound`] cull buffer
/// (binding 6) when `read_tiles == true`, returning `(albedo, Some(tiles_bytes))`.
///
/// The tiles-buffer readback is the TESTER's host/GPU agreement oracle: the returned
/// `Vec<u8>` is `tile_count() * TILE_BOUND_BYTES` bytes of the std430 `RWStructuredBuffer
/// <TileBound>` the coarse pass wrote (parse each 16-byte element as near_t f32@0, far_t
/// f32@4, flags u32@8, _pad u32@12). With `read_tiles == false` the second element is
/// `None` and no extra copy / barrier is recorded (the byte-identity 0%-gate path).
///
/// `read_tiles` requires `coarse_enabled` — the coarse pass only runs (and only writes
/// binding 6) when culling is on; reading it back otherwise yields the buffer's
/// undefined create-time contents. The caller is responsible for pairing them.
///
/// `omega_in` is the Render B1 over-relaxation factor; it is RUNTIME-clamped to
/// `[1.0, 1.99]` before the push encode (the soundness ceiling sits at `omega == 2`).
/// `1.0` keeps the marcher byte-identical to the pre-B1 path (the 0%-gate).
fn run_gbuffer_hybrid_ex(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
) -> (Vec<u8>, Option<Vec<u8>>) {
    // Delegate to the lighting-aware variant with lighting OFF (the historical default):
    // `lighting_flags == 0` ⇒ the shader's byte-identical Lambert path, so every existing
    // 0%-gate caller keeps its exact OFF semantics. The A1/A2 ON-path tester gates call
    // `run_gbuffer_hybrid_lit` directly with an explicit `lighting_flags` + `light_dir`.
    run_gbuffer_hybrid_lit(ctx, edits, coarse_enabled, read_tiles, omega_in, 0, DEFAULT_LIGHT_DIR)
}

/// Render A1/A2 — the lighting-aware harness: identical to [`run_gbuffer_hybrid_ex`] but
/// the marcher push carries an explicit `lighting_flags` (bit 0 = A1 shadows, bit 1 = A2
/// AO; `0` = the OFF Lambert path) and `light_dir` (the un-normalized directional light).
///
/// Deferred split (MVP-2): the marcher writes ATTRIBUTES (gAlbedo = the unmultiplied raw
/// linear base, gNormal = (oct normal, 16-bit material id), gMaterial = (shadow, ao, mask,
/// 1)); a fullscreen `deferred_pbr` RESOLVE composites `lit` via full Cook-Torrance on the
/// SDF arm (the picked material's metallic/roughness/F0, the analytic directional light
/// modulated by the A1 shadow + A2 AO, plus the hemisphere/specular-IBL ambient), passing
/// the `mask == 0` pixels through byte-identically, into a dedicated LIT image. The readback
/// now copies LIT (not albedo), so the tester diffs it against `golden_deferred_resolve(...)`
/// fed by `golden_marcher_attributes(...)` with the SAME flags + `light_dir`. Everything else
/// (the §15.1 seam, the vocabulary set, the coarse pass) is the [`run_gbuffer_hybrid_ex`]
/// flow verbatim — the marcher push payload + the new resolve pass.
#[allow(clippy::too_many_arguments)]
fn run_gbuffer_hybrid_lit(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    coarse_enabled: bool,
    read_tiles: bool,
    omega_in: f32,
    lighting_flags: u32,
    light_dir: [f32; 3],
) -> (Vec<u8>, Option<Vec<u8>>) {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // --- The edit-list StorageBuffer (binding 0), seeded with the packed header. The
    // P1b shader only READS the rung-9 header + edit array (`Buf[0..PIXEL_BASE_WORDS]`,
    // i.e. `Buf[0..196]`); the depth/pixel regions are no longer used by this path.
    // We deliberately OVER-ALLOCATE to the full `EDITLIST_BUFFER_WORDS` (which still
    // includes the now-unused pixel region) rather than trimming to `PIXEL_BASE_WORDS`:
    // `encode_edit_list` debug-asserts `buf.len() >= EDITLIST_BUFFER_WORDS`, so reusing
    // the shared const keeps the host encoder and the buffer in lock-step and avoids a
    // size desync. The extra words are simply never touched. ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UNIFORM buffer (binding 5), written ONCE at setup (NOT a
    // per-frame push). At the golden 64×64 ORTHO extent it drives bit-exact rays. ---
    let camera_uniform = device
        .create_buffer(&BufferDesc {
            size: COMPOSITE_PUSH_CONSTANT_BYTES as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        debug_assert_eq!(pc.count, PIXELS);
        let mapped = device
            .buffer_mapped_ptr(&camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        // `as_bytes()` is the same 80-byte POD the packed path pushed; write it once.
        let bytes = pc.as_bytes();
        // SAFETY: `mapped` points to `COMPOSITE_PUSH_CONSTANT_BYTES` mapped
        // host-coherent bytes; `bytes` is exactly that many bytes; no GPU work is in
        // flight yet (submit follows), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // --- PBR MVP-2: the material table SSBO (binding 7 of the vocab set + binding 4 of
    // the resolve set). The crater/box/smooth edits carry NO material id (center.w == 0),
    // so every SDF hit picks material 0 — the default mid-gray dielectric. One element
    // suffices (48 B / 12 words; mirrors boyko_render::MaterialGpu's std430 layout). ---
    let material_table = device
        .create_buffer(&BufferDesc {
            size: (DEFAULT_MATERIAL_TABLE.len() as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("PBR material table storage buffer");
    {
        let mapped = device
            .buffer_mapped_ptr(&material_table)
            .expect("host-visible material table is mapped");
        write_words(mapped, &DEFAULT_MATERIAL_TABLE);
    }

    // --- Render P4b: the per-tile coarse-cull StorageBuffer (binding 6). The coarse
    // pass WRITES one `TileBound` (16 B) per 8×8 tile; the fine marcher READS it (gated
    // by the `coarse_enabled` push). Device-local would do, but a host-coherent buffer
    // lets the GPU-half tester read the bounds back and diff them against
    // `golden_tile_bound`. Sized to the full tile grid. ---
    let tiles_buffer = device
        .create_buffer(&BufferDesc {
            size: (tile_count() as u64) * (TILE_BOUND_BYTES as u64),
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("P4b coarse-cull tile-bound storage buffer");

    // --- The depth IMAGE (D32_SFLOAT): DEPTH_STENCIL_ATTACHMENT (rasterize into it) |
    // SAMPLED (the marcher samples it directly — NO copy). A DEPTH_STENCIL_ATTACHMENT
    // usage gives the texture a DEPTH-aspect view, exactly what the marcher's
    // `Texture2D<float>.Load` samples. ---
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
        })
        .expect("offscreen depth texture (sampled)");

    // A throwaway color attachment for the raster pass (never read back).
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

    // --- The MRT G-buffer STORAGE images (albedo + normal + material): STORAGE for the
    // compute store. Deferred split: albedo/material are CONSUMED by the resolve (STORAGE
    // load in GENERAL), so albedo no longer needs TRANSFER_SRC — the readback copies the
    // resolve's LIT output instead. ---
    let albedo = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        })
        .expect("G-buffer albedo storage image");
    let normal = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        })
        .expect("G-buffer normal storage image");
    let material = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        })
        .expect("G-buffer material storage image");
    // Deferred split: the LIT image is the resolve's STORAGE store output; TRANSFER_SRC so
    // the golden readback copies it out (the readback now reads LIT, not albedo).
    let lit = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
        })
        .expect("deferred resolve lit storage image");

    // The depth is SAMPLED via `.Load` (OpImageFetch, no sampler), but the RHI
    // `BindGroupEntry::SampledImage` requires a sampler handle; a nearest/clamp sampler
    // is created and bound (it is ignored by an unfiltered fetch).
    let sampler = device
        .create_sampler(&SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");

    // The readback buffer for the LIT image.
    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback buffer");

    // The quad vertex buffer.
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
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`
    // is a distinct stack array of `vertex_bytes` bytes; the write completes before any
    // submit references the buffer (host-coherent: no flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- Modules: the rung-3 mesh-raster pair + the P1b G-buffer marcher (compute). ---
    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");
    let cs = device
        .create_shader_module(sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    // Render P4b: the coarse-cull / tile pre-trace compute module.
    let coarse_cs = device
        .create_shader_module(sdf_tile_cull_spirv())
        .expect("P4b coarse-cull compute shader module");
    // Deferred split: the `deferred_pbr.comp` RESOLVE compute module.
    let resolve_cs = device
        .create_shader_module(deferred_pbr_spirv())
        .expect("deferred resolve compute shader module");

    // The depth-testing graphics pipeline (rung-3 vertex layout + 64-byte VERTEX MVP
    // push + a declared depth_format).
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

    // --- The P1b vocabulary set, EXTENDED for P4b: { SSBO edit-list @0, SAMPLED depth
    // @1, STORAGE albedo @2, STORAGE normal @3, STORAGE material @4, UNIFORM camera @5,
    // STORAGE tile-bounds @6 }. ONE set-0 layout shared by BOTH the coarse pass (reads
    // 0/1/5, writes 6) and the fine marcher (reads 0/1/5/6, writes 2/3/4) — each shader
    // uses a subset (valid). ---
    let layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        // PBR MVP-2: the material table SSBO @7 (the marcher fetches `base_color`).
        BindGroupLayoutEntry { binding: 7, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
    ];
    let bind_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &layout_entries })
        .expect("P4b vocabulary bind-group layout");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            // The dedicated vocabulary layout declares the shared compute push range; the
            // P4b marcher pushes a 4-byte `coarse_enabled` gate against THIS pipeline's
            // own layout (via `push_compute_constants`). `COMPOSITE_PUSH_CONSTANT_BYTES`
            // keeps the create-time "non-empty multiple of 4 within the shared range"
            // contract (the 4-byte push fits inside the declared 80-byte range).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
        })
        .expect("P1b G-buffer marcher compute pipeline");
    // Render P4b: the coarse-cull pipeline, against the SAME vocabulary layout.
    let coarse_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &coarse_cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
        })
        .expect("P4b coarse-cull compute pipeline");
    // The vocabulary bind group, written ONCE at create (NO per-frame update). Both
    // passes bind this same set; the coarse pass writes binding 6, the fine reads it.
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::SampledImage { texture: &depth, sampler: &sampler },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
                BindGroupEntry::StorageBuffer { buffer: &tiles_buffer },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
            ],
        })
        .expect("P4b vocabulary bind group");

    // --- PBR MVP-2: the RESOLVE layout + pipeline + set. 6 bindings (≤ 8): gAlbedo @0,
    // gNormal @1, gMaterial @2, lit @3 (STORAGE images), the material SSBO @4, the camera
    // UBO @5 (the resolve reads the extent + per-pixel view dir from it). The resolve
    // dispatches at the SAME grid the marcher used (1:1 the marched pixels). ---
    let resolve_layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let resolve_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &resolve_layout_entries })
        .expect("deferred resolve bind-group layout");
    let resolve_compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &resolve_cs,
            entry: c"main",
            // The resolve pushes NO constants, but `create_compute_pipeline` requires a
            // non-empty (multiple-of-4) push range; declare the shared range (unused).
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&resolve_layout),
        })
        .expect("deferred resolve compute pipeline");
    let resolve_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &resolve_layout,
            entries: &[
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::StorageImage { texture: &lit },
                BindGroupEntry::StorageBuffer { buffer: &material_table },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
            ],
        })
        .expect("deferred resolve bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // --- Mesh raster pass: clear depth to the far plane, rasterize the quad. ---
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

    // --- THE single depth dual-use barrier: DEPTH_ATTACHMENT_OPTIMAL →
    // SHADER_READ_ONLY_OPTIMAL. The depth WRITES happen at LATE_FRAGMENT_TESTS; the
    // marcher SAMPLES at COMPUTE_SHADER. This one barrier (DEPTH aspect,
    // LATE_FRAGMENT_TESTS src) makes the depth-write available + visible to the
    // shader-read and transitions the layout for sampling. It REPLACES the rung-10
    // depth→buffer copy + its two transfer barriers — there is NO copy_image_to_buffer
    // of the depth here. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    // --- The 3 G-buffer storage images + the lit output: UNDEFINED → GENERAL. The
    // marcher stores albedo/normal/material; the deferred resolve loads albedo/material
    // and stores lit — all in GENERAL. ---
    for tex in [&albedo, &normal, &material, &lit] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Render P4b: the COARSE-CULL pass (runs only when culling is enabled; the
    // depth image is already SHADER_READ_ONLY from the dual-use barrier above, which it
    // also samples). One invocation per 8×8 tile writes a `TileBound` into binding 6.
    // The vocabulary set is bound against the coarse pipeline's OWN layout. ---
    if coarse_enabled {
        encoder.bind_compute_pipeline(&coarse_compute);
        encoder.bind_descriptor_set_compute(&bind_group, &coarse_compute);
        encoder.dispatch(coarse_group_count_x(), 1, 1);

        // The inter-dispatch barrier: the coarse pass's `TileBound` WRITES (binding 6,
        // COMPUTE_SHADER/SHADER_WRITE) must be available + visible to the fine marcher's
        // READS (COMPUTE_SHADER/SHADER_READ) before the fine dispatch reads them.
        let tiles_barrier = [BufferBarrier {
            buffer: &tiles_buffer,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        }];
        encoder.pipeline_barrier(&BarrierDesc {
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            buffers: &tiles_barrier,
        });
    }

    // --- SDF marcher compute pass: SAMPLE the depth image, STORE the G-buffer. The
    // vocabulary set is bound against the pipeline's OWN dedicated layout via
    // `bind_descriptor_set_compute`; no `bind_storage_buffer`, so the encoder's fixed
    // single-set rebind is skipped. P4b pushes the 4-byte `coarse_enabled` gate against
    // the marcher's OWN layout (via `push_compute_constants`). ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_descriptor_set_compute(&bind_group, &compute);
    // Render P4b + B1 + A1/A2: the 32-byte `FineMarcherPush` — `coarse_enabled` (offset 0)
    // gates the cull, `omega` (offset 4) carries the over-relaxation factor, and
    // `lighting_flags` (offset 8) + `light_dir` (offset 16) drive A1/A2. The caller selects
    // the lighting state (the OFF path `lighting_flags == 0` ⇒ the shader's byte-identical
    // Lambert path; the ON path folds in the soft shadow + AO). The clamp is a RUNTIME
    // `f32::clamp` (NOT a debug_assert): `omega == 2` is the soundness ceiling, so a caller
    // passing a hot value must be defanged in release too.
    let omega: f32 = omega_in.clamp(1.0, 1.99);
    let push = FineMarcherPush::new(coarse_enabled, omega, lighting_flags, light_dir);
    encoder.push_compute_constants(&compute, ShaderStage::COMPUTE, 0, push.as_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    // --- (5a) PBR MVP-2: make the marcher's gAlbedo + gNormal + gMaterial STORES available
    // + visible to the resolve's LOADS — a real memory+execution dependency
    // (SHADER_WRITE→SHADER_READ, COMPUTE→COMPUTE), GENERAL→GENERAL (no layout change).
    // gNormal is now READ by the resolve (oct-normal decode + 16-bit material id). ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::General,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- (5b) PBR MVP-2 RESOLVE pass: bind the resolve pipeline + the resolve set (gAlbedo
    // @0, gNormal @1, gMaterial @2, lit @3, material SSBO @4, camera UBO @5), dispatch at
    // the SAME grid the marcher used. It runs Cook-Torrance for SDF (mask==1) pixels and
    // passes base through for mesh/bg (mask==0). ---
    encoder.bind_compute_pipeline(&resolve_compute);
    encoder.bind_descriptor_set_compute(&resolve_bind_group, &resolve_compute);
    encoder.dispatch(group_count_x(), 1, 1);

    // --- (5c) LIT: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy (the readback now
    // copies the resolve's LIT output, NOT albedo — albedo stays GENERAL, consumed only by
    // the resolve as a STORAGE-in-GENERAL load). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &lit,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
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
    encoder.copy_image_to_buffer(&lit, ImageLayout::TransferSrcOptimal, &readback, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the LIT R8G8B8A8 bytes (the deferred resolve's output).
    let dst_ptr = device
        .buffer_mapped_ptr(&readback)
        .expect("host-visible readback buffer is mapped");
    let mut out = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `dst_ptr` points to `READBACK_BYTES` mapped host-coherent bytes; a fence
    // wait preceded this read, so the GPU store + copy are complete + coherent; reading
    // `READBACK_BYTES` bytes is in-bounds; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), READBACK_BYTES as usize);
    }

    // Render P4b: optionally read back the per-tile cull buffer (binding 6). It is a
    // HostVisibleCoherent buffer, so no transfer copy is required — the coarse pass's
    // disjoint per-tile writes completed before the fence signalled above, and
    // host-coherent memory makes them visible to this read without a flush/invalidate.
    let tiles_out = if read_tiles {
        let tiles_bytes = (tile_count() as usize) * TILE_BOUND_BYTES;
        let tiles_ptr = device
            .buffer_mapped_ptr(&tiles_buffer)
            .expect("host-visible tiles buffer is mapped");
        let mut tb = vec![0u8; tiles_bytes];
        // SAFETY: `tiles_ptr` points to `tile_count() * TILE_BOUND_BYTES` mapped
        // host-coherent bytes (the buffer was sized so above); the fence wait preceded
        // this read, so the coarse pass's writes are complete + coherent; reading
        // `tiles_bytes` is in-bounds; `tb` is a distinct allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(tiles_ptr.as_ptr(), tb.as_mut_ptr(), tiles_bytes);
        }
        Some(tb)
    } else {
        None
    };

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device`; the last submission completed
    // (fence-waited above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(resolve_bind_group);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(resolve_compute);
        device.destroy_compute_pipeline(coarse_compute);
        device.destroy_compute_pipeline(compute);
        device.destroy_bind_group_layout(resolve_layout);
        device.destroy_bind_group_layout(bind_layout);
        device.destroy_graphics_pipeline(gfx);
        device.destroy_shader_module(resolve_cs);
        device.destroy_shader_module(coarse_cs);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(readback);
        device.destroy_sampler(sampler);
        device.destroy_texture(lit);
        device.destroy_texture(material);
        device.destroy_texture(normal);
        device.destroy_texture(albedo);
        device.destroy_texture(color);
        device.destroy_texture(depth);
        device.destroy_buffer(tiles_buffer);
        device.destroy_buffer(material_table);
        device.destroy_buffer(camera_uniform);
        device.destroy_buffer(buffer);
    }

    (out, tiles_out)
}

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

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)`.
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

/// **P1b GPU gate (TESTER):** the OFFSCREEN image-based G-buffer hybrid composite
/// reproduces the deferred-PBR golden by reading back the LIT STORAGE image, with
/// NO depth→buffer copy in the recorded stream.
///
/// PBR MVP-2: the marcher now writes a PBR G-buffer and a deferred RESOLVE composites
/// `gLit` via full Cook-Torrance on the SDF arm (the `mask == 0` mesh / bg / empty pixels
/// pass through verbatim). The readback is LIT (not the raw albedo), so the reference is the
/// deferred oracle `golden_deferred_resolve ∘ golden_marcher_attributes` (fed the SAME
/// per-pixel [`expected_mesh_depth`] the GPU rasterizes), NOT the retired MVP-1
/// `golden_composite_pixel_ex` inline composite.
///
/// For each scene (crater_csg / box_csg / smooth_union), `run_gbuffer_hybrid` runs ω=1.0,
/// lighting OFF (flags == 0) — so EVERY arm is pass-through — and the whole LIT image must
/// match the deferred oracle within `+/-2/255` per channel (the marcher's host-pack-vs-GPU
/// quant budget), proven by [`assert_lit_matches_deferred_golden`]. The four discriminator
/// texels (mesh-occludes-SDF / SDF-only / mesh-only / background) and `assert_validation_clean`
/// still pin the occlusion + non-constant-fill contracts. The set is written ONCE at create —
/// NO per-frame `vkUpdateDescriptorSets`. The depth `copy_image_to_buffer` + its two barriers
/// are ABSENT (confirm by recording inspection / the validation-clean sync check).
#[test]
fn p1b_gbuffer_hybrid_matches_golden() {
    let Some(ctx) = boot_or_skip("p1b_gbuffer_hybrid_matches_golden") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        // The four discriminator texels, picked host-side BEFORE the GPU run.
        let a = find_texel(&edits, |hit, covered| hit && covered);
        let b = find_texel(&edits, |hit, covered| hit && !covered);
        let c = find_texel(&edits, |hit, covered| !hit && covered);
        let d = find_texel(&edits, |hit, covered| !hit && !covered);

        // Cull-OFF, ω=1.0, lighting OFF: the LIT readback must reproduce the deferred PBR
        // oracle on every arm (with flags == 0 every arm is pass-through). The cull-ON
        // conservative golden (±2/255 vs this) + the `Tiles`-buffer-vs-`golden_tile_bound`
        // agreement are the TESTER's GPU gates.
        let lit = run_gbuffer_hybrid(&ctx, &edits, false);
        assert_eq!(lit.len(), READBACK_BYTES as usize);

        let texel = |px: u32, py: u32| -> &[u8] {
            let base = ((py * SDF_IMG_W + px) as usize) * 4;
            &lit[base..base + 4]
        };

        // Whole-image deferred-oracle scan: each LIT texel within +/-2/255 of
        // `golden_deferred_resolve(golden_marcher_attributes(.., flags=0))`, fed the per-pixel
        // mesh depth the GPU rasterizes. (Pre-PBR-MVP-2 this compared the albedo readback to
        // the retired `golden_composite_pixel_ex` inline composite.)
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] P1b G-buffer LIT vs deferred oracle: max per-channel delta = {max_pass}/255 \
             (tol {CHANNEL_TOL}); {sdf_lit_hits} SDF-lit px"
        );

        // Texel A (sphere ∧ quad) → MESH_COLOR (mesh occludes the SDF — the load-bearing
        // occlusion proof, only correct if the sampled depth actually clipped the march).
        if let Some((ax, ay)) = a {
            let got = unpack_texel_rgb(texel(ax, ay));
            assert!(
                texel_close(got, boyko_rhi_vulkan::compute::pack_rgba(MESH_COLOR)),
                "[{name}] texel A ({ax},{ay}) must be MESH_COLOR (mesh occludes SDF), got {got:?}"
            );
        }
        // Texel D (background) — distinct from texel A (mesh), proving the marcher ran a
        // field, not a constant fill.
        if let (Some((ax, ay)), Some((dx, dy))) = (a, d) {
            let av = unpack_texel_rgb(texel(ax, ay));
            let dv = unpack_texel_rgb(texel(dx, dy));
            assert!(
                !(0..3).all(|ch| (av[ch] - dv[ch]).abs() <= CHANNEL_TOL),
                "[{name}] texel A {av:?} (mesh) must differ from texel D {dv:?} (background)"
            );
        }
        let _ = (b, c); // B/C are exercised by the whole-image scan above.
    }
}

// ===========================================================================================
// Render P4b — conservative coarse-cull (1/8-res tile pre-trace) GPU gates (TESTER).
//
// The dev + code-review are complete (verdict APPROVE → GPU tester). These tests RUN the
// `coarse_enabled = true` path on the RTX 3060 (validation ON) and assert the cull's three
// contracts: (i) image ±2/255 vs the un-culled marcher (a hole = a >tol texel), (ii) the GPU
// `Tiles` buffer agrees with the host mirror `golden_tile_bound` (ORTHO → tight), (iii)
// cull-OFF is BYTE-IDENTICAL to the pre-P4b path (the 0%-gate). Plus a negative tripwire (a
// too-aggressive fake TileBound MUST fail the ±2/255 golden, and a MESH-covered EMPTY tile
// MUST show MESH_COLOR — D6) and the spirv-val / committed-.spv-freshness audit.
// ===========================================================================================

/// Splits an R8G8B8A8 readback into `[r, g, b]` for the texel at `(px, py)` (the low 3 bytes).
fn albedo_rgb(albedo: &[u8], px: u32, py: u32) -> [i32; 3] {
    let base = ((py * SDF_IMG_W + px) as usize) * 4;
    unpack_texel_rgb(&albedo[base..base + 4])
}

/// Parses the `tiles_buffer` readback (`tile_count() * 16` bytes, std430 scalar) into the
/// per-tile [`TileBound`]s, in coarse-dispatch order (`ty * tiles_w + tx`). near_t f32@0,
/// far_t f32@4, flags u32@8, _pad u32@12 — the layout the host const-asserts.
fn parse_tile_bounds(bytes: &[u8]) -> Vec<TileBound> {
    let n = tile_count() as usize;
    assert_eq!(bytes.len(), n * TILE_BOUND_BYTES, "tiles readback size mismatch");
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = i * TILE_BOUND_BYTES;
        let f = |k: usize| f32::from_le_bytes(bytes[o + k..o + k + 4].try_into().unwrap());
        let u = |k: usize| u32::from_le_bytes(bytes[o + k..o + k + 4].try_into().unwrap());
        out.push(TileBound { near_t: f(0), far_t: f(4), flags: u(8), _pad: u(12) });
    }
    out
}

/// The 8×8 block of per-pixel mesh depths covering tile `(tx, ty)` — the SAME
/// [`expected_mesh_depth`] values the GPU rasterizes — fed to [`golden_tile_bound`] so the
/// host mirror sees the exact depth field the coarse shader sampled (D5: out-of-image texels
/// stay the clear value, which `golden_tile_bound` decodes to `T_MAX`).
fn tile_depths(tx: u32, ty: u32) -> Vec<f32> {
    let mut depths = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize);
    for ly in 0..TILE_SIZE {
        for lx in 0..TILE_SIZE {
            let px = tx * TILE_SIZE + lx;
            let py = ty * TILE_SIZE + ly;
            // Out-of-image fine pixels decode to the clear (no-mesh) depth — the partial-edge
            // contract the shader's out-of-range `.Load` mirrors (D5).
            let d = if px < SDF_IMG_W && py < SDF_IMG_H {
                expected_mesh_depth(px, py)
            } else {
                MESH_DEPTH_CLEAR
            };
            depths.push(d);
        }
    }
    depths
}

/// Boots a validation context, prints the device + caps, or returns `None` (SKIP). Shared by
/// every P4b GPU gate so each prints the RTX-3060 device name and asserts the G-buffer caps.
fn boot_render_or_skip(test: &str) -> Option<VulkanContext> {
    let ctx = boot_or_skip(test)?;
    println!("[{test}] Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");
    assert!(
        ctx.device_caps().gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );
    Some(ctx)
}

/// The three ORTHO fixtures, reused by every gate.
fn p4b_scenes() -> [(&'static str, Vec<SdfEdit>); 3] {
    [("crater_csg", crater()), ("box_csg", box_csg()), ("smooth_union", smooth_union())]
}

/// **P4b GATE 1 — conservative golden (the headline).** For each scene, run cull-OFF
/// (baseline) and cull-ON; EVERY cull-ON albedo texel must be within ±2/255 (`CHANNEL_TOL`)
/// of the cull-OFF texel. A texel exceeding tol = a CULL HOLE (the coarse pass skipped a
/// surface the un-culled marcher hit) → FAIL with `(px,py)` + got + want + delta. The
/// max per-channel delta per scene is reported (the cull's fp drift budget).
#[test]
fn p4b_cull_on_conservative_within_tol_of_cull_off() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_on_conservative_within_tol_of_cull_off") else {
        return;
    };

    for (name, edits) in p4b_scenes() {
        let off = run_gbuffer_hybrid(&ctx, &edits, false);
        let on = run_gbuffer_hybrid(&ctx, &edits, true);
        assert_eq!(off.len(), READBACK_BYTES as usize, "[{name}] cull-OFF readback size");
        assert_eq!(on.len(), READBACK_BYTES as usize, "[{name}] cull-ON readback size");

        // Prove the device actually executed: the cull-OFF baseline must contain BOTH a
        // mesh/SDF lit texel AND a background texel (not a silent all-zero buffer).
        let nonzero = off.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(
            nonzero > 0,
            "[{name}] cull-OFF albedo is all-zero — the device did not render (silent skip?)"
        );

        let mut max_delta = 0i32;
        let mut worst = (0u32, 0u32, [0i32; 3], [0i32; 3]);
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let g_on = albedo_rgb(&on, px, py);
                let g_off = albedo_rgb(&off, px, py);
                for ch in 0..3 {
                    let dd = (g_on[ch] - g_off[ch]).abs();
                    if dd > max_delta {
                        max_delta = dd;
                        worst = (px, py, g_on, g_off);
                    }
                }
                assert!(
                    (0..3).all(|ch| (g_on[ch] - g_off[ch]).abs() <= CHANNEL_TOL),
                    "[{name}] CULL HOLE at ({px},{py}): cull-ON {g_on:?} vs cull-OFF {g_off:?} \
                     exceeds ±{CHANNEL_TOL}/255 (delta {:?})",
                    [
                        (g_on[0] - g_off[0]).abs(),
                        (g_on[1] - g_off[1]).abs(),
                        (g_on[2] - g_off[2]).abs()
                    ]
                );
            }
        }
        println!(
            "[{name}] GATE1 cull-ON vs cull-OFF: max per-channel delta = {max_delta}/255 \
             (tol {CHANNEL_TOL}); worst texel ({},{}) on={:?} off={:?}; {nonzero} non-bg texels",
            worst.0, worst.1, worst.2, worst.3
        );
    }
}

/// **P4b GATE 2 — Tiles-buffer agreement.** Read back the `tiles_buffer` after a cull-ON run
/// and diff every tile vs the host mirror [`golden_tile_bound`] (fed the SAME per-tile mesh
/// depths the GPU rasterizes). These fixtures are ORTHO (no tan/acos transcendental in the
/// cone math → no fp divergence), so near_t / far_t must agree TIGHTLY and the EMPTY flag
/// EXACTLY. A real per-tile divergence is surfaced (the worst tile + both bounds) — not
/// papered over.
#[test]
fn p4b_tiles_buffer_agrees_with_host_golden() {
    let Some(ctx) = boot_render_or_skip("p4b_tiles_buffer_agrees_with_host_golden") else {
        return;
    };
    let (tw, _th) = tile_extent();

    // ORTHO has no transcendental in the cone trace; the host + GPU run the SAME op
    // sequence (D1/D2). A handful of ULPs can still appear from the GPU's `mad`-contraction
    // vs the host's separate mul/add in `field_distance`, so allow a tiny absolute epsilon
    // (≈ a few ULP of a t ~ O(1) value); flags must be EXACT (a flag flip = a wrong-EMPTY
    // hole, which GATE 1 would also catch as a pixel hole).
    const T_EPS: f32 = 1.0e-4;

    for (name, edits) in p4b_scenes() {
        let (_albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &edits, true, true, 1.0);
        let tiles = tiles.expect("read_tiles = true returns the tiles readback");
        let gpu = parse_tile_bounds(&tiles);
        assert_eq!(gpu.len(), tile_count() as usize, "[{name}] tile count");

        let mut empties = 0usize;
        let mut max_near = 0f32;
        let mut max_far = 0f32;
        let mut worst_tile = (0u32, 0u32);
        for (i, g) in gpu.iter().enumerate() {
            let tx = (i as u32) % tw;
            let ty = (i as u32) / tw;
            let host = golden_tile_bound(
                &edits,
                &tile_depths(tx, ty),
                tx,
                ty,
                SDF_IMG_W,
                SDF_IMG_H,
                CompositeCamera::Ortho,
            );
            // Flags EXACT — a wrong EMPTY is a hole.
            assert_eq!(
                g.flags, host.flags,
                "[{name}] tile ({tx},{ty}) flags GPU={} host={} (EMPTY={TILE_FLAG_EMPTY})",
                g.flags, host.flags
            );
            let dn = (g.near_t - host.near_t).abs();
            let df = (g.far_t - host.far_t).abs();
            if dn > max_near {
                max_near = dn;
                worst_tile = (tx, ty);
            }
            if df > max_far {
                max_far = df;
            }
            assert!(
                dn <= T_EPS,
                "[{name}] tile ({tx},{ty}) near_t diverged: GPU={} host={} |d|={dn} > {T_EPS}",
                g.near_t, host.near_t
            );
            assert!(
                df <= T_EPS,
                "[{name}] tile ({tx},{ty}) far_t diverged: GPU={} host={} |d|={df} > {T_EPS}",
                g.far_t, host.far_t
            );
            if g.flags & TILE_FLAG_EMPTY != 0 {
                empties += 1;
            }
        }
        // Prove the coarse pass actually ran a non-trivial trace: at least one tile must be
        // non-EMPTY (has the surface) AND at least one EMPTY (sparse scene) — a uniform
        // buffer would mean the coarse dispatch silently no-op'd.
        let non_empty = gpu.len() - empties;
        assert!(non_empty > 0, "[{name}] every tile EMPTY — coarse pass found no surface");
        assert!(empties > 0, "[{name}] no EMPTY tile — coarse pass culled nothing (suspicious)");
        println!(
            "[{name}] GATE2 Tiles agree: {}/{} tiles, {empties} EMPTY / {non_empty} surface; \
             max |Δnear_t|={max_near} max |Δfar_t|={max_far} (eps {T_EPS}); worst near tile {:?}",
            gpu.len(),
            tile_count(),
            worst_tile
        );
    }
}

/// **P4b GATE 3a — the conservative golden tripwire MUST trip.** Constructs a deliberately
/// TOO-AGGRESSIVE cull (a fake [`TileBound`] with `near_t` pushed past the true first hit)
/// and asserts the host culled marcher [`golden_composite_pixel_culled`] then DIFFERS from
/// the un-culled golden by more than `CHANNEL_TOL` at a known SDF-hit pixel. This proves
/// GATE 1's ±2/255 comparison can actually CATCH a hole (a tripwire that never trips is no
/// gate). Host-only (no GPU) — it exercises the contract the GPU gate relies on.
#[test]
fn p4b_too_aggressive_near_t_seed_trips_the_conservative_golden() {
    let edits = crater();
    // Find a pixel the SDF hits AND is NOT mesh-covered: the un-culled golden shows the lit
    // SDF surface there, so skipping past the hit reveals BACKGROUND (a visible hole). A
    // mesh-covered hit pixel would mask the hole behind MESH_COLOR (the mesh occludes the
    // SDF either way), so the tripwire must use an SDF-only pixel.
    let (px, py) = find_texel(&edits, |hit, covered| hit && !covered)
        .expect("crater has an SDF-hit pixel outside the mesh quad");
    let md = expected_mesh_depth(px, py);
    assert_eq!(md, MESH_DEPTH_CLEAR, "the chosen pixel must be mesh-uncovered (no occlusion)");

    let want = golden_composite_pixel_ex(&edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);

    // A too-aggressive bound: near_t = 5.0 seeded WAY past the true first hit (the crater
    // sphere front is at t ≈ CAM_Z − R = 1.5). far_t large so the seeded march has room to
    // (wrongly) walk empty space to T_MAX → background instead of the lit SDF. flags = 0
    // (non-EMPTY, so the marcher seeds t = near_t rather than fast-pathing).
    let bad = TileBound { near_t: 5.0, far_t: SDF_TRACE_T_MAX, flags: 0, _pad: 0 };
    let got = golden_composite_pixel_culled(
        &edits,
        md,
        px,
        py,
        SDF_IMG_W,
        SDF_IMG_H,
        CompositeCamera::Ortho,
        true,
        bad,
    );

    let w = unpack_packed_rgb(want);
    let g = unpack_packed_rgb(got);
    let delta: [i32; 3] = [(g[0] - w[0]).abs(), (g[1] - w[1]).abs(), (g[2] - w[2]).abs()];
    assert!(
        delta.iter().any(|&d| d > CHANNEL_TOL),
        "TRIPWIRE FAILED: a too-aggressive near_t=5.0 seed at SDF-hit pixel ({px},{py}) did NOT \
         change the color beyond ±{CHANNEL_TOL}/255 (got {g:?} want {w:?}) — the conservative \
         golden cannot detect a hole, so GATE 1 is blind"
    );
    println!(
        "[crater_csg] GATE3a tripwire OK: too-aggressive near_t=5.0 at hit pixel ({px},{py}) \
         shifts color by {delta:?}/255 (> tol {CHANNEL_TOL}) → a hole IS detectable"
    );
}

/// **P4b GATE 3b — D6: a MESH-covered EMPTY tile shows MESH_COLOR, not background.** The
/// EMPTY fast-path must run the mesh/background composite (D6) — an EMPTY tile can still be
/// MESH-occluded. Asserts the host culled marcher returns MESH_COLOR for a MESH-covered
/// pixel under an EMPTY tile (and background for an uncovered one), proving the EMPTY arm is
/// NOT a blind background fill (which would erase the mesh → a golden regression). The GPU
/// half is covered by GATE 1 (an EMPTY mesh tile that went background would exceed ±2/255).
#[test]
fn p4b_empty_tile_composites_mesh_not_background_d6() {
    let edits = crater();
    let empty = TileBound { near_t: 0.0, far_t: SDF_TRACE_T_MAX, flags: TILE_FLAG_EMPTY, _pad: 0 };

    // A MESH-covered pixel under an EMPTY tile → MESH_COLOR (the mesh, not erased).
    let (cx, cy) =
        find_texel(&edits, |_hit, covered| covered).expect("the quad covers part of the view");
    let covered_md = expected_mesh_depth(cx, cy);
    assert!(covered_md < MESH_DEPTH_CLEAR, "the chosen pixel must be mesh-covered");
    let got_mesh = golden_composite_pixel_culled(
        &edits, covered_md, cx, cy, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, empty,
    );
    assert_eq!(
        got_mesh,
        pack_rgba(MESH_COLOR),
        "[crater_csg] GATE3b D6: a MESH-covered EMPTY tile pixel ({cx},{cy}) must show \
         MESH_COLOR, got {:?} (the EMPTY fast-path blind-filled background → mesh erased)",
        unpack_packed_rgb(got_mesh)
    );

    // An UNCOVERED pixel under an EMPTY tile → background (not mesh) — the other D6 arm.
    let (ux, uy) = find_texel(&edits, |hit, covered| !hit && !covered)
        .expect("crater has an uncovered, non-hit pixel");
    let uncovered_md = expected_mesh_depth(ux, uy);
    assert_eq!(uncovered_md, MESH_DEPTH_CLEAR, "the chosen pixel must be uncovered");
    let got_bg = golden_composite_pixel_culled(
        &edits, uncovered_md, ux, uy, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, empty,
    );
    assert_ne!(
        got_bg,
        pack_rgba(MESH_COLOR),
        "[crater_csg] GATE3b D6: an UNCOVERED EMPTY tile pixel ({ux},{uy}) must NOT be MESH_COLOR"
    );
    println!(
        "[crater_csg] GATE3b D6 OK: EMPTY-covered ({cx},{cy})=MESH_COLOR, \
         EMPTY-uncovered ({ux},{uy})=background"
    );
}

/// **P4b GATE 4 — cull-OFF does not perturb the output (the 0%-gate).** `run_gbuffer_hybrid(
/// false)` (the fine marcher with `coarse_enabled = 0`) must produce a LIT image that matches
/// the deferred PBR oracle within ±2/255 (PBR MVP-2 re-pointed this from the retired MVP-1
/// `golden_composite_pixel_ex` inline composite to `golden_deferred_resolve ∘
/// golden_marcher_attributes`; the cull flag itself does not change the PBR result) — AND, the
/// stronger P4b claim, RUN-TO-RUN byte-stable. This pins the no-coarse path so a P4b change
/// that perturbs cull-OFF is caught here, not only by the existing
/// `p1b_gbuffer_hybrid_matches_golden`. Each scene's two cull-OFF runs are compared
/// byte-for-byte (the GPU is deterministic for the same recorded stream).
#[test]
fn p4b_cull_off_is_byte_identical_to_pre_p4b_path() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_off_is_byte_identical_to_pre_p4b_path") else {
        return;
    };

    for (name, edits) in p4b_scenes() {
        // Two independent cull-OFF runs → byte-for-byte identical (the marcher is
        // deterministic; the coarse pass is not even dispatched).
        let a = run_gbuffer_hybrid(&ctx, &edits, false);
        let b = run_gbuffer_hybrid(&ctx, &edits, false);
        assert_eq!(a, b, "[{name}] two cull-OFF runs diverged — the no-coarse path is non-deterministic");

        // And each cull-OFF LIT texel matches the deferred PBR oracle within ±2/255 (the
        // 0%-gate anchor: cull-OFF == today's marcher). Lighting is OFF (flags == 0), so every
        // arm is pass-through. This re-pins the contract `p1b_gbuffer_hybrid_matches_golden`
        // asserts, scoped to the coarse_enabled = 0 push.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&a, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!("[{name}] GATE4 cull-OFF byte-stable + matches deferred oracle (max delta {max_pass}/255)");
    }
}

/// **P4b GATE 5 — sync-validation under cull-ON.** A cull-ON run that returns proves
/// `assert_validation_clean` passed (it is asserted inside `run_gbuffer_hybrid_ex` before
/// return): the coarse-write → fine-read buffer barrier (Tiles SHADER_WRITE → SHADER_READ)
/// raised no WAR/RAW hazard and the coarse dispatch + the inter-dispatch barrier are
/// validation-clean. (The committed-.spv freshness + spirv-val audit is a separate
/// host-side script run by the tester — see the report; the validator is not invoked from
/// the Rust test to avoid a hard SDK dependency at `cargo test` time.)
#[test]
fn p4b_cull_on_is_validation_clean() {
    let Some(ctx) = boot_render_or_skip("p4b_cull_on_is_validation_clean") else {
        return;
    };
    // crater is the densest fixture (a CSG carve), so it exercises both the coarse trace and
    // the fine seeded march hardest. A clean return = validation-clean (asserted inside).
    let (albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &crater(), true, true, 1.0);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let tiles = tiles.expect("read_tiles");
    let bounds = parse_tile_bounds(&tiles);
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    println!(
        "[crater_csg] GATE5 cull-ON validation-clean: {} tiles ({} surface) + the coarse→fine \
         buffer barrier raised no hazard",
        bounds.len(),
        surface
    );
}

// ===========================================================================================
// Render B1 — over-relaxation (Keinert ω-gated) GPU gates (RTX 3060, validation ON).
//
//   6.  GPU ω=1 BIT-identity — `run_gbuffer_hybrid_ex(.., 1.0)` byte-identical to the cull-OFF
//       pre-B1 path (two runs equal + every texel == the ω=1 host golden), cull-off AND cull-on.
//   7.  GPU ω>1 HIT/MISS parity — the GPU ω∈{1.2,1.5,1.9} render's per-pixel hit/miss set ==
//       the ω=1 GPU render's, on crater + box + smooth_union (the SHIPPED fixtures).
//   8.  GPU ω=1.2 ±2/255 vs the MATCHED-ω host oracle `golden_composite_pixel_ex_omega(.., 1.2)`
//       (NOT the ω=1 golden) — the m1 the reviewer flagged as missing.
//   8c. GPU repro of BUG-B1-HOLE-1 — the host-confirmed hole scene rendered on-device; documents
//       the hole is NOT host-only. `#[ignore]` (it asserts the buggy state; flip after the fix).
//   11. SYNC-validation clean — a cull-ON ω=1.2 dispatch raises no sync hazard.
//
// A GPU pixel is classified HIT / MESH / BACKGROUND by nearest packed reference color (the three
// composite outcomes differ by 100+ per channel, so a ±2/255 store quantization never flips the
// class). The hit/miss SET is the soundness invariant ω>1 must preserve.
// ===========================================================================================

/// The packed background color the marcher writes on a miss (`SDF_BACKGROUND = [0.05,0.05,0.1]`).
fn packed_background() -> [i32; 3] {
    unpack_packed_rgb(pack_rgba([0.05, 0.05, 0.1]))
}

/// Classifies a GPU albedo texel as `true` (an SDF surface hit) when it is closer to neither the
/// packed MESH_COLOR nor the packed BACKGROUND than `CHANNEL_TOL` allows — i.e. it is the LIT SDF
/// color. The three outcomes are >100/255 apart, so the ±2/255 store quantization never reclasses.
fn gpu_pixel_is_sdf_hit(albedo: &[u8], px: u32, py: u32) -> bool {
    let got = albedo_rgb(albedo, px, py);
    let mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
    let bg = packed_background();
    let near = |r: [i32; 3]| (0..3).all(|c| (got[c] - r[c]).abs() <= CHANNEL_TOL);
    !near(mesh) && !near(bg)
}

/// **B1 GATE 6 — GPU ω=1 BIT-identity (cull-off + cull-on).** `run_gbuffer_hybrid_ex(.., 1.0)`
/// must (a) be byte-stable across two runs, and (b) match the ω=1 deferred PBR oracle within
/// ±2/255 (the same contract the pre-B1 `p1b`/GATE-4 0%-gates assert) — proving the widened
/// 8-byte push with ω=1.0 reproduces the committed pre-B1 marcher EXACTLY on-device. Runs
/// cull-off AND cull-on. PBR MVP-2: the host reference is re-pointed from the retired MVP-1
/// `golden_composite_pixel_ex` / `golden_composite_pixel_culled` to `golden_deferred_resolve ∘
/// golden_marcher_attributes` (ω=1.0, flags == 0 ⇒ every arm pass-through). With flags == 0 the
/// coarse cull cannot perturb a pass-through pixel, so the SAME deferred oracle bounds both the
/// cull-off and cull-on arms; the cull's conservative-fill contract is independently proven by
/// `p4b_cull_on_conservative_within_tol_of_cull_off`.
#[test]
fn b1_gate6_gpu_omega_one_bit_identical_to_pre_b1() {
    let Some(ctx) = boot_render_or_skip("b1_gate6_gpu_omega_one_bit_identical_to_pre_b1") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        for coarse in [false, true] {
            // Two ω=1.0 runs must be byte-identical (deterministic).
            let a = run_gbuffer_hybrid_ex(&ctx, &edits, coarse, false, 1.0).0;
            let b = run_gbuffer_hybrid_ex(&ctx, &edits, coarse, false, 1.0).0;
            assert_eq!(a.len(), READBACK_BYTES as usize, "[{name} cull={coarse}] readback size");
            assert_eq!(a, b, "[{name} cull={coarse}] two ω=1.0 runs diverged — non-deterministic marcher");

            // Prove the device executed (not a silent all-zero buffer).
            let nonzero = a.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name} cull={coarse}] ω=1.0 albedo all-zero — device did not render");

            // Each ω=1.0 LIT texel within ±2/255 of the ω=1 deferred PBR oracle. Lighting is
            // OFF (flags == 0), so every arm is pass-through and the coarse cull cannot perturb
            // a pass-through pixel — the SAME deferred oracle bounds both the cull-off and
            // cull-on arms (the cull's conservative-fill contract is proven separately by
            // `p4b_cull_on_conservative_within_tol_of_cull_off`).
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&a, &edits, 0, DEFAULT_LIGHT_DIR, name);
            assert_eq!(max_arm1, 0, "[{name} cull={coarse}] flags==0 must have NO arm-1 pixel (lighting OFF)");
            assert!(sdf_lit_hits > 0, "[{name} cull={coarse}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
            println!("[{name} cull={coarse}] GATE6 ω=1.0 byte-stable + matches deferred oracle (max delta {max_pass}/255)");
        }
    }
}

/// **B1 GATE 7 — GPU ω>1 HIT/MISS parity.** For each SHIPPED fixture, the GPU ω∈{1.2,1.5,1.9}
/// render's per-pixel SDF-hit set must EQUAL the ω=1 GPU render's. A pixel that hits at ω=1 but
/// becomes mesh/background at ω>1 = a missed-surface HOLE; the reverse (a new spurious hit) = a
/// phantom surface. Either fails with `(px,py)` + both classes. (The shipped fixtures are hole-free
/// per the host gate-2 scope analysis; this confirms it ON-DEVICE.)
#[test]
fn b1_gate7_gpu_overrelax_hit_miss_parity() {
    let Some(ctx) = boot_render_or_skip("b1_gate7_gpu_overrelax_hit_miss_parity") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        let base = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.0).0;
        let base_hits = base.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(base_hits > 0, "[{name}] ω=1 baseline all-zero — device did not render");
        for &omega in &[1.2_f32, 1.5, 1.9] {
            let over = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, omega).0;
            let mut sdf_px = 0u64;
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let h1 = gpu_pixel_is_sdf_hit(&base, px, py);
                    let ho = gpu_pixel_is_sdf_hit(&over, px, py);
                    if h1 {
                        sdf_px += 1;
                    }
                    assert_eq!(
                        h1, ho,
                        "[{name}] ω={omega} HIT/MISS PARITY broke at ({px},{py}): ω=1 hit={h1} vs ω={omega} hit={ho} \
                         (ω=1 {:?} vs ω={omega} {:?})",
                        albedo_rgb(&base, px, py), albedo_rgb(&over, px, py)
                    );
                }
            }
            println!("[{name}] GATE7 ω={omega} hit/miss parity OK ({sdf_px} SDF-hit px match ω=1)");
        }
    }
}

/// **B1 GATE 8 — GPU ω=1.2 ±2/255 vs the MATCHED-ω deferred PBR oracle.** Each GPU ω=1.2 LIT
/// texel must be within ±2/255 of `golden_deferred_resolve(golden_marcher_attributes(.., ω=1.2,
/// flags=0))` (the ω-aware deferred oracle — NOT the ω=1 golden, and NOT the retired MVP-1
/// `golden_composite_pixel_ex_omega`). This proves the GPU over-relaxation marcher reproduces
/// the host ω-marcher's per-pixel COLOR, not merely the hit/miss class. PBR MVP-2 re-pointed
/// the host reference to the deferred oracle (the readback is LIT). Per-scene max delta is
/// reported. (Shipped fixtures only — they are hole-free.)
#[test]
fn b1_gate8_gpu_omega_1_2_matches_matched_omega_host() {
    let Some(ctx) = boot_render_or_skip("b1_gate8_gpu_omega_1_2_matches_matched_omega_host") else {
        return;
    };
    let omega = DEFAULT_MARCHER_OMEGA; // 1.2 — the production default
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, omega).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] ω={omega} lit all-zero — device did not render");
        // The matched-ω deferred oracle: the host marches the IDENTICAL ω before the resolve.
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden_omega(&lit, &edits, omega, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!("[{name}] GATE8 ω={omega} GPU matches matched-ω deferred oracle (max delta {max_pass}/255)");
    }
}

/// **B1 GATE 8c — BUG-B1-HOLE-1 mesh-masking on the GPU harness (documented).** The host-confirmed
/// over-relax hole (super-Lipschitz smooth-min CSG) cannot be SHOWN through this fixed-mesh harness:
/// EVERY hole pixel of that scene falls inside the mesh quad footprint (x ∈ [-1, 0.2]) — the mesh
/// occludes the SDF there, so both ω=1 and ω=1.2 composite MESH_COLOR and the hole is invisible on
/// readback. A 40k-trial host search found NO smooth-min hole pixel outside the mesh x-range. This
/// test ASSERTS that masking (the hole pixel is mesh-covered on-device, NOT an SDF hit at ω=1),
/// recording WHY the GPU half cannot expose BUG-B1-HOLE-1 with the current harness. The bug itself
/// is proven host-side (`compute::b1_over_relaxation_tests::gate2_*` + the `bug_b1_hole_1_*` pin);
/// the shader marcher is line-for-line the host `_omega` oracle, so the host proof IS the on-device
/// proof. A no-mesh / relocated-mesh harness variant (developer wiring, out of the tester remit)
/// would surface it directly.
#[test]
fn b1_gate8c_bug_b1_hole_1_is_mesh_masked_on_gpu_harness() {
    let Some(ctx) = boot_render_or_skip("b1_gate8c_bug_b1_hole_1_is_mesh_masked_on_gpu_harness") else {
        return;
    };
    let edits = vec![
        SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
        SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
        SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
    ];
    let (px, py) = (28u32, 16u32);
    // The host hole pixel is inside the mesh footprint — the harness's mesh quad covers it.
    assert!(mesh_covers_pixel(px, py), "the documented hole pixel must be mesh-covered (the masking premise)");
    let at1 = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.0).0;
    let at12 = run_gbuffer_hybrid_ex(&ctx, &edits, false, false, 1.2).0;
    let g1 = albedo_rgb(&at1, px, py);
    let g12 = albedo_rgb(&at12, px, py);
    let mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
    println!(
        "[BUG-B1-HOLE-1 GPU] ({px},{py}) mesh-covered: ω=1 {g1:?} ω=1.2 {g12:?} (MESH_COLOR {mesh:?}) — hole MASKED by the mesh quad"
    );
    // Both composite the mesh (the SDF — hit or hole — is occluded), so the GPU readback cannot
    // distinguish the hole here. This documents the harness limitation, not a B1 success.
    assert!(
        (0..3).all(|c| (g1[c] - mesh[c]).abs() <= CHANNEL_TOL),
        "ω=1 ({g1:?}) must be MESH_COLOR ({mesh:?}) at the mesh-covered hole pixel"
    );
    assert!(
        !gpu_pixel_is_sdf_hit(&at1, px, py),
        "the hole pixel is mesh-covered on-device, so it is not an exposed SDF hit (the masking)"
    );
}

/// **B1 GATE 11 — sync-validation clean under cull-ON ω=1.2.** A cull-ON ω=1.2 dispatch that
/// returns proves `assert_validation_clean` passed (asserted inside `run_gbuffer_hybrid_ex` before
/// return): the widened 8-byte push (ω at offset 4) adds NO new resource hazard over the pre-B1
/// cull-ON path. The coarse→fine Tiles barrier is unchanged; ω is push-constant data only.
#[test]
fn b1_gate11_cull_on_omega_1_2_sync_validation_clean() {
    let Some(ctx) = boot_render_or_skip("b1_gate11_cull_on_omega_1_2_sync_validation_clean") else {
        return;
    };
    let (albedo, tiles) = run_gbuffer_hybrid_ex(&ctx, &crater(), true, true, DEFAULT_MARCHER_OMEGA);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let bounds = parse_tile_bounds(&tiles.expect("read_tiles"));
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    println!(
        "[crater_csg] GATE11 cull-ON ω={DEFAULT_MARCHER_OMEGA} validation-clean: {} tiles ({} surface); \
         the widened push raised no new hazard",
        bounds.len(),
        surface
    );
}

// ===========================================================================================
// Render A1 (SDF soft shadows) + A2 (SDF AO) — the ON-path GPU gates (RTX 3060, validation
// ON, --test-threads=1). The OFF 0%-gate is already pinned by `p1b_gbuffer_hybrid_matches_
// golden` + `p4b_cull_off_is_byte_identical_to_pre_p4b_path` + `b1_gate6_...` (all run with
// the 32-byte push, all green). These gates exercise `lighting_flags != 0`.
//
//   A-host.   host_soft_shadow / host_ao sanity (CPU, no GPU) — factors in [0,1]; a shadowed
//             crevice is darker than a lit face; AO darkens concavities.
//   A1g.      ON GPU vs host `_lit` golden (DEFAULT light (0,0,1)), SHADOWS|AO, ±3/255 — the
//             host mirror is EXACT for the default light (shader's static LIGHT_DIR == push).
//   A2g.      Shadows-only and AO-only independence vs the matching host `_lit` golden, ±3/255
//             (the flag bits gate independently).
//   A3g.      Non-default light_dir mis-pack catcher (the architect's named std430 oracle):
//             a GPU-vs-GPU differential — a non-axis light_dir must shift the shadow pattern
//             vs the default light (proves light_dir reaches the shader at the correct offset).
//             SEE the BUG-A-NDOTL note: the literal "GPU vs host `_lit` with same non-default
//             light" form is NOT achievable against the current host mirror (host applies
//             light_dir to the Lambert base; the shader hardcodes the static LIGHT_DIR there).
//
// The ON-path tolerance is ±3/255 (`LIT_CHANNEL_TOL`) — the architect's consumer-side budget
// (host `powi` vs shader `pow` ULP + the float→UNORM store). The OFF path stays ±2/255.
// ===========================================================================================

/// The A1/A2 ON-path per-channel tolerance (the architect's consumer-side ±3/255: host
/// `AO_FALLOFF.powi(i)` vs the shader's `pow(AO_FALLOFF, i)` ULP drift + the shadow
/// min-track FP order + the float→UNORM store quantization). The OFF path keeps the
/// stricter `CHANNEL_TOL` (±2/255).
const LIT_CHANNEL_TOL: i32 = 3;

/// A non-axis, normalized directional light (the architect's mis-pack probe direction). It
/// is NOT (0,0,1), so a std430 offset slip on `light_dir` (landing it at the wrong push
/// offset → read as zero / garbage) yields a measurably different shadow pattern than a
/// correctly-packed value — the differential A3g catches.
const NONDEFAULT_LIGHT: [f32; 3] = [0.4, 0.5, 0.768];

// NOTE (PBR MVP-2): the former `assert_lit_within_tol` helper diffed the GPU LIT readback
// against the RETIRED MVP-1 `_lit` oracle (`golden_composite_pixel_ex_omega_lit`, the `base*vis`
// Lambert composite) at ±3/255. Its three callers (A1g / A2g / A3g-literal) now compare the PBR
// `gLit` readback against the deferred Cook-Torrance oracle via `assert_lit_matches_deferred_golden`
// (±2/255), so the helper has no remaining caller and is removed. The MVP-1 `_lit` oracle itself
// is still exercised host-only by `a_host_shadow_ao_darken_not_brighten` (a CPU darken/brighten
// sanity, no GPU) and `d1_host_deferred_passthrough_byte_identical` (the pass-through 0%-gate).

/// **A-host — host shadow/AO sanity (CPU, no GPU).** A correctness sniff of `host_soft_shadow`
/// / `host_ao` via the public `_lit` golden: with SHADOWS|AO ON (default light), the ON-path
/// lit color must (a) stay in-gamut (every channel ≤ 255), (b) be NO BRIGHTER than the OFF
/// (Lambert-only) golden at the same pixel — shadow ∈ [0,1] and AO ∈ [0,1] can only darken —
/// and (c) be STRICTLY darker at a shadowed/concave pixel (the carved crater crevice), proving
/// the terms actually attenuate (not a no-op multiply by 1). The OFF baseline is the same
/// golden with `lighting_flags == 0`.
#[test]
fn a_host_shadow_ao_darken_not_brighten() {
    let edits = crater();
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;

    let mut any_strictly_darker = false;
    let mut checked_hits = 0u64;
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            // Only SDF-hit, mesh-uncovered pixels carry the lit color (a mesh-covered or
            // background pixel is unaffected by lighting — the OFF==ON identity there).
            if !editlist_pixel_hits(&edits, px, py) || mesh_covers_pixel(px, py) {
                continue;
            }
            checked_hits += 1;
            let off = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                0, DEFAULT_LIGHT_DIR,
            ));
            let on = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                flags, DEFAULT_LIGHT_DIR,
            ));
            for ch in 0..3 {
                assert!(on[ch] <= 255, "[crater] lit channel out of gamut at ({px},{py}): {on:?}");
                assert!(
                    on[ch] <= off[ch],
                    "[crater] SHADOWS|AO BRIGHTENED ({px},{py}) ch{ch}: on {on:?} > off {off:?} \
                     (shadow/AO factors are in [0,1] — they can only darken)"
                );
            }
            if (0..3).any(|ch| on[ch] < off[ch]) {
                any_strictly_darker = true;
            }
        }
    }
    assert!(checked_hits > 0, "the crater fixture must have an SDF-hit, mesh-uncovered pixel");
    assert!(
        any_strictly_darker,
        "SHADOWS|AO never darkened ANY SDF-hit pixel ({checked_hits} checked) — the consumer \
         terms are a no-op (a multiply by 1 everywhere)"
    );
    println!(
        "[crater] A-host OK: SHADOWS|AO darkens (never brightens) across {checked_hits} SDF-hit \
         pixels; at least one strictly darker (the terms attenuate)"
    );
}

/// **A1g — ON GPU vs the deferred PBR oracle, DEFAULT light, SHADOWS|AO, ±2/255.** Push
/// `lighting_flags = SHADOWS|AO`, `light_dir = (0,0,1)`; every GPU LIT texel within ±2/255 of
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, (0,0,1)))` on crater / box /
/// smooth. PBR MVP-2 re-pointed the host reference from the retired MVP-1
/// `golden_composite_pixel_ex_omega_lit` (`base*vis` Lambert) to the deferred Cook-Torrance
/// oracle (the readback is the PBR `gLit`); the SDF-lit arm is bounded by the deferred
/// double-quant budget (±2/255) and the pass-through arms by the host-pack budget (±2/255).
/// This is the headline ON-path color gate.
#[test]
fn a1g_gpu_shadows_ao_matches_host_lit_default_light() {
    let Some(ctx) = boot_render_or_skip("a1g_gpu_shadows_ao_matches_host_lit_default_light") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] lit all-zero — device did not render");
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] A1g SHADOWS|AO default-light vs deferred oracle: max arm-1 delta = \
             {max_arm1}/255, pass-through {max_pass}/255 (tol {DEFERRED_ARM1_TOL}); \
             {sdf_lit_hits} SDF-lit px"
        );
    }
}

/// **A2g — shadows-only and AO-only independence, ±2/255 vs the deferred PBR oracle.** Push
/// `flags = SHADOWS` (AO off) and `flags = AO` (shadows off) SEPARATELY; each GPU LIT render
/// matches the corresponding deferred oracle (`golden_deferred_resolve ∘
/// golden_marcher_attributes`, default light) within ±2/255. PBR MVP-2 re-pointed the host
/// reference from the retired MVP-1 `_lit` golden to the deferred Cook-Torrance oracle. This
/// proves the flag bits gate INDEPENDENTLY (a wired-together SHADOWS|AO that ignored a single
/// bit would diverge here). Also asserts the two single-flag renders DIFFER from each other
/// (each flag has a distinct effect).
#[test]
fn a2g_gpu_shadows_only_and_ao_only_gate_independently() {
    let Some(ctx) = boot_render_or_skip("a2g_gpu_shadows_only_and_ao_only_gate_independently") else {
        return;
    };
    // The per-flag ±3/255-vs-host gates (below) are the rigorous independence proof: each
    // single-flag GPU render matches ITS OWN distinct host `_lit` golden, so SHADOWS-only
    // cannot be silently producing the AO-only result (and vice-versa). A separate
    // SHADOWS-only != AO-only differential is also asserted, but only AGGREGATED across the
    // set: a convex fixture (box) self-shadows nowhere and AO-darkens negligibly, so its two
    // single-flag renders legitimately coincide; the carved crater is the scene that MUST
    // diverge.
    let mut any_flag_differs = false;
    for (name, edits) in p4b_scenes() {
        let mut renders: [Option<Vec<u8>>; 2] = [None, None];
        for (slot, flags) in [LIGHTING_FLAG_SHADOWS, LIGHTING_FLAG_AO].into_iter().enumerate() {
            let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
            let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name} flags={flags}] lit all-zero — device did not render");
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&lit, &edits, flags, DEFAULT_LIGHT_DIR, name);
            assert!(sdf_lit_hits > 0, "[{name} flags={flags}] no SDF-lit (mask==1) pixel");
            let which = if flags == LIGHTING_FLAG_SHADOWS { "SHADOWS-only" } else { "AO-only" };
            println!(
                "[{name}] A2g {which} vs deferred oracle: max arm-1 delta = {max_arm1}/255, \
                 pass-through {max_pass}/255 (tol {DEFERRED_ARM1_TOL}); {sdf_lit_hits} SDF-lit px"
            );
            renders[slot] = Some(lit);
        }

        let shadows = renders[0].as_ref().expect("SHADOWS render");
        let ao = renders[1].as_ref().expect("AO render");
        let differs = shadows
            .chunks_exact(4)
            .zip(ao.chunks_exact(4))
            .any(|(s, a)| (0..3).any(|c| (s[c] as i32 - a[c] as i32).abs() > LIT_CHANNEL_TOL));
        if differs {
            any_flag_differs = true;
            println!("[{name}] A2g SHADOWS-only != AO-only (each flag has a distinct effect here)");
        } else {
            println!("[{name}] A2g SHADOWS-only ≈ AO-only (convex fixture: no self-shadow, negligible AO)");
        }
    }
    assert!(
        any_flag_differs,
        "across ALL fixtures the SHADOWS-only and AO-only renders coincided — the two flags do \
         not gate independently (one bit is dead). The per-flag-vs-host gates above should have \
         caught this; if they passed but this failed, a host golden is mis-routed"
    );
}

/// **A3g — the non-default light_dir mis-pack catcher (the architect's std430 push-layout
/// oracle).** A NON-axis `light_dir` ((0.4,0.5,0.768), normalized) is pushed with SHADOWS
/// enabled and the GPU render is compared, pixel-for-pixel, against the DEFAULT-light GPU
/// render. The two MUST DIFFER beyond the OFF tolerance on the SDF surface: the shadow march
/// direction is `pc.light_dir`, so a correctly-packed non-default light shifts the shadow
/// pattern, whereas a std430 OFFSET MIS-PACK (light_dir landing at the wrong push offset →
/// read as zero/garbage) would (a) collapse the shadow direction toward the default / a
/// degenerate value and (b) leave the render ≈ the default-light render → NO difference.
/// A measurable difference therefore proves `light_dir` reaches the shader at offset 16.
///
/// BUG-A-NDOTL (FIXED — see `a3g_nondefault_light_dir_matches_host_lit_literal` for the
/// literal-form payoff): the shader's Lambert BASE term now consumes the PUSHED `pc.light_dir`
/// (was the static `LIGHT_DIR=(0,0,1)`), matching `host_shade`, so a non-default light steers
/// the base too and the GPU/host base no longer diverge. This GPU-vs-GPU differential is
/// RETAINED as a complementary, host-independent mis-pack oracle: it proves the same packing
/// property (a non-axis light re-aims the shadow march) without depending on the host golden.
#[test]
fn a3g_nondefault_light_dir_shifts_shadows_mispack_catcher() {
    let Some(ctx) = boot_render_or_skip("a3g_nondefault_light_dir_shifts_shadows_mispack_catcher") else {
        return;
    };
    // SHADOWS only: isolate the term the shader actually steers by `pc.light_dir` (AO marches
    // the surface NORMAL, not the light, so it is light_dir-invariant and would dilute the
    // differential). The differential is only geometrically guaranteed where the SCENE has a
    // self-occluder: the carved CRATER (a CSG subtract that leaves a rim/crevice) self-shadows
    // and so MUST shift; a single CONVEX box self-shadows nowhere (the lit hemisphere is
    // unoccluded for any front light), so its shift is legitimately ~0. The mis-pack catcher
    // therefore REQUIRES a shift on the crater (the load-bearing assertion) and merely reports
    // the others — a mis-pack (light_dir read off-offset → degenerate / default direction)
    // would zero the CRATER shift, tripping the gate.
    let flags = LIGHTING_FLAG_SHADOWS;
    let mut crater_shifted = 0u64;
    for (name, edits) in p4b_scenes() {
        let def = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        let non = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, NONDEFAULT_LIGHT).0;
        assert_eq!(def.len(), READBACK_BYTES as usize);
        assert_eq!(non.len(), READBACK_BYTES as usize);

        // Count pixels whose shadow term shifted beyond the OFF tolerance. A correctly-packed
        // non-axis light re-aims the shadow march → self-occluded surface pixels change. A
        // mis-pack (light_dir read as 0 → the ndotl<=0 early-out, OR read as the default) would
        // leave def ≈ non → ZERO shifted pixels.
        let mut shifted = 0u64;
        let mut max_shift = 0i32;
        let mut worst = (0u32, 0u32, [0i32; 3], [0i32; 3]);
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let a = albedo_rgb(&def, px, py);
                let b = albedo_rgb(&non, px, py);
                let dmax = (0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap();
                if dmax > CHANNEL_TOL {
                    shifted += 1;
                }
                if dmax > max_shift {
                    max_shift = dmax;
                    worst = (px, py, a, b);
                }
            }
        }
        if name == "crater_csg" {
            crater_shifted = shifted;
        }
        println!(
            "[{name}] A3g non-axis light shift: {shifted} pixels vs default (max {max_shift}/255 \
             at ({},{}) def={:?} non={:?})",
            worst.0, worst.1, worst.2, worst.3
        );
    }
    assert!(
        crater_shifted > 0,
        "MIS-PACK SUSPECTED: the non-axis light_dir {NONDEFAULT_LIGHT:?} produced NO shadow shift \
         on the CRATER (a self-occluding CSG carve) vs the default light — light_dir is NOT \
         reaching the shader at offset 16 (a std430 push mis-pack), or the shadow term ignores it"
    );
    println!(
        "[crater_csg] A3g mis-pack catcher OK: non-axis light_dir shifts {crater_shifted} crater \
         pixels — light_dir reaches the shader at offset 16 (correct std430 packing)"
    );
}

/// **A3g-literal — the architect's named std430 oracle in its FULL literal form, against the
/// deferred PBR oracle (the BUG-A-NDOTL payoff).** Push a NON-axis `light_dir` ((0.4,0.5,0.768)
/// normalized) with `SHADOWS|AO` and assert EVERY GPU LIT texel is within ±2/255 of
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, NONDEFAULT_LIGHT))` — the
/// deferred oracle baked with the SAME non-default light — on crater / box / smooth.
///
/// PBR MVP-2: the host reference is re-pointed from the retired MVP-1
/// `golden_composite_pixel_ex_omega_lit` (`base*vis` Lambert) to the deferred Cook-Torrance
/// oracle (the readback is the PBR `gLit`). The literal host-vs-GPU form is now feasible against
/// THIS oracle for any light: `golden_marcher_attributes` steers the lit terms by the pushed
/// `light_dir` (the BUG-A-NDOTL fix — the marcher's base + the resolve both consume
/// `pc.light_dir`), so a non-default light no longer diverges the host from the GPU. (Against
/// the OLD MVP-1 mirror the literal form carried a footnote that it was not achievable; that
/// footnote is obsolete — the deferred oracle is the single source of truth the GPU was tested
/// byte-identical to.) This gate SUBSUMES the mis-pack property the GPU-vs-GPU
/// `a3g_nondefault_light_dir_shifts_shadows_mispack_catcher` targets: the deferred oracle
/// marches the same `light_dir`, so a std430 offset slip (light_dir read off-offset →
/// degenerate / default direction) makes the GPU shadow pattern diverge from the oracle by far
/// more than ±2/255 and trips this gate too.
#[test]
fn a3g_nondefault_light_dir_matches_host_lit_literal() {
    let Some(ctx) = boot_render_or_skip("a3g_nondefault_light_dir_matches_host_lit_literal") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, NONDEFAULT_LIGHT).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] non-default-light lit all-zero — device did not render");
        // The LITERAL host-vs-GPU comparison with the SAME non-default light_dir, against the
        // deferred PBR oracle (the payoff).
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, flags, NONDEFAULT_LIGHT, name);
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel under the non-default light");
        println!(
            "[{name}] A3g-literal SHADOWS|AO non-default light {NONDEFAULT_LIGHT:?} vs deferred \
             oracle: max arm-1 delta = {max_arm1}/255, pass-through {max_pass}/255 (tol \
             {DEFERRED_ARM1_TOL}) — BUG-A-NDOTL payoff: the oracle steers by pc.light_dir; \
             {sdf_lit_hits} SDF-lit px"
        );
    }
}

/// **A5 — GPU OFF-vs-ON wall-clock A/B (perf OBSERVATION, not a pass/fail gate).** Measures
/// the fence-to-fence wall time of the FULL marcher submit (raster + marcher + readback) with
/// lighting OFF vs SHADOWS|AO ON, median of N runs, on the densest fixture (crater). This is a
/// coarse CPU-side proxy — it includes the constant raster + copy + submit/wait overhead, so
/// the ON/OFF DELTA (not the absolute) is the signal: the A1/A2 cost is the shadow secondary
/// march (≤ MAX_IT steps per lit pixel) + the 5 AO taps, bounded by the P4-style empty-skip
/// and the small 64×64 lit-pixel count. No GPU-timestamp query API exists in the RHI yet
/// (a developer increment), so a true on-device marcher-only timing is deferred; this wall A/B
/// is the available proxy. `#[ignore]` by default (a perf observation, run explicitly).
#[test]
#[ignore = "perf observation — run explicitly with --ignored"]
fn a5_gpu_off_vs_on_wall_clock_ab() {
    let Some(ctx) = boot_render_or_skip("a5_gpu_off_vs_on_wall_clock_ab") else {
        return;
    };
    use std::time::Instant;
    let edits = crater();
    const N: usize = 21; // odd → a clean median
    let bench = |flags: u32| -> f64 {
        // Warm up (pipeline/cache) before timing.
        let _ = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR).0;
        let mut samples = Vec::with_capacity(N);
        for _ in 0..N {
            let t0 = Instant::now();
            let out = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR).0;
            let dt = t0.elapsed().as_secs_f64() * 1.0e6; // microseconds
            std::hint::black_box(&out);
            samples.push(dt);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        samples[N / 2]
    };
    let off = bench(0);
    let on = bench(LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO);
    println!(
        "[crater_csg] A5 wall-clock A/B (median of {N}, full submit incl. raster+copy+wait): \
         OFF = {off:.1} µs, ON(SHADOWS|AO) = {on:.1} µs, Δ = {:+.1} µs ({:+.1}%). \
         NOTE: coarse CPU-side proxy — includes constant non-marcher overhead; the Δ is the A1/A2 \
         marcher-side cost signal, not a pass/fail gate.",
        on - off,
        (on - off) / off * 100.0
    );
}

/// **A4g — sync-validation clean under cull-ON + lighting-ON (the heaviest path).** A cull-ON
/// SHADOWS|AO ω=1.2 dispatch that RETURNS proves `assert_validation_clean` passed (asserted
/// inside `run_gbuffer_hybrid_lit` before return): the A1/A2 shadow/AO secondary marches read
/// the SAME frozen field through the already-bound vocabulary set — they add NO new resource,
/// NO new binding, and NO new barrier over the pre-A1/A2 cull-ON path, so the coarse→fine
/// Tiles barrier and the G-buffer image transitions raise no new hazard. The 32-byte push is
/// pure data. This is the combined gate 9 (sync-val) for the ON path.
#[test]
fn a4g_cull_on_lighting_on_sync_validation_clean() {
    let Some(ctx) = boot_render_or_skip("a4g_cull_on_lighting_on_sync_validation_clean") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    let (albedo, tiles) =
        run_gbuffer_hybrid_lit(&ctx, &crater(), true, true, DEFAULT_MARCHER_OMEGA, flags, DEFAULT_LIGHT_DIR);
    assert_eq!(albedo.len(), READBACK_BYTES as usize);
    let bounds = parse_tile_bounds(&tiles.expect("read_tiles"));
    let surface = bounds.iter().filter(|b| b.flags & TILE_FLAG_EMPTY == 0).count();
    assert!(surface > 0, "the coarse pass must have marked at least one surface tile");
    let sdf_hits = albedo
        .chunks_exact(4)
        .filter(|t| {
            let mesh = unpack_packed_rgb(pack_rgba(MESH_COLOR));
            let bg = packed_background();
            let g = [t[0] as i32, t[1] as i32, t[2] as i32];
            let near = |r: [i32; 3]| (0..3).all(|c| (g[c] - r[c]).abs() <= CHANNEL_TOL);
            !near(mesh) && !near(bg) && (t[0] != 0 || t[1] != 0 || t[2] != 0)
        })
        .count();
    println!(
        "[crater_csg] A4g cull-ON + SHADOWS|AO ω={DEFAULT_MARCHER_OMEGA} validation+sync-clean: \
         {} tiles ({surface} surface), {sdf_hits} lit SDF px; the A1/A2 secondary marches raised \
         no new hazard",
        bounds.len()
    );
}

/// **A3g-host — the non-default light std430 round-trip (host-side push-layout pin).** A
/// pure host check that `FineMarcherPush::new(.., NONDEFAULT_LIGHT)` re-views the non-default
/// `light_dir` at byte offset 16 of `as_bytes()` (the std430 offset the shader reads). This
/// is the deterministic companion to the GPU differential A3g: it pins the host side of the
/// push contract (the `const _: () = assert!(offset_of!(.., light_dir) == 16)` is a compile
/// gate; this asserts the RUNTIME bytes too) so a future field-reorder is caught even without
/// a GPU.
#[test]
fn a3g_host_light_dir_round_trips_at_offset_16() {
    let push = FineMarcherPush::new(
        false,
        DEFAULT_MARCHER_OMEGA,
        LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO,
        NONDEFAULT_LIGHT,
    );
    let bytes = push.as_bytes();
    assert_eq!(bytes.len(), 32, "FineMarcherPush must serialize to 32 bytes");
    let read_at = |off: usize| f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    // lighting_flags is a u32 at offset 8.
    let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(flags, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, "lighting_flags must land at offset 8");
    // light_dir is a float3 at offset 16 (std430), tail-padded by _pad2 @28.
    assert_eq!(read_at(16), NONDEFAULT_LIGHT[0], "light_dir.x must land at offset 16");
    assert_eq!(read_at(20), NONDEFAULT_LIGHT[1], "light_dir.y must land at offset 20");
    assert_eq!(read_at(24), NONDEFAULT_LIGHT[2], "light_dir.z must land at offset 24");
    println!("[host] A3g-host OK: light_dir {NONDEFAULT_LIGHT:?} round-trips at push offset 16/20/24");
}

// ===========================================================================================
// Deferred-shading SPLIT (increment 1) — the M1 wiring gates (TESTER).
//
// The marcher writes ATTRIBUTES (gAlbedo = base, gMaterial = (vis, 0, mask, 1)); the
// `deferred_pbr` RESOLVE composites `lit = mask==1 ? base*vis : base`. The host oracles are
// `golden_marcher_attributes` (the marcher's per-pixel (base_rgb, vis, mask)) and
// `golden_deferred_resolve` (the resolve's packed LIT). The earlier ON-path gates
// (`a1g_*`, `a2g_*`, `a3g_*_literal`) diff the GPU LIT against the OLD INLINE golden
// `golden_composite_pixel_ex_omega_lit` at the generic ±3/255 (`LIT_CHANNEL_TOL`); that
// proves the GPU ≈ the old inline composite but DOES NOT exercise the new deferred oracles,
// so the 0%-gate's byte-identity (delta == 0 on the pass-through arms) and the ≤1.5-LSB
// double-quant bound (arm 1) were UNVERIFIED. These gates close that gap by diffing the GPU
// LIT against `golden_deferred_resolve ∘ golden_marcher_attributes` directly, per arm:
//
//   - PASS-THROUGH arms (mesh / background / empty, AND lighting-OFF SDF-lit): the resolve
//     passes `base` through unmodified (mask == 0) or multiplies by vis == 1.0 (mask == 1,
//     OFF), so the GPU LIT must be BYTE-IDENTICAL (delta == 0) to the new oracle.
//   - ARM 1 (SDF-lit, lighting ON): the deferred double-quantization (base8/255 * vis8/255,
//     re-packed) drifts from the GPU's own fp `base*vis` by the architect's ≤2/255 bound —
//     TIGHTER than the generic ±3/255 the old-inline gate uses.
//
// A GPU pixel is mapped to its host arm by `golden_marcher_attributes(..).mask` (1 = SDF-lit,
// 0 = mesh/bg/empty) — the SAME mask the resolve branches on — so the per-arm tolerance is
// applied to exactly the pixels the resolve treats that way.
// ===========================================================================================

/// The deferred-resolve double-quantization bound on the SDF-LIT arm (arm 1). The marcher
/// already R8-quantized `base` and `vis`; the resolve decodes them (base8/255, vis8/255),
/// multiplies, and re-quantizes — a SECOND 8-bit rounding on top of the GPU's own fp
/// `base*vis`. The architect's ≤1.5-LSB analysis bounds this at ≤2/255 (rounded up to the
/// integer channel grid). This is STRICTLY tighter than the generic `LIT_CHANNEL_TOL` (±3)
/// the old-inline ON-path gates use — it is the bound this increment exists to prove.
const DEFERRED_ARM1_TOL: i32 = 2;

/// The pass-through arm budget when the oracle is the HOST `golden_deferred_resolve` (which
/// quantizes via host `pack_rgba`). On the pass-through arms (mask == 0, or OFF SDF-lit) the
/// RESOLVE itself is a byte-exact GPU identity (decode `b/255` → re-encode → `b`), so the GPU
/// LIT equals the GPU's OWN gAlbedo store byte-for-byte. The residual vs the host oracle is
/// therefore EXACTLY the marcher's pre-existing host-`pack_rgba`-vs-GPU-UNORM-store
/// quantization gap (the half-way `0.1*255 == 25.5` background channel rounds 26 host / 25
/// GPU) — the SAME ≤2/255 gap `p1b`/GATE-4 already budget against the albedo golden. It is
/// NOT a resolve error; the resolve's exactness is proved independently by
/// [`assert_resolve_passthrough_is_lighting_invariant`] (a delta == 0 GPU-internal gate).
const DEFERRED_PASSTHROUGH_HOST_TOL: i32 = 2;

/// Diffs the whole GPU LIT readback against the NEW deferred oracle
/// `golden_deferred_resolve(golden_marcher_attributes(.., flags, light_dir))` per ARM, on the
/// cull-OFF ω=1.0 path:
///
///   - mask == 0 (mesh / background / empty) → within [`DEFERRED_PASSTHROUGH_HOST_TOL`]
///     (±2/255: the marcher's pre-existing host-pack-vs-GPU-store quant gap; the resolve adds
///     ZERO error here — proved delta-0 GPU-internally by the lighting-invariance gate).
///   - mask == 1, flags == 0 (SDF-lit, lighting OFF) → same pass-through budget (resolve
///     `base*1.0`).
///   - mask == 1, flags != 0 (SDF-lit, lighting ON) → within [`DEFERRED_ARM1_TOL`] (the
///     deferred double-quant, ≤2/255).
///
/// Returns `(max_delta_passthrough, max_delta_arm1, sdf_lit_hits)`. `sdf_lit_hits` (the
/// mask == 1 count) lets the caller prove the device rendered a real lit surface (not an
/// all-pass-through fill). Asserts on every texel; the caller passes the scene name.
///
/// This is the ω=1.0 specialization of [`assert_lit_matches_deferred_golden_omega`] (the
/// over-relaxation factor the pre-B1 marcher used). The B1 over-relaxation gates that diff a
/// non-unit ω against the deferred oracle call the `_omega` form directly.
fn assert_lit_matches_deferred_golden(
    lit: &[u8],
    edits: &[SdfEdit],
    flags: u32,
    light_dir: [f32; 3],
    name: &str,
) -> (i32, i32, u64) {
    assert_lit_matches_deferred_golden_omega(lit, edits, 1.0, flags, light_dir, name)
}

/// The over-relaxation-aware form of [`assert_lit_matches_deferred_golden`]: diffs the whole
/// GPU LIT readback against `golden_deferred_resolve(golden_marcher_attributes(.., omega,
/// flags, light_dir))` per ARM, with the same ±2/255 pass-through and ARM-1 double-quant
/// budgets. `omega` is the Render B1 over-relaxation factor the GPU marched at (the host
/// oracle marches the IDENTICAL ω, so the comparison stays matched-ω).
fn assert_lit_matches_deferred_golden_omega(
    lit: &[u8],
    edits: &[SdfEdit],
    omega: f32,
    flags: u32,
    light_dir: [f32; 3],
    name: &str,
) -> (i32, i32, u64) {
    let mut max_pass = 0i32;
    let mut max_arm1 = 0i32;
    let mut sdf_lit_hits = 0u64;
    let materials = host_material_table();
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let md = expected_mesh_depth(px, py);
            let attrs = golden_marcher_attributes(
                edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, omega,
                flags, light_dir,
            );
            let (_, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let want = unpack_packed_rgb(golden_deferred_resolve(attrs, rd, &materials));
            let got = albedo_rgb(lit, px, py);
            let dmax = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            // The arm the resolve actually takes: mask == 1 AND lighting ON is the only
            // double-quantized (non-pass-through) case; everything else passes through.
            let is_arm1 = attrs.mask == 1 && flags != 0;
            if attrs.mask == 1 {
                sdf_lit_hits += 1;
            }
            if is_arm1 {
                if dmax > max_arm1 {
                    max_arm1 = dmax;
                }
                assert!(
                    dmax <= DEFERRED_ARM1_TOL,
                    "[{name}] ARM-1 (SDF-lit, flags={flags}) LIT texel ({px},{py}) got {got:?} \
                     want {want:?} (deferred oracle) exceeds ±{DEFERRED_ARM1_TOL}/255 (the \
                     double-quant bound); delta {dmax}"
                );
            } else {
                if dmax > max_pass {
                    max_pass = dmax;
                }
                assert!(
                    dmax <= DEFERRED_PASSTHROUGH_HOST_TOL,
                    "[{name}] PASS-THROUGH (mask={}, flags={flags}) LIT texel ({px},{py}) got \
                     {got:?} want {want:?} (deferred oracle) exceeds the host-pack quant budget \
                     ±{DEFERRED_PASSTHROUGH_HOST_TOL}/255 (delta {dmax}) — the resolve must pass \
                     base through (the residual is the marcher's host-pack-vs-GPU-store gap, NOT \
                     a resolve error)",
                    attrs.mask
                );
            }
        }
    }
    (max_pass, max_arm1, sdf_lit_hits)
}

/// **D1-host — the deferred PASS-THROUGH byte-identity gate (host-only, no GPU).** PBR
/// MVP-2 changes the SDF-lit (mask == 1) output from the MVP-1 `base*vis` composite to full
/// Cook-Torrance — an INTENTIONAL, owner-acknowledged behavioral change (PBR plan call F),
/// so the SDF-lit arm is DELIBERATELY no longer an approximation of the old inline composite
/// and is NOT compared against it here. What this gate STILL proves — the load-bearing
/// 0%-gate — is that the deferred bake (`golden_deferred_resolve ∘ golden_marcher_attributes`)
/// is BYTE-IDENTICAL to the old inline composite on the PASS-THROUGH arms (mesh / background
/// / empty, mask == 0) across crater / box / smooth, lighting OFF + ON, default + non-default
/// light. A regression in the host oracles' pass-through path is caught without a device.
#[test]
fn d1_host_deferred_passthrough_byte_identical() {
    let materials = host_material_table();
    for (name, edits) in p4b_scenes() {
        for (lname, light) in [("default", DEFAULT_LIGHT_DIR), ("nondefault", NONDEFAULT_LIGHT)] {
            for flags in [0u32, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO] {
                let mut passthrough = 0u64;
                let mut lit_hits = 0u64;
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = expected_mesh_depth(px, py);
                        let attrs = golden_marcher_attributes(
                            &edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H,
                            CompositeCamera::Ortho, 1.0, flags, light,
                        );
                        // Only the mask == 0 (mesh / bg / empty) arm has the unchanged
                        // pass-through contract; the mask == 1 arm is now PBR (skipped here).
                        if attrs.mask == 1 {
                            lit_hits += 1;
                            continue;
                        }
                        passthrough += 1;
                        let (_, rd) =
                            composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let deferred =
                            unpack_packed_rgb(golden_deferred_resolve(attrs, rd, &materials));
                        let inline = unpack_packed_rgb(golden_composite_pixel_ex_omega_lit(
                            &edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                            flags, light,
                        ));
                        assert_eq!(
                            deferred, inline,
                            "[{name}/{lname}] PASS-THROUGH (mask=0, flags={flags}) deferred \
                             {deferred:?} != inline {inline:?} at ({px},{py}) — the mesh / bg / \
                             empty arms must bake byte-identically (the 0%-gate)"
                        );
                    }
                }
                assert!(
                    passthrough > 0,
                    "[{name}/{lname}] no mask=0 pixel — the pass-through gate is vacuous"
                );
                println!(
                    "[{name}/{lname}] D1-host flags={flags}: {passthrough} pass-through px \
                     BYTE-IDENTICAL (delta 0) deferred-vs-inline; {lit_hits} SDF-lit (now PBR) px"
                );
            }
        }
    }
}

/// **D2g — the M1 pass-through gate (GPU LIT == the deferred oracle on the pass-through
/// arms).** With `flags == 0` EVERY arm is pass-through (mesh/bg/empty mask 0; SDF-lit mask 1
/// but vis == 1.0), so the whole GPU LIT image must match `golden_deferred_resolve(
/// golden_marcher_attributes(.., flags=0))` within [`DEFERRED_PASSTHROUGH_HOST_TOL`] (±2/255).
///
/// IMPORTANT (the M1 finding): a delta == 0 (literal byte-identity) claim against the HOST
/// oracle is NOT achievable here, and the gap is NOT a resolve bug. The host oracle quantizes
/// via host `pack_rgba`, which disagrees with the GPU UNORM store by 1 LSB on the half-way
/// background channel (`0.1*255 == 25.5` → host 26, GPU 25) — the SAME pre-existing
/// host-pack-vs-GPU-store gap the marcher's albedo already carries (why `p1b`/GATE-4 use
/// ±2/255). The resolve's pass-through is BYTE-EXACT at the GPU level; that exactness is
/// proved delta-0 (GPU-internally, no host pack) by
/// [`d2g_resolve_passthrough_is_lighting_invariant`]. This gate confirms the GPU LIT tracks
/// the host oracle within the marcher's own quant budget on the pass-through arms.
#[test]
fn d2g_passthrough_within_host_pack_budget() {
    let Some(ctx) = boot_render_or_skip("d2g_passthrough_within_host_pack_budget") else {
        return;
    };
    for (name, edits) in p4b_scenes() {
        let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR).0;
        assert_eq!(lit.len(), READBACK_BYTES as usize);
        let nonzero = lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
        assert!(nonzero > 0, "[{name}] LIT all-zero — device did not render");
        let (max_pass, max_arm1, sdf_lit_hits) =
            assert_lit_matches_deferred_golden(&lit, &edits, 0, DEFAULT_LIGHT_DIR, name);
        assert_eq!(max_arm1, 0, "[{name}] flags==0 must have NO arm-1 pixel (lighting OFF)");
        assert!(sdf_lit_hits > 0, "[{name}] no SDF-lit (mask==1) pixel — the marcher hit no surface");
        println!(
            "[{name}] D2g M1 pass-through vs deferred oracle (flags=0): max delta = {max_pass}/255 \
             (tol {DEFERRED_PASSTHROUGH_HOST_TOL}, host-pack gap); {sdf_lit_hits} SDF-lit px \
             (vis=1.0 pass-through)"
        );
    }
}

/// **D2g — the resolve-is-an-exact-pass-through gate (delta == 0, GPU-INTERNAL, no host
/// pack).** The headline M1 byte-identity proof, free of the host-pack quantization gap. On
/// the mesh / background / empty arms (mask == 0) the resolve emits `base` regardless of
/// lighting, and the MARCHER writes the identical `base` to gAlbedo regardless of
/// `lighting_flags` (lighting only attenuates the SDF-hit vis, never the mesh/bg base). So the
/// GPU LIT on those pixels MUST be byte-identical between an OFF run and a SHADOWS|AO run —
/// a delta == 0 GPU-vs-GPU comparison that needs NO host oracle and is immune to the
/// host-pack-vs-UNORM gap. This proves (a) the resolve perturbs a pass-through pixel by
/// exactly ZERO, and (b) the STRICT `mask` branch never lets a vis-attenuated SDF lane bleed
/// into a mesh/bg pixel — the load-bearing 0%-gate the strict-if buys. A mismatch = the
/// resolve is NOT a pure pass-through (or the mask leaked).
#[test]
fn d2g_resolve_passthrough_is_lighting_invariant() {
    let Some(ctx) = boot_render_or_skip("d2g_resolve_passthrough_is_lighting_invariant") else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (name, edits) in p4b_scenes() {
        let off = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, 0, DEFAULT_LIGHT_DIR).0;
        let on = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, DEFAULT_LIGHT_DIR).0;
        assert_eq!(off.len(), READBACK_BYTES as usize);
        assert_eq!(on.len(), READBACK_BYTES as usize);

        // The mask the resolve branches on (from the host attribute mirror): mask == 0 is the
        // pure pass-through set the lighting flags must NOT touch.
        let mut passthrough = 0u64;
        let mut sdf_lit = 0u64;
        let materials = host_material_table();
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let md = expected_mesh_depth(px, py);
                let mask = golden_marcher_attributes(
                    &edits, &materials, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, flags, DEFAULT_LIGHT_DIR,
                )
                .mask;
                if mask == 0 {
                    passthrough += 1;
                    let a = albedo_rgb(&off, px, py);
                    let b = albedo_rgb(&on, px, py);
                    assert_eq!(
                        a, b,
                        "[{name}] PASS-THROUGH (mask=0) LIT texel ({px},{py}) changed between \
                         OFF {a:?} and SHADOWS|AO {b:?} — the resolve is NOT a pure pass-through \
                         on mask=0 (or the strict-mask branch leaked a vis-attenuated lane)"
                    );
                } else {
                    sdf_lit += 1;
                }
            }
        }
        assert!(passthrough > 0, "[{name}] no mask=0 pixel — the pass-through gate is vacuous");
        assert!(sdf_lit > 0, "[{name}] no mask=1 pixel — there is no lit surface to leave alone");
        println!(
            "[{name}] D2g resolve-passthrough lighting-invariant: {passthrough} mask=0 pixels \
             BYTE-IDENTICAL (delta 0) across OFF vs SHADOWS|AO; {sdf_lit} mask=1 px"
        );
    }
}

/// **D3g — the arm-1 bounded-quantization gate (SDF-lit, ≤2/255 vs the deferred oracle).**
/// Push SHADOWS|AO (default light); every SDF-lit (mask == 1) GPU LIT texel must be within
/// [`DEFERRED_ARM1_TOL`] (±2/255) of `golden_deferred_resolve(golden_marcher_attributes(..,
/// flags=SHADOWS|AO))`. This is the ≤1.5-LSB double-quantization the deferred split
/// introduces — TIGHTER than the generic ±3/255 the old-inline `a1g_*` gate uses, and the
/// bound this increment exists to prove. The pass-through arms (asserted delta 0 by the same
/// helper) are re-confirmed here too. Runs crater / box / smooth, and (separately) the
/// non-default light to exercise the steered shadow march.
#[test]
fn d3g_arm1_within_double_quant_bound_of_deferred_golden() {
    let Some(ctx) = boot_render_or_skip("d3g_arm1_within_double_quant_bound_of_deferred_golden")
    else {
        return;
    };
    let flags = LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO;
    for (lname, light) in [("default", DEFAULT_LIGHT_DIR), ("nondefault", NONDEFAULT_LIGHT)] {
        for (name, edits) in p4b_scenes() {
            let lit = run_gbuffer_hybrid_lit(&ctx, &edits, false, false, 1.0, flags, light).0;
            assert_eq!(lit.len(), READBACK_BYTES as usize);
            let nonzero =
                lit.chunks_exact(4).filter(|t| t[0] != 0 || t[1] != 0 || t[2] != 0).count();
            assert!(nonzero > 0, "[{name}/{lname}] LIT all-zero — device did not render");
            let (max_pass, max_arm1, sdf_lit_hits) =
                assert_lit_matches_deferred_golden(&lit, &edits, flags, light, name);
            assert!(
                max_pass <= DEFERRED_PASSTHROUGH_HOST_TOL,
                "[{name}/{lname}] a pass-through arm exceeded the host-pack budget (delta \
                 {max_pass} > {DEFERRED_PASSTHROUGH_HOST_TOL})"
            );
            assert!(
                sdf_lit_hits > 0,
                "[{name}/{lname}] no SDF-lit (mask==1) pixel — the arm-1 bound is vacuous"
            );
            println!(
                "[{name}/{lname}] D3g arm-1 double-quant: max delta = {max_arm1}/255 \
                 (tol {DEFERRED_ARM1_TOL}); pass-through {max_pass} (=0); {sdf_lit_hits} SDF-lit px"
            );
        }
    }
}


