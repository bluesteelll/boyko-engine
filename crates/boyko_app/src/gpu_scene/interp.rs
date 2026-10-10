//! [`InterpGpuProd`] — the B3 interpolation pre-pass GPU resources (host plan
//! R5, refined-B). Extracted verbatim from the `gpu_scene` boot god-file (a
//! behaviour-preserving module split). Owned by [`super::GpuSceneBundles`].

use super::*;

/// The productionized B3 interpolation pre-pass GPU resources (host plan D7/R5,
/// refined-B): the interp compute pipeline + its 3-binding set layout, and the
/// `FRAMES_IN_FLIGHT`-ringed pair / out-slot SSBOs (frame-private, like the G-buffer
/// ring). The COMPUTE output target is the SHARED instance ring (`instance_rings`),
/// NOT a private draw ring — refined-B retires the private draw/draw_bg rings so
/// static and interpolated instances share ONE draw-ordered buffer.
///
/// The host writes this frame's `pairs[fi]` + `out_slot[fi]` (from the World's
/// `MeshRenderScratch` `pair_ring` / `pair_out_slot` lanes — the source of truth
/// stays in the World, Principle 0: the host only memcpys them) and CPU-scatters the
/// static rows into `instance_rings[fi]`; the interp compute reads `pairs[fi]` +
/// `out_slot[fi]` and OVERWRITES ONLY the dynamic slots of `instance_rings[fi]`; the
/// raster / CSM / atlas VS read that SAME shared ring via
/// `GBufferScene::instance_bind_group` = `instance_bind_groups[fi]` (unchanged — the
/// recorder binds one instance set, raster+CSM+atlas 3-pass reuse). Owned by
/// [`GpuSceneBundles`], created at boot, destroyed in the explicit reverse-order
/// teardown.
///
/// # Sizing
///
/// Both the pair ring and the out-slot ring are sized to [`INSTANCE_CAPACITY`] (the
/// same budget as the affine instance ring), so any per-frame gather up to the budget
/// fits. The PER-FRAME dynamic count (which may be `< INSTANCE_CAPACITY`) is the
/// activation's `instance_count` — the dispatch bound and the shader's loop guard;
/// the built `capacity` only bounds the SSBOs.
pub(super) struct InterpGpuProd {
    /// The B2 interp compute pipeline (`interp_instances.comp`).
    pipeline: ComputePipeline,
    /// The 3-binding interp set layout { pairs @0 (read), out_slot @1 (read),
    /// model_out @2 (write) }, all COMPUTE.
    layout: VulkanBindGroupLayout,
    /// FIF ring of pair SSBOs (host-written, COMPUTE-read); [`INSTANCE_CAPACITY`] × 96 B.
    pub(super) pairs: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of out-slot SSBOs (host-written, COMPUTE-read); [`INSTANCE_CAPACITY`] × 4 B.
    /// `out_slot[fi][d]` is dynamic instance `d`'s offset into the shared instance ring
    /// (the gather's `pair_out_slot` lane — the shader's `OutSlot` binding).
    pub(super) out_slot: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of interp bind groups { pairs[fi] @0, out_slot[fi] @1,
    /// instance_rings[fi] @2 } on [`Self::layout`] — the model_out target is the SHARED
    /// instance ring, so the compute writes what the raster VS reads (no private ring).
    interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The built SSBO capacity in instances, PER FIF SLOT ([`INSTANCE_CAPACITY`] at
    /// boot) — the SSBO bound, NOT the per-frame dispatch count (that arrives via the
    /// activation). Asset-streaming plan F7 §7.3: [`GpuSceneBundles::
    /// grow_instance_family_if_needed`](super::GpuSceneBundles::grow_instance_family_if_needed)
    /// grows ONE slot at a time (in lockstep with `instance_rings[s]`), so slots may sit
    /// at DIFFERENT capacities between grows — a single shared scalar would go stale for
    /// whichever slot last grew.
    capacity: [u32; FRAMES_IN_FLIGHT],
}

impl InterpGpuProd {
    /// Builds the interp pipeline + the FIF-ringed pair/out-slot SSBOs + their bind
    /// groups (whose model_out target is the SHARED `instance_rings`) sized to
    /// `capacity` instances. `instance_rings` is the gbuffer set-0 instance ring the
    /// raster VS reads — the compute writes it directly (refined-B). `capacity` must
    /// be ≥ 1.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — a setup-stage
    /// device failure by design (the `GpuSceneBundles::boot` contract).
    pub(super) fn create(
        device: &VulkanContext,
        instance_rings: &[BoundBuffer; FRAMES_IN_FLIGHT],
        capacity: u32,
    ) -> Self {
        debug_assert!(capacity >= 1, "invariant: the interp pass needs at least one instance slot");
        let cs = RhiDevice::create_shader_module(device, interp_instances_spirv())
            .expect("invariant: B3 interp compute shader module create");
        let layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: B3 interp bind-group layout create");
        let pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: INTERP_INSTANCES_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: B3 interp compute pipeline create");

        let pair_bytes = capacity as u64 * GPU_TRANSFORM3D_BYTES as u64;
        // The out-slot lane is one u32 per dynamic instance (the shader's `OutSlot`).
        let out_slot_bytes = capacity as u64 * 4;
        let make_buf = |size: u64, what: &str| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc { size, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent },
            )
            .unwrap_or_else(|e| panic!("invariant: B3 interp {what} SSBO create: {e:?}"));
            // Zero-seed: a fresh sub-allocation carries prior bytes, and a first
            // frame binds the ring before its first host write.
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .unwrap_or_else(|| panic!("invariant: host-visible B3 interp {what} SSBO is mapped"));
            zero_fill(mapped, size as usize);
            b
        };
        // Each ring slot is a distinct frame-private SSBO (host-coherent so the pair /
        // out-slot writes need no explicit flush).
        let pairs: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(pair_bytes, "pairs"));
        let out_slot: [BoundBuffer; FRAMES_IN_FLIGHT] =
            core::array::from_fn(|_| make_buf(out_slot_bytes, "out_slot"));
        // interp_bg binds { pairs[fi] @0, out_slot[fi] @1, instance_rings[fi] @2 } —
        // the model_out target is the SHARED instance ring (refined-B): the compute
        // writes the dynamic slots the raster VS then reads, on the SAME buffer.
        let interp_bg: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &pairs[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &out_slot[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &instance_rings[fi] },
                    ],
                },
            )
            .expect("invariant: B3 interp bind group create")
        });

        // The shader module is consumed by pipeline creation; destroy it now
        // (the ComputePipeline owns the compiled state — mirrors the boot's
        // post-create module teardown).
        // SAFETY: `cs` was created on `device` above and is no longer needed once
        // the pipeline exists; it is destroyed exactly once; no GPU work has been
        // submitted yet (boot stage).
        unsafe {
            RhiDevice::destroy_shader_module(device, cs);
        }

        Self {
            pipeline,
            layout,
            pairs,
            out_slot,
            interp_bg,
            capacity: [capacity; FRAMES_IN_FLIGHT],
        }
    }

    /// The [`InterpActivation`] for this frame slot `fi` and this frame's overstep
    /// `alpha`: the interp set (pairs@0 read + out_slot@1 read + model_out@2 write for
    /// slot `fi`), the pair / out-slot / model-out slot buffers, the per-frame dynamic
    /// `instance_count`, and `alpha`. `model_out_buffer` is the SHARED instance ring
    /// slot (`instance_rings[fi]`), which the caller ALSO binds as
    /// `GBufferScene::instance_bind_group` for the raster read — the ring contract.
    ///
    /// `instance_count` is THIS frame's gathered DYNAMIC count (the dispatch bound +
    /// the push count) — the caller passes the gather's `dynamic_count()`, which the
    /// pair-ring upload already hard-asserts fits `capacity`.
    #[inline]
    pub(super) fn activation<'a>(
        &'a self,
        fi: usize,
        model_out: &'a BoundBuffer,
        instance_count: u32,
        alpha: f32,
    ) -> InterpActivation<'a> {
        debug_assert!(
            instance_count <= self.capacity[fi],
            "invariant: the per-frame interp instance count fits slot fi's built SSBO capacity"
        );
        InterpActivation {
            pipeline: &self.pipeline,
            interp_set: &self.interp_bg[fi],
            pair_buffer: &self.pairs[fi],
            out_slot_buffer: &self.out_slot[fi],
            model_out_buffer: model_out,
            instance_count,
            alpha,
        }
    }

    /// Asset-streaming plan F7 §7.3: reallocates slot `s`'s pair/out-slot SSBOs to
    /// `new_cap` instances (in lockstep with the caller's `instance_rings[s]` grow,
    /// passed in as `model_out` — the SAME buffer the caller just repointed
    /// `instance_bind_groups[s]`@0 against) and repoints ALL THREE of `interp_bg[s]`'s
    /// bindings in place: `pairs`@0, `out_slot`@1, `model_out`@2.
    ///
    /// NO seed: `upload_pair_ring` / `upload_pair_out_slot` rewrite the whole ring THIS
    /// frame (mirrors the instance ring's own no-seed growth, F7 §7.3 step 4) — only a
    /// `write_bytes(0)` covers the gap until those writes land. The old pair/out-slot
    /// buffers are routed through `retired` at `retire_frame` (the caller's
    /// `epoch + RETIRE_DELAY`); `model_out`'s old buffer is the caller's own
    /// `instance_rings[s]`, already routed by the caller.
    ///
    /// # Panics
    /// Panics (`expect`/`unwrap_or_else`) on an RHI create/map failure — a device OOM on
    /// a post-boot grow is a setup-adjacent failure, not a recoverable per-frame error
    /// (mirrors [`Self::create`]'s boot-time panics).
    ///
    /// # Safety
    /// The caller guarantees slot `s`'s in-flight fence was waited THIS frame (the
    /// `FrameWriteToken` proof `GpuSceneBundles::grow_instance_family_if_needed` holds)
    /// — `interp_bg[s]`'s descriptor set is not command-buffer-pending, so rewriting its
    /// bindings in place is sound. `device` must be the live context every prior buffer
    /// here (and `model_out`) was created on.
    pub(super) unsafe fn grow_slot(
        &mut self,
        device: &VulkanContext,
        s: usize,
        new_cap: u32,
        model_out: &BoundBuffer,
        retired: &mut RetiredGpuBuffers,
        retire_frame: u64,
    ) {
        let pair_bytes = new_cap as u64 * GPU_TRANSFORM3D_BYTES as u64;
        let out_slot_bytes = new_cap as u64 * 4;
        let make_buf = |size: u64, what: &str| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc { size, usage: BufferUsage::STORAGE, location: MemoryLocation::HostVisibleCoherent },
            )
            .unwrap_or_else(|e| panic!("invariant: grown B3 interp {what} SSBO create: {e:?}"));
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b).unwrap_or_else(|| {
                panic!("invariant: host-visible grown B3 interp {what} SSBO is mapped")
            });
            zero_fill(mapped, size as usize);
            b
        };
        let new_pairs = make_buf(pair_bytes, "pairs");
        let new_out_slot = make_buf(out_slot_bytes, "out_slot");

        let old_pairs = core::mem::replace(&mut self.pairs[s], new_pairs);
        let old_out_slot = core::mem::replace(&mut self.out_slot[s], new_out_slot);
        retired.push(old_pairs, retire_frame);
        retired.push(old_out_slot, retire_frame);

        // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
        // above); its descriptor set is therefore non-pending — rewriting all three of
        // its bindings in place is sound.
        unsafe {
            rebind_storage_buffer(device, &self.interp_bg[s], 0, &self.pairs[s]);
            rebind_storage_buffer(device, &self.interp_bg[s], 1, &self.out_slot[s]);
            rebind_storage_buffer(device, &self.interp_bg[s], 2, model_out);
        }
        self.capacity[s] = new_cap;
    }

    /// Tears every owned resource down in reverse dependency order (interp_bg →
    /// out_slot → pairs → pipeline → layout). The SHARED instance rings are owned by
    /// `GpuSceneBundles`, not this struct, so they are NOT destroyed here.
    ///
    /// # Safety
    /// The device is idle (the caller's renderer drop waited) so no submission
    /// references these; each is destroyed exactly once (by-value `self`); `device`
    /// is the live context they were created on.
    pub(super) unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract the device is idle + live; reverse creation order.
        unsafe {
            for bg in self.interp_bg {
                RhiDevice::destroy_bind_group(device, bg);
            }
            for b in self.out_slot {
                RhiDevice::destroy_buffer(device, b);
            }
            for b in self.pairs {
                RhiDevice::destroy_buffer(device, b);
            }
            RhiDevice::destroy_compute_pipeline(device, self.pipeline);
            RhiDevice::destroy_bind_group_layout(device, self.layout);
        }
    }
}
