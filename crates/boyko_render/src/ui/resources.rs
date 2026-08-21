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
use boyko_rhi_vulkan::bindless::VulkanBindlessSet;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::VkDescriptorSetLayout;
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
/// `rhi_impl/device.rs`). GUI P5b adds NO push bytes: the per-atlas pxRange/atlasSize ride
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

/// The filter mode the UI's OWN sprite sampler is built with (`docs/UI-PLAN-SPRITES.md`
/// S-D4) — chosen ONCE at [`RhiContext::ui_setup`], costing ZERO per-instance bytes and
/// leaving the world-shared bindless set untouched (which D3 refuses to let a UI concern
/// mutate).
///
/// It exists because the bindless set's own sampler is IMMUTABLE, trilinear, 16×
/// anisotropic and `REPEAT` — chosen for tiled world material textures. Sampling UI sprites
/// through it would make [`Pixel`](Self::Pixel) permanently unreachable, and the ceiling
/// could not later be lifted without editing a descriptor set shared with every world
/// material. A per-SPRITE mode (rather than per-UI) is S7's deferred extension, with
/// `UiInstance.flags` bit 4 already reserved for its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiSamplerMode {
    /// `LINEAR` mag/min + `ClampToEdge`, no mips — photographic icons and scaled art.
    #[default]
    Smooth,
    /// `NEAREST` mag/min + `ClampToEdge`, no mips — pixel art, whose whole point is that a
    /// texel is a square.
    Pixel,
}

impl UiSamplerMode {
    /// The [`Filter`] this mode builds its sampler with (both mag and min).
    #[inline]
    fn filter(self) -> Filter {
        match self {
            UiSamplerMode::Smooth => Filter::Linear,
            UiSamplerMode::Pixel => Filter::Nearest,
        }
    }
}

/// Where set 1 — the sprite lane's `Texture2D g_sprites[]` — comes from, and who owns it
/// (`docs/UI-PLAN-SPRITES.md` S3).
///
/// # Why a bindless-LESS host still gets a set 1 (the S3 amendment, measured)
///
/// S3 as written said `ui_setup(bindless: None)` builds "the existing one-set path". It
/// cannot: `ui_rect.fs` STATICALLY uses set 1 (the sprite branch is reachable code), so a
/// one-set pipeline layout is `VUID-VkGraphicsPipelineCreateInfo-layout-07988` at create and
/// `VUID-vkCmdDraw-None-08600` at every draw — including a plain rect draw. Measured on this
/// box with the layer on, 2026-08-21: three validation messages, quoted in the commit. The
/// property G3-4 is actually about — *a host with no bindless table still boots and still
/// draws rects and text* — is preserved instead by giving that host a UI-OWNED fallback set 1
/// holding a single 1×1 TRANSPARENT texture, so the pipeline stays 2-set, set 1 is always
/// bound, and there is still exactly ONE `.spv` with no `-D` axis (S3 item 6).
struct UiSpriteSet {
    /// The descriptor set bound at set 1 by BOTH recorders (the generic offscreen
    /// `record_ui_rects` through `bind_descriptor_set_at(1, …)`, the concrete on-screen
    /// `present_blit` through its own `cmd_bind_descriptor_sets(first_set = 1)`).
    ///
    /// When [`owned`](Self::owned) is `None` these are a NON-OWNING copy of the HOST's
    /// `BindlessTextureTable` handles: this capability destroys nothing, and the host MUST
    /// keep its table alive for as long as the UI capability lives (the same un-tied
    /// raw-handle contract every other RHI resource in this engine carries —
    /// `boyko_rhi_vulkan` plan F1). A host that tears its table down must re-run
    /// `ui_setup`.
    group: VulkanBindGroup,
    /// `Some` iff this capability OWNS the table behind `group` — the `bindless: None`
    /// fallback. The `Option` IS the ownership statement: `destroy_sprites` frees
    /// `group` exactly when this is `Some`, so the shared arm cannot be freed by accident.
    owned: Option<UiOwnedSpriteTable>,
}

/// The UI-OWNED fallback sprite table's resources (`ui_setup(bindless: None)`): a
/// one-descriptor `SAMPLED_IMAGE` set holding a 1×1 transparent texture.
///
/// A stray `FLAG_TEXTURED` instance on such a host therefore samples something VALID and
/// invisible rather than an undefined descriptor — the same structural guard
/// `BindlessTextureTable` gets from writing its magenta error texture into every slot
/// before any registration.
struct UiOwnedSpriteTable {
    /// The one-binding set layout (`SAMPLED_IMAGE`, count 1, FRAGMENT).
    layout: VulkanBindGroupLayout,
    /// The 1×1 transparent `ShaderReadOnlyOptimal` texture every index resolves to.
    texture: VulkanTexture,
}

impl UiSpriteSet {
    /// The descriptor set bound at set 1 by both recorders.
    #[inline]
    fn group(&self) -> &VulkanBindGroup {
        &self.group
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
    /// The shared bind-group layout. FOUR bindings at set 0: the `StorageBuffer` ring
    /// at binding 0 (VERTEX and FRAGMENT), the `CombinedImageSampler` MSDF atlas at
    /// binding 1 (FRAGMENT), the `UniformBuffer` per-atlas pxRange/size at binding 2
    /// (FRAGMENT) — GUI P5b — and, since UI-ADVANCED S3, the UI's OWN sprite sampler at
    /// binding 3 (FRAGMENT, S-D4). Every per-FIF bind-group is allocated against it.
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
    /// The UI's OWN sprite sampler (S-D4), bound at set 0 / binding 3 as the SAMPLER half
    /// of a `CombinedImageSampler` whose image half is the atlas above and is never read
    /// there. Built once at setup from the caller's [`UiSamplerMode`]; every sprite in the
    /// pass samples the bindless table through it, so a pixel-art UI is expressible without
    /// touching the world-shared bindless set.
    sprite_sampler: VulkanSampler,
    /// Set 1 — the sprite lane's `Texture2D g_sprites[]` — and who owns it (see
    /// [`UiSpriteSet`]).
    sprites: UiSpriteSet,
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
    /// `font` is OPTIONAL since UI-ADVANCED S3 (architecture D8e): `None` default-fills the
    /// binding-1 atlas with a 1×1 TRANSPARENT texture, so a SPRITE-ONLY UI boots and draws
    /// (gate G3-3) — the same trick the bindless table already uses with its magenta error
    /// slot. `bindless` is likewise optional: `Some` binds the host's shared table at set 1;
    /// `None` builds the UI-owned fallback set 1 (see [`UiSpriteSet`], gate G3-4).
    /// `sampler_mode` picks the UI's OWN sprite sampler's filter (S-D4).
    ///
    /// On any partial failure every resource already created here is torn down
    /// before the error returns (no leak), since none is owned by the manager.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] on any shader-module / pipeline / layout / buffer /
    /// bind-group / sampler create failure.
    // `clippy::too_many_arguments` (8 with `device`): every parameter here is a distinct
    // SETUP-time decision with no natural grouping — the target format, the two committed
    // SPIR-V streams, the ring capacity, the optional font, the sampler mode, the optional
    // shared table. Bundling them into a `UiSetupDesc` is the right move the moment S4/S5
    // add another (they will), but doing it now would churn six call sites for zero
    // behaviour and is not what this rung is for. This is a once-per-process call, so the
    // lint's actual concern (a hot-path argument shuffle) does not apply.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        device: &VulkanContext,
        color_format: Format,
        spirv_vs: &[u32],
        spirv_fs: &[u32],
        initial_rows: u32,
        font: Option<&BakedFont>,
        sampler_mode: UiSamplerMode,
        bindless: Option<&VulkanBindlessSet>,
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

        // The UI's OWN sprite sampler (S-D4). Built BEFORE the atlas and the rings because
        // both the binding-3 write and the fallback set-1 write name it.
        let sprite_sampler = match device.create_sampler(&SamplerDesc {
            mag_filter: sampler_mode.filter(),
            min_filter: sampler_mode.filter(),
            // ClampToEdge in BOTH modes: `REPEAT` is what would make a sheet frame's UV
            // wrap into its neighbours (S-D7), and S4's tiled nine-slice is the one caller
            // that wants wrapping — it is not built yet and does not get to set the default.
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
            compare: None,
        }) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: both modules were just created on `device`, owned exclusively
                // here, referenced by no live pipeline; destroy each once on this edge.
                unsafe {
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // Build + upload the MSDF atlas (GUI P5b): the SAMPLED texture (staged copy +
        // barrier to ShaderReadOnlyOptimal), the no-mip bilinear sampler, and the
        // binding-2 UBO (written once). `font: None` default-fills it (D8e).
        let atlas = match Self::create_atlas(device, font) {
            Ok(a) => a,
            Err(e) => {
                // SAFETY: sampler + both modules were just created on `device`, owned
                // exclusively here, never submitted; destroy each once in reverse order.
                unsafe {
                    device.destroy_sampler(sprite_sampler);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(e);
            }
        };

        // Set 1 — the host's shared bindless table, or the UI-owned fallback (S3 amendment).
        let sprites = match Self::create_sprite_set(device, bindless, &sprite_sampler) {
            Ok(s) => s,
            Err(e) => {
                Self::destroy_atlas(device, atlas);
                // SAFETY: sampler + modules created on `device`, owned exclusively here,
                // never submitted; destroy each once in reverse creation order.
                unsafe {
                    device.destroy_sampler(sprite_sampler);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(e);
            }
        };
        let set1_layout: VkDescriptorSetLayout = match (&sprites.owned, bindless) {
            (Some(fallback), _) => fallback.layout.set_layout(),
            (None, Some(b)) => b.set_layout(),
            (None, None) => {
                unreachable!("invariant: a non-owned sprite set is built only from Some(bindless)")
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
                BindGroupLayoutEntry {
                    binding: 3,
                    count: 1,
                    // UI-ADVANCED S3 (S-D4): the UI's OWN sprite sampler. Declared as a
                    // COMBINED_IMAGE_SAMPLER rather than a plain SAMPLER because the
                    // fragment stage reads only its SAMPLER half — the same
                    // separate-image/separate-sampler access binding 1 already relies on,
                    // and Vulkan's own validation names both types as legal backings for a
                    // shader-declared `SamplerState` (gate G3-0, measured). Reusing the
                    // existing descriptor kind is what let this rung drop S-D4's additive
                    // `DescriptorKind::Sampler` / `BindGroupEntry::Sampler` RHI change.
                    kind: DescriptorKind::CombinedImageSampler,
                    stage: ShaderStage::FRAGMENT,
                },
            ],
        }) {
            Ok(l) => l,
            Err(e) => {
                Self::destroy_sprites(device, sprites);
                Self::destroy_atlas(device, atlas);
                // SAFETY: sampler + both modules were created on `device`, owned
                // exclusively here, referenced by no live pipeline; destroy each once.
                unsafe {
                    device.destroy_sampler(sprite_sampler);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // ALWAYS the 2-set pipeline (S3 amendment): `ui_rect.fs` statically uses set 1, so
        // a one-set layout is a pipeline-create validation error and an every-draw one —
        // measured, quoted in the commit. Set 1 is the shared bindless layout or the
        // UI-owned fallback's; either way the shape is `create_graphics_pipeline_bindless`'s.
        let pipeline = match device.create_graphics_pipeline_bindless(
            &GraphicsPipelineDesc {
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
            },
            set1_layout,
        ) {
            Ok(p) => p,
            Err(e) => {
                // SAFETY: the layout + sprite set + atlas + sampler + both modules were
                // just created on `device`, owned exclusively here, referenced by no live
                // pipeline (the create failed); destroy each once in reverse order.
                unsafe { device.destroy_bind_group_layout(layout) };
                Self::destroy_sprites(device, sprites);
                Self::destroy_atlas(device, atlas);
                unsafe {
                    device.destroy_sampler(sprite_sampler);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // Build the per-FIF rings. On a mid-array failure, every slot built so far
        // (plus the atlas + pipeline/layout/modules) is torn down before returning.
        let mut built: Vec<UiRingSlot> = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let init_bytes = initial_rows as u64 * UI_INSTANCE_SIZE as u64;
        for _ in 0..FRAMES_IN_FLIGHT {
            match Self::create_slot(device, &layout, &atlas, &sprite_sampler, init_bytes) {
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
                    Self::destroy_sprites(device, sprites);
                    Self::destroy_atlas(device, atlas);
                    // SAFETY: the pipeline/layout/sampler/modules above were created on
                    // `device`, owned exclusively here, never submitted; destroy each
                    // once in reverse creation order.
                    unsafe {
                        device.destroy_graphics_pipeline(pipeline);
                        device.destroy_bind_group_layout(layout);
                        device.destroy_sampler(sprite_sampler);
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
            sprite_sampler,
            sprites,
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

    /// The set-1 sprite group BOTH recorders bind (UI-ADVANCED S3, decision S-D9): the
    /// host's shared bindless table or the UI-owned fallback, resolved through ONE
    /// accessor so the offscreen and on-screen paths cannot drift into binding different
    /// sets. Frame-independent (set 1 holds no per-FIF state).
    #[inline]
    pub(crate) fn sprite_group(&self) -> &VulkanBindGroup {
        self.sprites.group()
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
        // UI-ADVANCED S3: the set-1 sprite source goes the same way — a no-op in the
        // `Shared` arm, where the host's `BindlessTextureTable` is the owner.
        Self::destroy_sprites(device, self.sprites);
        Self::destroy_atlas(device, self.atlas);
        // SAFETY: the pipeline/layout/sprite-sampler/modules were created on `device`,
        // owned exclusively here, and the device is idle; each is moved by value ⇒
        // destroyed exactly once, in reverse creation order (the pipeline before its
        // layout + modules). The sprite sampler is destroyed AFTER every bind-group that
        // named it (the slots above and the fallback set-1 group just now).
        unsafe {
            device.destroy_graphics_pipeline(self.pipeline);
            device.destroy_bind_group_layout(self.layout);
            device.destroy_sampler(self.sprite_sampler);
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
    fn create_atlas(
        device: &VulkanContext,
        font: Option<&BakedFont>,
    ) -> Result<UiAtlas, GpuColumnError> {
        // D8e (UI-ADVANCED S3): a sprite-only UI has no font, and the binding must still be
        // filled — an unwritten CombinedImageSampler descriptor is not a legal set. The
        // default fill is one TRANSPARENT texel, so a `FLAG_TEXT` instance on a font-less
        // host (which the pack cannot produce: glyphs come from a `BakedFont`) would
        // contribute nothing rather than sample garbage. `px_range = 1.0` keeps
        // `ui_screen_px_range`'s divisor finite and non-zero for the same reason.
        const DEFAULT_ATLAS_PIXELS: [u8; 4] = [0, 0, 0, 0];
        let (w, h, pixels, px_range): (u32, u32, &[u8], f32) = match font {
            Some(f) => (
                f.atlas.width,
                f.atlas.height,
                &f.atlas.pixels,
                f.meta.distance_range_texels,
            ),
            None => (1, 1, &DEFAULT_ATLAS_PIXELS, 1.0),
        };
        // 2026-07 audit (CRITICAL): this was a `debug_assert` pair — compiled OUT of
        // release, where it mattered. `w`/`h` drive the `vkCmdCopyBufferToImage`
        // extent below while the staging buffer is sized from `pixels.len()`, so a
        // `BakedFont` whose fields disagree makes the DRIVER read past the staging
        // allocation (e.g. 4096x4096 declared, 4 bytes supplied ⇒ a 64 MiB
        // out-of-bounds device read whose result the UI shader then samples), and a
        // zero extent is illegal for `VkImageCreateInfo`. `read_bfont` now rejects
        // both at the file boundary; this is the in-process second line of defence,
        // and it must be a REAL check, not an assertion.
        if w == 0 || h == 0 {
            return Err(GpuColumnError::MalformedAsset(
                "MTSDF atlas extent is zero (VkImageCreateInfo requires w > 0 && h > 0)",
            ));
        }
        if (w as u64) * (h as u64) * 4 != pixels.len() as u64 {
            return Err(GpuColumnError::MalformedAsset(
                "MTSDF atlas is not tightly-packed RGBA8 (pixels.len() != w * h * 4)",
            ));
        }

        // The SAMPLED atlas image.
        let texture = device.create_texture(&TextureDesc {
            width: w,
            height: h,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
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
            px_range,
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

    /// Builds set 1 — the sprite lane's `Texture2D g_sprites[]` (UI-ADVANCED S3).
    ///
    /// `Some(bindless)` takes the HOST's shared table: two raw handles copied into a
    /// non-owning [`UiSpriteSet::Shared`] (see that variant's ownership contract). `None`
    /// builds the UI-owned fallback: a one-descriptor `SAMPLED_IMAGE` set holding a 1×1
    /// transparent texture, so a bindless-less host still gets a bound, valid set 1 and
    /// still boots and draws (gate G3-4 — see [`UiSpriteSet`] for why the one-set pipeline
    /// the plan named is not a legal alternative).
    ///
    /// `sprite_sampler` is named in the fallback's descriptor write but never read there:
    /// binding 0 is a pure `SAMPLED_IMAGE`, and the fragment stage samples it through the
    /// set-0/binding-3 sampler. It is passed rather than a NULL because
    /// [`BindGroupEntry::SampledImage`] takes one.
    fn create_sprite_set(
        device: &VulkanContext,
        bindless: Option<&VulkanBindlessSet>,
        sprite_sampler: &VulkanSampler,
    ) -> Result<UiSpriteSet, GpuColumnError> {
        if let Some(b) = bindless {
            return Ok(UiSpriteSet {
                group: b.as_bind_group(),
                owned: None,
            });
        }

        let layout = device
            .create_bind_group_layout(&BindGroupLayoutDesc {
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    count: 1,
                    kind: DescriptorKind::SampledImage,
                    stage: ShaderStage::FRAGMENT,
                }],
            })
            .map_err(GpuColumnError::Rhi)?;

        // One TRANSPARENT texel — the D8e default-fill shape, reused: every index into this
        // fallback table resolves to it, so nothing a stray sprite could sample is undefined.
        let texture = match Self::create_transparent_texel(device) {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: `layout` was just created on `device`, owned exclusively here,
                // referenced by no live pipeline or set; destroy it once on this edge.
                unsafe { device.destroy_bind_group_layout(layout) };
                return Err(e);
            }
        };

        let group = match device.create_bind_group(&BindGroupDesc {
            layout: &layout,
            entries: &[BindGroupEntry::SampledImage {
                texture: &texture,
                sampler: sprite_sampler,
            }],
        }) {
            Ok(g) => g,
            Err(e) => {
                // SAFETY: `texture`/`layout` were just created on `device`, owned
                // exclusively here, never submitted or bound; destroy each once.
                unsafe {
                    device.destroy_texture(texture);
                    device.destroy_bind_group_layout(layout);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        Ok(UiSpriteSet {
            group,
            owned: Some(UiOwnedSpriteTable { layout, texture }),
        })
    }

    /// A 1×1 fully-transparent `R8G8B8A8Unorm` `SAMPLED` texture in
    /// `ShaderReadOnlyOptimal` — D8e's default fill, used for BOTH the font-less atlas and
    /// the fallback set-1 table. One staged, fence-waited submit (the `upload_atlas_pixels`
    /// path, verbatim).
    fn create_transparent_texel(device: &VulkanContext) -> Result<VulkanTexture, GpuColumnError> {
        let texture = device.create_texture(&TextureDesc {
            width: 1,
            height: 1,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })?;
        if let Err(e) = Self::upload_atlas_pixels(device, &texture, 1, 1, &[0u8; 4]) {
            // SAFETY: `texture` was just created on `device`, owned exclusively here, and
            // the upload's own submit (if it ran) is fence-waited internally before this
            // error could surface; destroy it once on this edge.
            unsafe { device.destroy_texture(texture) };
            return Err(e);
        }
        Ok(texture)
    }

    /// Tears down whatever set 1 OWNS. A set whose `owned` is `None` destroys NOTHING:
    /// those handles belong to the host's `BindlessTextureTable`, and destroying them here
    /// would free a descriptor pool the world's textured passes are still binding.
    fn destroy_sprites(device: &VulkanContext, sprites: UiSpriteSet) {
        let UiSpriteSet { group, owned } = sprites;
        let Some(UiOwnedSpriteTable { layout, texture }) = owned else {
            return;
        };
        // SAFETY: each was created by `create_sprite_set` on `device`, is owned exclusively
        // by this capability (`owned` is `Some`, which is exactly that statement), and the
        // caller drained the device (`destroy`'s `wait_idle`, or a create-path edge where
        // nothing was ever submitted); each is moved by value ⇒ destroyed exactly once,
        // group before its layout, texture last.
        unsafe {
            device.destroy_bind_group(group);
            device.destroy_bind_group_layout(layout);
            device.destroy_texture(texture);
        }
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
    /// `layout`, writing ALL FOUR entries: the ring at binding 0, the `atlas`
    /// texture+sampler at binding 1, the per-atlas UBO at binding 2 (GUI P5b Decision
    /// T4-C), and — since UI-ADVANCED S3 — the UI's own `sprite_sampler` at binding 3,
    /// so a grown slot is complete for the four-binding layout. The buffer is
    /// host-visible + host-coherent (mapped once at create).
    ///
    /// Binding 3's IMAGE half is the atlas texture again, deliberately: the fragment
    /// stage declares only a `SamplerState` there and never reads an image through it, so
    /// the cheapest legal thing to pair with the sampler is a texture already resident and
    /// already in `ShaderReadOnlyOptimal`. That is what let this rung honour S-D4 (the UI
    /// owns its sprite sampler, mode-selectable) with ZERO new descriptor kinds.
    fn create_slot(
        device: &VulkanContext,
        layout: &VulkanBindGroupLayout,
        atlas: &UiAtlas,
        sprite_sampler: &VulkanSampler,
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
                BindGroupEntry::CombinedImage {
                    texture: &atlas.texture,
                    sampler: sprite_sampler,
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
        // GUI P5b + UI-ADVANCED S3: the grown slot's bind-group re-binds all FOUR entries
        // (the atlas + UBO via `self.atlas`, the sprite sampler via `self.sprite_sampler`),
        // so it stays complete for the four-binding layout — the grow-hole fix, widened.
        let new_slot = Self::create_slot(
            device,
            &self.layout,
            &self.atlas,
            &self.sprite_sampler,
            new_cap,
        )?;

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
