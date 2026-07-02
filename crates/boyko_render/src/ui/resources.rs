//! The owned UI render capability sub-owner (`UiRenderResources`) — GUI P5a
//! Rung 3 / Decision 7 + 8.
//!
//! This is the first-class, `Drop`-wired owner of every GPU resource the on-screen
//! UI pass needs (a NAMED owner on [`RhiContext`], NOT a side store — Principle 0):
//!
//! - the UI graphics pipeline (built once for a `color_format`, blend = premultiplied),
//! - the bind-group layout (one `StorageBuffer` @ set0/binding0, VERTEX|FRAGMENT),
//! - one persistent host-mapped grow-only STORAGE ring + one bind-group PER
//!   [`FRAMES_IN_FLIGHT`] slot, each created once and
//!   selected by `frame_index` (Decision 7).
//!
//! It is owned by [`RhiContext`] as a field, so its teardown rides
//! [`RhiContext::destroy_all`] and `RhiContext::Drop` — nothing leaks past the
//! device owner (the Decision-8 leak fix).
//!
//! All resources are created/destroyed through the real [`RhiDevice`] verbs on
//! `&VulkanContext` (reached via `RhiContext::split_mut().0`), so this module names
//! the device only by reference and never owns it (one device owner, one teardown
//! order).

use boyko_rhi::enums::{
    AddressMode, BarrierAccess, BarrierStage, BlendState, DescriptorKind, Filter, Format,
    ImageAspect, ImageLayout, ImageUsage, ShaderStage, TextureDimension,
};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, CullMode, GraphicsPipelineDesc, ImageBarrierDesc, ImageSubresourceRange,
    MemoryLocation, MipMode, PrimitiveTopology, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc,
    TextureDesc,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline, VulkanSampler,
    VulkanShaderModule,
};
use boyko_rhi_vulkan::texture::VulkanTexture;

use boyko_fontbake::atlas::BakedFont;

use crate::error::GpuColumnError;
use crate::ui::instance::{UiOrtho, UI_INSTANCE_SIZE};
use crate::ui::FRAMES_IN_FLIGHT;
// `RhiContext` is referenced only in doc links (this sub-owner is a field on it);
// importing it lets the bare `[`RhiContext`]` intra-doc links resolve.
#[allow(unused_imports)]
use crate::gpu_column::RhiContext;

/// The VERTEX-stage push-constant range the UI pipeline declares (one [`UiOrtho`],
/// 16 B). The fragment shader reads only the SSBO + the per-atlas UBO, so the ortho
/// is pushed VERTEX-only (matching the backend's VERTEX-only graphics push range —
/// `rhi_impl.rs`). GUI P5b adds NO push bytes: the per-atlas pxRange/atlasSize ride
/// the binding-2 UBO (Decision T4-A), so this stays VERTEX-only and unchanged.
const UI_PUSH_CONSTANT_BYTES: u32 = size_of::<UiOrtho>() as u32;

/// The per-atlas FRAGMENT uniform (GUI P5b Decision T4-A): the baked distance range
/// in TEXELS + the atlas size in texels. 16 B, std140/std430-compatible
/// (`scalar, pad, vec2`), written ONCE at atlas upload and immutable thereafter (one
/// atlas → one constant pxRange/size). The fragment shader reads it at set0/binding2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiAtlasUniform {
    /// `= AtlasMeta.distance_range_texels` — the `screenPxRange` divisor.
    pub px_range: f32,
    /// std140 pad so `atlas_size` lands on an 8 B boundary (16 B total).
    pub _pad0: f32,
    /// `(atlas_w, atlas_h)` in texels.
    pub atlas_size: [f32; 2],
}

const _: () = assert!(size_of::<UiAtlasUniform>() == 16);

impl UiAtlasUniform {
    /// Re-views this 16-byte POD as the `&[u8]` the setup path memcpys into the
    /// host-visible binding-2 UBO once at atlas upload.
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `UiAtlasUniform` is `#[repr(C)]` POD (f32 only, 16 B, no padding
        // beyond the explicit `_pad0` — const-asserted size), so its byte image is a
        // valid `[u8; 16]`; the `&self` borrow keeps it alive for the slice; the slice
        // is read-only.
        unsafe { core::slice::from_raw_parts((self as *const Self).cast::<u8>(), size_of::<Self>()) }
    }
}

/// One loaded MSDF atlas on the GPU (GUI P5b Decision T4-C): an upload-once SAMPLED
/// texture + its no-mip bilinear sampler + the per-atlas binding-2 UBO. Owned as a
/// field on [`UiRenderResources`] so `create_slot`/`grow_slot` can re-bind bindings 1
/// and 2 when a ring grow rebuilds the bind-group (the grow-hole fix). Upload-once,
/// all-FIF sample concurrently, NO per-frame barrier (the SampledComposite pattern).
struct UiAtlas {
    /// The MTSDF atlas image (`SAMPLED | TRANSFER_DST`, `ShaderReadOnlyOptimal` after
    /// upload). Outlives every submission that binds it (the `BindGroupEntry` contract).
    texture: VulkanTexture,
    /// The bilinear, clamp-to-edge, NO-MIP sampler (Decision T4-D).
    sampler: VulkanSampler,
    /// The host-visible UBO holding the immutable [`UiAtlasUniform`], written once.
    uniform: BoundBuffer,
}

/// One persistent-mapped, grow-only STORAGE ring slot + its bind-group (Decision 7).
///
/// Created once per frame-in-flight at [`UiRenderResources`] setup; the buffer is
/// host-visible + host-coherent and mapped once (never unmapped). On overflow the
/// whole slot (buffer + bind-group) is recreated at the pow2-rounded capacity (the
/// affected slot only) — `create_bind_group` writes the descriptor set ONCE at
/// create, so the grow MUST rebuild the bind-group (there is no update verb).
struct UiRingSlot {
    /// The host-mapped STORAGE buffer this slot's instances are memcpy'd into.
    buffer: BoundBuffer,
    /// The bind-group binding `buffer` at set0/binding0 (created once; rebuilt on grow).
    bind_group: VulkanBindGroup,
    /// The slot's current byte capacity (grows pow2 only on overflow).
    cap_bytes: u64,
}

/// The owned, `Drop`-wired UI render capability (Decision 8): the pipeline +
/// bind-group layout + the per-FIF host-mapped rings + bind-groups.
///
/// Owned as a field on [`RhiContext`] so its teardown is wired into
/// `RhiContext::destroy_all` + `RhiContext::Drop` — a first-class kernel capability
/// with explicit teardown (NOT a side store). `!Send + !Sync` by its owning
/// `RhiContext` (touched only on the dispatcher thread).
pub(crate) struct UiRenderResources {
    /// The UI graphics pipeline (built once for the setup `color_format`; blend =
    /// premultiplied). Re-resolved each frame by `frame_index` indirection through
    /// the owning [`RhiContext`] (MF-7) — the on-screen recorder never caches it.
    pipeline: VulkanGraphicsPipeline,
    /// The shared bind-group layout. Three bindings at set 0: the `StorageBuffer` ring
    /// at binding 0 (VERTEX and FRAGMENT), the `CombinedImageSampler` MSDF atlas at
    /// binding 1 (FRAGMENT), and the `UniformBuffer` per-atlas pxRange/size at binding 2
    /// (FRAGMENT) — GUI P5b. Every per-FIF bind-group is allocated against it.
    layout: VulkanBindGroupLayout,
    /// The two committed shader modules (vertex + fragment), retained for teardown
    /// ordering (the pipeline owns the compiled stages; the modules are destroyed
    /// after the pipeline at teardown).
    vertex_module: VulkanShaderModule,
    fragment_module: VulkanShaderModule,
    /// The loaded MSDF glyph atlas (GUI P5b Decision T4-C): texture + sampler + the
    /// binding-2 UBO. Owned HERE (not on `RhiContext`) so `create_slot`/`grow_slot`
    /// write all three bind-group entries — a grown slot is complete.
    atlas: UiAtlas,
    /// One persistent-mapped grow-only ring + bind-group per frame-in-flight.
    slots: [UiRingSlot; FRAMES_IN_FLIGHT],
}

impl UiRenderResources {
    /// Builds the UI pipeline + bind-group layout + per-FIF host-mapped rings +
    /// bind-groups, once (Rung 3 step 9). Every resource is created through the real
    /// [`RhiDevice`] verbs on `device`.
    ///
    /// `color_format` is the format of the image the UI pass renders into (the
    /// swapchain surface format for the on-screen path, `R8G8B8A8Unorm` for the
    /// offscreen golden — Decision 9's two-pipeline-from-one-shader contract).
    /// `initial_rows` is each ring's starting capacity in `UiInstance` records (the
    /// rings grow pow2 on overflow).
    ///
    /// On any partial failure every resource already created here is torn down
    /// before the error returns (no leak), since none is owned by the manager.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] on any shader-module / pipeline / layout / buffer /
    /// bind-group create failure.
    pub(crate) fn create(
        device: &VulkanContext,
        color_format: Format,
        spirv_vs: &[u32],
        spirv_fs: &[u32],
        initial_rows: u32,
        font: &BakedFont,
    ) -> Result<Self, GpuColumnError> {
        debug_assert!(initial_rows > 0, "invariant: UI ring initial_rows is non-zero");

        let vertex_module = device.create_shader_module(spirv_vs)?;
        let fragment_module = match device.create_shader_module(spirv_fs) {
            Ok(m) => m,
            Err(e) => {
                // SAFETY: `vertex_module` was just created on `device`, is owned
                // exclusively here (never registered), and no pipeline references it
                // (the fragment module failed first), so destroying it once is sound.
                unsafe { device.destroy_shader_module(vertex_module) };
                return Err(GpuColumnError::Rhi(e));
            }
        };

        let layout = match device.create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    count: 1,
                    kind: DescriptorKind::StorageBuffer,
                    // The Rung-0.5-proven combination: a STORAGE buffer visible in BOTH
                    // the vertex (transform) and fragment (SDF/clip) stages.
                    stage: ShaderStage::VERTEX | ShaderStage::FRAGMENT,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    count: 1,
                    // GUI P5b: the MSDF atlas (combined image+sampler), FRAGMENT-only.
                    kind: DescriptorKind::CombinedImageSampler,
                    stage: ShaderStage::FRAGMENT,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    count: 1,
                    // GUI P5b: the per-atlas pxRange/atlasSize uniform, FRAGMENT-only.
                    kind: DescriptorKind::UniformBuffer,
                    stage: ShaderStage::FRAGMENT,
                },
            ],
        }) {
            Ok(l) => l,
            Err(e) => {
                // SAFETY: both modules were just created on `device`, owned
                // exclusively here, referenced by no live pipeline; destroy each once.
                unsafe {
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        let pipeline = match device.create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vertex_module,
            vertex_entry: c"main",
            fragment_module: &fragment_module,
            fragment_entry: c"main",
            color_formats: &[color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            // Vertexless quad (`SV_VertexID`), the Rung-0.5 shape.
            vertex_layout: None,
            push_constant_bytes: UI_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&layout),
            blend: Some(BlendState::PREMULTIPLIED_ALPHA),
            cull_mode: CullMode::None,
            depth_bias: None,
        }) {
            Ok(p) => p,
            Err(e) => {
                // SAFETY: the layout + both modules were just created on `device`,
                // owned exclusively here, referenced by no live pipeline (the create
                // failed); destroy each once in reverse creation order.
                unsafe {
                    device.destroy_bind_group_layout(layout);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // Build + upload the MSDF atlas (GUI P5b): the SAMPLED texture (staged copy +
        // barrier to ShaderReadOnlyOptimal), the no-mip bilinear sampler, and the
        // binding-2 UBO (written once). On failure tear down pipeline/layout/modules.
        let atlas = match Self::create_atlas(device, font) {
            Ok(a) => a,
            Err(e) => {
                // SAFETY: pipeline/layout/modules above were created on `device`, owned
                // exclusively here, never submitted; destroy each once in reverse order.
                unsafe {
                    device.destroy_graphics_pipeline(pipeline);
                    device.destroy_bind_group_layout(layout);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(e);
            }
        };

        // Build the per-FIF rings. On a mid-array failure, every slot built so far
        // (plus the atlas + pipeline/layout/modules) is torn down before returning.
        let mut built: Vec<UiRingSlot> = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let init_bytes = initial_rows as u64 * UI_INSTANCE_SIZE as u64;
        for _ in 0..FRAMES_IN_FLIGHT {
            match Self::create_slot(device, &layout, &atlas, init_bytes) {
                Ok(slot) => built.push(slot),
                Err(e) => {
                    for slot in built {
                        // SAFETY: each `slot`'s buffer + bind-group were created on
                        // `device`, are owned exclusively here, and were never
                        // submitted to (setup-time), so destroying each once is sound.
                        unsafe {
                            device.destroy_bind_group(slot.bind_group);
                            device.destroy_buffer(slot.buffer);
                        }
                    }
                    Self::destroy_atlas(device, atlas);
                    // SAFETY: the pipeline/layout/modules above were created on
                    // `device`, owned exclusively here, never submitted; destroy each
                    // once in reverse creation order.
                    unsafe {
                        device.destroy_graphics_pipeline(pipeline);
                        device.destroy_bind_group_layout(layout);
                        device.destroy_shader_module(fragment_module);
                        device.destroy_shader_module(vertex_module);
                    }
                    return Err(e);
                }
            }
        }

        let slots: [UiRingSlot; FRAMES_IN_FLIGHT] = built
            .try_into()
            .unwrap_or_else(|_| unreachable!("invariant: built exactly FRAMES_IN_FLIGHT slots"));

        Ok(Self {
            pipeline,
            layout,
            vertex_module,
            fragment_module,
            atlas,
            slots,
        })
    }

    /// Ensures slot `frame_index` can hold `instance_count` records, growing pow2 on
    /// overflow (fence-wait the device, recreate the buffer + rebuild this slot's
    /// bind-group), then memcpys `packed` into the mapped slot (Rung 3 step 10 / A1
    /// steps 4-5).
    ///
    /// `packed` is the contiguous byte image of the `instance_count` records (the
    /// no-bytemuck POD view from [`UiInstance::slice_as_bytes`](crate::ui::UiInstance::slice_as_bytes));
    /// `packed.len()` MUST equal `instance_count * UI_INSTANCE_SIZE`.
    ///
    /// # Caller contract (the write-after-read fence — GUI P5a)
    ///
    /// The caller MUST have waited slot `frame_index`'s present in-flight fence
    /// BEFORE this call — enforced one level up: `RhiContext::ui_upload` derives
    /// `frame_index` from the `FrameWriteToken` minted by
    /// `Renderer::wait_frame_in_flight` (or forged unsafely at setup time) —
    /// so the GPU's last read of this persistently-mapped, host-coherent ring slot (the
    /// submit two presents back, with `FRAMES_IN_FLIGHT == 2`) is complete. Without
    /// that wait the memcpy below is a write-after-read race on a buffer the GPU may
    /// still be reading. The grow path (`grow_slot`) `wait_idle`s the whole device, so
    /// a grow frame is covered regardless; the steady-state (no-grow) memcpy relies on
    /// the caller's per-slot fence wait.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a grow buffer-create / bind-group-create failure or a
    /// missing ring mapping.
    ///
    /// [`UiInstance::slice_as_bytes`]: crate::ui::UiInstance::slice_as_bytes
    pub(crate) fn upload(
        &mut self,
        device: &VulkanContext,
        packed: &[u8],
        instance_count: u32,
        frame_index: usize,
    ) -> Result<(), GpuColumnError> {
        debug_assert!(frame_index < FRAMES_IN_FLIGHT, "invariant: frame_index in range");
        debug_assert_eq!(
            packed.len(),
            instance_count as usize * UI_INSTANCE_SIZE,
            "invariant: packed byte length matches instance_count * UI_INSTANCE_SIZE"
        );

        let need = packed.len() as u64;
        // Clamp the slot index defensively (a release-time out-of-range would index
        // out of bounds; the debug_assert above traps it in debug).
        let frame_index = frame_index.min(FRAMES_IN_FLIGHT - 1);

        if need > self.slots[frame_index].cap_bytes {
            self.grow_slot(device, frame_index, need)?;
        }

        // memcpy into the mapped slot. A zero-instance frame still resolves the
        // mapping (the ring stays valid) but copies nothing.
        if need > 0 {
            let slot = &self.slots[frame_index];
            let dst = device
                .buffer_mapped_ptr(&slot.buffer)
                .ok_or(GpuColumnError::StagingNotMapped)?;
            debug_assert!(
                need <= slot.cap_bytes,
                "invariant: instance bytes fit the (grown) ring capacity"
            );
            // SAFETY: `dst` is the persistently-mapped first byte of slot
            // `frame_index`'s host-visible + host-coherent ring, whose `cap_bytes`
            // is `>= need` (grown just above on overflow). `packed` is a distinct
            // allocation (the pack scratch), so the regions never overlap; `&mut
            // self` makes this the unique writer. The GPU's last read of this SAME
            // ring slot (the submit two presents back) is complete: the caller waited
            // slot `frame_index`'s present in-flight fence before this call (the
            // documented caller contract above) — and a grow frame additionally
            // `wait_idle`s the whole device in `grow_slot`. Host-coherent ⇒ no flush.
            unsafe {
                core::ptr::copy_nonoverlapping(packed.as_ptr(), dst.as_ptr(), packed.len());
            }
        }
        Ok(())
    }

    /// Re-resolves the current-frame UI pipeline + bind-group by `frame_index`
    /// (MF-7) — the on-screen recorder reads the handles indirectly each frame, never
    /// a cached raw handle, so a grow that rebuilt slot `frame_index`'s bind-group
    /// between upload and draw is transparent.
    #[inline]
    pub(crate) fn handles(
        &self,
        frame_index: usize,
    ) -> (&VulkanGraphicsPipeline, &VulkanBindGroup) {
        debug_assert!(frame_index < FRAMES_IN_FLIGHT, "invariant: frame_index in range");
        let frame_index = frame_index.min(FRAMES_IN_FLIGHT - 1);
        (&self.pipeline, &self.slots[frame_index].bind_group)
    }

    /// Tears down every owned resource (Rung 3 — wired into `RhiContext::destroy_all`
    /// and `RhiContext::Drop`). Consumes `self`, so it runs exactly once; the caller
    /// `take()`s the `Option<UiRenderResources>` so a second `destroy_all`/`Drop`
    /// finds `None` and is a no-op (idempotent like the manager).
    ///
    /// `device.wait_idle()` is called first so no in-flight present submission is
    /// still reading any ring; then each resource is destroyed in reverse creation
    /// order (slots → pipeline → layout → modules).
    pub(crate) fn destroy(self, device: &VulkanContext) {
        // Belt-and-braces: the caller's teardown contract already drains the device,
        // but a `wait_idle` here makes the destroy sound regardless of caller order.
        let _ = device.wait_idle();
        for slot in self.slots {
            // SAFETY: each slot's bind-group + buffer were created on `device`, owned
            // exclusively here, and the device is idle (waited above), so no GPU work
            // references them; each is moved by value ⇒ destroyed exactly once.
            unsafe {
                device.destroy_bind_group(slot.bind_group);
                device.destroy_buffer(slot.buffer);
            }
        }
        // GUI P5b: tear down the atlas (texture + sampler + UBO) AFTER the slots that
        // bound it (the bind-groups are already gone) and BEFORE the pipeline/layout.
        Self::destroy_atlas(device, self.atlas);
        // SAFETY: the pipeline/layout/modules were created on `device`, owned
        // exclusively here, and the device is idle; each is moved by value ⇒
        // destroyed exactly once, in reverse creation order (the pipeline before its
        // layout + modules).
        unsafe {
            device.destroy_graphics_pipeline(self.pipeline);
            device.destroy_bind_group_layout(self.layout);
            device.destroy_shader_module(self.fragment_module);
            device.destroy_shader_module(self.vertex_module);
        }
    }

    // ===== internals =====

    /// Builds + uploads the MSDF atlas (GUI P5b): a `SAMPLED | TRANSFER_DST` texture
    /// staged-copy from `font.atlas.pixels`, barriered UNDEFINED → TRANSFER_DST →
    /// SHADER_READ_ONLY_OPTIMAL (so every FIF samples it concurrently with no per-frame
    /// barrier — the SampledComposite pattern), the no-mip bilinear sampler (Decision
    /// T4-D), and the binding-2 UBO written once with `(pxRange, atlasSize)`. The whole
    /// upload is a single fenced submit at setup; on any partial failure every object
    /// created so far is torn down before the error returns (no leak).
    fn create_atlas(device: &VulkanContext, font: &BakedFont) -> Result<UiAtlas, GpuColumnError> {
        let w = font.atlas.width;
        let h = font.atlas.height;
        let pixels = &font.atlas.pixels;
        debug_assert!(w > 0 && h > 0, "invariant: atlas extent is non-zero");
        debug_assert_eq!(
            pixels.len(),
            (w as usize) * (h as usize) * 4,
            "invariant: MTSDF atlas is tightly-packed RGBA8 (w*h*4 bytes)"
        );

        // The SAMPLED atlas image.
        let texture = device.create_texture(&TextureDesc {
            width: w,
            height: h,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
        })?;

        // The bilinear, clamp-to-edge, NO-MIP sampler (Decision T4-D).
        let sampler = match device.create_sampler(&SamplerDesc {
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
            compare: None,
        }) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: `texture` was just created on `device`, owned exclusively
                // here, never submitted; destroy it once on this edge.
                unsafe { device.destroy_texture(texture) };
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // The per-atlas UBO (host-visible, written once); 16 B (UiAtlasUniform).
        let uniform = match device.create_buffer(&BufferDesc {
            size: size_of::<UiAtlasUniform>() as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        }) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: texture + sampler were just created on `device`, owned
                // exclusively here, never submitted; destroy each once on this edge.
                unsafe {
                    device.destroy_sampler(sampler);
                    device.destroy_texture(texture);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };
        let ubo = UiAtlasUniform {
            px_range: font.meta.distance_range_texels,
            _pad0: 0.0,
            atlas_size: [w as f32, h as f32],
        };
        if let Some(dst) = device.buffer_mapped_ptr(&uniform) {
            // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent
            // UBO (≥ 16 B, just created); `ubo.as_bytes()` is exactly 16 distinct,
            // non-overlapping bytes; this is the unique writer (the UBO is immutable
            // after this), and no GPU submission has bound it yet. Host-coherent ⇒ no
            // flush. The write happens before the atlas is first sampled.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    ubo.as_bytes().as_ptr(),
                    dst.as_ptr(),
                    size_of::<UiAtlasUniform>(),
                );
            }
        }

        // Stage the pixels through a host-visible TRANSFER_SRC buffer, then a single
        // fenced submit: copy_buffer_to_image + the two layout barriers.
        if let Err(e) = Self::upload_atlas_pixels(device, &texture, w, h, pixels) {
            // SAFETY: uniform + sampler + texture were just created on `device`, owned
            // exclusively here, the upload submit (if any) is fence-waited or never
            // happened; destroy each once on this edge.
            unsafe {
                device.destroy_buffer(uniform);
                device.destroy_sampler(sampler);
                device.destroy_texture(texture);
            }
            return Err(e);
        }

        Ok(UiAtlas {
            texture,
            sampler,
            uniform,
        })
    }

    /// Records + submits the one-time staged copy of `pixels` into `texture`, fence-
    /// waited, transitioning UNDEFINED → TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
    /// so the atlas is sample-ready for every frame thereafter (no per-frame barrier).
    /// The staging buffer + encoder + fence are setup-class transients, torn down here.
    fn upload_atlas_pixels(
        device: &VulkanContext,
        texture: &VulkanTexture,
        w: u32,
        h: u32,
        pixels: &[u8],
    ) -> Result<(), GpuColumnError> {
        let size = pixels.len() as u64;
        let staging = device.create_buffer(&BufferDesc {
            size,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })?;
        if let Some(dst) = device.buffer_mapped_ptr(&staging) {
            // SAFETY: `dst` is the persistently-mapped first byte of the host-coherent
            // staging buffer (exactly `size` bytes, just created); `pixels` is a
            // distinct, non-overlapping allocation of `size` bytes; this is the unique
            // writer before any submission binds the buffer. Host-coherent ⇒ no flush.
            unsafe {
                core::ptr::copy_nonoverlapping(pixels.as_ptr(), dst.as_ptr(), pixels.len());
            }
        } else {
            // SAFETY: `staging` was just created on `device`, owned exclusively here,
            // never submitted; destroy it once on this edge.
            unsafe { device.destroy_buffer(staging) };
            return Err(GpuColumnError::StagingNotMapped);
        }

        let mut encoder = match device.create_command_encoder() {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: `staging` was just created, never submitted; destroy once.
                unsafe { device.destroy_buffer(staging) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        let fence = match device.create_fence(false) {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: encoder + staging just created, never submitted; destroy each.
                unsafe {
                    device.destroy_command_encoder(encoder);
                    device.destroy_buffer(staging);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        let region = [BufferImageCopy {
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
            image_extent_w: w,
            image_extent_h: h,
            image_extent_d: 1,
        }];

        let record = (|| -> Result<(), GpuColumnError> {
            encoder.begin().map_err(GpuColumnError::Rhi)?;
            // UNDEFINED → TRANSFER_DST_OPTIMAL (the copy destination).
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::TRANSFER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::TRANSFER_WRITE,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::TransferDstOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.copy_buffer_to_image(
                &staging,
                texture,
                ImageLayout::TransferDstOptimal,
                &region,
            );
            // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (sample-ready, all FIF).
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TRANSFER,
                dst_stage: BarrierStage::FRAGMENT_SHADER,
                src_access: BarrierAccess::TRANSFER_WRITE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::TransferDstOptimal,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range: ImageSubresourceRange::COLOR,
            });
            encoder.end().map_err(GpuColumnError::Rhi)?;
            let queue = device.rhi_queue();
            queue.submit(&encoder, &fence).map_err(GpuColumnError::Rhi)?;
            device.wait_fence(&fence, u64::MAX).map_err(GpuColumnError::Rhi)?;
            Ok(())
        })();

        // Tear down the setup-class transients. The submit (if it ran) is fence-waited.
        // SAFETY: encoder/fence/staging were created on `device`; the encoder's only
        // submission (if any) completed (fence-waited above on the Ok path, or never
        // submitted on an error path), and each is moved by value ⇒ destroyed once.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
            device.destroy_buffer(staging);
        }
        record
    }

    /// Tears down a [`UiAtlas`] (texture + sampler + UBO). The caller has drained the
    /// device (`wait_idle`) so no submission still samples it.
    fn destroy_atlas(device: &VulkanContext, atlas: UiAtlas) {
        // SAFETY: each of `atlas`'s objects was created on `device` by `create_atlas`,
        // the device is idle / drained by the caller (no GPU work references them), and
        // each is moved by value ⇒ destroyed exactly once, view→image→memory then
        // sampler then UBO.
        unsafe {
            device.destroy_texture(atlas.texture);
            device.destroy_sampler(atlas.sampler);
            device.destroy_buffer(atlas.uniform);
        }
    }

    /// Creates one host-mapped STORAGE ring of `cap_bytes` + its bind-group against
    /// `layout`, writing ALL THREE entries (GUI P5b Decision T4-C): the ring at
    /// binding 0, the `atlas` texture+sampler at binding 1, and the per-atlas UBO at
    /// binding 2 — so a grown slot is complete for the three-binding layout. The buffer
    /// is host-visible + host-coherent (mapped once at create).
    fn create_slot(
        device: &VulkanContext,
        layout: &VulkanBindGroupLayout,
        atlas: &UiAtlas,
        cap_bytes: u64,
    ) -> Result<UiRingSlot, GpuColumnError> {
        let buffer = device.create_buffer(&BufferDesc {
            size: cap_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })?;
        let bind_group = match device.create_bind_group(&BindGroupDesc {
            layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::CombinedImage {
                    texture: &atlas.texture,
                    sampler: &atlas.sampler,
                },
                BindGroupEntry::UniformBuffer {
                    buffer: &atlas.uniform,
                },
            ],
        }) {
            Ok(bg) => bg,
            Err(e) => {
                // SAFETY: `buffer` was just created on `device`, owned exclusively
                // here, never submitted; destroy it once on this error edge.
                unsafe { device.destroy_buffer(buffer) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        Ok(UiRingSlot {
            buffer,
            bind_group,
            cap_bytes,
        })
    }

    /// Grows slot `frame_index` to `>= need` bytes (pow2-rounded): fence-wait the
    /// device, create the new buffer + bind-group, then destroy the old ones
    /// (Decision 7 grow path). `#[cold]` — a setup-class cost, off the steady-state
    /// I-cache.
    #[cold]
    fn grow_slot(
        &mut self,
        device: &VulkanContext,
        frame_index: usize,
        need: u64,
    ) -> Result<(), GpuColumnError> {
        // Drain the device so the slot being recreated is not read by any in-flight
        // present submission (the `RhiContext` does not own the per-FIF present fence;
        // `wait_idle` is the available device-level drain, and a grow is setup-class).
        let _ = device.wait_idle();

        let new_cap = need.next_power_of_two().max(self.slots[frame_index].cap_bytes);
        // GUI P5b: the grown slot's bind-group re-binds all three entries (the atlas +
        // UBO via `self.atlas`), so it stays complete for the three-binding layout.
        let new_slot = Self::create_slot(device, &self.layout, &self.atlas, new_cap)?;

        // Swap in the new slot, then destroy the old buffer + bind-group.
        let old = core::mem::replace(&mut self.slots[frame_index], new_slot);
        // SAFETY: `old`'s bind-group + buffer were created on `device`, owned
        // exclusively here, and the device was drained (`wait_idle` above), so no GPU
        // work references them; each is moved by value ⇒ destroyed exactly once.
        unsafe {
            device.destroy_bind_group(old.bind_group);
            device.destroy_buffer(old.buffer);
        }
        Ok(())
    }
}

// NOTE (no `unsafe`): `UiRenderResources` holds backend handles + a `BoundBuffer`
// whose `mapped` is a raw `NonNull<u8>` (so it is `!Send + !Sync` automatically),
// exactly what its owning `!Send + !Sync` `RhiContext` requires — the UI
// rings/pipeline are touched only on the dispatcher thread. No `unsafe impl`.
