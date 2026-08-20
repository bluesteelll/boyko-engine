//! [`ParticleGpuBundle`] — the host-owned GPU resources of the particle system
//! (`docs/PARTICLES-PLAN.md` Rev 4, §Host-side). Owned by [`super::GpuSceneBundles`], built ONLY
//! when [`ParticleConfig::enabled`] holds at boot.
//!
//! # Structural absence, and what it costs when disarmed
//!
//! `mode == Off` ⇒ this module allocates nothing, compiles no shader module, mints no descriptor
//! set and reaches no device queue. The bundle is an `Option` on the scene bundles; the frame's
//! `GBufferScene::particle` is `None`; the three declarators declare no `ResId` and no pass; the
//! recorders emit no command. There is no dark tax to measure because there is no dark code path
//! — the particle work lives in its OWN passes and its OWN pipelines, never as a gated branch
//! inside someone else's shader.
//!
//! # The binding table is a BUILD-TIME contract, not a convention
//!
//! [`PARTICLE_LAYOUT_ENTRIES`] is the host mirror of the `(set, binding, kind)` tables written in
//! each shader's header. HLSL's bare `register(tN/uN)` shares one binding-number space across
//! resource classes, so a silent kind/index mismatch is expressible; the shaders therefore use
//! explicit `[[vk::binding(N, S)]]` and this table's `const fn` assert makes a drift a BUILD
//! error rather than a descriptor that reads the wrong buffer at 60 Hz.
//!
//! # The boot fill, and why exactly one buffer is not zero
//!
//! `p_dead` is boot-initialised to the IDENTITY permutation `p_dead[i] = i` with
//! `dead_count = CAP` — the only non-zero boot fill in the subsystem, and what makes the plan's
//! four-boundary equality `N_prev + D == CAP` true at frame 0 with `N_prev == 0`. Every other
//! device buffer is zero-filled. Both happen through ONE fence-waited boot submit (the
//! `CsmSceneResources::seed_boot_layouts` idiom), so no per-frame path pays for them and no
//! device buffer is ever read before it was written.

use boyko_rhi::enums::BlendState;
use boyko_rhi::{BarrierDesc, BufferBarrier, BufferCopy};
use boyko_rhi_vulkan::compute::{
    PARTICLE_DRAW_PUSH_BYTES, PARTICLE_EMIT_PUSH_BYTES, PARTICLE_KICKOFF_PUSH_BYTES,
    PARTICLE_QUAD_IB_BYTES, PARTICLE_SIM_PUSH_BYTES, particle_draw_dlin_fs_spirv,
    particle_draw_dlin_vs_spirv, particle_draw_fs_spirv, particle_draw_vs_spirv,
    particle_emit_spirv, particle_kickoff_spirv, particle_sim_spirv,
};
use boyko_rhi_vulkan::ffi::{VK_COMPARE_OP_GREATER, VK_COMPARE_OP_LESS, VkDescriptorSet};
use boyko_rhi_vulkan::swapchain::ParticleActivation;
use boyko_render::{
    EffectParamsGpu, EmitRequestGpu, MAX_EFFECTS, MAX_EMITTERS, PARTICLE_QUAD_INDEX_COUNT,
    ParticleCounters, ParticleDispatchArgs, ParticleDrawArgs, ParticleRender, ParticleSim,
};

use super::*;

// ── The binding table (D12's build-time contract) ────────────────────────────────────

/// One row of [`PARTICLE_LAYOUT_ENTRIES`]: `(set, binding, kind)`, mirroring the
/// `# Set / binding vocabulary` block in each particle shader's header.
///
/// `kind` is a [`DescriptorKind`], whose `#[repr(i32)]` discriminant is the `VkDescriptorType`
/// value — so the `const fn` check below can compare kinds without a `PartialEq` call a `const`
/// context cannot make.
#[derive(Clone, Copy)]
pub(crate) struct ParticleLayoutEntry {
    /// The descriptor SET this binding lives in (`0` for the compute vocabulary and for the
    /// draw's vertex-stage set; `1` for the draw's bindless texture set, which this table does
    /// NOT own — see [`PARTICLE_LAYOUT_ENTRIES`]'s doc).
    pub(crate) set: u32,
    /// The binding number inside that set.
    pub(crate) binding: u32,
    /// The descriptor type the shader declares at that binding.
    pub(crate) kind: DescriptorKind,
}

/// The COMPUTE Set-0 vocabulary, bindings 0..9 — the union of what
/// `particle_{kickoff,emit,sim}.comp.hlsl` declare. Each module names only the subset it uses and
/// DXC strips the rest; the host layout is the union, and a set that binds more than a module
/// declares is legal (an unreferenced descriptor is simply never read).
///
/// ⚠️ The DRAW's two sets are deliberately NOT in this table. Set 0 of the draw is a DIFFERENT
/// vocabulary over the same set number (`p_render` @0 + the camera cbuffer @1, both `VERTEX`),
/// and set 1 is the SHARED bindless texture set this subsystem does not own. Folding either into
/// one flat table would make the well-formedness check below meaningless: it asserts that no two
/// rows share a `(set, binding)`, which is exactly the property two independent set-0 layouts do
/// not have.
pub(crate) const PARTICLE_LAYOUT_ENTRIES: [ParticleLayoutEntry; 10] = [
    // `p_counters` — kickoff read+write, emit read, sim read+write (atomic).
    ParticleLayoutEntry { set: 0, binding: 0, kind: DescriptorKind::StorageBuffer },
    // `p_dispatch_args` — kickoff write; read by `DRAW_INDIRECT`, never by a shader.
    ParticleLayoutEntry { set: 0, binding: 1, kind: DescriptorKind::StorageBuffer },
    // `p_draw_args` — kickoff write, sim read+write (the returning `InterlockedAdd`).
    ParticleLayoutEntry { set: 0, binding: 2, kind: DescriptorKind::StorageBuffer },
    // `p_dead` — emit read, sim write.
    ParticleLayoutEntry { set: 0, binding: 3, kind: DescriptorKind::StorageBuffer },
    // `p_alive_read` — emit write, sim read. Bound to `alive[parity]`.
    ParticleLayoutEntry { set: 0, binding: 4, kind: DescriptorKind::StorageBuffer },
    // `p_alive_write` — sim write only. Bound to `alive[parity ^ 1]`.
    ParticleLayoutEntry { set: 0, binding: 5, kind: DescriptorKind::StorageBuffer },
    // `p_particle` — emit write (init), sim read+write (step).
    ParticleLayoutEntry { set: 0, binding: 6, kind: DescriptorKind::StorageBuffer },
    // `p_render` — sim write.
    ParticleLayoutEntry { set: 0, binding: 7, kind: DescriptorKind::StorageBuffer },
    // `p_emit_req` — emit read (a `StructuredBuffer`, still a STORAGE_BUFFER descriptor).
    ParticleLayoutEntry { set: 0, binding: 8, kind: DescriptorKind::StorageBuffer },
    // `p_effects` — emit read, sim read.
    ParticleLayoutEntry { set: 0, binding: 9, kind: DescriptorKind::StorageBuffer },
];

/// The DRAW's Set-0 vocabulary — a different table over the same set number (see
/// [`PARTICLE_LAYOUT_ENTRIES`]'s warning). Both entries are `VERTEX`-stage: the fragment half
/// reads set 1 only.
pub(crate) const PARTICLE_DRAW_LAYOUT_ENTRIES: [ParticleLayoutEntry; 2] = [
    // `StructuredBuffer<ParticleRender> p_render` @0.
    ParticleLayoutEntry { set: 0, binding: 0, kind: DescriptorKind::StorageBuffer },
    // The 80-byte camera/extent cbuffer @1 — the SAME shape every other consumer declares. The VS
    // reads `cam_right`/`cam_up` (the billboard basis), and under `-D DEPTH_LINEAR` also
    // `cam_eye`/`camera_mode` for the Deferred depth encode. Reading MORE of an already-bound
    // block adds no binding, which is why both shader pairs share this one table.
    ParticleLayoutEntry { set: 0, binding: 1, kind: DescriptorKind::UniformBuffer },
];

/// `true` iff `entries` is a well-formed layout table: bindings strictly ascending from 0 with no
/// gap and no repeat, every row in the same set, and every row a real descriptor kind.
///
/// A `const fn` so the assert below runs at BUILD time. "Strictly ascending with no gap" is the
/// strong form on purpose: it makes the table's INDEX equal its binding number, which is what
/// lets the descriptor-write array below be built positionally and lets a reader check a shader
/// header against this file by reading straight down.
const fn layout_table_is_well_formed(entries: &[ParticleLayoutEntry], set: u32) -> bool {
    let mut i = 0;
    while i < entries.len() {
        if entries[i].set != set {
            return false;
        }
        if entries[i].binding != i as u32 {
            return false;
        }
        // `as i32` reads the `#[repr(i32)]` discriminant (== the `VkDescriptorType` value); a
        // `PartialEq` call is not available in a `const fn`.
        let kind = entries[i].kind as i32;
        if kind != DescriptorKind::StorageBuffer as i32
            && kind != DescriptorKind::UniformBuffer as i32
        {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(
    layout_table_is_well_formed(&PARTICLE_LAYOUT_ENTRIES, 0),
    "the particle COMPUTE Set-0 table must be bindings 0..9 in order, all in set 0"
);
const _: () = assert!(
    layout_table_is_well_formed(&PARTICLE_DRAW_LAYOUT_ENTRIES, 0),
    "the particle DRAW Set-0 table must be bindings 0..1 in order, all in set 0"
);
// The compute vocabulary is exactly the ten seed-table rows — the same count the declarators
// append and the same count each sink reserves. A drift here and a drift there would otherwise
// have to be noticed by a human.
const _: () = assert!(PARTICLE_LAYOUT_ENTRIES.len() == 10);

// ── Boot sizing ──────────────────────────────────────────────────────────────────────

/// The staging chunk the boot zero-fill copies through — 1 MiB.
///
/// The bulk buffers (`p_particle` at 48 B/particle, `p_render` at 32 B) are far larger than this
/// at a realistic capacity, so they are zeroed by repeating this ONE zero-filled staging
/// allocation across disjoint destination chunks. Sizing the staging to the largest destination
/// instead would cost a transient ~12 MB host allocation at boot to save a couple of dozen
/// `vkCmdCopyBuffer` regions in a submit that happens once.
const ZERO_STAGING_BYTES: u64 = 1 << 20;

/// Byte size of the emit-request device table — the plan's ≤ 16 KB per-frame host→device budget,
/// and the size of each staging ring slot.
const EMIT_REQ_TABLE_BYTES: u64 = (MAX_EMITTERS * size_of::<EmitRequestGpu>()) as u64;

/// Byte size of the effect-parameter device table (32 KB — effectively constant-resident in the
/// sim's cache), and the size of each staging ring slot.
const EFFECT_TABLE_BYTES: u64 = (MAX_EFFECTS * size_of::<EffectParamsGpu>()) as u64;

// ── The bundle ───────────────────────────────────────────────────────────────────────

/// Every GPU resource the particle system owns. Built at boot when armed, destroyed in
/// [`super::GpuSceneBundles::destroy`]'s particle arm.
///
/// Field order is DECLARATION order (the seed table / Set-0 binding numbering), and
/// [`Self::destroy`] tears down in reverse dependency order.
pub(crate) struct ParticleGpuBundle {
    /// Seed row 6 — the 64-byte bookkeeping cache line.
    counters: BoundBuffer,
    /// Seed row 7 — the two `VkDispatchIndirectCommand`s.
    dispatch_args: BoundBuffer,
    /// Seed row 8 — the two `VkDrawIndexedIndirectCommand`s.
    draw_args: BoundBuffer,
    /// Seed row 3 — the free list, boot-filled with the identity permutation.
    dead: BoundBuffer,
    /// Seed rows 4/5 — the two PHYSICAL alive lists. Their ROLES swap every frame; the host owns
    /// the parity and bakes the roles into [`Self::sets`], so there is no device-side parity word
    /// and no parity arithmetic anywhere on the GPU.
    alive: [BoundBuffer; 2],
    /// Seed row 1 — the 48-byte sim records.
    particle: BoundBuffer,
    /// Seed row 2 — the 32-byte render records.
    render: BoundBuffer,
    /// Seed row 9 — the device-side emit-request table.
    emit_req_device: BoundBuffer,
    /// Seed row 10 — the device-side effect-parameter table.
    effects_device: BoundBuffer,
    /// The billboard quad's 12-byte `u16` index buffer. NOT a framegraph resource: written once
    /// at boot under its own `TRANSFER_WRITE → INDEX_READ` barrier and read-only forever.
    quad_ib: BoundBuffer,
    /// Per-in-flight-slot host-visible staging for the emit-request table. A RING, not one
    /// instance: frame N's recorded staging→device copy READS this buffer while it executes, so a
    /// single instance rewritten on frame N+1 would race it.
    emit_req_staging: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// Per-in-flight-slot host-visible staging for the effect table. Same ring rationale.
    effects_staging: [BoundBuffer; FRAMES_IN_FLIGHT],
    /// The shared COMPUTE Set-0 layout ([`PARTICLE_LAYOUT_ENTRIES`]).
    compute_layout: VulkanBindGroupLayout,
    /// The two PARITY sets over [`Self::compute_layout`]. `sets[0]` binds `alive[0]` at binding 4
    /// (`p_alive_read`) and `alive[1]` at binding 5 (`p_alive_write`); `sets[1]` swaps them. The
    /// host picks the set from its own frame counter — which is the whole parity mechanism.
    sets: [VulkanBindGroup; 2],
    /// The one-thread bookkeeping pipeline.
    kickoff: ComputePipeline,
    /// The spawn pipeline.
    emit: ComputePipeline,
    /// The hot-loop pipeline.
    sim: ComputePipeline,
    /// The draw's Set-0 layout ([`PARTICLE_DRAW_LAYOUT_ENTRIES`]).
    draw_layout0: VulkanBindGroupLayout,
    /// Per-in-flight-slot draw Set-0 groups — a RING because binding 1 is the camera UBO ring
    /// slot, which is per-frame. Binding 0 (`p_render`) is the single shared buffer in every slot.
    draw_set0: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// The additive billboard pipeline. Its depth compare op was frozen HERE, at boot, from the
    /// resolved render path, so exactly one `VkPipeline` exists per process.
    draw_pipeline: VulkanGraphicsPipeline,
    /// The SHARED bindless texture set bound at set 1 — owned by `BindlessTextureTable`, borrowed
    /// as a raw handle exactly as `GBufferScene::bindless_set` does. NOT destroyed here.
    bindless_set: VkDescriptorSet,
    /// The boot-frozen pool capacity, in particles (plan D14: bounds MEMORY only — per-frame work
    /// is `O(alive)`). Carried so the activation can push it without the runner re-reading the
    /// config, which would give the number a second home.
    capacity: u32,
}

impl ParticleGpuBundle {
    /// Builds every particle resource and performs the ONE fence-waited boot submit that fills
    /// them (`p_dead` = identity permutation with `dead_count = CAP`; everything else zeroed).
    ///
    /// `camera_ring` is the per-in-flight-slot camera UBO ring the draw's Set-0 binding 1 reads;
    /// `bindless` owns the set-1 texture table.
    ///
    /// # `deferred_path` decides BOTH halves of the depth contract, and that is why it is one
    /// argument
    ///
    /// The compare op ([`particle_depth_compare_for`]) and the draw's shader pair
    /// ([`particle_draw_spirv_for`]) are two answers to ONE question — what does this path's depth
    /// image hold — so the boot resolution crosses this boundary as the single predicate that
    /// produced them, never as two pre-derived values that could disagree. Deferred gets
    /// `VK_COMPARE_OP_LESS` and the `-D DEPTH_LINEAR` pair (its depth is the G-buffer fragment's
    /// euclidean encode); the three reverse-Z paths get `VK_COMPARE_OP_GREATER` and the base pair.
    ///
    /// # Panics
    ///
    /// Panics (`expect("invariant: ...")`) on any RHI create/map/submit failure — a setup-stage
    /// device failure by design (the `GpuSceneBundles::boot` contract).
    pub(crate) fn create(
        ctx: &VulkanContext,
        camera_ring: &[BoundBuffer; FRAMES_IN_FLIGHT],
        bindless: &BindlessTextureTable,
        capacity: u32,
        deferred_path: bool,
    ) -> Self {
        debug_assert!(capacity >= 1, "invariant: the particle pool needs at least one slot");
        let device = ctx;
        let cap = u64::from(capacity);

        // ── The nine device-local buffers + the index buffer. `create_buffer` already ORs both
        //    TRANSFER bits into every DeviceLocal allocation, so the explicit spellings below are
        //    DECLARATIVE (they state which of them the boot fill and the per-frame copies rely
        //    on) and change no created usage.
        let device_buffer = |size: u64, usage: BufferUsage, what: &str| {
            RhiDevice::create_buffer(device, &BufferDesc {
                size,
                usage,
                location: MemoryLocation::DeviceLocal,
            })
            .unwrap_or_else(|e| panic!("invariant: particle {what} buffer create: {e:?}"))
        };
        let storage = BufferUsage::STORAGE | BufferUsage::TRANSFER_DST;
        // The two indirect blocks are STORAGE too: kickoff WRITES them through a descriptor, and
        // the sim atomically accumulates into `p_draw_args` — the INDIRECT bit only covers the
        // command processor's fetch.
        let indirect = BufferUsage::STORAGE | BufferUsage::INDIRECT | BufferUsage::TRANSFER_DST;

        let counters = device_buffer(size_of::<ParticleCounters>() as u64, storage, "counters");
        let dispatch_args =
            device_buffer(size_of::<ParticleDispatchArgs>() as u64, indirect, "dispatch_args");
        let draw_args = device_buffer(size_of::<ParticleDrawArgs>() as u64, indirect, "draw_args");
        let dead = device_buffer(cap * 4, storage, "dead");
        let alive: [BoundBuffer; 2] =
            core::array::from_fn(|_| device_buffer(cap * 4, storage, "alive"));
        let particle =
            device_buffer(cap * size_of::<ParticleSim>() as u64, storage, "particle records");
        let render =
            device_buffer(cap * size_of::<ParticleRender>() as u64, storage, "render records");
        let emit_req_device = device_buffer(EMIT_REQ_TABLE_BYTES, storage, "emit-request table");
        let effects_device = device_buffer(EFFECT_TABLE_BYTES, storage, "effect table");
        let quad_ib = device_buffer(
            PARTICLE_QUAD_IB_BYTES,
            BufferUsage::INDEX | BufferUsage::TRANSFER_DST,
            "quad index",
        );

        // ── The two host-visible staging RINGS. Host-coherent so the per-frame writes need no
        //    explicit flush; zero-seeded because a fresh sub-allocation carries prior bytes and
        //    the first frame's copy may run before the first host write of a slot.
        let staging_ring = |size: u64, what: &'static str| -> [BoundBuffer; FRAMES_IN_FLIGHT] {
            core::array::from_fn(|_| {
                let b = RhiDevice::create_buffer(device, &BufferDesc {
                    size,
                    usage: BufferUsage::TRANSFER_SRC,
                    location: MemoryLocation::HostVisibleCoherent,
                })
                .unwrap_or_else(|e| panic!("invariant: particle {what} staging create: {e:?}"));
                let mapped = RhiDevice::buffer_mapped_ptr(device, &b).unwrap_or_else(|| {
                    panic!("invariant: host-visible particle {what} staging is mapped")
                });
                zero_fill(mapped, size as usize);
                b
            })
        };
        let emit_req_staging = staging_ring(EMIT_REQ_TABLE_BYTES, "emit-request");
        let effects_staging = staging_ring(EFFECT_TABLE_BYTES, "effect");

        // ── The boot fill. ONE encoder, ONE submit, ONE fence wait.
        boot_fill(
            ctx,
            capacity,
            BootFillTargets {
                counters: &counters,
                dispatch_args: &dispatch_args,
                draw_args: &draw_args,
                dead: &dead,
                alive: &alive,
                particle: &particle,
                render: &render,
                emit_req_device: &emit_req_device,
                effects_device: &effects_device,
                quad_ib: &quad_ib,
            },
        );

        // ── The COMPUTE Set-0 layout + the two PARITY sets.
        let compute_entries: [BindGroupLayoutEntry; PARTICLE_LAYOUT_ENTRIES.len()] =
            core::array::from_fn(|i| BindGroupLayoutEntry {
                binding: PARTICLE_LAYOUT_ENTRIES[i].binding,
                count: 1,
                kind: PARTICLE_LAYOUT_ENTRIES[i].kind,
                stage: ShaderStage::COMPUTE,
            });
        let compute_layout = RhiDevice::create_bind_group_layout(device, &BindGroupLayoutDesc {
            entries: &compute_entries,
        })
        .expect("invariant: particle compute Set-0 bind-group layout create");

        // The ONE place the alive-list ROLES are decided. `sets[p]` binds `alive[p]` as
        // `p_alive_read` (@4) and `alive[p ^ 1]` as `p_alive_write` (@5); the declarator seeds
        // those two ResIds with OPPOSITE constructors on exactly that assumption, so swapping the
        // two lines below would leave one cross-frame hazard unordered every frame while every
        // barrier COUNT stayed identical — invisible on a static scene.
        let sets: [VulkanBindGroup; 2] = core::array::from_fn(|p| {
            RhiDevice::create_bind_group(device, &BindGroupDesc {
                layout: &compute_layout,
                entries: &[
                    BindGroupEntry::StorageBuffer { buffer: &counters },
                    BindGroupEntry::StorageBuffer { buffer: &dispatch_args },
                    BindGroupEntry::StorageBuffer { buffer: &draw_args },
                    BindGroupEntry::StorageBuffer { buffer: &dead },
                    BindGroupEntry::StorageBuffer { buffer: &alive[p] },
                    BindGroupEntry::StorageBuffer { buffer: &alive[p ^ 1] },
                    BindGroupEntry::StorageBuffer { buffer: &particle },
                    BindGroupEntry::StorageBuffer { buffer: &render },
                    BindGroupEntry::StorageBuffer { buffer: &emit_req_device },
                    BindGroupEntry::StorageBuffer { buffer: &effects_device },
                ],
            })
            .expect("invariant: particle parity bind group create")
        });

        // ── The three compute pipelines. Each gets its OWN pipeline layout (D12's "dedicated
        //    layouts"), which is what keeps `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` — the SHARED
        //    range, at 112 of a 128-byte guaranteed floor with 16 bytes of headroom — from
        //    having to grow for these three 8-byte blocks.
        let compute_pipeline = |spirv: &'static [u32], push: u32, what: &str| {
            let module = RhiDevice::create_shader_module(device, spirv)
                .unwrap_or_else(|e| panic!("invariant: particle {what} shader module: {e:?}"));
            let pipeline = RhiDevice::create_compute_pipeline(device, &ComputePipelineDesc {
                module: &module,
                entry: c"main",
                push_constant_bytes: push,
                bind_group_layout: Some(&compute_layout),
                spec_constants: &[],
            })
            .unwrap_or_else(|e| panic!("invariant: particle {what} compute pipeline: {e:?}"));
            // SAFETY: `module` was created on `device` just above and is no longer needed once
            // the pipeline owns the compiled state; it is destroyed exactly once; no GPU work
            // referencing it has been submitted (boot stage, and the boot fill above submitted
            // only transfers).
            unsafe {
                RhiDevice::destroy_shader_module(device, module);
            }
            pipeline
        };
        let kickoff =
            compute_pipeline(particle_kickoff_spirv(), PARTICLE_KICKOFF_PUSH_BYTES, "kickoff");
        let emit = compute_pipeline(particle_emit_spirv(), PARTICLE_EMIT_PUSH_BYTES, "emit");
        let sim = compute_pipeline(particle_sim_spirv(), PARTICLE_SIM_PUSH_BYTES, "sim");

        // ── The draw's Set-0 layout + its per-slot groups + the graphics pipeline.
        let draw_entries: [BindGroupLayoutEntry; PARTICLE_DRAW_LAYOUT_ENTRIES.len()] =
            core::array::from_fn(|i| BindGroupLayoutEntry {
                binding: PARTICLE_DRAW_LAYOUT_ENTRIES[i].binding,
                count: 1,
                kind: PARTICLE_DRAW_LAYOUT_ENTRIES[i].kind,
                stage: ShaderStage::VERTEX,
            });
        let draw_layout0 = RhiDevice::create_bind_group_layout(device, &BindGroupLayoutDesc {
            entries: &draw_entries,
        })
        .expect("invariant: particle draw Set-0 bind-group layout create");
        let draw_set0: [VulkanBindGroup; FRAMES_IN_FLIGHT] = core::array::from_fn(|fi| {
            RhiDevice::create_bind_group(device, &BindGroupDesc {
                layout: &draw_layout0,
                entries: &[
                    BindGroupEntry::StorageBuffer { buffer: &render },
                    BindGroupEntry::UniformBuffer { buffer: &camera_ring[fi] },
                ],
            })
            .expect("invariant: particle draw Set-0 bind group create")
        });

        // The path's depth contract, resolved from the ONE predicate: which encode the depth image
        // holds decides the compare op AND which of the two interface-identical shader pairs the
        // draw is built from.
        let (draw_vs_spirv, draw_fs_spirv) = particle_draw_spirv_for(deferred_path);
        let depth_compare = particle_depth_compare_for(deferred_path);
        let draw_vs = RhiDevice::create_shader_module(device, draw_vs_spirv)
            .expect("invariant: particle draw vertex shader module create");
        let draw_fs = RhiDevice::create_shader_module(device, draw_fs_spirv)
            .expect("invariant: particle draw fragment shader module create");
        let draw_pipeline = ctx
            .create_graphics_pipeline_particle(
                &GraphicsPipelineDesc {
                    vertex_module: &draw_vs,
                    vertex_entry: c"main",
                    fragment_module: &draw_fs,
                    fragment_entry: c"main",
                    // Composited into `lit` (`R8G8B8A8_UNORM`), depth-tested against the path's
                    // own `D32_SFLOAT`. NO vertex layout: the four corners are generated from
                    // `SV_VertexID` and the six indices come from `quad_ib`.
                    color_formats: &[RASTER_COLOR_FORMAT],
                    depth_format: Some(Format::D32Sfloat),
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: None,
                    push_constant_bytes: PARTICLE_DRAW_PUSH_BYTES,
                    bind_group_layout: Some(&draw_layout0),
                    // The blend is PIPELINE state, never shader code — and it is the additive one
                    // precisely because additive is COMMUTATIVE, which is what lets P0 ship
                    // unsorted with a proof rather than a hope.
                    blend: Some(BlendState::ADDITIVE),
                    // A billboard quad is two triangles facing the camera; culling either winding
                    // would drop half of them depending on the rotation the sim stored.
                    cull_mode: CullMode::None,
                    depth_bias: None,
                },
                bindless.set().set_layout(),
                depth_compare,
            )
            .expect("invariant: particle draw graphics pipeline create");
        // SAFETY: both modules were created on `device` above and are consumed by the pipeline
        // create; each is destroyed exactly once; no GPU work referencing them is in flight.
        unsafe {
            RhiDevice::destroy_shader_module(device, draw_fs);
            RhiDevice::destroy_shader_module(device, draw_vs);
        }

        Self {
            counters,
            dispatch_args,
            draw_args,
            dead,
            alive,
            particle,
            render,
            emit_req_device,
            effects_device,
            quad_ib,
            emit_req_staging,
            effects_staging,
            compute_layout,
            sets,
            kickoff,
            emit,
            sim,
            draw_layout0,
            draw_set0,
            draw_pipeline,
            bindless_set: bindless.set().set(),
            capacity,
        }
    }

    /// This frame slot's emit-request staging buffer — the destination of
    /// `boyko_render::upload_particle_emit_requests`.
    #[inline]
    pub(crate) fn emit_req_staging_slot(&self, slot: usize) -> &BoundBuffer {
        &self.emit_req_staging[slot]
    }

    /// This frame slot's effect-table staging buffer — the destination of
    /// `boyko_render::upload_particle_effects`.
    #[inline]
    pub(crate) fn effects_staging_slot(&self, slot: usize) -> &BoundBuffer {
        &self.effects_staging[slot]
    }

    /// Assembles this frame's [`ParticleActivation`].
    ///
    /// `fi` is the frame-in-flight SLOT (it selects the camera-UBO-backed draw set and the two
    /// staging slots); `parity` is the HOST frame counter's low bit (it selects the alive-list
    /// role assignment). They are the same number in a double-buffered engine and are still
    /// passed separately, because they answer different questions and a future ring depth would
    /// make them differ.
    ///
    /// `emit_upload_bytes` / `effects_upload_bytes` are the caller's gate results: `0` means the
    /// declarator declares no copy for that half and the recorder emits none.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn activation<'a>(
        &'a self,
        fi: usize,
        parity: u32,
        push: ParticleFramePush,
        emit_upload_bytes: u64,
        effects_upload_bytes: u64,
    ) -> ParticleActivation<'a> {
        debug_assert!(parity < 2, "invariant: parity is the frame counter's low bit");
        debug_assert!(
            emit_upload_bytes == 0 || push.requested_spawn > 0,
            "invariant: the emit-request upload and the emit pass share ONE predicate — an \
             upload with no requested spawn would make row 9's reader seed wrong"
        );
        ParticleActivation {
            kickoff_pipeline: &self.kickoff,
            emit_pipeline: &self.emit,
            sim_pipeline: &self.sim,
            draw_pipeline: &self.draw_pipeline,
            sets: &self.sets[parity as usize],
            draw_set0: &self.draw_set0[fi],
            draw_set1: self.bindless_set,
            counters: &self.counters,
            dispatch_args: &self.dispatch_args,
            draw_args: &self.draw_args,
            dead: &self.dead,
            alive_read: &self.alive[parity as usize],
            alive_write: &self.alive[(parity ^ 1) as usize],
            particle_records: &self.particle,
            render_records: &self.render,
            emit_req_device: &self.emit_req_device,
            effects_device: &self.effects_device,
            emit_req_staging: &self.emit_req_staging[fi],
            emit_upload_bytes,
            effects_staging: &self.effects_staging[fi],
            effects_upload_bytes,
            quad_ib: &self.quad_ib,
            requested_spawn: push.requested_spawn,
            emitter_count: push.emitter_count,
            capacity: self.capacity,
            steps: push.steps,
            timestep: push.timestep,
            frame_index: push.frame_index,
            parity,
            draw_push: push.draw_push,
        }
    }

    /// Plan gate #7/#9 (cold, once per run): copies `p_counters` and `p_draw_args` back to the
    /// host and decodes the pool partition.
    ///
    /// # Why an out-of-band submit rather than a framegraph pass
    ///
    /// The sibling probes (`vb_cull_readback`, the HZB dump) copy inside the frame's recorded
    /// command buffer and DRAIN, because they run while the loop continues and must not stall it.
    /// This one is the last thing a run does — the caller ends the frame loop immediately after —
    /// so it takes the simpler and stricter route: idle the device, then one fenced
    /// transfer submit. That buys three things a graph pass would have cost real work to get: no
    /// `ResId` is added to any declarator (so no armed/disarmed barrier-stream baseline moves), no
    /// per-FIF staging ring is needed (nothing else is in flight), and the values read are
    /// unambiguously the ones the LAST submitted frame's `particle_sim` left behind.
    ///
    /// The barrier is still explicit and not implied by the idle: `vkDeviceWaitIdle` orders
    /// EXECUTION, and the compute writes must additionally be made AVAILABLE to a transfer read.
    ///
    /// # Panics
    ///
    /// Panics (`expect("invariant: ...")`) on any RHI failure — this runs on a diagnostic path
    /// that ends the process, where a silent `None` would read exactly like a scene that produced
    /// nothing.
    pub(crate) fn read_counters(&self, ctx: &VulkanContext) -> ParticleCountersRaw {
        const COUNTERS_BYTES: u64 = size_of::<ParticleCounters>() as u64;
        const DRAW_ARGS_BYTES: u64 = size_of::<ParticleDrawArgs>() as u64;
        const TOTAL: u64 = COUNTERS_BYTES + DRAW_ARGS_BYTES;

        let device = ctx;
        RhiDevice::wait_idle(device).expect("invariant: particle readback device idle");

        let staging = RhiDevice::create_buffer(device, &BufferDesc {
            size: TOTAL,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("invariant: particle readback staging create");
        let mapped = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("invariant: host-visible particle readback staging is mapped");

        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: particle readback command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: particle readback fence create");
        encoder.begin().expect("invariant: particle readback encoder begin");
        // Availability: the last frame's `particle_sim`/`particle_kickoff` wrote both blocks
        // through a STORAGE descriptor, and this submit reads them as a transfer source. The
        // device idle above already ordered the execution; this makes the writes visible.
        encoder.pipeline_barrier(&BarrierDesc {
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::TRANSFER,
            buffers: &[
                BufferBarrier {
                    buffer: &self.counters,
                    src_access: BarrierAccess::SHADER_WRITE,
                    dst_access: BarrierAccess::TRANSFER_READ,
                },
                BufferBarrier {
                    buffer: &self.draw_args,
                    src_access: BarrierAccess::SHADER_WRITE,
                    dst_access: BarrierAccess::TRANSFER_READ,
                },
            ],
        });
        encoder.copy_buffer(&self.counters, &staging, &[BufferCopy {
            src_offset: 0,
            dst_offset: 0,
            size: COUNTERS_BYTES,
        }]);
        encoder.copy_buffer(&self.draw_args, &staging, &[BufferCopy {
            src_offset: 0,
            dst_offset: COUNTERS_BYTES,
            size: DRAW_ARGS_BYTES,
        }]);
        encoder.end().expect("invariant: particle readback encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: particle readback submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: particle readback fence wait");

        let mut counters = ParticleCounters::default();
        let mut draw_args = ParticleDrawArgs::default();
        // SAFETY: `mapped` addresses `TOTAL` valid mapped host-coherent bytes of a buffer this fn
        // just created; the fence wait above completed the ONLY submission that writes them, so
        // the transfer's results are complete and (the memory being HOST_COHERENT) host-visible.
        // Both destinations are `#[repr(C)]` + `Pod` — every byte pattern of their exact size is a
        // valid value — and the two source ranges `[0, COUNTERS_BYTES)` and
        // `[COUNTERS_BYTES, TOTAL)` are in bounds, disjoint, and exactly the two struct sizes.
        // The destinations are fresh locals, so nothing aliases them.
        unsafe {
            core::ptr::copy_nonoverlapping(
                mapped.as_ptr(),
                (&raw mut counters).cast::<u8>(),
                COUNTERS_BYTES as usize,
            );
            core::ptr::copy_nonoverlapping(
                mapped.as_ptr().add(COUNTERS_BYTES as usize),
                (&raw mut draw_args).cast::<u8>(),
                DRAW_ARGS_BYTES as usize,
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

        ParticleCountersRaw { counters, draw_args, capacity: self.capacity }
    }

    /// Tears every owned resource down in reverse dependency order. The bindless set-1 table and
    /// the camera UBO ring are owned elsewhere and are NOT destroyed here.
    ///
    /// # Safety
    ///
    /// The device is idle (the caller's renderer drop waited), so no submission references any of
    /// these; each is destroyed exactly once (the by-value `self` enforces it); `ctx` is the live
    /// context they were created on.
    pub(crate) unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract the device is idle and `ctx` is live; reverse creation order
        // (sets before their layout, pipelines before the layout their layouts embed, buffers
        // last).
        unsafe {
            RhiDevice::destroy_graphics_pipeline(ctx, self.draw_pipeline);
            for bg in self.draw_set0 {
                RhiDevice::destroy_bind_group(ctx, bg);
            }
            RhiDevice::destroy_bind_group_layout(ctx, self.draw_layout0);
            RhiDevice::destroy_compute_pipeline(ctx, self.sim);
            RhiDevice::destroy_compute_pipeline(ctx, self.emit);
            RhiDevice::destroy_compute_pipeline(ctx, self.kickoff);
            for bg in self.sets {
                RhiDevice::destroy_bind_group(ctx, bg);
            }
            RhiDevice::destroy_bind_group_layout(ctx, self.compute_layout);
            for b in self.effects_staging {
                RhiDevice::destroy_buffer(ctx, b);
            }
            for b in self.emit_req_staging {
                RhiDevice::destroy_buffer(ctx, b);
            }
            RhiDevice::destroy_buffer(ctx, self.quad_ib);
            RhiDevice::destroy_buffer(ctx, self.effects_device);
            RhiDevice::destroy_buffer(ctx, self.emit_req_device);
            RhiDevice::destroy_buffer(ctx, self.render);
            RhiDevice::destroy_buffer(ctx, self.particle);
            for b in self.alive {
                RhiDevice::destroy_buffer(ctx, b);
            }
            RhiDevice::destroy_buffer(ctx, self.dead);
            RhiDevice::destroy_buffer(ctx, self.draw_args);
            RhiDevice::destroy_buffer(ctx, self.dispatch_args);
            RhiDevice::destroy_buffer(ctx, self.counters);
        }
    }
}

/// The raw device blocks [`ParticleGpuBundle::read_counters`] brings back, plus the capacity they
/// must partition — the three values plan gate #7's arithmetic is stated over.
///
/// Deliberately the DEVICE structs and not a pre-digested summary: the decode into named
/// quantities and the partition predicates live in [`crate::particle_readback`], which is pure and
/// therefore testable without a GPU, and this type is the seam between the two halves.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParticleCountersRaw {
    /// The bookkeeping cache line as the last submitted frame's passes left it.
    pub(crate) counters: ParticleCounters,
    /// The two indirect draw commands; `additive.instance_count` is the sim's own render counter.
    pub(crate) draw_args: ParticleDrawArgs,
    /// The boot-frozen pool capacity the partition is checked against.
    pub(crate) capacity: u32,
}

/// The per-frame scalars the activation carries to the device, gathered into one struct so the
/// activation constructor does not take a dozen loose `u32`s a caller could transpose.
///
/// Every field has exactly ONE home on the CPU; see [`ParticleActivation`]'s doc for the map.
#[derive(Clone, Copy)]
pub(crate) struct ParticleFramePush {
    /// `ParticleEmitScratch::total_spawn()`.
    pub(crate) requested_spawn: u32,
    /// `ParticleEmitScratch::emitter_count()`.
    pub(crate) emitter_count: u32,
    /// `ParticleClock::steps()` — already ceiling-clamped on the host.
    pub(crate) steps: u32,
    /// `ParticleClock::timestep()`.
    pub(crate) timestep: f32,
    /// The runner's monotonic engine frame index.
    pub(crate) frame_index: u32,
    /// The draw's assembled 72-byte `VERTEX` push.
    pub(crate) draw_push: [u8; PARTICLE_DRAW_PUSH_BYTES as usize],
}

/// Everything the runner decides per frame and the scene assembler needs to build the frame's
/// [`ParticleActivation`]. `None` at the call site ⇒ no activation ⇒ structural absence.
///
/// It exists so the assembler takes ONE parameter instead of five, and so the two upload byte
/// counts travel BESIDE the predicate that produced them: `emit_upload_bytes` is non-zero exactly
/// when `push.requested_spawn` is (the conditional-pass proof's load-bearing detail), and the
/// activation constructor `debug_assert!`s that pairing rather than trusting the caller.
#[derive(Clone, Copy)]
pub(crate) struct ParticleFrameInputs {
    /// The host frame counter's low bit — selects the alive-list ROLE assignment.
    pub(crate) parity: u32,
    /// The frame's push values.
    pub(crate) push: ParticleFramePush,
    /// Bytes to copy staging→device for the emit-request table, or `0` for no copy.
    pub(crate) emit_upload_bytes: u64,
    /// Bytes to copy staging→device for the effect table, or `0` for no copy.
    pub(crate) effects_upload_bytes: u64,
}

/// The depth compare op the particle pipeline is frozen with for a resolved render path (D7).
///
/// `VK_COMPARE_OP_LESS` under Deferred, whose depth image holds a CUSTOM LINEAR value, and
/// `VK_COMPARE_OP_GREATER` everywhere else, where depth is hardware reverse-Z. Getting it wrong
/// INVERTS occlusion — particles behind walls draw in front of them — and no automated image gate
/// would catch it, which is why the plan's gate #12 pairs the boot-value assertion with an
/// owner-eval screenshot per path.
///
/// `mesh_leg`-independent by construction: the compare op is a property of what the depth BUFFER
/// contains, and both legs of every path agree on that.
#[inline]
pub(crate) const fn particle_depth_compare_for(deferred_path: bool) -> i32 {
    if deferred_path { VK_COMPARE_OP_LESS } else { VK_COMPARE_OP_GREATER }
}

/// The `(vertex, fragment)` SPIR-V the draw pipeline is frozen with for a resolved render path —
/// [`particle_depth_compare_for`]'s other half, off the SAME predicate.
///
/// **Deferred is the `-D DEPTH_LINEAR` pair.** That path's depth image is not hardware depth: the
/// G-buffer fragment overwrites it with `length(cam_eye - P) / MESH_DEPTH_T_MAX`, while the
/// projection this path hands the particle VS is the marcher's, whose `row2 == row3` pins every
/// billboard vertex to `SV_Position.z == 1.0`. Under `LESS` that fails on every pixel including
/// the cleared sky — MEASURED at P0, and not fixable by any host-side matrix (`z_ndc` is a ratio
/// of affine functions of the world position; a euclidean norm is not one). The variant's fragment
/// writes the depth image's OWN encode through `SV_Depth` instead, which costs early-Z ON THIS LEG
/// (see the shader header) and nothing anywhere else.
///
/// The three reverse-Z paths take the base pair unchanged: their depth image holds exactly the
/// projective `SV_Position.z` the base VS already emits.
///
/// Interface-identical by construction (same bindings, same push range), so ONE pipeline layout
/// serves either pair and the pick never reaches the descriptor plumbing.
#[inline]
pub(crate) fn particle_draw_spirv_for(deferred_path: bool) -> (&'static [u32], &'static [u32]) {
    if deferred_path {
        (particle_draw_dlin_vs_spirv(), particle_draw_dlin_fs_spirv())
    } else {
        (particle_draw_vs_spirv(), particle_draw_fs_spirv())
    }
}

/// The nine device buffers the boot fill writes, plus the index buffer. Grouped so
/// [`boot_fill`] takes one argument instead of ten.
struct BootFillTargets<'a> {
    counters: &'a BoundBuffer,
    dispatch_args: &'a BoundBuffer,
    draw_args: &'a BoundBuffer,
    dead: &'a BoundBuffer,
    alive: &'a [BoundBuffer; 2],
    particle: &'a BoundBuffer,
    render: &'a BoundBuffer,
    emit_req_device: &'a BoundBuffer,
    effects_device: &'a BoundBuffer,
    quad_ib: &'a BoundBuffer,
}

/// The ONE fence-waited boot submit that makes every particle buffer defined before frame 0.
///
/// # What it writes, and why each one has to be written
///
/// * `p_dead` ← the IDENTITY permutation `[0, 1, .., CAP-1]`, and `p_counters.dead_count` ← `CAP`.
///   This is the frame-0 contract: at boundary B0 the partition is `live = p_alive_read[0..0)`
///   and `free = p_dead[0..CAP)`, so `N_prev + D == CAP` holds with no live particles. Frame 0's
///   kickoff then reads `alive_count_next == 0`, clamps the spawn against `CAP`, and emit walks
///   exactly the slots it reserved — no leak, no stale read, and no dependence on any device-side
///   parity state.
/// * `p_counters`' other fields ← 0. Kickoff overwrites all of them, but `alive_count_next` is
///   the one it READS before writing, so it must be defined.
/// * The two indirect blocks ← 0. Kickoff rewrites both every frame before either is fetched;
///   zeroing them means an instrumented first frame reads zeros rather than whatever the
///   allocation carried.
/// * The two device tables ← 0. `robustBufferAccess` is OFF on this device, so a stale effect
///   index reading unwritten allocation contents is undefined behaviour rather than garbage
///   colour; zeroing them bounds that to "reads a zeroed row".
/// * `p_particle` / `p_render` / both alive lists ← 0. Every LIVE read of these is preceded by a
///   write within the same frame's counters, so zeroing is not load-bearing for the shipped
///   passes; it is done anyway because "defined at boot" is a property worth having when the next
///   rung adds a reader, and it costs one boot submit.
/// * `quad_ib` ← the six `u16` indices of two triangles, then a `TRANSFER_WRITE → INDEX_READ`
///   barrier. This buffer is NOT a framegraph resource (written once, read-only forever), so this
///   is the ONE hand-written barrier in the whole subsystem — and it must be here, at the write,
///   because nothing downstream would ever re-emit it.
///
/// # Panics
///
/// Panics on any RHI failure — a setup-stage failure by design.
fn boot_fill(ctx: &VulkanContext, capacity: u32, t: BootFillTargets<'_>) {
    let device = ctx;
    let cap = u64::from(capacity);

    // ── Staging 1: the zero source, reused across chunked destination ranges. Sized to the
    //    LARGEST destination, capped at [`ZERO_STAGING_BYTES`] — the `min` alone would collapse
    //    to a few dozen bytes at a tiny capacity and turn the 32 KB effect table into hundreds of
    //    copy regions, and the `max` alone would allocate ~12 MB at the default one.
    let largest_zero_target = (cap * size_of::<ParticleSim>() as u64)
        .max(cap * size_of::<ParticleRender>() as u64)
        .max(EFFECT_TABLE_BYTES)
        .max(EMIT_REQ_TABLE_BYTES);
    let zero_bytes = ZERO_STAGING_BYTES.min(largest_zero_target);
    debug_assert!(zero_bytes > 0, "invariant: a zero-length staging would not terminate the fill");
    let zero_staging = RhiDevice::create_buffer(device, &BufferDesc {
        size: zero_bytes,
        usage: BufferUsage::TRANSFER_SRC,
        location: MemoryLocation::HostVisibleCoherent,
    })
    .expect("invariant: particle boot zero staging create");
    let zero_mapped = RhiDevice::buffer_mapped_ptr(device, &zero_staging)
        .expect("invariant: host-visible particle boot zero staging is mapped");
    zero_fill(zero_mapped, zero_bytes as usize);

    // ── Staging 2: the identity permutation for `p_dead`.
    let dead_staging = RhiDevice::create_buffer(device, &BufferDesc {
        size: cap * 4,
        usage: BufferUsage::TRANSFER_SRC,
        location: MemoryLocation::HostVisibleCoherent,
    })
    .expect("invariant: particle boot dead-list staging create");
    let dead_mapped = RhiDevice::buffer_mapped_ptr(device, &dead_staging)
        .expect("invariant: host-visible particle boot dead-list staging is mapped");
    // SAFETY: `dead_mapped` targets `cap * 4` valid mapped host-coherent bytes of a buffer this
    // fn just created (the map succeeded above), it is 4-byte aligned (the RHI allocates buffers
    // at >= 16-byte alignment), and nothing else aliases it — the allocation is local to this fn
    // and no GPU work has been submitted against it yet. `capacity` elements is exactly the
    // allocation.
    let dead_words = unsafe {
        core::slice::from_raw_parts_mut(dead_mapped.as_ptr().cast::<u32>(), capacity as usize)
    };
    for (i, slot) in dead_words.iter_mut().enumerate() {
        *slot = i as u32;
    }

    // ── Staging 3: the counters' boot value + the quad's six indices, in one small allocation.
    //    The counters go at offset 0 and the indices at `size_of::<ParticleCounters>()`, so each
    //    copy names its own disjoint source range.
    const QUAD_SRC_OFFSET: u64 = size_of::<ParticleCounters>() as u64;
    let ctrl_bytes = QUAD_SRC_OFFSET + PARTICLE_QUAD_IB_BYTES;
    let ctrl_staging = RhiDevice::create_buffer(device, &BufferDesc {
        size: ctrl_bytes,
        usage: BufferUsage::TRANSFER_SRC,
        location: MemoryLocation::HostVisibleCoherent,
    })
    .expect("invariant: particle boot control staging create");
    let ctrl_mapped = RhiDevice::buffer_mapped_ptr(device, &ctrl_staging)
        .expect("invariant: host-visible particle boot control staging is mapped");
    zero_fill(ctrl_mapped, ctrl_bytes as usize);
    // The frame-0 contract, written through the REAL struct rather than as typed word indices —
    // a layout drift then moves both halves together or fails the build.
    let counters = ParticleCounters { dead_count: capacity, ..ParticleCounters::default() };
    // Two triangles over the four corners `SV_VertexID` generates: (0,1,2) and (0,2,3).
    const QUAD_INDICES: [u16; PARTICLE_QUAD_INDEX_COUNT as usize] = [0, 1, 2, 0, 2, 3];
    // SAFETY: `ctrl_mapped` targets `ctrl_bytes` valid mapped host-coherent bytes of a buffer
    // this fn just created; `size_of::<ParticleCounters>() + PARTICLE_QUAD_IB_BYTES` IS
    // `ctrl_bytes`, so both writes are in bounds and non-overlapping. Reading `counters` as bytes
    // is sound because `ParticleCounters` is `#[repr(C)]` and `Pod` (its own crate asserts both,
    // and `Pod` is exactly the promise that every byte of the value — padding included, of which
    // it has none — is initialized). `[u16; 6]` is likewise plain data. Nothing else aliases the
    // mapping and no GPU work has been submitted against it.
    unsafe {
        core::ptr::copy_nonoverlapping(
            (&raw const counters).cast::<u8>(),
            ctrl_mapped.as_ptr(),
            size_of::<ParticleCounters>(),
        );
        core::ptr::copy_nonoverlapping(
            QUAD_INDICES.as_ptr().cast::<u8>(),
            ctrl_mapped.as_ptr().add(QUAD_SRC_OFFSET as usize),
            PARTICLE_QUAD_IB_BYTES as usize,
        );
    }

    // ── Record + submit.
    let mut encoder = RhiDevice::create_command_encoder(device)
        .expect("invariant: particle boot-fill command encoder create");
    let fence =
        RhiDevice::create_fence(device, false).expect("invariant: particle boot-fill fence create");
    encoder.begin().expect("invariant: particle boot-fill encoder begin");

    // The zeroed destinations, in declaration order. `p_counters` is deliberately absent: the
    // control copy below writes all 64 of its bytes, so zeroing it first would be a copy whose
    // every byte is immediately overwritten.
    let zero_targets: [(&BoundBuffer, u64); 8] = [
        (t.dispatch_args, size_of::<ParticleDispatchArgs>() as u64),
        (t.draw_args, size_of::<ParticleDrawArgs>() as u64),
        (&t.alive[0], cap * 4),
        (&t.alive[1], cap * 4),
        (t.particle, cap * size_of::<ParticleSim>() as u64),
        (t.render, cap * size_of::<ParticleRender>() as u64),
        (t.emit_req_device, EMIT_REQ_TABLE_BYTES),
        (t.effects_device, EFFECT_TABLE_BYTES),
    ];
    for (dst, total) in zero_targets {
        let mut written = 0u64;
        while written < total {
            let size = (total - written).min(zero_bytes);
            encoder.copy_buffer(&zero_staging, dst, &[BufferCopy {
                src_offset: 0,
                dst_offset: written,
                size,
            }]);
            written += size;
        }
    }

    // `p_dead` ← the identity permutation, and `p_counters` ← `{ dead_count: CAP, .. }`.
    encoder.copy_buffer(&dead_staging, t.dead, &[BufferCopy {
        src_offset: 0,
        dst_offset: 0,
        size: cap * 4,
    }]);
    encoder.copy_buffer(&ctrl_staging, t.counters, &[BufferCopy {
        src_offset: 0,
        dst_offset: 0,
        size: size_of::<ParticleCounters>() as u64,
    }]);
    encoder.copy_buffer(&ctrl_staging, t.quad_ib, &[BufferCopy {
        src_offset: QUAD_SRC_OFFSET,
        dst_offset: 0,
        size: PARTICLE_QUAD_IB_BYTES,
    }]);

    // The ONE hand-written barrier in the subsystem — see this fn's doc. Every other particle
    // buffer's ordering is DERIVED by the framegraph, because every other particle buffer is a
    // framegraph resource; this one is not, so its transfer→index-fetch hand-off has to be stated
    // here, exactly once, beside the write it orders.
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::VERTEX_INPUT,
        buffers: &[BufferBarrier {
            buffer: t.quad_ib,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::INDEX_READ,
        }],
    });

    encoder.end().expect("invariant: particle boot-fill encoder end");
    device
        .rhi_queue()
        .submit(&encoder, &fence)
        .expect("invariant: particle boot-fill submit");
    RhiDevice::wait_fence(device, &fence, u64::MAX)
        .expect("invariant: particle boot-fill fence wait");

    // SAFETY: `encoder` and `fence` were created on `device` above; the encoder's ONLY submission
    // completed (the fence wait just returned), so no GPU work references either of them or any
    // of the three staging buffers; each is moved by value ⇒ destroyed exactly once. Boot stage:
    // no other submission is in flight.
    unsafe {
        RhiDevice::destroy_command_encoder(device, encoder);
        RhiDevice::destroy_fence(device, fence);
        RhiDevice::destroy_buffer(device, ctrl_staging);
        RhiDevice::destroy_buffer(device, dead_staging);
        RhiDevice::destroy_buffer(device, zero_staging);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `OpExecutionMode`'s opcode (SPIR-V 1.x core, section 3.32.6).
    const OP_EXECUTION_MODE: u32 = 16;
    /// The `DepthReplacing` execution mode — "this fragment stage writes `FragDepth`"
    /// (SPIR-V 1.x, section 3.6). A stage that declares it CANNOT be early-Z tested; a stage that
    /// does not, can.
    const EXEC_MODE_DEPTH_REPLACING: u32 = 12;
    /// The SPIR-V module header, in words (magic, version, generator, bound, schema).
    const SPIRV_HEADER_WORDS: usize = 5;

    /// Whether `spirv` declares [`EXEC_MODE_DEPTH_REPLACING`].
    ///
    /// A real instruction walk, not a word scan: every instruction's first word packs
    /// `(word_count << 16) | opcode`, so stepping by `word_count` from the header is exact, and the
    /// mode operand is the instruction's THIRD word (`%entry_point`, then the mode). A bare
    /// "contains the value 12 somewhere" search would match any constant, id or literal in the
    /// module and could never fail honestly.
    fn declares_depth_replacing(spirv: &[u32]) -> bool {
        let mut i = SPIRV_HEADER_WORDS;
        while i < spirv.len() {
            let word_count = (spirv[i] >> 16) as usize;
            let opcode = spirv[i] & 0xFFFF;
            assert!(word_count >= 1, "malformed SPIR-V: zero-length instruction at word {i}");
            if opcode == OP_EXECUTION_MODE
                && word_count >= 3
                && spirv[i + 2] == EXEC_MODE_DEPTH_REPLACING
            {
                return true;
            }
            i += word_count;
        }
        false
    }

    /// Slice IDENTITY — same address and same length, i.e. the same `static` blob, not merely
    /// equal bytes.
    fn is_the_same_blob(a: &'static [u32], b: &'static [u32]) -> bool {
        core::ptr::eq(a.as_ptr(), b.as_ptr()) && a.len() == b.len()
    }

    /// **Plan gate #12's compare-op half, and the depth contract's other half with it.**
    ///
    /// # What this closes
    ///
    /// Both selectors are `pub(crate)` with exactly one production caller, and the ONLY thing that
    /// noticed a swapped arm before this test was a GPU with a window and `BOYKO_HOST_DUMP` set:
    /// the 25 shader pins are all statements about shader TEXT and BYTES, and every one of them
    /// stays green while the host hands the wrong pair to `create`. That is a gate that cannot
    /// fail on the defect it exists for.
    ///
    /// # What it asserts, and why identity is not enough on its own
    ///
    /// Three claims per leg. The IDENTITY claim (this leg gets that accessor's blob) would still
    /// pass if the `embed_spirv!` for the variant had been pointed back at the base `.spv`, so the
    /// PROPERTY claim is asserted too: the Deferred leg's fragment declares `DepthReplacing` and
    /// the reverse-Z leg's does not. That is the actual difference the render depends on, read out
    /// of the committed artifact rather than trusted from a file name.
    #[test]
    fn the_deferred_leg_takes_the_depth_linear_pair_and_the_less_compare_op() {
        let (vs, fs) = particle_draw_spirv_for(true);
        assert!(
            is_the_same_blob(vs, particle_draw_dlin_vs_spirv()),
            "the Deferred leg must be built from the -D DEPTH_LINEAR VERTEX module: its two extra \
             interpolants are what the fragment's depth encode reads"
        );
        assert!(
            is_the_same_blob(fs, particle_draw_dlin_fs_spirv()),
            "the Deferred leg must be built from the -D DEPTH_LINEAR FRAGMENT module: that path's \
             depth image holds `length(cam_eye - P) / MESH_DEPTH_T_MAX`, and the base fragment \
             writes no depth at all, so every fragment would fail VK_COMPARE_OP_LESS against the \
             VS's pinned SV_Position.z == 1.0 — the P0 live-fire erratum, verbatim"
        );
        assert!(
            declares_depth_replacing(fs),
            "the Deferred fragment module must declare OpExecutionMode DepthReplacing — the \
             accessor may name the variant while the artifact behind it is the base compile"
        );
        assert_eq!(
            particle_depth_compare_for(true),
            VK_COMPARE_OP_LESS,
            "Deferred's depth image increases with distance (a linear encode), so nearer is LESS"
        );
    }

    /// The reverse-Z legs' row of the same table — see the Deferred test for what these claims
    /// are for.
    #[test]
    fn the_reverse_z_legs_take_the_base_pair_and_the_greater_compare_op() {
        let (vs, fs) = particle_draw_spirv_for(false);
        assert!(
            is_the_same_blob(vs, particle_draw_vs_spirv()),
            "Forward / ForwardPlus / VisibilityBuffer must take the BASE vertex module"
        );
        assert!(
            is_the_same_blob(fs, particle_draw_fs_spirv()),
            "Forward / ForwardPlus / VisibilityBuffer must take the BASE fragment module"
        );
        assert!(
            !declares_depth_replacing(fs),
            "the base fragment must NOT declare DepthReplacing: those three paths hold hardware \
             reverse-Z depth, the VS's own SV_Position.z is already the right value, and writing \
             SV_Depth there would cost them early-Z for nothing"
        );
        assert_eq!(
            particle_depth_compare_for(false),
            VK_COMPARE_OP_GREATER,
            "reverse-Z depth decreases with distance, so nearer is GREATER"
        );
    }

    /// The non-vacuity control: the two legs really are two different pipelines.
    ///
    /// Without this, a build in which both accessors resolved to one artifact — or in which the
    /// two `.spv` happened to be copies — would satisfy every assertion above, and the selector
    /// would be a branch with one outcome.
    #[test]
    fn the_two_legs_are_distinct_artifacts_in_both_stages() {
        let (dlin_vs, dlin_fs) = particle_draw_spirv_for(true);
        let (base_vs, base_fs) = particle_draw_spirv_for(false);
        assert!(!is_the_same_blob(dlin_vs, base_vs), "the two vertex blobs must be distinct");
        assert!(!is_the_same_blob(dlin_fs, base_fs), "the two fragment blobs must be distinct");
        assert_ne!(
            dlin_vs, base_vs,
            "the two vertex modules must differ in CONTENT, not only in address"
        );
        assert_ne!(
            dlin_fs, base_fs,
            "the two fragment modules must differ in CONTENT, not only in address"
        );
        assert_ne!(
            particle_depth_compare_for(true),
            particle_depth_compare_for(false),
            "the two legs must not share a compare op"
        );
    }
}
