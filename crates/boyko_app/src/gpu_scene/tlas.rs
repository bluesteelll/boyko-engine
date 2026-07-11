//! [`TlasResources`] — the HW-RT rung R2a-3 GPU-resident per-frame TLAS
//! resources (feature `hwrt`). Extracted verbatim from the `gpu_scene` boot
//! god-file (a behaviour-preserving module split). Owned by
//! [`super::GpuSceneBundles`]. The whole module is `hwrt`-gated at its
//! declaration in `mod.rs`, so the inner `#[cfg(feature = "hwrt")]` attributes
//! are preserved verbatim (redundant but harmless).

use super::*;

/// HW-RT rung R2a-3: the GPU-resident per-frame TLAS resources — the named owner (mirrors
/// [`InterpGpuProd`] / [`CsmResources`]) of every device buffer + the persistent per-slot TLAS
/// the host rebuilds each frame from the compute-written instance array (Principle 0: a named
/// owner is not a side store; the durable per-mesh BLAS data lives ON `MeshGpu`, not here).
///
/// Per-FIF duplication of (mesh-id ring, instance array, TLAS backing/scratch): `drive_frame`
/// waits slot `fi`'s in-flight fence BEFORE recording, so slot `fi`'s previous-use GPU reads
/// (pack read of its instance array, build read of its scratch, future trace of its TLAS) all
/// completed → the host rebuilds slot `fi` race-free while the sibling frame uses the other slot
/// (the same discipline the instance ring uses). The `blas_addr` table is frame-INVARIANT (a
/// BLAS never moves at ITS OWN slot, but F6's fence-gated retire can FREE a slot and a later
/// re-add can REUSE it for a DIFFERENT mesh's BLAS) → a single host-visible column, rewritten
/// whenever the mesh table's `install_epoch` advances (asset-streaming plan F6 — see
/// [`TlasResources::sync_blas_addr`]'s doc for why row-count growth alone cannot gate this).
#[cfg(feature = "hwrt")]
pub(super) struct TlasResources {
    /// The R2a-3 TLAS-instance packer compute pipeline (`build_tlas_instances.comp`).
    pipeline: ComputePipeline,
    /// The 4-binding pack set layout { M3 ring @0 (read), mesh-ids @1 (read), blas-addr @2
    /// (read), instance-array @3 (write) }, all COMPUTE.
    layout: VulkanBindGroupLayout,
    /// FIF ring of mesh-id SSBOs (host-written, COMPUTE-read); [`INSTANCE_CAPACITY`] × 4 B.
    mesh_id_rings: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of `VkAccelerationStructureInstanceKHR[]` output arrays (COMPUTE-written,
    /// AS_BUILD-read); [`INSTANCE_CAPACITY`] × 64 B. `STORAGE | ACCEL_BUILD_INPUT |
    /// SHADER_DEVICE_ADDRESS`. GPU-ONLY (never CPU-touched) → DEVICE-LOCAL VRAM (unmappable, no
    /// boot seed — the pack fully overwrites `[0..count)` each frame).
    instance_arrays: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// FIF ring of PERSISTENT TLASes (backing + scratch sized ONCE for [`INSTANCE_CAPACITY`]),
    /// built into each frame from `instance_arrays[fi]`.
    tlas: [PersistentTlas; FRAMES_IN_FLIGHT],
    /// FIF ring of pack bind groups { instance_rings[fi] @0, mesh_id_rings[fi] @1, blas_addr @2,
    /// instance_arrays[fi] @3 }.
    bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The per-mesh BLAS device-address table ([`MESH_ADDR_CAP`] × 8 B, host-visible u64 column):
    /// `blas_addr[m]` is mesh `m`'s BLAS device address (frame-invariant → a single buffer, no
    /// ring). Rewritten only when [`Assets::install_epoch`](boyko_ecs::ecs::core::asset::Assets::install_epoch)
    /// advances (asset-streaming plan F6 — see [`sync_blas_addr`](Self::sync_blas_addr)'s doc for
    /// why this table's staleness gate cannot use row-count growth alone).
    blas_addr: BoundBuffer,
    /// The cached device address of each `instance_arrays[fi]` (the per-frame build's
    /// instance-array address), filled once at create.
    instance_array_addr: [u64; FRAMES_IN_FLIGHT],
    /// The built SSBO capacity in instances, PER FIF SLOT ([`INSTANCE_CAPACITY`] at boot)
    /// — the sizing MAX + the per-frame count's `debug_assert` bound. Asset-streaming
    /// plan F7-hwrt (task#11, review O1): [`Self::grow_slot`] grows ONE slot at a time
    /// (in lockstep with the caller's `instance_rings[s]`), so slots may sit at DIFFERENT
    /// capacities between grows — a single shared scalar would go stale for whichever
    /// slot last grew (mirrors [`InterpGpuProd::capacity`](super::interp::InterpGpuProd)'s
    /// identical per-slot shape).
    capacity: [u32; FRAMES_IN_FLIGHT],
    /// The last [`Assets::install_epoch`](boyko_ecs::ecs::core::asset::Assets::install_epoch) the
    /// `blas_addr` table reflects (interior-mutable: the table sync runs through `&self`). Starts
    /// `u64::MAX` so the first `sync_blas_addr` call always rewrites.
    blas_addr_epoch: core::cell::Cell<u64>,
}

#[cfg(feature = "hwrt")]
impl TlasResources {
    /// Builds the packer pipeline + the FIF-ringed mesh-id / instance-array SSBOs + the
    /// persistent per-slot TLASes + the pack bind groups + the frame-invariant BLAS-address
    /// table, sized to `capacity` instances. `instance_rings` is the SHARED gbuffer set-0
    /// instance ring the packer reads at @0 (the SAME ring the raster VS reads). `capacity` ≥ 1.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI/AS create failure — a setup-stage device
    /// failure by design (the `GpuSceneBundles::boot` contract, gated on `ray_query_enabled`).
    pub(super) fn create(
        device: &VulkanContext,
        instance_rings: &[BoundBuffer; FRAMES_IN_FLIGHT],
        capacity: u32,
    ) -> Self {
        debug_assert!(capacity >= 1, "invariant: the TLAS pack needs at least one instance slot");
        let cs = RhiDevice::create_shader_module(device, build_tlas_instances_spirv())
            .expect("invariant: R2a-3 TLAS packer compute shader module create");
        let layout = RhiDevice::create_bind_group_layout(
            device,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                    BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
                ],
            },
        )
        .expect("invariant: R2a-3 pack bind-group layout create");
        let pipeline = RhiDevice::create_compute_pipeline(
            device,
            &ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: BUILD_TLAS_INSTANCES_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            },
        )
        .expect("invariant: R2a-3 pack compute pipeline create");

        // Each ring slot / the blas-addr table is a distinct host-coherent SSBO; the instance
        // arrays additionally carry the AS-build-input + device-address usage the TLAS build needs.
        let mesh_id_bytes = capacity as u64 * 4;
        let instance_array_bytes = capacity as u64 * TLAS_INSTANCE_BYTES as u64;
        let blas_addr_bytes = MESH_ADDR_CAP as u64 * 8;
        // HOST-VISIBLE storage buffers: the CPU writes these (the mesh-id lane via
        // `upload_mesh_ids`, the BLAS-address table via `sync_blas_addr`). Zero-seeded because a
        // fresh sub-allocation carries prior bytes and a first frame binds the ring before its
        // first host write.
        let make_host_storage = |size: u64, usage: BufferUsage, what: &str| {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc { size, usage, location: MemoryLocation::HostVisibleCoherent },
            )
            .unwrap_or_else(|e| panic!("invariant: R2a-3 {what} SSBO create: {e:?}"));
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .unwrap_or_else(|| panic!("invariant: host-visible R2a-3 {what} SSBO is mapped"));
            zero_fill(mapped, size as usize);
            b
        };
        let mesh_id_rings: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            make_host_storage(mesh_id_bytes, BufferUsage::STORAGE, "mesh-id")
        });
        // The instance arrays are GPU-ONLY (written by the pack compute, read only by the AS
        // build — never CPU-touched) → DEVICE-LOCAL VRAM (avoids streaming up to 64 KB/frame over
        // BAR/PCIe). NOT mappable ⇒ NO zero-seed: the plan's seed is undefined (the pack fully
        // overwrites `[0..count)` each frame; the build reads only `[0..count)`), so none is
        // needed. The device-local block carries the DEVICE_ADDRESS alloc flag under hwrt, so the
        // cached instance-array address (below) resolves non-zero.
        let instance_arrays: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: instance_array_bytes,
                    usage: BufferUsage::STORAGE
                        | BufferUsage::ACCEL_BUILD_INPUT
                        | BufferUsage::SHADER_DEVICE_ADDRESS,
                    location: MemoryLocation::DeviceLocal,
                },
            )
            .unwrap_or_else(|e| panic!("invariant: R2a-3 instance-array SSBO create: {e:?}"))
        });
        let blas_addr = make_host_storage(blas_addr_bytes, BufferUsage::STORAGE, "blas-addr");

        // The cached instance-array device addresses (the per-frame build's instance-array address).
        let instance_array_addr: [u64; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            let addr = buffer_device_address(device, &instance_arrays[fi])
                .expect("invariant: R2a-3 instance-array device address");
            debug_assert!(addr != 0, "invariant: instance-array has a non-zero device address");
            addr
        });

        // The persistent per-slot TLASes (backing + scratch sized ONCE for the MAX capacity).
        let tlas: [PersistentTlas; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            create_persistent_tlas(device, capacity)
                .expect("invariant: R2a-3 persistent TLAS create")
        });

        // The pack bind groups: { instance_rings[fi] @0, mesh_id_rings[fi] @1, blas_addr @2,
        // instance_arrays[fi] @3 }. @0 is the SHARED instance ring (the SAME buffer the raster VS
        // reads); the packer reads it after the interp compute wrote the dynamic slots.
        let bind_groups: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(
                device,
                &BindGroupDesc {
                    layout: &layout,
                    entries: &[
                        BindGroupEntry::StorageBuffer { buffer: &instance_rings[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &mesh_id_rings[fi] },
                        BindGroupEntry::StorageBuffer { buffer: &blas_addr },
                        BindGroupEntry::StorageBuffer { buffer: &instance_arrays[fi] },
                    ],
                },
            )
            .expect("invariant: R2a-3 pack bind group create")
        });

        // The shader module is consumed by pipeline creation; destroy it now.
        // SAFETY: `cs` was created on `device` above and is no longer needed once the pipeline
        // exists; it is destroyed exactly once; no GPU work has been submitted yet (boot stage).
        unsafe {
            RhiDevice::destroy_shader_module(device, cs);
        }

        Self {
            pipeline,
            layout,
            mesh_id_rings,
            instance_arrays,
            tlas,
            bind_groups,
            blas_addr,
            instance_array_addr,
            capacity: [capacity; FRAMES_IN_FLIGHT],
            blas_addr_epoch: core::cell::Cell::new(u64::MAX),
        }
    }

    /// The FENCED slot's mesh-id SSBO — the write target of the runner's per-frame
    /// [`upload_mesh_ids`](boyko_render::upload_mesh_ids) (the packer reads the same slot at
    /// binding 1). The sibling in-flight frame binds the OTHER slot.
    #[inline]
    pub(super) fn mesh_id_slot(&self, slot: usize) -> &BoundBuffer {
        &self.mesh_id_rings[slot]
    }

    /// Rewrites the frame-invariant `blas_addr` table from `mesh_assets` IFF its
    /// [`install_epoch`](boyko_ecs::ecs::core::asset::Assets::install_epoch) advanced since the
    /// last sync. A plain host-coherent memcpy of the per-mesh BLAS device addresses (RISK-3): no
    /// staging, no barrier — the submit's host-write → device domain dependency covers the packer's
    /// COMPUTE read visibility.
    ///
    /// # Why `install_epoch`, not `blas_generation`/`high_water` (asset-streaming plan F6 FIX-1)
    ///
    /// A retire+reuse cycle (F6: `Assets::retire` frees a slot, a later `Assets::add` reuses it
    /// via the free-list) installs a BRAND-NEW mesh's BLAS at the SAME slot index WITHOUT growing
    /// `high_water`/`blas_generation` (`take_at` does not pop the column; `add`'s reuse path
    /// overwrites the hole in place) — gating on either would silently skip the rewrite and leave
    /// `blas_addr[slot]` pointing at the FREED BLAS's device address (a GPU use-after-free the
    /// instant a drawable references the reused slot). `install_epoch` advances on EVERY
    /// `Assets::add`/`fill` call, including a free-list reuse, so it is the one signal that always
    /// catches this case; it also strictly subsumes `blas_generation` (a fresh append is an `add`
    /// too), so no second counter is needed.
    ///
    /// # Why `high_water()`, not `len()`, bounds the rewrite loop (a latent hole bug this closes)
    ///
    /// `blas_addr[m]` is addressed by `m`'s ABSOLUTE slot index (mirrors
    /// [`MaterialTable`](boyko_render::MaterialTable)'s identical W1 fix), not its rank among live
    /// meshes. Bounding the loop by [`Assets::len`](boyko_ecs::ecs::core::asset::Assets::len) (the
    /// LIVE count) under-covers the table the instant a hole exists below a still-live mesh's
    /// index — that live mesh's slot would silently fall outside `0..len()` and never be
    /// refreshed. [`Assets::high_water`](boyko_ecs::ecs::core::asset::Assets::high_water) is the
    /// slot-row high-water mark (every index ever minted, holes included), so every live or
    /// freed-but-not-yet-reused slot is unconditionally covered; a `Vacant` hole resolves to
    /// `blas_address() == 0` (a safe, non-dereferenceable sentinel — [`MeshAssetsExt::blas_address`](boyko_render::MeshAssetsExt::blas_address)'s
    /// `map_or(0, ..)`).
    ///
    /// Runs through `&self` (interior-mutable `blas_addr_epoch`) so the host can call it right
    /// before `scene()` without a `&mut` borrow of the bundles. Harmless to call more often than
    /// strictly necessary: the rewrite is idempotent (same `mesh_assets` state ⇒ same bytes), so
    /// this never perturbs the golden (never-retires) scene's rendered output.
    pub(super) fn sync_blas_addr(&self, device: &VulkanContext, mesh_assets: &Assets<MeshGpu>) {
        let epoch = mesh_assets.install_epoch();
        if self.blas_addr_epoch.get() == epoch {
            return;
        }
        let mesh_high = mesh_assets.high_water();
        // HARD assert (not `debug_assert`): a silent `.min()` clamp would leave the shader reading
        // `BlasAddr[mesh_id]` past the written region (garbage `accelerationStructureReference` →
        // bogus TLAS / device-lost) for a scene with > MESH_ADDR_CAP mesh SLOTS (holes included).
        // Fail fast in every build, matching `upload_mesh_ids`'s ring-overflow assert.
        assert!(
            mesh_high <= MESH_ADDR_CAP,
            "BLAS-address table overflow: {mesh_high} mesh slots exceed the {MESH_ADDR_CAP}-slot \
             table (grow MESH_ADDR_CAP)"
        );
        let mapped = RhiDevice::buffer_mapped_ptr(device, &self.blas_addr)
            .expect("invariant: host-visible BLAS-address table is mapped");
        for m in 0..mesh_high {
            let addr = mesh_assets.blas_address(MeshHandle(m as u32));
            // SAFETY: `mapped` targets `MESH_ADDR_CAP * 8` host-coherent bytes; `m < mesh_high <=
            // MESH_ADDR_CAP` (hard-asserted above), so the 8-byte write at `m * 8` is in-bounds.
            // `addr` is a plain `u64` (any bit pattern valid); the packer reads it as `uint2` (lo,
            // hi) — LE-consistent on x86_64. The write happens at setup / on a registration event,
            // never concurrent with a GPU read of this frame-invariant table (the submit domain
            // dependency orders it).
            unsafe {
                let dst = mapped.as_ptr().add(m * 8).cast::<u64>();
                core::ptr::write_unaligned(dst, addr);
            }
        }
        self.blas_addr_epoch.set(epoch);
    }

    /// The [`TlasBuildActivation`] for this frame slot `fi` and this frame's drawable `count`:
    /// the pack pipeline + this slot's pack bind group, this slot's persistent TLAS (the build
    /// target), this slot's compute-written instance array (the sink's tlas slot) + its cached
    /// address, this slot's scratch address, and `count`.
    ///
    /// `count` is THIS frame's total drawable count (the gather's `instance_count()`), which the
    /// mesh-id upload already bounds against `capacity` and the caller asserts `<= capacity`.
    #[inline]
    pub(super) fn activation(&self, fi: usize, count: u32) -> TlasBuildActivation<'_> {
        debug_assert!(
            count <= self.capacity[fi],
            "invariant: the per-frame drawable count fits slot fi's built TLAS capacity"
        );
        TlasBuildActivation {
            pipeline: &self.pipeline,
            bind_group: &self.bind_groups[fi],
            dest: &self.tlas[fi].accel,
            instance_array: &self.instance_arrays[fi],
            instance_array_addr: self.instance_array_addr[fi],
            scratch_addr: self.tlas[fi].scratch_addr,
            count,
        }
    }

    /// Asset-streaming plan F7-hwrt (task#11): grows slot `s`'s TLAS-side buffers
    /// (`mesh_id_rings[s]`, `instance_arrays[s]`) + rebuilds slot `s`'s [`PersistentTlas`]
    /// to `new_cap` instances, in lockstep with the caller's ALREADY-grown
    /// `instance_rings_s` (the SAME buffer the caller just repointed
    /// `instance_bind_groups[s]`@0 against) — reallocating (not resizing) each buffer,
    /// deferring the superseded one through `retired` at `retire_frame`. Repoints the
    /// pack bind group `bind_groups[s]` (@0/@1/@3 — @2 `blas_addr` is frame-invariant,
    /// untouched) and surfaces the new AS via the existing [`Self::resolve_accels`] (no
    /// separate wiring needed there: it always reads `self.tlas[i].accel` fresh). The
    /// CALLER is responsible for driving [`GBufferFrame::repoint_tlas_accel`]
    /// (the resolve-family AS rebind) once `targets_ready()` — this fn only rebuilds the
    /// pack-side wiring the PACK/BUILD passes read fresh every frame via
    /// [`Self::activation`].
    ///
    /// # No seed
    ///
    /// `mesh_id_rings[s]`: the runner's `upload_mesh_ids` rewrites the whole mesh-id lane
    /// THIS frame. `instance_arrays[s]`: the pack compute fully overwrites `[0..count)`
    /// every frame (mirrors [`Self::create`]'s own no-seed reasoning for both buffers).
    ///
    /// # Panics
    ///
    /// Panics (`expect`) on any RHI/AS create failure — a device OOM on a post-boot grow
    /// is a setup-adjacent failure, not a recoverable per-frame error (mirrors
    /// [`Self::create`]'s boot-time panics).
    ///
    /// # Safety
    ///
    /// The caller guarantees slot `s`'s in-flight fence was waited THIS frame (the
    /// `FrameWriteToken` proof [`GpuSceneBundles::grow_instance_family_rt`](super::GpuSceneBundles::grow_instance_family_rt)
    /// holds) — `bind_groups[s]`'s descriptor set is therefore non-command-buffer-pending,
    /// so rewriting its bindings in place is sound. The NEW `tlas[s].accel` is created but
    /// not yet built (no submission references it), so replacing the old one needs no
    /// fence proof of its own; only the OLD `accel`'s PRIOR uses matter, which
    /// `RetiredGpuBuffers::push_tlas`'s doc proves safe at `retire_frame`.
    #[cfg(feature = "hwrt")]
    pub(super) unsafe fn grow_slot(
        &mut self,
        device: &VulkanContext,
        s: usize,
        new_cap: u32,
        instance_rings_s: &BoundBuffer,
        retired: &mut RetiredGpuBuffers,
        retire_frame: u64,
    ) {
        let mesh_id_bytes = new_cap as u64 * 4;
        let new_mesh_ids = {
            let b = RhiDevice::create_buffer(
                device,
                &BufferDesc {
                    size: mesh_id_bytes,
                    usage: BufferUsage::STORAGE,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: grown R2a-3 mesh-id SSBO create");
            let mapped = RhiDevice::buffer_mapped_ptr(device, &b)
                .expect("invariant: host-visible grown R2a-3 mesh-id SSBO is mapped");
            zero_fill(mapped, mesh_id_bytes as usize);
            b
        };
        let old_mesh_ids = core::mem::replace(&mut self.mesh_id_rings[s], new_mesh_ids);
        retired.push(old_mesh_ids, retire_frame);

        // GPU-ONLY (never CPU-touched) → DEVICE-LOCAL VRAM, no seed (mirrors `create`'s
        // own instance-array reasoning): the pack compute overwrites `[0..count)` every
        // frame; the build reads only `[0..count)`.
        let instance_array_bytes = new_cap as u64 * TLAS_INSTANCE_BYTES as u64;
        let new_instance_array = RhiDevice::create_buffer(
            device,
            &BufferDesc {
                size: instance_array_bytes,
                usage: BufferUsage::STORAGE
                    | BufferUsage::ACCEL_BUILD_INPUT
                    | BufferUsage::SHADER_DEVICE_ADDRESS,
                location: MemoryLocation::DeviceLocal,
            },
        )
        .expect("invariant: grown R2a-3 instance-array SSBO create");
        let old_instance_array = core::mem::replace(&mut self.instance_arrays[s], new_instance_array);
        retired.push(old_instance_array, retire_frame);

        self.instance_array_addr[s] = buffer_device_address(device, &self.instance_arrays[s])
            .expect("invariant: grown R2a-3 instance-array device address");
        debug_assert!(
            self.instance_array_addr[s] != 0,
            "invariant: the grown instance-array has a non-zero device address"
        );

        let new_tlas =
            create_persistent_tlas(device, new_cap).expect("invariant: grown R2a-3 persistent TLAS create");
        let old_tlas = core::mem::replace(&mut self.tlas[s], new_tlas);
        retired.push_tlas(old_tlas, retire_frame);

        const PACK_GROWN_BINDINGS: usize = 3;
        let mut rebound = 0usize;
        // SAFETY: slot `s`'s fence was waited this frame (this fn's caller contract
        // above) — `bind_groups[s]`'s descriptor set is therefore non-pending, so
        // rewriting its bindings in place is sound. @2 (`blas_addr`) is frame-invariant
        // and untouched.
        unsafe {
            rebind_storage_buffer(device, &self.bind_groups[s], 0, instance_rings_s);
            rebound += 1;
            rebind_storage_buffer(device, &self.bind_groups[s], 1, &self.mesh_id_rings[s]);
            rebound += 1;
            rebind_storage_buffer(device, &self.bind_groups[s], 3, &self.instance_arrays[s]);
            rebound += 1;
        }
        debug_assert_eq!(
            rebound, PACK_GROWN_BINDINGS,
            "invariant: exactly 3 pack bindings rebound (@0 instance ring / @1 mesh-id / \
             @3 instance array; @2 blas_addr untouched)"
        );

        self.capacity[s] = new_cap;
    }

    /// R2a-4b: the per-FIF persistent TLAS handles — the frame-stable `rayQuery` trace targets the
    /// HWRT resolve set binds at binding 19. Slot `i` is `tlas[i].accel` (built into every frame,
    /// never recreated), so the once-per-FIF resolve-set write holds.
    #[inline]
    pub(super) fn resolve_accels(&self) -> [&BoundAccelStruct; FRAMES_IN_FLIGHT] {
        core::array::from_fn(|i| &self.tlas[i].accel)
    }

    /// Tears every owned resource down in reverse dependency order (bind_groups → TLASes
    /// (AS before backing, P0-3) → instance arrays → mesh-id rings → blas_addr → pipeline →
    /// layout). The SHARED instance rings are owned by `GpuSceneBundles`, not this struct.
    ///
    /// # Safety
    /// The device is idle (the caller's renderer drop waited) so no submission references these;
    /// each is destroyed exactly once (by-value `self`); `device` is the live context.
    pub(super) unsafe fn destroy(self, device: &VulkanContext) {
        // SAFETY: per the contract the device is idle + live; reverse creation order. Each TLAS
        // is freed AS-before-backing by `destroy_persistent_tlas` (the AS's memory lives in its
        // backing, which must outlive it).
        unsafe {
            for bg in self.bind_groups {
                RhiDevice::destroy_bind_group(device, bg);
            }
            for t in self.tlas {
                destroy_persistent_tlas(device, t);
            }
            for b in self.instance_arrays {
                RhiDevice::destroy_buffer(device, b);
            }
            for b in self.mesh_id_rings {
                RhiDevice::destroy_buffer(device, b);
            }
            RhiDevice::destroy_buffer(device, self.blas_addr);
            RhiDevice::destroy_compute_pipeline(device, self.pipeline);
            RhiDevice::destroy_bind_group_layout(device, self.layout);
        }
    }
}
