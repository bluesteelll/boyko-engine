//! Particles P0 — the ECS component vocabulary and the GPU-facing POD records.
//!
//! # D1: particles are not entities; EMITTERS are
//!
//! At the house's measured spawn rate, 20k spawns/frame through `Commands` is 0.6–2 ms of CPU
//! against ~2 µs of GPU emit, so a particle is never an `Entity`. An EMITTER is: it carries a
//! [`ParticleEmitter`], a `Transform`/`GlobalTransform` pose and a [`ParticleEffectHandle`], and
//! `particle_tick_emitters` folds ≤ [`MAX_EMITTERS`] of them into one
//! [`EmitRequestGpu`] table each frame. The consequence is stated rather than hidden: particles
//! are not queryable, observable, or serializable.
//!
//! # Principle 0, and the one sanctioned exception
//!
//! Per-particle state is a **GPU-contiguity buffer** — the sanctioned exception, the same class as
//! a Vulkan `*const T + count` or a swapchain image — with **no CPU mirror**. Everything that DOES
//! live on the CPU lives on the engine's own storage: the emitter rows are `ComponentPool`
//! columns, the per-frame staging is a
//! [`ScratchColumn`](boyko_ecs::ecs::core::component::scratch::ScratchColumn), and the effect
//! table is `Assets<ParticleEffect>`. There is no `std::Vec` side store anywhere in this
//! subsystem.
//!
//! # Two records, AoS, not SoA (D2)
//!
//! [`ParticleSim`] (48 B) is the sim's working set: under the alive-list gather a 48 B 16-aligned
//! record is ONE fully-consumed 64 B line per particle, where SoA would fetch three lines and use
//! 48 of 192 bytes — the CPU-side SoA argument does not transfer to a GPU gather.
//! [`ParticleRender`] (32 B) is the draw's: a dense sequential stream with no gather, which also
//! moves every curve/ramp evaluation into the sim (once per particle) instead of the VS (four
//! times per particle).
//!
//! # Every offset here is a GENERATOR INPUT
//!
//! [`PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET`] and [`PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET`] are
//! `offset_of!` chains, not typed literals: the shader generator emits the word indices FROM these
//! consts, so a layout change moves the shader and the host together or fails the build. The same
//! discipline covers every `const _: () = assert!(offset_of!(..))` below.
//!
//! # Where each record lands on the device (informational)
//!
//! The authoritative binding table is the host layout table in the integration crate, not this
//! file; this map is here so a reader of a struct can see what it becomes. Compute set 0, bindings
//! 0..9: [`ParticleCounters`], [`ParticleDispatchArgs`], [`ParticleDrawArgs`], the dead list, the
//! two alive lists, [`ParticleSim`] (48 B), [`ParticleRender`] (32 B), [`EmitRequestGpu`] (SRV),
//! [`EffectParamsGpu`] (SRV). The draw binds set 0 = the render buffer at 0 plus the camera
//! cbuffer at 1, and set 1 = the bindless `Texture2D[]` at 0 plus a sampler at 1.
//!
//! # What P0's shaders actually evaluate
//!
//! Recorded once here because several fields below would otherwise read as promises: the shipped
//! P0 shaders evaluate the **SIZE ramp only**, over [`EffectParamsGpu::size_keys`]' four `f16`
//! keys at UNIFORM key times. Colour is **spawn-passthrough** — `color_keys[0]`, carried unchanged
//! for the particle's whole life — and [`EffectParamsGpu::color_times`] is layout parity, read by
//! nothing. The 4-key RGBA8 colour blend is a named post-P0 item. The host packer fills all three
//! lanes per the plan's layout regardless, so landing the consumer later moves no offset.

use std::mem::offset_of;

use boyko_ecs::ecs::constants::pool_reserve_rows;
use boyko_ecs::ecs::core::asset::{GEN_UNSYNCED, register_asset_layout};
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Resource};
use boyko_scene::transform::{GlobalTransform, Transform};
use bytemuck::{Pod, Zeroable};

// ── Boot constants ───────────────────────────────────────────────────────────────────

/// The fixed size of the CPU-facing emitter table uploaded to the device each frame (D15/R8).
///
/// A WRITER-side release clamp, never a `debug_assert!` alone: Hanabi shipped a 12 B indirect
/// overrun at ~260 instances because a GPU table was sized from a constant, and this device runs
/// with `robustBufferAccess` OFF, so an out-of-range fetch is undefined behaviour rather than a
/// clamp. `particle_tick_emitters` drops emitters past this bound and COUNTS them
/// ([`ParticleEmitScratch::dropped_emitters`](crate::particle_system::ParticleEmitScratch::dropped_emitters)).
pub const MAX_EMITTERS: usize = 256;

/// The fixed size of the effect-parameter table uploaded to the device (32 KB at 128 B/row —
/// effectively constant-resident in the sim's cache). Same release-clamp discipline as
/// [`MAX_EMITTERS`].
pub const MAX_EFFECTS: usize = 256;

/// The hard ceiling on substeps the sim may run in one frame (M3) — the HOST-applied,
/// time-DROPPING clamp.
///
/// Reachable in practice only through `Time::relative_speed`, which is public and validated
/// `finite && >= 0`: speed 8.0 on the stock 250 ms `max_delta` asks for 128 substeps at 64 Hz.
/// [`ParticleClock::advance`](crate::particle_clock::ParticleClock::advance) applies the clamp
/// ONCE, on the host, and pushes the clamped value to the shader; the shader's own
/// `min(pc.steps, PARTICLE_SUBSTEP_CEILING)` survives solely as the hang guard against a corrupt
/// push constant and can never bind. Raising it costs only shader loop iterations on hitch frames.
pub const PARTICLE_SUBSTEP_CEILING: u32 = 64;

/// Blend class `0`: ADDITIVE — the only class P0 ships. Commutative under 8-bit saturation
/// (`sat(sat(x)+y) = min(1, x+y)`), so P0 ships unsorted, provably (D10).
pub const PARTICLE_BLEND_ADDITIVE: u32 = 0;

/// Blend class `1`: ALPHA — declared here so the class word has ONE definition, but unreachable at
/// P0 (rung P2 lands the sort, the second draw slot and the mirrored render index).
pub const PARTICLE_BLEND_ALPHA: u32 = 1;

// ── The emitter components ───────────────────────────────────────────────────────────

/// A particle emitter — the only particle-system object that is an `Entity` (D1).
///
/// 16 B. D16 records the hot/cold mix (`rate`/`speed_scale` are cold owner state, `accumulator`
/// and `burst` are written every frame) as a DECIDED EXCEPTION rather than an oversight: at
/// ≤ [`MAX_EMITTERS`] rows a hot/cold split would cost a second column fetch to save nothing
/// measurable, and `&mut T` stamps no change tick, so the unconditional accumulator write is free.
///
/// # Required components
///
/// `#[require(Transform, GlobalTransform, ParticleEffectHandle)]` enforces *an emitter can never
/// exist without a pose and an effect*: the spawn basis is read from `GlobalTransform` and the
/// effect index from the handle, so a row missing either would be a silent no-spawn. Supplying a
/// component explicitly suppresses its auto-insert.
///
/// # Arming (D13, three axes)
///
/// Owner armed → [`ParticleConfig::mode`](crate::particle_config::ParticleConfig::mode);
/// entity IS an emitter → this component's PRESENCE (opt-in); emitter on NOW →
/// [`EmitterActive`]'s enable bit.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Transform, GlobalTransform, ParticleEffectHandle)]
pub struct ParticleEmitter {
    /// Continuous spawn rate in particles per second. Multiplied by this frame's
    /// `steps · timestep`, accumulated in [`accumulator`](Self::accumulator).
    pub rate: f32,
    /// The fractional spawn carried between frames, always in `[0, 1)`. Written every frame; a
    /// rate of 0.4/s therefore spawns exactly two particles every five seconds rather than none.
    pub accumulator: f32,
    /// One-shot spawns to add to THIS frame's count, CONSUMED (zeroed) by the tick that reads it —
    /// a burst fires exactly once no matter how many frames elapse before the next write.
    pub burst: u32,
    /// Per-emitter multiplier on the effect's initial speed range, forwarded to the device with
    /// the spawn basis. `1.0` is the effect's authored speed.
    pub speed_scale: f32,
}

impl Default for ParticleEmitter {
    /// A DORMANT emitter: no continuous rate, no burst, unit speed. An emitter auto-inserted by a
    /// bundle therefore spawns nothing until the author sets a rate — the same
    /// absence-is-the-safe-default discipline the config's `Off` follows.
    #[inline]
    fn default() -> Self {
        Self { rate: 0.0, accumulator: 0.0, burst: 0, speed_scale: 1.0 }
    }
}

/// The O(1) per-frame emitter toggle — an `EnableTag` bitset tag (the
/// [`RenderEnabled`](boyko_scene::render_caps::RenderEnabled) shape).
///
/// A bitset tag has NO `ComponentPool` and is NOT part of any archetype signature, so
/// enabling/disabling it is O(1) with no archetype migration, no structural generation bump and no
/// per-row bytes. `particle_tick_emitters` filters on `Enabled<EmitterActive>`, so a row with the
/// bit CLEAR is skipped branch-free at iteration.
///
/// ⚠️ `Added`/`Changed` filters over a bitset tag are compile-REJECTED by the kernel: the bit is
/// the whole datum and it carries no tick. Use the enable/disable pair, not change detection.
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct EmitterActive;

/// An index into the world's `Assets<ParticleEffect>` table — the emitter's effect binding.
///
/// `#[repr(transparent)]` so the column is byte-identical to its `u32` (the emit-request packer
/// reads the raw index straight into [`EmitRequestGpu::effect_index`]).
///
/// # Refcount lifetime hooks
///
/// `on_insert` ([`effect_on_insert`]) pushes `+1` for the NEW slot; `on_replace`
/// ([`effect_on_replace`]) pushes `-1` for the OLD slot, into [`ParticleEffectRefs`]. `on_remove`
/// is deliberately NOT also wired, for the reason `boyko_scene::render_caps` records in full:
/// `on_replace` already fires exactly once per value-departure event (in-place overwrite AND
/// genuine removal/despawn), so wiring both would double-decrement every genuine removal whenever
/// the slot's refcount is still `> 1` — the common shared-effect case.
///
/// The deltas land in a SUBSYSTEM-OWNED queue rather than `boyko_scene::RefcountDeltas` — see
/// [`ParticleEffectRefs`]'s doc for why that is a containment requirement, not a preference.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[component(on_insert = effect_on_insert, on_replace = effect_on_replace)]
pub struct ParticleEffectHandle(pub u32);

// Layout pins (house style): the emit packer reads these widths, and a silent layout drift (an
// added field, a wider carrier) must fail the build rather than corrupt the device table.
const _: () = assert!(size_of::<ParticleEmitter>() == 16 && align_of::<ParticleEmitter>() == 4);
const _: () = assert!(size_of::<ParticleEffectHandle>() == 4);
const _: () = assert!(size_of::<EmitterActive>() == 0);

// ── The subsystem-owned refcount queue ───────────────────────────────────────────────

/// One refcount delta pushed by a [`ParticleEffectHandle`] hook. POD, `Copy`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleEffectRefDelta {
    /// The carrier entity whose hook pushed this delta. Recorded for diagnostics and for symmetry
    /// with `boyko_scene::RefDelta`; on a despawn the entity is already gone by apply time, which
    /// does not invalidate the delta.
    pub entity: Entity,
    /// The dense `Assets<ParticleEffect>` row the delta targets (the carrier's raw index).
    pub slot: u32,
    /// `+1` on attach (`on_insert`), `-1` on detach (`on_replace`).
    pub delta: i32,
}

/// World-global queue of [`ParticleEffectRefDelta`]s awaiting
/// [`particle_apply_effect_refs`](crate::particle_system::particle_apply_effect_refs).
///
/// # Why a subsystem-owned queue and not `boyko_scene::RefcountDeltas`
///
/// `RefcountDeltas` routes by `boyko_scene::AssetRefKind`, whose variants are `Mesh` and
/// `Material`. Widening that enum would force `boyko_render::apply_refcount_deltas` — a system
/// every world runs, particles or not — to acquire `Assets<ParticleEffect>` as a system param,
/// which only [`ParticlePlugin`](crate::particle_plugin::ParticlePlugin) inserts. An
/// always-scheduled system would then depend on a resource that exists only when an OPTIONAL
/// plugin is installed: precisely the cross-subsystem coupling plan invariant 3 (D17) forbids, and
/// the reason the plan gave this subsystem its own clock. The queue therefore lives here, drains
/// here, and is observable by nothing outside the subsystem.
///
/// # Principle 0
///
/// `ScratchColumn`-backed, not `std::Vec`: the same `ComponentPool`-backed, address-stable,
/// VM-native primitive every other per-frame staging lane in this crate uses. `clear()`ed and
/// re-filled, never reallocated.
#[derive(Resource)]
pub struct ParticleEffectRefs {
    /// The queued deltas, in push (FIFO) order.
    deltas: ScratchColumn<ParticleEffectRefDelta>,
}

impl Default for ParticleEffectRefs {
    /// Registers ONE `ComponentId` for the delta element type (memoized process-wide) and sizes
    /// the lane at [`pool_reserve_rows`] — address space only, lazy commit, so an unused queue
    /// costs an unbacked VA reservation and zero committed pages (the
    /// [`MeshRenderScratch`](crate::mesh_draw::MeshRenderScratch) construction idiom).
    fn default() -> Self {
        let id = register_asset_layout::<ParticleEffectRefDelta>(None);
        let rows = pool_reserve_rows(size_of::<ParticleEffectRefDelta>());
        Self { deltas: ScratchColumn::new(id, rows) }
    }
}

impl ParticleEffectRefs {
    /// Queues a delta. Called ONLY from a [`ParticleEffectHandle`] hook body.
    #[inline]
    pub fn push(&mut self, delta: ParticleEffectRefDelta) {
        self.deltas.build_view().push(delta);
    }

    /// Every currently-queued delta, in push (FIFO) order.
    #[inline]
    pub fn queued(&self) -> &[ParticleEffectRefDelta] {
        self.deltas.as_read_slice()
    }

    /// `true` if no delta is awaiting the apply system — the O(1) early-out a non-churning
    /// (golden) scene takes on every frame.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    /// Drops every queued delta WITHOUT freeing the backing reservation (the reuse contract).
    /// Called by the apply system after it has folded them in.
    #[inline]
    pub fn clear(&mut self) {
        self.deltas.build_view().clear();
    }
}

// ── The carrier hooks ────────────────────────────────────────────────────────────────
//
// Declared `unsafe fn` only to match the kernel's `HookFn` signature; each body calls ONLY the
// safe `get_component` / `resource_mut` — no `unsafe` block inside.

/// `ParticleEffectHandle::on_insert`: the carrier just attached (a fresh add, or the NEW value of
/// an in-place replace) — pushes `+1` for the NEW slot.
///
/// # Safety
///
/// The caller is always a `trigger_on_insert` dispatch firing synchronously under the outermost
/// apply's `&mut EcsMaster` (the single-threaded apply window, POST the row write — so
/// `get_component` reads the NEW value). `resource_mut` returns a `&mut ParticleEffectRefs` into
/// resource storage, disjoint from every archetype/pool buffer, so this never aliases the apply's
/// component reborrows — the canonical `on_insert`-mutates-a-resource pattern (mirrors
/// `boyko_scene::render_caps::mesh_handle_on_insert`).
unsafe fn effect_on_insert(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&ParticleEffectHandle(slot)) = dm.get_component::<ParticleEffectHandle>(ctx.entity)
    else {
        return;
    };
    if let Some(refs) = dm.resource_mut::<ParticleEffectRefs>() {
        refs.push(ParticleEffectRefDelta { entity: ctx.entity, slot, delta: 1 });
    }
}

/// `ParticleEffectHandle::on_replace`: the carrier's current value is about to depart (an in-place
/// overwrite OR a genuine removal/despawn) — pushes `-1` for the OLD (still-live, pre-overwrite)
/// slot.
///
/// # Why no bind-generation is captured
///
/// `boyko_scene`'s mesh/material carriers capture a `*RefGen` lane here so `dec_ref`'s gen-check
/// can suppress a stale decrement against a slot that was retired and REUSED underneath the
/// carrier. That hazard needs slot reuse, and the effect table is APPEND-ONLY at P0 (the plan's
/// edge-case list states it, and every effect slot is pinned at mint by
/// [`ParticleEffectsExt`](crate::particle_effect::ParticleEffectsExt), so a refcount zero-crossing
/// leaves the row `Loaded` and never frees it). The delta therefore carries `GEN_UNSYNCED`, which
/// `dec_ref` documents as the explicit bypass, and no sibling generation lane exists to go stale.
/// A future rung that retires effect rows must add that lane BEFORE it does.
///
/// # Safety
///
/// Same contract as [`effect_on_insert`], reading the row PRE-overwrite (the kernel's `on_replace`
/// dispatch point) — `get_component` therefore resolves the OLD/dying value, not the incoming one.
unsafe fn effect_on_replace(mut dm: DeferredEcsMaster<'_>, ctx: HookContext) {
    let Some(&ParticleEffectHandle(slot)) = dm.get_component::<ParticleEffectHandle>(ctx.entity)
    else {
        return;
    };
    if let Some(refs) = dm.resource_mut::<ParticleEffectRefs>() {
        refs.push(ParticleEffectRefDelta { entity: ctx.entity, slot, delta: -1 });
    }
}

/// The generation value a particle-effect `-1` delta carries — the documented `dec_ref` bypass,
/// re-exported here so the apply system does not have to reach into `boyko_ecs`'s asset module for
/// a constant whose MEANING is local ("this subsystem's effect table is append-only").
pub const PARTICLE_EFFECT_REF_GEN: u32 = GEN_UNSYNCED;

// ── GPU POD records ──────────────────────────────────────────────────────────────────

/// The sim's working set — one contiguous AoS record, one fully-consumed 64 B line per particle
/// under the alive-list gather (D2). Field order follows Hanabi's packer.
///
/// Written by `particle_emit` (init) and `particle_sim` (step); **one lane owns a slot**, so a
/// cross-lane read-modify-write of any attribute is structurally impossible.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ParticleSim {
    /// World position.
    pub position: [f32; 3],
    /// Seconds of life left; `<= 0` retires the particle in the sim's liveness test.
    pub life_remaining: f32,
    /// World velocity.
    pub velocity: [f32; 3],
    /// Rung P1's Lipschitz distance cache for the SDF-collision skip. **Zero at P0** — the field
    /// exists now because adding it later would move every offset below it and re-pin every
    /// generated shader.
    pub cached_field_d: f32,
    /// Current colour, `RGBA8` packed.
    ///
    /// ⚠️ **Spawn-passthrough at P0.** The shipped shaders carry
    /// [`EffectParamsGpu::color_keys`]`[0]` here unresolved for the particle's whole life; the
    /// 4-key RGBA8 colour blend is a named post-P0 item. The device's curve evaluation is
    /// instantiated for the SIZE ramp only.
    pub color_rgba8: u32,
    /// `f16 size0 | f16 inv_life_total` — the spawn size and the reciprocal total lifetime the
    /// curve evaluation needs, in one word.
    pub size0_invlife: u32,
    /// `u16 effect_index | u16 flags`; the flags half carries the blend class
    /// ([`PARTICLE_BLEND_ADDITIVE`] / [`PARTICLE_BLEND_ALPHA`]).
    pub effect_flags: u32,
    /// `snorm16 cos | snorm16 sin` — the billboard rotation as a stored unit pair, advanced by a
    /// complex multiply against the effect's host-precomputed multiplier. No trig on the device,
    /// and no renormalization: a divide inside a leaf would drag `OpFDiv`'s 2.5 ULP into that
    /// leaf's bit-exact oracle (M7).
    pub rot_cs: u32,
}

/// The draw's working set (D2) — written by the sim at the CLASS-DENSE render index, read by the
/// VS at `pc.index_base + pc.index_step * SV_InstanceID` (at P0 the identity: `(0, +1)`).
///
/// Separate from [`ParticleSim`] because the draw reads a dense sequential 32 B stream with no
/// gather, and — the larger win — because every curve/ramp evaluation (and rung P3's lighting)
/// then happens ONCE per particle in the sim instead of four times per particle in the VS.
///
/// Rung P2b grows this to 40 B by adding a packed velocity lane, as a COMPILE-TIME variant
/// (`-D PARTICLE_INTERP`): a runtime flag over an always-40 B record would pay +25 % of the draw's
/// read traffic while the feature is OFF — the dark-tax class this plan cites everywhere else.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ParticleRender {
    /// World position of the billboard centre.
    pub position: [f32; 3],
    /// World-space billboard half-extent, SIZE-ramp-resolved in the sim — the one ramp the shipped
    /// P0 shaders evaluate, over [`EffectParamsGpu::size_keys`]' four `f16` keys at UNIFORM key
    /// times.
    pub size: f32,
    /// `RGBA8` colour.
    ///
    /// ⚠️ **Spawn-passthrough at P0**, copied from [`ParticleSim::color_rgba8`]: the 4-key colour
    /// blend is a named post-P0 item and rung P3's light resolve is later still. Stated here
    /// because "ramp-resolved" would be a promise no shipped shader keeps.
    pub color_rgba8: u32,
    /// `snorm16 cos | snorm16 sin` — the rotation the VS multiplies the corner offset by.
    pub rot_cs: u32,
    /// Bindless texture index. This is what makes ONE draw cover EVERY effect: there is no
    /// per-effect batch key, which closes the "many tiny emitters" pitfall structurally.
    pub tex_index: u32,
    /// Per-particle render flags (soft-particle opt-in, blend class, …).
    pub flags: u32,
}

/// The bookkeeping cache line (plan §Counter and list ownership, NORMATIVE).
///
/// **NO `ping` field** — the host owns parity, selecting `sets[parity]` from its own monotonic
/// frame counter; a device-side parity word would be the same value living in two places.
/// **NO `alpha_count` field** — the two RENDER counters live in [`ParticleDrawArgs`], because the
/// list counter and the render counters are different quantities with incompatible
/// synchronisation needs (the list count must be AVAILABLE to next frame's kickoff; the render
/// counts are an indirect fetch that this frame's writes must WAR against, and one `ResSync` state
/// cannot express both). Both absences are pinned by `counters_have_no_parity_field` /
/// `counters_have_no_alpha_count_field` below.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ParticleCounters {
    /// Live particles to walk THIS frame — written by kickoff only (`= alive_count_next`, then
    /// `+= real_emit_count`). The sim's guard bound and kickoff's dispatch size are the SAME
    /// field, read twice, never two derivations that must be kept in agreement.
    pub alive_count_cur: u32,
    /// The LIST counter — written by the sim only, shared by BOTH blend classes. Every survivor of
    /// either class takes its list index from here, which is what prevents an alpha survivor from
    /// leaking: kickoff reads only this field, so a class that allocated its list index elsewhere
    /// would vanish from the next frame's walk entirely.
    pub alive_count_next: u32,
    /// Free-list depth. Kickoff pre-DECREMENTS it to reserve the emit window; the sim pushes
    /// retired slots back. Different passes with a derived barrier between them, so the classic
    /// concurrent push/pop on a dead stack is structurally impossible.
    pub dead_count: u32,
    /// Base of the reserved free-list window `p_dead[dead_base .. dead_base + real_emit_count)` —
    /// kickoff only.
    pub dead_base: u32,
    /// Base of the reserved alive-list window — kickoff only. Emit lane `gid` writes
    /// `p_alive_read[emit_append_base + gid]`, an arithmetic index needing ZERO atomics.
    pub emit_append_base: u32,
    /// `min(requested_spawn, dead_count)` — kickoff only; the emit dispatch is sized from it.
    pub real_emit_count: u32,
    /// Diagnostic: spawns refused because the pool was full, accumulated by kickoff. Read by the
    /// host on a cold path only.
    pub clamped_spawns: u32,
    /// **Rung P1b's skip-rate instrument, word 7.** Wave-substeps in which AT LEAST ONE lane needed
    /// the field — i.e. the whole wave paid the edit-list walk, because the skip is a divergent
    /// branch and a wave executes both of its sides.
    ///
    /// Written ONLY by the `-D SDF_COLLIDE_STATS` sim module, which is the third committed sim
    /// artifact and is selected by [`ParticleCollision::SdfStats`](crate::ParticleCollision) alone.
    /// The two shipping modules never name this word, so on every shipping configuration it holds
    /// the boot zero — which is what makes the instrument free rather than a dark tax (F24).
    ///
    /// ⚠️ **Accumulates across frames and is never reset — so it WRAPS.** `particle_kickoff` is one
    /// module for all three sim variants and does not clear it, so the value is a running total
    /// from boot. That is deliberate: the quantity of interest is the RATIO
    /// [`waves_skipped`](Self::waves_skipped)`/(skipped + evaluated)`, which is frame-count
    /// independent, and a per-frame reset would need a fourth writer in a shipping shader.
    ///
    /// **All three stats words wrap, not only [`lanes_evaluated`](Self::lanes_evaluated)** — that
    /// one merely wraps ~`W` times sooner, because it counts lanes where these count waves. The
    /// wave pair grows by `ceil(alive / W) × steps` per frame, so 1 M particles at 64 substeps
    /// wraps `u32` after ~2 100 frames; `lanes_evaluated` wraps after ~67. `particle_counters_
    /// readback` derives the run's own ceiling and refuses to believe counters that could have
    /// wrapped, which is the only detection available from a single sample.
    pub waves_evaluated: u32,
    /// **Rung P1b's skip-rate instrument, word 8.** Wave-substeps in which NO lane needed the field
    /// — the ones where the Lipschitz cache actually saved the walk.
    ///
    /// Exclusive with [`waves_evaluated`](Self::waves_evaluated) by construction: exactly one of the
    /// two increments per wave per substep, so their sum IS the wave-substep count and the skip rate
    /// needs no separate denominator. Same accumulation and same writer as its sibling.
    pub waves_skipped: u32,
    /// **Rung P1b's skip-rate instrument, word 9.** LANES that needed the field, summed over every
    /// wave-substep — the per-lane numerator the wave-coherence argument predicts will overstate the
    /// saving.
    ///
    /// The pair is the point: `lanes_evaluated` divided by the lane-substep count is the rate a
    /// naive per-lane counter would report, and the gap between that and the wave rate IS the wave's
    /// incoherence. Same accumulation and same writer as its siblings.
    ///
    /// ⚠️ It is the FIRST of the three to wrap — not the only one (see
    /// [`waves_evaluated`](Self::waves_evaluated)). It grows by up to `alive × steps` per frame, so
    /// a 1 M-particle run at 64 substeps wraps `u32` after ~67 frames, roughly `W` times sooner than
    /// its wave-counting siblings. The measurement fixtures run 30 frames at one substep.
    pub lanes_evaluated: u32,
    /// Padding to the full cache line. Present so the struct's size is a layout PIN rather than a
    /// consequence of the field list.
    pub _pad: [u32; 6],
}

/// The two `VkDispatchIndirectCommand`s (D4) — written ONLY by kickoff, read ONLY by
/// `DRAW_INDIRECT`.
///
/// Split away from [`ParticleDrawArgs`] because a fused block would take both a `DRAW_INDIRECT`
/// read and a `COMPUTE` write on ONE `ResId` inside the sim pass, and the framegraph has no
/// sub-buffer granularity. The split also buys this buffer a barrier-free second read.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ParticleDispatchArgs {
    /// `ceil(real_emit_count / 256)` groups for `particle_emit`.
    pub emit: [u32; 3],
    /// Padding to the 16 B `VkDispatchIndirectCommand` slot stride.
    pub _p0: u32,
    /// `ceil(alive_count_cur / 256)` groups for `particle_sim`.
    pub sim: [u32; 3],
    /// Padding to the 16 B slot stride.
    pub _p1: u32,
}

/// The host's `#[repr(C)]` mirror of Vulkan's `VkDrawIndexedIndirectCommand` (20 B).
///
/// Defined HERE rather than imported: the real FFI struct lives in `boyko_rhi_vulkan`'s `ffi`
/// module, and `boyko_render` deliberately does not take a dependency edge on it for a
/// twenty-byte plain-old-data record. The `boyko_rhi_vulkan` integration asserts the two are
/// equivalent (size, alignment and every field offset) at its own boundary, so a drift in either
/// direction fails a build rather than a frame.
///
/// ⚠️ **`first_instance` MUST be 0 on this device** — a nonzero value there is a silent
/// corruption class, which is exactly why rung P2's two blend classes are distinguished by a
/// push-constant index transform and never by `firstInstance`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct VkDrawIndexedIndirectCommandMirror {
    /// Indices per instance — 6 for the two-triangle billboard quad.
    pub index_count: u32,
    /// Live instances. **Also this class's RENDER COUNTER**: the sim's returning
    /// `InterlockedAdd` on this word yields each wave's dense render position, and the final value
    /// IS the count — which closes the "render capacity instead of live count" pitfall by
    /// construction, with no fourth "finish" pass.
    pub instance_count: u32,
    /// First index into the 6-entry `u16` index buffer. Always 0.
    pub first_index: u32,
    /// Vertex offset. Always 0.
    pub vertex_offset: i32,
    /// **Always 0** — see the type doc.
    pub first_instance: u32,
}

/// TWO `VkDrawIndexedIndirectCommand`s in one 64 B block (D4/D10): additive at byte 0, alpha at
/// byte 24.
///
/// Each `instance_count` is ALSO its class's render counter (see
/// [`VkDrawIndexedIndirectCommandMirror::instance_count`]). At P0 only the additive slot is live;
/// the alpha slot is zeroed and its draw is not declared.
///
/// # The padding arithmetic (a plan erratum, closed here)
///
/// The plan writes the tail as one `_pad: [u32; 6]` AFTER `alpha`. Taken literally that is 68 B,
/// not the 64 B the same paragraph states: two 20 B commands placed at 0 and 24 end at byte 44,
/// and `44 + 6·4 == 68`. The six words are therefore split here into the ONE that opens the 4 B gap
/// putting `alpha` at byte 24 (`#[repr(C)]` would otherwise pack it at 20, moving its
/// `instance_count` to 24 and breaking the pinned 28) and the FIVE that fill 44 → 64. Same six
/// words, correct total, and each one's job is nameable. The size pin below says 64.
///
/// The generated shaders are agnostic to this: they address only bytes 0..48 and derive their word
/// indices from [`PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET`] /
/// [`PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET`] as generator inputs rather than typing them.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct ParticleDrawArgs {
    /// The additive draw — the only one P0 records.
    pub additive: VkDrawIndexedIndirectCommandMirror,
    /// Places [`alpha`](Self::alpha) at byte 24 (the plan's offset), so
    /// [`PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET`] is 28.
    pub _gap: u32,
    /// The alpha draw (rung P2). Zeroed at P0, and its pass is not declared.
    pub alpha: VkDrawIndexedIndirectCommandMirror,
    /// Fills the block to 64 B.
    pub _pad: [u32; 5],
}

impl ParticleDrawArgs {
    /// The boot/kickoff value: a 6-index quad, ZERO instances in both slots, and
    /// `first_instance == 0` in both — the shape every frame's kickoff rewrites.
    ///
    /// `instance_count` starts at 0 because it is a COUNTER the sim accumulates into, not a size
    /// the host predicts.
    #[inline]
    pub const fn quad_zeroed() -> Self {
        const SLOT: VkDrawIndexedIndirectCommandMirror = VkDrawIndexedIndirectCommandMirror {
            index_count: PARTICLE_QUAD_INDEX_COUNT,
            instance_count: 0,
            first_index: 0,
            vertex_offset: 0,
            first_instance: 0,
        };
        Self { additive: SLOT, _gap: 0, alpha: SLOT, _pad: [0; 5] }
    }
}

/// Indices in the billboard quad's 12-byte `u16` index buffer — two triangles.
///
/// `vkCmdDrawIndirect` (non-indexed) is not loaded on this device, so the draw is INDEXED and this
/// tiny buffer is uploaded once at boot and never rewritten.
pub const PARTICLE_QUAD_INDEX_COUNT: u32 = 6;

/// One per-emitter spawn request, uploaded to the device each frame (≤ 16 KB at
/// [`MAX_EMITTERS`], and **0 B on a frame with no spawns** — the upload is gated on
/// `total_spawn > 0`).
///
/// The basis is the emitter's world axes, so the device samples the spawn cone in emitter space
/// without ever seeing a matrix.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EmitRequestGpu {
    /// World-space spawn origin (the emitter's `GlobalTransform` translation).
    pub origin: [f32; 3],
    /// Row index into the effect-parameter table.
    pub effect_index: u32,
    /// The emitter's world +X axis.
    pub basis_x: [f32; 3],
    /// Particles this emitter asks for THIS frame.
    pub spawn_count: u32,
    /// The emitter's world +Y axis.
    pub basis_y: [f32; 3],
    /// This emitter's start in the frame's global spawn numbering — the CPU prefix sum. It orders
    /// LANES only (D8): the slot comes from the free list and the list position from
    /// `emit_append_base + gid`, three independent indexings none of which assumes structure in
    /// another.
    pub first_spawn: u32,
    /// The emitter's world +Z axis.
    pub basis_z: [f32; 3],
    /// A per-emitter CONSTANT seed. The frame number enters the device RNG through a push
    /// constant, so this word does not have to be rewritten to decorrelate successive frames.
    pub rng_seed: u32,
}

/// One effect's device-side parameters (D6) — the host-precomputed form of
/// [`ParticleEffect`](crate::particle_effect::ParticleEffect).
///
/// Everything the device would otherwise need `exp2` or trig for is baked here against the
/// clock's CONSTANT timestep: [`damping`](Self::damping) and the
/// [`rot_mul_cos`](Self::rot_mul_cos)/[`rot_mul_sin`](Self::rot_mul_sin) pair. That is what lets
/// every particle leaf stay pure multiply/add and therefore bit-exact against its host oracle.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EffectParamsGpu {
    /// World gravity applied per substep.
    pub gravity: [f32; 3],
    /// `exp2(-drag · timestep)` — the per-substep velocity multiplier, HOST-computed, which is
    /// what deletes `exp2` from the device.
    pub damping: f32,
    /// `cos(ω · timestep)` — the real half of the per-substep rotation multiplier.
    ///
    /// **f32, deliberately NOT snorm16.** A quantized multiplier's magnitude error is a PER-EFFECT
    /// CONSTANT and compounds geometrically: at `δ ~ 2×10⁻⁵`, `(1+δ)⁶⁴⁰ ≈ ±1 %`, coherent across
    /// every particle of the effect. At f32, `|δ| ≤ 1 ULP ≈ 6×10⁻⁸` ⇒ `(1+δ)⁶⁴⁰ − 1 ≈ 4×10⁻⁵`,
    /// below the per-particle record's own snorm16 precision. Zero GPU cost — the struct had the
    /// padding.
    pub rot_mul_cos: f32,
    /// `sin(ω · timestep)` — the imaginary half. Same f32 rationale as
    /// [`rot_mul_cos`](Self::rot_mul_cos).
    pub rot_mul_sin: f32,
    /// Reserved (the two words the f32 rotation pair was carved out of).
    pub _r0: [u32; 2],
    /// Four `RGBA8` colour-ramp keys.
    ///
    /// ⚠️ **At P0 only `color_keys[0]` is read**: emit copies it into
    /// [`ParticleSim::color_rgba8`] and the particle keeps it for its whole life
    /// (spawn-passthrough). Keys 1–3 are packed and uploaded for layout parity; the 4-key RGBA8
    /// blend is a named post-P0 item.
    pub color_keys: [u32; 4],
    /// Four `f16` normalized ramp times, packed two per word.
    ///
    /// ⚠️ **Layout parity only at P0 — read by no shader.** The colour ramp is not evaluated
    /// (see [`color_keys`](Self::color_keys)), and the SIZE ramp the shipped shaders DO evaluate
    /// uses UNIFORM key times rather than these. Kept in the layout so adding either consumer later
    /// moves no offset and re-pins no generated shader.
    pub color_times: [u32; 2],
    /// Four `f16` size-ramp keys, packed two per word — **the one ramp P0 evaluates on the
    /// device**, at uniform key times over the particle's normalized life.
    pub size_keys: [u32; 2],
    /// Minimum spawn lifetime, seconds.
    pub lifetime_min: f32,
    /// Maximum spawn lifetime, seconds.
    pub lifetime_max: f32,
    /// Minimum initial speed.
    pub speed_min: f32,
    /// Maximum initial speed.
    pub speed_max: f32,
    /// Base billboard size the size ramp multiplies.
    pub size_base: f32,
    /// `cos` of the spawn cone half-angle. Stored as a cosine so the device samples the cone with
    /// the concentric-disc form — `sqrt` only, no trig.
    pub cone_cos: f32,
    /// Reserved.
    pub _r1: f32,
    /// Reserved.
    pub _r2: f32,
    /// Bindless texture index copied into every particle's render record.
    pub tex_index: u32,
    /// [`PARTICLE_BLEND_ADDITIVE`] or [`PARTICLE_BLEND_ALPHA`].
    pub blend_class: u32,
    /// Effect-level feature bits.
    pub flags: u32,
    /// Rung P1 collision radius.
    pub collision_radius: f32,
    /// Rung P1 bounce restitution.
    pub restitution: f32,
    /// Rung P1 tangential friction.
    pub friction: f32,
    /// Spawn-volume discriminant (point / sphere / cone / box).
    pub emitter_shape: u32,
    /// Reserved, filling the 128 B row.
    pub _r3: u32,
}

// ── Offset pins (GENERATOR INPUTS — the shaders' word indices derive from these) ──────

/// Byte offset of the ADDITIVE draw's `instance_count` inside [`ParticleDrawArgs`] — **4**.
///
/// A generator input, never a typed literal: the sim shader's `InterlockedAdd` word index is
/// emitted from this const, so the host and the device cannot disagree about where the additive
/// render counter lives.
pub const PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET: u32 = (offset_of!(ParticleDrawArgs, additive)
    + offset_of!(VkDrawIndexedIndirectCommandMirror, instance_count))
    as u32;

/// Byte offset of the ALPHA draw's `instance_count` inside [`ParticleDrawArgs`] — **28**. Same
/// generator-input discipline as [`PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET`]; unused at P0.
pub const PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET: u32 = (offset_of!(ParticleDrawArgs, alpha)
    + offset_of!(VkDrawIndexedIndirectCommandMirror, instance_count))
    as u32;

// `ParticleSim` — 48 B, one fully-consumed 64 B line under the gather.
const _: () = assert!(size_of::<ParticleSim>() == 48 && align_of::<ParticleSim>() == 16);
const _: () = assert!(offset_of!(ParticleSim, position) == 0);
const _: () = assert!(offset_of!(ParticleSim, life_remaining) == 12);
const _: () = assert!(offset_of!(ParticleSim, velocity) == 16);
const _: () = assert!(offset_of!(ParticleSim, cached_field_d) == 28);
const _: () = assert!(offset_of!(ParticleSim, color_rgba8) == 32);
const _: () = assert!(offset_of!(ParticleSim, size0_invlife) == 36);
const _: () = assert!(offset_of!(ParticleSim, effect_flags) == 40);
const _: () = assert!(offset_of!(ParticleSim, rot_cs) == 44);

// `ParticleRender` — 32 B, sequential in the draw.
const _: () = assert!(size_of::<ParticleRender>() == 32 && align_of::<ParticleRender>() == 16);
const _: () = assert!(offset_of!(ParticleRender, position) == 0);
const _: () = assert!(offset_of!(ParticleRender, size) == 12);
const _: () = assert!(offset_of!(ParticleRender, color_rgba8) == 16);
const _: () = assert!(offset_of!(ParticleRender, rot_cs) == 20);
const _: () = assert!(offset_of!(ParticleRender, tex_index) == 24);
const _: () = assert!(offset_of!(ParticleRender, flags) == 28);

// `ParticleCounters` — exactly one 64 B cache line.
const _: () = assert!(size_of::<ParticleCounters>() == 64 && align_of::<ParticleCounters>() == 64);
const _: () = assert!(offset_of!(ParticleCounters, alive_count_cur) == 0);
const _: () = assert!(offset_of!(ParticleCounters, alive_count_next) == 4);
const _: () = assert!(offset_of!(ParticleCounters, dead_count) == 8);
const _: () = assert!(offset_of!(ParticleCounters, dead_base) == 12);
const _: () = assert!(offset_of!(ParticleCounters, emit_append_base) == 16);
const _: () = assert!(offset_of!(ParticleCounters, real_emit_count) == 20);
const _: () = assert!(offset_of!(ParticleCounters, clamped_spawns) == 24);
// Rung P1b's three stats words — GENERATOR INPUTS for the `-D SDF_COLLIDE_STATS` module's
// `CTR_WAVES_EVALUATED`/`CTR_WAVES_SKIPPED`/`CTR_LANES_EVALUATED`, whose word indices are these
// offsets divided by four (7, 8, 9). Carved out of the pad, so no field above them moved and the
// two shipping `.spv` are untouched by their existence.
const _: () = assert!(offset_of!(ParticleCounters, waves_evaluated) == 28);
const _: () = assert!(offset_of!(ParticleCounters, waves_skipped) == 32);
const _: () = assert!(offset_of!(ParticleCounters, lanes_evaluated) == 36);
const _: () = assert!(offset_of!(ParticleCounters, _pad) == 40);

// `ParticleDispatchArgs` — two 16 B `VkDispatchIndirectCommand` slots.
const _: () =
    assert!(size_of::<ParticleDispatchArgs>() == 32 && align_of::<ParticleDispatchArgs>() == 16);
const _: () = assert!(offset_of!(ParticleDispatchArgs, emit) == 0);
const _: () = assert!(offset_of!(ParticleDispatchArgs, sim) == 16);

// `VkDrawIndexedIndirectCommandMirror` — the 20 B Vulkan record, field for field.
const _: () = assert!(size_of::<VkDrawIndexedIndirectCommandMirror>() == 20);
const _: () = assert!(align_of::<VkDrawIndexedIndirectCommandMirror>() == 4);
const _: () = assert!(offset_of!(VkDrawIndexedIndirectCommandMirror, index_count) == 0);
const _: () = assert!(offset_of!(VkDrawIndexedIndirectCommandMirror, instance_count) == 4);
const _: () = assert!(offset_of!(VkDrawIndexedIndirectCommandMirror, first_index) == 8);
const _: () = assert!(offset_of!(VkDrawIndexedIndirectCommandMirror, vertex_offset) == 12);
const _: () = assert!(offset_of!(VkDrawIndexedIndirectCommandMirror, first_instance) == 16);

// `ParticleDrawArgs` — the two slots at 0 and 24; both counter offsets are multiples of 4.
const _: () = assert!(size_of::<ParticleDrawArgs>() == 64 && align_of::<ParticleDrawArgs>() == 16);
const _: () = assert!(offset_of!(ParticleDrawArgs, additive) == 0);
const _: () = assert!(offset_of!(ParticleDrawArgs, alpha) == 24);
const _: () = assert!(PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET == 4);
const _: () = assert!(PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET == 28);
const _: () = assert!(PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET.is_multiple_of(4));
const _: () = assert!(PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET.is_multiple_of(4));

// `EmitRequestGpu` — 64 B, four `float3 + uint` lanes.
const _: () = assert!(size_of::<EmitRequestGpu>() == 64 && align_of::<EmitRequestGpu>() == 16);
const _: () = assert!(offset_of!(EmitRequestGpu, origin) == 0);
const _: () = assert!(offset_of!(EmitRequestGpu, effect_index) == 12);
const _: () = assert!(offset_of!(EmitRequestGpu, basis_x) == 16);
const _: () = assert!(offset_of!(EmitRequestGpu, spawn_count) == 28);
const _: () = assert!(offset_of!(EmitRequestGpu, basis_y) == 32);
const _: () = assert!(offset_of!(EmitRequestGpu, first_spawn) == 44);
const _: () = assert!(offset_of!(EmitRequestGpu, basis_z) == 48);
const _: () = assert!(offset_of!(EmitRequestGpu, rng_seed) == 60);
// The whole table fits the plan's ≤ 16 KB per-frame host→device budget.
const _: () = assert!(size_of::<EmitRequestGpu>() * MAX_EMITTERS == 16_384);

// `EffectParamsGpu` — 128 B; the rotation multiplier is an f32 PAIR (K1), not a packed word.
const _: () = assert!(size_of::<EffectParamsGpu>() == 128 && align_of::<EffectParamsGpu>() == 16);
const _: () = assert!(offset_of!(EffectParamsGpu, gravity) == 0);
const _: () = assert!(offset_of!(EffectParamsGpu, damping) == 12);
const _: () = assert!(offset_of!(EffectParamsGpu, rot_mul_cos) == 16);
const _: () = assert!(offset_of!(EffectParamsGpu, rot_mul_sin) == 20);
const _: () = assert!(offset_of!(EffectParamsGpu, color_keys) == 32);
const _: () = assert!(offset_of!(EffectParamsGpu, color_times) == 48);
const _: () = assert!(offset_of!(EffectParamsGpu, size_keys) == 56);
const _: () = assert!(offset_of!(EffectParamsGpu, lifetime_min) == 64);
const _: () = assert!(offset_of!(EffectParamsGpu, size_base) == 80);
const _: () = assert!(offset_of!(EffectParamsGpu, tex_index) == 96);
const _: () = assert!(offset_of!(EffectParamsGpu, restitution) == 112);
const _: () = assert!(offset_of!(EffectParamsGpu, _r3) == 124);
// The whole effect table is 32 KB — effectively constant-resident in the sim's cache.
const _: () = assert!(size_of::<EffectParamsGpu>() * MAX_EFFECTS == 32_768);

// The plan's VRAM arithmetic: 48 + 32 + 4 (dead) + 2×4 (alive) == 92 B/particle.
const _: () = assert!(
    size_of::<ParticleSim>() + size_of::<ParticleRender>() + size_of::<u32>() + 2 * size_of::<u32>()
        == 92
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Gate #9 / M2, the COMPILE-TIME absence pins. An exhaustive destructuring with NO `..` rest
    /// pattern: adding any field to [`ParticleCounters`] — a `ping` parity word, an `alpha_count`
    /// render counter, anything — fails to COMPILE here rather than silently widening a struct
    /// whose size the device layout depends on.
    ///
    /// Both absences are load-bearing and were both defects in an earlier revision of the plan:
    /// a device-side `ping` would be the host's parity living in two places, and an `alpha_count`
    /// beside the list counter would be one counter trying to serve two consumers with
    /// incompatible synchronisation needs.
    ///
    /// Rung P1b's three stats words are listed here for the same reason the others are: they were
    /// CARVED OUT OF THE PAD, so the destructuring is what makes "they took pad, they did not move
    /// a live field" a compile-time statement rather than a claim about a diff.
    #[test]
    fn counters_have_no_parity_field_and_no_alpha_count_field() {
        let counters = ParticleCounters::default();
        let ParticleCounters {
            alive_count_cur,
            alive_count_next,
            dead_count,
            dead_base,
            emit_append_base,
            real_emit_count,
            clamped_spawns,
            waves_evaluated,
            waves_skipped,
            lanes_evaluated,
            _pad,
        } = counters;

        assert_eq!(alive_count_cur, 0);
        assert_eq!(alive_count_next, 0);
        assert_eq!(dead_count, 0);
        assert_eq!(dead_base, 0);
        assert_eq!(emit_append_base, 0);
        assert_eq!(real_emit_count, 0);
        assert_eq!(clamped_spawns, 0);
        assert_eq!(waves_evaluated, 0);
        assert_eq!(waves_skipped, 0);
        assert_eq!(lanes_evaluated, 0);
        assert_eq!(_pad, [0; 6]);

        // The ten live fields plus the pad account for the WHOLE cache line, so there is no room
        // for an eleventh counter even if the destructuring above were relaxed.
        assert_eq!(size_of::<ParticleCounters>(), 64);
        assert_eq!(offset_of!(ParticleCounters, _pad) + size_of::<[u32; 6]>(), 64);
    }

    /// Rung P1b: the three stats words sit at the word indices the `-D SDF_COLLIDE_STATS` generator
    /// spells, and they are DISJOINT from every shipping counter.
    ///
    /// Read back through raw bytes rather than through the field names the offsets were derived
    /// from — the same discipline `draw_args_offsets_and_first_instance_are_pinned` uses, and for
    /// the same reason: the shader addresses WORDS, so the pin has to be about words.
    #[test]
    fn the_stats_words_are_seven_eight_and_nine_and_collide_with_no_shipping_counter() {
        assert_eq!(offset_of!(ParticleCounters, waves_evaluated) / 4, 7);
        assert_eq!(offset_of!(ParticleCounters, waves_skipped) / 4, 8);
        assert_eq!(offset_of!(ParticleCounters, lanes_evaluated) / 4, 9);

        // Every word kickoff or the shipping sim writes is below 7, so the stats module cannot
        // perturb a shipping counter even though it shares the cache line with them.
        for shipping in [
            offset_of!(ParticleCounters, alive_count_cur),
            offset_of!(ParticleCounters, alive_count_next),
            offset_of!(ParticleCounters, dead_count),
            offset_of!(ParticleCounters, dead_base),
            offset_of!(ParticleCounters, emit_append_base),
            offset_of!(ParticleCounters, real_emit_count),
            offset_of!(ParticleCounters, clamped_spawns),
        ] {
            assert!(
                shipping / 4 < 7,
                "a shipping counter moved into the stats words' range ({shipping} B)"
            );
        }

        // A default (boot-zeroed) block reads zero at all three words: the shipping modules never
        // name them, so this is the value every non-stats run leaves behind.
        let zeroed = ParticleCounters::default();
        let bytes = bytemuck::bytes_of(&zeroed);
        for off in [28usize, 32, 36] {
            assert_eq!(u32::from_ne_bytes(bytes[off..][..4].try_into().unwrap()), 0);
        }
    }

    /// Gate #8: the two render-counter offsets, and `first_instance == 0` in BOTH draw slots
    /// (asserted host-side, because a nonzero value there is a silent corruption class this device
    /// gives no diagnostic for).
    #[test]
    fn draw_args_offsets_and_first_instance_are_pinned() {
        assert_eq!(PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET, 4);
        assert_eq!(PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET, 28);

        let args = ParticleDrawArgs::quad_zeroed();
        assert_eq!(args.additive.first_instance, 0, "first_instance MUST be 0 (additive slot)");
        assert_eq!(args.alpha.first_instance, 0, "first_instance MUST be 0 (alpha slot)");
        assert_eq!(args.additive.index_count, PARTICLE_QUAD_INDEX_COUNT);
        assert_eq!(args.alpha.index_count, PARTICLE_QUAD_INDEX_COUNT);
        assert_eq!(args.additive.instance_count, 0, "the render counter starts at zero");
        assert_eq!(args.alpha.instance_count, 0, "the alpha slot is zeroed at P0");

        // The offsets must address the words the shader will `InterlockedAdd`, so read them back
        // through raw bytes rather than through the field names the consts were built from.
        let bytes = bytemuck::bytes_of(&args);
        let additive_at = u32::from_ne_bytes(
            bytes[PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET as usize..][..4].try_into().unwrap(),
        );
        let alpha_at = u32::from_ne_bytes(
            bytes[PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET as usize..][..4].try_into().unwrap(),
        );
        assert_eq!(additive_at, 0);
        assert_eq!(alpha_at, 0);
    }

    /// The two offsets address DISJOINT words: a shared offset would make the two classes'
    /// `InterlockedAdd`s collide and silently merge their counts.
    #[test]
    fn the_two_render_counters_are_disjoint_words() {
        assert_ne!(PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET, PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET);
        const { assert!(PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET >= PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET + 4) };
        assert!((PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET as usize) + 4 <= size_of::<ParticleDrawArgs>());
    }

    /// The record sizes the plan's traffic table is derived from. A silent widening would invalidate
    /// every bandwidth number in it AND every device buffer's allocation size.
    #[test]
    fn gpu_record_sizes_match_the_plan() {
        assert_eq!(size_of::<ParticleSim>(), 48);
        assert_eq!(size_of::<ParticleRender>(), 32);
        assert_eq!(size_of::<ParticleCounters>(), 64);
        assert_eq!(size_of::<ParticleDispatchArgs>(), 32);
        assert_eq!(size_of::<ParticleDrawArgs>(), 64);
        assert_eq!(size_of::<EmitRequestGpu>(), 64);
        assert_eq!(size_of::<EffectParamsGpu>(), 128);
        assert_eq!(size_of::<VkDrawIndexedIndirectCommandMirror>(), 20);
    }

    /// The refcount queue: push preserves FIFO order, `clear` empties it without dropping the
    /// backing reservation, and an empty queue is the O(1) early-out a non-churning scene takes.
    #[test]
    fn effect_refs_push_then_read_yields_fifo_order() {
        use boyko_ecs::ecs::identifiers::primitives::EntityId;

        let e = Entity::new(EntityId(0), 0);
        let mut refs = ParticleEffectRefs::default();
        assert!(refs.is_empty());

        refs.push(ParticleEffectRefDelta { entity: e, slot: 1, delta: 1 });
        refs.push(ParticleEffectRefDelta { entity: e, slot: 2, delta: -1 });

        let queued = refs.queued();
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].slot, 1, "push order is FIFO");
        assert_eq!(queued[1].slot, 2);
        assert_eq!(queued[0].delta, 1);
        assert_eq!(queued[1].delta, -1);

        refs.clear();
        assert!(refs.is_empty(), "clear must empty the queue");
    }

    /// A default emitter is DORMANT: an auto-inserted row must not start spawning on its own.
    #[test]
    fn a_default_emitter_spawns_nothing() {
        let e = ParticleEmitter::default();
        assert_eq!(e.rate, 0.0);
        assert_eq!(e.accumulator, 0.0);
        assert_eq!(e.burst, 0);
        assert_eq!(e.speed_scale, 1.0, "unit speed scale, not zero — a zero would freeze spawns");
    }
}
