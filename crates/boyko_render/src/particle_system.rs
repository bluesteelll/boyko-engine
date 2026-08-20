//! Particles P0 — the CPU half: the per-frame emitter fold (A1), the effect-table bake, and the
//! refcount fold.
//!
//! # A1 is SEQUENTIAL by design, and that is a measurement, not a concession
//!
//! `particle_tick_emitters` walks ≤ [`MAX_EMITTERS`] rows in one thread: 256 rows × ~40 ns ≈ 10 µs,
//! where a fork/join costs more than the work. Sequential is also what lets each emitter write its
//! own [`ScratchColumn`] row behind a PLAIN counter — **zero CPU atomics anywhere in this
//! subsystem** (no `Mutex`, no `RwLock`, no `Rc`, no `RefCell` either).
//!
//! # Principle 0
//!
//! Both staging lanes are `ComponentPool`-backed
//! [`ScratchColumn`](boyko_ecs::ecs::core::component::scratch::ScratchColumn)s built through the
//! [`MeshRenderScratch`](crate::mesh_draw::MeshRenderScratch) idiom
//! (`register_asset_layout::<T>(None)` + [`pool_reserve_rows`]), never `std::Vec`. An unused lane
//! costs an unbacked VA reservation and zero committed pages, and a steady-state frame `clear()`s
//! and re-fills in place — zero allocations per frame, which is one of the plan's own metric rows.
//!
//! # Where the device's push constants come from (one home each)
//!
//! The three compute passes take small push constants, and every field of them is READ FROM a
//! public accessor on this subsystem's own Resources rather than recomputed at the recording site.
//! That is the plan's "one number, two consumers" rule applied to the host↔device seam:
//!
//! | Pass | Push field | Its ONE home |
//! |---|---|---|
//! | kickoff | `requested_spawn` | [`ParticleEmitScratch::total_spawn`] |
//! | kickoff | `capacity` | [`ParticleConfig::capacity`](crate::particle_config::ParticleConfig::capacity) |
//! | emit | `emitter_count` | [`ParticleEmitScratch::emitter_count`] |
//! | emit | `frame_index` | the host's monotonic frame counter (it also selects `sets[parity]`) |
//! | sim | `steps` | [`ParticleClock::steps`] — already ceiling-clamped on the host (M3) |
//! | sim | `timestep` | [`ParticleClock::timestep`] |
//!
//! `total_spawn` is doing double duty on purpose: it is both the pushed `requested_spawn` and the
//! `> 0` upload/declare gate, so the value the device is told and the predicate that decides
//! whether the device is told anything cannot disagree.
//!
//! # D15: the fixed-size CPU-facing table gets a RELEASE-present clamp
//!
//! Hanabi shipped a 12 B indirect overrun at ~260 instances because a GPU table was sized from a
//! constant. This device runs with `robustBufferAccess` OFF, so an out-of-range fetch is undefined
//! behaviour rather than a clamp — which makes a `debug_assert!` alone insufficient by
//! construction. Both fixed-size tables here therefore clamp in RELEASE, count what they dropped,
//! and carry the `debug_assert!` alongside for the developer build.

use boyko_ecs::ecs::constants::pool_reserve_rows;
use boyko_ecs::ecs::core::asset::{Assets, register_asset_layout};
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::Time;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Resource;
use boyko_scene::transform::GlobalTransform;
use bytemuck::Zeroable;

use crate::particle::{
    EffectParamsGpu, EmitRequestGpu, EmitterActive, MAX_EFFECTS, MAX_EMITTERS,
    PARTICLE_EFFECT_REF_GEN, ParticleEffectHandle, ParticleEffectRefs, ParticleEmitter,
};
use crate::particle_clock::ParticleClock;
use crate::particle_effect::{ParticleEffect, pack_effect_params};

// ── The per-frame emit staging ───────────────────────────────────────────────────────

/// The per-frame emit-request staging lane (Principle 0: `ScratchColumn`, never `std::Vec`).
///
/// One writer ([`particle_tick_emitters`]) and one reader (the host's gated upload, on the
/// dispatcher after `Schedule::run` has returned with zero workers in flight). `ResMut` serialises
/// the writer; the reader needs no lock because it runs when nothing else can touch the world.
#[derive(Resource)]
pub struct ParticleEmitScratch {
    /// This frame's requests, one per ENABLED emitter, in query order. `first_spawn` is the
    /// running prefix sum, assigned by [`push_request`](Self::push_request) so the prefix has
    /// exactly one home.
    requests: ScratchColumn<EmitRequestGpu>,
    /// The frame's total spawn count — the running prefix after the last row.
    ///
    /// **This is the upload gate.** `total_spawn == 0` ⇒ zero bytes cross PCIe AND the emit pass
    /// is not declared; the two are the SAME predicate read at both sites, which is what makes the
    /// "written but unread this frame" state (a wrong reader seed on the request buffer)
    /// unconstructible.
    total_spawn: u32,
    /// Emitters DROPPED by the [`MAX_EMITTERS`] release clamp this frame (D15).
    dropped_emitters: u32,
    /// Spawns lost with those emitters (their `spawn_count` sum).
    ///
    /// ⚠️ NOT the same quantity as
    /// [`ParticleCounters::clamped_spawns`](crate::particle::ParticleCounters::clamped_spawns),
    /// which counts spawns the GPU kickoff refused because the POOL was full. This one counts
    /// spawns that never reached the device at all because the EMITTER TABLE was full. Two clamps,
    /// two causes, two counters — deliberately not one name.
    dropped_spawns: u32,
}

impl Default for ParticleEmitScratch {
    /// Registers ONE `ComponentId` for [`EmitRequestGpu`] (memoized process-wide) and sizes the
    /// lane at [`pool_reserve_rows`] — the `MeshRenderScratch::default` idiom, so this stays a
    /// valid zero-argument constructor callable from `insert_resource(..)`.
    fn default() -> Self {
        let id = register_asset_layout::<EmitRequestGpu>(None);
        let rows = pool_reserve_rows(size_of::<EmitRequestGpu>());
        Self {
            requests: ScratchColumn::new(id, rows),
            total_spawn: 0,
            dropped_emitters: 0,
            dropped_spawns: 0,
        }
    }
}

impl ParticleEmitScratch {
    /// Clears the lane and every per-frame counter, keeping the backing reservation (the reuse
    /// contract).
    ///
    /// Called unconditionally at the top of [`particle_tick_emitters`], BEFORE any early-out: a
    /// persistent `Resource` must never let a previous frame's rows survive into a frame that
    /// wrote none, which would upload stale spawn requests.
    #[inline]
    pub fn begin_frame(&mut self) {
        self.requests.build_view().clear();
        self.total_spawn = 0;
        self.dropped_emitters = 0;
        self.dropped_spawns = 0;
    }

    /// Appends one emitter's request, stamping its `first_spawn` from the running prefix and
    /// advancing that prefix by `request.spawn_count`.
    ///
    /// `request.first_spawn` is IGNORED on input and overwritten here — the prefix sum has exactly
    /// one home, so a caller cannot compute a second, disagreeing one.
    ///
    /// Returns `false` when the [`MAX_EMITTERS`] release clamp (D15) drops the request: nothing is
    /// written, [`dropped_emitters`](Self::dropped_emitters) and
    /// [`dropped_spawns`](Self::dropped_spawns) advance, and the prefix does NOT — a dropped
    /// emitter must not renumber the emitters that were accepted, or every device-side lane→row
    /// mapping after it would shift.
    ///
    /// # Where the `debug_assert!` lives
    ///
    /// D15 asks for a release clamp AND a `debug_assert!` alongside. The assert lives at the
    /// CALLER ([`particle_tick_emitters`], which checks `dropped_emitters() == 0` after its walk),
    /// not here: an assert inside this branch would unwind BEFORE the counters it is supposed to
    /// document were written, so the developer build and the release build would disagree about
    /// what the clamp did — and the OOB test could only observe one of them.
    pub fn push_request(&mut self, mut request: EmitRequestGpu) -> bool {
        if self.requests.len() >= MAX_EMITTERS {
            self.dropped_emitters = self.dropped_emitters.saturating_add(1);
            self.dropped_spawns = self.dropped_spawns.saturating_add(request.spawn_count);
            return false;
        }
        request.first_spawn = self.total_spawn;
        self.total_spawn = self.total_spawn.saturating_add(request.spawn_count);
        self.requests.build_view().push(request);
        true
    }

    /// This frame's requests, in the order the device's `first_spawn` binary search expects.
    #[inline]
    pub fn requests(&self) -> &[EmitRequestGpu] {
        self.requests.as_read_slice()
    }

    /// Rows written this frame — the `emitter_count` field of `particle_emit`'s push constant.
    ///
    /// Always `<= MAX_EMITTERS` (the D15 clamp), so the `usize as u32` narrowing is exact; the
    /// accessor exists so the recording site pushes a number rather than performing that cast at
    /// the call site, where the bound would not be visible.
    #[inline]
    pub fn emitter_count(&self) -> u32 {
        let count = self.requests.len();
        debug_assert!(count <= MAX_EMITTERS, "invariant: the D15 clamp bounds the emitter table");
        count as u32
    }

    /// The frame's total spawn count.
    ///
    /// Two consumers, ONE field: it is the `requested_spawn` half of `particle_kickoff`'s push
    /// constant, **and** the upload/declare gate — `total_spawn == 0` ⇒ zero bytes cross PCIe and
    /// the emit pass is not declared. The gate being the same read as the pushed value is what
    /// makes the "written but unread this frame" state unconstructible.
    #[inline]
    pub const fn total_spawn(&self) -> u32 {
        self.total_spawn
    }

    /// Emitters dropped by the [`MAX_EMITTERS`] clamp this frame (D15). Zero on every well-formed
    /// scene.
    #[inline]
    pub const fn dropped_emitters(&self) -> u32 {
        self.dropped_emitters
    }

    /// Spawns lost with the dropped emitters — see the field doc for why this is NOT
    /// `ParticleCounters::clamped_spawns`.
    #[inline]
    pub const fn dropped_spawns(&self) -> u32 {
        self.dropped_spawns
    }
}

// ── The effect-table staging ─────────────────────────────────────────────────────────

/// The device effect-table staging lane, rebuilt only when its inputs change.
#[derive(Resource)]
pub struct ParticleEffectScratch {
    /// The baked rows, INDEX-ADDRESSED: `rows[i]` is `Assets<ParticleEffect>` row `i`, so an
    /// effect index carried in a component addresses the same row on the host and the device. A
    /// hole (a row that was never minted) is zero-filled rather than skipped — skipping would
    /// renumber every row after it.
    rows: ScratchColumn<EffectParamsGpu>,
    /// The `Assets::dirty_gen()` this lane was last baked against.
    seen_gen: u64,
    /// The `ParticleClock::timestep()` this lane was last baked against.
    ///
    /// Part of the gate because the bake is a function of BOTH inputs: `damping` and the rotation
    /// multiplier are per-substep constants, so a timestep change invalidates every row even
    /// though `dirty_gen` did not move. Gating on the asset generation alone would silently keep
    /// simulating at the old rate.
    seen_timestep: f32,
    /// Monotonic, bumped once per ACTUAL rebuild — the writer-side signal the host's per-in-flight
    /// slot upload gate compares against. A writer-side generation, never a hash and never a
    /// byte-compare.
    rows_gen: u64,
    /// Effect rows DROPPED by the [`MAX_EFFECTS`] release clamp (D15's second table).
    dropped_effects: u32,
}

impl Default for ParticleEffectScratch {
    fn default() -> Self {
        let id = register_asset_layout::<EffectParamsGpu>(None);
        let rows = pool_reserve_rows(size_of::<EffectParamsGpu>());
        Self {
            rows: ScratchColumn::new(id, rows),
            seen_gen: u64::MAX,
            seen_timestep: f32::NAN,
            rows_gen: 0,
            dropped_effects: 0,
        }
    }
}

impl ParticleEffectScratch {
    /// The baked rows, index-addressed by effect index.
    #[inline]
    pub fn rows(&self) -> &[EffectParamsGpu] {
        self.rows.as_read_slice()
    }

    /// The writer-side generation, bumped once per actual rebuild. The host uploads iff its
    /// per-slot record disagrees with this number.
    #[inline]
    pub const fn rows_gen(&self) -> u64 {
        self.rows_gen
    }

    /// Effect rows dropped by the [`MAX_EFFECTS`] clamp. Zero on every well-formed scene.
    #[inline]
    pub const fn dropped_effects(&self) -> u32 {
        self.dropped_effects
    }
}

// ── A1 — the per-frame emitter fold ──────────────────────────────────────────────────

/// **A1** — advance the subsystem clock, then fold every ENABLED emitter into one frame's
/// [`EmitRequestGpu`] table. `CoreSchedule::Main`, once per rendered frame.
///
/// ```text
/// clock.advance(Time::delta_secs())            // F27: already clamped, scaled, pause-aware
/// dt = clock.steps() * clock.timestep()        // the SAME number the sim's push constant gets
/// for (emitter, xform, handle) in Query<.., Enabled<EmitterActive>>:   // sequential
///     acc += rate * dt;  n = floor(acc) + burst;  acc -= floor(acc);  burst = 0
///     push EmitRequestGpu { origin, basis from xform, effect_index, spawn_count: n,
///                           first_spawn: running };  running += n
/// total_spawn = running
/// ```
///
/// * **Complexity** `O(emitters)`, ≤ [`MAX_EMITTERS`]. **Sequential, not `par_iter`** — see the
///   module doc.
/// * **Cache** two linear column streams.
/// * **Branching** one predicted `Enabled` bit test and one `floor` per row.
/// * **Change detection** `&mut T` stamps no tick, so the unconditional accumulator write is free
///   (D16 — which is also why the emitter's hot/cold mix is a decided exception rather than an
///   oversight).
/// * **Allocations** zero after the first frame's page commit.
///
/// # `steps == 0` is a real frame, not a skipped one (M6)
///
/// Above the step rate most frames step zero times. This system still runs, still walks the
/// emitters, and still produces a table — with `dt == 0`, so the accumulators hold and only
/// pending `burst`s spawn. The GPU sim likewise still rebuilds the alive list and the render
/// records on such a frame; only the integrator loop is empty.
///
/// # Not gated on [`ParticleConfig`](crate::particle_config::ParticleConfig)
///
/// Deliberately: `ParticleEmitter` is opt-IN, so a world that does not use particles matches zero
/// rows and this is one empty query walk. Adding a config read here would give "is the subsystem
/// running" a second home (the declarators already own the first) — the shape the plan's reversal
/// ledger names as where the defects keep arriving.
//
// `clippy::needless_pass_by_value`: `Res` / `ResMut` / `Query` are by-value `SystemParam`s
// reborrowed internally — the same false positive every other system in this crate carries.
#[allow(clippy::needless_pass_by_value)]
pub fn particle_tick_emitters(
    time: Res<Time>,
    mut clock: ResMut<ParticleClock>,
    mut scratch: ResMut<ParticleEmitScratch>,
    mut emitters: Query<
        (&mut ParticleEmitter, &GlobalTransform, &ParticleEffectHandle),
        Enabled<EmitterActive>,
    >,
) {
    // Before the early-outs: a persistent Resource must not carry a previous frame's rows into a
    // frame that writes none.
    scratch.begin_frame();

    // The clock advance comes FIRST and unconditionally — `steps` is this frame's one number, and
    // the sim's push constant reads it whether or not any emitter asked for a spawn.
    clock.advance(time.delta_secs());
    let dt = clock.steps() as f32 * clock.timestep();

    for (id, (emitter, transform, handle)) in emitters.iter_entities_mut() {
        let spawn_count = advance_emitter(emitter, dt);
        scratch.push_request(emit_request_for(id, emitter, transform, handle, spawn_count));
    }

    // D15's developer-build half: the release clamp inside `push_request` has already kept every
    // write in bounds and counted what it refused, so this assert reports the CONDITION rather
    // than guarding the write — which is why it can sit after the fact without weakening anything.
    debug_assert!(
        scratch.dropped_emitters() == 0,
        "invariant: emitter_count <= MAX_EMITTERS ({MAX_EMITTERS}); {} enabled emitter(s) and {} \
         spawn(s) were dropped by the release clamp (D15)",
        scratch.dropped_emitters(),
        scratch.dropped_spawns()
    );
    debug_assert!(
        scratch.requests().len() <= MAX_EMITTERS,
        "invariant: the emit table never exceeds MAX_EMITTERS (the D15 clamp holds)"
    );
}

/// Advances one emitter's spawn accumulator by `dt` and returns the particles it asks for this
/// frame, CONSUMING its pending burst.
///
/// Extracted from [`particle_tick_emitters`] so the per-emitter arithmetic is testable without a
/// `World`, a scheduler, or a frame loop — the `fly_step` pattern.
///
/// The fractional carry is exact: the accumulator keeps `acc - floor(acc)`, so a 0.4/s rate
/// spawns exactly two particles every five seconds rather than none.
///
/// # Panics (debug)
///
/// `debug_assert!`s `rate >= 0` and a finite accumulator. In release a negative rate cannot drive
/// the count negative (`f32 as u32` saturates at zero) and the accumulator is re-clamped into
/// `[0, 1)`, so a mis-authored emitter degrades to "spawns nothing" instead of corrupting the
/// prefix sum.
#[inline]
pub fn advance_emitter(emitter: &mut ParticleEmitter, dt: f32) -> u32 {
    debug_assert!(
        emitter.rate >= 0.0 && emitter.rate.is_finite(),
        "invariant: a ParticleEmitter's rate is finite and non-negative (got {})",
        emitter.rate
    );
    debug_assert!(dt >= 0.0 && dt.is_finite(), "invariant: dt is finite and non-negative");

    emitter.accumulator += emitter.rate * dt;
    let whole = emitter.accumulator.floor();
    // `f32 as u32` saturates at u32::MAX and maps NaN to 0, so neither a pathological rate nor a
    // poisoned accumulator can wrap this into a small, plausible-looking count.
    let continuous = whole as u32;
    let carry = emitter.accumulator - whole;
    emitter.accumulator = if (0.0..1.0).contains(&carry) { carry } else { 0.0 };

    // A burst fires EXACTLY once, no matter how many frames pass before the next write.
    let spawn_count = continuous.saturating_add(emitter.burst);
    emitter.burst = 0;
    spawn_count
}

/// Builds one emitter's device request from its world pose, effect binding and this frame's spawn
/// count.
///
/// `first_spawn` is left at zero here and stamped by
/// [`ParticleEmitScratch::push_request`](ParticleEmitScratch::push_request) — the running prefix
/// has exactly one home.
///
/// The basis is the emitter's world axes, which are the COLUMNS of the row-major linear part, so
/// the device samples the spawn volume in emitter space without ever seeing a matrix. Scale rides
/// the basis vectors' lengths, which is what makes a scaled emitter spawn into a scaled volume for
/// free.
#[inline]
fn emit_request_for(
    id: EntityId,
    emitter: &ParticleEmitter,
    transform: &GlobalTransform,
    handle: &ParticleEffectHandle,
    spawn_count: u32,
) -> EmitRequestGpu {
    let affine = transform.affine();
    let m = affine.matrix3.rows;
    let origin = affine.translation;

    debug_assert!(
        (handle.0 as usize) < MAX_EFFECTS,
        "invariant: effect_index < MAX_EFFECTS ({MAX_EFFECTS}), got {}",
        handle.0
    );
    debug_assert!(
        emitter.speed_scale >= 0.0 && emitter.speed_scale.is_finite(),
        "invariant: speed_scale is finite and non-negative"
    );

    EmitRequestGpu {
        origin: [origin.x, origin.y, origin.z],
        effect_index: clamp_effect_index(handle.0),
        // Row-major linear part ⇒ the world basis vectors are its COLUMNS.
        basis_x: [m[0].x, m[1].x, m[2].x],
        spawn_count,
        // `speed_scale` rides the +Y basis's length: the device multiplies its sampled direction
        // by the basis, so scaling the basis IS scaling the launch speed, with no extra lane.
        basis_y: [
            m[0].y * emitter.speed_scale,
            m[1].y * emitter.speed_scale,
            m[2].y * emitter.speed_scale,
        ],
        first_spawn: 0,
        basis_z: [m[0].z, m[1].z, m[2].z],
        rng_seed: emitter_seed(id),
    }
}

/// The release-present clamp on a carrier's effect index (D15's discipline, applied to the second
/// fixed-size table).
///
/// A [`ParticleEffectHandle`] is a raw `u32` an author can set to anything, and the device fetches
/// `Effects[effect_index]` with `robustBufferAccess` OFF — so an out-of-range index is undefined
/// behaviour rather than a clamp, and the check cannot live behind `debug_assert!` alone.
///
/// Deliberately assert-FREE so it behaves identically in a debug and a release build: the
/// `debug_assert!` that says "a well-formed carrier is already in range" belongs to the caller
/// ([`emit_request_for`]), and keeping it out of here is what lets the clamp itself be observed by
/// a test in either profile.
#[inline]
const fn clamp_effect_index(raw: u32) -> u32 {
    if raw >= MAX_EFFECTS as u32 { MAX_EFFECTS as u32 - 1 } else { raw }
}

/// A per-emitter CONSTANT RNG seed derived from its entity id.
///
/// Constant on purpose: the frame number enters the device RNG through a push constant, so this
/// word never has to be rewritten to decorrelate successive frames — which is part of what keeps
/// the per-frame host→device traffic at zero on a frame with no spawns.
///
/// A multiply-xorshift finaliser, not the raw id: adjacent entity ids must not produce adjacent
/// seeds, or two emitters spawned back to back would emit correlated particles.
#[inline]
fn emitter_seed(id: EntityId) -> u32 {
    let mut z = (id.get() as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) as u32) | 1
}

// ── The effect-table bake ────────────────────────────────────────────────────────────

/// Bakes `Assets<ParticleEffect>` into the device's [`EffectParamsGpu`] table, rebuilding ONLY
/// when an input actually changed.
///
/// The gate is `(Assets::dirty_gen(), ParticleClock::timestep())` — a writer-side generation plus
/// the bake's other input, never a hash and never a byte-compare. Both matter: `damping` and the
/// rotation multiplier are PER-SUBSTEP constants, so a timestep change invalidates every row even
/// though the asset table did not move.
///
/// Rows are written INDEX-ADDRESSED over `0..high_water()`, with a never-minted or retired hole
/// zero-filled rather than skipped: an effect index is a raw table index carried in a component,
/// so skipping a hole would renumber every row after it and silently re-point live handles.
///
/// The [`MAX_EFFECTS`] clamp is release-present and counted (D15's second table).
//
// `clippy::needless_pass_by_value`: see `particle_tick_emitters`.
#[allow(clippy::needless_pass_by_value)]
pub fn particle_pack_effects(
    clock: Res<ParticleClock>,
    effects: Res<Assets<ParticleEffect>>,
    mut scratch: ResMut<ParticleEffectScratch>,
) {
    let dirty_gen = effects.dirty_gen();
    let timestep = clock.timestep();
    // `to_bits` rather than `==`: the boot sentinel is NaN, and NaN != NaN would make the first
    // frame's "unchanged" branch unreachable for the wrong reason instead of the right one.
    if scratch.seen_gen == dirty_gen && scratch.seen_timestep.to_bits() == timestep.to_bits() {
        return;
    }

    let high_water = effects.high_water();
    let clamped = high_water.min(MAX_EFFECTS);
    scratch.dropped_effects = (high_water - clamped) as u32;
    debug_assert!(
        high_water <= MAX_EFFECTS,
        "invariant: the effect table holds at most MAX_EFFECTS ({MAX_EFFECTS}) rows; the extra \
         rows are dropped and counted (D15)"
    );

    {
        let mut view = scratch.rows.build_view();
        view.clear();
        for index in 0..clamped {
            let row = match effects.get_by_index(index as u32) {
                Some(effect) => pack_effect_params(effect, timestep),
                // A hole keeps its slot so every later index stays put. Zeroed rather than
                // defaulted: a zero row has zero lifetime, so a particle that somehow named it
                // retires on its first substep instead of living forever.
                None => EffectParamsGpu::zeroed(),
            };
            view.push(row);
        }
    }

    scratch.seen_gen = dirty_gen;
    scratch.seen_timestep = timestep;
    scratch.rows_gen = scratch.rows_gen.wrapping_add(1);
}

// ── The refcount fold ────────────────────────────────────────────────────────────────

/// Folds the [`ParticleEffectHandle`] hooks' queued `±1` deltas into `Assets<ParticleEffect>`.
///
/// `CoreSchedule::Main`, and SUBSYSTEM-LOCAL by necessity: the shared
/// `boyko_render::apply_refcount_deltas` routes by an enum whose widening would force an
/// always-scheduled system to acquire a resource only this optional plugin inserts — the
/// cross-subsystem coupling D17 forbids. See [`ParticleEffectRefs`]'s doc for the full argument.
///
/// # No retire ticket is expected, and that is checked
///
/// Every row minted through
/// [`ParticleEffectsExt`](crate::particle_effect::ParticleEffectsExt) is pinned, so a refcount
/// zero-crossing leaves it `Loaded` and returns `None`. A `Some` here would mean a row reached
/// this system unpinned — i.e. minted straight through `Assets::add` — and would put the
/// append-only invariant that
/// [`ParticleEffectHandle`](crate::particle::ParticleEffectHandle)'s generation-free index depends
/// on at risk. It is a `debug_assert!`, and in release the ticket is dropped: an effect row owns
/// no device resource (`NEEDS_TEARDOWN == false`), so there is nothing to leak — only a slot that
/// will not be reused.
///
/// # 0%-gate
///
/// A frame with no emitter churn drains an empty queue and returns after one `is_empty()` test.
//
// `clippy::needless_pass_by_value`: see `particle_tick_emitters`.
#[allow(clippy::needless_pass_by_value)]
pub fn particle_apply_effect_refs(
    mut refs: ResMut<ParticleEffectRefs>,
    mut effects: ResMut<Assets<ParticleEffect>>,
) {
    if refs.is_empty() {
        return;
    }

    for delta in refs.queued() {
        match delta.delta {
            1 => {
                // `inc_ref` refuses a slot that is not Loading/Loaded/Failed; a refused attach is
                // a carrier bound to a never-minted index, which the emit packer clamps anyway.
                let _bound = effects.inc_ref(delta.slot);
            }
            -1 => {
                let ticket = effects.dec_ref(delta.slot, PARTICLE_EFFECT_REF_GEN);
                debug_assert!(
                    ticket.is_none(),
                    "invariant: every effect row minted through ParticleEffectsExt is pinned, so a \
                     zero-crossing must not retire it (slot {})",
                    delta.slot
                );
            }
            other => debug_assert!(
                false,
                "invariant: a ParticleEffectHandle hook pushes exactly +1 or -1, got {other}"
            ),
        }
    }

    refs.clear();
}

#[cfg(test)]
mod tests {
    use boyko_math::{Affine3A, Mat3, Vec3};

    use super::*;
    use crate::particle::PARTICLE_QUAD_INDEX_COUNT;

    /// One 64 Hz substep.
    const TS: f32 = 1.0 / 64.0;

    fn request(spawn_count: u32) -> EmitRequestGpu {
        EmitRequestGpu { spawn_count, ..EmitRequestGpu::default() }
    }

    // ── advance_emitter ─────────────────────────────────────────────────────

    /// The mandated `emitter_accumulator_carries_fractional_spawns_exactly`: a rate below one
    /// particle per frame accrues and fires on the crossing, losing nothing.
    #[test]
    fn emitter_accumulator_carries_fractional_spawns_exactly() {
        // 4 particles/s at a 64 Hz substep is 1/16 of a particle per substep.
        let mut emitter = ParticleEmitter { rate: 4.0, ..ParticleEmitter::default() };

        let mut total = 0u32;
        for _ in 0..15 {
            let n = advance_emitter(&mut emitter, TS);
            assert_eq!(n, 0, "fifteen sixteenths of a particle is not a particle");
            total += n;
        }
        total += advance_emitter(&mut emitter, TS);
        assert_eq!(total, 1, "the sixteenth substep completes exactly one particle");
        assert!(emitter.accumulator.abs() < 1e-6, "and the carry drains: {}", emitter.accumulator);

        // Over a full second the rate is preserved: 4 particles.
        let mut second = 0u32;
        for _ in 0..64 {
            second += advance_emitter(&mut emitter, TS);
        }
        assert_eq!(second, 4, "4 particles/s over 64 substeps of 1/64 s");
    }

    /// The mandated `burst_is_consumed_once`: a pending burst fires on the next tick and never
    /// again, no matter how many frames pass.
    #[test]
    fn burst_is_consumed_once() {
        let mut emitter = ParticleEmitter { burst: 32, ..ParticleEmitter::default() };

        assert_eq!(advance_emitter(&mut emitter, TS), 32, "the burst fires on the next tick");
        assert_eq!(emitter.burst, 0, "and is consumed");
        for _ in 0..10 {
            assert_eq!(advance_emitter(&mut emitter, TS), 0, "a consumed burst never re-fires");
        }
    }

    /// A burst on a rate-carrying emitter ADDS to the continuous count rather than replacing it —
    /// the two spawn sources are independent.
    #[test]
    fn a_burst_adds_to_the_continuous_count() {
        let mut emitter = ParticleEmitter { rate: 64.0, burst: 5, ..ParticleEmitter::default() };
        assert_eq!(advance_emitter(&mut emitter, TS), 1 + 5);
    }

    /// M6: a zero-`dt` frame (the common case above the step rate, and every paused frame) spawns
    /// nothing continuous and does NOT disturb the accumulator — but a pending burst still fires,
    /// because a burst is an event, not an integral.
    #[test]
    fn a_zero_dt_frame_holds_the_accumulator_but_still_fires_a_burst() {
        let mut emitter = ParticleEmitter { rate: 100.0, ..ParticleEmitter::default() };
        advance_emitter(&mut emitter, TS);
        let held = emitter.accumulator;

        assert_eq!(advance_emitter(&mut emitter, 0.0), 0);
        assert_eq!(emitter.accumulator, held, "dt == 0 must not move the accumulator");

        emitter.burst = 3;
        assert_eq!(advance_emitter(&mut emitter, 0.0), 3, "a burst is an event, not an integral");
    }

    // ── The prefix sum + the D15 clamp ──────────────────────────────────────

    /// The mandated `prefix_sum_is_monotone_and_totals_match`: `first_spawn` is the exact running
    /// prefix, monotone non-decreasing, and `total_spawn` is the sum.
    #[test]
    fn prefix_sum_is_monotone_and_totals_match() {
        let mut scratch = ParticleEmitScratch::default();
        scratch.begin_frame();

        let counts = [3u32, 0, 7, 1, 0, 12];
        for n in counts {
            assert!(scratch.push_request(request(n)));
        }

        let rows = scratch.requests();
        assert_eq!(rows.len(), counts.len());
        let mut running = 0u32;
        for (row, expected) in rows.iter().zip(counts) {
            assert_eq!(row.first_spawn, running, "first_spawn is the exact running prefix");
            assert_eq!(row.spawn_count, expected);
            running += expected;
        }
        for pair in rows.windows(2) {
            assert!(pair[0].first_spawn <= pair[1].first_spawn, "the prefix is monotone");
        }
        assert_eq!(scratch.total_spawn(), counts.iter().sum::<u32>());
    }

    /// `begin_frame` is what makes the table a per-FRAME statement: a frame that writes no rows
    /// must not upload the previous frame's.
    #[test]
    fn begin_frame_drops_the_previous_frames_rows_and_counters() {
        let mut scratch = ParticleEmitScratch::default();
        scratch.begin_frame();
        assert!(scratch.push_request(request(9)));
        assert_eq!(scratch.total_spawn(), 9);

        scratch.begin_frame();
        assert!(scratch.requests().is_empty(), "no stale rows survive into the next frame");
        assert_eq!(scratch.total_spawn(), 0, "and total_spawn is the frame's own number");
        assert_eq!(scratch.dropped_emitters(), 0);
        assert_eq!(scratch.dropped_spawns(), 0);
    }

    /// **Gate #15 / D15 / R8 — the OOB test at `MAX_EMITTERS + 1`.**
    ///
    /// Every write stays in bounds, the extra emitter is dropped, and the drop is counted EXACTLY
    /// — both in emitters and in the spawns that went with them. The clamp is release-present
    /// because `robustBufferAccess` is OFF: a 257th row would be undefined behaviour on the
    /// device, which is the failure Hanabi shipped.
    #[test]
    fn max_emitters_plus_one_stays_in_bounds_and_counts_the_drop() {
        let mut scratch = ParticleEmitScratch::default();
        scratch.begin_frame();

        for i in 0..MAX_EMITTERS {
            assert!(scratch.push_request(request(1)), "row {i} must be accepted");
        }
        assert_eq!(scratch.requests().len(), MAX_EMITTERS);
        assert_eq!(scratch.total_spawn(), MAX_EMITTERS as u32);

        // The 257th. `push_request` is panic-free by design (the D15 `debug_assert!` lives at the
        // caller), so this observes the SAME clamp in a debug and a release build.
        assert!(!scratch.push_request(request(11)), "the 257th emitter must be refused");

        assert_eq!(scratch.requests().len(), MAX_EMITTERS, "no write past the table's end");
        assert_eq!(scratch.dropped_emitters(), 1, "exactly one emitter dropped");
        assert_eq!(scratch.dropped_spawns(), 11, "and exactly its spawns counted");
        assert_eq!(
            scratch.total_spawn(),
            MAX_EMITTERS as u32,
            "a dropped emitter must NOT advance the prefix, or every accepted row after it would \
             renumber"
        );
    }

    // ── emit_request_for ────────────────────────────────────────────────────

    /// The request carries the emitter's WORLD basis — the columns of the row-major linear part —
    /// and its world origin, so the device samples the spawn volume in emitter space.
    #[test]
    fn the_request_carries_the_world_basis_columns_and_origin() {
        // A 90° yaw about +Y: local +X maps to world -Z, local +Z maps to world +X.
        let m = Mat3::from_columns(
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let transform = GlobalTransform(Affine3A { matrix3: m, translation: Vec3::new(5.0, 6.0, 7.0) });
        let emitter = ParticleEmitter::default();

        let req = emit_request_for(
            EntityId(3),
            &emitter,
            &transform,
            &ParticleEffectHandle(2),
            17,
        );

        assert_eq!(req.origin, [5.0, 6.0, 7.0]);
        assert_eq!(req.basis_x, [0.0, 0.0, -1.0], "world image of local +X");
        assert_eq!(req.basis_y, [0.0, 1.0, 0.0]);
        assert_eq!(req.basis_z, [1.0, 0.0, 0.0], "world image of local +Z");
        assert_eq!(req.effect_index, 2);
        assert_eq!(req.spawn_count, 17);
        assert_eq!(req.first_spawn, 0, "the prefix is stamped by push_request, not here");
    }

    /// `speed_scale` rides the +Y basis's length rather than a lane of its own.
    #[test]
    fn speed_scale_scales_the_launch_basis() {
        let transform = GlobalTransform(Affine3A::IDENTITY);
        let emitter = ParticleEmitter { speed_scale: 3.0, ..ParticleEmitter::default() };

        let req = emit_request_for(EntityId(0), &emitter, &transform, &ParticleEffectHandle(0), 1);

        assert_eq!(req.basis_y, [0.0, 3.0, 0.0]);
        assert_eq!(req.basis_x, [1.0, 0.0, 0.0], "the other axes are untouched");
        assert_eq!(req.basis_z, [0.0, 0.0, 1.0]);
    }

    /// Adjacent entity ids must NOT produce adjacent seeds, or two emitters spawned back to back
    /// would emit visibly correlated particles. The seed is also never zero (a zero state is a
    /// fixed point for several integer generators).
    #[test]
    fn adjacent_entity_ids_produce_decorrelated_nonzero_seeds() {
        let seeds: [u32; 8] = std::array::from_fn(|i| emitter_seed(EntityId(i)));
        for (i, &s) in seeds.iter().enumerate() {
            assert_ne!(s, 0, "seed {i} must not be zero");
            for &t in &seeds[i + 1..] {
                assert_ne!(s, t, "two entity ids must not share a seed");
                assert!(s.abs_diff(t) > 1, "adjacent ids must not give adjacent seeds");
            }
        }
        // Stable across calls: the seed is a per-emitter CONSTANT, which is what lets a frame with
        // no spawns upload zero bytes.
        assert_eq!(emitter_seed(EntityId(42)), emitter_seed(EntityId(42)));
    }

    /// **D15's second table.** An out-of-range effect index is clamped into the table before it is
    /// ever written to a buffer whose device-side fetches are unchecked (`robustBufferAccess` is
    /// OFF — an out-of-range fetch is undefined behaviour, not a clamp).
    ///
    /// In-range values pass through untouched, including the last valid row: a clamp that shifted
    /// legitimate indices would silently re-point every emitter.
    #[test]
    fn an_out_of_range_effect_index_is_clamped_into_the_table() {
        assert_eq!(clamp_effect_index(0), 0, "row 0 passes through");
        assert_eq!(
            clamp_effect_index(MAX_EFFECTS as u32 - 1),
            MAX_EFFECTS as u32 - 1,
            "the LAST valid row passes through — the off-by-one this clamp is most likely to get \
             wrong"
        );
        assert_eq!(
            clamp_effect_index(MAX_EFFECTS as u32),
            MAX_EFFECTS as u32 - 1,
            "the first out-of-range index clamps to the last valid row"
        );
        assert_eq!(clamp_effect_index(u32::MAX), MAX_EFFECTS as u32 - 1, "and so does the worst one");

        // And the packer routes through it: an in-range carrier reaches the device unchanged.
        let transform = GlobalTransform(Affine3A::IDENTITY);
        let emitter = ParticleEmitter::default();
        let req = emit_request_for(EntityId(0), &emitter, &transform, &ParticleEffectHandle(5), 0);
        assert_eq!(req.effect_index, 5);
        assert!((req.effect_index as usize) < MAX_EFFECTS);
    }

    // ── The quad index count, pinned beside its consumer ────────────────────

    /// The billboard is 4 vertices / 6 indices, and the index buffer is 12 bytes. Pinned here
    /// because the draw-args builder and the boot upload must agree on the number.
    #[test]
    fn the_billboard_quad_is_six_indices() {
        assert_eq!(PARTICLE_QUAD_INDEX_COUNT, 6);
        assert_eq!(PARTICLE_QUAD_INDEX_COUNT as usize * size_of::<u16>(), 12);
    }
}
