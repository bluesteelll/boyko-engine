//! [`CsmResources`] — the CSM cascade + punctual shadow-atlas + SDFDDGI probe
//! trio. Extracted verbatim from the `gpu_scene` boot god-file (a
//! behaviour-preserving module split). Owned by [`super::GpuSceneBundles`],
//! created in its boot orchestrator and torn down in its reverse-order teardown.

use super::*;

use crate::shadow_poison::ShadowPoison;

/// The cascade array's layer count — the `MAX_CASCADES` array the resolve binds whole.
const CASCADE_LAYERS: u32 = 4;

/// The centre texel of every layer of both shadow maps, as raw `f32` bits — what the
/// `BOYKO_SHADOW_POISON_PROBE` readback copies back (see `crate::shadow_poison`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowCentreTexels {
    /// One per cascade layer, in layer order.
    pub(crate) cascade: [u32; CASCADE_LAYERS as usize],
    /// One per atlas layer, in layer order.
    pub(crate) atlas: [u32; SPOT_ATLAS_SLOTS as usize],
}

/// The CSM + shadow-atlas trio (host lift of the showcase's `CsmSceneResources`):
/// ALWAYS created so the resolve set can bind @12/@13 (cascade map + UBO) and
/// @14/@15 (atlas map + UBO) — the resolve SPIR-V statically references them.
/// BOTH sides are live. Every frame the runner memcpys each UBO's frame choice
/// into its ring slot `[token.slot()]` — `ResolvedCsm::frame_uniform` /
/// `ResolvedShadowAtlas::frame_uniform`: the live fit on a frame whose depth pass
/// is armed, the DISABLED bytes on every other frame (shadow gate SG4) — and
/// `scene()` arms the cascade depth pass (`GBufferScene::csm`, host plan R4) and
/// the punctual spot/point depth pass (`GBufferScene::atlas_punctual`: a
/// `CastsPunctualShadow` light holds an atlas slot) when the matching
/// `depth_pass_armed` holds. A leg set without mesh-shadow producers arms neither,
/// on any frame (SG1). Zero-seeded UBOs are the boot OFF state. Both depth maps are
/// one-shot BOOT-TRANSITIONED to `SHADER_READ_ONLY_OPTIMAL` (review R4-W1 — see
/// [`Self::seed_boot_layouts`]); that makes the binding's LAYOUT valid on every
/// frame and defines no texel VALUES. Keeping a never-written layer out of the
/// pixel is the SG1/SG4 gates' job (the one open point-light case is F2 in
/// `boyko_render`'s `sync_punctual_light_gate`, "What a punctual sample can read").
pub(super) struct CsmResources {
    pub(super) cascade: VulkanTexture,
    pub(super) sampler: VulkanSampler,
    /// The cascade UBO RING (one host-coherent slot per in-flight frame),
    /// zero-seeded — bound-but-unread while the depth pass is OFF.
    pub(super) ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    pub(super) depth_pipeline: VulkanGraphicsPipeline,
    depth_vs: VulkanShaderModule,
    depth_fs: VulkanShaderModule,
    pub(super) atlas: VulkanTexture,
    pub(super) atlas_sampler: VulkanSampler,
    /// The shadow-atlas UBO RING (one host-coherent slot per in-flight frame), zero-seeded —
    /// bound-but-unread while the punctual depth pass is OFF. RINGED (was a single buffer): the
    /// atlas fit is CAMERA-DEPENDENT (`spot_priority` = range²/dist²), re-uploaded through the
    /// fenced write token every frame, so it needs a per-in-flight-frame slot exactly like the
    /// CSM cascade UBO.
    pub(super) atlas_ubo: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// SDFDDGI I0: the DDGI grid UBO (single buffer — the grid is world-fixed, so no per-FIF ring),
    /// zero-seeded ⇒ `ddgi_mode_word == 0`, bound-but-unread at resolve binding 18 while the GI gate
    /// is OFF (the default).
    pub(super) ddgi_ubo: BoundBuffer,
    /// SDFDDGI I1: the REAL probe atlas — irradiance (`B10G11R11_UFLOAT`) + depth (`R16G16_SFLOAT`)
    /// `Texture2DArray`s + the per-probe classification buffer + a dedicated LINEAR sampler,
    /// boot-cleared + boot-transitioned to `SHADER_READ_ONLY_OPTIMAL`. Bound at resolve @16/@17
    /// (severing the I0a CSM-cascade/comparison-sampler dummy). Bound-but-UNREAD while the GI gate
    /// is OFF (the default) — the resolve's `SampleLevel`s live INSIDE the `if (ddgi_mode != 0u)`
    /// structural gate (`deferred_pbr.hlsl`), so on the OFF path they never run at all (not merely
    /// ×0), and the swap is byte-identical.
    pub(super) ddgi_atlas: DdgiAtlas,
    /// SDFDDGI I2: the boot-static Fibonacci RAY-TABLE storage buffer (`GI_MAX_RAYS` `float4`s),
    /// boot-filled ONCE with the spherical-Fibonacci directions (identity ray-rotation at I2). A
    /// single host-coherent STORAGE buffer (RHI-owned device buffer — Principle 0, not a host
    /// `Vec`); non-ringed (the table is static, world-fixed grid). Bound at the update set @4 (R);
    /// bound-but-UNREAD while the GI update pass is OFF (the default 0%-gate — `ddgi_update == None`).
    pub(super) ddgi_ray_table: BoundBuffer,
    /// SDFDDGI I2: the probe-update parameter UBO (`DdgiUpdateUbo`, 48 B — the b6 cbuffer mirror),
    /// zero-seeded. A single host-coherent buffer (I2 ships identity ray-rotation → the UBO is
    /// effectively static, so no per-FIF ring). Bound at the update set @6; bound-but-UNREAD while
    /// the GI update pass is OFF (the default 0%-gate).
    pub(super) ddgi_update_ubo: BoundBuffer,
    /// SDFDDGI I2 (the ARM rung): the probe-update `DdgiUpdateResources` — the compute pipeline for
    /// the `GI_MAX_IT_DEFAULT` variant (`sdf_probe_update_spirv`) + its dedicated 7-binding
    /// bind-group layout. Co-located with the atlas/ray-table/UBO it drives (an RHI-owned carrier —
    /// Principle 0). The activation-populate in `Self::scene` borrows these into
    /// `scene.ddgi_update = Some(...)` when GI is enabled; torn down with the atlas. The bind group
    /// itself is written ONCE (non-ringed) by `GBufferTargets` against this layout.
    pub(super) ddgi_update_pipeline: ComputePipeline,
    pub(super) ddgi_update_layout: VulkanBindGroupLayout,
    pub(super) point_depth_pipeline: VulkanGraphicsPipeline,
    point_depth_vs: VulkanShaderModule,
    point_depth_fs: VulkanShaderModule,
    /// The `BOYKO_SHADOW_POISON` diagnostic knob, read once at boot. `None` on every steady run
    /// (no usage bit changes, no clear, no probe).
    poison: Option<ShadowPoison>,
}

impl CsmResources {
    /// Creates the cascade trio + atlas trio + both depth-only pipelines
    /// (mirrors `CsmSceneResources::create`). `instance_layout` is the SAME
    /// set-0 instance-SSBO layout the gbuffer raster pipeline uses.
    pub(super) fn create(device: &VulkanContext, instance_layout: &VulkanBindGroupLayout) -> Self {
        let poison = ShadowPoison::from_env();
        // `TRANSFER_SRC` ONLY under the poison knob, so its probe can copy texels back. Every run
        // a poison gate compares sets the knob, so the compared runs share one usage.
        let probe_usage = if poison.is_some() { ImageUsage::TRANSFER_SRC } else { ImageUsage::NONE };
        let cascade = RhiDevice::create_texture(
            device,
            &TextureDesc {
                width: CSM_SHADOW_DIM,
                height: CSM_SHADOW_DIM,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED | probe_usage,
                array_layers: CASCADE_LAYERS,
                mip_levels: 1,
                view_format: None,
            },
        )
        .expect("invariant: CSM cascade array texture create (setup stage)");
        let sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            },
        )
        .expect("invariant: CSM PCF comparison sampler create");
        let ubo: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: CSM_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: CSM cascade UBO create");
            // Zero seed: csm_mode_word == 0 (bound-but-unread on the OFF path).
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible CSM UBO is mapped");
            zero_fill(mapped, CSM_UBO_BYTES as usize);
            b
        });

        let depth_vs = RhiDevice::create_shader_module(device, csm_depth_vs_spirv())
            .expect("invariant: CSM depth VS module create");
        let depth_fs = RhiDevice::create_shader_module(device, csm_depth_fs_spirv())
            .expect("invariant: CSM depth FS module create");
        // POSITION-ONLY input for the depth-only pipelines: both depth VSes
        // (`csm_depth_vs`/`csm_depth_point_vs`) consume location 0 alone, and declaring the
        // unconsumed normal/color attributes trips the validation layer's
        // "vertex attribute not consumed" warning (the messenger oracle counts warnings).
        // The stride still spans the FULL mesh vertex, so the same vertex buffers bind unchanged.
        let attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        ];
        let depth_bias = Some(DepthBias {
            constant_factor: 0.0015,
            slope_factor: 1.5,
            clamp: 0.0,
        });
        let depth_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &depth_vs,
                vertex_entry: c"main",
                fragment_module: &depth_fs,
                fragment_entry: c"main",
                color_formats: &[],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(instance_layout),
                blend: None,
                cull_mode: CullMode::Front,
                depth_bias,
            },
        )
        .expect("invariant: CSM depth-only graphics pipeline create");

        let atlas = RhiDevice::create_texture(
            device,
            &TextureDesc {
                width: SPOT_SHADOW_DIM,
                height: SPOT_SHADOW_DIM,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED | probe_usage,
                array_layers: SPOT_ATLAS_SLOTS,
                mip_levels: 1,
                view_format: None,
            },
        )
        .expect("invariant: shadow-atlas array texture create");
        let atlas_sampler = RhiDevice::create_sampler(
            device,
            &SamplerDesc {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: AddressMode::ClampToEdge,
                mip: MipMode::None,
                compare: Some(CompareOp::LessOrEqual),
            },
        )
        .expect("invariant: shadow-atlas PCF comparison sampler create");
        let atlas_ubo: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: SPOT_ATLAS_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: shadow-atlas UBO create");
            // Zero seed: mode_word == 0 (bound-but-unread on the OFF path).
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible atlas UBO is mapped");
            zero_fill(mapped, SPOT_ATLAS_UBO_BYTES as usize);
            b
        });

        // SDFDDGI I0: the DDGI grid UBO — a SINGLE host-coherent buffer (the grid is world-fixed,
        // Decision D1, so no per-FIF ring). Zero-seeded ⇒ `ddgi_mode_word == 0`, bound-but-unread at
        // resolve binding 18 while the GI gate is OFF (the default 0%-gate).
        let ddgi_ubo = {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: DDGI_UBO_BYTES,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: SDFDDGI grid UBO create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible DDGI grid UBO is mapped");
            zero_fill(mapped, DDGI_UBO_BYTES as usize);
            b
        };

        // SDFDDGI I1: the REAL probe atlas + classification buffer + LINEAR sampler. Created here,
        // boot-cleared + boot-transitioned to SHADER_READ_ONLY_OPTIMAL inside `DdgiAtlas::create`.
        // Bound at resolve @16/@17 (replacing the I0a dummy) — bound-but-unread while GI is OFF.
        let ddgi_atlas =
            DdgiAtlas::create(device).expect("invariant: SDFDDGI probe atlas create (setup stage)");

        // SDFDDGI I2: the boot-static Fibonacci RAY-TABLE storage buffer — a single host-coherent
        // STORAGE buffer boot-filled ONCE with `GI_MAX_RAYS` spherical-Fibonacci directions (identity
        // ray-rotation at I2). Bound at the update set @4; bound-but-unread while the GI update pass is
        // OFF (the default 0%-gate — `ddgi_update == None`). Principle 0: an RHI-owned device buffer,
        // not a host `Vec`.
        let ddgi_ray_table = {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: DDGI_RAY_TABLE_BYTES,
                    usage: BufferUsage::STORAGE,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: SDFDDGI ray-table create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible DDGI ray table is mapped");
            // Fill the mapped bytes with the unit spherical-Fibonacci directions (the CPU precompute
            // writes directly into the mapped slice — no host scratch `Vec`).
            // SAFETY: `mapped` points at `DDGI_RAY_TABLE_BYTES` host-coherent bytes (= `GI_MAX_RAYS`
            // `[f32; 4]`s); the slice covers exactly that region and every `[f32; 4]` is a POD, so the
            // reinterpret + write only touches owned, correctly-sized memory.
            let rays: &mut [[f32; 4]] = unsafe {
                core::slice::from_raw_parts_mut(mapped.as_ptr().cast::<[f32; 4]>(), GI_MAX_RAYS as usize)
            };
            fill_fibonacci_ray_table(rays);
            b
        };

        // SDFDDGI I2: the probe-update parameter UBO — a single host-coherent buffer (identity
        // ray-rotation → static UBO, no per-FIF ring). Zero-seeded ⇒ bound-but-unread on the OFF path.
        let ddgi_update_ubo = {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: DDGI_UPDATE_UBO_SIZE,
                    usage: BufferUsage::UNIFORM,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: SDFDDGI update UBO create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible DDGI update UBO is mapped");
            // Zero-seed = `DdgiUpdateUbo::ZERO` (bound-but-unread while the update pass is OFF).
            zero_fill(mapped, DDGI_UPDATE_UBO_SIZE as usize);
            // Pin the mirror shape against the host buffer size (a drift is a bug).
            debug_assert_eq!(DDGI_UPDATE_UBO_SIZE as usize, size_of::<DdgiUpdateUbo>());
            b
        };

        // SDFDDGI I2 (the ARM rung): the probe-update `DdgiUpdateResources` — the compute pipeline
        // for the shipped `GI_MAX_IT_DEFAULT` variant + its dedicated 7-binding layout (set 0):
        // t0 `Buf` StorageBuffer (R), u1 `gIrrOut` StorageImage (W), u2 `gDepthOut` StorageImage (W),
        // u3 `Classification` StorageBuffer (RW), t4 `RayTable` StorageBuffer (R), t5 `LightBuf`
        // StorageBuffer (R), b6 `DdgiUpdate` UniformBuffer. The pipeline declares `push_constant_bytes
        // = 4` (the shared compute push range this RHI mandates — a 0-byte range is rejected; the
        // shader reads no push). The shader module is a boot transient dropped after the pipeline
        // captures the compiled state.
        let (ddgi_update_pipeline, ddgi_update_layout) = {
            let module = RhiDevice::create_shader_module(device, sdf_probe_update_spirv())
                .expect("invariant: SDFDDGI probe-update compute shader module create");
            let layout = RhiDevice::create_bind_group_layout(
                device,
                &BindGroupLayoutDesc {
                    entries: &[
                        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                        BindGroupLayoutEntry { binding: 6, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
                    ],
                },
            )
            .expect("invariant: SDFDDGI probe-update bind-group layout create");
            let pipeline = RhiDevice::create_compute_pipeline(
                device,
                &ComputePipelineDesc {
                    module: &module,
                    entry: c"main",
                    // The shared 4-byte compute push range (a 0-byte range is rejected); the update
                    // shader reads no push constant — every param rides the b6 UBO.
                    push_constant_bytes: 4,
                    bind_group_layout: Some(&layout),
                    spec_constants: &[],
                },
            )
            .expect("invariant: SDFDDGI probe-update compute pipeline create");
            // SAFETY: `module` was just created on `device`, never submitted; the pipeline captured
            // the compiled state, so the module is a boot transient destroyed once here.
            unsafe { RhiDevice::destroy_shader_module(device, module) };
            (pipeline, layout)
        };

        let point_depth_vs = RhiDevice::create_shader_module(device, punctual_depth_vs_spirv())
            .expect("invariant: punctual point depth VS module create");
        let point_depth_fs = RhiDevice::create_shader_module(device, punctual_depth_fs_spirv())
            .expect("invariant: punctual point depth FS module create");
        let point_depth_pipeline = RhiDevice::create_graphics_pipeline(
            device,
            &GraphicsPipelineDesc {
                vertex_module: &point_depth_vs,
                vertex_entry: c"main",
                fragment_module: &point_depth_fs,
                fragment_entry: c"main",
                color_formats: &[],
                depth_format: Some(Format::D32Sfloat),
                topology: PrimitiveTopology::TriangleList,
                vertex_layout: Some(VertexBufferLayout {
                    stride: MESH_VERTEX_STRIDE as u32,
                    attributes: &attributes,
                }),
                push_constant_bytes: GBUFFER_PUSH_BYTES as u32,
                bind_group_layout: Some(instance_layout),
                blend: None,
                cull_mode: CullMode::Front,
                depth_bias,
            },
        )
        .expect("invariant: punctual point depth-write graphics pipeline create");

        // ── One-time BOOT LAYOUT SEED (review R4-W1): transition the cascade array
        // + the shadow atlas from their created UNDEFINED layout to
        // SHADER_READ_ONLY_OPTIMAL, fence-waited, before any frame is recorded.
        // The resolve set binds both as combined image+samplers whose descriptors
        // expect SHADER_READ_ONLY_OPTIMAL whether or not any pass ever renders them.
        // This makes every ACCESS valid; it does not make the VALUES defined — a
        // layer no pass wrote holds whatever memory held. Value soundness is the
        // gates': a leg set without mesh-shadow producers never arms a sample of
        // either map (the resolves publish DISABLED fits), and a header bit that
        // trails a disarm or leads the first arming reaches a DISABLED UBO (the
        // runner's step 5d / 5d'; the leading case is measured on a static scene,
        // `taa_jitter_eval`'s frame 0). An earlier version of this comment called an
        // unwritten sample "a benign 1–2 frame artifact"; on a mesh-less VB×Sdf boot
        // it was a permanent dark SDF sphere. The graph's
        // armed-frame transition is unaffected: its seeded model uses `oldLayout =
        // UNDEFINED` (content re-rendered, discard-legal from ANY actual layout).
        Self::seed_boot_layouts(device, &cascade, &atlas);
        // The poison knob (`crate::shadow_poison`): overwrite every layer of both maps with a
        // chosen depth, AFTER the seed, so a frame that samples a layer no pass wrote reads a
        // value the gate controls.
        if let Some(p) = &poison {
            Self::poison_layers(device, &cascade, &atlas, p.depth());
        }

        Self {
            cascade,
            sampler,
            ubo,
            depth_pipeline,
            depth_vs,
            depth_fs,
            atlas,
            atlas_sampler,
            atlas_ubo,
            ddgi_ubo,
            ddgi_atlas,
            ddgi_ray_table,
            ddgi_update_ubo,
            ddgi_update_pipeline,
            ddgi_update_layout,
            point_depth_pipeline,
            point_depth_vs,
            point_depth_fs,
            poison,
        }
    }

    /// Records + submits the one-shot boot transition of the cascade array +
    /// shadow atlas to `SHADER_READ_ONLY_OPTIMAL` (see the call-site comment in
    /// [`Self::create`]), fence-waited; the encoder + fence are setup-class
    /// transients torn down here (the `BrickClipmap::upload_region` boot-submit
    /// shape). Panics on any RHI failure — a setup-stage failure by design.
    fn seed_boot_layouts(device: &VulkanContext, cascade: &VulkanTexture, atlas: &VulkanTexture) {
        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: CSM boot-layout command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: CSM boot-layout fence create");
        encoder.begin().expect("invariant: CSM boot-layout encoder begin");
        for (texture, layer_count) in [(cascade, CASCADE_LAYERS), (atlas, SPOT_ATLAS_SLOTS)] {
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::COMPUTE_SHADER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range: ImageSubresourceRange {
                    aspect: ImageAspect::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                },
            });
        }
        encoder.end().expect("invariant: CSM boot-layout encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: CSM boot-layout submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: CSM boot-layout fence wait");
        // SAFETY: `encoder` and `fence` were created on `device` above; the
        // encoder's ONLY submission completed (the fence wait just returned), so
        // no GPU work references either; each is moved by value ⇒ destroyed
        // exactly once. Boot-stage: no other submission is in flight (the scene
        // boot runs before the first frame, and the only earlier boot submit —
        // the brick clip-map bake — is itself fence-waited).
        unsafe {
            RhiDevice::destroy_command_encoder(device, encoder);
            RhiDevice::destroy_fence(device, fence);
        }
    }

    /// The poison knob's boot fill: every layer of the cascade array and the atlas cleared to
    /// `depth`, through a CLEAR-only dynamic-rendering scope per layer
    /// (`VulkanCommandEncoder::clear_depth_layers` — no `TRANSFER_DST` usage needed), and left
    /// at `SHADER_READ_ONLY_OPTIMAL` exactly as the seed left them. One fence-waited submit; the
    /// encoder and fence are setup-class transients, the [`Self::seed_boot_layouts`] shape.
    /// Cold: runs only under `BOYKO_SHADOW_POISON`. Panics on any RHI failure (setup stage).
    #[cold]
    #[inline(never)]
    fn poison_layers(device: &VulkanContext, cascade: &VulkanTexture, atlas: &VulkanTexture, depth: f32) {
        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: shadow-poison command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: shadow-poison fence create");
        encoder.begin().expect("invariant: shadow-poison encoder begin");
        for (texture, layer_count, extent) in
            [(cascade, CASCADE_LAYERS, CSM_SHADOW_DIM), (atlas, SPOT_ATLAS_SLOTS, SPOT_SHADOW_DIM)]
        {
            let range = ImageSubresourceRange {
                aspect: ImageAspect::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count,
            };
            // `UNDEFINED` as the old layout: every texel is about to be cleared, so the seed's
            // (undefined) content is discarded on purpose. The seed's submit is fence-waited, so
            // no access precedes this in the queue.
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TOP_OF_PIPE,
                dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_READ
                    | BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
                old_layout: ImageLayout::Undefined,
                new_layout: ImageLayout::DepthAttachmentOptimal,
                range,
            });
            encoder.clear_depth_layers(texture, extent, depth);
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::LATE_FRAGMENT_TESTS,
                dst_stage: BarrierStage::COMPUTE_SHADER | BarrierStage::FRAGMENT_SHADER,
                src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::DepthAttachmentOptimal,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range,
            });
        }
        encoder.end().expect("invariant: shadow-poison encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: shadow-poison submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: shadow-poison fence wait");
        // SAFETY: `encoder` and `fence` were created on `device` above; the encoder's ONLY
        // submission completed (the fence wait just returned), so no GPU work references either;
        // each is moved by value ⇒ destroyed exactly once. Boot stage: the only earlier submits
        // (the brick bake, the layout seed) are themselves fence-waited.
        unsafe {
            RhiDevice::destroy_command_encoder(device, encoder);
            RhiDevice::destroy_fence(device, fence);
        }
    }

    /// The poison probe's readback: the CENTRE texel of every layer of both maps, copied out of
    /// band — the device is idled first, then one fence-waited transfer submit brackets the
    /// copies with `SHADER_READ_ONLY_OPTIMAL ⇄ TRANSFER_SRC_OPTIMAL` transitions, so the maps
    /// are left in the layout every frame's descriptors expect. The particle-counter readback's
    /// shape; affordable because the caller ends the frame loop straight after.
    ///
    /// `None` when the poison knob is unset: the images then lack `TRANSFER_SRC` usage, and a
    /// readback of unpoisoned maps would prove nothing.
    ///
    /// The `SHADER_READ_ONLY_OPTIMAL` old layout is the one every path leaves both maps in at
    /// frame end (the resolve set binds them whole, every frame). On a boot that never renders a
    /// map, the graph's per-frame transition to that layout is the discard-legal
    /// `UNDEFINED → SHADER_READ_ONLY`, so the poison surviving to this read is a property of the
    /// driver, which is exactly what the gate's texel check measures rather than assumes.
    #[cold]
    #[inline(never)]
    pub(super) fn read_centre_texels(&self, device: &VulkanContext) -> Option<ShadowCentreTexels> {
        self.poison.as_ref()?;
        const CASCADE_N: usize = CASCADE_LAYERS as usize;
        const ATLAS_N: usize = SPOT_ATLAS_SLOTS as usize;
        const TEXEL_BYTES: u64 = 4;
        const TOTAL: u64 = ((CASCADE_N + ATLAS_N) as u64) * TEXEL_BYTES;

        RhiDevice::wait_idle(device).expect("invariant: shadow-poison probe device idle");
        let staging = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: TOTAL,
                usage: BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: shadow-poison probe staging create");
        let mapped = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("invariant: host-visible shadow-poison probe staging is mapped");

        let region = |layer: u32, dim: u32, slot: u64| boyko_rhi::BufferImageCopy {
            buffer_offset: slot * TEXEL_BYTES,
            buffer_row_length: 0,
            buffer_image_height: 0,
            aspect: ImageAspect::DEPTH,
            mip_level: 0,
            base_array_layer: layer,
            layer_count: 1,
            image_offset_x: (dim / 2) as i32,
            image_offset_y: (dim / 2) as i32,
            image_offset_z: 0,
            image_extent_w: 1,
            image_extent_h: 1,
            image_extent_d: 1,
        };
        let cascade_regions: [boyko_rhi::BufferImageCopy; CASCADE_N] =
            core::array::from_fn(|i| region(i as u32, CSM_SHADOW_DIM, i as u64));
        let atlas_regions: [boyko_rhi::BufferImageCopy; ATLAS_N] =
            core::array::from_fn(|i| region(i as u32, SPOT_SHADOW_DIM, (CASCADE_N + i) as u64));

        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: shadow-poison probe command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: shadow-poison probe fence create");
        encoder.begin().expect("invariant: shadow-poison probe encoder begin");
        for (texture, layer_count) in [(&self.cascade, CASCADE_LAYERS), (&self.atlas, SPOT_ATLAS_SLOTS)] {
            // Availability: the last armed frame's depth pass wrote these layers as a depth
            // attachment; the idle above ordered the execution, this makes the writes visible to
            // the transfer read.
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::LATE_FRAGMENT_TESTS
                    | BarrierStage::FRAGMENT_SHADER
                    | BarrierStage::COMPUTE_SHADER,
                dst_stage: BarrierStage::TRANSFER,
                src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
                dst_access: BarrierAccess::TRANSFER_READ,
                old_layout: ImageLayout::ShaderReadOnlyOptimal,
                new_layout: ImageLayout::TransferSrcOptimal,
                range: ImageSubresourceRange {
                    aspect: ImageAspect::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                },
            });
        }
        encoder.copy_image_to_buffer(&self.cascade, ImageLayout::TransferSrcOptimal, &staging, &cascade_regions);
        encoder.copy_image_to_buffer(&self.atlas, ImageLayout::TransferSrcOptimal, &staging, &atlas_regions);
        for (texture, layer_count) in [(&self.cascade, CASCADE_LAYERS), (&self.atlas, SPOT_ATLAS_SLOTS)] {
            encoder.image_barrier(&ImageBarrierDesc {
                texture,
                src_stage: BarrierStage::TRANSFER,
                dst_stage: BarrierStage::COMPUTE_SHADER | BarrierStage::FRAGMENT_SHADER,
                src_access: BarrierAccess::NONE,
                dst_access: BarrierAccess::SHADER_READ,
                old_layout: ImageLayout::TransferSrcOptimal,
                new_layout: ImageLayout::ShaderReadOnlyOptimal,
                range: ImageSubresourceRange {
                    aspect: ImageAspect::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                },
            });
        }
        encoder.end().expect("invariant: shadow-poison probe encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: shadow-poison probe submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: shadow-poison probe fence wait");

        let mut words = [0u32; CASCADE_N + ATLAS_N];
        // SAFETY: `mapped` addresses `TOTAL` valid mapped host-coherent bytes of a buffer this fn
        // just created; the fence wait above completed the ONLY submission that writes them, so
        // the copies are complete and (the memory being HOST_COHERENT) host-visible. `words` is a
        // fresh local of exactly `TOTAL` bytes, every bit pattern of a `u32` is valid, and the two
        // regions do not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(
                mapped.as_ptr(),
                words.as_mut_ptr().cast::<u8>(),
                TOTAL as usize,
            );
        }
        // SAFETY: `encoder`, `fence` and `staging` were created on `device` above; the encoder's
        // only submission completed (the fence wait returned), so no GPU work references any of
        // them; each is moved by value ⇒ destroyed exactly once.
        unsafe {
            RhiDevice::destroy_command_encoder(device, encoder);
            RhiDevice::destroy_fence(device, fence);
            RhiDevice::destroy_buffer(device, staging);
        }

        let mut texels =
            ShadowCentreTexels { cascade: [0; CASCADE_N], atlas: [0; ATLAS_N] };
        texels.cascade.copy_from_slice(&words[..CASCADE_N]);
        texels.atlas.copy_from_slice(&words[CASCADE_N..]);
        Some(texels)
    }

    /// Tears the trio down in reverse creation order (mirrors
    /// `CsmSceneResources::destroy`).
    ///
    /// # Safety
    /// Each resource was created on `device`, the device is idle (the caller's
    /// renderer drop waited), and each is destroyed exactly once (by-value).
    pub(super) unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract `device` is live + idle and nothing
        // references these resources; reverse creation order.
        unsafe {
            RhiDevice::destroy_graphics_pipeline(device, self.point_depth_pipeline);
            RhiDevice::destroy_shader_module(device, self.point_depth_fs);
            RhiDevice::destroy_shader_module(device, self.point_depth_vs);
            // SDFDDGI I2 (arm): the probe-update pipeline + its bind-group layout (reverse creation
            // order — created after ddgi_update_ubo, before the point-depth pipeline).
            RhiDevice::destroy_compute_pipeline(device, self.ddgi_update_pipeline);
            RhiDevice::destroy_bind_group_layout(device, self.ddgi_update_layout);
            // SDFDDGI I2: the probe-update UBO + Fibonacci ray-table (reverse creation order —
            // created after ddgi_atlas).
            RhiDevice::destroy_buffer(device, self.ddgi_update_ubo);
            RhiDevice::destroy_buffer(device, self.ddgi_ray_table);
            // SDFDDGI I1: the probe atlas + classification buffer + LINEAR sampler (reverse creation
            // order — created after ddgi_ubo). `DdgiAtlas::destroy` is `unsafe` on the same device-idle
            // contract this block already upholds (the caller drained the device).
            self.ddgi_atlas.destroy(device);
            // SDFDDGI I0: the single DDGI grid UBO (reverse creation order — created after atlas_ubo).
            RhiDevice::destroy_buffer(device, self.ddgi_ubo);
            for slot in self.atlas_ubo {
                RhiDevice::destroy_buffer(device, slot);
            }
            RhiDevice::destroy_sampler(device, self.atlas_sampler);
            RhiDevice::destroy_texture(device, self.atlas);
            RhiDevice::destroy_graphics_pipeline(device, self.depth_pipeline);
            RhiDevice::destroy_shader_module(device, self.depth_fs);
            RhiDevice::destroy_shader_module(device, self.depth_vs);
            for slot in self.ubo {
                RhiDevice::destroy_buffer(device, slot);
            }
            RhiDevice::destroy_sampler(device, self.sampler);
            RhiDevice::destroy_texture(device, self.cascade);
        }
    }
}

impl GpuSceneBundles {
    /// The `BOYKO_SHADOW_POISON_PROBE` path, when the poison knob armed the probe at boot — the
    /// runner builds its loop-exit driver from THIS, so the boot that allocated the probe's
    /// `TRANSFER_SRC` usage and the driver that reads it back come from one env read.
    pub(crate) fn shadow_poison_probe(&self) -> Option<&std::path::Path> {
        self.csm.poison.as_ref().and_then(ShadowPoison::probe)
    }

    /// The depth the poison knob cleared every shadow layer to at boot; `None` when unset.
    pub(crate) fn shadow_poison_depth(&self) -> Option<f32> {
        self.csm.poison.as_ref().map(ShadowPoison::depth)
    }

    /// The poison probe's out-of-band centre-texel readback (see
    /// [`CsmResources::read_centre_texels`]); `None` when the knob is unset. Idles the device —
    /// the caller ends the frame loop straight after.
    pub(crate) fn read_shadow_centre_texels(&self, ctx: &VulkanContext) -> Option<ShadowCentreTexels> {
        self.csm.read_centre_texels(ctx)
    }
}
