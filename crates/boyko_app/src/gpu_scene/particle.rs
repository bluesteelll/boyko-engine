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
    PARTICLE_QUAD_IB_BYTES, PARTICLE_SIM_PUSH_BYTES, PARTICLE_SORT_BINS_WORDS,
    PARTICLE_SORT_PUSH_BYTES, particle_draw_dlin_fs_spirv, particle_draw_dlin_vs_spirv,
    particle_draw_fs_spirv, particle_draw_vs_spirv, particle_emit_spirv, particle_kickoff_spirv,
    particle_sim_sdf_spirv, particle_sim_spirv, particle_sim_stats_spirv, particle_sort_hist_spirv,
    particle_sort_scan_spirv, particle_sort_scatter_spirv,
};
use boyko_rhi_vulkan::ffi::{VK_COMPARE_OP_GREATER, VK_COMPARE_OP_LESS, VkDescriptorSet};
use boyko_rhi_vulkan::swapchain::ParticleActivation;
use boyko_render::{
    EffectParamsGpu, EmitRequestGpu, MAX_EFFECTS, MAX_EMITTERS, PARTICLE_QUAD_INDEX_COUNT,
    ParticleCollision, ParticleCounters, ParticleDispatchArgs, ParticleDrawArgs, ParticleRender,
    ParticleSim, ParticleSortMode,
};

use crate::particle_readback::{
    PARTICLE_SORT_READBACK_MAX_RECORDS, ParticleSortRangeScan, ParticleSortReadback,
    scan_alpha_range,
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

/// The COMPUTE Set-0 vocabulary, bindings 0..12 — the union of what
/// `particle_{kickoff,emit,sim,sort_hist,sort_scan,sort_scatter}.comp.hlsl` declare. Each module
/// names only the subset it uses and DXC strips the rest; the host layout is the union, and a set
/// that binds more than a module declares is legal (an unreferenced descriptor is simply never
/// read).
///
/// Binding 10 is rung P1's SDF edit list, and it is the first row that is a union member without
/// being in EVERY armed frame's flow: the base `particle_sim` module does not declare it (the
/// `-D SDF_COLLIDE` block is invisible to DXC), so on a non-colliding run it is bound-but-unread —
/// the same shape the marcher's `tiles_buffer`/`PointerGrid` bindings already have. One layout
/// serves both sim variants, which is what keeps the pipeline pick from reaching the descriptor
/// plumbing.
///
/// Bindings 11 and 12 are rung P2 item 3's, and they are the SAME shape: on a
/// [`ParticleSortMode::None`](boyko_render::ParticleSortMode::None) run the two sort buffers are
/// never allocated, so those two slots are filled with a PLACEHOLDER (`p_render` and `p_counters`,
/// both already live) and no module that could read them is built. Binding a live buffer rather
/// than leaving the descriptor unwritten is deliberate: an unwritten descriptor in a bound set is
/// undefined behaviour on this device even when no shader reads it, whereas a bound-but-unread one
/// is explicitly legal. One layout therefore serves both armings, and — like the collision pick —
/// the sort pick never reaches the descriptor plumbing.
///
/// ⚠️ The DRAW's two sets are deliberately NOT in this table. Set 0 of the draw is a DIFFERENT
/// vocabulary over the same set number (`p_render` @0 + the camera cbuffer @1, both `VERTEX`),
/// and set 1 is the SHARED bindless texture set this subsystem does not own. Folding either into
/// one flat table would make the well-formedness check below meaningless: it asserts that no two
/// rows share a `(set, binding)`, which is exactly the property two independent set-0 layouts do
/// not have.
pub(crate) const PARTICLE_LAYOUT_ENTRIES: [ParticleLayoutEntry; 13] = [
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
    // `Buf` — the SDF edit list, read by the `-D SDF_COLLIDE` sim only (rung P1 / plan D9). The
    // SAME binding number every other field consumer in the tree uses
    // (`sdf_mesh_shadow.comp.hlsl:97`), and the engine's ONE edit list: boot-static, read-only for
    // the whole present loop, hence not a framegraph resource and not a seed-table row.
    ParticleLayoutEntry { set: 0, binding: 10, kind: DescriptorKind::StorageBuffer },
    // `p_render_sorted` — the sort SCATTER's destination and the ALPHA draw's source (rung P2 item
    // 3 / plan D10). Written by `particle_sort_scatter` only. On an unsorted run this slot holds
    // the `p_render` placeholder; see this table's doc.
    ParticleLayoutEntry { set: 0, binding: 11, kind: DescriptorKind::StorageBuffer },
    // `p_sort_bins` — the radix's 512-word scratch: the histogram half `[0, 256)` and the running
    // offsets half `[256, 512)`. Read+write (atomic) by the histogram and the scatter, read+write
    // by the scan. On an unsorted run this slot holds the `p_counters` placeholder.
    ParticleLayoutEntry { set: 0, binding: 12, kind: DescriptorKind::StorageBuffer },
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
    "the particle COMPUTE Set-0 table must be bindings 0..12 in order, all in set 0"
);
const _: () = assert!(
    layout_table_is_well_formed(&PARTICLE_DRAW_LAYOUT_ENTRIES, 0),
    "the particle DRAW Set-0 table must be bindings 0..1 in order, all in set 0"
);
// The compute vocabulary is the TWELVE seed-table rows — the same count the declarators append and
// the same count each sink reserves — PLUS rung P1's edit list, which is deliberately not one of
// them (boot-static, read-only, no `ResId`). Spelling the sum rather than the total is what keeps
// the two claims separable: a new GRAPH resource must move the declarators too, a new read-only
// resident one must not.
//
// Rung P2 item 3 moved the first term from 10 to 12 (`p_render_sorted`, `p_sort_bins`), and both of
// its additions ARE graph resources: each is written by one sort pass and read by another (or by
// the draw), so each carries a seed row and a derived barrier. The second term is unchanged.
const _: () = assert!(PARTICLE_LAYOUT_ENTRIES.len() == 12 + 1);

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
    /// Seed row 11 — rung P2 item 3's SORTED render records, `CAP × 32 B`. `None` on a
    /// [`ParticleSortMode::None`](boyko_render::ParticleSortMode::None) run: structural absence, so
    /// an unsorted arming allocates not one byte of it.
    ///
    /// Only the ALPHA sub-range `[CAP − alpha.instanceCount, CAP)` is ever written; the additive
    /// half of `p_render` is never copied, because that class needs no sort (D10/R5) and the
    /// additive draw keeps reading `p_render` directly.
    render_sorted: Option<BoundBuffer>,
    /// Seed row 12 — rung P2 item 3's 512-word radix scratch (the histogram half followed by the
    /// running-offsets half). `None` on an unsorted run, for [`Self::render_sorted`]'s reason.
    sort_bins: Option<BoundBuffer>,
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
    /// Rung P2 item 3's three sort pipelines — histogram, scan, scatter — or `None` on an unsorted
    /// run. They travel as ONE `Option` rather than three because they are one decision: a partial
    /// set is not a state the sort can be in, and three independent `Option`s would admit five
    /// combinations that mean nothing.
    sort: Option<ParticleSortPipelines>,
    /// The draw's Set-0 layout ([`PARTICLE_DRAW_LAYOUT_ENTRIES`]).
    draw_layout0: VulkanBindGroupLayout,
    /// Per-in-flight-slot draw Set-0 groups — a RING because binding 1 is the camera UBO ring
    /// slot, which is per-frame. Binding 0 (`p_render`) is the single shared buffer in every slot.
    draw_set0: [VulkanBindGroup; FRAMES_IN_FLIGHT],
    /// Rung P2 item 3: the ALPHA draw's own Set-0 ring, IDENTICAL to [`Self::draw_set0`] except
    /// that binding 0 names `p_render_sorted`. `None` on an unsorted run, where the alpha draw
    /// shares the base ring exactly as it did at rung P2 item 2.
    ///
    /// This is the ONLY thing the sort changes about the draw: the push pair stays
    /// `(capacity - 1, -1)` (the scatter wrote the class with the same mirror the sim used), the
    /// pipeline stays the same object, and the VS was not recompiled — so D10's "no shader variant"
    /// survives this rung untouched.
    draw_set0_alpha: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>,
    /// The additive billboard pipeline. Its depth compare op was frozen HERE, at boot, from the
    /// resolved render path, so exactly one `VkPipeline` exists per process.
    draw_pipeline: VulkanGraphicsPipeline,
    /// Rung P2: the ALPHA billboard pipeline — the same descriptor, the same two shader modules and
    /// the same boot-frozen depth compare op, differing ONLY in its `BlendState`.
    ///
    /// Two pipelines and not one, because blend factors are static pipeline state here (no
    /// `VK_EXT_extended_dynamic_state3`). Two `VkPipeline`s per process, both boot-frozen: D10's
    /// "no shader variant" holds — the two classes share one VS, one FS and one push layout.
    draw_pipeline_alpha: VulkanGraphicsPipeline,
    /// The SHARED bindless texture set bound at set 1 — owned by `BindlessTextureTable`, borrowed
    /// as a raw handle exactly as `GBufferScene::bindless_set` does. NOT destroyed here.
    bindless_set: VkDescriptorSet,
    /// The boot-frozen pool capacity, in particles (plan D14: bounds MEMORY only — per-frame work
    /// is `O(alive)`). Carried so the activation can push it without the runner re-reading the
    /// config, which would give the number a second home.
    capacity: u32,
    /// The boot-frozen collision arm this bundle's sim `VkPipeline` was built from.
    ///
    /// Carried for ONE reason, and it is a correctness reason rather than a diagnostic one: rung
    /// P1b's census is only well-defined at **one substep per dispatch**, and
    /// [`Self::activation`] is the site that sees both this arm and the frame's substep count. See
    /// the refusal there.
    collision: ParticleCollision,
    /// The boot-frozen sort arm (rung P2 item 3 / plan D10). Carried so the activation can state
    /// R10's rule and so a reader of this struct can tell whether the three `Option`s above are
    /// `Some` for a reason or by accident — [`Self::sort_mode`] asserts the agreement.
    sort_mode: ParticleSortMode,
}

/// Rung P2 item 3: the radix sort's three boot-frozen compute pipelines, in dispatch order.
///
/// Grouped rather than three loose fields on [`ParticleGpuBundle`] because they are ONE decision
/// (see that struct's `sort` field): the sort either exists with all three passes or does not
/// exist, and D10's "3 dispatches" is a property of the group.
pub(crate) struct ParticleSortPipelines {
    /// Dispatch 1 — the 256-bin histogram over the alpha class's keys.
    hist: ComputePipeline,
    /// Dispatch 2 — the ONE-GROUP exclusive scan that turns bin populations into offsets and
    /// re-zeroes the histogram half for the next frame.
    scan: ComputePipeline,
    /// Dispatch 3 — the permutation into `p_render_sorted`.
    scatter: ComputePipeline,
}

impl ParticleGpuBundle {
    /// Builds every particle resource and performs the ONE fence-waited boot submit that fills
    /// them (`p_dead` = identity permutation with `dead_count = CAP`; everything else zeroed).
    ///
    /// `camera_ring` is the per-in-flight-slot camera UBO ring the draw's Set-0 binding 1 reads;
    /// `bindless` owns the set-1 texture table; `edit_list` is the engine's ONE SDF edit list,
    /// bound at Set-0 binding 10 for rung P1's `-D SDF_COLLIDE` sim (and bound-but-unread under
    /// [`ParticleCollision::Off`] — see [`PARTICLE_LAYOUT_ENTRIES`]).
    ///
    /// # `collision` picks a MODULE, not a branch
    ///
    /// [`particle_sim_spirv_for`] resolves it once, here, into one of the THREE committed sim
    /// artifacts — base, rung P1's `-D SDF_COLLIDE`, or rung P1b's `-D SDF_COLLIDE_STATS`
    /// instrument. A runtime flag inside one shader would pay the field-consumer's register
    /// pressure and its `#include`d code (and, for the instrument, its atomics) on every disarmed
    /// frame — the F24 dark tax this plan refuses everywhere else.
    ///
    /// The whole ENUM crosses this boundary rather than a `bool`, because there are now three
    /// answers and a predicate can only carry two: a `collides()` flag would have to be joined by a
    /// second `counts_waves()` flag, and two booleans admit a fourth combination the enum does not.
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
    #[allow(clippy::too_many_arguments)]
    // SEVEN independent inputs, each from a different owner, and the count is worth naming one by
    // one because the list is the argument: `ctx` (the device); THREE borrowed resources this
    // bundle does NOT own and must not destroy — `camera_ring` (the per-FIF UBO ring, owned by
    // `GpuSceneBundles`), `bindless` (the shared texture table, owned by `BindlessTextureTable`)
    // and `edit_list` (the engine's ONE SDF edit list, owned by `GpuSceneBundles`, bound here at
    // Set-0 binding 10 for rung P1 and boot-static thereafter); the boot-frozen `capacity`; and the
    // two boot-frozen ARMINGS `deferred_path` and `collision`, each of which picks a shader
    // artifact. Grouping them would either hide which of the four are borrows of somebody else's
    // resource — the distinction `destroy` depends on — or invent a struct whose only purpose is
    // this one call.
    pub(crate) fn create(
        ctx: &VulkanContext,
        camera_ring: &[BoundBuffer; FRAMES_IN_FLIGHT],
        bindless: &BindlessTextureTable,
        edit_list: &BoundBuffer,
        capacity: u32,
        deferred_path: bool,
        collision: ParticleCollision,
        sort_mode: ParticleSortMode,
    ) -> Self {
        debug_assert!(capacity >= 1, "invariant: the particle pool needs at least one slot");
        // **R10, live rather than filed** (plan D10 / research fact R10). A sort re-permutes
        // `p_render` every frame, so slot `k` of frame N and slot `k` of frame N+1 are different
        // particles and their difference is not a velocity. Rung P3's `-D MOTION` resolver will
        // read `ParticleSortMode::motion_vectors_allowed`; until it exists this is the site that
        // makes the rule observable, and it is stated as an implication over the arm rather than as
        // a comment so the arm that breaks it fails here.
        debug_assert!(
            sort_mode.motion_vectors_allowed() == matches!(sort_mode, ParticleSortMode::None),
            "invariant: R10 — a ParticleSortMode that permutes p_render may not carry motion \
             vectors, and only ParticleSortMode::None does not permute it"
        );
        let device = ctx;
        let cap = u64::from(capacity);
        let sorts = !matches!(sort_mode, ParticleSortMode::None);

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
        // Rung P2 item 3's two buffers, allocated ONLY under a sorting arming (structural absence,
        // D13's rule on a fourth axis). `p_render_sorted` costs a second `CAP × 32 B` — 8.4 MB at
        // the default capacity — which is the price of the sort and is charged to nobody who does
        // not arm it.
        let render_sorted = sorts.then(|| {
            device_buffer(cap * size_of::<ParticleRender>() as u64, storage, "sorted render records")
        });
        let sort_bins =
            sorts.then(|| device_buffer(u64::from(PARTICLE_SORT_BINS_WORDS) * 4, storage, "sort bins"));
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
                render_sorted: render_sorted.as_ref(),
                sort_bins: sort_bins.as_ref(),
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
                    // Rung P1's field. Bound in BOTH parity sets and on a non-colliding run alike:
                    // the descriptor is written once at boot, and a bound-but-unread storage buffer
                    // costs nothing per frame.
                    BindGroupEntry::StorageBuffer { buffer: edit_list },
                    // Rung P2 item 3's two, with the PLACEHOLDER fallback this table's doc states.
                    // The placeholders are `p_render` and `p_counters` — both live for the bundle's
                    // whole lifetime, so the descriptor is never dangling — and no module that
                    // could read either slot is built on an unsorted run.
                    BindGroupEntry::StorageBuffer {
                        buffer: render_sorted.as_ref().unwrap_or(&render),
                    },
                    BindGroupEntry::StorageBuffer {
                        buffer: sort_bins.as_ref().unwrap_or(&counters),
                    },
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
        let sim =
            compute_pipeline(particle_sim_spirv_for(collision), PARTICLE_SIM_PUSH_BYTES, "sim");
        // Rung P2 item 3's three, built together or not at all (see [`ParticleSortPipelines`]).
        //
        // The SCAN's SHADER declares no push block at all — it is a pure reduction over a
        // fixed-size array and needs neither the capacity nor the camera. Its pipeline LAYOUT is
        // nonetheless given the family's range, because the RHI has no zero-range form
        // (`create_compute_pipeline` rejects `push_constant_bytes == 0`) and widening that
        // validation for one pipeline would be a device-wide change for a local convenience. A
        // declared range no shader references is legal Vulkan and costs nothing: the recorder
        // emits no `vkCmdPushConstants` for this pass, which is the property that matters and the
        // one its command census states.
        let sort = sorts.then(|| ParticleSortPipelines {
            hist: compute_pipeline(
                particle_sort_hist_spirv(),
                PARTICLE_SORT_PUSH_BYTES,
                "sort histogram",
            ),
            scan: compute_pipeline(particle_sort_scan_spirv(), PARTICLE_SORT_PUSH_BYTES, "sort scan"),
            scatter: compute_pipeline(
                particle_sort_scatter_spirv(),
                PARTICLE_SORT_PUSH_BYTES,
                "sort scatter",
            ),
        });

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
        // Rung P2 item 3: the alpha ring, differing in binding 0 alone. Built from the SAME layout
        // object, so the two are interchangeable at `vkCmdBindDescriptorSets` and the pipeline is
        // not re-bound between them.
        let draw_set0_alpha: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> =
            render_sorted.as_ref().map(|sorted| {
                core::array::from_fn(|fi| {
                    RhiDevice::create_bind_group(device, &BindGroupDesc {
                        layout: &draw_layout0,
                        entries: &[
                            BindGroupEntry::StorageBuffer { buffer: sorted },
                            BindGroupEntry::UniformBuffer { buffer: &camera_ring[fi] },
                        ],
                    })
                    .expect("invariant: particle sorted-alpha draw Set-0 bind group create")
                })
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
        // ONE descriptor, TWO pipelines: the blend is the only field that differs between the
        // classes, and building both from one `desc` closure is what keeps every other piece of
        // state — the formats, the push range, the layout, the depth compare op, `CullMode::None` —
        // provably identical instead of duplicated and drifting.
        let draw_desc = |blend: BlendState| GraphicsPipelineDesc {
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
            // The blend is PIPELINE state, never shader code.
            blend: Some(blend),
            // A billboard quad is two triangles facing the camera; culling either winding
            // would drop half of them depending on the rotation the sim stored.
            cull_mode: CullMode::None,
            depth_bias: None,
        };
        // ADDITIVE is COMMUTATIVE, which is what lets the class ship unsorted with a proof rather
        // than a hope (plan D10).
        let draw_pipeline = ctx
            .create_graphics_pipeline_particle(
                &draw_desc(BlendState::ADDITIVE),
                bindless.set().set_layout(),
                depth_compare,
            )
            .expect("invariant: particle draw graphics pipeline create");
        // STRAIGHT, not PREMULTIPLIED: `particle_draw.fs` emits `color_rgba8` unpacked and (when
        // textured) modulated — a straight-alpha source, with no `rgb *= a` anywhere in the chain.
        // Feeding it to the premultiplied state would double-count coverage and darken every edge.
        //
        // This class is NOT commutative, which is precisely why the next rung sorts it; until then
        // its intra-class order is the order waves retired in. No image pin may be authored over
        // OVERLAPPING alpha billboards before that lands.
        let draw_pipeline_alpha = ctx
            .create_graphics_pipeline_particle(
                &draw_desc(BlendState::STRAIGHT_ALPHA),
                bindless.set().set_layout(),
                depth_compare,
            )
            .expect("invariant: particle alpha draw graphics pipeline create");
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
            render_sorted,
            sort_bins,
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
            sort,
            draw_layout0,
            draw_set0,
            draw_set0_alpha,
            draw_pipeline,
            draw_pipeline_alpha,
            bindless_set: bindless.set().set(),
            capacity,
            collision,
            sort_mode,
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
        if self.collision.counts_waves() {
            assert_one_substep_for_the_census(push.steps);
        }
        ParticleActivation {
            kickoff_pipeline: &self.kickoff,
            emit_pipeline: &self.emit,
            sim_pipeline: &self.sim,
            sort_hist_pipeline: self.sort.as_ref().map(|s| &s.hist),
            sort_scan_pipeline: self.sort.as_ref().map(|s| &s.scan),
            sort_scatter_pipeline: self.sort.as_ref().map(|s| &s.scatter),
            draw_pipeline: &self.draw_pipeline,
            draw_pipeline_alpha: &self.draw_pipeline_alpha,
            sets: &self.sets[parity as usize],
            draw_set0: &self.draw_set0[fi],
            draw_set0_alpha: self.draw_set0_alpha.as_ref().map(|ring| &ring[fi]),
            draw_set1: self.bindless_set,
            counters: &self.counters,
            dispatch_args: &self.dispatch_args,
            draw_args: &self.draw_args,
            dead: &self.dead,
            alive_read: &self.alive[parity as usize],
            alive_write: &self.alive[(parity ^ 1) as usize],
            particle_records: &self.particle,
            render_records: &self.render,
            sorted_render_records: self.render_sorted.as_ref(),
            sort_bins: self.sort_bins.as_ref(),
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
            cam_eye: push.cam_eye,
            steps: push.steps,
            timestep: push.timestep,
            frame_index: push.frame_index,
            parity,
            draw_push: push.draw_push,
            draw_push_alpha: alpha_draw_push(push.draw_push, self.capacity),
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

    /// **Rung P2 item 3's monotonicity readback** (plan P2's named gate for the sort), with its
    /// non-vacuity CONTROL taken in the same submit.
    ///
    /// Copies back the alpha class's live range from BOTH `p_render_sorted` (the scatter's output,
    /// which the alpha draw reads) and `p_render` (the sim's unsorted output), decodes each into a
    /// [`ParticleSortRangeScan`], and hands the pair back. `None` on a run that did not arm the
    /// sort — there is no sorted buffer then, and a scan of the source alone would be a measurement
    /// with nothing to compare it to.
    ///
    /// # Why THIS readback exists where the transform's could not
    ///
    /// Rung P2 item 2's gate correction ruled that `p_render` cannot be read back to verify the
    /// alpha index transform: at the default capacity it is 8.4 MB, "for a value whose per-slot
    /// meaning the host cannot check without re-deriving the whole sim". Both halves of that
    /// objection fail here, which is why the plan names a monotonicity readback for the SORT and
    /// named none for the transform:
    ///
    /// * the range copied is `alpha.instanceCount` records, not `CAP` — 160 KB on the saturated lab
    ///   leg, bounded at [`PARTICLE_SORT_READBACK_MAX_RECORDS`] for anything larger;
    /// * and the per-slot meaning IS checkable without the sim: the property is a relation between
    ///   ADJACENT records (their keys do not decrease), and the key is a function of the record's
    ///   own `position` and the eye — nothing the sim decided enters it.
    ///
    /// # Rank order, and the mirror this fn undoes
    ///
    /// The class is stored DESCENDING (rank `r` at `capacity - 1 - r`), so the copied range is
    /// reversed before scanning. That reversal is done HERE, once, on the host — the alternative
    /// (scanning backwards) would make every rank in the report an index the reader has to invert.
    ///
    /// # Panics
    ///
    /// Panics (`expect("invariant: ...")`) on any RHI failure, exactly as [`Self::read_counters`]
    /// does and for the same reason: this runs on a diagnostic path that ends the process, where a
    /// silent `None` would read like a scene that produced nothing.
    pub(crate) fn read_sort_scan(
        &self,
        ctx: &VulkanContext,
        alpha_count: u32,
        cam_eye: [f32; 3],
        frames_presented: u32,
    ) -> Option<ParticleSortReadback> {
        let sorted_buffer = self.render_sorted.as_ref()?;
        debug_assert!(
            !matches!(self.sort_mode, ParticleSortMode::None),
            "invariant: the sorted render buffer exists exactly when the sort arm does"
        );
        let record_bytes = size_of::<ParticleRender>() as u64;
        let count = alpha_count.min(PARTICLE_SORT_READBACK_MAX_RECORDS).min(self.capacity);
        if count == 0 {
            // A class with nothing in it: report the empty pair rather than submitting a
            // zero-length copy, so the caller's `is_conclusive` is what refuses it.
            return Some(ParticleSortReadback {
                frames_presented,
                sorted: ParticleSortRangeScan::EMPTY,
                source: ParticleSortRangeScan::EMPTY,
            });
        }
        // The class occupies `[capacity - alpha_count, capacity)`. When `alpha_count` exceeds the
        // readback bound the PREFIX of the RANK order is what matters — ranks 0..count, i.e. the
        // TOP `count` slots — so the source offset is measured from the top of the buffer, never
        // from the class's own base.
        let src_offset = (u64::from(self.capacity) - u64::from(count)) * record_bytes;
        let range_bytes = u64::from(count) * record_bytes;
        let total = range_bytes * 2;

        let device = ctx;
        RhiDevice::wait_idle(device).expect("invariant: particle sort readback device idle");

        let staging = RhiDevice::create_buffer(device, &BufferDesc {
            size: total,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("invariant: particle sort readback staging create");
        let mapped = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("invariant: host-visible particle sort readback staging is mapped");

        let mut encoder = RhiDevice::create_command_encoder(device)
            .expect("invariant: particle sort readback command encoder create");
        let fence = RhiDevice::create_fence(device, false)
            .expect("invariant: particle sort readback fence create");
        encoder.begin().expect("invariant: particle sort readback encoder begin");
        // Availability for both sources: `p_render_sorted` was written by `particle_sort_scatter`
        // and `p_render` by `particle_sim`, both through STORAGE descriptors. The device idle above
        // ordered the execution; this makes the writes visible to a transfer read.
        encoder.pipeline_barrier(&BarrierDesc {
            src_stage: BarrierStage::COMPUTE_SHADER,
            dst_stage: BarrierStage::TRANSFER,
            buffers: &[
                BufferBarrier {
                    buffer: sorted_buffer,
                    src_access: BarrierAccess::SHADER_WRITE,
                    dst_access: BarrierAccess::TRANSFER_READ,
                },
                BufferBarrier {
                    buffer: &self.render,
                    src_access: BarrierAccess::SHADER_WRITE,
                    dst_access: BarrierAccess::TRANSFER_READ,
                },
            ],
        });
        encoder.copy_buffer(sorted_buffer, &staging, &[BufferCopy {
            src_offset,
            dst_offset: 0,
            size: range_bytes,
        }]);
        encoder.copy_buffer(&self.render, &staging, &[BufferCopy {
            src_offset,
            dst_offset: range_bytes,
            size: range_bytes,
        }]);
        encoder.end().expect("invariant: particle sort readback encoder end");
        device
            .rhi_queue()
            .submit(&encoder, &fence)
            .expect("invariant: particle sort readback submit");
        RhiDevice::wait_fence(device, &fence, u64::MAX)
            .expect("invariant: particle sort readback fence wait");

        let n = count as usize;
        // SAFETY: `mapped` addresses `total == 2 * n * size_of::<ParticleRender>()` valid mapped
        // host-coherent bytes of a buffer this fn just created, and the fence wait above completed
        // the ONLY submission that writes them (the memory being HOST_COHERENT, the results are
        // host-visible). `ParticleRender` is `#[repr(C)]` + `Pod`, so every byte pattern of its
        // exact size is a valid value and no padding is uninitialized. The two slices cover the two
        // disjoint halves `[0, n)` and `[n, 2n)` of one allocation whose base the RHI aligns to at
        // least 16 bytes — more than `ParticleRender`'s alignment. Nothing else aliases the mapping
        // and the slices do not outlive this fn.
        let (sorted_records, source_records) = unsafe {
            let base = mapped.as_ptr().cast::<ParticleRender>();
            (
                core::slice::from_raw_parts(base, n),
                core::slice::from_raw_parts(base.add(n), n),
            )
        };

        // The class is stored DESCENDING, so slot `capacity - 1 - r` holds rank `r`: the copied
        // range read BACKWARDS is rank order. Reversed into a scratch column rather than scanned in
        // reverse so every rank the report names is a rank and not its mirror.
        let mut rank_order: Vec<ParticleRender> = Vec::with_capacity(n);
        let mut scan_one = |records: &[ParticleRender]| {
            rank_order.clear();
            rank_order.extend(records.iter().rev().copied());
            let mut scan = scan_alpha_range(&rank_order, cam_eye);
            // The scan only ever saw the prefix it was handed; the CLASS's true length is this
            // fn's, and `is_complete()` is the difference.
            scan.alpha_count = alpha_count;
            scan
        };
        let sorted = scan_one(sorted_records);
        let source = scan_one(source_records);

        // SAFETY: `encoder`, `fence` and `staging` were created on `device` above; the encoder's
        // only submission completed (the fence wait returned), so no GPU work references any of
        // them; each is moved by value ⇒ destroyed exactly once. The two slices above borrow the
        // mapping this destroys — they are NOT read after this point (both scans ran above, and
        // `rank_order` holds COPIES, not references into it), so nothing observes the freed
        // memory. The borrow checker cannot see that dependency, which is why it is stated.
        unsafe {
            RhiDevice::destroy_command_encoder(device, encoder);
            RhiDevice::destroy_fence(device, fence);
            RhiDevice::destroy_buffer(device, staging);
        }

        Some(ParticleSortReadback { frames_presented, sorted, source })
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
            RhiDevice::destroy_graphics_pipeline(ctx, self.draw_pipeline_alpha);
            RhiDevice::destroy_graphics_pipeline(ctx, self.draw_pipeline);
            // Rung P2 item 3's alpha ring, before the layout both rings were built from.
            if let Some(ring) = self.draw_set0_alpha {
                for bg in ring {
                    RhiDevice::destroy_bind_group(ctx, bg);
                }
            }
            for bg in self.draw_set0 {
                RhiDevice::destroy_bind_group(ctx, bg);
            }
            RhiDevice::destroy_bind_group_layout(ctx, self.draw_layout0);
            // Rung P2 item 3's three, in reverse creation order like every other pipeline here.
            if let Some(s) = self.sort {
                RhiDevice::destroy_compute_pipeline(ctx, s.scatter);
                RhiDevice::destroy_compute_pipeline(ctx, s.scan);
                RhiDevice::destroy_compute_pipeline(ctx, s.hist);
            }
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
            // Rung P2 item 3's two. `Option::map` over a by-value field ⇒ destroyed exactly once,
            // and never at all when the sort was disarmed (nothing was created).
            if let Some(b) = self.sort_bins {
                RhiDevice::destroy_buffer(ctx, b);
            }
            if let Some(b) = self.render_sorted {
                RhiDevice::destroy_buffer(ctx, b);
            }
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
    /// Rung P2 item 3: `ViewUniform::camera_pos.xyz` — the eye the sort key measures from, and the
    /// same one the camera UBO carries.
    pub(crate) cam_eye: [f32; 3],
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

/// Rung P2 (plan D10): the ALPHA class's `VERTEX` push, derived from the ADDITIVE one.
///
/// The two differ in their last eight bytes and nowhere else: bytes `[0,64)` are the path's own
/// view-projection rows, which BOTH classes must agree on to the last bit (they are one raster of
/// one scene), and `[64,72)` carry the render-index affine the VS applies as
/// `index_base + index_step * SV_InstanceID` — `(0, +1)` for additive and `(capacity - 1, -1)`
/// here, the reverse walk of the far end the sim wrote this class into.
///
/// Derived rather than assembled: copying the matrix from the additive push makes the
/// "one projection" property structural instead of a thing two call sites have to keep true.
///
/// Pure, so the transform is testable without a device.
///
/// # Panics (debug)
///
/// `debug_assert!`s `capacity > 0` — a zero capacity would make `index_base` wrap to `u32::MAX`
/// and every alpha instance read past the render buffer, on a device with `robustBufferAccess`
/// OFF. The bundle's capacity is boot-frozen from `ParticleConfig`, which clamps it above zero.
///
/// No `#[inline]`: this is a private fn in the same crate as its one caller, called ONCE per frame
/// from `activation`. LLVM inlines it or does not on its own evidence, and an attribute here would
/// be decoration — principle 7's measured-inlining rule, which is about not annotating for
/// cosmetics as much as it is about not over-annotating hot code.
fn alpha_draw_push(
    additive: [u8; PARTICLE_DRAW_PUSH_BYTES as usize],
    capacity: u32,
) -> [u8; PARTICLE_DRAW_PUSH_BYTES as usize] {
    debug_assert!(capacity > 0, "invariant: the boot-frozen particle capacity is non-zero");
    let mut push = additive;
    push[64..68].copy_from_slice(&(capacity - 1).to_ne_bytes());
    push[68..72].copy_from_slice(&(-1i32).to_ne_bytes());
    push
}

/// **Rung P1b: the census's one-substep precondition, enforced as a HARD REFUSAL.**
///
/// The `-D SDF_COLLIDE_STATS` census sits at the TOP OF THE SUBSTEP LOOP BODY, so on the second and
/// later iterations it is reached from the previous iteration's DIVERGENT skip branch. Vulkan does
/// not guarantee reconvergence at a merge block without `VK_KHR_shader_maximal_reconvergence`, so a
/// still-split wave may run the census once per divergent group: one `WaveIsFirstLane()` elects per
/// group, each ballots over its own subset, and **the same wave-substep is counted more than once**.
/// The wave counters then stop summing to the wave-substep total and the skip rate loses its
/// denominator — silently, because every internal consistency bound the readback gate checks still
/// holds.
///
/// `ParticleClock` supports up to `PARTICLE_SUBSTEP_CEILING = 64` (D6/M3), so this is reachable by
/// configuration and not merely in theory. The instrument therefore REFUSES rather than misleads:
/// the measurement fixture pins one substep per frame, and any other caller that arms the census
/// gets a panic naming what would lift the restriction.
///
/// It is a hard `assert!`, not a `debug_assert!`: a release measurement run is exactly when this
/// would otherwise pass silently and hand back a wrong number.
///
/// # Panics
///
/// When the census arm is armed and `steps != 1`.
#[cold]
#[inline(never)]
fn assert_one_substep_for_the_census(steps: u32) {
    assert_eq!(
        steps, 1,
        "ParticleCollision::SdfStats requires EXACTLY ONE SUBSTEP per dispatch, got {steps}. The \
         per-wave census is taken inside the substep loop, and from the second iteration on it is \
         reached from a divergent branch — without VK_KHR_shader_maximal_reconvergence the wave \
         may still be split there, so one wave-substep would be counted once per divergent group \
         and `waves_skipped + waves_evaluated` would stop being the wave-substep total. The rate \
         would be wrong and every consistency bound would still pass. Lift this by enabling \
         maximal reconvergence (and re-deriving the census), or drive the clock at one substep per \
         frame as the measurement fixture does."
    );
}

/// The `particle_sim` SPIR-V the compute pipeline is frozen with for a resolved
/// [`ParticleCollision`](boyko_render::ParticleCollision) arming (rung P1 / plan D9, rung P1b).
///
/// * [`Off`](ParticleCollision::Off) — the BASE module. The define is invisible to DXC, so the
///   committed base `.spv` is byte-frozen and a non-colliding run pays exactly what P0 paid.
/// * [`Sdf`](ParticleCollision::Sdf) — the `-D SDF_COLLIDE` module. Its per-substep loop either
///   skips the field on the Lipschitz bound cached in the sim record's `cached_field_d` lane, or
///   evaluates `field_distance` once and — inside `collision_radius` — resolves the contact against
///   `sdf_normal`.
/// * [`SdfStats`](ParticleCollision::SdfStats) — rung P1b's INSTRUMENT: that same simulation plus a
///   per-wave census of the skip, published into `p_counters`' three stats words. A measurement
///   arm; it runs atomics a shipping configuration should not pay for.
///
/// The match is WILDCARD-FREE, so a fourth collider arm has to state which module it builds instead
/// of inheriting whichever one a `_` swallowed — the defect class gate #12 exists for.
///
/// All three are interface-identical apart from ONE added read (`Buf` @10, in the layout either
/// way), so the pick never reaches the descriptor plumbing and every variant shares one pipeline
/// layout, one push range and one bind group.
///
/// Boot-frozen, like [`particle_draw_spirv_for`]: exactly one sim `VkPipeline` exists per process.
#[inline]
pub(crate) fn particle_sim_spirv_for(collision: ParticleCollision) -> &'static [u32] {
    match collision {
        ParticleCollision::Off => particle_sim_spirv(),
        ParticleCollision::Sdf => particle_sim_sdf_spirv(),
        ParticleCollision::SdfStats => particle_sim_stats_spirv(),
    }
}

/// The device buffers the boot fill writes, plus the index buffer. Grouped so [`boot_fill`] takes
/// one argument instead of a dozen.
struct BootFillTargets<'a> {
    counters: &'a BoundBuffer,
    dispatch_args: &'a BoundBuffer,
    draw_args: &'a BoundBuffer,
    dead: &'a BoundBuffer,
    alive: &'a [BoundBuffer; 2],
    particle: &'a BoundBuffer,
    render: &'a BoundBuffer,
    /// Rung P2 item 3's sorted render records, or `None` on an unsorted arming.
    render_sorted: Option<&'a BoundBuffer>,
    /// Rung P2 item 3's radix scratch, or `None` on an unsorted arming. Zeroing it is
    /// LOAD-BEARING, unlike most of the fill: `particle_sort_hist` ACCUMULATES into the histogram
    /// half, and every frame after the first is handed a zeroed half by `particle_sort_scan`'s own
    /// re-zero — so frame 0's zero has to come from here or the first histogram is built on
    /// whatever the allocation carried.
    sort_bins: Option<&'a BoundBuffer>,
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
/// * `p_sort_bins` ← 0 (rung P2 item 3, when armed) — and THIS one IS load-bearing. The histogram
///   pass ACCUMULATES; every frame after the first is handed a zeroed histogram half by the scan's
///   own re-zero, so frame 0's has to come from here. `p_render_sorted` ← 0 for `p_render`'s
///   reason.
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
    // every byte is immediately overwritten. The last two rows are rung P2 item 3's and are
    // present only under a sorting arming — `flatten` drops them structurally rather than emitting
    // a zero-length copy.
    let zero_targets: [Option<(&BoundBuffer, u64)>; 10] = [
        Some((t.dispatch_args, size_of::<ParticleDispatchArgs>() as u64)),
        Some((t.draw_args, size_of::<ParticleDrawArgs>() as u64)),
        Some((&t.alive[0], cap * 4)),
        Some((&t.alive[1], cap * 4)),
        Some((t.particle, cap * size_of::<ParticleSim>() as u64)),
        Some((t.render, cap * size_of::<ParticleRender>() as u64)),
        t.render_sorted.map(|b| (b, cap * size_of::<ParticleRender>() as u64)),
        t.sort_bins.map(|b| (b, u64::from(PARTICLE_SORT_BINS_WORDS) * 4)),
        Some((t.emit_req_device, EMIT_REQ_TABLE_BYTES)),
        Some((t.effects_device, EFFECT_TABLE_BYTES)),
    ];
    for (dst, total) in zero_targets.into_iter().flatten() {
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

    // ── Rung P2: the alpha class's index transform ──────────────────────────
    //
    // The transform's failure mode is INVISIBLE TO EVERY IMAGE GATE, measured rather than assumed:
    // forcing it to the additive identity produced a dump BYTE-IDENTICAL to the `particle_additive`
    // golden (the alpha command then re-draws the additive records, which are already white). So
    // the cheap pin has to live here — and `alpha_draw_push` is a pure fn precisely so it can.

    /// The additive push this fixture derives from: a recognisable matrix pattern in `[0,64)` and
    /// the P0 identity `(0, +1)` in the tail, i.e. exactly what the runner assembles.
    fn additive_push_fixture() -> [u8; PARTICLE_DRAW_PUSH_BYTES as usize] {
        let mut push = [0u8; PARTICLE_DRAW_PUSH_BYTES as usize];
        for (i, b) in push[..64].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        push[64..68].copy_from_slice(&0u32.to_ne_bytes());
        push[68..72].copy_from_slice(&1i32.to_ne_bytes());
        push
    }

    #[test]
    fn the_alpha_push_mirrors_the_index_and_reverses_the_step() {
        let additive = additive_push_fixture();
        let alpha = alpha_draw_push(additive, 65_536);

        assert_eq!(
            u32::from_ne_bytes(alpha[64..68].try_into().expect("invariant: 4 bytes")),
            65_535,
            "index_base must be `capacity - 1` — the TOP of the render buffer, which is where the \
             sim wrote this class from"
        );
        assert_eq!(
            i32::from_ne_bytes(alpha[68..72].try_into().expect("invariant: 4 bytes")),
            -1,
            "index_step must be -1: the alpha class is written DOWNWARD from the far end, so the \
             VS must walk it downward or read records nothing wrote"
        );
    }

    /// The two classes must agree on the projection **to the last bit** — they are one raster of
    /// one scene, and two derivations of the view-projection would be two chances to disagree.
    /// This is why `alpha_draw_push` derives from the additive push instead of assembling its own.
    #[test]
    fn the_alpha_push_shares_the_additive_view_projection_byte_for_byte() {
        let additive = additive_push_fixture();
        let alpha = alpha_draw_push(additive, 4_096);

        assert_eq!(
            alpha[..64],
            additive[..64],
            "bytes [0,64) are the path's own view-projection rows and must be copied, not rebuilt"
        );
        assert_ne!(
            alpha[64..72],
            additive[64..72],
            "...and the tail must differ, or the alpha draw walks the ADDITIVE range — the failure \
             this whole test exists for, and the one no image gate can see"
        );
    }

    /// The transform is exactly an involution on the index space the sim mirrors with: for every
    /// class-dense position `q`, the VS's `index_base + index_step * q` must land on the sim's own
    /// `capacity - 1 - q`. Stated over a RANGE rather than one point, because an off-by-one here
    /// puts the first alpha billboard one slot past the last written record.
    #[test]
    fn the_vs_affine_lands_on_the_slot_the_sim_wrote() {
        const CAP: u32 = 1_024;
        let alpha = alpha_draw_push(additive_push_fixture(), CAP);
        let base = u32::from_ne_bytes(alpha[64..68].try_into().expect("invariant: 4 bytes"));
        let step = i32::from_ne_bytes(alpha[68..72].try_into().expect("invariant: 4 bytes"));

        for q in [0u32, 1, 2, 17, CAP / 2, CAP - 2, CAP - 1] {
            let vs_reads = (base as i32 + step * q as i32) as u32;
            let sim_wrote = CAP - 1 - q;
            assert_eq!(vs_reads, sim_wrote, "class-dense alpha position {q} at capacity {CAP}");
        }
    }

    /// The wave-leader `InterlockedAdd` sites both SHIPPING sim modules carry: the list counter
    /// (`alive_count_next`), each blend class's render counter, and the dying path's free-list push.
    ///
    /// **Four since rung P2, up from three** — D10's blend partition gives each class its own render
    /// counter, since the two take positions from opposite ends of `p_render` and one shared counter
    /// cannot yield both. Not a widening of the aggregation: still one op per wave per counter,
    /// each `> 0u`-guarded, so an additive-only wave issues the three it always did.
    ///
    /// Mirrors `particle_edsl_sync`'s `SIM_WAVE_LEADER_ATOMIC_SITES`; both read the same committed
    /// artifacts, from opposite sides of the crate boundary.
    const SIM_SHIPPING_ATOMIC_SITES: usize = 4;

    /// The census's own `InterlockedAdd` sites in the `-D SDF_COLLIDE_STATS` instrument
    /// (`waves_evaluated`, `waves_skipped`, `lanes_evaluated`). Unmoved by rung P2.
    const SIM_CENSUS_ATOMIC_SITES: usize = 3;

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

    /// `OpDecorate`'s opcode (SPIR-V 1.x core, section 3.32.3).
    const OP_DECORATE: u32 = 71;
    /// The `Binding` decoration (SPIR-V 1.x, section 3.20) — its literal operand is the binding
    /// number the descriptor set exposes the variable at.
    const DECORATION_BINDING: u32 = 33;

    /// Whether `spirv` decorates any variable with `Binding <binding>`.
    ///
    /// The same real instruction walk [`declares_depth_replacing`] does, for the same reason: a
    /// bare "contains the value 10" scan would match a constant, an id or a literal anywhere in the
    /// module and could never fail honestly.
    fn declares_binding(spirv: &[u32], binding: u32) -> bool {
        let mut i = SPIRV_HEADER_WORDS;
        while i < spirv.len() {
            let word_count = (spirv[i] >> 16) as usize;
            let opcode = spirv[i] & 0xFFFF;
            assert!(word_count >= 1, "malformed SPIR-V: zero-length instruction at word {i}");
            if opcode == OP_DECORATE
                && word_count >= 4
                && spirv[i + 2] == DECORATION_BINDING
                && spirv[i + 3] == binding
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

    /// **Rung P1's selector, pinned by identity AND by artifact property** — the same class of
    /// defect the two tests above exist for, one rung later.
    ///
    /// A swapped arm here is invisible to every text and byte pin in the tree: both sim modules are
    /// re-DXC'd and censused by `particle_edsl_sync`, and both stay green while the host builds the
    /// pipeline from the wrong one. The visible symptom would be particles falling through the
    /// world (or paying the field walk for nothing), which only a GPU run shows.
    ///
    /// The property claim is the field binding read out of the committed artifact: rung P1's module
    /// decorates a variable `Binding 10` (the edit list `sdf_field.hlsli` walks), and the base one
    /// cannot — the `#ifdef SDF_COLLIDE` block is invisible to DXC there, which is exactly what
    /// makes the base `.spv` byte-frozen.
    #[test]
    fn the_collide_arm_takes_the_sdf_module_and_the_base_arm_does_not() {
        let collide = particle_sim_spirv_for(ParticleCollision::Sdf);
        assert!(
            is_the_same_blob(collide, particle_sim_sdf_spirv()),
            "ParticleCollision::Sdf must build the -D SDF_COLLIDE sim module"
        );
        assert!(
            declares_binding(collide, 10),
            "the collide module must declare the SDF edit list at binding 10 — the accessor may \
             name the variant while the artifact behind it is the base compile"
        );

        let base = particle_sim_spirv_for(ParticleCollision::Off);
        assert!(
            is_the_same_blob(base, particle_sim_spirv()),
            "ParticleCollision::Off must build the BASE sim module — a colliding module on a \
             disarmed run pays the field consumer's cost on every substep for nothing"
        );
        assert!(
            !declares_binding(base, 10),
            "the base module must NOT declare binding 10: structural absence is what makes the \
             base .spv byte-frozen and the disarmed run identical to P0's"
        );

        assert!(
            !is_the_same_blob(collide, base),
            "the two arms must be distinct artifacts — otherwise the selector is a branch with one \
             outcome"
        );
        assert_ne!(collide, base, "the two sim modules must differ in CONTENT, not only in address");
    }

    /// **Rung P1b's selector**, the same two claims one arm further: identity, then an ARTIFACT
    /// PROPERTY that separates the instrument from the module it instruments.
    ///
    /// Identity alone is weaker here than it was for rung P1, because the stats module and the
    /// collide module share every binding — the census writes to `p_counters` @0, which the sim has
    /// bound since P0. So `declares_binding` cannot tell them apart, and the property claim is the
    /// ATOMIC POPULATION instead: the census is three more `OpAtomicIAdd` sites (7 against the
    /// shipping 4 since rung P2 — see [`SIM_SHIPPING_ATOMIC_SITES`]), which is the one thing an
    /// `embed_spirv!` pointed back at the wrong `.spv` could not fake.
    #[test]
    fn the_stats_arm_takes_the_instrumented_module_and_the_shipping_arms_do_not() {
        let stats = particle_sim_spirv_for(ParticleCollision::SdfStats);
        assert!(
            is_the_same_blob(stats, particle_sim_stats_spirv()),
            "ParticleCollision::SdfStats must build the -D SDF_COLLIDE_STATS sim module"
        );
        assert!(
            declares_binding(stats, 10),
            "the stats module is the COLLIDE module plus a census — it must still read the field"
        );

        let sdf = particle_sim_spirv_for(ParticleCollision::Sdf);
        let base = particle_sim_spirv_for(ParticleCollision::Off);
        assert!(
            !is_the_same_blob(stats, sdf) && !is_the_same_blob(stats, base),
            "the instrument must be its own artifact: an arm that resolved to the shipping module \
             would report a skip rate of 0/0 while every other pin stayed green"
        );

        // The atomic population, read out of the committed artifact: the shipping wave-leader
        // sites in both shipping modules, plus the census's three in the instrument.
        //
        // Expressed against the named consts rather than as literals, because rung P2 moved the
        // shipping half from 3 to 4 (D10's per-class render counter) and a literal would have had
        // to be re-derived by hand at exactly the moment it changed. This test went RED on that
        // move, which is what a pin is for.
        assert_eq!(
            count_atomic_iadd(base),
            SIM_SHIPPING_ATOMIC_SITES,
            "the base sim's wave-leader budget"
        );
        assert_eq!(
            count_atomic_iadd(sdf),
            SIM_SHIPPING_ATOMIC_SITES,
            "collision publishes nothing (rung P1's claim)"
        );
        assert_eq!(
            count_atomic_iadd(stats),
            SIM_SHIPPING_ATOMIC_SITES + SIM_CENSUS_ATOMIC_SITES,
            "the instrument must carry the shipping sites PLUS the census's three — this is the \
             property that distinguishes it from the module it measures"
        );
    }

    /// **Rung P1b's one-substep refusal, gated rather than merely written.**
    ///
    /// The hazard it covers cannot be reached through the measurement fixture (which pins one
    /// substep per frame), so without this test the refusal would be a line of code no run ever
    /// executes — and a later edit could invert or delete it with every gate green. `ParticleClock`
    /// supports up to `PARTICLE_SUBSTEP_CEILING = 64`, so the configuration IS reachable.
    #[test]
    fn the_census_refuses_more_than_one_substep() {
        // The legal case: exactly one substep, no panic.
        assert_one_substep_for_the_census(1);

        for steps in [0u32, 2, 64] {
            let refused = std::panic::catch_unwind(|| assert_one_substep_for_the_census(steps));
            assert!(
                refused.is_err(),
                "steps = {steps} must be REFUSED: from the second substep on the census is reached \
                 from a divergent branch, so a still-split wave counts one wave-substep more than \
                 once and the skip rate silently loses its denominator"
            );
        }
    }

    /// Counts `OpAtomicIAdd` (opcode 234) instructions in a SPIR-V word stream.
    ///
    /// Walks the instruction stream by word length rather than scanning for a value, because the
    /// opcode number can also appear as a literal or a result id inside another instruction — the
    /// whole-token discipline `particle_edsl_sync`'s census follows, in the binary.
    fn count_atomic_iadd(words: &[u32]) -> usize {
        const OP_ATOMIC_IADD: u32 = 234;
        const HEADER_WORDS: usize = 5;
        let mut i = HEADER_WORDS;
        let mut found = 0;
        while i < words.len() {
            let opcode = words[i] & 0xFFFF;
            let len = (words[i] >> 16) as usize;
            if len == 0 {
                break; // malformed; the byte gates own that failure, not this counter
            }
            if opcode == OP_ATOMIC_IADD {
                found += 1;
            }
            i += len;
        }
        found
    }
}
