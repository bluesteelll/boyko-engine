//! The render-input data bundles + push-constant PODs: [`Scene`],
//! [`SampledComposite`], [`UiPass`], [`GBufferScene`] (+ its activation sub-bundles
//! `BrickActivation`, `SsaoActivation`, `CsmDepthActivation`, `PunctualDepthActivation`,
//! `GBufferMeshDraw`), and the associated size/offset constants. Split out of the
//! former monolithic `swapchain.rs` (audit W4).

use boyko_rhi::{Format, ImageUsage, RhiDevice, TextureDesc, TextureDimension};

#[cfg(feature = "hwrt")]
use crate::accel::BoundAccelStruct;
use crate::compute::CoarseMode;
use crate::device::VulkanContext;
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline, VulkanSampler,
};
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS, VulkanTexture};

use super::gpu_timing::TimestampCollector;
use super::{FRAMES_IN_FLIGHT, SwapchainError};

// Doc-link scope: types referenced only from doc-comments in this module (the render
// bundles document how the `Renderer` frame methods + `GBufferTargets` consume them).
#[allow(unused_imports)]
use super::frame_driver::Renderer;
#[allow(unused_imports)]
use super::targets::GBufferTargets;
#[allow(unused_imports)]
use crate::compute::FineMarcherPush;

/// The MVP push-constant size (a `float4x4`), matching the committed rung-3/4 MVP
/// vertex shader's `VERTEX`-stage push range.
pub const SCENE_MVP_BYTES: usize = 64;

/// A device-local depth image (`D32_SFLOAT` + DEPTH-aspect view) sized to one
/// swapchain extent. Recreated when the extent changes (resize); the wrapping
/// [`VulkanTexture`] owns the image/view/memory and is torn down through the
/// originating [`VulkanContext`].
pub(crate) struct DepthImage {
    /// The owned `VkImage` + DEPTH view + dedicated allocation.
    pub(crate) texture: VulkanTexture,
    /// The extent the depth image was created at (so [`Scene::sync_depth`] can detect
    /// a resize and recreate it).
    pub(crate) extent: VkExtent2D,
}

/// The rung-7 on-screen scene resources: the depth-tested graphics pipeline, the
/// hardcoded mesh's vertex buffer, the MVP push constant, and the per-extent depth
/// image — everything [`Renderer::render_scene_frame`] needs beyond the swapchain.
///
/// The pipeline + vertex buffer are created by the caller through the
/// [`RhiDevice`] trait (so the proven S0 pipeline-creation
/// path is reused, not duplicated) and moved into the `Scene`; the depth image is
/// created + resized internally via the same device's `create_texture` path. The
/// `Scene` is **not** `Copy`/`Clone`: it is torn down by value through
/// [`Scene::destroy`] (the move encodes "destroyed exactly once").
///
/// The pipeline's declared color format MUST equal the swapchain's color format and
/// its declared depth format MUST equal [`Format::D32Sfloat`](boyko_rhi::Format) —
/// the W2-b format-matching contract — or the validation layer faults at draw time.
///
/// # Safety
///
/// The originating [`VulkanContext`] MUST still be alive whenever the scene is
/// rendered or destroyed: each of its owned handles is torn down through that
/// context's device fn-table. There is no compile-time `'ctx` tie this phase (plan
/// F1; mirrors the other S0 graphics resources).
pub struct Scene {
    /// The depth-tested graphics pipeline (raw `VkPipeline` + `VkPipelineLayout`),
    /// created via `RhiDevice::create_graphics_pipeline` and owned here.
    pub(crate) pipeline: VulkanGraphicsPipeline,
    /// The hardcoded mesh's host-visible vertex buffer (position + color), created
    /// via `RhiDevice::create_buffer` and owned here.
    pub(crate) vertex_buffer: BoundBuffer,
    /// The number of vertices to `draw` (the hardcoded mesh's vertex count).
    pub(crate) vertex_count: u32,
    /// The MVP `float4x4` pushed to the pipeline's VERTEX range each frame.
    pub(crate) mvp: [u8; SCENE_MVP_BYTES],
    /// The per-extent depth image, created lazily on the first frame and recreated
    /// on resize ([`Scene::sync_depth`]).
    pub(crate) depth: Option<DepthImage>,
}

impl Scene {
    /// Bundles a caller-created depth-tested graphics pipeline + vertex buffer + MVP
    /// into a renderable scene. The depth image is created lazily on the first frame
    /// (sized to the swapchain extent then), so no extent is needed here.
    ///
    /// `pipeline` MUST declare the swapchain's color format as its single color
    /// attachment and [`Format::D32Sfloat`](boyko_rhi::Format) as its depth format
    /// (W2-b); `vertex_buffer` holds `vertex_count` vertices in the pipeline's
    /// declared vertex layout; `mvp` is the 64-byte `float4x4` push constant.
    #[inline]
    pub fn new(
        pipeline: VulkanGraphicsPipeline,
        vertex_buffer: BoundBuffer,
        vertex_count: u32,
        mvp: [u8; SCENE_MVP_BYTES],
    ) -> Self {
        Self {
            pipeline,
            vertex_buffer,
            vertex_count,
            mvp,
            depth: None,
        }
    }

    /// Overwrites the per-frame MVP push-constant bytes (a column-major 4x4 `f32`
    /// matrix the vertex shader reads at VERTEX-stage offset 0). The next
    /// [`Renderer::render_scene_frame`] (its `record_scene` re-pushes `mvp`
    /// unconditionally each frame — swapchain.rs:1356) — and likewise the raster
    /// [`Renderer::render_gbuffer_frame`] (`record_gbuffer` re-pushes it at
    /// swapchain.rs:2542) — picks up these bytes with NO pipeline/scene rebuild.
    /// This is the live render-view seam a windowed loop uses to drive the
    /// on-screen view each frame from a per-frame `ViewUniform.view_proj`.
    #[inline]
    pub fn set_mvp(&mut self, mvp: [u8; SCENE_MVP_BYTES]) {
        self.mvp = mvp;
    }

    /// Ensures the depth image exists and matches `extent`, (re)creating it through
    /// `ctx` when it is absent (first frame) or stale (resize). The caller
    /// ([`Renderer::render_scene_frame`]) calls this only after fence-waiting the
    /// frame slot, so no in-flight frame still references an old depth image.
    pub(crate) fn sync_depth(&mut self, ctx: &VulkanContext, extent: VkExtent2D) -> Result<(), SwapchainError> {
        if let Some(d) = &self.depth
            && d.extent.width == extent.width
            && d.extent.height == extent.height
        {
            return Ok(());
        }

        // A (re)create is rare (first frame + resize). When REPLACING an existing
        // depth image, wait the device idle first: with multiple frames in flight a
        // sibling slot may still reference the old depth image, and the caller only
        // fence-waited THIS slot. The idle guarantees no submission references the old
        // image before it is freed (the same belt-and-braces the swapchain `recreate`
        // uses). The first-ever create (no old image) needs no idle.
        if self.depth.is_some() {
            // SAFETY: `ctx` is live; waiting idle guarantees every prior submission —
            // including any sibling-slot frame still referencing the old depth image —
            // has completed before it is destroyed below.
            unsafe { (ctx.device_fns().device_wait_idle)(ctx.device()) };
        }

        // Build the new depth image BEFORE tearing down the old one so an allocation
        // failure leaves the previous (still-valid) depth image in place.
        let desc = TextureDesc {
            width: extent.width,
            height: extent.height,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
            array_layers: 1,
        };
        let texture = RhiDevice::create_texture(ctx, &desc).map_err(SwapchainError::DepthImage)?;

        // Destroy the previous depth image (the device was waited idle above, so no
        // submission references it).
        if let Some(old) = self.depth.take() {
            // SAFETY: the old depth texture was created on `ctx` by a prior
            // `sync_depth`; the device was waited idle above so its last referencing
            // frame completed; the by-value move destroys it exactly once.
            unsafe { RhiDevice::destroy_texture(ctx, old.texture) };
        }

        self.depth = Some(DepthImage { texture, extent });
        Ok(())
    }

    /// Tears down the scene's owned resources (depth image, vertex buffer, graphics
    /// pipeline) through `ctx`, consuming `self`. The caller MUST have made the
    /// device idle (e.g. dropped the [`Renderer`], whose `Drop` waits idle) so no
    /// submission still references them.
    ///
    /// # Safety
    ///
    /// `ctx` is the live context the scene's resources were created on; no GPU work
    /// referencing them is in flight (caller `wait_idle`'d / dropped the renderer);
    /// each is destroyed exactly once (the by-value `self` enforces the latter).
    pub unsafe fn destroy(mut self, ctx: &VulkanContext) {
        // SAFETY: per the contract `ctx` is live and nothing references these
        // resources; each was created on `ctx` and is destroyed exactly once, in
        // reverse acquisition order (depth → vertex buffer → pipeline).
        unsafe {
            if let Some(depth) = self.depth.take() {
                RhiDevice::destroy_texture(ctx, depth.texture);
            }
            RhiDevice::destroy_buffer(ctx, self.vertex_buffer);
            RhiDevice::destroy_graphics_pipeline(ctx, self.pipeline);
        }
    }
}

/// The rung-11 on-screen hybrid-composite present inputs: the ALREADY-UPLOADED,
/// already-resident SAMPLED texture the compute composite lives in, the sampler +
/// bind group binding it, and the fullscreen-sample graphics pipeline that samples
/// it into the swapchain image.
///
/// # The texture is resident + read-only BEFORE this bundle is built
///
/// The composite is STATIC across the whole present loop (it never changes between
/// frames), so the caller uploads the compute composite into `texture` and
/// transitions it to `SHADER_READ_ONLY_OPTIMAL` EXACTLY ONCE — in its own fenced
/// submit (or folded into the composite-producing submit) — BEFORE the present loop.
/// From then on the texture stays in `SHADER_READ_ONLY_OPTIMAL` permanently and
/// [`Renderer::present_sampled`] only ever READS it (a `FRAGMENT_SHADER` sample).
/// Multiple frames-in-flight concurrently reading a read-only texture is sound — no
/// write-after-read hazard, no per-frame upload, no per-frame barrier, and no
/// cross-frame fence on the texture. This is what makes the present loop sound across
/// `FRAMES_IN_FLIGHT` (the bundle carries no source buffer / copy extent precisely
/// because the present path performs no copy).
///
/// Unlike [`Scene`] (which OWNS its resources and is destroyed by value), this is a
/// lightweight BORROW bundle: the caller creates the resources through the
/// [`RhiDevice`] trait, owns them, and tears them down (the
/// `'a` lifetime ties the bundle to those borrows for the present call). It exists
/// only to keep [`Renderer::present_sampled`]'s signature compact.
///
/// The pipeline's declared `color_formats[0]` MUST equal the swapchain's color
/// format (the W2-b format-matching contract) and its layout MUST declare
/// `bind_group`'s set-0 layout (one COMBINED_IMAGE_SAMPLER), or the validation layer
/// faults at draw time.
///
/// # Native-size, top-left present
///
/// The composite is presented at its NATIVE size ([`texture_extent`](Self::texture_extent))
/// in the top-left of the swapchain image — never stretched to the (possibly
/// WSI-clamped) swapchain extent. See [`texture_extent`](Self::texture_extent).
pub struct SampledComposite<'a> {
    /// The SAMPLED texture the compute composite has ALREADY been uploaded into and
    /// transitioned to `SHADER_READ_ONLY_OPTIMAL` (caller's pre-loop one-time
    /// submit). The present path only samples it.
    pub texture: &'a VulkanTexture,
    /// The sampler bound alongside `texture` in `bind_group`. Not read by the present
    /// path directly; the bind group already references it. Kept here as a lifetime
    /// tie so the sampler outlives the bind group's use.
    pub sampler: &'a VulkanSampler,
    /// The bind group (one COMBINED_IMAGE_SAMPLER at set 0) binding `texture` +
    /// `sampler` for the fullscreen-sample draw.
    pub bind_group: &'a VulkanBindGroup,
    /// The fullscreen-sample graphics pipeline (no vertex buffer, no depth; its
    /// `color_formats[0]` equals the swapchain format, W2-b).
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// The `texture`'s OWN dimensions (the composite's native size), NOT the
    /// swapchain extent.
    ///
    /// [`Renderer::present_sampled`] presents the composite at its native size in
    /// the TOP-LEFT of the swapchain image: it sets the present pass's
    /// viewport/scissor to `min(swapchain_extent, texture_extent)`, so the
    /// fullscreen-sample triangle writes exactly the composite's pixels 1:1 and the
    /// rest of the (possibly wider, WSI-clamped) swapchain image stays the clear
    /// color. A 1:1 top-left mapping makes a per-texel golden exact regardless of
    /// any WSI `current_extent` clamp (e.g. a driver-minimum swapchain width wider
    /// than the texture).
    ///
    /// This denotes the TEXTURE, never the swapchain — passing the swapchain extent
    /// here would re-introduce the stretch this field exists to remove.
    pub texture_extent: VkExtent2D,
}

/// The on-screen UI rect sub-pass inputs (GUI P5a Rung 5 / Decision 9), recorded by
/// [`Renderer::present_sampled`] into the SAME swapchain `cmd` AFTER the composite
/// scope ends and BEFORE the COLOR→PRESENT barrier.
///
/// All fields are CONCRETE `boyko_rhi_vulkan` handles + POD — `boyko_rhi_vulkan` does
/// not (and must not) depend on `boyko_render`, so the caller (the render host, which
/// owns the `RhiContext`) RE-RESOLVES the current-frame UI pipeline + bind-group by
/// `frame_index` (`RhiContext::ui_handles`, MF-7) and passes them here by reference,
/// together with the instance count + the 16-byte ortho push block. The pass opens
/// its OWN `begin_rendering(LoadOp::Load)` at the FULL swapchain extent (preserving
/// the composited scene), so a rect at the bottom-right corner lands at the
/// bottom-right swapchain texel (the ortho denominator = the swapchain extent).
///
/// A pass with `instance_count == 0` records NOTHING (no empty draw, no UI scope).
pub struct UiPass<'a> {
    /// The UI graphics pipeline (vertexless quad, blend = premultiplied, its
    /// `color_formats[0]` equals the swapchain format — W2-b). Re-resolved by the
    /// caller from the current `frame_index`.
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// The current-FIF ring's bind-group (one STORAGE buffer @ set0/binding0). The
    /// backing ring holds `instance_count` valid `UiInstance` records uploaded for
    /// THIS frame index before this draw. Re-resolved by the caller.
    pub bind_group: &'a VulkanBindGroup,
    /// The number of UI instances to draw (`draw(6, instance_count, 0, 0)`); `0`
    /// records nothing.
    pub instance_count: u32,
    /// The 16-byte pixel→NDC ortho push block (`UiOrtho` byte image), pushed to the
    /// pipeline's VERTEX range. Borrowed for the record call only.
    pub ortho_bytes: &'a [u8],
}

/// The byte size of the mesh-raster pipeline's `VERTEX` push, pushed each on-screen
/// G-buffer frame. The hybrid-mesh-room PERSPECTIVE step widened it from a bare
/// `float4x4 mvp` (64 B) to `{ float4x4 mvp; float4 cam_eye }` (80 B): `cam_eye.xyz` is
/// the world eye + `cam_eye.w` the camera mode (0 ortho / 1 perspective), which
/// `gbuffer_mrt.{vs,fs}` use to write the marcher-aligned `SV_Depth` (euclidean under
/// perspective, axial under ortho). ORTHO scenes append a zeroed `cam_eye` (mode 0), so
/// their `SV_Position.z` depth — and the ortho goldens — are byte-identical.
///
/// M1 (instanced-capable raster) WIDENS it 80 -> 88 B, appending the `gbuffer_mrt.vs`
/// instanced-arm selectors: `uint base_instance` (offset 80, the SSBO bucket base) +
/// `uint use_model_matrix` (offset 84). The legacy merged draw pushes
/// `use_model_matrix == 0` + `base_instance == 0`, which makes the VS take the LEGACY arm
/// (`mul(view_proj, p)`) — BYTE-IDENTICAL pixels to the pre-M1 80-byte push (the leading
/// 64-byte `view_proj` IS the old `mvp` field, same bytes). The first 80 bytes of every
/// existing push builder are unchanged; the 8 trailing bytes are both zero.
///
/// The descriptor set (set 0 = the per-instance model SSBO) and the push range occupy
/// DISJOINT pipeline-layout slots — adding the set does NOT move the push range (still
/// offset 0, VERTEX stage).
pub const GBUFFER_PUSH_BYTES: usize = 88;

/// The byte offset of the `gbuffer_mrt.vs` push's `uint base_instance` field within the
/// 88-byte VERTEX push range (M3). The instanced batch loop re-pushes JUST this 4-byte
/// word per [`GBufferMeshDraw`] (after the once-per-pass full-`mvp` push) so each mesh's
/// draw indexes its own instance bucket (`instances[base_instance + SV_InstanceID]`).
/// Equals the `base_instance` field's offset documented on [`GBUFFER_PUSH_BYTES`] (80).
pub(crate) const GBUFFER_PUSH_BASE_INSTANCE_OFFSET: u32 = 80;

/// The byte size of ONE [`gbuffer_mrt.vs`'s `InstanceModelCol`] record (M1): a 3x4
/// ROW-MAJOR affine, 12 `f32` = 48 B (`std430` `StructuredBuffer` element, 16-B aligned).
/// The instance SSBO the gbuffer raster pipeline binds at `set 0` binding 0 holds an array
/// of these; M1 binds a 1-element dummy (the identity affine — see
/// [`GBUFFER_IDENTITY_INSTANCE`]) for every legacy draw, which the legacy arm never reads.
pub const GBUFFER_INSTANCE_MODEL_BYTES: usize = 48;

/// The IDENTITY [`gbuffer_mrt.vs`'s `InstanceModelCol`] affine (M1): a 3x4 row-major
/// `[r0, r1, r2]` with `r0 = (1,0,0,0)`, `r1 = (0,1,0,0)`, `r2 = (0,0,1,0)` — the rotation
/// is identity and the translation zero. Uploaded ONCE into the dummy 1-element instance
/// SSBO every legacy gbuffer draw binds at `set 0` binding 0. The legacy arm
/// (`use_model_matrix == 0`) NEVER reads it; it exists only to satisfy the pipeline
/// layout's static reference to the instance buffer (the MDF binding-15 bound-but-unread
/// precedent). The instanced arm (M2+) would mul by this and reproduce the legacy world
/// position exactly.
pub const GBUFFER_IDENTITY_INSTANCE: [f32; 12] = [
    1.0, 0.0, 0.0, 0.0, // r0: rotation row 0 | translation.x
    0.0, 1.0, 0.0, 0.0, // r1: rotation row 1 | translation.y
    0.0, 0.0, 1.0, 0.0, // r2: rotation row 2 | translation.z
];

/// Mesh foundation M3: ONE per-mesh INSTANCED-ARM draw in pass A's mesh-MRT G-buffer
/// producer batch list ([`GBufferScene::mesh_draw`]). An EMPTY slice keeps the LEGACY
/// merged draw (`vkCmdDraw(vertex_count, 1, 0, 0)` over [`GBufferScene::vertex_buffer`],
/// the `use_model_matrix == 0` arm) BYTE-IDENTICAL — every pre-M2 scene takes that path.
/// A NON-empty slice switches pass A to a batch loop: the shared instance SSBO
/// ([`GBufferScene::instance_bind_group`]) is bound ONCE at set 0, then each batch:
///
///   * binds [`Self::vertex_buffer`] + [`Self::index_buffer`] (the mesh's GPU buffers,
///     with [`Self::index_type`] — O3 mixed `Uint16`/`Uint32` width),
///   * pushes [`Self::base_instance`] (the M3 gather's per-mesh bucket offset — NONZERO
///     for every mesh after the first, the C1 proof) + `use_model_matrix == 1`,
///   * issues `vkCmdDrawIndexed(index_count, instance_count, 0, 0, 0)`.
///
/// The VS reads `instances[base_instance + SV_InstanceID]` from the shared SSBO, so each
/// batch draws its mesh's contiguous instance bucket. The mesh vertices are MODEL-SPACE;
/// each instance's affine places + orients them. The caller MUST have built
/// [`GBufferScene::mvp`] with `use_model_matrix == 1` (its byte 84) when the slice is
/// non-empty; the recorder OVERWRITES the push's `base_instance` word per batch.
pub struct GBufferMeshDraw<'a> {
    /// The mesh's MODEL-SPACE vertex buffer (position\@0 / normal\@12 / color\@24, the
    /// gbuffer raster pipeline's 40-byte stride). Bound at vertex binding 0 for pass A.
    pub vertex_buffer: &'a BoundBuffer,
    /// The mesh's index buffer, `index_type`-wide, bound before the indexed draw.
    pub index_buffer: &'a BoundBuffer,
    /// The number of indices to draw (`vkCmdDrawIndexed`'s `index_count`).
    pub index_count: u32,
    /// The bound index width (`VK_INDEX_TYPE_UINT16`/`UINT32` as the agnostic `i32`).
    /// O3: each batch carries its OWN width, so a `u16`-indexed mesh and a `u32`-indexed
    /// mesh draw correctly in the same pass.
    pub index_type: i32,
    /// This mesh's bucket start in the shared instance SSBO (the M3 gather's
    /// `base_instance` prefix-sum offset). Pushed into the VERTEX push constant's
    /// `base_instance` word (offset 80) per batch; the VS reads
    /// `instances[base_instance + SV_InstanceID]`. NONZERO for every batch after the
    /// first (the C1 nonzero-base proof).
    pub base_instance: u32,
    /// The number of instances of this mesh to draw (`vkCmdDrawIndexed`'s
    /// `instance_count`); the shared SSBO MUST hold at least `base_instance +
    /// instance_count` `InstanceModelCol`s.
    pub instance_count: u32,
    /// Whether this mesh CASTS shadows. The main G-buffer pass rasterizes every mesh
    /// regardless (all meshes are visible + RECEIVE shadows); the CSM cascade + punctual
    /// cube/spot DEPTH passes skip a batch with `casts_shadow == false`, so a RECEIVER-only
    /// mesh (a room floor / wall) does not stamp itself into the shadow maps and cast a
    /// spurious shadow over the scene. `true` reproduces the prior all-casters behavior.
    pub casts_shadow: bool,
}

/// The byte size of the marcher's COMPUTE push constant — DERIVED from the
/// [`FineMarcherPush`](crate::compute::FineMarcherPush) `#[repr(C)]` struct (Render A1/A2
/// widened it 8 → 32 bytes: it now carries `lighting_flags` @8 + `light_dir` @16 alongside
/// the P4b `coarse_enabled` @0 + the B1 `omega` @4). The windowed path pushes
/// `coarse_enabled = 0` (the coarse cull pass is not run on-screen), `omega =
/// DEFAULT_MARCHER_OMEGA` (the B1 over-relaxation speedup), and lighting ON with the
/// default directional light (the demo). It is a subset of the marcher pipeline's declared
/// 80-byte (`COMPOSITE_PUSH_CONSTANT_BYTES`) range.
pub(crate) const GBUFFER_MARCHER_PUSH_BYTES: u32 = crate::compute::GBUFFER_MARCHER_PUSH_BYTES;

/// The Lighting-L1 cull pipeline's COMPUTE push range size (16 B
/// [`crate::compute::ClusterCullPush`]). Re-exported so [`GBufferScene::cluster_cull_push`]
/// can size its inline byte array without depending on `compute` at the field-decl site.
pub(crate) const CLUSTER_CULL_PUSH_BYTES: u32 = crate::compute::CLUSTER_CULL_PUSH_BYTES;

/// The Lighting-L1 cull shader's `[numthreads(64,1,1)]` group width. The cull's 1D dispatch
/// group count is `ceil(cluster_count / LIGHT_CULL_LOCAL_SIZE_X)`.
pub(crate) const LIGHT_CULL_LOCAL_SIZE_X: u32 = 64;

/// The runtime brick-cache activation the windowed/offscreen G-buffer present applies to the
/// marcher's [`FineMarcherPush`] (the SDF brick-atlas campaign — empty-skip + trilinear/cubic
/// surface cache + clip-map LOD). `None` on [`GBufferScene::brick`] is the OFF path: the recorder
/// builds the push exactly as before (`brick_enabled == 0` / `brick_trilinear == 0` /
/// `brick_levels == 1`), byte-identical to the pre-brick command stream. `Some(_)` turns the brick
/// path ON per-frame, so the caller can flip it at runtime (an A/B toggle) without re-recording any
/// pipeline — the gates live entirely in the per-frame push.
///
/// When `Some`, the recorder stamps the empty-skip grid uniforms (`grid_origin`/`grid_dims`/
/// `brick_world` — level 0's [`boyko_sdf_math::brick::PointerGrid`] geometry the marcher's `lvl == 0`
/// arm indexes binding 9 with) via [`FineMarcherPush::with_brick`], turns on the trilinear+cubic
/// surface path via [`FineMarcherPush::with_brick_trilinear`], and sets the clip-map level count via
/// [`FineMarcherPush::with_brick_levels`]. The per-level atlas/grid SSBOs the marcher samples MUST
/// already be bound at bindings 9..=14 (via the [`GBufferScene`]'s `pointer_grid` / `atlas` /
/// `level_grids` / `level_atlases` fields — pointed at a real [`crate::brick_atlas::BrickClipmap`]),
/// and the b5 camera UBO's `M4GridParams` tail (offset 80) MUST hold the clip-map's baked per-level
/// origins — exactly the offscreen RTX-verified binding discipline. This struct carries ONLY the
/// push-side gates; the descriptor binding + the UBO tail are the caller's (they are extent-stable,
/// written once, NOT per frame).
///
/// `#[repr(C)]`, `Copy` — a small POD the caller flips each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickActivation {
    /// Level 0's empty-skip pointer-grid minimum world corner (cell `(0,0,0)`'s min). The marcher's
    /// `lvl == 0` arm indexes binding 9 with `(grid_origin, grid_dims, brick_world)` — this MUST equal
    /// the [`boyko_sdf_math::brick::PointerGrid`] geometry the level-0 grid bound at binding 9 was
    /// baked at (`PointerGrid::default_near_field().origin` for the origin-centered demo clip-map).
    pub grid_origin: [f32; 3],
    /// Level 0's pointer-grid cell count per axis (`PointerGrid::dims`, e.g. `[16, 16, 16]`).
    pub grid_dims: [u32; 3],
    /// Level 0's pointer-grid cell size — the world width of one brick cell (`PointerGrid::brick_world`,
    /// e.g. `0.5`).
    pub brick_world: f32,
    /// The clip-map level count the marcher loops over (`with_brick_levels`): `BRICK_LEVELS` (3) for the
    /// full clip-map, `1` for the single-level near-field cache. `0` is treated as OFF by the shader.
    pub levels: u32,
}

/// The on-screen Render-P1c G-buffer frame's STATIC inputs: the resources the
/// [`Renderer::render_gbuffer_frame`] 3-pass needs that do NOT depend on the
/// (WSI-clamped) swapchain extent. The EXTENT-dependent targets (depth + the MRT
/// G-buffer images + the descriptor sets bound against them) are owned by
/// [`GBufferTargets`] and (re)allocated by [`GBufferTargets::sync_gbuffer`].
///
/// This is the P1c on-screen counterpart of the P1b OFFSCREEN driver
/// (`tests/sdf_gbuffer_hybrid.rs::run_gbuffer_hybrid`): it mirrors the SAME
/// vocabulary (SSBO edit-list, sampled depth, albedo/normal/material storage, camera
/// UBO) and the SAME marcher compute pipeline, but routes the marcher's ALBEDO
/// (final composite) onto the swapchain image via a present-blit (pass C) instead of a
/// `copy_image_to_buffer` readback.
///
/// # Borrow bundle (like [`SampledComposite`])
///
/// The caller creates each resource through the [`RhiDevice`] trait, OWNS it, and
/// tears it down; the `'a` lifetime ties the bundle to those borrows for the frame
/// call. The bundle keeps [`Renderer::render_gbuffer_frame`]'s signature compact and
/// keeps the recorder out of the resource-creation business (the P1b marcher +
/// G-buffer + present-blit are REUSED verbatim).
///
/// # Static inputs only — the camera UBO + SSBO are seeded ONCE
///
/// The camera/extent UBO (`camera_uniform`) and the edit-list SSBO (`edit_list`) are
/// host-seeded by the caller BEFORE the present loop and are READ-ONLY for the
/// marcher across every frame — so multiple frames-in-flight may dispatch against
/// them with no host write-after-read hazard (the SAME read-only-resident contract
/// [`SampledComposite`] relies on for its texture). The vocabulary descriptor set
/// that binds them is written ONCE per extent in [`GBufferTargets::sync_gbuffer`],
/// NEVER per frame.
///
/// # W2-b format contracts
///
/// `raster_pipeline`'s declared depth format MUST be [`Format::D32Sfloat`] (the
/// depth image the recorder rasterizes into) and its single color format the
/// throwaway-raster format; `marcher`'s layout MUST declare `vocab_layout` at `set
/// 0`; `present_pipeline`'s `color_formats[0]` MUST equal the swapchain format and
/// its layout MUST declare `present_layout` (one COMBINED_IMAGE_SAMPLER) — or the
/// validation layer faults at record/draw time.
/// The Render P7 SSAO compute pass activation: the SSAO pipeline + its DEDICATED 5-binding
/// bind-group LAYOUT, threaded into [`GBufferScene::ssao`] as `Some` to turn the SSAO pass ON.
///
/// `None` on [`GBufferScene::ssao`] is the OFF path: the recorder records NOTHING new (no SSAO
/// descriptor-set write in [`GBufferTargets::create`], no transition / dispatch / barrier in
/// [`Renderer::record_gbuffer`]), so the command stream is BYTE-IDENTICAL to the pre-P7 path
/// (the 0%-gate — proven by C1, since the `ssao` image is always allocated + transitioned
/// regardless of this field). The resolve's `ssao_mode` header gate must be set in lock-step:
/// `Some` ⇒ the scene's light table carries `ssao_mode != 0`; `None` ⇒ `ssao_mode == 0` (the
/// resolve then never reads the SSAO image, so the un-written contents are irrelevant).
///
/// # Borrow bundle (like the marcher / resolve pipelines)
///
/// The caller OWNS the SSAO pipeline + layout and tears them down; the `'a` lifetime ties this
/// activation to those borrows for the frame call. [`GBufferTargets`] writes a 5-binding
/// `ssao_set` against [`Self::layout`] ONCE per extent in [`GBufferTargets::sync_gbuffer`] —
/// binding { gNormal @0 (R), gMaterial @1 (R), gViewT @2 (R), the `ssao` out image @3 (W), the
/// camera UBO @4 } — exactly the SSAO shader's interface (`sdf_ssao.comp.hlsl`).
///
/// `#[derive(Clone, Copy)]` — a pair of borrows the caller flips between frames with no re-record.
#[derive(Clone, Copy)]
pub struct SsaoActivation<'a> {
    /// The Render P7 SSAO compute pipeline (`sdf_ssao.comp` / [`crate::compute::sdf_ssao_spirv`]):
    /// its layout declares [`Self::layout`] at `set 0` + the 80-byte COMPUTE push range (the
    /// shared `CompositePushConstants` range — the SSAO pass reads its camera from the UBO @4, so
    /// it pushes NO constant, but the layout's range must match for create-time validity). The
    /// recorder binds it + dispatches `dispatch_group_count_x` BEFORE the resolve.
    pub pipeline: &'a ComputePipeline,
    /// The DEDICATED 5-binding SSAO bind-group LAYOUT { gNormal STORAGE image @0, gMaterial
    /// STORAGE image @1, gViewT STORAGE image @2, the `ssao` out STORAGE image @3, the camera
    /// UNIFORM buffer @4 } — matching `sdf_ssao.comp`'s set 0. The renderer writes a `ssao_set`
    /// against it once per extent (pointing at the per-extent G-buffer + `ssao` images + the
    /// scene's camera UBO).
    pub layout: &'a VulkanBindGroupLayout,
}

/// HW-RT rung 3a: the spatial (à-trous) RT soft-shadow DENOISE pass activation threaded into
/// [`GBufferScene::shadow`] to turn the denoise pipeline ON. Mirrors [`SsaoActivation`]'s
/// borrow-bundle shape: a `Copy` bundle of caller-owned pipeline/layout borrows plus the
/// per-frame filter parameters.
///
/// `None` on [`GBufferScene::shadow`] is the OFF path (the DEFAULT — the 0%-gate): the recorder
/// records NOTHING new (no VIS/à-trous descriptor-set write in [`GBufferTargets::create`], no VIS
/// / à-trous RDG pass / dispatch / barrier in [`crate::present::passes::gbuffer`]), and the resolve
/// binds the RESOLVE_INLINE-hwrt pipeline (`resolve_pipeline_hwrt`) — so the command stream is
/// BYTE-IDENTICAL to the pre-rung-3a path (the golden). `Some(_)` (populated ONLY when
/// `ShadowDenoiseConfig::enabled()` + a primary directional + a non-empty TLAS all hold — the
/// rung-3a step-7 gate) records the VIS pre-pass + the `levels` à-trous passes, and the resolve
/// binds the DENOISED pipeline reading the filtered visibility.
///
/// # Borrow bundle
///
/// The caller OWNS the VIS / DENOISED resolve pipelines, the à-trous pipeline + its layout (in the
/// host's boot bundle) and tears them down; the `'a` lifetime ties this activation to those borrows
/// for the frame call. The VIS/DENOISED resolve descriptor sets + the per-level à-trous sets are
/// written by [`GBufferTargets`] ONCE per extent (the SSAO `ssao_set` precedent); the recorder
/// selects them by [`Self::final_is_vis2`] / the frame slot at record time.
///
/// `#[derive(Clone, Copy)]` — a bundle of borrows plus the `levels` / `final_is_vis2` scalars,
/// flipped between frames with no re-record.
#[cfg(feature = "hwrt")]
#[derive(Clone, Copy)]
pub struct ShadowVisActivation<'a> {
    /// The VIS-variant resolve pipeline (`deferred_pbr_hwrt_vis.comp` /
    /// [`crate::compute::deferred_pbr_vis_spirv`]) — runs the inline Vogel `rayQuery` trace and
    /// WRITES `gShadowVis` (the 22-binding VIS/DENOISED layout's binding 21) instead of lighting.
    /// Dispatched as the à-trous pre-pass at the resolve's 1D group count BEFORE the à-trous
    /// passes. Its layout is [`Self::resolve_layout`].
    pub vis_pipeline: &'a ComputePipeline,
    /// The DENOISED-variant resolve pipeline (`deferred_pbr_hwrt_denoised.comp` /
    /// [`crate::compute::deferred_pbr_denoised_spirv`]) — reads the FILTERED `gShadowVis` (@21) and
    /// runs the full lighting. Bound as the resolve pipeline (in place of the RESOLVE_INLINE-hwrt
    /// pipeline) when this activation is `Some`. Its layout is [`Self::resolve_layout`].
    pub denoised_pipeline: &'a ComputePipeline,
    /// The 22-binding VIS/DENOISED resolve bind-group LAYOUT (the 21-binding RESOLVE_INLINE-hwrt
    /// layout + `gShadowVis` STORAGE image @21). BOTH [`Self::vis_pipeline`] +
    /// [`Self::denoised_pipeline`] declare it at set 0. The renderer writes the per-FIF VIS +
    /// DENOISED resolve sets against it once per extent.
    pub resolve_layout: &'a VulkanBindGroupLayout,
    /// The à-trous filter pipeline (`shadow_atrous.comp` / [`crate::compute::shadow_atrous_spirv`]),
    /// its layout declares [`Self::atrous_layout`] at set 0 + the 4-byte `{ uint step; }` COMPUTE
    /// push. Dispatched once per level (ping-pong between `shadow_vis` / `shadow_vis2`).
    pub atrous_pipeline: &'a ComputePipeline,
    /// The DEDICATED 6-binding à-trous bind-group LAYOUT { `gVisIn` STORAGE image @0 (R), `gVisOut`
    /// STORAGE image @1 (W), `gNormal` STORAGE image @2, `gViewT` STORAGE image @3, the
    /// `ResolvedShadowDenoise` UNIFORM buffer @4, the camera UNIFORM buffer @5 } — matching
    /// `shadow_atrous.comp`'s set 0. The renderer writes one `atrous_set` per level against it once
    /// per extent (level `i` reads `i`-even ? `shadow_vis` : `shadow_vis2`, writes the other).
    pub atrous_layout: &'a VulkanBindGroupLayout,
    /// HW-RT Rung 3b: the number of à-trous iterations to dispatch this frame — `0..=MAX_ATROUS_LEVELS`.
    /// `spatial ? clamped_levels() (>=1) : 0` (the runner threads `0` for the Temporal-only mode, so
    /// the VIS pass feeds the temporal reproject its RAW output — `final_vis_res == shadow_vis`). Each
    /// pushes `step = 1 << level`; the recorder dispatches this many à-trous passes after the VIS
    /// pre-pass (0 ⇒ none). (Rung 3a called this `levels` and floored it at 1.)
    pub atrous_levels: u32,
    /// `atrous_levels % 2 == 1` — whether the FINAL à-trous output landed in `shadow_vis2` (odd count)
    /// vs `shadow_vis` (even count, incl. `0` ⇒ `shadow_vis` = the raw VIS). Threaded so the DENOISED
    /// resolve set + the temporal pass's `gVisIn` bind the correct final target and the
    /// last-write → read barrier names the right ResId.
    pub final_is_vis2: bool,
    /// HW-RT Rung 3b step 6: `true` iff the author's mode ∈ {`Temporal`, `Both`} (the runner's
    /// `ShadowDenoiseConfig::temporal_enabled()` read). When `true`, the recorder runs the temporal
    /// reproject+accumulate pass AFTER the à-trous chain (reading the final à-trous output / the raw
    /// VIS when `atrous_levels == 0`) and the resolve reads `temporal_out` instead of the à-trous
    /// output. `false` ⇒ the Rung-3a Spatial path (byte-identical): no temporal pass, the resolve
    /// reads the à-trous output.
    pub temporal: bool,
    /// HW-RT Rung 3b step 6: the temporal reproject compute pipeline (`shadow_temporal.comp` /
    /// [`crate::compute::shadow_temporal_spirv`]) — bound by the recorder when [`Self::temporal`] AND
    /// the temporal descriptor sets exist. `Some` iff the boot temporal pipeline was built (an RT
    /// device); `None` on a device without it (the recorder then degrades — no temporal dispatch).
    /// Its 8-binding layout is threaded stably as [`GBufferScene::temporal_layout`] for the set build.
    pub temporal_pipeline: Option<&'a ComputePipeline>,
}

/// The SDFDDGI I2 probe-update compute pass activation: the update pipeline + its DEDICATED
/// 7-binding bind-group LAYOUT, threaded into [`GBufferScene::ddgi_update`] as `Some` to turn the
/// probe-update pass ON. Mirrors [`SsaoActivation`]'s borrow-bundle shape.
///
/// `None` on [`GBufferScene::ddgi_update`] is the OFF path (the DEFAULT — the GI-OFF 0%-gate): the
/// recorder records NOTHING new (no update descriptor-set write in [`GBufferTargets::create`], no
/// RDG pass / dispatch / barrier in [`crate::present::passes::gbuffer`]), so the command stream is
/// BYTE-IDENTICAL to the pre-I2 path (the grand_showcase golden). The atlas + ray-table + UBO are
/// allocated regardless (like the SSAO image), but stay in boot `SHADER_READ_ONLY_OPTIMAL`, unread.
/// `Some(_)` is populated ONLY when `ResolvedDdgi::enabled()` — the SAME predicate driving the
/// LightBuf word-7 GI gate, so the update dispatch and the resolve read never disagree.
///
/// # Borrow bundle
///
/// The caller OWNS the update pipeline + layout (in `boyko_render`'s `DdgiUpdateResources`) and
/// tears them down; the `'a` lifetime ties this activation to those borrows for the frame call.
/// [`GBufferTargets`] writes a SINGLE 7-binding `ddgi_update_set` against [`Self::layout`] ONCE per
/// extent (NOT `[FRAMES_IN_FLIGHT]` — every input is non-ringed per plan §2.2/§7: the update pass
/// binds neither the ringed camera UBO nor any ringed input). The dispatch is sized to the current
/// round-robin subset: `groups_x = DDGI_PROBE_COUNT / subset_n` blocks (one block per active probe).
///
/// `#[derive(Clone, Copy)]` — a pair of borrows plus the dispatch group count, flipped between
/// frames with no re-record.
#[derive(Clone, Copy)]
pub struct DdgiUpdateActivation<'a> {
    /// The I2 probe-update compute pipeline (`sdf_probe_update.comp` /
    /// [`crate::compute::sdf_probe_update_spirv`]): its layout declares [`Self::layout`] at `set 0`.
    /// The shader reads NO push constant (every param rides the b6 `DdgiUpdate` UBO, like SSAO's
    /// camera UBO), but the arming site MUST still create the pipeline with `push_constant_bytes: 4`
    /// (the standard shared compute push range this RHI mandates — a `0`/empty range is rejected;
    /// Vulkan allows a declared-but-unread range). The recorder pushes nothing. It binds the pipeline
    /// + the single bind group + dispatches [`Self::dispatch_group_count_x`] blocks AFTER the marcher
    /// + the L0 light-table copy, BEFORE the resolve.
    pub pipeline: &'a ComputePipeline,
    /// The DEDICATED 7-binding update bind-group LAYOUT { `Buf` STORAGE @0 (R), `gIrrOut` STORAGE
    /// image @1 (W), `gDepthOut` STORAGE image @2 (W), `Classification` STORAGE @3 (RW), `RayTable`
    /// STORAGE @4 (R), `LightBuf` STORAGE @5 (R), `DdgiUpdate` UNIFORM @6 } — matching
    /// `sdf_probe_update.comp`'s set 0. The renderer writes a single `ddgi_update_set` against it
    /// once per extent (the caller-owned bind group in `DdgiUpdateResources` may also be reused).
    pub layout: &'a VulkanBindGroupLayout,
    /// The number of thread-BLOCKS to dispatch (`groups_x`) — one `[numthreads(64,1,1)]` block per
    /// ACTIVE probe in this frame's round-robin subset, i.e. `DDGI_PROBE_COUNT / subset_n` (exact
    /// division; `subset_n` divides `DDGI_PROBE_COUNT = 2048`). The shader maps block `b` → probe
    /// `b * subset_n + (frame_index % subset_n)`. `cmd_dispatch(dispatch_group_count_x, 1, 1)`.
    pub dispatch_group_count_x: u32,
}

/// Pillar B increment B3: the per-instance TRS interpolation compute PRE-PASS activation
/// threaded into [`GBufferScene::interp`] to turn the interp pass ON. Mirrors
/// [`SsaoActivation`]'s borrow-bundle shape: a per-frame `Copy` bundle the caller rebuilds
/// each frame (the CURRENT frame slot's bind group + this frame's overstep alpha).
///
/// `None` (the default for every dump/offscreen scene) keeps the command stream
/// BYTE-IDENTICAL to the pre-B3 path — NO interp dispatch, NO interp barrier is recorded,
/// and the raster/shadow VS read whatever `instance_bind_group` the caller supplies (the
/// static instance ring). `Some(_)` records the interp dispatch BEFORE the raster pass,
/// interpolating each dynamic body's model column into the SHARED instance ring the raster +
/// shadow vertex shaders read; the graph derives the COMPUTE→VERTEX RAW barrier.
///
/// # The shared-ring contract (refined-B — the caller's responsibility)
///
/// Refined-B unifies the output: there is NO private draw SSBO. The host CPU-scatters the
/// STATIC rows into the instance ring, this pass's [`Self::model_out_buffer`] (bound at
/// [`Self::interp_set`] @2) is that SAME ring, and the compute overwrites ONLY the DYNAMIC
/// slots (via [`Self::out_slot_buffer`], the shader's `OutSlot` lane). So the caller keeps
/// [`GBufferScene::instance_bind_group`] pointed at the ring's bind group UNCHANGED whether
/// interp is ON or OFF — no bind swap. The pair, out-slot, and shared-ring slots are all
/// FIF-ringed (frame-private), so no cross-frame barrier is needed beyond the intra-frame
/// COMPUTE→VERTEX dependency the graph derives on the shared ring. The static slots are never
/// in `OutSlot`, so the compute never touches them (single-writer-per-slot).
#[derive(Clone, Copy)]
pub struct InterpActivation<'a> {
    /// The B2 interp compute pipeline (`interp_instances.comp` /
    /// [`crate::compute::interp_instances_spirv`]): its layout declares [`Self::interp_set`]'s
    /// layout at `set 0` + the 8-byte COMPUTE push range
    /// ([`crate::compute::INTERP_INSTANCES_PUSH_BYTES`] — `{ uint count; float alpha }`). The
    /// recorder binds it + dispatches `ceil(count / LOCAL_SIZE_X)` groups BEFORE the raster pass.
    pub pipeline: &'a ComputePipeline,
    /// The CURRENT frame slot's interp bind group { `StructuredBuffer<TransformPair>` @0 (the
    /// host-written pair SSBO, read), `StructuredBuffer<uint>` @1 (the host-written out-slot
    /// SSBO, read), `RWStructuredBuffer<InterpModel>` @2 (the SHARED instance ring, written) }.
    /// A ring the caller rebuilds/selects per frame (all three `frame_index()`).
    pub interp_set: &'a VulkanBindGroup,
    /// The CURRENT frame slot's PAIR SSBO physical buffer (bound at [`Self::interp_set`] @0).
    /// The framegraph resolves the interp pass's declared pair read to this handle; that read
    /// is a first touch on a frame-private slot, so no barrier is derived (the handle is
    /// declared for completeness). Same `frame_index()` slot the host wrote this frame.
    pub pair_buffer: &'a BoundBuffer,
    /// The CURRENT frame slot's OUT-SLOT SSBO physical buffer (bound at [`Self::interp_set`] @1,
    /// the shader's `OutSlot` lane). `out_slot[d]` is dynamic instance `d`'s offset into the
    /// shared model-out ring. Same first-touch handling as the pair buffer (no barrier).
    pub out_slot_buffer: &'a BoundBuffer,
    /// The CURRENT frame slot's SHARED instance-ring physical buffer (refined-B): bound at
    /// [`Self::interp_set`] @2 for the compute WRITE (the dynamic slots), and at
    /// [`GBufferScene::instance_bind_group`] @0 for the raster/shadow VS READ — the SAME
    /// buffer. The host CPU-scatters the STATIC rows into it before this pass; the compute
    /// overwrites ONLY the dynamic slots. The framegraph resolves the COMPUTE→VERTEX RAW
    /// barrier on this ring to this handle — the barrier the raster pass emits so the VS reads
    /// the freshly interpolated columns beside the static ones. Same `frame_index()` slot as
    /// [`Self::interp_set`]'s @2 target.
    pub model_out_buffer: &'a BoundBuffer,
    /// The number of DYNAMIC (interpolated) instances this frame — the compute dispatch element
    /// count (`ceil(count / LOCAL_SIZE_X)` groups) AND the push's `count` bounds guard. `0`
    /// records NO dispatch (a pure-static frame skips the pass entirely, byte-identical to
    /// interp OFF for that frame).
    pub instance_count: u32,
    /// This frame's fixed-timestep overstep fraction (`FixedTime::overstep_fraction()`, in
    /// `[0, 1)`) — pushed as the interp `alpha`. Updates EVERY frame (a per-frame push, no
    /// re-record); the pair SSBO is re-uploaded only on a substep or count change, but the
    /// alpha slides every frame so the interpolated pose advances smoothly between substeps.
    pub alpha: f32,
}

/// HW-RT rung R2a-3: the per-frame TLAS-build activation threaded into
/// [`GBufferScene::tlas`] to turn the GPU-resident TLAS pack + build ON. `None` = the OFF path
/// (the default for every golden/host frame — byte-identical command stream): NO pack dispatch,
/// NO TLAS build, NO barrier is recorded, and the `tlas_instances` framegraph resource routes
/// zero barriers. `Some(_)` = the ON path (armed only under hwrt + ray_query + `count > 0`):
/// BEFORE the raster pass the recorder runs the pack pre-pass (the compute writes one 64-byte
/// `VkAccelerationStructureInstanceKHR` per instance into [`Self::instance_array`], reading the
/// shared M3 ring), the graph derives the pack-write → build-read barrier, then the recorder
/// records the per-frame TLAS build into [`Self::dest`] (the UNTRACKED backing/scratch). Nothing
/// traces the TLAS yet (R2a-4), so the render stays byte-identical.
///
/// A per-frame `Copy` borrow-bundle (mirrors [`InterpActivation`] / [`SsaoActivation`]): the
/// caller flips it between frames with no re-record.
#[cfg(feature = "hwrt")]
#[derive(Clone, Copy)]
pub struct TlasBuildActivation<'a> {
    /// The R2a-3 TLAS-instance packer compute pipeline (`build_tlas_instances.comp` /
    /// [`crate::compute::build_tlas_instances_spirv`]): its layout declares [`Self::bind_group`]'s
    /// 4-binding set at `set 0` + the 4-byte COMPUTE push range
    /// ([`crate::compute::BUILD_TLAS_INSTANCES_PUSH_BYTES`] — `{ uint count }`). The recorder binds
    /// it + dispatches `ceil(count / LOCAL_SIZE_X)` groups BEFORE the raster pass.
    pub pipeline: &'a ComputePipeline,
    /// The CURRENT frame slot's pack bind group { `StructuredBuffer<InstanceModelCol>` @0 (the
    /// shared M3 ring, read), `StructuredBuffer<uint>` @1 (the host-written mesh-id lane, read),
    /// `StructuredBuffer<uint2>` @2 (the per-mesh BLAS-address table, read),
    /// `RWByteAddressBuffer` @3 ([`Self::instance_array`], the 64-byte record output, write) }.
    pub bind_group: &'a VulkanBindGroup,
    /// This slot's persistent TLAS (the per-frame build target). Its backing + scratch are
    /// UNTRACKED by the framegraph (the build's AS write is invisible to the graph), so the
    /// build is a raw `crate::accel::cmd_build_acceleration_structures` call.
    pub dest: &'a BoundAccelStruct,
    /// The compute-written `VkAccelerationStructureInstanceKHR[]` array (the sink's `tlas_instances`
    /// slot): the pack writes it (COMPUTE/SHADER_WRITE), the build reads it (AS_BUILD/SHADER_READ).
    /// The graph derives that single barrier; this is the ONLY framegraph-tracked TLAS resource.
    pub instance_array: &'a BoundBuffer,
    /// The device address of [`Self::instance_array`] (cached once at create) — the build's
    /// `AsGeometryDesc::vertex_data` (the instance-array address).
    pub instance_array_addr: u64,
    /// The device address of this slot's scratch buffer (aligned to `as_scratch_align`, cached
    /// once at create) — the build's `AsBuildEntry::scratch_address`.
    pub scratch_addr: u64,
    /// The host-known drawable instance count this frame (`<= capacity`) — the pack dispatch
    /// element count (`ceil(count / LOCAL_SIZE_X)` groups) + the push's `count` bounds guard +
    /// the build's `primitive_count`. `0` never reaches here (the activation is `None` then).
    pub count: u32,
}

pub struct GBufferScene<'a> {
    /// The mesh-raster graphics pipeline (pass A). Render P5-r0: a 3-MRT G-buffer
    /// PRODUCER — the fronto-parallel quad is drawn into the D32 depth image AND the three
    /// RGBA8 G-buffer color attachments (albedo@0, normal@1, material@2) in the marcher's
    /// exact encoding (mask=1). The caller MUST build it with `color_formats =
    /// [R8G8B8A8_UNORM; 3]` + 3 per-target blend states (W2-b) and the new
    /// `gbuffer_mrt.{vs,fs}` shader pair. (Pre-P5 it was a depth-only prepass with one
    /// throwaway color format.)
    pub raster_pipeline: &'a VulkanGraphicsPipeline,
    /// The mesh quad's host-visible vertex buffer (position + color).
    pub vertex_buffer: &'a BoundBuffer,
    /// The number of vertices to `draw` (the mesh quad's vertex count, e.g. 6).
    pub vertex_count: u32,
    /// The 88-byte `{ float4x4 view_proj; float4 cam_eye; uint base_instance; uint
    /// use_model_matrix }` push to `raster_pipeline`'s `VERTEX` range (see
    /// [`GBUFFER_PUSH_BYTES`]). The leading 64 bytes are the (renamed) `view_proj` matrix
    /// (the old `mvp`); ORTHO scenes append a zeroed `cam_eye` (mode 0), PERSPECTIVE scenes
    /// the world eye + mode 1. M1: every legacy merged draw appends
    /// `base_instance == 0` + `use_model_matrix == 0`, so the VS takes the LEGACY arm —
    /// BYTE-IDENTICAL pixels to the pre-M1 80-byte push (the bit-identity gate).
    pub mvp: [u8; GBUFFER_PUSH_BYTES],
    /// M1: the per-instance model SSBO bind group bound at the raster pipeline's `set 0`
    /// before the pass-A draw — a 1-element [`gbuffer_mrt.vs`'s `InstanceModelCol`] holding
    /// the [`GBUFFER_IDENTITY_INSTANCE`] affine. The legacy merged draw
    /// (`use_model_matrix == 0`) NEVER reads it; it exists only to satisfy the pipeline
    /// layout's static reference to `StructuredBuffer<InstanceModelCol> instances`
    /// (binding 0). The scene OWNS the underlying buffer + bind group; the recorder binds
    /// `instance_bind_group.descriptor_set` once before the draw.
    pub instance_bind_group: &'a VulkanBindGroup,
    /// The P1b SDF G-buffer marcher compute pipeline (its layout declares
    /// `vocab_layout` at `set 0`). Byte-untouched from P1b (pass B).
    pub marcher: &'a ComputePipeline,
    /// The vocabulary bind-group LAYOUT { SSBO @0, sampled depth @1, storage albedo
    /// @2, storage normal @3, storage material @4, UNIFORM camera @5, STORAGE tiles
    /// @6, STORAGE material-table @7, STORAGE `gViewT` @8, STORAGE `PointerGrid` @9,
    /// COMBINED_IMAGE_SAMPLER `BrickAtlas` @10 (M2) }. 11 bindings, within the 12-binding cap. The
    /// renderer allocates + writes a SET against it once per extent (pointing at the
    /// per-extent G-buffer + `gViewT` images + the bundle's `edit_list` / `camera_uniform` /
    /// `depth_sampler` / `tiles_buffer` / `material_table` / `pointer_grid`). PBR MVP-2 added
    /// binding 7 (the material SSBO the marcher fetches `base_color` from); Lighting L0b added
    /// binding 8 (the `gViewT` lane the marcher stores the surface `t` into); M1 added binding
    /// 9 (the empty-skip `PointerGrid`).
    ///
    /// The caller MUST declare binding 6 = `DescriptorKind::StorageBuffer`
    /// (`ShaderStage::COMPUTE`): the P4b marcher shader unconditionally DECLARES `[Set
    /// 0, Binding 6, "Tiles"]`, so the layout + the bound set must carry a VALID
    /// descriptor there even when the coarse cull is gated OFF (`coarse_enabled == 0`).
    ///
    /// The caller MUST likewise declare binding 9 = `DescriptorKind::StorageBuffer`
    /// (`ShaderStage::COMPUTE`): the M1 marcher SPIR-V STATICALLY references
    /// `StructuredBuffer<uint> PointerGrid : register(t9)` inside the empty-skip branch (DXC
    /// does NOT dead-strip the reference despite the runtime `brick_enabled` gate), so the
    /// layout + the bound set must carry a VALID descriptor there even when the empty skip is
    /// gated OFF (`brick_enabled == 0`, the windowed-present path), or
    /// `vkCreateComputePipelines` / `vkCmdDispatch` fail validation
    /// (VUID-…-layout-07988 / -08114).
    ///
    /// The caller MUST likewise declare binding 10 = `DescriptorKind::CombinedImageSampler`
    /// (`ShaderStage::COMPUTE`): the M2 marcher SPIR-V STATICALLY references
    /// `Texture3D BrickAtlas : register(t10)` + `SamplerState BrickSampler : register(s10)`
    /// (collapsed to ONE combined descriptor by DXC) inside the runtime-gated `brick_trilinear`
    /// branch, so the layout + the bound set must carry a VALID combined image+sampler there even
    /// when the trilinear path is gated OFF (`brick_trilinear == 0`, the windowed-present path), or
    /// the SAME layout VUIDs fail.
    ///
    /// The caller MUST likewise declare the M4 clip-map (Slice C) LEVEL-1 + LEVEL-2 bindings:
    /// 11 = `StorageBuffer` (`PointerGrid1`@t11), 12 = `CombinedImageSampler` (`BrickAtlas1`@t12),
    /// 13 = `StorageBuffer` (`PointerGrid2`@t13), 14 = `CombinedImageSampler` (`BrickAtlas2`@t14).
    /// The M4 marcher SPIR-V STATICALLY references all four inside the runtime level branch-ladder
    /// (DXC keeps them past the `brick_levels` gate), so the layout + set must bind VALID descriptors
    /// even on the OFF/N=1 path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread).
    /// 6 brick bindings total (9..=14), within the descriptor cap (`MAX_BIND_GROUP_BINDINGS`).
    ///
    /// The caller MUST likewise declare binding 15 = `DescriptorKind::CombinedImageSampler` (MDF
    /// Stage-2c): the recompiled marcher SPIR-V STATICALLY references the dense mesh-SDF
    /// `Texture3D MeshSdf : register(t15)` + `SamplerState MeshSdfSampler : register(s15)` inside the
    /// runtime-gated `mesh_sdf_enabled` branch, so the layout + set must bind a VALID combined
    /// image+sampler there even when the MDF path is gated OFF (`mesh_sdf_enabled == false` →
    /// bound-but-unread). Binding 15 is the last slot this vocab layout declares (within
    /// the descriptor cap `MAX_BIND_GROUP_BINDINGS`).
    pub vocab_layout: &'a VulkanBindGroupLayout,
    /// The edit-list StorageBuffer (binding 0), host-seeded ONCE before the loop.
    pub edit_list: &'a BoundBuffer,
    /// The camera/extent UNIFORM buffer RING (binding 5), one slot per in-flight frame.
    /// The recorder binds `camera_ring[self.frame_index]` (the slot the upcoming present
    /// waits on, [`Swapchain::frame_index`]); the host writes that SAME slot before the
    /// present, so the sibling in-flight frame reads a DIFFERENT slot — the lock-free
    /// write-after-read fix (no fence stall). For a STATIC scene (offscreen/dump) every
    /// slot is seeded identically and never rewritten, so the output stays byte-identical.
    pub camera_ring: &'a [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The P4b per-tile coarse-cull StorageBuffer (binding 6), sized to the full tile
    /// grid (`tile_grid_extent(w, h)` → `tw * th * TILE_BOUND_BYTES`, STORAGE usage).
    ///
    /// The windowed present path runs the marcher with the coarse cull GATED OFF
    /// (`coarse_enabled == 0`), so the marcher never reads this buffer's contents — but
    /// the marcher shader unconditionally DECLARES binding 6, so Vulkan requires a
    /// VALID StorageBuffer descriptor bound there regardless. The scene OWNS this
    /// buffer; [`GBufferTargets`] only borrows it into the vocabulary set.
    pub tiles_buffer: &'a BoundBuffer,
    /// The M1 empty-space-skip `PointerGrid` StorageBuffer (vocab binding 9): the dense
    /// `dims.0 × dims.1 × dims.2` lattice of [`boyko_sdf_math::brick::BrickClass`] codes
    /// (one `u32` each — the GPU `StructuredBuffer<uint>` element), baked from the ONE edit
    /// authority via [`boyko_sdf_math::brick::build_pointer_grid`] (principle 0 — no parallel
    /// field store) and host-seeded ONCE before the loop, exactly like `edit_list`.
    ///
    /// The windowed present path runs the marcher with the empty skip GATED OFF
    /// (`brick_enabled == 0`), so the marcher NEVER reads this buffer's contents — the
    /// on-screen output stays BYTE-IDENTICAL to the pre-M1 marcher. But the M1 marcher SPIR-V
    /// STATICALLY references `PointerGrid : register(t9)` (DXC keeps the reference past the
    /// runtime gate), so Vulkan requires a VALID StorageBuffer descriptor bound at binding 9
    /// regardless. The scene OWNS this buffer; [`GBufferTargets`] only borrows it into the
    /// vocabulary set. Activating the empty skip on-screen is a separate step
    /// (`FineMarcherPush::with_brick`) — NOT done here.
    pub pointer_grid: &'a BoundBuffer,
    /// The M2 brick-atlas 3D image (vocab binding 10): the dense `M2_ATLAS_DIM³` `R8_SNORM`
    /// (or `R16_SFLOAT` fallback) tile-grid, baked from the ONE edit authority via
    /// [`crate::compute::bake_brick_atlas`] (principle 0 — no parallel field store; the atlas is a
    /// transient GPU mirror rebuilt on the edit `gen`). Created + filled by
    /// [`crate::brick_atlas::BrickAtlas`]; pass [`BrickAtlas::texture`](crate::brick_atlas::BrickAtlas::texture).
    ///
    /// Bound as a `COMBINED_IMAGE_SAMPLER` at binding 10 (with [`Self::atlas_sampler`]): the M2
    /// marcher SPIR-V STATICALLY references `Texture3D BrickAtlas : register(t10)` +
    /// `SamplerState BrickSampler : register(s10)` (collapsed to ONE combined descriptor by DXC)
    /// inside the runtime-gated `brick_trilinear` branch, so the layout MUST declare binding 10 =
    /// `DescriptorKind::CombinedImageSampler` and bind a VALID atlas here even when the trilinear
    /// path is gated OFF (the windowed present path runs `brick_trilinear == 0` → bound-but-unread,
    /// byte-identical output), or `vkCreateComputePipelines` / `vkCmdDispatch` trip the layout VUIDs
    /// (the M1 R2 lesson at binding 9). Activating the trilinear path on-screen is a separate step
    /// (`FineMarcherPush::with_brick_trilinear`) — NOT done here.
    pub atlas: &'a VulkanTexture,
    /// The M2 brick-atlas trilinear / clamp-to-edge / no-mip sampler (vocab binding 10, alongside
    /// [`Self::atlas`] in the combined-image-sampler). Pass
    /// [`BrickAtlas::sampler`](crate::brick_atlas::BrickAtlas::sampler). The hardware trilinear
    /// fetch decodes the `R8_SNORM`/`R16_SFLOAT` codes; clamp keeps an out-of-tile fetch reading the
    /// apron, not a neighbour tile.
    pub atlas_sampler: &'a VulkanSampler,
    /// The M4 clip-map LEVEL-1 + LEVEL-2 pointer grids (vocab bindings 11 + 13): the coarser levels'
    /// `M2_GRID_DIM³` empty-skip lattices ([`crate::brick_atlas::BrickClipmap::grid_buffer`]). The M4
    /// marcher SPIR-V STATICALLY references `PointerGrid1 : register(t11)` + `PointerGrid2 :
    /// register(t13)` inside the runtime level branch-ladder (DXC keeps them past the gate), so the
    /// layout MUST declare bindings 11/13 = `StorageBuffer` and bind VALID buffers even on the OFF/N=1
    /// path (`brick_levels == 1` takes only the lvl==0 arm → bound-but-unread). With no clipmap, bind
    /// level 0's grid ([`Self::pointer_grid`]) as a benign duplicate; `[0]` = level 1, `[1]` = level 2.
    pub level_grids: [&'a BoundBuffer; 2],
    /// The M4 clip-map LEVEL-1 + LEVEL-2 brick atlases (vocab bindings 12 + 14): the coarser levels'
    /// `M2_ATLAS_DIM³` tile-grids ([`crate::brick_atlas::BrickClipmap::atlas`]'s texture). The M4
    /// marcher SPIR-V STATICALLY references `BrickAtlas1 : register(t12)` + `BrickAtlas2 :
    /// register(t14)` (each a COMBINED_IMAGE_SAMPLER with [`Self::level_atlas_samplers`]) inside the
    /// branch-ladder, so the layout MUST declare bindings 12/14 = `CombinedImageSampler` and bind VALID
    /// atlases even on the OFF/N=1 path (bound-but-unread). With no clipmap, bind level 0's atlas
    /// ([`Self::atlas`]) as a benign duplicate; `[0]` = level 1, `[1]` = level 2.
    pub level_atlases: [&'a VulkanTexture; 2],
    /// The M4 clip-map LEVEL-1 + LEVEL-2 atlas samplers (vocab bindings 12 + 14, alongside
    /// [`Self::level_atlases`]). NEAREST / clamp-to-edge / no-mip like [`Self::atlas_sampler`]. With no
    /// clipmap, bind level 0's sampler; `[0]` = level 1, `[1]` = level 2.
    pub level_atlas_samplers: [&'a VulkanSampler; 2],
    /// MDF Stage-2c: the DEDICATED DENSE mesh-SDF shadow-caster 3D image (vocab binding 15): a static
    /// mesh's baked `R8_SNORM` signed-distance grid ([`crate::mesh_sdf_texture::MeshSdfTexture`]'s
    /// texture). Bound as a `COMBINED_IMAGE_SAMPLER` at binding 15 (with [`Self::mesh_sdf_sampler`]):
    /// the recompiled marcher SPIR-V STATICALLY references `Texture3D MeshSdf : register(t15)` +
    /// `SamplerState MeshSdfSampler : register(s15)` inside the runtime-gated `mesh_sdf_enabled`
    /// branch, so the layout MUST declare binding 15 = `DescriptorKind::CombinedImageSampler` and bind a
    /// VALID texture here even when the MDF path is gated OFF (`mesh_sdf_enabled == false` → bound-but-
    /// unread, byte-identical output), or `vkCreateComputePipelines`/`vkCmdDispatch` trip the layout
    /// VUIDs (the M2 binding-10 R2 lesson). For a non-MDF scene, bind a benign placeholder (e.g.
    /// [`Self::atlas`] as a duplicate). Activating the MDF path is the per-frame
    /// [`Self::mesh_sdf_enabled`] gate.
    pub mesh_sdf: &'a VulkanTexture,
    /// MDF Stage-2c: the mesh-SDF LINEAR / clamp-to-edge / no-mip sampler (vocab binding 15, alongside
    /// [`Self::mesh_sdf`] in the combined-image-sampler). LINEAR is sound — the dense grid is a
    /// conservative lower bound (a trilinear blend never overshoots). Pass
    /// [`MeshSdfTexture::sampler`](crate::mesh_sdf_texture::MeshSdfTexture::sampler); for a non-MDF
    /// scene a benign placeholder (e.g. [`Self::atlas_sampler`]).
    pub mesh_sdf_sampler: &'a VulkanSampler,
    /// MDF Stage-2c: the per-frame mesh-distance-field SHADOW gate stamped into the marcher's
    /// [`FineMarcherPush`] `mesh_sdf_enabled` (offset 72). `false` (the default for every non-MDF
    /// scene) keeps the push + output BYTE-IDENTICAL to pre-MDF — the mesh-SDF texture is
    /// bound-but-unread, the shadow march stays the frozen analytic `sdf_soft_shadow` (the 0%-gate).
    /// `true` marches `sdf_soft_shadow_mesh` (the mesh-aware union), so a raster static mesh casts its
    /// baked MDF soft shadow. The caller MUST have written the b5 UBO's
    /// [`MeshSdfParams`](crate::compute::MeshSdfParams) tail (the grid transform) when this is `true`.
    /// A per-frame push field (no re-record on a flip).
    pub mesh_sdf_enabled: bool,
    /// The sampler bound alongside the depth image at binding 1 (ignored by the
    /// marcher's unfiltered `.Load`, but the SAMPLED_IMAGE descriptor requires one).
    pub depth_sampler: &'a VulkanSampler,
    /// The fullscreen-sample present pipeline (pass C): samples the LIT image (the
    /// deferred resolve's output) into the swapchain (`color_formats[0]` == the swapchain
    /// format). The deferred split rewired this from ALBEDO → LIT (the only present change).
    pub present_pipeline: &'a VulkanGraphicsPipeline,
    /// The present-sample bind-group LAYOUT (one COMBINED_IMAGE_SAMPLER @ set 0). The
    /// renderer allocates + writes a SET against it once per extent (pointing at the
    /// per-extent LIT image + `present_sampler`).
    pub present_layout: &'a VulkanBindGroupLayout,
    /// The sampler the present-blit samples the LIT image with (nearest/clamp for
    /// a 1:1 sample).
    pub present_sampler: &'a VulkanSampler,
    /// The PBR MVP-2 material table SSBO (`MaterialGpu[]`), host-seeded ONCE before the
    /// loop. Bound at the marcher vocab set's binding 7 (the marcher fetches `base_color`)
    /// AND the resolve set's binding 4 (the resolve fetches metallic/roughness/etc.). The
    /// scene OWNS it; [`GBufferTargets`] borrows it into both sets.
    pub material_table: &'a BoundBuffer,
    /// The Lighting-L0 light table SSBO (`[LightHeaderGpu || GpuLight[]]`, word-indexed;
    /// `light_table.hlsli`). A DEVICE-LOCAL buffer minted with `TRANSFER_DST | STORAGE`
    /// usage, bound to the resolve set's binding 6. Seeded ONCE via the fence-waited
    /// `upload_initial`; re-uploaded on-change via the async recorded copy below (C3 /
    /// rung L0-r0). The scene OWNS it; [`GBufferTargets`] borrows it into the resolve set.
    pub light_table: &'a BoundBuffer,
    /// The host-coherent STAGING source for the light table (rung L0-r0). On a dirty
    /// frame the recorder copies `light_upload_bytes` from this into `light_table` +
    /// records a TRANSFER_WRITE→SHADER_READ barrier, fence-free, BEFORE the marcher
    /// dispatch. The collection system writes the new table into this buffer's mapped
    /// bytes and sets `light_dirty`.
    pub light_staging: &'a BoundBuffer,
    /// The number of bytes to copy on a dirty frame (`[header || GpuLight[]]` length).
    pub light_upload_bytes: u64,
    /// `true` on a frame where the light table changed: the recorder records the async
    /// staging→`light_table` copy + barrier; `false` records NOTHING (idle frame → zero
    /// cost, byte-identical command stream — the rung L0-r0 0%-gate).
    pub light_dirty: bool,
    /// The Lighting-L1 clustered froxel light-cull compute pipeline (`cluster_cull.comp`):
    /// its layout declares the cull bind-group LAYOUT at `set 0` + a 16-byte
    /// [`crate::compute::ClusterCullPush`] COMPUTE push range. `None` ⇒ L1 is not wired (the
    /// L0b-only build) and the cull pass + its barriers are skipped entirely (the resolve's
    /// `clusters_enabled` header gate then loops the flat table — the L1 OFF path). When
    /// `Some`, the recorder dispatches it (over [`Self::cluster_count`] froxels) BEFORE the
    /// resolve, with a COMPUTE→COMPUTE buffer barrier so the resolve reads see the cull writes.
    pub cluster_cull: Option<&'a ComputePipeline>,
    /// The cull bind-group LAYOUT { camera UBO @0, light table SSBO @1, `ClusterGrid` SSBO
    /// @2, `LightIndexList` SSBO @3, `LightIndexAlloc` SSBO @4 } — matching `cluster_cull.hlsl`'s
    /// set 0. The renderer writes a `cull_set` against it once per extent (pointing at the
    /// scene's camera UBO + light table + cluster buffers). `None` when [`Self::cluster_cull`]
    /// is `None`.
    pub cull_layout: Option<&'a VulkanBindGroupLayout>,
    /// The L1 per-froxel `ClusterCell`/`{offset,count}` grid SSBO (`DEVICE_LOCAL`, STORAGE),
    /// sized `cluster_count * 8 B`. Written by the cull pass, read by the resolve set's
    /// binding 8. The scene OWNS it; [`GBufferTargets`] borrows it into both the cull set and
    /// the resolve set. `None` when L1 is off (the resolve set then binds the light table at
    /// @8/@9 as a harmless valid placeholder — see [`GBufferTargets::create`]).
    pub cluster_grid: Option<&'a BoundBuffer>,
    /// The L1 flat light-index list SSBO (`DEVICE_LOCAL`, STORAGE), sized `index_list_cap *
    /// 4 B`. The cull atomic-appends survivor indices; the resolve reads the pixel's froxel
    /// slice from it (resolve binding 9). `None` when L1 is off.
    pub light_index: Option<&'a BoundBuffer>,
    /// The L1 global slice-allocation counter SSBO (one `u32`, `DEVICE_LOCAL`, STORAGE). The
    /// cull `InterlockedAdd`s element 0 to claim disjoint `light_index` slices. It is RESET to
    /// 0 (a `cmd_fill_buffer`) before each cull dispatch (the per-frame rebuild). `None` when
    /// L1 is off.
    pub light_index_alloc: Option<&'a BoundBuffer>,
    /// The 16-byte [`crate::compute::ClusterCullPush`] bytes (exp-Z near/far + the caps) the
    /// cull pass pushes. Ignored when [`Self::cluster_cull`] is `None`.
    pub cluster_cull_push: [u8; CLUSTER_CULL_PUSH_BYTES as usize],
    /// The L1 froxel count (`dim_x * dim_y * dim_z`, default 3456) — the cull's 1D dispatch
    /// thread count (`ceil(cluster_count / LOCAL_SIZE_X)` groups). Ignored when L1 is off.
    pub cluster_count: u32,
    /// The deferred PBR RESOLVE compute pipeline (`deferred_pbr.comp`): its layout declares
    /// `resolve_layout` at `set 0`. Reads the marcher's gAlbedo + gNormal + gMaterial
    /// (STORAGE, GENERAL) + the material SSBO + the camera UBO, runs Cook-Torrance, and
    /// stores the final LIT color into the dedicated lit image.
    pub resolve_pipeline: &'a ComputePipeline,
    /// The deferred resolve bind-group LAYOUT (8 bindings, ≤ 12): { storage gAlbedo @0,
    /// storage gNormal @1, storage gMaterial @2, storage lit @3, material SSBO @4, camera
    /// UBO @5, light table SSBO @6, storage `gViewT` @7 }. The renderer allocates + writes a
    /// SET against it once per extent (pointing at the per-extent G-buffer + lit + `gViewT`
    /// images + the scene's material SSBO + camera UBO + light table). Binding 6 (Lighting
    /// L0a) replaces the compiled-in `LIGHT_DIR`/`SKY_*` constants with the header+table
    /// read; binding 7 (Lighting L0b) is the `gViewT` lane the resolve reconstructs `P` from.
    pub resolve_layout: &'a VulkanBindGroupLayout,
    /// R2a-4b: the HWRT-variant deferred RESOLVE pipeline (`deferred_pbr_hwrt.comp` /
    /// [`crate::compute::deferred_pbr_hwrt_spirv`]) — the mesh-shadow term routes to an inline
    /// `rayQuery` TLAS trace (binding 19) instead of the CSM shadow-map sample. `None` on EVERY
    /// non-hwrt / non-RT / config-Software path ⇒ the recorder binds the software
    /// [`Self::resolve_pipeline`] ⇒ byte-identical to the golden. `Some(_)` (built only under
    /// `feature = "hwrt"` + `ctx.ray_query_enabled()`) carries its OWN 20-binding layout
    /// ([`Self::resolve_layout_hwrt`]) — the record-site selects the `(pipeline, layout, set)`
    /// TRIPLE together (a layout mismatch is a device-lost). The whole field is `#[cfg(hwrt)]`, so
    /// a `not(hwrt)` build has it absent entirely.
    #[cfg(feature = "hwrt")]
    pub resolve_pipeline_hwrt: Option<&'a ComputePipeline>,
    /// R2a-4b: the 20-binding bind-group LAYOUT [`Self::resolve_pipeline_hwrt`] declares at set 0
    /// (the 19 software bindings + binding 19 `AccelerationStructure`). `Some` iff
    /// [`Self::resolve_pipeline_hwrt`] is `Some` (they are built + selected in lock-step); the
    /// record-site binds [`GBufferTargets::resolve_set_hwrt`] against THIS layout when routing is
    /// Hardware, never the software [`Self::resolve_layout`].
    #[cfg(feature = "hwrt")]
    pub resolve_layout_hwrt: Option<&'a VulkanBindGroupLayout>,
    /// R2a-4b: the per-FIF persistent TLAS handles (the host's `PersistentTlas.accel`) the HWRT
    /// resolve set binds at binding 19 — the stable, built-into (not recreated) acceleration
    /// structures the `rayQuery` trace reads. An array of per-slot borrows (the host's per-FIF
    /// TLASes are not contiguous, so this is `[&BoundAccelStruct; N]`, not `&[_; N]`). `Some` iff
    /// [`Self::resolve_pipeline_hwrt`] is `Some`. The resolve-set builder
    /// ([`GBufferTargets::create`]) writes slot `i`'s TLAS into slot `i`'s HWRT set (the
    /// once-per-FIF write model holds — the handle is frame-stable). `None` on the software path
    /// (no AS descriptor is written).
    #[cfg(feature = "hwrt")]
    pub resolve_tlas_hwrt: Option<[&'a BoundAccelStruct; FRAMES_IN_FLIGHT]>,
    /// HW-RT rung 1b: the HWRT soft-shadow-params UBO ring
    /// (`boyko_render::ResolvedRayShadow`, 16 B — cone/tmax/tmin/bias) the HWRT resolve set
    /// binds at binding 20. Written ONLY into the HWRT resolve set (the software resolve set
    /// stays EXACT at 19 bindings). The whole field is `#[cfg(hwrt)]`, so a `not(hwrt)` build
    /// has it absent entirely; the host supplies it only on an RT device
    /// (`ray_query_enabled()`), the same gate as the TLAS handles above.
    ///
    /// A RING (one slot per in-flight frame, like [`Self::csm_cascade_ring`]): each FIF frame
    /// binds its own slot `ray_shadow_ubo[self.frame_index]` @20, the host writes that SAME slot
    /// via `upload_ray_shadow_ring` before the present (a per-frame cone/tmax/tmin/bias retune),
    /// so the sibling in-flight frame reads a DIFFERENT slot — the lock-free write-after-read
    /// fix. A STATIC config seeds every slot identically (byte-identical).
    #[cfg(feature = "hwrt")]
    pub ray_shadow_ubo: &'a [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The marcher's 1D dispatch group count (`ceil(pixels / LOCAL_SIZE_X)` at the
    /// WSI-clamped extent the recorder dispatches). The deferred resolve dispatches at the
    /// SAME grid (1:1 the marched pixels).
    pub dispatch_group_count_x: u32,
    /// The SDF brick-cache activation applied to the marcher push THIS frame. `None` = the OFF
    /// path (`brick_enabled == 0` / `brick_trilinear == 0` / `brick_levels == 1`), byte-identical
    /// to the pre-brick command stream — the bound brick descriptors at 9..=14 stay bound-but-unread.
    /// `Some(_)` turns the empty-skip + trilinear/cubic surface cache + clip-map LOD ON (the gates
    /// live entirely in the per-frame push, so the caller may flip this every frame for an A/B
    /// toggle). When `Some`, the caller MUST have bound the real [`crate::brick_atlas::BrickClipmap`]
    /// per-level resources at 9..=14 and written its `M4GridParams` tail into the b5 UBO (see
    /// [`BrickActivation`]).
    pub brick: Option<BrickActivation>,
    /// The P4b COARSE TILE-CULL compute pipeline (`sdf_tile_cull.comp`), applied to this frame.
    /// `None` = the OFF path (the default, byte-identical to the pre-P0 command stream): NO coarse
    /// dispatch + NO `tiles_buffer` barrier are recorded, and the marcher push carries
    /// `coarse_enabled == 0`, so the marcher never reads [`Self::tiles_buffer`]'s contents.
    ///
    /// `Some(coarse)` = the ON path (a PERF optimization, not a visual one): BEFORE the marcher
    /// (pass B), the recorder binds this pipeline against the SAME vocabulary descriptor set (the
    /// coarse-cull shader declares only a subset of the vocab layout — sharing the full layout is
    /// valid), dispatches `ceil(tile_count / LOCAL_SIZE_X)` groups (one invocation per 8×8 tile,
    /// each writing a `TileBound` into vocab binding 6 — [`Self::tiles_buffer`]), records a
    /// COMPUTE-WRITE → COMPUTE-READ buffer barrier on `tiles_buffer`, and then the marcher push
    /// carries `coarse_enabled == 1`, so the fine marcher reads the per-tile bounds and skips
    /// empty / cone-rejected tiles. The cull MUST NOT change pixels (only fewer marches), so the ON
    /// output equals the OFF output within the goldens' per-channel tolerance.
    ///
    /// `coarse`'s layout MUST declare [`Self::vocab_layout`] at `set 0` (it shares the marcher's
    /// vocabulary set verbatim) and the same compute push range; the depth image it samples is
    /// already `SHADER_READ_ONLY_OPTIMAL` (the dual-use depth barrier the recorder emits before pass
    /// B) and the `tiles_buffer` it writes is bound at vocab binding 6 (always — caller contract).
    /// Flipping `coarse` between frames needs NO re-record (it gates only the recorded dispatch +
    /// the push byte), so the caller may A/B-toggle it live.
    pub coarse: Option<&'a ComputePipeline>,
    /// The marcher's coarse-cull CONSUMPTION mode ([`CoarseMode`]) stamped into the push when
    /// [`Self::coarse`] is `Some` (when `None`, the recorder forces [`CoarseMode::Off`], so this
    /// field is a don't-care on the OFF path). The cull DISPATCH is identical across modes — only
    /// the marcher's reading of the per-tile bounds differs:
    ///
    /// - [`CoarseMode::Full`] — the historical EMPTY-skip + `near_t` seed (the offscreen goldens'
    ///   mode; image-transparent under the UNLIT contract).
    /// - [`CoarseMode::EmptySkipOnly`] — the LIT-TRANSPARENT cull: EMPTY-skip only, NO `near_t`
    ///   seed (the seed shifts the grazing-silhouette AO/shadow rim; dropping it removes the rim).
    ///   This is the on-screen windowed-present mode (lit-transparent, near-identical perf).
    ///
    /// A per-frame push field (no re-record on a flip). Defaults to [`CoarseMode::Off`].
    pub coarse_mode: CoarseMode,
    /// The A1/A2 lighting flags stamped into the marcher's [`FineMarcherPush`] `lighting_flags`
    /// (offset 8) THIS frame: `LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO` for the on-screen demo's
    /// soft-shadow + AO shading, or `0` for the byte-identical Lambert path.
    ///
    /// This is a per-frame push field (NOT a descriptor), so flipping it needs no re-record and the
    /// OFF (`coarse == None`) command stream stays byte-identical for any fixed value.
    ///
    /// # Why it is a field (the P0 coarse-cull transparency contract)
    ///
    /// The coarse cull ([`Self::coarse`]) is proven IMAGE-TRANSPARENT — cull-ON equals cull-OFF
    /// within the goldens' tolerance — by the offscreen golden
    /// `sdf_gbuffer_hybrid::p4b_cull_on_conservative_within_tol_of_cull_off`, which runs that
    /// comparison on the UNLIT marcher (`lighting_flags == 0`). With shadows + AO ON, the cull's
    /// conservative per-tile `near_t` / EMPTY classification (tuned for the primary hit test) is
    /// NOT transparent to the secondary AO / shadow rays near a grazing silhouette: a tile the cull
    /// deems empty-enough for the primary ray still owes an AO darkening the un-culled march would
    /// have produced, so the lit cull-ON image drops that darkening (a visible ring). That cull ⇄
    /// lighting interaction is a separate, un-shipped invariant — the shipped cull contract is the
    /// unlit one. Exposing `lighting_flags` lets a cull-transparency test compare under the proven
    /// (`0`) condition while the on-screen present keeps shadows + AO.
    pub lighting_flags: u32,
    /// The directional-light direction (the un-normalized "direction TO the light", `L`) the
    /// marcher marches the A1 soft shadow toward — stamped into the marcher's [`FineMarcherPush`]
    /// `light_dir` (offset 16) THIS frame. It MUST equal the resolve's PRIMARY directional light
    /// direction (the first directional in the light table, the one whose `vis` reads
    /// `gMaterial.r`): the marcher bakes the cast shadow toward `light_dir`, and the resolve
    /// consumes it as the primary directional's visibility — a mismatch detaches the shadow from
    /// the light. A per-frame push field (no re-record on a change). For a head-on scene use the
    /// legacy [`DEFAULT_LIGHT_DIR`](crate::compute::DEFAULT_LIGHT_DIR) `[0, 0, 1]`; an angled /
    /// floor-and-object scene supplies the real sun direction so a real cast shadow lands.
    pub light_dir: [f32; 3],
    /// The Render P7 SSAO compute pass activation. `None` = the OFF path (the default): the
    /// recorder records NOTHING new (no SSAO set-write, no transition / dispatch / barrier), so
    /// the command stream is BYTE-IDENTICAL to the pre-P7 path (the 0%-gate — the `ssao` image is
    /// allocated + transitioned regardless, C1). `Some(_)` = the ON path: [`GBufferTargets`]
    /// writes the 5-binding `ssao_set` against the activation's layout, and BEFORE the resolve the
    /// recorder binds the SSAO pipeline + that set, dispatches [`Self::dispatch_group_count_x`],
    /// and barriers the `ssao` image (COMPUTE→COMPUTE, GENERAL) so the resolve's `gSsao.Load` sees
    /// the store. The caller MUST set the scene's light table `ssao_mode` in lock-step (`!= 0` ON,
    /// `0` OFF) — the resolve's structural gate that decides whether the combine reads the image.
    pub ssao: Option<SsaoActivation<'a>>,
    /// Mesh foundation M3: the per-mesh instanced-arm draw BATCH LIST. An EMPTY slice
    /// (every pre-M2 scene) records the LEGACY pass-A draw — `vkCmdDraw(vertex_count, 1,
    /// 0, 0)` over [`Self::vertex_buffer`] binding [`Self::instance_bind_group`] (the
    /// identity dummy), the `use_model_matrix == 0` arm — BYTE-IDENTICAL to the pre-M2
    /// stream (the 0%-gate). A NON-empty slice records the M3 batch loop: [`Self::
    /// instance_bind_group`] (now the SHARED N-instance model SSBO the gather filled) is
    /// bound ONCE at set 0, then one INSTANCED INDEXED draw per [`GBufferMeshDraw`] — each
    /// binding its mesh's vertex+index buffers (with its O3 index width) and pushing its
    /// `base_instance` bucket offset. The caller MUST build [`Self::mvp`] with
    /// `use_model_matrix == 1` (its byte 84) when the slice is non-empty; the recorder
    /// overwrites the push's `base_instance` word per batch. See [`GBufferMeshDraw`].
    pub mesh_draw: &'a [GBufferMeshDraw<'a>],
    /// CSM Increment 1b (Rung A): the cascade shadow-map ARRAY texture (a D32 array; Rung A
    /// renders ONLY layer 0). ALWAYS supplied (a real cascade map when CSM is on, a 1×1×1 D32
    /// array DUMMY when off) so the resolve set can ALWAYS bind binding 12 — the resolve `.spv`
    /// statically references `gCsm`, so the descriptor MUST be valid even on the OFF path
    /// (bound-but-unread; the `SampleCmpLevelZero` runs only under the `csm_mode != 0` gate). The
    /// scene OWNS it; the depth pass (when [`Self::csm`] is `Some`) renders into
    /// `layer_render_view(0)`, the resolve PCF-samples through the array sample view bundled with
    /// [`Self::csm_compare_sampler`].
    ///
    /// INTENTIONALLY NOT RINGED (a single instance, unlike the G-buffer render targets): the
    /// cascade map is WORLD-FIXED — rendered every frame from CONSTANT cascade matrices over the
    /// STATIC caster batch, so its content is byte-identical each frame and a cross-frame read of
    /// it is benign (no Write-After-Read jitter). It would only need ringing if a future scene made
    /// its content camera-dependent (per-frame re-fit cascades that actually move the casters).
    pub csm_cascade_texture: &'a VulkanTexture,
    /// CSM Increment 1b (Rung A): the PCF COMPARISON sampler (`compareEnable = VK_TRUE`,
    /// `LessOrEqual` — Inc-0 `SamplerDesc.compare = Some(LessOrEqual)`), BUNDLED with
    /// [`Self::csm_cascade_texture`] as the resolve's binding-12 combined image+sampler. ALWAYS
    /// supplied (bound-but-unread on the OFF path).
    pub csm_compare_sampler: &'a VulkanSampler,
    /// CSM Increment 1b (Rung A): the cascade UBO (host-coherent), a 336-byte byte-mirror of
    /// `boyko_render::ResolvedCsm` (the inline `[CascadeData; 4]` plus `active_count`,
    /// `csm_mode_word`, and pad). The host uploads `ResolvedCsm` verbatim before the frame; the
    /// resolve reads `gCascades[0].view_proj` (Rung A) through binding 13. ALWAYS supplied — a
    /// ZEROED UBO on the OFF path (bound-but-unread). The depth pass pushes the SAME
    /// `gCascades[0].view_proj`, so the host writes it once into this UBO and the recorder reads it
    /// back for the depth-pass push.
    ///
    /// A RING (one slot per in-flight frame, like [`Self::camera_ring`]): the resolve binds
    /// `csm_cascade_ring[self.frame_index]` @13, the host writes that SAME slot before the present
    /// (a per-frame CSM re-fit), so the sibling in-flight frame reads a DIFFERENT slot — the
    /// lock-free write-after-read fix. A STATIC scene seeds every slot identically (byte-identical).
    pub csm_cascade_ring: &'a [BoundBuffer; FRAMES_IN_FLIGHT],
    /// CSM Increment 1b (Rung A): the cascade DEPTH-PASS activation. `None` = the OFF path (the
    /// default, byte-identical command stream): NO depth pass is recorded, the resolve's `csm_mode`
    /// header gate is 0, and the always-bound cascade map/sampler/UBO are bound-but-unread.
    /// `Some(_)` = the ON path: BEFORE the resolve dispatch the recorder runs [`record_csm_depth`]
    /// — barriers the cascade image, LOOPS the `[0..active_count)` cascades (Rung B: N), rendering
    /// the SAME caster batches ([`Self::mesh_draw`] + [`Self::instance_bind_group`], build-once-
    /// consume-N-views) into `layer_render_view(c)` with cascade `c`'s `view_proj` pushed, then
    /// barriers the whole array to `SHADER_READ_ONLY_OPTIMAL` for the resolve sample. The caller
    /// MUST set the scene's light-header `csm_mode` in lock-step (`with_csm_mode(true)`).
    pub csm: Option<CsmDepthActivation<'a>>,
    /// Shadow Phase 5 Inc-1-GPU: the sparse SPOT/POINT shadow-ATLAS array texture (a D32 array;
    /// `array_layers == boyko_render::shadow_atlas::M_SLOTS == 16`). ALWAYS supplied (a real atlas
    /// when sparse shadows are on, a 1×1×1 D32 array DUMMY when off) so the resolve set can ALWAYS
    /// bind binding 14 — the resolve `.spv` statically references `gShadowAtlas`, so the descriptor
    /// MUST be valid even on the OFF path (bound-but-unread; the `SampleCmpLevelZero` runs only under
    /// the `punctual_shadow_mode != 0` gate). The scene OWNS it; the depth pass (when
    /// [`Self::atlas_punctual`] is `Some`) renders into `layer_render_view(s)` per spot slot, the
    /// resolve PCF-samples through the array sample view bundled with [`Self::shadow_atlas_sampler`].
    ///
    /// The DEPTH TEXTURE itself is a single instance (not ring-swapped): the punctual depth pass
    /// re-renders every ACTIVE layer this frame and then barriers the WHOLE `M_SLOTS` array to
    /// `SHADER_READ_ONLY_OPTIMAL` before the resolve sample (see [`Self::atlas_punctual`]), so the
    /// resolve only ever reads layers written this frame. NOTE: the atlas CONTENT is NOT
    /// byte-identical across frames — the host fit (`resolve_shadow_atlas`) is CAMERA-DEPENDENT (the
    /// `spot_priority` = range²/dist² top-K selection shifts which lights get slots as the camera
    /// moves), so the per-slot `view_proj` matrices and slot assignment change frame to frame. The
    /// UBO carrying that fit ([`Self::shadow_atlas_ubo`]) IS host-ringed for exactly this reason;
    /// the depth image needs no ring because the whole-array barrier closes the cross-frame read.
    pub shadow_atlas_texture: &'a VulkanTexture,
    /// Shadow Phase 5 Inc-1-GPU: the PCF COMPARISON sampler (`compareEnable = VK_TRUE`,
    /// `LessOrEqual`), BUNDLED with [`Self::shadow_atlas_texture`] as the resolve's binding-14
    /// combined image+sampler. ALWAYS supplied (bound-but-unread on the OFF path).
    pub shadow_atlas_sampler: &'a VulkanSampler,
    /// Shadow Phase 5 Inc-1-GPU: the shadow-atlas UBO (host-coherent), a 1296-byte byte-mirror of
    /// `boyko_render::ResolvedShadowAtlas` (the inline `[FaceTransform; M_SLOTS]` plus
    /// `active_layers`, `mode_word`, and pad). The host uploads `ResolvedShadowAtlas` verbatim before
    /// the frame; the resolve reads `gFaces[slot].view_proj` through binding 15. ALWAYS supplied — a
    /// ZEROED UBO on the OFF path (bound-but-unread). The depth pass pushes the SAME
    /// `gFaces[slot].view_proj`, so the host writes it once into this UBO and the recorder reads it
    /// back for the depth-pass push.
    pub shadow_atlas_ubo: &'a BoundBuffer,
    /// SDFDDGI I0: the DDGI probe-IRRADIANCE atlas texture (a bound-but-unread `Texture2DArray`
    /// dummy this rung — a real R11G11B10F octahedral atlas at I1). ALWAYS supplied so the resolve
    /// set can bind binding 16 — the resolve `.spv` statically references `gDdgiIrr`, so the
    /// descriptor MUST be valid even on the OFF path (the `SampleLevel` runs only under the
    /// `ddgi_mode != 0` gate, OFF by default → bound-but-unread, the `gCsm`/`gShadowAtlas`
    /// precedent). Reuse an existing bound-but-unread array texture (e.g. the cascade map) for the
    /// I0 dummy — the descriptor TYPE (`COMBINED_IMAGE_SAMPLER`) is all Vulkan validates, not the
    /// shader's element type. BUNDLED with [`Self::ddgi_irr_sampler`] as the combined image+sampler.
    pub ddgi_irr_texture: &'a VulkanTexture,
    /// SDFDDGI I0: the LINEAR sampler BUNDLED with [`Self::ddgi_irr_texture`] as the resolve's
    /// binding-16 combined image+sampler (the `gDdgiIrr`(t16)+`gDdgiIrrSamp`(s16) DXC collapse).
    /// ALWAYS supplied (bound-but-unread on the OFF path); reuse any existing linear sampler.
    pub ddgi_irr_sampler: &'a VulkanSampler,
    /// SDFDDGI I0: the DDGI probe DEPTH-MOMENT atlas texture (a bound-but-unread `Texture2DArray`
    /// dummy this rung — a real RG16F two-moment atlas at I1). ALWAYS supplied so the resolve set can
    /// bind binding 17 — the resolve `.spv` statically references `gDdgiDepth` (bound-but-unread on
    /// the OFF path, same contract as [`Self::ddgi_irr_texture`]). BUNDLED with
    /// [`Self::ddgi_depth_sampler`]. Reuse an existing bound-but-unread array texture for the dummy.
    pub ddgi_depth_texture: &'a VulkanTexture,
    /// SDFDDGI I0: the LINEAR sampler BUNDLED with [`Self::ddgi_depth_texture`] as the resolve's
    /// binding-17 combined image+sampler. ALWAYS supplied (bound-but-unread on the OFF path).
    pub ddgi_depth_sampler: &'a VulkanSampler,
    /// SDFDDGI I0: the DDGI grid UBO (host-coherent), a
    /// [`RESOLVED_DDGI_BYTES`](boyko_render's `RESOLVED_DDGI_BYTES`, 48 B) byte-mirror of
    /// `boyko_render::ResolvedDdgi` (`origin` + `inv_spacing`/dims + `ddgi_mode_word` + pad). ALWAYS
    /// supplied — a ZEROED UBO on the OFF path (bound-but-unread; `ddgi_mode_word == 0`). The grid is
    /// WORLD-FIXED (Decision D1), so this is a SINGLE buffer, NOT a per-FIF ring (unlike the
    /// camera-dependent CSM/atlas UBOs). Bound at resolve binding 18. UNREAD at I0.
    pub ddgi_grid_ubo: &'a BoundBuffer,
    /// SDFDDGI I2: the probe-update compute pass activation. `None` = the OFF path (the DEFAULT —
    /// the GI-OFF 0%-gate): NO update RDG pass / set-write / dispatch / barrier is recorded, so the
    /// command stream is BYTE-IDENTICAL to the pre-I2 path (the grand_showcase golden). `Some(_)` is
    /// populated by `boyko_render`'s activation-populate system ONLY when `ResolvedDdgi::enabled()`:
    /// [`GBufferTargets`] writes the single 7-binding `ddgi_update_set`, and AFTER the marcher + the
    /// L0 light-table copy, BEFORE the resolve, the recorder records the `ddgi_update` RDG pass +
    /// binds + dispatches. The update→resolve atlas barrier is DERIVED BY THE RDG at the resolve.
    pub ddgi_update: Option<DdgiUpdateActivation<'a>>,
    /// SDFDDGI I2: the per-probe CLASSIFICATION storage buffer (1 u32/probe — `DdgiAtlas::
    /// classification()`), bound at the update set @3 (RW) and named as an RDG READ resource so the
    /// derived barrier chain covers it. ALWAYS supplied (bound only when the update set is written,
    /// i.e. `ddgi_update.is_some()`). Its handle rides the scene so the sink can resolve it.
    pub ddgi_classification: &'a BoundBuffer,
    /// SDFDDGI I2: the boot-static Fibonacci RAY-TABLE storage buffer (`rays_per_probe` `float4`s —
    /// `DdgiUpdateResources`), bound at the update set @4 (R). ALWAYS supplied; used only on the
    /// update ON path. Boot-uploaded ONCE (identity ray-rotation at I2), so it is non-ringed.
    pub ddgi_ray_table: &'a BoundBuffer,
    /// SDFDDGI I2: the update-pass parameter UBO (`DdgiUpdateUbo` — `DdgiUpdateResources`), bound at
    /// the update set @6. Host-written when the activation is populated (grid origin/spacing/dims +
    /// subset + ray/light counts). ALWAYS supplied; read only on the update ON path. Non-ringed (I2
    /// ships identity ray-rotation → the UBO is effectively static, plan §2.3/§7).
    pub ddgi_update_ubo: &'a BoundBuffer,
    /// Shadow Phase 5 Inc-1-GPU: the sparse spot/point DEPTH-PASS activation. `None` = the OFF path
    /// (the default, byte-identical command stream): NO depth pass is recorded, the resolve's
    /// `punctual_shadow_mode` header gate is 0, and the always-bound atlas map/sampler/UBO are
    /// bound-but-unread. `Some(_)` = the ON path: BEFORE the resolve dispatch the recorder runs the
    /// punctual depth pass — barriers the atlas image, LOOPS the `[0..active_layers)` slots,
    /// rendering the SAME caster batches ([`Self::mesh_draw`] + [`Self::instance_bind_group`],
    /// build-once-consume-N-views) into `layer_render_view(s)` with slot `s`'s `view_proj` pushed,
    /// then barriers the whole array to `SHADER_READ_ONLY_OPTIMAL` for the resolve sample. The caller
    /// MUST set the scene's light-header `punctual_shadow_mode` in lock-step
    /// (`with_punctual_shadow_mode(true)`).
    pub atlas_punctual: Option<PunctualDepthActivation<'a>>,
    /// Pillar B increment B3: the per-instance TRS interpolation compute PRE-PASS activation.
    /// `None` = the OFF path (the default for every dump/offscreen scene, byte-identical
    /// command stream): NO interp dispatch, NO interp barrier is recorded, and the raster +
    /// shadow vertex shaders read the caller-supplied [`Self::instance_bind_group`] (the
    /// legacy hand-affine or M3-gather SSBO) unchanged. `Some(_)` = the ON path: BEFORE the
    /// raster pass the recorder binds the interp pipeline + the activation's current-slot set,
    /// dispatches `ceil(instance_count / LOCAL_SIZE_X)` groups (interpolating each entity's
    /// prev/curr pair at `alpha` into the draw SSBO), and the graph derives the COMPUTE→VERTEX
    /// RAW barrier so the raster/CSM/atlas VS reads the freshly interpolated columns. When
    /// `Some`, the caller MUST set [`Self::instance_bind_group`] to the SAME current frame
    /// slot's draw-SSBO bind group the activation's set writes at binding 1 (see
    /// [`InterpActivation`]).
    pub interp: Option<InterpActivation<'a>>,
    /// HW-RT rung R0: the optional GPU timestamp bracket collector. `None` on EVERY
    /// golden/host frame (the DEFAULT — the capability-as-presence discipline) ⇒ the recorder
    /// emits ZERO reset/write commands, so the recorded command stream is BYTE-IDENTICAL to
    /// the pre-R0 path (proven by the framegraph byte-identity golden + the grand_showcase
    /// pixel dump, both run with `None`). `Some(tc)` (the offline `software_ray_baseline_cost`
    /// harness) brackets the four software-ray passes (DDGI update, deferred resolve, CSM
    /// cascade depth, punctual atlas depth) so the harness reports per-pass GPU wall-clock.
    /// The `is_some()` branch is COLD + perfectly predicted; a runtime `Option`, NOT a cargo
    /// feature (a feature would risk the timed build diverging from the shipped pipeline the
    /// calibration must measure).
    pub gpu_timing: Option<&'a TimestampCollector>,
    /// HW-RT rung R2a-3: the optional GPU-resident per-frame TLAS pack + build activation. `None`
    /// on EVERY golden/host frame (the DEFAULT — capability-as-presence) ⇒ NO pack dispatch, NO
    /// build, NO barrier, so the recorded command stream is BYTE-IDENTICAL to the pre-R2a-3 path
    /// (and the `tlas_instances` framegraph resource routes zero barriers). `Some(_)` (armed under
    /// hwrt + ray_query + a non-empty gather) runs the pack pre-pass + the TLAS build BEFORE the
    /// raster pass; nothing traces the TLAS yet (R2a-4), so the render stays byte-identical even
    /// when armed. The whole field is `#[cfg(feature = "hwrt")]`-gated, so a `not(hwrt)` build has
    /// this field absent entirely (no ResId shift, no OFF-path change).
    #[cfg(feature = "hwrt")]
    pub tlas: Option<TlasBuildActivation<'a>>,
    /// HW-RT rung 3a: the spatial (à-trous) RT soft-shadow DENOISE activation. `None` on EVERY
    /// golden/host frame (the DEFAULT — rung-3a steps 4-6 keep the host gate a literal `None`; step
    /// 7 flips it to `Some` under `mode == Spatial && backend == HardwareTri && has_primary_directional
    /// && tlas_nonempty`) ⇒ NO VIS pre-pass, NO à-trous passes, and the resolve binds the
    /// RESOLVE_INLINE-hwrt pipeline (`resolve_pipeline_hwrt`) ⇒ the command stream is BYTE-IDENTICAL
    /// to the pre-rung-3a path. `Some(_)` records the VIS pre-pass (writing `gShadowVis`) + the
    /// `levels` à-trous passes (ping-ponging `shadow_vis`/`shadow_vis2`), and the resolve binds the
    /// DENOISED pipeline reading the filtered visibility. `Some` REQUIRES the scene to also wire
    /// [`Self::resolve_pipeline_hwrt`] (the à-trous stack sits under the same hwrt+RT capability
    /// gate) and the targets to carry the `shadow_vis`/`shadow_vis2` rings (device
    /// `shadow_denoise_storage_ok()`). The whole field is `#[cfg(feature = "hwrt")]`, so a
    /// `not(hwrt)` build has it absent entirely.
    #[cfg(feature = "hwrt")]
    pub shadow: Option<ShadowVisActivation<'a>>,
    /// HW-RT rung 3a: the STABLE 22-binding VIS/DENOISED resolve bind-group LAYOUT (the same layout
    /// [`ShadowVisActivation::resolve_layout`] carries when the per-frame gate opens). Populated from
    /// the boot VIS/DENOISED pipelines REGARDLESS of the per-frame [`Self::shadow`] activation —
    /// `Some` on EVERY frame the boot denoise pipelines exist (an RT + `hwrt` device), including the
    /// create frame where [`Self::shadow`] is still `None`. Threaded so
    /// [`GBufferTargets::build_shadow_denoise_sets`](crate::present::targets) can write the resolve
    /// sets ONCE per extent decoupled from the per-frame activation (the create frame's `shadow ==
    /// None` no longer starves the set build → no record-time `None`-set panic). `None` on the
    /// software / non-RT path (no denoise sets built). Mirrors [`Self::resolve_layout_hwrt`]'s
    /// stable-populate shape.
    #[cfg(feature = "hwrt")]
    pub resolve_layout_denoise_hwrt: Option<&'a VulkanBindGroupLayout>,
    /// HW-RT rung 3a: the STABLE 6-binding à-trous bind-group LAYOUT (the same layout
    /// [`ShadowVisActivation::atrous_layout`] carries when the per-frame gate opens). Populated from
    /// the boot à-trous pipeline REGARDLESS of the per-frame [`Self::shadow`] activation — `Some`
    /// whenever [`Self::resolve_layout_denoise_hwrt`] is `Some` (they are built in lock-step at
    /// boot). Threaded so [`GBufferTargets::build_shadow_denoise_sets`](crate::present::targets) can
    /// write the per-level à-trous sets at create without the per-frame activation. `None` on the
    /// software / non-RT path.
    #[cfg(feature = "hwrt")]
    pub atrous_layout_denoise_hwrt: Option<&'a VulkanBindGroupLayout>,
    /// HW-RT rung 3a/3b: `true` iff the denoise is ARMED — `spatial_enabled() || temporal_enabled()`
    /// (the runner's `denoise_armed`). Rung 3a used the spatial-only predicate; Rung 3b widened it so
    /// the Temporal-only mode (spatial off, temporal on) still builds the VIS set + the temporal sets.
    /// The STABLE "denoise is on" signal used at CREATE time to gate the resolve/à-trous/temporal SET
    /// build — it does NOT depend on the per-frame [`Self::shadow`] activation (which is still `None`
    /// on the create frame), so the sets get built before the render frame flips the activation on.
    /// Kept in sync across frames (a live config read). `false` on the default (mode `None`) path ⇒ NO
    /// sets built ⇒ byte-identical. For Spatial the value is unchanged from 3a (`true`).
    #[cfg(feature = "hwrt")]
    pub shadow_denoise_enabled: bool,
    /// HW-RT rung 3a/3b: the STABLE `atrous_levels % 2 == 1` parity — whether the FINAL à-trous
    /// output lands in `shadow_vis2` (odd count) vs `shadow_vis` (even count, incl. Temporal-only's
    /// `atrous_levels == 0` ⇒ `shadow_vis` = the raw VIS). Derived from the runner's `atrous_levels`
    /// (`spatial ? clamped_levels() : 0` — the SAME parity the record + graph +
    /// [`ShadowVisActivation::final_is_vis2`] use — W1 consistency), threaded stably so the DENOISED
    /// resolve set binds `gShadowVis` @21 + the temporal set binds `gVisIn` @0 to the correct final
    /// ring at CREATE time, independent of the per-frame [`Self::shadow`] activation. When the
    /// activation IS present, it MUST equal [`ShadowVisActivation::final_is_vis2`] (asserted at the
    /// set-build site).
    #[cfg(feature = "hwrt")]
    pub shadow_denoise_final_is_vis2: bool,
    /// HW-RT Rung 3b step 5a: `true` iff the author's `ShadowDenoiseConfig.mode ∈ {Temporal, Both}`
    /// (the runner's `ShadowDenoiseConfig::temporal_enabled()` read) — the per-frame gate that
    /// swaps the raster pass to the MESH motion-vector pipeline (a 4th MRT writing Δuv). `false` on
    /// the default (`None`/`Spatial`) path ⇒ the base 3-MRT raster draws ⇒ byte-identical. Combined
    /// with [`Self::raster_pipeline_mv`]/[`Self::mv_bind_group`] being `Some` (an RT + storage
    /// device); when any is absent the recorder takes the base path. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub temporal_enabled: bool,
    /// HW-RT Rung 3b step 5a: the MESH motion-vector raster pipeline (`gbuffer_mrt_mv.{vs,fs}`) —
    /// carries its own `.pipeline` + `.layout` (the 3-binding set-0 layout: current @0 / prev @1 /
    /// motion-cam @2). `Some` on an RT + storage device; `None` otherwise (the recorder takes the
    /// base [`Self::raster_pipeline`]). Bound instead of the base ONLY when [`Self::temporal_enabled`]
    /// AND this is `Some` AND [`Self::mv_bind_group`] is `Some`. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub raster_pipeline_mv: Option<&'a VulkanGraphicsPipeline>,
    /// HW-RT Rung 3b step 5a: this frame's motion-vector set-0 bind group (slot `frame_index` of the
    /// MV resources' per-FIF bind groups: `{ instances[i], prev_instances[i], motion_cam[i] }`).
    /// Bound at set 0 when the MV pipeline is selected. `Some` iff [`Self::raster_pipeline_mv`] is
    /// `Some`. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub mv_bind_group: Option<&'a VulkanBindGroup>,
    /// HW-RT Rung 3b step 5b: the SDF motion-vector VIS-variant resolve pipeline
    /// (`deferred_pbr_hwrt_vis_mv.comp`) — writes `gShadowVis` @21 (like the base VIS) AND each SDF
    /// pixel's camera-only `Δuv` to `motion_vec` @23. Bound instead of
    /// [`ShadowVisActivation::vis_pipeline`] in the VIS pass ONLY when [`Self::sdf_mv_active`] (and
    /// the VIS pass runs, i.e. `Self::shadow.is_some()`). The recorder's ref — `Some` only on a
    /// temporal frame with the MV resources (mirrors [`Self::raster_pipeline_mv`]).
    /// `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub vis_mv_pipeline: Option<&'a ComputePipeline>,
    /// HW-RT Rung 3b step 5b: the STABLE 24-binding VIS-MV resolve bind-group LAYOUT (the 22-binding
    /// VIS/DENOISED layout + the `MotionCam` UBO @22 + the `motion_vec` STORAGE image @23). Populated
    /// whenever the boot MV resources exist (an RT + storage device), REGARDLESS of the per-frame
    /// temporal gate (mirrors [`Self::resolve_layout_denoise_hwrt`]) — so
    /// [`GBufferTargets::build_shadow_vis_mv_resolve_set`](crate::present::targets) can write the
    /// per-FIF VIS-MV set ONCE per extent decoupled from the activation. `None` on a non-storage /
    /// non-hwrt device. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub vis_mv_layout: Option<&'a VulkanBindGroupLayout>,
    /// HW-RT Rung 3b step 5b: the STABLE `MotionCam` UBO ring the VIS-MV set binds @22 (the runner
    /// uploads `MotionCam` into slot `frame_index` under the same temporal gate that feeds the mesh MV
    /// pass). Populated whenever the boot MV resources exist, like [`Self::vis_mv_layout`]; the VIS-MV
    /// set-build reads slot `fi`. `None` on a non-storage / non-hwrt device. `#[cfg(feature =
    /// "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub motion_cam_ubo_ring: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// HW-RT Rung 3b step 6: the STABLE 8-binding temporal reproject bind-group LAYOUT (the layout
    /// [`ShadowVisActivation::temporal_pipeline`] declares: `gVisIn` @0 / `gMotionVec` @1 / `gViewT`
    /// @2 / `gHistIn` @3 / `gHistOut` @4 / `gTemporalOut` @5 STORAGE images + the `ResolvedTemporalShadow`
    /// UBO @6 + the camera UBO @7). Populated from the boot temporal pipeline REGARDLESS of the
    /// per-frame temporal gate (mirrors [`Self::resolve_layout_denoise_hwrt`]), so
    /// [`GBufferTargets::build_shadow_temporal_sets`](crate::present::targets) can write the per-FIF
    /// temporal set ONCE per extent decoupled from the activation. `None` on a non-RT / non-hwrt
    /// device. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub temporal_layout: Option<&'a VulkanBindGroupLayout>,
    /// Asset-streaming plan F8: `true` iff THIS frame's gather scattered any non-default
    /// material id (`MeshRenderScratch::any_non_default_material`) — the per-frame
    /// `PER_INSTANCE_MATERIAL` raster-pipeline selection gate. NOT `#[cfg(feature =
    /// "hwrt")]` — materials are device-agnostic (the 2-material golden runs on the
    /// software leg). `false` on every all-default scene (the goldens) ⇒ the recorder
    /// binds the FROZEN base pipeline (byte-identity by construction).
    pub pm_enabled: bool,
    /// Asset-streaming plan F8: the PER_INSTANCE_MATERIAL gbuffer producer pipeline
    /// (`gbuffer_mrt_pm.{vs,fs}`) — built UNCONDITIONALLY at boot (unlike `mv`, this is
    /// not RT-specific). `Some` iff [`Self::pm_enabled`] (belt-and-suspenders); bound
    /// instead of [`Self::raster_pipeline`] ONLY when [`Self::pm_enabled`] AND this is
    /// `Some` AND [`Self::pm_bind_group`] is `Some`, AND no MV frame is active (MV takes
    /// priority, F8 §2.3).
    pub raster_pipeline_pm: Option<&'a VulkanGraphicsPipeline>,
    /// Asset-streaming plan F8: this frame's PM set-0 bind group (slot `frame_index` of
    /// the PM resources' per-FIF bind groups: `{ instance_rings[i] @0,
    /// pm_instance_material_rings[i] @1 }`). Bound at set 0 when the PM pipeline is
    /// selected. `Some` iff [`Self::pm_enabled`].
    pub pm_bind_group: Option<&'a VulkanBindGroup>,
}

impl GBufferScene<'_> {
    /// HW-RT Rung 3b step 5a — the SINGLE source of the "the raster pass writes the mesh
    /// motion-vector 4th MRT this frame" decision, so the framegraph barrier declaration
    /// (`declare_gbuffer_graph`) and the draw recording (`record_gbuffer`) can never diverge.
    ///
    /// True iff temporal is on AND the MV pipeline + this frame's MV bind group both exist (an
    /// RT + RG16-storage device built them at boot). The pipeline/bind-group presence is NOT
    /// implied by `temporal_enabled` alone: a device with RG16 storage but no ray-query (e.g.
    /// `BOYKO_FORCE_SOFTWARE=1`) allocates the `motion_vec` target yet builds no MV pipeline, so
    /// gating the barrier on `temporal_enabled` alone would declare a write the recorder never
    /// emits. Both call sites MUST use this method.
    #[cfg(feature = "hwrt")]
    pub(crate) fn mesh_mv_active(&self) -> bool {
        self.temporal_enabled && self.raster_pipeline_mv.is_some() && self.mv_bind_group.is_some()
    }

    /// Asset-streaming plan F8 — the SINGLE source of the "this frame draws with the
    /// PER_INSTANCE_MATERIAL pipeline" decision. NOT `#[cfg(feature = "hwrt")]` — PM works
    /// on the software leg (the 2-material golden). MV takes priority over PM at the
    /// recorder's selection site (F8 §2.3): a temporal frame with the MV pipeline active
    /// renders default materials (the tracked F8-mv follow-up), never a crash.
    pub(crate) fn mesh_pm_active(&self) -> bool {
        self.pm_enabled && self.raster_pipeline_pm.is_some() && self.pm_bind_group.is_some()
    }

    /// HW-RT Rung 3b step 5b — the SINGLE source of the "the VIS pass ALSO writes the SDF pixels'
    /// camera-only motion vector to `motion_vec` this frame" decision, so the framegraph barrier
    /// declaration (`declare_gbuffer_graph`) and the VIS-pass recording (`record_gbuffer`) can never
    /// diverge (the W1 lesson, mirroring [`Self::mesh_mv_active`]).
    ///
    /// True iff temporal is on AND the VIS-MV pipeline + its build-time inputs (the 24-binding
    /// layout + the `MotionCam` UBO ring) all exist (an RT + RG16-storage device built them at boot).
    /// The pipeline/layout/ring presence is NOT implied by `temporal_enabled` alone: a device with
    /// RG16 storage but no ray-query (e.g. `BOYKO_FORCE_SOFTWARE=1`) allocates the `motion_vec`
    /// target yet builds no VIS-MV pipeline, so gating on `temporal_enabled` alone would declare a
    /// STORAGE write the recorder never emits.
    ///
    /// NOTE: the actual SDF-MV write only happens when the VIS pass runs (`self.shadow.is_some()`,
    /// i.e. `mode == Both` this rung — spatial gates the VIS pass on). Both the graph declaration and
    /// the recording sit INSIDE the `self.shadow.is_some()` branch and additionally gate on this
    /// method, so the effective predicate is `self.shadow.is_some() && self.sdf_mv_active()`.
    #[cfg(feature = "hwrt")]
    pub(crate) fn sdf_mv_active(&self) -> bool {
        self.temporal_enabled
            && self.vis_mv_pipeline.is_some()
            && self.vis_mv_layout.is_some()
            && self.motion_cam_ubo_ring.is_some()
    }

    /// HW-RT Rung 3b step 6 — the SINGLE source of the "the temporal reproject+accumulate pass runs
    /// this frame" decision, so the framegraph declaration (`declare_gbuffer_graph`: the temporal
    /// pass + the resolve's `temporal_out`-vs-à-trous read) and the recording (`record_gbuffer`: the
    /// temporal dispatch + the DENOISED resolve set selection) can never diverge (the W1 lesson,
    /// mirroring [`Self::mesh_mv_active`] / [`Self::sdf_mv_active`]).
    ///
    /// True iff the spatial/temporal denoise arm opened this frame (`self.shadow.is_some()`, so the
    /// VIS pass produced the input) AND the author's mode is temporal (`ShadowVisActivation::temporal`).
    /// The physical temporal sets/pipeline presence is a STRICT SUPERSET the recorder additionally
    /// checks (degrade-graceful), exactly as the à-trous recorder re-checks its pre-built sets.
    #[cfg(feature = "hwrt")]
    pub(crate) fn temporal_active(&self) -> bool {
        self.shadow.as_ref().is_some_and(|sh| sh.temporal)
    }
}

/// CSM Increment 1b (Rung A): the cascade DEPTH-PASS activation threaded into
/// [`GBufferScene::csm`] to turn the depth pass ON. Mirrors [`SsaoActivation`]'s borrow-bundle
/// shape: a per-frame `Copy` pair the caller flips between frames with no re-record.
///
/// The casters are the SAME instanced [`GBufferMeshDraw`] batches the main pass rasterizes
/// ([`GBufferScene::mesh_draw`]) — the depth pass renders them from the SUN's point of view into
/// the cascade's depth layer. A real app would gather a `With<ShadowCaster>` subset; the inline
/// demo reuses the full mesh batch list (its single box is the caster).
#[derive(Clone, Copy)]
pub struct CsmDepthActivation<'a> {
    /// The depth-only graphics pipeline (`csm_depth.vs/fs`): EMPTY `color_formats`, `depth_format
    /// = Some(D32Sfloat)`, `cull_mode: Front`, `depth_bias: Some(slope/constant)`, the set-0
    /// instance SSBO layout. Bound by [`record_csm_depth`].
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// The 88-byte VERTEX push TEMPLATE for the depth pass: the trailing words (`use_model_matrix
    /// == 1` `@84`; the recorder overwrites the `base_instance` word `@80` per caster batch). The
    /// leading 64 bytes (the `view_proj` matrix) are OVERWRITTEN per cascade from
    /// [`Self::cascade_view_proj`] — so this template carries the NON-matrix words only (its
    /// `@0..64` are unused on the depth path, stamped over each iteration).
    pub push: [u8; GBUFFER_PUSH_BYTES],
    /// CSM Increment 3 (Rung B): the per-cascade COLUMN-MAJOR `view_proj` matrices (64 bytes each),
    /// `[0..active_count)` valid. The depth pass loops cascades, stamping `cascade_view_proj[c]`
    /// into the push's leading 64 bytes and rendering the casters into `layer_render_view(c)`. Each
    /// matrix MUST byte-equal the resolve UBO's `gCascades[c].view_proj` (the O1 single-matrix pin:
    /// the depth VS + the resolve read the SAME per-cascade matrix).
    pub cascade_view_proj: [[u8; 64]; MAX_CASCADES],
    /// CSM Increment 3 (Rung B): the number of cascades to render (`1..=MAX_CASCADES`); mirrors the
    /// cascade UBO's `active_count` / the scene's CSM activation. Rung A is `1`; Rung B is N. The
    /// depth pass renders layers `[0..active_count)`; the resolve SELECTs among the same set.
    pub active_count: u32,
    /// The cascade shadow-map resolution (texels per side — the map is square, e.g. 2048). The
    /// depth pass's render area + viewport. MUST equal the resolution
    /// [`GBufferScene::csm_cascade_texture`] was created at.
    pub shadow_dim: u32,
}

/// Shadow Phase 5 Inc-1-GPU: the sparse SPOT/POINT atlas DEPTH-PASS activation threaded into
/// [`GBufferScene::atlas_punctual`] to turn the punctual depth pass ON. Mirrors
/// [`CsmDepthActivation`]'s borrow-bundle shape EXACTLY (the depth-only pipeline + push template +
/// per-layer matrices + the layer count); the ONLY difference is the layer budget
/// ([`MAX_TEXTURE_LAYERS`] = 16, the atlas's `M_SLOTS`, vs `MAX_CASCADES` = 4 for CSM).
///
/// The casters are the SAME instanced [`GBufferMeshDraw`] batches the main pass rasterizes
/// ([`GBufferScene::mesh_draw`]) — the depth pass renders them from each spot's point of view into
/// the atlas's depth layer. A real app would gather a `With<ShadowCaster>` subset; the inline demo
/// reuses the full mesh batch list.
#[derive(Clone, Copy)]
pub struct PunctualDepthActivation<'a> {
    /// The SPOT depth-only graphics pipeline (`csm_depth.vs/fs` VERBATIM — SPOT uses NDC-z like a
    /// CSM cascade, so the cascade depth pipeline works unchanged): EMPTY `color_formats`,
    /// `depth_format = Some(D32Sfloat)`, `cull_mode: Front`, `depth_bias: Some(slope/constant)`, the
    /// set-0 instance SSBO layout. Bound for SPOT-face layers (`face_is_point[s] == false`).
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// Shadow Phase 5 Inc-2 (POINT cube): the POINT depth-WRITE graphics pipeline
    /// (`punctual_depth.vs/fs`): the FS writes `SV_Depth = saturate(length(world - light_pos) *
    /// inv_range)` (the linear radial distance), so it is a SEPARATE pipeline from the SPOT one (the
    /// SPOT FS is empty NDC-z). Bound for POINT-face layers (`face_is_point[s] == true`); the layers
    /// are grouped so at most ONE bind of each pipeline is recorded.
    pub point_pipeline: &'a VulkanGraphicsPipeline,
    /// The 88-byte VERTEX push TEMPLATE for the depth pass: the trailing words (`use_model_matrix
    /// == 1` `@84`; the recorder overwrites the `base_instance` word `@80` per caster batch). The
    /// leading 64 bytes (the `view_proj` matrix) are OVERWRITTEN per atlas slot from
    /// [`Self::face_view_proj`]; the `cam_eye@64` lane (16 B) is OVERWRITTEN per POINT slot from
    /// [`Self::face_light`] (`light_pos.xyz` + `inv_range` in `.w`) and left as-is for SPOT slots.
    pub push: [u8; GBUFFER_PUSH_BYTES],
    /// The per-slot COLUMN-MAJOR `view_proj` matrices (64 bytes each), `[0..active_layers)` valid.
    /// The depth pass loops slots, stamping `face_view_proj[s]` into the push's leading 64 bytes and
    /// rendering the casters into `layer_render_view(s)`. Each matrix MUST byte-equal the resolve
    /// UBO's `gFaces[s].view_proj` (the O1 single-matrix pin: the depth VS + the resolve read the
    /// SAME per-slot matrix).
    pub face_view_proj: [[u8; 64]; MAX_TEXTURE_LAYERS],
    /// Shadow Phase 5 Inc-2 (POINT cube): per-layer TYPE — `true` = a POINT cube face (the
    /// `point_pipeline` + the `face_light` push), `false` = a SPOT face (the SPOT `pipeline`,
    /// `cam_eye` unused). The recorder GROUPS the active layers by this flag (spot-faces then
    /// point-faces, or vice versa) so it binds each pipeline at most once.
    pub face_is_point: [bool; MAX_TEXTURE_LAYERS],
    /// Shadow Phase 5 Inc-2 (POINT cube): per-POINT-face `cam_eye@64` push bytes — `light_pos.xyz`
    /// (12 B) + `inv_range` (4 B), 16 B. Stamped into the push's `@64..80` lane before a POINT-face
    /// layer renders, so the FS reads `cam_eye.xyz == light_pos` / `cam_eye.w == inv_range`. The six
    /// faces of one point share identical bytes (one cube center). Unused for SPOT-face layers.
    pub face_light: [[u8; 16]; MAX_TEXTURE_LAYERS],
    /// The number of atlas layers to render (`1..=MAX_TEXTURE_LAYERS`); mirrors the atlas UBO's
    /// `active_layers` / `ResolvedShadowAtlas::active_layers`. The depth pass renders layers
    /// `[0..active_layers)`.
    pub active_layers: u32,
    /// The shadow-atlas resolution (texels per side — the map is square, e.g. 512). The depth pass's
    /// render area + viewport. MUST equal the resolution [`GBufferScene::shadow_atlas_texture`] was
    /// created at.
    pub shadow_dim: u32,
}

