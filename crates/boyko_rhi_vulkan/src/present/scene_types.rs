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
use crate::geometry_bindless::VulkanGeometryBindlessSet;
use crate::rhi_impl::{
    ComputePipeline, VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline, VulkanSampler,
};
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS, VulkanTexture};

use super::gpu_timing::{Sv0TimestampCollector, TimestampCollector, VbTimestampCollector};
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
            mip_levels: 1,
            view_format: None,
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
    /// gbuffer raster pipeline's `VERTEX_STRIDE`-wide stride, 64 bytes since `uv`\@40 /
    /// `tangent`\@48 were appended — this pipeline reads only the first 3 attributes).
    /// Bound at vertex binding 0 for pass A.
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
    /// VG rung R2c: this batch's WORLD-space AABB — the union of its instances' Arvo-transformed
    /// local boxes, computed host-side by `boyko_render::csm_caster::batch_world_aabb`.
    ///
    /// `None` means "bounds unavailable" (the mesh has not resolved `Loaded`, or it carries the C0
    /// zero-vertex sentinel), and the cull then KEEPS the batch: absence of bounds is not evidence
    /// of invisibility. The recorder writes [`VbBatchDesc::UNBOUNDED`] corners for a `None`, which
    /// survives every frustum plane — so the degraded path is the conservative one by construction
    /// rather than by a branch anyone has to remember.
    ///
    /// Host-computed on purpose: there is no per-instance mesh id on the GPU (the `mesh_ids` lane
    /// is host-side only), so a shader could not look these up; and computing them here means the
    /// cull's oracle and the shader read the SAME numbers, making any disagreement a shader bug
    /// rather than a math bug.
    pub world_aabb: Option<([f32; 3], [f32; 3])>,
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

/// VG rung R2c0: the batch-cull shader's `[numthreads(64,1,1)]` group width
/// (`vb_batch_cull.comp.hlsl`'s own `LOCAL_SIZE_X`). The dispatch is
/// `ceil(batch_count / VB_BATCH_CULL_LOCAL_SIZE_X)` groups; the tail group's out-of-range lanes
/// are trimmed by the shader's own `i >= pc.batch_count` guard.
pub(crate) const VB_BATCH_CULL_LOCAL_SIZE_X: u32 = crate::compute::VB_BATCH_CULL_LOCAL_SIZE_X;

/// VG rung R2c0: the batch-cull pipeline's COMPUTE push range (`{ batch_count, visible_cap }`,
/// `vb_batch_cull.comp.hlsl`'s `VbBatchCullPush`). Re-exported from `compute` so this field-decl
/// site does not depend on it — the SAME idiom [`CLUSTER_CULL_PUSH_BYTES`] follows.
pub(crate) const VB_BATCH_CULL_PUSH_BYTES: u32 = crate::compute::VB_BATCH_CULL_PUSH_BYTES;

/// VG rung R2c0: one batch's cull inputs, as `vb_batch_cull.comp.hlsl`'s `VbBatchDescGpu` reads
/// them. 32 bytes; `[f32; 3]`-then-`u32` twice, so both 16-byte halves are naturally packed and
/// the struct needs no explicit padding member beyond the reserved `pad` word.
///
/// # Why the AABB is here at rung R2c0, where nothing reads it
///
/// The DECISION is what rung R2c adds; the LAYOUT is what it needs. Fixing the record now means
/// R2c touches the shader body and this struct's fill, and leaves the descriptor-set layout, the
/// buffer sizes and the framegraph alone.
///
/// [`Self::UNBOUNDED`] is the rung-R2c0 corner value and the CONSERVATIVE one: an unfilled batch
/// survives every plane test, so the only reachable error direction is a wasted draw — never a
/// false cull. See that constant's own doc for why it is finite.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VbBatchDesc {
    /// World-space AABB min corner. `-`[`Self::UNBOUNDED`] when the host could not compute real
    /// bounds. (Rung R2c0 wrote the sentinel unconditionally and the shader dead-coded the field;
    /// rung R2c fills it for real and reads it.)
    pub aabb_min: [f32; 3],
    /// The `instanceCount` a VISIBLE batch draws — the SAME value the transfer fill already put in
    /// this batch's `VkDrawIndexedIndirectCommand`. A culled batch gets `0` written over it
    /// instead; at rung R2c0 nothing was ever culled, which is what made that rung inert.
    pub instance_count: u32,
    /// World-space AABB max corner. Rung R2c0 writes `+`[`Self::UNBOUNDED`].
    pub aabb_max: [f32; 3],
    /// Reserved (rung R2c: the plane-set / batch-flags word). Written zero.
    pub pad: u32,
}

impl VbBatchDesc {
    /// The rung-R2c0 "unbounded box" corner magnitude — a large FINITE value, never an infinity.
    ///
    /// A frustum plane evaluated at an infinite corner (`dot(n, p) + d` with a zero normal
    /// component) yields a NaN, and a NaN does not propagate through `NMin`/`NMax`: it silently
    /// selects the OTHER operand, so a NaN-poisoned test can read as *culled* rather than as
    /// obviously broken. A finite magnitude keeps every plane evaluation finite and correctly
    /// signed while still exceeding any world extent this engine renders.
    pub const UNBOUNDED: f32 = 1.0e30;

    /// VG rung R2c: the descriptor for a batch whose world AABB the host COULD compute.
    ///
    /// No validation of `min <= max` here on purpose: an inverted box is already filtered upstream
    /// by `batch_world_aabb` (it returns `None` for the C0 zero-vertex sentinel), and re-checking
    /// it here would either duplicate that rule or quietly disagree with it.
    #[inline]
    #[must_use]
    pub const fn bounded(instance_count: u32, aabb_min: [f32; 3], aabb_max: [f32; 3]) -> Self {
        Self { aabb_min, instance_count, aabb_max, pad: 0 }
    }

    /// The rung-R2c0 descriptor for a batch of `instance_count` instances: an unbounded box, so
    /// the cull cannot reject it once rung R2c arms the test.
    #[inline]
    #[must_use]
    pub const fn unbounded(instance_count: u32) -> Self {
        Self {
            aabb_min: [-Self::UNBOUNDED; 3],
            instance_count,
            aabb_max: [Self::UNBOUNDED; 3],
            pad: 0,
        }
    }
}

/// VG rung R2c: the batch-cull's push constants — `vb_batch_cull.comp.hlsl`'s `VbBatchCullPush`.
///
/// `float4 planes[6]` occupies the leading 96 bytes (std430 push layout: a `float4` array needs no
/// interior padding), then the two counts. 104 bytes total, inside Vulkan's guaranteed 128-byte
/// `maxPushConstantsSize` minimum.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VbBatchCullPush {
    /// The six frustum planes, `(a, b, c, d)`, inside ⇒ `a·x + b·y + c·z + d ≥ 0`, in the order
    /// left, right, bottom, top, near, far.
    pub planes: [[f32; 4]; 6],
    /// Live `DrawBatch` count this frame — the shader's tail-lane range guard.
    pub batch_count: u32,
    /// `VbCullVisible`'s element capacity — the clamp-and-drop bound, derived from the ALLOCATION.
    pub visible_cap: u32,
}

impl VbBatchCullPush {
    /// The DISARMED plane set: every plane `(0, 0, 0, 0)`, so `dist + radius == 0.0` and the
    /// `< 0.0` rejection never fires — every batch survives.
    ///
    /// This is what a frame with no `vb_cull_planes` pushes, and it degrades the cull to rung
    /// R2c0's null control EXACTLY rather than to something merely similar: a zeroed plane cannot
    /// reject, whatever the box. Zero is the right disarm value here precisely because the planes
    /// are unnormalised — there is no normalisation step to turn it into a NaN.
    pub const DISARMED_PLANES: [[f32; 4]; 6] = [[0.0; 4]; 6];
}

const _: () = assert!(
    core::mem::size_of::<VbBatchCullPush>() == VB_BATCH_CULL_PUSH_BYTES as usize,
    "rung R2c: VbBatchCullPush must match the shader's 104-byte push block"
);

/// The [`VbBatchDesc`] byte stride — mirrors `vb_batch_cull.comp.hlsl`'s `VbBatchDescGpu`.
pub(crate) const VB_BATCH_DESC_STRIDE: u32 = crate::compute::VB_BATCH_DESC_STRIDE;

const _: () = assert!(
    core::mem::size_of::<VbBatchDesc>() == VB_BATCH_DESC_STRIDE as usize,
    "rung R2c0: VbBatchDesc must match the shader's 32-byte VbBatchDescGpu stride"
);

/// VB-P1e D11: the hierarchical cull pipeline's COMPUTE push range size (24 B
/// [`crate::compute::ClusterCullHierPush`]). Re-exported so [`ClusterCullHierDispatch::push`]
/// can size its inline byte array without depending on `compute` at the field-decl site.
pub(crate) const CLUSTER_CULL_HIER_PUSH_BYTES: u32 = crate::compute::CLUSTER_CULL_HIER_PUSH_BYTES;

/// VB-P1e D11/H4: the hierarchical cull's per-frame dispatch record — `Some` IFF
/// [`GBufferScene::cluster_cull`] holds the `-D HIER=1` pipeline instead of the base 64-wide
/// arm (`GpuSceneBundles::build_froxel_light_cull` builds exactly ONE of the two pipelines per
/// boot and stores it there; this struct is metadata about WHICH one, never a second pipeline
/// slot — Principle 0: one derived accessor + one activation struct, no side store).
///
/// `groups` is the host-derived 256-wide dispatch count
/// (`boyko_render::ClusterConfig::hier_group_count` — a dev-only back-edge dependency, not
/// doc-linkable from this crate); `push` is the 24-byte
/// [`crate::compute::ClusterCullHierPush`] bytes (the base push fields plus D11's BOOT
/// snapshot: the packed dims + the full-precision `cluster_count()`). `#[repr(C)]`-free — this
/// is a host-side record, not a device buffer layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterCullHierDispatch {
    /// The 256-wide dispatch group count (`ceil(dim_x * dim_y / 256) * dim_z`).
    pub groups: u32,
    /// The 24-byte [`crate::compute::ClusterCullHierPush`] bytes this dispatch pushes.
    pub push: [u8; CLUSTER_CULL_HIER_PUSH_BYTES as usize],
}

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
    /// The SSAO edge-avoiding à-trous denoise chain: the CLAMPED number of passes to dispatch
    /// THIS frame — `0` (denoise off, the resolve reads the raw gather) or
    /// `2..=`[`crate::present::MAX_SSAO_ATROUS_LEVELS`] (`boyko_render::SsaoConfig::clamped_atrous_levels`'s
    /// contract, threaded by the host). The chain's pipelines/layout/sets are STABLE
    /// [`GBufferScene`] fields (`ssao_atrous_read8_pipeline` etc.) — DECOUPLED from THIS per-frame
    /// activation, mirroring `ShadowVisActivation`'s "the set-builder reads the stable boot
    /// signals, not the per-frame gate" discipline (the "set=None panic when the gate opens late"
    /// trap `build_shadow_denoise_sets`'s doc names). `0` records NO à-trous pass (byte-identical
    /// to the pre-dispatch-wiring path) regardless of whether the device supports the interior
    /// ring format — a SEPARATE 0%-gate from that device-capability degrade (an owner-authored `0`
    /// vs a device lacking `R16_UNORM` storage are two different reasons for the same "no à-trous
    /// pass" outcome; the recorder checks [`GBufferTargets`]'s role-keyed sets for the latter).
    pub atrous_levels: u32,
}

/// Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off) —
/// threaded into [`GBufferScene::viewt_from_depth`] to turn the `viewt_from_depth`
/// `gViewT`-producer pass ON. The R3 audit (`sdf_gbuffer_composite.hlsl`, both terminal
/// `gViewT` write sites) found the SDF marcher is the SOLE writer of `gViewT` even for
/// MESH-owned pixels; under `Deferred × Mesh` the marcher is not dispatched at all
/// (`GBufferScene::marcher` pass is `None`), so nothing writes `gViewT` for the resolve's `P =
/// ro + rd*view_t` reconstruction / SSAO's mesh/SDF classification. This pass reproduces JUST
/// the marcher's mesh-depth → `gViewT` conversion (`mesh_norm`/`t_mesh`, byte-for-byte) for
/// every pixel.
///
/// `None` on [`GBufferScene::viewt_from_depth`] is the OFF path (the DEFAULT — every leg except
/// `Deferred × Mesh`, the 0%-gate): no `viewt_from_depth_set`, no framegraph pass, no dispatch —
/// capability = component presence, not a runtime flag. `Some` arms the whole seam; the caller
/// MUST supply it exactly when the resolved leg set is `GeometryLegs::Mesh`
/// (`mesh_leg && !sdf_leg`) — `declare_deferred_graph` (`graph_bridge.rs`) carries a
/// `debug_assert!` belt-and-braces check the seam was not missed (mirrors the R3
/// mesh-shadow-producer invariant guards).
///
/// `#[derive(Clone, Copy)]` — a pair of borrows + a scalar the caller flips between frames with
/// no re-record, mirroring [`SsaoActivation`]'s shape.
#[derive(Clone, Copy)]
pub struct ViewtFromDepthActivation<'a> {
    /// The `viewt_from_depth` compute pipeline (`viewt_from_depth.comp.hlsl` /
    /// [`crate::compute::viewt_from_depth_spirv`]): its layout declares [`Self::layout`] at
    /// `set 0` + the 12-byte [`crate::compute::ViewtFromDepthPush`] COMPUTE push range. The
    /// caller OWNS the pipeline + layout and tears them down; the `'a` lifetime ties this
    /// activation to those borrows for the frame call — mirrors [`SsaoActivation::pipeline`].
    pub pipeline: &'a ComputePipeline,
    /// The DEDICATED 2-binding `viewt_from_depth` bind-group LAYOUT { SAMPLED depth @0, STORAGE
    /// `gViewT` @1 } — matching `viewt_from_depth.comp`'s set 0. [`GBufferTargets`] writes a
    /// `viewt_from_depth_set` against it once per extent (pointing at the per-extent depth +
    /// `gViewT` images), mirroring [`SsaoActivation::layout`].
    pub layout: &'a VulkanBindGroupLayout,
    /// The host-precomputed mesh-depth ray-t normalizer this frame's push carries
    /// (`boyko_render::gbuffer_depth::mesh_view_t_norm` — see
    /// [`crate::compute::ViewtFromDepthPush`]'s doc for why it is NOT re-derived in HLSL): the
    /// SAME value the marcher's own `mesh_norm` ternary would select for this frame's camera.
    pub mesh_view_t_norm: f32,
}

/// TAA-under-VB (`VisibilityBuffer × Mesh`): the `gViewT` producer activation threaded into
/// [`GBufferScene::viewt_from_vb_depth`] to turn the VB path's `viewt_from_depth_rz`
/// `gViewT`-producer pass ON — the REVERSE-Z sibling of [`ViewtFromDepthActivation`]
/// (`viewt_from_depth_rz.comp.hlsl` / [`crate::compute::viewt_from_depth_rz_spirv`]). Under
/// `VisibilityBuffer × Mesh` there is no marcher dispatch and no Deferred custom-linear depth to
/// decode: `vb_raster` writes a standard HARDWARE reverse-Z depth, so this pass inverts THAT
/// encode (`view_z = B / (d − A)`, the proven `sdf_forward_march` HAS_MESH decode) and
/// reparameterizes to the marcher ray metric, giving the UNCHANGED TAA resolve a real `gViewT`
/// lane to reconstruct `P = ro + rd·view_t` from.
///
/// Bound to a DEDICATED 3-binding layout { SAMPLED depth @0, STORAGE `gViewT` @1, UNIFORM camera
/// @2 } — one more binding than [`ViewtFromDepthActivation::layout`] because the reverse-Z ray
/// reparameterization needs `cam_forward`/`generate_ray`, unlike the Deferred custom-linear
/// decode. Pushes the 16-byte [`crate::compute::ViewtFromDepthRzPush`] (`img_w`, `img_h`,
/// `view_z_a`, `view_z_b`) — the coefficients MUST come from
/// `boyko_render::view::forward_view_z_coeffs(near, far)`, the single-sourced host mirror of
/// `forward_view_proj_rows`'s reverse-Z encode (never re-derived ad hoc at the call site).
///
/// `None` on [`GBufferScene::viewt_from_vb_depth`] is the OFF path (the DEFAULT — the 0%-gate):
/// no `viewt_from_vb_depth_set`, no framegraph pass, no dispatch — capability = component
/// presence, not a runtime flag. `Some` iff the caller resolved `VisibilityBuffer × Mesh` with
/// TAA armed ([`GBufferScene::taa`] `Some`) — mirrors [`ViewtFromDepthActivation`]'s arming
/// discipline.
///
/// `#[derive(Clone, Copy)]` — a pair of borrows + two scalars the caller flips between frames with
/// no re-record, mirroring [`ViewtFromDepthActivation`]'s shape.
#[derive(Clone, Copy)]
pub struct ViewtFromVbDepthActivation<'a> {
    /// The `viewt_from_depth_rz` compute pipeline (`viewt_from_depth_rz.comp.hlsl` /
    /// [`crate::compute::viewt_from_depth_rz_spirv`]): its layout declares [`Self::layout`] at
    /// `set 0` + the 16-byte [`crate::compute::ViewtFromDepthRzPush`] COMPUTE push range. The
    /// caller OWNS the pipeline + layout and tears them down; the `'a` lifetime ties this
    /// activation to those borrows for the frame call — mirrors [`ViewtFromDepthActivation::pipeline`].
    pub pipeline: &'a ComputePipeline,
    /// The DEDICATED 3-binding `viewt_from_depth_rz` bind-group LAYOUT { SAMPLED depth @0,
    /// STORAGE `gViewT` @1, UNIFORM camera @2 } — matching `viewt_from_depth_rz.comp`'s set 0.
    /// [`GBufferTargets`] writes a `viewt_from_vb_depth_set` against it once per extent (pointing
    /// at the per-extent reverse-Z `ForwardTargets::depth` + `gViewT` images + the shared camera
    /// ring), mirroring [`ViewtFromDepthActivation::layout`].
    pub layout: &'a VulkanBindGroupLayout,
    /// `boyko_render::view::forward_view_z_coeffs(near, far).0` — the reverse-Z encode's `A` in
    /// `view_z = B / (d − A)`. See [`crate::compute::ViewtFromDepthRzPush::view_z_a`]'s doc.
    pub view_z_a: f32,
    /// `boyko_render::view::forward_view_z_coeffs(near, far).1` — the encode's `B`. See
    /// [`crate::compute::ViewtFromDepthRzPush::view_z_b`]'s doc.
    pub view_z_b: f32,
}

/// Anti-aliasing Stage 1: the FXAA post-process pass activation threaded into
/// [`GBufferScene::aa`] to turn the AA pass ON. Mirrors [`SsaoActivation`]'s borrow-bundle
/// shape: a `Copy` pair of caller-owned pipeline/sampler borrows.
///
/// `None` on [`GBufferScene::aa`] is the OFF path (the DEFAULT — the 0%-gate): no `aa_out`
/// target, no `fxaa_set`, the present-blit samples `lit` directly, no FXAA pass recorded.
/// `Some` arms the whole seam.
#[derive(Clone, Copy)]
pub struct AaActivation<'a> {
    /// FXAA fullscreen graphics pipeline (`fullscreen_sample.vs` + `fxaa.fs`). Its pipeline
    /// layout declares `present_layout` at set 0 plus an 8-byte VERTEX|FRAGMENT push range
    /// (`rcp_frame`). `color_formats[0]` == `R8G8B8A8_UNORM` (`aa_out`'s format), NOT the
    /// swapchain format.
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// LINEAR/ClampToEdge sampler bound WITH `lit` in `fxaa_set` (FXAA's sub-texel tap needs
    /// bilinear filtering). DISTINCT from the NEAREST `present_sampler`.
    pub sampler: &'a VulkanSampler,
}

/// Anti-aliasing Stage 2: the SMAA 1x (3-pass morphological AA) post-process activation
/// threaded into [`GBufferScene::smaa`] to turn the SMAA pass ON. Mirrors [`AaActivation`]'s
/// borrow-bundle shape (a `Copy` bundle of caller-owned pipeline/layout/sampler/LUT borrows);
/// a PARALLEL field, not an enum sharing `AaActivation`'s slot — [`GBufferScene::aa`] stays
/// byte-for-byte unchanged (Decision 1).
///
/// `None` on [`GBufferScene::smaa`] is the OFF path (the DEFAULT — the 0%-gate): no `aa_out`
/// target, no `smaa_{edge,weight,blend}_set`, the present-blit samples `lit` directly, no SMAA
/// pass recorded. `Some` arms the whole seam. Mutually exclusive with [`GBufferScene::aa`] at
/// the populate site (`debug_assert!` at both `GBufferTargets::create` and the record site).
#[derive(Clone, Copy)]
pub struct SmaaActivation<'a> {
    /// Pass-1 (edge detection) fullscreen graphics pipeline (`fullscreen_sample.vs` +
    /// `smaa_edge.fs`). Layout = [`GBufferScene::present_layout`] (1 CIS: `lit`) + a 16-byte
    /// FRAGMENT push range (`rt_metrics`). `color_formats[0]` == `R8G8_UNORM` (`edges`'
    /// format).
    pub edge_pipeline: &'a VulkanGraphicsPipeline,
    /// Pass-2 (blending-weight calculation) fullscreen graphics pipeline (`smaa_weight.fs`).
    /// Layout = [`Self::weight_layout`] (3 CIS) + a 16-byte FRAGMENT push range.
    /// `color_formats[0]` == `R8G8B8A8_UNORM` (`weights`' format).
    pub weight_pipeline: &'a VulkanGraphicsPipeline,
    /// Pass-3 (neighborhood blending) fullscreen graphics pipeline (`smaa_blend.fs`). Layout
    /// = [`Self::blend_layout`] (2 CIS) + a 16-byte FRAGMENT push range. `color_formats[0]`
    /// == `R8G8B8A8_UNORM` (`aa_out`'s format — the same target FXAA's single pass writes).
    pub blend_pipeline: &'a VulkanGraphicsPipeline,
    /// The 3-CIS bind-group LAYOUT `{ edges @0, areaTex @1, searchTex @2 }` [`Self::weight_pipeline`]
    /// declares at set 0. [`GBufferTargets`] writes a
    /// `smaa_weight_set` against it once per extent.
    pub weight_layout: &'a VulkanBindGroupLayout,
    /// The 2-CIS bind-group LAYOUT `{ lit @0, weights @1 }` [`Self::blend_pipeline`] declares
    /// at set 0. [`GBufferTargets`] writes a `smaa_blend_set` against it once per extent.
    pub blend_layout: &'a VulkanBindGroupLayout,
    /// LINEAR/ClampToEdge sampler bound with EVERY SMAA tap (`lit`, `edges`, `weights`,
    /// `areaTex`, `searchTex` — Open Q2: a single sampler suffices for the whole 1x path,
    /// including the manual-decode diagonal search's bilinear `edgesTex` reads). DISTINCT
    /// boot object from [`AaActivation::sampler`] (FXAA path untouched).
    pub sampler: &'a VulkanSampler,
    /// Boot-resident `AreaTex` (160×560, `R8G8_UNORM`), `ShaderReadOnlyOptimal` forever —
    /// `boyko_render::smaa_luts::AREA_TEX_BYTES` uploaded once at boot via
    /// `boyko_render::upload_texture_2d_raw`.
    pub area_tex: &'a VulkanTexture,
    /// Boot-resident `SearchTex` (64×16, `R8_UNORM`), `ShaderReadOnlyOptimal` forever —
    /// `boyko_render::smaa_luts::SEARCH_TEX_BYTES` uploaded once at boot.
    pub search_tex: &'a VulkanTexture,
}

/// Anti-aliasing Stage 3: the SSAA 2× downsample post-process activation threaded into
/// [`GBufferScene::ssaa`] to turn the SSAA pass ON. Mirrors [`AaActivation`]'s borrow-bundle
/// shape: a `Copy` pair of caller-owned pipeline/sampler borrows.
///
/// `None` on [`GBufferScene::ssaa`] is the OFF path (the DEFAULT — the 0%-gate): `aa_out`
/// stays sized to `present_extent` (== native when off), no `downsample_set`, the
/// present-blit samples `lit` directly, no SSAA pass recorded. `Some` arms the whole seam —
/// mutually exclusive with [`GBufferScene::aa`] / [`GBufferScene::smaa`] at the populate site
/// (`debug_assert!` at both `GBufferTargets::create` and the record site). Unlike
/// `aa`/`smaa`, SSAA is host-authoritative: it can only be `Some` when the host armed the 2×
/// `composite_extent` at boot (see `boyko_app::host::WindowHost`).
#[derive(Clone, Copy)]
pub struct SsaaActivation<'a> {
    /// SSAA downsample fullscreen graphics pipeline (`fullscreen_sample.vs` +
    /// `ssaa_downsample.fs`). Layout = [`GBufferScene::present_layout`] (1 CIS: `lit`), NO
    /// push constants (the 2× ratio is compiled into the shader). `color_formats[0]` ==
    /// `R8G8B8A8_UNORM` (`aa_out`'s format, native-sized under SSAA — see
    /// [`GBufferTargets::aa_out`](crate::present::GBufferTargets)).
    pub pipeline: &'a VulkanGraphicsPipeline,
    /// NEAREST/ClampToEdge sampler bound WITH `lit` in `downsample_set` — the shared
    /// `present_sampler`. The shader uses `.Load` (texelFetch), which bypasses filtering, so
    /// the sampler is IGNORED; it exists only to satisfy the 1-CIS `present_layout` shape.
    /// DISTINCT borrow role from [`AaActivation::sampler`] (FXAA's tap needs bilinear).
    pub sampler: &'a VulkanSampler,
}

/// Anti-aliasing Stage 4: the TAA (Temporal Anti-Aliasing) temporal-resolve pass activation
/// threaded into [`GBufferScene::taa`] to turn the TAA seam ON. Mirrors [`SmaaActivation`]'s
/// borrow-bundle shape: a `Copy` bundle of caller-owned layout/format/sampler borrows.
///
/// `None` on [`GBufferScene::taa`] is the OFF path (the DEFAULT — the 0%-gate): no `aa_out`
/// target, no `taa_hist` history ring, the present-blit samples `lit` directly, no resolve pass
/// recorded. `Some` arms the seam: [`GBufferTargets`] allocates `aa_out` + `taa_hist` + the
/// resolve's own `taa_ubo`/`taa_motion_cam_ubo` rings + `taa_resolve_set`,
/// [`GBufferTargets::sync_gbuffer`] treats an arm-state change exactly like an extent change, and
/// `crate::present::passes::gbuffer`'s `record_taa` dispatches [`Self::resolve_pipeline`] at the
/// resolve→present seam, writing both `taa_hist[fi]` and `aa_out` directly (no dedicated
/// FXAA/SMAA-style INPUT descriptor set: the resolve set binds `lit`/`viewt`/`taa_hist`/the
/// tunables + camera + `MotionCam` UBOs itself). Mutually exclusive with [`GBufferScene::aa`] /
/// [`GBufferScene::smaa`] / [`GBufferScene::ssaa`] (`debug_assert!` — see [`SsaaActivation`]'s
/// doc for the same pattern).
///
/// # Why a DEDICATED `MotionCam` ring (not the hwrt mesh-shadow `motion_cam_ubo`)
///
/// TAA and the hwrt temporal shadow denoiser are independently armable features that can BOTH be
/// live in the same frame. `MotionCamState::advance` is a ONE-call-per-frame contract per
/// `Resource` instance — a second `advance()` call in the same frame (one per consumer) would
/// corrupt the persisted `prev` for whichever ran second. `boyko_app::runner` therefore advances
/// `MotionCamState` AT MOST ONCE per frame (reusing the hwrt mesh-shadow producer's pair when it
/// already ran this frame) and uploads that ONE pair into EACH consumer's OWN GPU ring — TAA's
/// dedicated `taa_motion_cam_ubo` (built in [`GBufferTargets`], NOT sourced from `GBufferScene`,
/// mirroring how `taa_ubo` is self-contained there) is that separate destination.
///
/// # Compute, not graphics — recorded BEFORE `present_sample` (unlike FXAA/SMAA/SSAA)
///
/// The resolve is a COMPUTE dispatch reading `lit`/`viewt`/`taa_hist_read` at `GENERAL` (the
/// framegraph's `taa_resolve` pass, declared BEFORE `present_sample` — see `graph_bridge.rs`),
/// straight out of the deferred resolve's write, rather than at `SHADER_READ_ONLY_OPTIMAL` the
/// way FXAA/SMAA/SSAA's FRAGMENT passes read `lit` AFTER `present_sample`'s transition. `record_taa`
/// is therefore recorded immediately after the main resolve dispatch, BEFORE
/// `present_sample`'s graph pass — see `gbuffer.rs`'s record-site ordering comment.
#[derive(Clone, Copy)]
pub struct TaaActivation<'a> {
    /// The TAA temporal-resolve compute pipeline (`taa_resolve.comp` /
    /// [`crate::compute::taa_resolve_spirv`]): its layout declares [`Self::resolve_layout`] at
    /// `set 0` + a 4-byte `{ uint reset; }` COMPUTE push range (`boyko_render::taa_state::TaaState`).
    /// Built UNCONDITIONALLY at boot (NOT `hwrt`-gated — the resolve's motion vector reconstructs
    /// from `gViewT`, never a `rayQuery` trace), mirroring [`AaActivation::pipeline`]'s
    /// always-built discipline.
    pub resolve_pipeline: &'a ComputePipeline,
    /// The TAA resolve compute pipeline's bind-group LAYOUT — the DEDICATED 8-binding shape {
    /// `gLit` COMBINED_IMAGE_SAMPLER @0, `gViewT` STORAGE @1, `gHistIn` STORAGE @2, `gHistOut`
    /// STORAGE @3, `gAaOut` STORAGE @4, the `ResolvedTaa` UBO @5, the camera UBO @6
    /// (UNJITTERED — C1 cut), the `MotionCam` UBO @7 }. [`GBufferTargets`] writes a per-FIF
    /// `taa_resolve_set` against it once per extent (the SSAO `ssao_set` precedent).
    pub resolve_layout: &'a VulkanBindGroupLayout,
    /// `[R8G8B8A8_UNORM]` — `aa_out`'s format, carried here (rather than hard-coded at the call
    /// site) so a future format change has one source of truth, mirroring [`SmaaActivation`]'s
    /// `color_formats`-shaped fields.
    pub color_formats: &'a [Format],
    /// LINEAR/ClampToEdge sampler for the resolve's `lit` combined-image-sampler read (the same
    /// resolve→AA-input seam FXAA's `fxaa_set` binds `lit` through). DISTINCT boot object from
    /// [`AaActivation::sampler`]/[`SmaaActivation::sampler`].
    pub linear_sampler: &'a VulkanSampler,
    /// `true` ⇒ this frame's resolve must force `blend_factor == 1.0` (full replace, never blend)
    /// — `boyko_render::taa_state::TaaState::advance`'s consumed-this-frame result, threaded
    /// from `boyko_app::runner` (the SAME "the runner resolves; the RHI only carries the scalar"
    /// discipline `GBufferScene::terminator_wrap`-shaped fields use). Pushed as the pipeline's
    /// 4-byte `{ uint reset; }` COMPUTE range.
    pub reset: bool,
}

/// TAA rung T3: the post-resolve CONTRAST-ADAPTIVE SHARPEN (AMD FidelityFX CAS) pass activation
/// (`rcas.comp` / [`crate::compute::rcas_spirv`]). `None` = OFF, the 0%-gate: no `taa_resolved`
/// intermediate image, no `rcas_set`, the resolve writes `aa_out` directly (byte-identical to
/// the pre-RCAS resolve — see `rcas.comp.hlsl`'s module doc for the full "aa_out ping-pong"
/// placement rationale). `Some` REQUIRES [`GBufferScene::taa`] to ALSO be `Some` — RCAS is a
/// pure post-process over the resolve's OWN intermediate output, never a standalone pass
/// (enforced by a `debug_assert!` in [`GBufferTargets::create`](crate::present::targets::GBufferTargets::create)).
///
/// `#[derive(Clone, Copy)]` — a bundle of borrows plus the `sharpness` scalar, mirroring
/// [`TaaActivation`]'s own shape.
#[derive(Clone, Copy)]
pub struct RcasActivation<'a> {
    /// The RCAS compute pipeline (`rcas.comp` / [`crate::compute::rcas_spirv`]): its layout
    /// declares [`Self::rcas_layout`] at `set 0` + a 16-byte `RcasPush` (`crate::compute::
    /// RcasPush`) COMPUTE push range. Built UNCONDITIONALLY at boot (mirroring
    /// [`TaaActivation::resolve_pipeline`]'s always-built discipline) so the mode can flip at
    /// runtime.
    pub rcas_pipeline: &'a ComputePipeline,
    /// The RCAS compute pipeline's bind-group LAYOUT — the DEDICATED 2-binding shape { `gRcasIn`
    /// STORAGE @0 (`taa_resolved`, the READ — the resolve's re-pointed intermediate output),
    /// `gAaOut` STORAGE @1 (`aa_out`, the WRITE — the present-blit's input, unchanged) }.
    /// [`GBufferTargets`] writes a per-FIF `rcas_set` against it once per extent.
    pub rcas_layout: &'a VulkanBindGroupLayout,
    /// The owner-set `SharpenMode::Rcas` (boyko_render's) strength in
    /// `[0, 1]` (`boyko_render::taa_config::TaaConfig::rcas_sharpness`), pushed verbatim as
    /// `RcasPush::sharpness`. `0` = mild (peak `-1/8`), `1` = strong (peak `-1/5`), per the
    /// FidelityFX CAS sharpness mapping `rcas.comp.hlsl` implements.
    pub sharpness: f32,
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
/// RDG pass / dispatch / barrier in `crate::present::passes::gbuffer`), so the command stream is
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

/// Multi-paradigm render-path plan, rung R1 — a plain POD mirror of
/// `boyko_render::render_path_config::ResolvedRenderPath`, carried across the
/// `boyko_render` → `boyko_rhi_vulkan` dependency-DIRECTION seam: this crate sits BELOW
/// `boyko_render` in the dependency graph (`boyko_render` depends on `boyko_rhi_vulkan`, never
/// the reverse — see `boyko_render`'s crate-root doc), so it cannot NAME that type — the SAME
/// reason `AaMode`/`ResolvedSsao` never appear on [`GBufferScene`] directly.
/// `boyko_app::gpu_scene::GpuSceneBundles::scene` converts at the frame-assembly seam (the
/// `aa_mode: AaMode` parameter → `matches!`-derived `Option<AaActivation>` fields precedent),
/// so the conversion here is a plain field-by-field copy into primitive/enum-discriminant form.
/// `#[repr(C)]`, `Copy` — mirrors `ResolvedRenderPath`'s own shape.
///
/// # Which fields have readers
///
/// This carrier landed at rung R1 as a whole-struct DEAD-BUT-THREADED plumbing rung, and its doc
/// said "nothing in this crate reads this field yet" long after that stopped being true. Per
/// field, as of the shipped VB path:
///
/// * **Dispatch inputs** — `path`, `mesh_leg`, `sdf_leg`, `sdf_forward_marched`,
///   `needs_depth_prepass`, `mesh_geo_shade_split` select the declarator/recorder shape (the
///   `path_*` predicates below are their single readers, per the O1 rule).
/// * **Checked, not dispatched** — `shadow`. Read at exactly one site, and by an ASSERTION:
///   `GBufferScene::shadow_has_sdf_soft_march` (an unlinked code span — the method is
///   `#[cfg(feature = "hwrt")]`, so a default-feature rustdoc build cannot resolve a link to it)
///   backs `record_vb`'s `vb_shade_split_*hwrt`
///   exclusion check. It deliberately selects nothing. The bits are a BOOT record of which
///   shadow sources a scene may use, and every per-frame arming gate is strictly stronger than
///   the corresponding bit — `csm_armed` additionally needs live caster batches, the hwrt chain
///   additionally needs a non-empty TLAS — so substituting a bit for a gate would arm shadow
///   work on frames the passes deliberately skip. The bits record a NECESSARY condition; only as
///   such are they sound to read.
/// * **No reader at all** — `legs`, `prepass_writes_motion`, `sdf_geo_shade_split`,
///   `sdf_surface_cache`, `depth_kind`, `thin_aux`, `vb_geometry_table`, `froxel_light_cull`.
///
/// The last two are worth spelling out, because the DECISIONS they carry are very much live and
/// it would be easy to conclude the copies here are what carries them. They are not.
/// `boyko_app::runner` reads `vb_geometry_table` / `froxel_light_cull` off its OWN
/// `boyko_render::ResolvedRenderPath` — the value BEFORE this conversion — and each reaches this
/// crate by a different route: `vb_geometry_table` through `VulkanContext::
/// set_vb_geometry_table_armed` (a boot-once `OnceCell` on the device), the froxel arm through
/// `cluster_cull_armed()`. Nothing in this crate reads either field of THIS struct. Wiring a
/// reader to one of them would not be a cleanup: it would be a second, unsynchronised arming
/// source for machinery that already has exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ResolvedRenderPathGpu {
    /// `RenderPath` discriminant (`Deferred = 0`, `Forward = 1`, `ForwardPlus = 2`,
    /// `VisibilityBuffer = 3` — see [`RENDER_PATH_VISIBILITY_BUFFER`]).
    pub path: u32,
    /// `GeometryLegs` discriminant (`Both = 0`, `Mesh = 1`, `Sdf = 2`).
    pub legs: u32,
    /// `ResolvedRenderPath::mesh_leg`.
    pub mesh_leg: bool,
    /// `ResolvedRenderPath::sdf_leg`.
    pub sdf_leg: bool,
    /// `ResolvedRenderPath::sdf_forward_marched`.
    pub sdf_forward_marched: bool,
    /// `ResolvedRenderPath::needs_depth_prepass`.
    pub needs_depth_prepass: bool,
    /// `ResolvedRenderPath::prepass_writes_motion`.
    pub prepass_writes_motion: bool,
    /// `ResolvedRenderPath::mesh_geo_shade_split`.
    pub mesh_geo_shade_split: bool,
    /// `ResolvedRenderPath::sdf_geo_shade_split`.
    pub sdf_geo_shade_split: bool,
    /// `ResolvedRenderPath::sdf_surface_cache`.
    pub sdf_surface_cache: bool,
    /// `ResolvedRenderPath::vb_geometry_table`.
    pub vb_geometry_table: bool,
    /// `DepthKind` discriminant (`CustomLinear = 0`, `HardwareReverseZ = 1`).
    pub depth_kind: u32,
    /// `ThinAuxMask::bits()`.
    pub thin_aux: u32,
    /// `ShadowSources::bits()`.
    pub shadow: u32,
    /// `ResolvedRenderPath::froxel_light_cull`. NOT hardcoded — the resolver derives it
    /// (`consumers.clusters_wanted && path == VisibilityBuffer`), and VB-P1b's opt-in scenes arm
    /// it. This COPY has no reader; see the struct doc for the route the arm actually takes.
    pub froxel_light_cull: bool,
}

/// Code review P2-5: the `RenderPath::VisibilityBuffer` discriminant
/// (`boyko_render::render_path_config::RenderPath`), named instead of a bare `3` literal at
/// [`GBufferScene::path_is_vb`]'s comparison site — mirrors [`ResolvedRenderPathGpu::path`]'s
/// own doc table.
pub(crate) const RENDER_PATH_VISIBILITY_BUFFER: u32 = 3;

/// `boyko_render::ShadowSources::SDF_SOFT_MARCH`'s bit, named here for the same reason
/// `RENDER_PATH_VISIBILITY_BUFFER` above is: this crate cannot depend on `boyko_render` (the
/// dependency runs the other way — that is WHY [`ResolvedRenderPathGpu`] exists), so the bit
/// value has to be restated to be read at all.
///
/// `pub` rather than `pub(crate)` so the restatement is PINNED against the owning definition
/// instead of drifting from it: `boyko_app` depends on both crates and asserts the two agree
/// (`shadow_source_sdf_soft_march_bit_matches_boyko_render`). Without that pin this const would
/// be a second, unchecked copy of a value only the other crate can change.
pub const SHADOW_SOURCE_SDF_SOFT_MARCH: u32 = 1 << 2;

impl Default for ResolvedRenderPathGpu {
    /// `Deferred + Both`, every derived flag off, `depth_kind = CustomLinear` — the byte-identity
    /// anchor, matching `boyko_render::render_path_config::ResolvedRenderPath::default()`.
    #[inline]
    fn default() -> Self {
        Self {
            path: 0,
            legs: 0,
            mesh_leg: true,
            sdf_leg: true,
            sdf_forward_marched: false,
            needs_depth_prepass: false,
            prepass_writes_motion: false,
            mesh_geo_shade_split: false,
            sdf_geo_shade_split: false,
            sdf_surface_cache: false,
            vb_geometry_table: false,
            depth_kind: 0,
            thin_aux: 0,
            shadow: 0,
            froxel_light_cull: false,
        }
    }
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
    /// Multi-paradigm render-path plan, rung R3b (`Deferred × Mesh` — the SDF leg fully off):
    /// the `viewt_from_depth` activation — the `gViewT` producer that stands in for the
    /// (undispatched) marcher on a mesh-only frame. `None` (the default, every OTHER leg) ⇒ NO
    /// descriptor set (`GBufferTargets` never builds `viewt_from_depth_set`), NO framegraph pass,
    /// NO dispatch — the 0%-gate, capability = component presence (not a runtime flag). `Some`
    /// iff the caller resolved `mesh_leg && !sdf_leg` (`GeometryLegs::Mesh`) — see
    /// [`ViewtFromDepthActivation`]'s doc.
    pub viewt_from_depth: Option<ViewtFromDepthActivation<'a>>,
    /// TAA-under-VB (`VisibilityBuffer × Mesh`): the `viewt_from_depth_rz` `gViewT`-producer
    /// activation. `None` (the default) ⇒ NO descriptor set (`GBufferTargets` never builds
    /// `viewt_from_vb_depth_set`), NO framegraph pass, NO dispatch — the 0%-gate, capability =
    /// component presence (not a runtime flag). `Some` iff the caller resolved
    /// `VisibilityBuffer × Mesh` with TAA armed — see [`ViewtFromVbDepthActivation`]'s doc.
    pub viewt_from_vb_depth: Option<ViewtFromVbDepthActivation<'a>>,
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
    /// waits on, [`Renderer::frame_index`](crate::present::Renderer::frame_index)); the host writes that SAME slot before the
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
    /// `dims.0 × dims.1 × dims.2` lattice of [`boyko_sdf_math::BrickClass`] codes
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
    /// The Lighting-L1 clustered froxel light-cull compute pipeline: ONE pipeline per boot,
    /// built from EITHER `cluster_cull.comp` (the base arm) OR `cluster_cull_hier.comp` (the
    /// VB-P1e hierarchical arm). Its layout declares the cull bind-group LAYOUT at `set 0` plus
    /// a COMPUTE push range whose SIZE FOLLOWS THE ARM: 16 bytes
    /// ([`crate::compute::ClusterCullPush`]) for the base arm, 24
    /// ([`crate::compute::ClusterCullHierPush`], the base four words plus the D11 boot-snapshot
    /// `cluster_dims_packed` + `cluster_capacity`) for the hierarchical one.
    /// [`Self::cluster_cull_hier`] is `Some` IFF this field holds the hierarchical pipeline, and
    /// it carries the matching push image and group count — the two MUST be read together, since
    /// pushing 16 bytes into a 24-byte layout would leave the hierarchical arm's write bound
    /// undefined.
    ///
    /// `None` ⇒ L1 is not wired (the L0b-only build) and the cull pass + its barriers are skipped
    /// entirely, so the resolve loops the flat table — the L1 OFF path.
    ///
    /// The gate producing that flat walk is `use_clusters`, and it is THREE terms since VB-P1k:
    /// `clusters_enabled != 0 && cluster_count != 0 && cluster_count <= grid_capacity`, the
    /// capacity read off the BOUND `ClusterGrid` descriptor with `GetDimensions`. The two terms
    /// past the enabled bit are an out-of-bounds guard, not a style choice: `robustBufferAccess`
    /// is OFF in this engine and there is no GPU-assisted validation, so an out-of-range
    /// `ClusterGrid` read is real UB that no layer reports.
    ///
    /// This field being `None` is strictly WIDER than the header's arm bit, and the two are
    /// resolved in different places — do not infer one from the other:
    ///
    /// * the header's dims lane is zeroed by `sync_cluster_light_gate` exactly when
    ///   `ResolvedRenderPath::froxel_light_cull` is false, and that bit is TWO terms —
    ///   `LightingConfig::clusters_enabled` AND `path == RenderPath::VisibilityBuffer`
    ///   (`render_path_config.rs`'s resolver; `runner.rs` threads `clusters_wanted` from
    ///   `clusters_enabled` alone). **No geometry-leg term enters it**, unlike the sibling
    ///   `vb_geometry_table`, which does carry `mesh_leg`;
    /// * this field's sole writer, `GpuSceneBundles::build_froxel_light_cull`, additionally needs
    ///   a live `MeshGeometryTableSlot` — `runner.rs` nests the call inside `if let Some(table)`.
    ///   That skew is the one `GpuSceneBundles::cluster_cull_armed`'s doc records.
    ///
    /// ⚠️ **Which term decides on a given boot, and whether any `ClusterGrid` reader is bound at
    /// all, is deliberately NOT enumerated here.** Two attempts to write that boot-by-boot chain
    /// were each refuted against the code — it spans the path resolver, the runner, the frame
    /// router, three resolve-pipeline variants (`resolve_pipeline`, its `terminator_wrap`
    /// substitute, and the hwrt one) and the VB recorder's own `FROXEL` selection, so a comment
    /// restating it rots faster than it can be verified. Read the resolver and the recorder; the
    /// authority is `cluster_grid_read_bound.rs`, whose census is closed over the shader roots and
    /// fails on an unenumerated `ClusterGrid` consumer.
    ///
    /// When `Some`, the recorder dispatches it BEFORE the resolve, with a
    /// COMPUTE→COMPUTE buffer barrier so the resolve reads see the cull writes. The base arm
    /// dispatches over [`Self::cluster_count`] froxels at 64 lanes; the hierarchical arm uses
    /// [`Self::cluster_cull_hier`]'s own group count at 256 lanes.
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
    /// VB-P1e D11/H4: `Some` IFF [`Self::cluster_cull`] holds the `-D HIER=1` pipeline instead
    /// of the base arm — the record site then dispatches [`ClusterCullHierDispatch::groups`]
    /// groups of 256 and pushes [`ClusterCullHierDispatch::push`] (24 B) INSTEAD of
    /// [`Self::cluster_count`]/[`Self::cluster_cull_push`] (which stay populated but unused by
    /// the record site in that case). `None` (the default, every pre-H4 boot and every boot
    /// that does not opt into the hierarchical arm) ⇒ the base arm records exactly as before —
    /// byte-identical command stream.
    pub cluster_cull_hier: Option<ClusterCullHierDispatch>,
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
    /// The SSAO edge-avoiding à-trous denoise chain's `level == 0` pipeline variant
    /// (`ssao_atrous_read8.comp` / [`crate::compute::ssao_atrous_read8_spirv`]) — `gAoIn` pinned
    /// `r8` (reads the frozen `gSsao` gather endpoint), `gAoOut` pinned `r16` (writes ring 0). Its
    /// layout is [`Self::ssao_atrous_layout`]. UNCONDITIONAL (both feature legs, NOT `hwrt`-gated
    /// — SOFTWARE) + ALWAYS BUILT (unlike the RT-gated shadow denoise, this pipeline needs no
    /// device precondition to CREATE — only the interior ring IMAGE needs `R16_UNORM` storage,
    /// checked separately by [`GBufferTargets`]'s degrade). DECOUPLED from the per-frame
    /// [`Self::ssao`] activation (mirroring `Self::atrous_layout_denoise_hwrt`'s "the
    /// set-builder reads the stable boot signals" discipline) — [`GBufferTargets::create`] reads
    /// THIS field directly to build the role-keyed sets, so a later frame that arms
    /// [`SsaoActivation::atrous_levels`] finds them already built (no resize/rebuild needed).
    /// `Option`-typed for uniformity with `resolve_layout_denoise_hwrt`'s shape (a host that
    /// never wires the boot à-trous pipelines threads `None`, e.g. a minimal test harness that
    /// exercises only the gather); production (`crate::present::passes::gbuffer`'s caller)
    /// always threads `Some`.
    pub ssao_atrous_read8_pipeline: Option<&'a ComputePipeline>,
    /// The SSAO à-trous chain's INTERIOR pipeline variant (`ssao_atrous.comp` /
    /// [`crate::compute::ssao_atrous_spirv`]): both `gAoIn`/`gAoOut` pinned `r16` (the two
    /// ping-pong rings). Its layout is [`Self::ssao_atrous_layout`]. Built in lock-step with
    /// [`Self::ssao_atrous_read8_pipeline`] (same discipline, same doc).
    pub ssao_atrous_interior_pipeline: Option<&'a ComputePipeline>,
    /// The SSAO à-trous chain's LAST-level pipeline variant (`ssao_atrous_write8.comp` /
    /// [`crate::compute::ssao_atrous_write8_spirv`]): `gAoIn` pinned `r16` (reads a ring),
    /// `gAoOut` pinned `r8` (writes BACK into the frozen `gSsao` endpoint the resolve reads — the
    /// C1 fix). Its layout is [`Self::ssao_atrous_layout`]. Built in lock-step with
    /// [`Self::ssao_atrous_read8_pipeline`] (same discipline, same doc).
    pub ssao_atrous_write8_pipeline: Option<&'a ComputePipeline>,
    /// The SSAO à-trous chain's DEDICATED 4-binding bind-group LAYOUT { `gAoIn` STORAGE image @0
    /// (R), `gAoOut` STORAGE image @1 (W), `gViewT` STORAGE image @2 (R), the camera UNIFORM
    /// buffer @3 } — IDENTICAL across all three pipeline variants above (only the bound VIEW +
    /// the `[[vk::image_format]]` pin differ — see `ssao_atrous.comp.hlsl`'s header doc). Unlike
    /// the shadow à-trous layout, this pass carries NO `gNormal` (depth-plane-fit-only edge stop)
    /// and NO tunables UBO (baked `static const`). [`GBufferTargets`] writes FIVE role-keyed sets
    /// against it ONCE per extent (`ssao_atrous_read8_set` / `ssao_atrous_interior_from0_set` /
    /// `ssao_atrous_interior_from1_set` / `ssao_atrous_write8_from0_set` /
    /// `ssao_atrous_write8_from1_set` — see [`crate::present::ssao_atrous_step`]'s role→set
    /// mapping) WHEN [`Self::ssao_atrous_read8_pipeline`] is `Some` AND the device supports
    /// `R16_UNORM` STORAGE ([`crate::device::DeviceCaps::ssao_atrous_storage_ok`]); `None` sets
    /// otherwise — mirroring the DDGI/shadow-denoise graceful-degrade discipline (opt-in, never a
    /// boot fault).
    pub ssao_atrous_layout: Option<&'a VulkanBindGroupLayout>,
    /// Anti-aliasing Stage 1: the FXAA post-process pass activation. `None` = OFF, the
    /// 0%-gate (no `aa_out` target, no `fxaa_set`, the present-blit samples `lit` directly,
    /// no FXAA pass recorded). `Some` arms the whole seam: [`GBufferTargets`] allocates
    /// `aa_out` + builds `fxaa_set`, [`GBufferTargets::sync_gbuffer`] treats an arm-state
    /// change exactly like an extent change (a fence-safe rebuild), and the present-blit
    /// samples `aa_out` instead of `lit`. Toggling `Some`↔`None` between frames is a genuine
    /// live runtime switch, not a boot-only choice.
    pub aa: Option<AaActivation<'a>>,
    /// Anti-aliasing Stage 2: the SMAA 1x post-process pass activation. `None` = OFF, the
    /// 0%-gate (no `aa_out` target, no `smaa_{edge,weight,blend}_set`, the present-blit
    /// samples `lit` directly, no SMAA pass recorded). `Some` arms the 3-pass seam:
    /// [`GBufferTargets`] allocates `aa_out` + `smaa_edges`/`smaa_weights` + builds the three
    /// SMAA sets, [`GBufferTargets::sync_gbuffer`] treats an arm-state change exactly like an
    /// extent change, and the present-blit samples `aa_out` instead of `lit`. Mutually
    /// exclusive with [`Self::aa`] (`debug_assert!` — see [`SmaaActivation`]'s doc).
    pub smaa: Option<SmaaActivation<'a>>,
    /// Anti-aliasing Stage 3: the SSAA 2× downsample post-process pass activation. `None` =
    /// OFF, the 0%-gate (`aa_out` stays sized to `present_extent`, no `downsample_set`, the
    /// present-blit samples `lit` directly, no SSAA pass recorded). `Some` arms the seam:
    /// [`GBufferTargets`] sizes `aa_out` to the NATIVE `aa_extent` (not `present_extent`,
    /// which is 2× under SSAA) + builds `downsample_set`, [`GBufferTargets::sync_gbuffer`]
    /// treats an arm-state change exactly like an extent change, and the recorded downsample
    /// pass resolves the 2× `lit` into native `aa_out` (the present-blit's unchanged 1:1 crop
    /// then samples it). Mutually exclusive with [`Self::aa`] / [`Self::smaa`]
    /// (`debug_assert!` — see [`SsaaActivation`]'s doc). UNLIKE `aa`/`smaa`, this is NOT a
    /// live per-frame toggle: `Some` can only occur when the host armed the 2×
    /// `composite_extent` at boot (`boyko_app::host::WindowHost`) — the render-scale
    /// resolution is a boot commitment, not a per-frame choice.
    pub ssaa: Option<SsaaActivation<'a>>,
    /// Anti-aliasing Stage 4: the TAA temporal-resolve pass activation. `None` = OFF, the
    /// 0%-gate (`aa_out`/`taa_hist` stay unallocated, the present-blit samples `lit` directly, no
    /// resolve pass recorded). `Some` arms the seam (see [`TaaActivation`]'s doc for the full
    /// shape). Mutually exclusive with [`Self::aa`] / [`Self::smaa`] / [`Self::ssaa`]
    /// (`debug_assert!` — see [`SsaaActivation`]'s doc for the same pattern). Native resolution
    /// like `aa`/`smaa` (NOT render-scaled, unlike `ssaa`) — a live per-frame toggle.
    ///
    /// **v1 motion-quality caveat** (see boyko_render's `AaMode::Taa` doc):
    /// only the raster mesh path is sub-pixel jittered (C1 — SDF-marched pixels stay temporally
    /// stable but un-supersampled); the resolve is landed OFF-byte-identical and
    /// converged-static-validated, but in-motion quality (ghosting, disocclusion) is owner-gated,
    /// not yet visually blessed.
    pub taa: Option<TaaActivation<'a>>,
    /// TAA rung T3: the post-resolve RCAS sharpen pass activation. `None` = OFF, the
    /// 0%-gate — see [`RcasActivation`]'s doc. REQUIRES [`Self::taa`] to also be `Some`
    /// (`SharpenMode::Rcas` is meaningless without `AaMode::Taa` — RCAS reads the resolve's
    /// OWN intermediate output); never armed standalone (enforced by a `debug_assert!` in
    /// [`GBufferTargets::create`](crate::present::targets::GBufferTargets::create)).
    pub rcas: Option<RcasActivation<'a>>,
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
    /// `Some(_)` = the ON path: BEFORE the resolve dispatch the recorder runs `record_csm_depth`
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
    /// `RESOLVED_DDGI_BYTES` (boyko_render's, 48 B) byte-mirror of
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
    /// VB-P1d: the optional VisibilityBuffer froxel light-cull GPU-timestamp bench collector.
    /// `None` on EVERY golden/host/interactive frame (the DEFAULT — the SAME capability-as-
    /// presence discipline [`Self::gpu_timing`] uses) ⇒ the recorder emits ZERO reset/write
    /// commands around the `record_vb` cull/shade dispatches, so the recorded command stream is
    /// BYTE-IDENTICAL to the pre-VB-P1d path. `Some(tc)` (the offline VB-P1d froxel cull/shade
    /// cost bench, `boyko_app::runner`'s `BOYKO_VB_BENCH`-gated collector) brackets the L1
    /// clustered light-cull dispatch and the `vb_shade`/`vb_resolve` lit-producer dispatch (the
    /// `record_vb`-only sibling of [`Self::gpu_timing`]'s four G-buffer software-ray passes) so
    /// the bench reports per-pass GPU wall-clock. A separate collector TYPE from
    /// [`Self::gpu_timing`] (not a shared enlarged `PASS_COUNT`) — see
    /// [`super::gpu_timing::VbTimestampCollector`]'s own doc for why.
    pub vb_gpu_timing: Option<&'a VbTimestampCollector>,
    /// VB-SV0 rung S1.5: the optional DEFERRED fine-marcher GPU-timestamp bench collector.
    /// `None` on EVERY golden/host/interactive frame (the DEFAULT — the SAME capability-as-
    /// presence discipline [`Self::gpu_timing`] uses) ⇒ the recorder emits ZERO reset/write
    /// commands around the marcher dispatch, so the recorded command stream is BYTE-IDENTICAL to
    /// the pre-S1.5 path. `Some(tc)` (`boyko_app::runner`'s `BOYKO_SV0_BENCH`-gated collector)
    /// brackets the `sdf_gbuffer_composite.hlsl` dispatch — the pass that carries both
    /// `pc.lighting_flags`-gated shadow/AO arms S1.5 measures by interleaved paired A/B.
    ///
    /// A THIRD collector TYPE rather than a widened [`Self::gpu_timing`] / [`Self::vb_gpu_timing`]
    /// — see [`super::gpu_timing::Sv0TimestampCollector`]'s own doc for the deadlock that forces
    /// independent pool sizing.
    pub sv0_gpu_timing: Option<&'a Sv0TimestampCollector>,
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
    /// F8-mv: the combined MOTION_VECTORS + PER_INSTANCE_MATERIAL raster pipeline
    /// (`gbuffer_mrt_mvpm.{vs,fs}`) — identical to [`Self::raster_pipeline_mv`] except its
    /// set-0 layout also declares the per-instance material SSBO at binding 3. `Some` on an
    /// RT + storage device (the same boot gate as [`Self::raster_pipeline_mv`]); `None`
    /// otherwise. Bound instead of [`Self::raster_pipeline_mv`]/[`Self::raster_pipeline_pm`]
    /// ONLY when [`Self::mesh_mvpm_active`] holds. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub raster_pipeline_mvpm: Option<&'a VulkanGraphicsPipeline>,
    /// F8-mv: this frame's combined set-0 bind group (slot `frame_index` of the mvpm
    /// resources' per-FIF bind groups: `{ instances[i], prev_instances[i], motion_cam[i],
    /// pm_instance_material_rings[i] }`). Bound at set 0 when the mvpm pipeline is selected.
    /// `Some` iff [`Self::raster_pipeline_mvpm`] is `Some`. `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub mvpm_bind_group: Option<&'a VulkanBindGroup>,
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
    /// Textured-PBR T6c: `true` iff THIS frame's TEXTURED gather scattered at least one
    /// bound bindless texture slot (`MeshRenderScratch::any_textured_material`) — the
    /// per-frame TEXTURED raster-pipeline selection gate. NOT `#[cfg(feature = "hwrt")]` —
    /// materials/textures are device-agnostic (mirrors [`Self::pm_enabled`]). `false` on
    /// every non-textured scene ⇒ the recorder binds the FROZEN base/pm pipeline.
    pub tex_enabled: bool,
    /// Textured-PBR T6c: the TEXTURED gbuffer producer pipeline (`gbuffer_mrt_tex.{vs,fs}`)
    /// — a 2-SET pipeline (set 0 = the `PerInstanceMaterialTex` layout, VERTEX; set 1 = the
    /// bindless texture-array set, FRAGMENT — built via
    /// [`VulkanContext::create_graphics_pipeline_bindless`]), built UNCONDITIONALLY at boot
    /// (materials/textures are device-agnostic, like `pm`). `Some` iff [`Self::tex_enabled`]
    /// (belt-and-suspenders); bound instead of
    /// [`Self::raster_pipeline`]/[`Self::raster_pipeline_pm`] ONLY when
    /// [`Self::mesh_tex_active`] holds.
    pub raster_pipeline_tex: Option<&'a VulkanGraphicsPipeline>,
    /// Textured-PBR T6c: this frame's TEXTURED set-0 bind group (slot `frame_index` of the
    /// TEX resources' per-FIF bind groups: `{ instance_rings[i] @0,
    /// tex_instance_material_rings[i] @1 }`). Bound at set 0 when the TEXTURED pipeline is
    /// selected. `Some` iff [`Self::tex_enabled`].
    pub tex_bind_group: Option<&'a VulkanBindGroup>,
    /// Textured-PBR T6c: the raw bindless texture-array descriptor SET (its LAYOUT is
    /// already baked into [`Self::raster_pipeline_tex`]'s `VkPipelineLayout` at boot — this
    /// is only the per-frame BIND) — bound at set 1 by `cmd_bind_descriptor_sets` when the
    /// TEXTURED pipeline is selected. `Some` iff the boot-time `BindlessTextureTable` create
    /// succeeded (`boyko_render::bindless::BindlessTextureTable::new` is fallible).
    pub bindless_set: Option<VkDescriptorSet>,
    /// Multi-paradigm render-path plan, rung R1: the boot-committed render-path selection,
    /// threaded down from `boyko_app::runner`'s ONE-TIME `resolve_render_path` call (Decision
    /// 1 — never re-derived per frame, unlike `aa`/`ssao` above). As of rung R4b-b this is READ
    /// (`declare_frame_graph`'s dispatch, `TargetsProfile::from_scene`), not merely threaded.
    /// See [`ResolvedRenderPathGpu`]'s doc for the `boyko_render` → `boyko_rhi_vulkan` boundary
    /// crossing.
    pub resolved_render_path: ResolvedRenderPathGpu,

    // ---- Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh pipeline ---------
    //
    // Built UNCONDITIONALLY at `boyko_app::gpu_scene::GpuSceneBundles::boot` (the
    // `ssao_pipelines` precedent — cheap to create, no per-frame cost either way), so PRODUCTION
    // always threads `Some(...)` here, regardless of `resolved_render_path.path`; only a
    // `Forward`-resolved boot ever RECORDS the pass (`declare_forward_graph`/`record_forward`).
    //
    // Code-review fix: these 5 fields were originally plain non-`Option` references. That forced
    // EVERY `GBufferScene`-constructing test fixture that never exercises `Forward` (every
    // fixture in `window_present_gbuffer.rs`) to thread a type-matching-but-semantically-WRONG
    // placeholder (e.g. `&raster_pipeline` standing in for `forward_pipeline`, `&vocab_layout`
    // for both `forward_layout0`/`forward_layout1` despite their different binding counts/shapes)
    // just to satisfy the compiler — exactly the kind of "compiles but is nonsense if ever read"
    // trap `Option::None` exists to make impossible. `Option` lets those fixtures say `None`
    // (an honest "Forward is not wired here") instead.
    /// The Forward-family opaque mesh raster pipeline — EITHER the plain `Forward` variant
    /// (`forward_opaque.fs.hlsl`'s base compile, `VK_COMPARE_OP_GREATER`, depth-write ON) OR the
    /// `ForwardPlus` froxel variant (`forward_opaque_froxel.fs.hlsl`, `VK_COMPARE_OP_EQUAL`,
    /// depth-write OFF), selected at the `GpuSceneBundles::scene()` seam. A plain 2-Vulkan-set
    /// layout (set 0 = [`Self::forward_layout0`]'s 7 bindings, set 1 = [`Self::forward_layout1`]'s
    /// 4 bindings) EITHER WAY — rung R5 code-review fix: exactly ONE Set-0 layout OBJECT for the
    /// whole family, not two structurally-identical-but-distinct handles (a real Vulkan
    /// pipeline/descriptor-set incompatibility bug an earlier revision shipped). Built via
    /// [`VulkanContext::create_graphics_pipeline_forward`]/
    /// [`VulkanContext::create_graphics_pipeline_forward_plus`]. Boot-panic fix: an earlier
    /// revision used a 3-set `[Set0, <empty Set1 placeholder>, Set2]` shape — a zero-binding
    /// bind-group layout is REJECTED by `RhiDevice::create_bind_group_layout`'s own
    /// `1..=MAX_BIND_GROUP_BINDINGS` invariant, crashing `GpuSceneBundles::boot`.
    /// `forward_opaque.fs.hlsl`'s shadow bindings were renumbered from Set 2 to Set 1 instead
    /// (that shader's doc), eliminating the placeholder. `None` in every test fixture that never
    /// resolves a Forward-family path (`TargetsProfile::from_scene`/`record_forward` are the only
    /// readers, both gated on [`Self::path_is_forward`], so a `None` here is never `.expect`-ed).
    pub forward_pipeline: Option<&'a VulkanGraphicsPipeline>,
    /// Code-review follow-up (rung R4b-b): the Forward v1 sky BACKGROUND pipeline
    /// (`forward_sky.{vs,fs}.hlsl`) — replicates the deferred resolve's `mask == 0` analytic
    /// sky/ground gradient + sun disc for uncovered pixels (`deferred_pbr.hlsl:1369-1414`).
    /// REUSES [`Self::forward_layout0`] as its ONLY set (its FS reads just `Camera`/`LightBuf`,
    /// a subset — the SAME "shader references a subset of what its layout declares" idiom
    /// [`Self::forward_pipeline`]'s own Set-0 VS-only bindings already establish); `depth_format:
    /// None` (no depth attachment declared — Vulkan permits this within a dynamic-rendering scope
    /// that DOES bind one, the pipeline simply neither tests nor writes it), so
    /// [`Renderer::record_forward`](super::frame_driver::Renderer::record_forward) draws it
    /// FIRST inside `forward_opaque`'s SAME `begin_rendering` scope, before the opaque mesh loop
    /// (which keeps its own real depth test/write). Same `Option`/`None`-in-non-Forward-fixtures
    /// rationale as [`Self::forward_pipeline`].
    pub forward_sky_pipeline: Option<&'a VulkanGraphicsPipeline>,
    /// The UNIFIED Forward-family Set-0 (core) bind-group LAYOUT — 7 bindings: `instances` @0
    /// (VERTEX, [`Self::forward_instance_ring`]), `instance_materials` @1 (VERTEX,
    /// [`Self::forward_instance_material_ring`]), `Camera` @2 (FRAGMENT, [`Self::camera_ring`]),
    /// `LightBuf` @3 (FRAGMENT, [`Self::light_table`]), `Materials` @4 (FRAGMENT,
    /// [`Self::material_table`]), `ClusterGrid` @5 / `LightIndexList` @6 (FRAGMENT,
    /// [`Self::cluster_grid`]/[`Self::light_index`], or the [`Self::light_table`] placeholder
    /// when unarmed) — byte-identical binding SHAPE to `forward_opaque.fs.hlsl`'s doc.
    /// [`GBufferTargets`] writes the per-FIF bind group against this SAME layout once per extent
    /// (the `vocab_layout`/`resolve_layout` precedent). Rung R5 code-review fix: ONE layout
    /// object for BOTH `Forward` (the base FS references only a 5-binding subset — a pipeline
    /// layout may always be a superset of what a shader stage declares) AND `ForwardPlus` (the
    /// froxel FS references all 7) — an earlier revision built two structurally-identical-but-
    /// distinct layout handles, which is a genuine Vulkan pipeline/descriptor-set incompatibility
    /// (not merely a style choice). See [`Self::forward_pipeline`]'s doc for the `Option`
    /// rationale.
    pub forward_layout0: Option<&'a VulkanBindGroupLayout>,
    /// The Forward v1 Set-1 (shadow) bind-group LAYOUT — 4 bindings: `gCsm`+`gCsmCmp` @0
    /// (FRAGMENT, COMBINED_IMAGE_SAMPLER, [`Self::csm_cascade_texture`] +
    /// [`Self::csm_compare_sampler`]), `CsmCascades` UBO @1 (FRAGMENT,
    /// [`Self::csm_cascade_ring`]), `gShadowAtlas`+`gShadowAtlasCmp` @2 (FRAGMENT,
    /// COMBINED_IMAGE_SAMPLER, [`Self::shadow_atlas_texture`] + [`Self::shadow_atlas_sampler`]),
    /// `ShadowAtlas` UBO @3 (FRAGMENT, [`Self::shadow_atlas_ubo`]) — byte-identical binding
    /// SHAPE to `forward_opaque.fs.hlsl`'s doc (a DIFFERENT binding-number layout than the
    /// deferred resolve's single compute set, same underlying resources). NOT the bindless
    /// texture table (unlike Deferred/TEXTURED's Set 1) — this v1 pipeline has none. See
    /// [`Self::forward_pipeline`]'s doc for the `Option` rationale.
    pub forward_layout1: Option<&'a VulkanBindGroupLayout>,
    /// The RAW per-FIF instance-model SSBO ring (`InstanceModelCol`) — the SAME
    /// `instance_rings` [`Self::instance_bind_group`]/[`Self::pm_bind_group`] already bind, now
    /// ALSO exposed as raw buffer references so [`GBufferTargets`] can fold them into Forward's
    /// OWN 5-binding Set 0 (a different grouping than any existing bind group). Zero new
    /// allocation — a plumbing-only reference. See [`Self::forward_pipeline`]'s doc for the
    /// `Option` rationale.
    pub forward_instance_ring: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// The RAW per-FIF `PerInstanceMaterial` SSBO ring — the SAME ring [`Self::pm_bind_group`]
    /// already binds at its own binding 1, exposed raw for the same reason as
    /// [`Self::forward_instance_ring`]. See [`Self::forward_pipeline`]'s doc for the `Option`
    /// rationale.
    pub forward_instance_material_ring: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,

    // ---- Multi-paradigm render-path plan, rung R5: the ForwardPlus depth PRE-PASS -----------
    /// The `depth_prepass` pipeline (`depth_prepass.{vs,fs}.hlsl`) — a depth-only
    /// (`VK_COMPARE_OP_GREATER`, depth-write ON) pipeline reusing [`Self::forward_layout0`] as
    /// its ONLY set (the VS references only the `instances` binding, a subset of that layout —
    /// the SAME bound-but-unread-subset idiom [`Self::forward_sky_pipeline`] already
    /// establishes). Built UNCONDITIONALLY at boot (the `forward_pipeline` precedent); recorded
    /// only when [`GBufferScene::path_needs_depth_prepass`] holds (`ForwardPlus`, this rung).
    /// `None` in every test fixture that never resolves a Forward-family path — same `Option`/
    /// `None` rationale as [`Self::forward_pipeline`].
    pub forward_prepass_pipeline: Option<&'a VulkanGraphicsPipeline>,

    // ---- Multi-paradigm render-path plan, rung R-SDFFWD: the SDF forward-march compute pass ---
    //
    // Built UNCONDITIONALLY at `boyko_app::gpu_scene::GpuSceneBundles::boot` (the
    // `forward_pipeline` precedent — cheap to create, no per-frame cost either way), so
    // PRODUCTION always threads `Some(...)` here regardless of `resolved_render_path
    // .sdf_forward_marched`; only recorded when [`GBufferScene::path_has_sdf_forward`] holds
    // (`declare_forward_graph`/`record_forward`). `None` in every test fixture that never
    // resolves a Forward-family path — same `Option`/`None` rationale as [`Self::forward_pipeline`].
    /// The `sdf_forward_march` `HAS_MESH` compute pipeline (`shaders/sdf_forward_march.comp.hlsl`
    /// compiled with `-D HAS_MESH=1`, [`crate::compute::sdf_forward_march_spirv`]): marches the
    /// SDF field, bounds the march at the sampled Forward reverse-Z `forward_depth` (Decision 4's
    /// ownership gate), shades inline, and stores into `lit` (STORAGE). Selected when the
    /// resolved legs carry the mesh leg too (`resolved_render_path.mesh_leg`); paired with
    /// [`Self::sdf_forward_march_sdfonly_pipeline`] for the mesh-less leg set. Both variants are
    /// built against the SAME [`Self::sdf_forward_march_layout`] (the code-review-fixed "one
    /// layout object per pipeline family" discipline [`Self::forward_layout0`] already
    /// establishes) — the mesh-less SPIR-V simply never references the layout's `t12`
    /// (`gForwardDepth`) slot (bound-but-unread, the R2 contract).
    pub sdf_forward_march_pipeline: Option<&'a ComputePipeline>,
    /// The `sdf_forward_march` mesh-less compute pipeline (compiled with no `-D`,
    /// [`crate::compute::sdf_forward_march_sdfonly_spirv`]): never samples `forward_depth` (every
    /// hit is owned — `sdf_owns = hit` unconditionally). Selected when the resolved legs are
    /// exactly `GeometryLegs::Sdf` (`!resolved_render_path.mesh_leg`). See
    /// [`Self::sdf_forward_march_pipeline`]'s doc for the shared-layout discipline.
    pub sdf_forward_march_sdfonly_pipeline: Option<&'a ComputePipeline>,
    /// TAA-under-VB: the `sdf_forward_march` `HAS_MESH + VIEWT` compute pipeline
    /// ([`crate::compute::sdf_forward_march_viewt_spirv`]) — [`Self::sdf_forward_march_pipeline`]
    /// plus the `gViewT` binding-13 write (the marcher IS the composite and the SOLE gViewT
    /// producer on a TAA-armed SDF-carrying VB leg — the `sdf_gbuffer_composite.hlsl` u8
    /// precedent). Selected when [`Self::path_sdf_forward_writes_viewt`] holds AND
    /// `resolved_render_path.mesh_leg`; built against the SAME
    /// [`Self::sdf_forward_march_layout`].
    pub sdf_forward_march_viewt_pipeline: Option<&'a ComputePipeline>,
    /// TAA-under-VB: the `sdf_forward_march` mesh-less `VIEWT` compute pipeline
    /// ([`crate::compute::sdf_forward_march_sdfonly_viewt_spirv`]) —
    /// [`Self::sdf_forward_march_sdfonly_pipeline`] plus the `gViewT` binding-13 write. Selected
    /// when [`Self::path_sdf_forward_writes_viewt`] holds AND `!resolved_render_path.mesh_leg`.
    pub sdf_forward_march_sdfonly_viewt_pipeline: Option<&'a ComputePipeline>,
    /// The `sdf_forward_march` pass's dedicated Set-0 bind-group LAYOUT (14 bindings: edit-list
    /// `Buf` @0, `LightBuf` @1, `Materials` @2, `Camera` UBO @3, `gLit` STORAGE @4,
    /// `PointerGrid`/`BrickAtlas` @5/6, `PointerGrid1`/`BrickAtlas1` @7/8, `PointerGrid2`/
    /// `BrickAtlas2` @9/10, `BrickLevels` UBO @11, `gForwardDepth` SAMPLED @12, `gViewT` STORAGE
    /// @13 — the shader's own binding table doc; @12 is HAS_MESH-referenced only and @13
    /// VIEWT-referenced only, both bound-but-unread by the other variants, the R2 contract, which
    /// is why ONE layout object serves all FOUR `{HAS_MESH} x {VIEWT}` pipelines).
    /// [`GBufferTargets`] writes an `sdf_forward_set` against it (Set 0) once
    /// per extent when [`Self::path_has_sdf_forward`] holds; Set 1 is
    /// [`Self::forward_layout1`] (the Forward-family shadow set, reused VERBATIM — this pass's
    /// own Set-1 binding table is a byte-for-byte copy of `forward_opaque.fs.hlsl`'s).
    pub sdf_forward_march_layout: Option<&'a VulkanBindGroupLayout>,
    /// The `sdf_forward_march` pass's dedicated BrickLevels UBO (Set-0 binding 11, 144 B = 3 ×
    /// 48-byte `M4Level` lanes) — a STANDALONE buffer, DISTINCT from the deferred marcher's own
    /// b5 UBO tail (which packs the SAME `M4Level` array INSIDE the widened camera block): this
    /// pass's own Camera @3 stays the plain 80-byte Forward shape
    /// (`boyko_render::view::forward_gbuffer_push_from_view`'s contract), so the levels need
    /// their own dedicated UBO. Zero-seeded at boot, never rewritten — `brick_enabled =
    /// brick_trilinear = brick_levels = 0` is threaded into every `SdfForwardMarchPush` this
    /// rung (the M1/M2/M4 acceleration is genuinely live in the compiled SPIR-V but dynamically
    /// inactive, mirroring the deferred marcher's own first-landed 0%-gate), so this UBO's
    /// contents are never read (`pc.brick_levels == 0` makes every `m2_levels[...]` access
    /// unreachable). `None` in every test fixture that never resolves a Forward-family path.
    pub brick_levels_ubo: Option<&'a BoundBuffer>,
    /// The host-precomputed `boyko_render::view::forward_view_z_coeffs` `A` coefficient for
    /// this frame's reverse-Z decode (`view_z = B / (depth - A)`) — [`SdfForwardMarchPush::
    /// has_mesh`](crate::compute::SdfForwardMarchPush::has_mesh)'s `view_z_a` argument. Don't-care
    /// under every OTHER leg/path (the mesh-less pipeline variant never reads it; a Deferred scene
    /// never builds this pass at all) — mirrors [`Self::light_dir`]'s always-present, sometimes-
    /// unread threading discipline.
    pub sdf_forward_view_z_a: f32,
    /// The `B` coefficient sibling of [`Self::sdf_forward_view_z_a`].
    pub sdf_forward_view_z_b: f32,

    // ---- Multi-paradigm render-path plan, rung R8: the VisibilityBuffer FUSED v1 path -------
    //
    // Built UNCONDITIONALLY at `boyko_app::gpu_scene::GpuSceneBundles::boot` (the
    // `forward_pipeline` precedent — cheap to create, no per-frame cost either way), so
    // PRODUCTION always threads `Some(...)` here regardless of `resolved_render_path.path`; only
    // a `VisibilityBuffer`-resolved boot ever RECORDS `vb_raster`/`vb_resolve`
    // (`declare_vb_graph`/`record_vb`). `None` in every test fixture that never resolves VB —
    // same `Option`/`None` rationale as [`Self::forward_pipeline`].
    /// The `vb_raster` mesh id-raster pipeline (`vb_raster.{vs,fs}.hlsl`): writes ONLY
    /// `SV_Target0 = uint2(instance_id, raw SV_PrimitiveID)` into the `vb_id` `R32G32_UINT`
    /// color attachment, `VK_COMPARE_OP_GREATER` HW reverse-Z depth-write ON (Decision 4/9). A
    /// plain 1-Vulkan-set pipeline built against [`Self::vb_layout0`] (its VS references only
    /// the `instances`/`Camera` subset — the SAME bound-but-unread-subset idiom
    /// [`Self::forward_sky_pipeline`] establishes).
    pub vb_raster_pipeline: Option<&'a VulkanGraphicsPipeline>,
    /// The VB v1 sky BACKGROUND pipeline — REUSES the EXISTING compiled `forward_sky.{vs,fs}.hlsl`
    /// SPIR-V verbatim (byte-identical shader modules to [`Self::forward_sky_pipeline`]'s own),
    /// built as a NEW pipeline OBJECT against [`Self::vb_layout0`] (a DIFFERENT descriptor-set
    /// layout object than [`Self::forward_layout0`] — Vulkan pipeline-layout compatibility needs
    /// the SAME layout object the pipeline was built with, so a new object is required even
    /// though the shader source is unchanged). Reuse is sound because `vb_layout0` places
    /// `Camera`/`LightBuf` at the SAME binding numbers (2/3) the sky FS's fixed SPIR-V already
    /// references (`vb_geom_fetch.hlsli`'s Set-numbering doc explains the analogous discipline).
    pub vb_sky_pipeline: Option<&'a VulkanGraphicsPipeline>,
    /// The `vb_resolve` FUSED compute pipeline (`vb_resolve.comp.hlsl`, `mesh_geo_shade_split ==
    /// false` — SSAO/DDGI/shadow-denoise/TAA capped off this rung, `cap_vb_v1_consumers`): unpacks
    /// `vb_id`, re-fetches the covered triangle's geometry via the Decision-0 table (Set 2), shades
    /// ALL-LIGHTS, and writes `lit` (STORAGE). A 3-Vulkan-set pipeline: Set 0 = [`Self::vb_layout0`],
    /// Set 1 = [`Self::forward_layout1`] (the shadow set, REUSED VERBATIM — the SAME reuse
    /// `sdf_forward_march_pipeline` already establishes), Set 2 = [`Self::vb_geometry_set`]'s
    /// layout.
    pub vb_resolve_pipeline: Option<&'a ComputePipeline>,
    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2a (dark infra,
    /// unwired): the `count` classify compute pipeline (`vb_classify_count.comp.hlsl`) — a
    /// 1-Vulkan-set pipeline built against [`Self::vb_layout0`]. `Some` only AFTER
    /// `GpuSceneBundles::build_vb_classify_pipelines` ran (the SAME `Option` shape as
    /// [`Self::vb_resolve_pipeline`]). UNREAD this rung — `record_vb`/`declare_vb_graph` are
    /// untouched; threaded here so a later rung (P2b/P2c) needs no further plumbing.
    pub vb_classify_count_pipeline: Option<&'a ComputePipeline>,
    /// The `scan` classify compute pipeline (`vb_classify_scan.comp.hlsl`) — a 1-Vulkan-set
    /// pipeline, the SAME `Option`/UNREAD rationale as [`Self::vb_classify_count_pipeline`].
    pub vb_classify_scan_pipeline: Option<&'a ComputePipeline>,
    /// The `scatter` classify compute pipeline (`vb_classify_scatter.comp.hlsl`) — a
    /// 1-Vulkan-set pipeline, the SAME `Option`/UNREAD rationale as
    /// [`Self::vb_classify_count_pipeline`].
    pub vb_classify_scatter_pipeline: Option<&'a ComputePipeline>,
    /// The `vb_shade` material-classified shading compute pipeline (`vb_shade.comp.hlsl`) — a
    /// 3-Vulkan-set pipeline (Set 0 = [`Self::vb_layout0`], Set 1 = [`Self::forward_layout1`],
    /// Set 2 = [`Self::vb_geometry_set`]'s layout), the SAME `Option`/UNREAD rationale as
    /// [`Self::vb_classify_count_pipeline`].
    pub vb_shade_pipeline: Option<&'a ComputePipeline>,
    /// The VB-only Set-0 (core + images + classify) bind-group LAYOUT — 8 bindings:
    /// `gVbInstances` @0 (VERTEX+COMPUTE, [`Self::vb_instance_ring`]), `instance_materials` @1
    /// (COMPUTE, [`Self::forward_instance_material_ring`] — the SAME per-instance-material ring
    /// the Forward family already threads, indexed identically by global instance id), `Camera`
    /// @2 (VERTEX+COMPUTE+FRAGMENT, [`Self::camera_ring`]), `LightBuf` @3 (COMPUTE+FRAGMENT,
    /// [`Self::light_table`]), `Materials` @4 (COMPUTE, [`Self::material_table`]), `gVbId` @5
    /// (COMPUTE, SAMPLED, [`VbTargets::vb_id`](super::targets::VbTargets::vb_id)), `gLit` @6
    /// (COMPUTE, STORAGE, the shared `lit` target) — see `vb_resolve.comp.hlsl`'s own binding
    /// table doc. `gClassify` @7 (COMPUTE, STORAGE_BUFFER, the packed classify buffer — VB-P2
    /// classification plan rung P2a; bound-but-unread by every pipeline's frozen SPIR-V except
    /// the classify/`vb_shade` family). [`GBufferTargets`] writes the per-FIF bind group against
    /// this layout once per extent (the `forward_layout0` precedent). `None` rationale as
    /// [`Self::forward_pipeline`].
    pub vb_layout0: Option<&'a VulkanBindGroupLayout>,
    /// The RAW per-FIF `VbInstanceRow` (64 B) SSBO ring — Decision 0's VB-path instance row
    /// (byte-identical leading 48 bytes to `InstanceModelCol`, plus an appended `mesh_id` lane).
    /// A DEDICATED ring, distinct from [`Self::forward_instance_ring`] (`InstanceModelCol`, 48 B)
    /// — built from `boyko_render::mesh_draw::MeshRenderScratch::vb_ring`, uploaded ONLY on a
    /// `VisibilityBuffer`-resolved boot. `None` rationale as [`Self::forward_pipeline`].
    pub vb_instance_ring: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Rung R2a': the per-FIF `VkDrawIndexedIndirectCommand` record buffer the VB id-raster's
    /// indirect draws fetch. `Some` on every VB boot; `None` degrades the recorder to the direct
    /// `vkCmdDrawIndexed` path it replaced, so a boot that failed to build it still renders.
    pub vb_indirect: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the per-FIF [`VbBatchDesc`] buffer the batch-cull compute pass reads. `Some`
    /// exactly when [`Self::vb_batch_cull_pipeline`] and the other two cull buffers are — the
    /// whole R2c0 arm is one all-or-nothing gate, so no half-wired frame can dispatch a cull that
    /// reads an unbound descriptor.
    pub vb_batch_desc: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the per-FIF compacted visible-batch list. Written by the cull; read only by
    /// the rung-R2c-tail readback probe ([`Self::vb_cull_readback`]), which copies its prefix to
    /// the host so a test can assert WHICH batches survived. No RENDER pass consumes it — that
    /// needs `multiDrawIndirect` plus a merged vertex/index arena, neither of which this
    /// device/engine has.
    pub vb_cull_visible: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the per-FIF visible-batch counter (one `u32` at element 0), transfer-zeroed
    /// each frame ahead of the graph-derived `TRANSFER → COMPUTE` barrier.
    pub vb_cull_count: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c-tail: the per-FIF HOST-VISIBLE staging the cull's outputs are copied into —
    /// `Some` only under the `BOYKO_VB_CULL_READBACK` probe, `None` on every golden/interactive
    /// boot (so no readback pass is declared and no command is recorded).
    ///
    /// The staging is what is host-visible; [`Self::vb_cull_count`] and [`Self::vb_cull_visible`]
    /// stay DEVICE_LOCAL exactly as they ship. Probing a copy rather than relocating the counter
    /// is the difference between proving the cull in the configuration that renders and proving it
    /// in one nobody does.
    pub vb_cull_readback: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// VG rung R2c0: the batch-cull compute pipeline (`vb_batch_cull.comp.hlsl`), built against
    /// [`Self::vb_cull_layout`]. `None` degrades the recorder to R2a''s transfer-only record
    /// path, which is byte-identical to this one by construction.
    pub vb_batch_cull_pipeline: Option<&'a ComputePipeline>,
    /// VG rung R2c: the six camera-frustum planes, `(a, b, c, d)` with **inside ⇒
    /// `a·x + b·y + c·z + d ≥ 0`**, in the fixed order left, right, bottom, top, near, far that
    /// `vb_batch_cull.comp.hlsl` mirrors.
    ///
    /// Extracted host-side by `boyko_render::frustum::frustum_planes_from_push_bytes` **from the
    /// first 64 bytes of [`Self::mvp`]** — the same bytes the raster's vertex shader reads as its
    /// `view_proj`. Not re-derived from the camera UBO (which holds basis vectors, not a matrix)
    /// and not taken from a separately computed matrix: the TAA path jitters the projection per
    /// frame, and byte provenance is what keeps the cull and the raster on one matrix without
    /// anyone having to remember to update two places.
    ///
    /// `None` DISARMS the decision — the recorder then pushes planes that reject nothing, so the
    /// cull degrades to the rung-R2c0 null control rather than to an unbounded one. This crate
    /// cannot compute them itself (`boyko_render` sits ABOVE it in the dependency graph), which is
    /// exactly why they arrive as data rather than as a call.
    pub vb_cull_planes: Option<[[f32; 4]; 6]>,
    /// VG rung R2c0: the batch-cull's OWN 1-set bind-group layout (`VbIndirect` @0,
    /// `VbBatchDesc` @1, `VbCullVisible` @2, `VbCullCount` @3 — all COMPUTE, all STORAGE_BUFFER,
    /// matching `vb_batch_cull.comp.hlsl`'s binding table).
    ///
    /// A DEDICATED layout rather than four more bindings on `vb_layout0`, and the reason is
    /// structural: `vb_layout0_froxel` already occupies @8/@9 with the froxel pair, so appended
    /// bindings would land on DIFFERENT numbers in the two layouts and no single compiled module
    /// could name both. The [`Self::cull_layout`] precedent — the L1 cull's own 1-set pipeline —
    /// is the shape this follows.
    pub vb_cull_layout: Option<&'a VulkanBindGroupLayout>,
    /// The Decision-0 bindless per-mesh geometry table's OWN Set (`gMeshVerts[]`/
    /// `gMeshIndices[]`/`gMeshMeta` — `boyko_render::mesh_geometry_table::MeshGeometryTable::set()`),
    /// threaded down as the raw low-level type (this crate cannot depend on `boyko_render`, which
    /// sits ABOVE it in the dependency graph — the SAME plain-reference boundary crossing
    /// [`Self::resolved_render_path`]'s doc explains for `ResolvedRenderPathGpu`). `Some` only
    /// when `ResolvedRenderPath::vb_geometry_table`
    /// is armed (a live `MeshGeometryTable` exists); `None` otherwise (including on a device that
    /// lacks the descriptor-indexing prerequisite — VB itself degrades to `Deferred` at resolve
    /// time in that case, so this field is never read on such a boot).
    pub vb_geometry_set: Option<&'a VulkanGeometryBindlessSet>,
    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2b: the classify
    /// `scan` pass's `present_material_count` push-constant LOOP BOUND (`vb_classify_scan.comp
    /// .hlsl`'s `PushConstants.material_count` — never an OFFSET into `gClassify`, see that
    /// file's + `vb_classify_common.hlsli`'s doc). Sourced from
    /// `boyko_render::material_table::MaterialTable::capacity_rows()` (the material table's row
    /// capacity, NOT the frame's distinct material-id count the plan's D2 describes — a P2b
    /// simplification: `capacity_rows` is a valid, always-safe upper bound because every live
    /// `MaterialId` is `< capacity_rows` by construction, so `scan`'s `[0, material_count)` sweep
    /// still folds every material id the frame could reference). This crate cannot depend on
    /// `boyko_render` (which sits ABOVE it in the dependency graph — the SAME plain-value
    /// boundary crossing [`Self::resolved_render_path`]'s doc explains), so this is threaded as a
    /// plain `u32`, mirroring [`Self::dispatch_group_count_x`]'s own threading. Read at TWO
    /// `record_vb` sites within the `mesh_leg` gate: the `scan` pass's push constant
    /// (unconditional under `mesh_leg`, rung P2b) and — rung P2c — `vb_shade`'s over-dispatch
    /// group count (`dispatch_group_count_x + vb_classify_material_count`, plan D2), the latter
    /// ONLY under [`Self::vb_use_classified`].
    pub vb_classify_material_count: u32,
    /// VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2c (the P1-4
    /// owner-decided selector): `true` iff THIS frame's `lit` producer is the material-classified
    /// `vb_shade` pipeline (classify passes + `vb_shade`) instead of the fused `vb_resolve` — the
    /// SINGLE source both `Renderer::declare_vb_graph`
    /// and `Renderer::record_vb` read (the SAME W1 "declare/record parity" discipline every other
    /// per-frame selector in this file follows, e.g. [`Self::mesh_tex_active`]). Computed once per
    /// frame at the `GpuSceneBundles::scene()` assembly seam: `force ||
    /// vb_tex_active_this_frame`, where `force` is the `BOYKO_VB_FORCE_CLASSIFIED` dev/golden
    /// env var (the orchestrator's channel to exercise `vb_shade` on real hardware — mirrors
    /// `boyko_app::plugins`'s `BOYKO_AA`/`BOYKO_RENDER_PATH` launch-env seam) and (rung TV0)
    /// `vb_tex_active_this_frame` is [`Self::vb_tex_active`]'s own per-frame gather ("any VB
    /// instance bound a non-zero material texture slot this frame" — the VB-specific sibling of
    /// [`Self::mesh_tex_active`]). So in production `vb_use_classified` is `true` whenever a
    /// textured material is in play under VB (landing textures on the classified pipeline,
    /// TV0's whole point) OR `BOYKO_VB_FORCE_CLASSIFIED=1`; the fast fused `vb_resolve` still
    /// shades every OTHER (flat) VB frame (P1-4's perf point — flat scenes pay zero classify
    /// tax). Gates BOTH the classify passes (`fill`/`count`/`scan`/`scatter` —
    /// `mesh_leg && vb_use_classified`) and the `lit`-producer choice (`vb_shade`/`vb_shade_tex`
    /// when `true`, `vb_resolve` when `false`) — exactly one of the two ever produces `lit` per
    /// frame (mutually exclusive by construction, mirroring
    /// [`Self::path_has_raster`]/[`Self::path_has_mesh_depth_neutral_clear`]'s own partition).
    pub vb_use_classified: bool,
    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): the `vb_shade` TEXTURED-variant
    /// shading compute pipeline (`vb_shade.comp.hlsl`, `-D TEXTURED=1`) — a 4-Vulkan-set
    /// pipeline (Set 0 = [`Self::vb_layout0`], Set 1 = [`Self::forward_layout1`], Set 2 =
    /// [`Self::vb_geometry_set`]'s layout, Set 3 = the shared bindless texture-array table's
    /// layout — [`Self::bindless_set`]'s own object). `Some` only AFTER
    /// `GpuSceneBundles::build_vb_shade_textured_pipeline` ran (needs BOTH the Decision-0
    /// geometry table's Set-2 layout AND the bindless table's Set-3 layout — neither exists at
    /// `GpuSceneBundles::boot`'s call site, the SAME two-dependency deferred-build reason
    /// [`Self::vb_shade_pipeline`]/[`Self::raster_pipeline_tex`] are each individually `Option`).
    pub vb_shade_tex_pipeline: Option<&'a ComputePipeline>,
    /// Textured-PBR rung TV0: the RAW per-FIF TEXTURED instance-material SSBO ring
    /// (`PerInstanceMaterialTex`, 48 B/instance) — the SAME ring
    /// [`Self::raster_pipeline_tex`]/`tex_bind_group` bind at their own Set-0 binding 1
    /// (`boyko_app::gpu_scene::TexturedResources::tex_instance_material_rings`), exposed raw so
    /// [`GBufferTargets`](super::targets::GBufferTargets) can fold it into the VB TEXTURED
    /// Set-0 bind group (`vb_set0_tex`) at binding 1 — a DIFFERENT buffer than
    /// [`Self::forward_instance_material_ring`] (`PerInstanceMaterial`, 32 B) `vb_set0`'s own
    /// binding 1 points at, bound against the SAME `vb_layout0` layout object (Vulkan's
    /// `STORAGE_BUFFER` binding shape carries no element-stride constraint — R5's "one shared
    /// layout, a distinct set" rule, not "one shared set"). `Some` iff the TEXTURED resources
    /// exist (`GpuSceneBundles::build_textured_resources` ran) — device-agnostic, unconditioned
    /// on `resolved_render_path.path` (mirrors [`Self::forward_instance_material_ring`]'s own
    /// unconditional-once-built threading).
    pub vb_tex_instance_material_ring: Option<&'a [BoundBuffer; FRAMES_IN_FLIGHT]>,

    // ---- VB-P1a ("dark infra"): the froxel light-cull machinery, gated on the single boot-frozen
    // arm bit `ResolvedRenderPath::froxel_light_cull` (⚠️ default-OFF via the owner's
    // `LightingConfig::clusters_enabled`, NOT hardcoded off — every field below stays `None` on an
    // unarmed boot, so nothing is built/selected/recorded there; `vb_mesh_froxel` arms them). See
    // `GpuSceneBundles::build_froxel_light_cull`'s doc for the app-side build this feeds. -------
    /// The froxel-only Set-0 bind-group LAYOUT — 10 bindings: [`Self::vb_layout0`]'s own 0..7
    /// PLUS `ClusterGrid` @8 (COMPUTE, STORAGE_BUFFER) + `LightIndexList` @9 (COMPUTE,
    /// STORAGE_BUFFER) — matching `vb_resolve.comp.hlsl`'s/`vb_shade.comp.hlsl`'s own `-D FROXEL`
    /// binding table doc. A DISTINCT layout OBJECT from [`Self::vb_layout0`] (Vulkan pipeline
    /// layouts are structurally compared; a 10-binding layout is never compatible with an
    /// 8-binding one), never a second Set — the froxel VB pipelines are built against THIS
    /// object, `vb_layout0` itself stays UNCHANGED (8 bindings, byte-identical descriptor-set
    /// shape). `None` unless `ResolvedRenderPath::froxel_light_cull`'s gate is armed.
    pub vb_layout0_froxel: Option<&'a VulkanBindGroupLayout>,
    /// The `vb_resolve` FROXEL-variant compute pipeline (`vb_resolve.comp.hlsl`, `-D FROXEL=1`) —
    /// the SAME 3-Vulkan-set shape as [`Self::vb_resolve_pipeline`] (Set 0 =
    /// [`Self::vb_layout0_froxel`], Set 1 = [`Self::forward_layout1`], Set 2 =
    /// [`Self::vb_geometry_set`]'s layout), built against the WIDER Set-0 layout. `None` unless
    /// the froxel arm is built.
    pub vb_resolve_froxel_pipeline: Option<&'a ComputePipeline>,
    /// The `vb_shade` FROXEL-variant compute pipeline (`vb_shade.comp.hlsl`, `-D FROXEL=1`) — the
    /// SAME 3-Vulkan-set shape as [`Self::vb_shade_pipeline`], built against
    /// [`Self::vb_layout0_froxel`]. `None` unless the froxel arm is built.
    pub vb_shade_froxel_pipeline: Option<&'a ComputePipeline>,
    /// The `vb_shade` TEXTURED+FROXEL-variant compute pipeline (`vb_shade.comp.hlsl`, `-D
    /// TEXTURED=1 -D FROXEL=1`) — the SAME 4-Vulkan-set shape as [`Self::vb_shade_tex_pipeline`],
    /// built against [`Self::vb_layout0_froxel`]. `None` unless the froxel arm is built.
    pub vb_shade_tex_froxel_pipeline: Option<&'a ComputePipeline>,

    // ---- Rung R9b: the VB geo/shade SPLIT pair + its SSAO gather (docs/R9-VB-SPLIT-PLAN.md) --
    /// The `vb_geo` thin-aux geometry compute pipeline (`vb_geo.comp.hlsl`): the split's
    /// producer half — fetch + interp via the Decision-0 geometry table, writing `thin_normal`
    /// (oct RG + roughness B). Set 0 = [`Self::vb_layout0`] (reused), Set 1 =
    /// [`Self::vb_geo_aux_layout`], Set 2 = the geometry table. `Some` only after the deferred
    /// build hook ran (the SAME two-dependency reason as [`Self::vb_shade_pipeline`]).
    pub vb_geo_pipeline: Option<&'a ComputePipeline>,
    /// The `vb_shade_split` lit-producer compute pipeline (`vb_shade_split.comp.hlsl`, base
    /// variant): the split's consumer half — RE-fetch + shade (the `vb_resolve` tail
    /// character-identical) + `gSsao` Filament combine (+ DDGI/hwrt-vis consumption as their
    /// rungs land). Set 1 = [`Self::vb_split_layout1`] (NOT `forward_layout1`).
    pub vb_shade_split_pipeline: Option<&'a ComputePipeline>,
    /// The `-D TEXTURED=1` sibling of [`Self::vb_shade_split_pipeline`] (Set 3 = the bindless
    /// table; the per-frame base/_tex choice mirrors the fused `vb_resolve`/`vb_shade_tex`
    /// selection — boot-frozen split arming, per-frame texture pick).
    pub vb_shade_split_tex_pipeline: Option<&'a ComputePipeline>,
    /// The `vb_geo` pass's Set-1 aux LAYOUT: `thin_normal` STORAGE @0 (+ the R9d
    /// `motion`/`MotionCam` slots, placeholder-bound until then).
    pub vb_geo_aux_layout: Option<&'a VulkanBindGroupLayout>,
    /// The `vb_shade_split` pass's Set-1 LAYOUT (9 bindings; 8 on the software leg): @0-3 =
    /// `forward_layout1`'s shadow table verbatim, @4 `gSsao` STORAGE, @5 `ddgi_irr` COMBINED
    /// image+sampler, @6 `ddgi_depth` COMBINED, @7 `ResolvedDdgi` UBO, @8 cfg(hwrt)
    /// `gShadowVis` STORAGE (hwrt-declared only — the software `.spv` never references it, so
    /// the software layout simply omits the entry, an exact fill). A DISTINCT layout object so
    /// [`Self::forward_layout1`] (the Forward family + `vb_resolve`) stays byte-untouched.
    pub vb_split_layout1: Option<&'a VulkanBindGroupLayout>,
    /// The ACTIVE `-D VB_THIN=1` SSAO gather pipeline (the quality-variant selection happens at
    /// the `scene()` assembly seam, where the freeze-clamped `ResolvedSsao::variant` index is in
    /// scope — the recorder just binds it): the VB split's gather reads `thin_normal` + `gViewT`
    /// (background = the `1e30` sentinel) instead of the Deferred `gNormal`/`gMaterial` pair.
    /// `Some` exactly when [`Self::ssao`] is armed on a VB boot.
    pub ssao_vb_pipeline: Option<&'a ComputePipeline>,
    /// The VB SSAO gather's dedicated 4-binding LAYOUT: `thin_normal` @0, `gViewT` @1,
    /// `ssao` @2 (write), Camera UBO @3 — the `-D VB_THIN` dense table.
    pub vb_ssao_layout: Option<&'a VulkanBindGroupLayout>,

    // ---- Rung R9d: the VB hardware shadow chain (docs/R9-VB-SPLIT-PLAN.md §6) --------------
    /// The VB split's DEDICATED hardware shadow-vis gather pipeline (`vb_shadow_vis.comp` /
    /// [`crate::compute::vb_shadow_vis_spirv`]) — the split's own standalone sibling of
    /// [`ShadowVisActivation::vis_pipeline`] (that one re-runs the FUSED deferred resolve's
    /// front-matter against the fat G-buffer; this one has no gbuffer to read, so it traces
    /// against `thin_normal`/`gViewT` instead). `Some` after `GpuSceneBundles::boot`'s hwrt gate
    /// (`ctx.ray_query_enabled()` + the shadow-denoise storage probe) built it; `None` on a
    /// software / non-RT device.
    #[cfg(feature = "hwrt")]
    pub vb_shadow_vis_pipeline: Option<&'a ComputePipeline>,
    /// The 7-binding bind-group LAYOUT [`Self::vb_shadow_vis_pipeline`] declares at set 0
    /// (`thin_normal` STORAGE read @0, `gViewT` STORAGE read @1, `LightTable` STORAGE read @2,
    /// the camera UBO @3, the TLAS `AccelerationStructure` @4, the `ResolvedRayShadow` UBO @5,
    /// `gShadowVis` STORAGE write @6). `Some` iff [`Self::vb_shadow_vis_pipeline`] is `Some`.
    #[cfg(feature = "hwrt")]
    pub vb_shadow_vis_layout: Option<&'a VulkanBindGroupLayout>,
    /// The `-D MOTION=1` sibling of [`Self::vb_geo_pipeline`] (`vb_geo_mv.comp` /
    /// [`crate::compute::vb_geo_mv_spirv`]) — selected instead of it when
    /// [`Self::vb_geo_mv_active`] holds this frame. `Some` after the deferred build hook ran on
    /// an RT + storage device (the SAME two-dependency reason [`Self::vb_geo_pipeline`] is
    /// `Option`).
    #[cfg(feature = "hwrt")]
    pub vb_geo_mv_pipeline: Option<&'a ComputePipeline>,
    /// The `-D HWRT=1` sibling of [`Self::vb_shade_split_pipeline`]
    /// (`vb_shade_split.comp.hlsl -D HWRT=1` / [`crate::compute::vb_shade_split_hwrt_spirv`]) —
    /// reads the denoised/undenoised `gShadowVis` (`vb_split_layout1`'s hwrt @8 entry) instead of
    /// the software shadow term. Selected instead of [`Self::vb_shade_split_pipeline`] only when
    /// [`Self::path_vb_hwrt_shadow`] holds.
    #[cfg(feature = "hwrt")]
    pub vb_shade_split_hwrt_pipeline: Option<&'a ComputePipeline>,
    /// The `-D TEXTURED=1 -D HWRT=1` sibling of [`Self::vb_shade_split_tex_pipeline`]
    /// (/ [`crate::compute::vb_shade_split_tex_hwrt_spirv`]) — the textured-PBR counterpart of
    /// [`Self::vb_shade_split_hwrt_pipeline`].
    #[cfg(feature = "hwrt")]
    pub vb_shade_split_tex_hwrt_pipeline: Option<&'a ComputePipeline>,
}

impl GBufferScene<'_> {
    /// HW-RT Rung 3b step 5a — the SINGLE source of the "the raster pass writes the mesh
    /// motion-vector 4th MRT this frame" decision, so the framegraph barrier declaration
    /// (`declare_deferred_graph`) and the draw recording (`record_gbuffer`) can never diverge.
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

    /// Textured-PBR T6c — the SINGLE source of the "this frame draws with the TEXTURED
    /// gbuffer pipeline" decision, so the framegraph barrier declaration
    /// (`declare_deferred_graph`) and the draw recording (`record_gbuffer`) can never diverge
    /// (the W1 lesson, mirroring [`Self::mesh_mv_active`]). NOT `#[cfg(feature = "hwrt")]` —
    /// TEXTURED works on the software leg (materials/textures are device-agnostic, like PM).
    ///
    /// True iff [`Self::tex_enabled`] AND the TEXTURED pipeline + this frame's TEX bind
    /// group + the bindless descriptor set all exist AND no MV frame is active this frame
    /// (TEXTURED is NEVER compiled with MOTION_VECTORS — T6c plan Decision D4; under active
    /// temporal denoise a textured material renders base_color/scalar through the MV/mvpm
    /// pipeline instead, with a one-time host-side warning at the recorder's selection
    /// site).
    pub(crate) fn mesh_tex_active(&self) -> bool {
        #[cfg(feature = "hwrt")]
        let mv = self.mesh_mv_active();
        #[cfg(not(feature = "hwrt"))]
        let mv = false;
        !mv
            && self.tex_enabled
            && self.raster_pipeline_tex.is_some()
            && self.tex_bind_group.is_some()
            && self.bindless_set.is_some()
    }

    /// Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3) — the VB sibling of
    /// [`Self::mesh_tex_active`]: the SINGLE source of the "this VB frame's material eval needs
    /// the TEXTURED `vb_shade` pipeline" decision, feeding [`Self::vb_use_classified`]'s OR-in at
    /// the `GpuSceneBundles::scene()` assembly seam (so the classify-chain gate, the `lit`-
    /// producer choice, AND the Set-0/Set-3 pipeline selection can never disagree — the SAME W1
    /// discipline [`Self::vb_use_classified`]'s own doc explains).
    ///
    /// True iff [`Self::tex_enabled`] (this frame's `any_textured_material` gather — device- and
    /// path-agnostic) AND the TEXTURED `vb_shade` pipeline, the TEXTURED instance-material ring,
    /// and the bindless descriptor set all exist. Unlike [`Self::mesh_tex_active`] there is no
    /// motion-vector exclusion — VB v1 has no motion-vector consumer at all
    /// (`cap_vb_v1_consumers`).
    pub(crate) fn vb_tex_active(&self) -> bool {
        self.tex_enabled
            && self.vb_shade_tex_pipeline.is_some()
            && self.vb_tex_instance_material_ring.is_some()
            && self.bindless_set.is_some()
    }

    /// F8-mv — the SINGLE source of the "this frame draws with the combined
    /// MOTION_VECTORS + PER_INSTANCE_MATERIAL pipeline" decision, so the raster pipeline/set
    /// selection (`record_gbuffer`) can never diverge between the pipeline and bind-group
    /// arms. `#[cfg(feature = "hwrt")]` — mvpm is an MV extension, and MV is hwrt-only.
    ///
    /// True iff BOTH [`Self::mesh_mv_active`] and [`Self::mesh_pm_active`] hold AND the mvpm
    /// pipeline + this frame's mvpm bind group both exist (an RT + storage device built them
    /// at boot). `mesh_mv_active()`/`mesh_pm_active()` stay SUPERSETS — unchanged by this
    /// method — and mutual exclusion between the mv-only / pm-only / mvpm arms is enforced by
    /// the recorder's `if mvpm_active { .. } else if mv_active { .. } else if pm_active { .. }`
    /// ordering (mvpm checked FIRST).
    #[cfg(feature = "hwrt")]
    pub(crate) fn mesh_mvpm_active(&self) -> bool {
        self.mesh_mv_active()
            && self.mesh_pm_active()
            && self.raster_pipeline_mvpm.is_some()
            && self.mvpm_bind_group.is_some()
    }

    /// HW-RT Rung 3b step 5b — the SINGLE source of the "the VIS pass ALSO writes the SDF pixels'
    /// camera-only motion vector to `motion_vec` this frame" decision, so the framegraph barrier
    /// declaration (`declare_deferred_graph`) and the VIS-pass recording (`record_gbuffer`) can never
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
    /// this frame" decision, so the framegraph declaration (`declare_deferred_graph`: the temporal
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

    /// Multi-paradigm render-path plan, rung R2 (Decision 2 / O1) — the SINGLE source of "does
    /// this frame's declarator/recorder emit the mesh raster pass" decision, so
    /// `declare_deferred_graph`'s `raster` pass declaration and `record_gbuffer`'s raster
    /// begin/end-rendering block can never diverge (the W1 lesson, mirroring
    /// [`Self::mesh_mv_active`]).
    ///
    /// `== resolved_render_path.mesh_leg`. Rung R3 lifted the SDF-only leg-disable
    /// (`boyko_render::render_path_config`), so `Deferred × Sdf` reaches here with this `false`
    /// — see [`Self::path_has_mesh_depth_neutral_clear`] for the pass that replaces the raster
    /// pass's depth-clear producer on that leg. Rung R3b lifted the mesh-only leg-disable too
    /// (the `viewt_from_depth` producer, [`GBufferScene::viewt_from_depth`]), so this is `true`
    /// on every reachable `Deferred` frame EXCEPT `Deferred × Sdf`.
    #[inline]
    pub(crate) fn path_has_raster(&self) -> bool {
        self.resolved_render_path.mesh_leg
    }

    /// Sibling of [`Self::path_has_raster`] for the SDF marcher pass — `== resolved_render_path
    /// .sdf_leg`. See [`Self::path_has_raster`]'s doc for the R3 guard state.
    #[inline]
    pub(crate) fn path_has_marcher(&self) -> bool {
        self.resolved_render_path.sdf_leg
    }

    /// Multi-paradigm render-path plan, rung R3 (§E leg-disable / the O2 audit finding) — the
    /// SINGLE source of "does this frame's declarator/recorder emit the mesh-depth NEUTRAL
    /// CLEAR pass" decision, so `declare_deferred_graph`'s `mesh_depth_neutral_clear` pass
    /// declaration and `record_gbuffer`'s depth-only clear block can never diverge (the W1
    /// lesson, mirroring [`Self::path_has_raster`]).
    ///
    /// `== sdf_leg && !mesh_leg` (`GeometryLegs::Sdf` exactly) — mutually exclusive with
    /// [`Self::path_has_raster`] by construction, since the two predicates partition
    /// `mesh_leg`'s two states. Under `Deferred × Sdf` the raster pass (the depth image's normal
    /// clear + `DEPTH_ATTACHMENT_OPTIMAL` producer) is skipped, so nothing gives the marcher's
    /// `gDepth.Load` at binding 1 a defined value; this pass reproduces JUST the depth half of
    /// the raster pass's own clear (`CLEAR` to the far-plane sentinel, `DEPTH_ATTACHMENT_OPTIMAL`)
    /// so the marcher deterministically reads "no mesh" for every pixel — the SAME code path an
    /// entirely mesh-less scene already exercises byte-identically today
    /// (`sdf_gbuffer_composite.hlsl`'s own documented 0%-gate), with ZERO shader change.
    #[inline]
    pub(crate) fn path_has_mesh_depth_neutral_clear(&self) -> bool {
        self.resolved_render_path.sdf_leg && !self.resolved_render_path.mesh_leg
    }

    /// Multi-paradigm render-path plan, rung R4b-b (widened at rung R5) — the SINGLE source of
    /// "is this frame's declarator/recorder the Forward FAMILY" decision, so
    /// `declare_frame_graph`'s dispatch and `render_gbuffer_frame`'s record-site dispatch
    /// (`record_forward` vs `record_gbuffer`) can never diverge (the W1 lesson, mirroring
    /// [`Self::path_has_raster`]). `RenderPath::Forward` is discriminant `1`,
    /// `RenderPath::ForwardPlus` is discriminant `2` (`boyko_render::render_path_config
    /// ::RenderPath`) — `ForwardPlus` reuses `declare_forward_graph`/`record_forward`/
    /// `ForwardTargets` verbatim (Decision 2's per-path declarator is shared, not duplicated;
    /// the two paths diverge only in WHICH passes/pipelines that shared machinery selects).
    #[inline]
    pub(crate) fn path_is_forward(&self) -> bool {
        matches!(self.resolved_render_path.path, 1 | 2)
    }

    /// Multi-paradigm render-path plan, rung R5 — `true` iff the resolved path is EXACTLY
    /// `RenderPath::ForwardPlus` (discriminant `2`), NOT plain `Forward`. The single source of
    /// the `light_cull` pass's path-level gate, used at BOTH `declare_forward_graph` and
    /// `record_forward` (the O1 single-predicate rule): the base `Forward` pipeline's Set 0 has
    /// no `ClusterGrid`/`LightIndexList` bindings, so `light_cull` must never be declared OR
    /// recorded under plain `Forward` even if a scene fixture happens to wire the cull
    /// pipeline/buffers (a hand-built test harness, e.g. — production never does this today).
    #[inline]
    pub(crate) fn path_is_forward_plus(&self) -> bool {
        self.resolved_render_path.path == 2
    }

    /// Multi-paradigm render-path plan, rung R5 (ForwardPlus) — the SINGLE source of "does this
    /// frame need the `depth_prepass` pass" decision (Decision 4's EQUAL-depth early-Z contract),
    /// used at BOTH `declare_forward_graph` and `record_forward` (the O1 single-predicate rule, a
    /// declare/record parity `debug_assert!` guards it). A plain field read of the boot-resolved
    /// carrier — `ResolvedRenderPath::needs_depth_prepass` is `true` for `ForwardPlus`
    /// UNCONDITIONALLY (`resolve_rules`), and for `Forward` only when a pre-light consumer is
    /// armed, which `cap_forward_v1_consumers` still forces off this rung (SCOPE: the prepass
    /// lands for ForwardPlus's zero-overdraw early-Z, not for consumer wiring yet) — so in
    /// practice this predicate is `true` iff `ForwardPlus`.
    #[inline]
    pub(crate) fn path_needs_depth_prepass(&self) -> bool {
        self.resolved_render_path.needs_depth_prepass
    }

    /// Multi-paradigm render-path plan, rung R-SDFFWD — the SINGLE source of "does this frame
    /// need the `sdf_forward_march` pass" decision, used at BOTH `declare_forward_graph` and
    /// `record_forward` (the O1 single-predicate rule, mirroring [`Self::path_needs_depth_prepass`]).
    /// `== resolved_render_path.sdf_forward_marched` (`sdf_leg && path != Deferred` —
    /// `resolve_rules`'s doc): `true` for every Forward-family resolve carrying the SDF leg
    /// (`GeometryLegs::Both` or `Sdf`), regardless of `mesh_leg` (the `HAS_MESH`/mesh-less
    /// pipeline variant selection is a SEPARATE decision, keyed off `mesh_leg` directly at the
    /// record site).
    #[inline]
    pub(crate) fn path_has_sdf_forward(&self) -> bool {
        self.resolved_render_path.sdf_forward_marched
    }

    /// TAA-under-VB — the SINGLE source of "does this frame's `sdf_forward_march` dispatch write
    /// the `gViewT` lane" decision, read at BOTH `declare_vb_graph` (the pass's conditional
    /// `viewt` GENERAL-write access) and `record_vb` (the `VIEWT`-variant pipeline selection) —
    /// the O1 single-predicate rule ([`Self::path_has_sdf_forward`]'s own discipline). True
    /// exactly when the marcher runs AND the TAA resolve is armed this frame: on an SDF-carrying
    /// VB leg the marcher IS the composite and the SOLE gViewT producer (`viewt_from_vb_depth`
    /// covers only the marcher-less `VB x Mesh` config — the two arming predicates are disjoint
    /// by construction, mirroring the Deferred `viewt_from_depth` / `sdf_gbuffer_composite`
    /// split). With TAA off nothing reads `viewt`, so the no-`VIEWT` marcher variants keep the
    /// lane untouched (the 0%-gate).
    #[inline]
    pub(crate) fn path_sdf_forward_writes_viewt(&self) -> bool {
        self.path_has_sdf_forward() && self.taa.is_some()
    }

    /// Rung R9b — the SINGLE source of "this frame runs the VB geo/shade SPLIT pair"
    /// (`vb_geo` + `vb_shade_split` arm/disarm together), read at BOTH `declare_vb_graph` and
    /// `record_vb` (the O1 discipline). Boot-frozen: `mesh_geo_shade_split` is resolver-set
    /// exactly once (Decision 1) and only under VB with the mesh leg present.
    #[inline]
    pub(crate) fn path_vb_split(&self) -> bool {
        self.path_is_vb() && self.resolved_render_path.mesh_geo_shade_split
    }

    /// Rung R9b — the SINGLE source of "this frame's `lit` producer is the FUSED arm"
    /// (the classify chain + the `vb_resolve`/`vb_shade` selection; NOT `vb_raster`, which
    /// both arms consume and which stays gated on bare `mesh_leg`). The split DISPLACES the
    /// classification chain in v1 (docs/R9-VB-SPLIT-PLAN.md §0): `vb_use_classified` is
    /// consulted only inside this arm.
    #[inline]
    pub(crate) fn path_vb_fused(&self) -> bool {
        self.path_is_vb()
            && self.resolved_render_path.mesh_leg
            && !self.resolved_render_path.mesh_geo_shade_split
    }

    /// Rung R9b — the SINGLE source of "this frame runs the VB SSAO gather + à-trous chain".
    /// Anchored to the BOOT-frozen split flag (the resolver sets `mesh_geo_shade_split` only
    /// under VB), so the gather can only arm when its `thin_normal` producer (`vb_geo`) is
    /// boot-armed — correct even if the `RenderPathFrozenConsumers` clamp ever regressed.
    #[inline]
    pub(crate) fn path_vb_ssao(&self) -> bool {
        debug_assert!(
            !self.resolved_render_path.mesh_geo_shade_split || self.path_is_vb(),
            "invariant: mesh_geo_shade_split is resolver-set only under VisibilityBuffer"
        );
        self.resolved_render_path.mesh_geo_shade_split && self.ssao.is_some()
    }

    /// Rung R9c — the SINGLE source of "this frame runs the DDGI probe update + the split
    /// shade's probe sampling under VB" (read at BOTH `declare_vb_graph` and `record_vb`).
    /// Anchored to the boot-frozen split; the `ddgi_update` activation itself already carries
    /// the `sdf_leg` AND from `gpu_scene` (probes are SDF-marched), so this is reachable only
    /// on `VB × Both`.
    #[inline]
    pub(crate) fn path_vb_ddgi(&self) -> bool {
        self.path_is_vb()
            && self.resolved_render_path.mesh_geo_shade_split
            && self.ddgi_update.is_some()
    }

    /// Rung R9d — the SINGLE source of "this frame runs the VB hardware shadow chain" (TLAS
    /// pack/build + `shadow_vis` + à-trous + temporal), read at BOTH `declare_vb_graph` and
    /// `record_vb` (the O1 discipline). Requires the split to be armed (the hwrt vis gather's
    /// normal source IS `thin_normal`, the split's OWN thin-aux producer) AND the boot-armed
    /// shadow activation to exist — `self.shadow` is the SAME `Option<ShadowVisActivation>`
    /// field Deferred's own hwrt shadow chain shares (a single carrier for both paths).
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn path_vb_hwrt_shadow(&self) -> bool {
        self.path_vb_split() && self.shadow.is_some()
    }

    /// Whether the boot resolver armed an SDF soft-march shadow source
    /// (`boyko_render::ShadowSources::SDF_SOFT_MARCH`) for this scene.
    ///
    /// The FIRST reader of `ResolvedRenderPathGpu::shadow` outside the POD copy itself. The bits
    /// remain a RECORDED boot decision, not a dispatch input — nothing selects a pass from them,
    /// because the per-frame arming gates are strictly stronger than the boot bits (a
    /// config-enabled CSM with zero caster batches does not run a cascade pass). What this
    /// accessor exists for is the one place a boot bit is a NECESSARY condition of a shipped
    /// shader variant's reachability: see its use at the `vb_shade_split_*hwrt` selection site.
    ///
    /// `#[cfg(feature = "hwrt")]` because that site is the ONLY caller — the exclusion it guards
    /// is between an SDF-march source and the hwrt visibility term, which cannot exist in a
    /// build with no hwrt chain.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn shadow_has_sdf_soft_march(&self) -> bool {
        self.resolved_render_path.shadow & SHADOW_SOURCE_SDF_SOFT_MARCH != 0
    }

    /// Rung R9d — the SINGLE source of "this frame's `vb_geo` dispatch writes the `motion_vec`
    /// lane" decision, read at BOTH `declare_vb_graph` (the `vb_geo` access list's conditional
    /// `motion_vec` write) and `record_vb` (the `vb_geo_mv_pipeline` selection) — mirrors
    /// [`Self::sdf_mv_active`]'s deferred sibling (the W1 discipline). True iff the hwrt shadow
    /// chain runs this frame AND its temporal stage is armed.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub(crate) fn vb_geo_mv_active(&self) -> bool {
        self.path_vb_hwrt_shadow() && self.temporal_active()
    }

    /// Multi-paradigm render-path plan, rung R8 — the SINGLE source of "is this frame's
    /// declarator/recorder the `VisibilityBuffer` path" decision, so `declare_frame_graph`'s
    /// dispatch and `render_gbuffer_frame`'s record-site dispatch (`record_vb` vs
    /// `record_forward`/`record_gbuffer`) can never diverge (the SAME W1 lesson
    /// [`Self::path_is_forward`]'s doc explains). Compares against
    /// [`RENDER_PATH_VISIBILITY_BUFFER`] (code review P2-5: a named const instead of a bare `3`
    /// literal — `RenderPath::VisibilityBuffer`'s discriminant,
    /// `boyko_render::render_path_config::RenderPath`).
    #[inline]
    pub(crate) fn path_is_vb(&self) -> bool {
        self.resolved_render_path.path == RENDER_PATH_VISIBILITY_BUFFER
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
    /// instance SSBO layout. Bound by `record_csm_depth`.
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

