//! [`CsmResources`] — the CSM cascade + punctual shadow-atlas + SDFDDGI probe
//! trio. Extracted verbatim from the `gpu_scene` boot god-file (a
//! behaviour-preserving module split). Owned by [`super::GpuSceneBundles`],
//! created in its boot orchestrator and torn down in its reverse-order teardown.

use super::*;

/// The CSM + shadow-atlas trio (host lift of the showcase's `CsmSceneResources`):
/// ALWAYS created so the resolve set can bind @12/@13 (cascade map + UBO) and
/// @14/@15 (atlas map + UBO) — the resolve SPIR-V statically references them.
/// Since host plan R4 the CASCADE side is live: the runner memcpys the frame's
/// `ResolvedCsm` into `ubo[token.slot()]` and `scene()` arms the depth pass when
/// the ECS predicate holds (zero-seeded UBOs = the boot OFF state). Both depth
/// maps are one-shot BOOT-TRANSITIONED to `SHADER_READ_ONLY_OPTIMAL` (review
/// R4-W1 — see [`Self::seed_boot_layouts`]), so a resolve that reaches them
/// under a stale header gate before any depth pass ever recorded samples a
/// DEFINED layout. The punctual ATLAS side stays OFF (`atlas_punctual == None`,
/// `mode_word == 0` ⇒ bound-but-unread) — the shadowed-punctual composition is
/// a later rung.
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
}

impl CsmResources {
    /// Creates the cascade trio + atlas trio + both depth-only pipelines
    /// (mirrors `CsmSceneResources::create`). `instance_layout` is the SAME
    /// set-0 instance-SSBO layout the gbuffer raster pipeline uses.
    pub(super) fn create(device: &VulkanContext, instance_layout: &VulkanBindGroupLayout) -> Self {
        let cascade = RhiDevice::create_texture(
            device,
            &TextureDesc {
                width: CSM_SHADOW_DIM,
                height: CSM_SHADOW_DIM,
                depth: 1,
                format: Format::D32Sfloat,
                dimension: TextureDimension::D2,
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                array_layers: 4,
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
        let attributes = [
            VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 2, offset: 12, format: VertexFormat::Float32x3 },
            VertexAttribute { location: 1, offset: 24, format: VertexFormat::Float32x4 },
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
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
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
        // expect SHADER_READ_ONLY_OPTIMAL; on the armed path the depth pass
        // transitions them every frame, but the header CSM gate can lag the arming
        // predicate by a frame (cross-plugin ordering is unconstrained), so a
        // multi-coincidence exists where the resolve samples the cascade on a frame
        // stream where the depth pass NEVER ran — this seed closes that
        // never-rendered class categorically (a sample then reads undefined VALUES
        // at a DEFINED layout: a benign 1–2 frame shadow artifact, never an invalid
        // access). The graph's armed-frame transition is unaffected: its seeded
        // model uses `oldLayout = UNDEFINED` (content re-rendered, discard-legal
        // from ANY actual layout).
        Self::seed_boot_layouts(device, &cascade, &atlas);

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
        for (texture, layer_count) in [(cascade, 4u32), (atlas, SPOT_ATLAS_SLOTS)] {
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
