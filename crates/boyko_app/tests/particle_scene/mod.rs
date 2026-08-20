//! The shared scene, clock pin and image reader for the **particles P0 live-fire fixtures**
//! (`docs/PARTICLES-PLAN.md` Rev 4, gates #7 / #9 / #12 / #16 / #17).
//!
//! This directory is NOT an integration-test binary of its own (Cargo only auto-discovers
//! `tests/*.rs`); it is pulled into each fixture binary with `mod particle_scene;`, the
//! `tests/common/mod.rs` pattern. Two binaries share it — `particle_lab.rs` (the image dumps) and
//! `particle_counters_readback.rs` (the pool-partition readback) — because a gate that renders a
//! DIFFERENT scene from the one it measures proves nothing about the one that ships.
//!
//! # Why one windowed test per binary
//!
//! `EnginePlugins` composes `LightingPlugin`, whose eviction hooks are process-global, and the
//! device-singleton boot is a once-per-process event. Every windowed fixture in this workspace is
//! therefore a SINGLE-TEST binary (`taa_jitter_eval.rs`'s standing warning); the two particle
//! fixtures follow it rather than co-locating two `app.run()`s in one process.
//!
//! # Determinism: the wall clock is REMOVED from the loop, not tamed
//!
//! Gate #16 wants a frame-30 image that reproduces bit-for-bit. The windowed runner advances the
//! ECS with the REAL wall delta (`runner.rs`: `app.update_with_delta(now - last)`), and
//! `ParticleClock::advance` consumes `Time::delta_secs()` — so a scene left at engine defaults
//! spawns and integrates a wall-clock-dependent number of substeps by frame 30, and its image is
//! not a pin at all. It would still LOOK plausible, which is the failure mode a hash cannot report
//! as a cause.
//!
//! The first attempt CLAMPED the inflow instead: a config-time `Time` with `max_delta` at the
//! substep, so every frame's virtual delta would be exactly the clamp. **That was measured and it
//! failed.** The witness reported 34 frames delivering 114.6 ms against a 4 ms pin, then, at a
//! 1 ms pin, 5 unclamped frames with a 0.51 ms floor — this host's frame times are 0.1–34 ms, not
//! refresh-bounded, so no pin binds on every frame. A clamp is only a pin when it always binds,
//! and there is no value here that always does.
//!
//! So the fixture takes the wall clock OUT:
//!
//! 1. [`build_app`] inserts a **PAUSED** `Time` at config time — the seam `App::finish` documents
//!    in its own words ("a user-inserted value during config wins — e.g. a custom `Time`",
//!    `app.rs:597-602`). A paused clock yields `delta_secs() == 0.0` on EVERY frame including
//!    frame 0, whatever the wall did. Nothing in the scene reads `Time` for motion, so this costs
//!    the fixture nothing and buys it total wall-independence.
//! 2. Spawning is driven by [`ParticleEmitter::burst`] (consumed exactly once per tick, and
//!    INDEPENDENT of `dt`) rather than by `rate · dt`, which a zero `dt` would silence.
//!    [`lab_arm_burst`] re-arms one burst per frame, ordered BEFORE the fold.
//! 3. Stepping is driven by [`lab_drive_clock`], ordered AFTER the fold: it calls
//!    `ParticleClock::advance(LAB_SUBSTEP_SECS)` so the value the runner pushes to `particle_sim`
//!    is exactly one substep per frame. It runs after the fold because the fold's own
//!    `advance(0.0)` would otherwise overwrite `steps` back to zero — the two writes are ordered,
//!    not raced.
//!
//! Both orderings are why this fixture composes the particle subsystem BY HAND instead of calling
//! `add_plugin(ParticlePlugin)`: an ordering edge needs the other system's `SystemKey`, which only
//! its registration site holds. [`build_app`] mirrors `ParticlePlugin::build` resource for resource
//! and system for system — the plugin's own composition is gated separately by
//! `boyko_render/tests/particle_containment.rs` (gate #11), which is the right place for it.
//!
//! The result is a substep count that is a function of the FRAME INDEX alone. That matters beyond
//! the integrator: `particle_emit` seeds its RNG with `req.rng_seed ^ gid ^ pc.frame_index`, so
//! two runs that spawned on different frames would draw different DIRECTIONS, not merely different
//! positions. [`LabClockWitness`] asserts the shape directly — every frame's virtual delta zero,
//! one substep per frame — and both fixtures print it.
//!
//! # Why the scene is what it is (gate #16's no-overdraw constraint)
//!
//! The pin is only order-independent if no two billboards overlap — additive blending is
//! commutative, but the 8-bit `lit` target saturates, and `sat(sat(a)+b)` is only order-free while
//! nothing clips. The scene therefore emits **one particle per frame** at a UNIFORM speed from a
//! cone whose axis points away from the view axis:
//!
//! * one particle per frame ⇒ every live particle has a distinct age ⇒ a distinct radius;
//! * `speed_min == speed_max` ⇒ radius is a function of age alone, so the live set is a series of
//!   concentric shells [`LAB_SHELL_SPACING`] apart — 0.1 world units, ~7 px at the fixture's
//!   camera (a 512 px window, `fov_y = 60°`, the fan ~6 units out ⇒ ~74 px/unit) against a ~3.7 px
//!   sprite. MEASURED at the capture: 30 particles leaving 354 saturated pixels, ~12 px each,
//!   i.e. the sprites are separated with room to spare;
//! * the cone axis is the emitter's `+Z` rotated to world `+Y` ([`EMITTER_ROTATION`]), so no spawn
//!   direction lies within 50° of the view axis and two particles on the same ray at different
//!   radii cannot project onto each other (the degenerate case a camera-facing cone would hit at
//!   the screen centre).
//!
//! `size_keys` is flat and `color_keys[0]` is white: P0's shaders evaluate the SIZE ramp only and
//! carry `color_keys[0]` for the particle's whole life, so a shrinking ramp would make the pin's
//! sprites sub-pixel and a dim colour would land under the 2/255 floor `lit`'s 8 bits impose.
//!
//! # Env knobs
//!
//! | Var | Default | Meaning |
//! |---|---|---|
//! | `BOYKO_RENDER_PATH` | `deferred` | honoured by `EnginePlugins` itself, not read here |
//! | `BOYKO_WIN` | 512 | square window/composite extent |
//! | `BOYKO_PARTICLE_RATE` | 1 | particles spawned per FRAME, via `ParticleEmitter::burst` — applied at BOTH the spawn site and [`lab_arm_burst`]'s per-frame re-arm; the re-arm is the load-bearing one (see its doc: it was hardcoded to 1 until 2026-08-20, which made this knob DEAD) |
//! | `BOYKO_PARTICLE_SPEED` | 100 | uniform launch speed (units/s) = spacing / substep |
//! | `BOYKO_PARTICLE_SIZE` | 0.05 | billboard extent (world units) |
//! | `BOYKO_PARTICLE_CAPACITY` | 65536 | the boot-frozen pool capacity |
//! | `BOYKO_PARTICLE_OCCLUDER` | unset | set ⇒ spawn the opaque wall gate #12 needs |
//! | `BOYKO_PARTICLE_CONE` | `LAB_CONE_COS` (cos 40°) | the spawn cone's half-angle cosine; `1.0` degenerates it to the axis — the tunneling probe's instrument |
//! | `BOYKO_PARTICLE_SDF` | unset | set ⇒ spawn rung P1's SDF collider slab AND give the effect its contact parameters — the SCENE half |
//! | `BOYKO_PARTICLE_COLLIDE` | unset | set ⇒ `ParticleCollision::Sdf`, i.e. build the sim from the `-D SDF_COLLIDE` module — the SHADER half |
//! | `BOYKO_PARTICLE_STATS` | unset | set ⇒ `ParticleCollision::SdfStats` (rung P1b's `-D SDF_COLLIDE_STATS` instrument). Implies the collide arm; it is the ONLY way `p_counters`' three stats words become non-zero, so a skip-rate run reads them through `BOYKO_PARTICLE_READBACK_FRAME` |
//! | `BOYKO_PARTICLE_READBACK_FRAME` | unset | the runner's own knob (gates #7/#9 and rung P1b's skip-rate readback), not read here |
//!
//! The two P1 knobs are separate ON PURPOSE (see [`sdf_collider_armed`]): with the scene one set
//! and the shader one unset, the control run renders a byte-identical scene whose particles fly
//! straight through the slab, so "they bounced" cannot be explained by anything except the module
//! the pipeline was built from.

#![allow(dead_code)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::Time;
use boyko_macros::Resource;
use boyko_render::{
    PARTICLE_BLEND_ADDITIVE, PARTICLE_SHAPE_CONE, EmitterActive, ParticleClock, ParticleCollision,
    ParticleConfig, ParticleEffect, ParticleEffectHandle, ParticleEffectRefs,
    ParticleEffectScratch, ParticleEffectsExt, ParticleEmitScratch, ParticleEmitter, ParticleMode,
    particle_apply_effect_refs, particle_pack_effects, particle_tick_emitters,
};

// ── The pinned clock ─────────────────────────────────────────────────────────────────

/// The fixture's particle step rate — 1 kHz.
///
/// A power-of-ten rate rather than the stock 64 Hz because the fixture advances the clock by
/// exactly ONE substep per frame (see the module doc), so this number sets the per-frame slice of
/// simulated time and, with [`LAB_SHELL_SPACING`], the launch speed.
pub const LAB_STEP_HZ: f32 = 1_000.0;

/// One substep, in seconds — the amount [`lab_drive_clock`] advances the clock by each frame.
///
/// `1.0 / LAB_STEP_HZ` and `ParticleClock::from_hz(LAB_STEP_HZ).timestep()` are the SAME
/// expression evaluated once each, so `floor(accumulator / timestep) == 1` exactly and the
/// accumulator returns to exactly zero every frame — no residue, no drift, no frame that
/// silently takes two substeps or none.
pub const LAB_SUBSTEP_SECS: f32 = 1.0 / LAB_STEP_HZ;

/// The presented-frame index the host dump captures — `host_dump::SETTLE_FRAMES`, mirrored here
/// because the fixture's virtual-time arithmetic is stated against it.
pub const CAPTURE_FRAME: u32 = 30;

// ── The scene ────────────────────────────────────────────────────────────────────────

/// Camera eye — 6 units back on `+Z`, at the emitter's fan height.
pub const CAMERA_EYE: Vec3 = Vec3::new(0.0, 1.4, 6.0);

/// Camera target — level with the eye, so the fan sits centred.
pub const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 1.4, 0.0);

/// The emitter's world position: just above the floor, on the view axis.
pub const EMITTER_POS: Vec3 = Vec3::new(0.0, 0.35, 0.0);

/// Rotates the emitter's `+Z` (the spawn cone's axis — `PARTICLE_SHAPE_CONE`) onto world `+Y`:
/// a −90° turn about `+X`, i.e. `Quat::new(sin(−45°), 0, 0, cos(45°))`.
///
/// Load-bearing for the no-overdraw constraint, not decoration: at the IDENTITY rotation the cone
/// axis is world `+Z` — straight at the camera — and every particle would project onto the same
/// screen point regardless of its radius.
pub const EMITTER_ROTATION: Quat = Quat::new(
    -core::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    core::f32::consts::FRAC_1_SQRT_2,
);

/// `cos(40°)` — the spawn cone's half-angle. Wide enough to fan the shells apart horizontally,
/// narrow enough that no direction comes within 50° of the view axis.
pub const LAB_CONE_COS: f32 = 0.766_044_4;

/// Particle lifetime, seconds. Far longer than the capture window's 124 ms of virtual time, so
/// NOTHING retires before frame 30 and the live count is exactly the spawn count — which is what
/// makes the readback gate's partition arithmetic checkable by hand.
pub const LAB_LIFETIME: f32 = 8.0;

/// The 90° turn about `+X` that stands a `plane` quad up as a wall facing the camera — verbatim
/// from `taa_jitter_eval.rs`'s `WALL_ROTATION`, whose facing is already proven on this device.
pub const WALL_ROTATION: Quat = Quat::new(
    core::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    core::f32::consts::FRAC_1_SQRT_2,
);

/// Gate #12's opaque occluder: a 2.2-unit quad standing between the camera and the LEFT half of
/// the fan, so one dump shows both classes of pixel — particles in front of nothing, and particles
/// behind an opaque surface.
pub const OCCLUDER_POS: Vec3 = Vec3::new(-1.25, 1.4, 2.5);

/// The occluder quad's side length.
pub const OCCLUDER_SIZE: f32 = 2.2;

/// The sun direction TO the light — `examples/room.rs`'s, so the floor and the occluder are lit
/// the way every other host fixture lights them.
pub const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

// ── Rung P1's collider (`BOYKO_PARTICLE_SDF`) ────────────────────────────────────────

/// Rung P1's SDF collider: a slab centred 2.0 units up, i.e. a CEILING the upward fan runs into.
///
/// A ceiling rather than a floor because this fixture's effect has no gravity (the module doc's
/// determinism constraint): the particles' only motion is their launch velocity, so the one surface
/// they will certainly reach is the one they are already flying at. Its underside sits at
/// `2.0 − 0.1 = 1.9`, which the oldest particle reaches at ~frame 15 of a 30-frame capture — early
/// enough that the bounce is half the fan's history by the time the dump is taken.
pub const SDF_COLLIDER_POS: [f32; 3] = [0.0, 2.0, 0.0];

/// The collider slab's half-extents. 2.0 in x/z covers the whole fan: a 40° cone travelling the
/// 1.55 units to the slab spreads at most `1.55 · sin 40° ≈ 1.0` sideways, so no particle escapes
/// past an edge and the image has no "some bounced, some did not" ambiguity in it.
pub const SDF_COLLIDER_HALF: [f32; 3] = [2.0, 0.1, 2.0];

/// The colliding effect's contact radius, in world units — the billboard's own half-extent, so a
/// sprite comes to rest visually TOUCHING the slab rather than half inside it.
pub const LAB_COLLISION_RADIUS: f32 = 0.05;

/// The colliding effect's restitution. 0.5 is chosen to be unmistakable in a still image: the
/// bounced half of the fan travels back down at exactly half the launch speed, so the returning
/// shells are spaced at half the pitch of the outgoing ones and the two families cannot be confused
/// for one.
pub const LAB_RESTITUTION: f32 = 0.5;

/// The colliding effect's friction. Zero: the tangential component is what carries a particle
/// ACROSS the slab, and keeping it undamped is what makes the contact read as a bounce rather than
/// as particles sticking where they landed.
pub const LAB_FRICTION: f32 = 0.0;

// ── Env knobs ────────────────────────────────────────────────────────────────────────

/// The square window/composite extent (`BOYKO_WIN`, default 512 — the extent every image pin in
/// `goldens/PINS.toml` is blessed at).
pub fn window_size() -> u32 {
    env_parsed("BOYKO_WIN").unwrap_or(512)
}

/// Particles spawned per FRAME (`BOYKO_PARTICLE_RATE`), driven through
/// [`ParticleEmitter::burst`].
///
/// The default of 1 is the whole no-overdraw mechanism: one spawn per frame ⇒ every live particle
/// has a distinct age ⇒ a distinct radius ⇒ no two billboards land on the same pixels. A higher
/// value is for the density/measurement runs (gate #17), where overlap is expected and the image
/// is not a pin.
///
/// **Read at TWO sites, and only one of them mattered.** [`setup`] seeds the component with this
/// value and [`lab_arm_burst`] re-arms it every frame; the re-arm hardcoded `1` until 2026-08-20,
/// which overwrote the seed BEFORE the fold ever consumed it — the knob was dead, MEASURED as a
/// rate-8 run producing a dump byte-identical to a rate-1 one. Both sites now read this fn, so a
/// future divergence needs two edits rather than none.
pub fn spawn_per_frame() -> u32 {
    env_parsed("BOYKO_PARTICLE_RATE").unwrap_or(1)
}

/// Uniform launch speed in units/second (`BOYKO_PARTICLE_SPEED`).
///
/// The default is `LAB_SHELL_SPACING / LAB_SUBSTEP_SECS` — the fixture's geometry depends on the
/// PRODUCT `speed · timestep` (the per-substep radial advance), not on either alone, which is why
/// re-rating the clock leaves the image's layout unmoved.
pub fn launch_speed() -> f32 {
    env_parsed("BOYKO_PARTICLE_SPEED").unwrap_or(LAB_SHELL_SPACING / LAB_SUBSTEP_SECS)
}

/// The world-space gap between two consecutive particles' radii — one substep of travel.
///
/// 0.1 units is ~15 px at the fixture's camera (a 512 px window, `fov_y = 60°`, the fan 6 units
/// away ⇒ ~148 px/unit) against a ~15 px sprite, so consecutive shells touch at most at their
/// corners and no two billboards overlap: gate #16's constraint, expressed as a number rather than
/// as a hope.
pub const LAB_SHELL_SPACING: f32 = 0.1;

/// Billboard half-extent in world units (`BOYKO_PARTICLE_SIZE`).
pub fn billboard_size() -> f32 {
    env_parsed("BOYKO_PARTICLE_SIZE").unwrap_or(0.05)
}

/// The boot-frozen pool capacity (`BOYKO_PARTICLE_CAPACITY`, default 65 536 — the plan's P0
/// fixture capacity, a quarter of the shipping default so the boot fill is quick).
pub fn pool_capacity() -> u32 {
    env_parsed("BOYKO_PARTICLE_CAPACITY").unwrap_or(65_536)
}

/// Whether gate #12's opaque occluder is in the scene (`BOYKO_PARTICLE_OCCLUDER`).
pub fn occluder_armed() -> bool {
    std::env::var("BOYKO_PARTICLE_OCCLUDER").is_ok()
}

/// The spawn cone's half-angle cosine (`BOYKO_PARTICLE_CONE`, default [`LAB_CONE_COS`]).
///
/// Exists for ONE measurement: rung P1's tunneling probe (plan P1 gate, R6). A fan is the wrong
/// instrument for it — at a raised launch speed the cone's outer particles leave past the collider's
/// EDGE, which is indistinguishable in an image from particles that stepped THROUGH it. At
/// `BOYKO_PARTICLE_CONE=1.0` the cone degenerates to the axis (`cap == 0` ⇒ the direction is exactly
/// `basis_z`, the leaf's own documented degenerate case), every particle flies straight at the
/// collider, and a white pixel above it can only be a particle that tunneled.
///
/// The default is the fixture constant, so every existing pin renders the same bytes.
pub fn cone_cos() -> f32 {
    env_parsed("BOYKO_PARTICLE_CONE").unwrap_or(LAB_CONE_COS)
}

/// Whether rung P1's SDF collider slab is in the SCENE (`BOYKO_PARTICLE_SDF`) — and, with it, the
/// per-effect contact parameters.
///
/// Deliberately SEPARATE from [`collision_armed`], which arms the shader variant. Splitting them is
/// what makes the live-fire control exact: with `BOYKO_PARTICLE_SDF` set and
/// `BOYKO_PARTICLE_COLLIDE` unset the scene, the effect table, the emitter, the clock and the
/// camera are all identical and the ONLY difference between the two runs is which `particle_sim`
/// module the pipeline was built from. One knob doing both would leave "the particles moved because
/// the scene changed" on the table as an explanation.
pub fn sdf_collider_armed() -> bool {
    std::env::var("BOYKO_PARTICLE_SDF").is_ok()
}

/// Whether the sim is built from a COLLIDING module (`BOYKO_PARTICLE_COLLIDE`) — rung P1's
/// `ParticleCollision::Sdf` arming. See [`sdf_collider_armed`] for why this is its own knob.
pub fn collision_armed() -> bool {
    std::env::var("BOYKO_PARTICLE_COLLIDE").is_ok()
}

/// Whether the sim is built from rung P1b's INSTRUMENTED module (`BOYKO_PARTICLE_STATS`) — the
/// `-D SDF_COLLIDE_STATS` variant, whose per-wave census is the only way `p_counters`' three stats
/// words become non-zero.
///
/// A third knob rather than a second value of [`collision_armed`], for the reason the two P1 knobs
/// are already split: the skip-rate measurement compares the census run against the plain collide
/// run at the SAME density, and a measurer must be able to state the two runs' arming
/// independently of the scene's.
pub fn collision_stats_armed() -> bool {
    std::env::var("BOYKO_PARTICLE_STATS").is_ok()
}

/// The resolved [`ParticleCollision`] arm for this run — ONE function, so the three-valued axis is
/// decided in one place rather than by two booleans at the config literal (two booleans admit a
/// fourth combination the enum does not).
///
/// `BOYKO_PARTICLE_STATS` implies the collide arm: the census instruments the field walk and has
/// nothing to count without it, exactly as its `-D` stacks on rung P1's.
///
/// # It REFUSES the census without the scene half, and that is the rung's own subject
///
/// `BOYKO_PARTICLE_STATS` without `BOYKO_PARTICLE_SDF` is a legal-looking configuration that is not
/// a legal control: the shader half is armed, the module walks the field — but there is no collider
/// entity and the effect's `collision_radius` is 0, so the edit list is empty and the "skip rate"
/// measured is a property of *no geometry existing*, not of the Lipschitz cache.
///
/// It has to be a refusal rather than a caveat because such a run passes EVERYTHING: the census is
/// armed, the counters are non-zero, and all three of `assert_skip_census`'s construction
/// inequalities hold on it. That is the instrument-cannot-see-its-subject class — gate #17's own
/// finding — one axis over, inside the rung built to close it.
///
/// # Panics
///
/// When `BOYKO_PARTICLE_STATS` is set and `BOYKO_PARTICLE_SDF` is not.
pub fn collision_arming() -> ParticleCollision {
    if collision_stats_armed() {
        assert!(
            sdf_collider_armed(),
            "BOYKO_PARTICLE_STATS was set without BOYKO_PARTICLE_SDF. The census would run against \
             an EMPTY edit list with collision_radius = 0 — no collider in the scene, no contact \
             parameters on the effect — so the skip rate it reports would be a property of `no \
             geometry exists`, not of the Lipschitz cache. Such a run is indistinguishable from a \
             real measurement in every artifact and passes every consistency bound. Set \
             BOYKO_PARTICLE_SDF=1 as well (the scene half), or drop BOYKO_PARTICLE_STATS."
        );
        ParticleCollision::SdfStats
    } else if collision_armed() {
        ParticleCollision::Sdf
    } else {
        ParticleCollision::Off
    }
}

/// Whether this run resolves to the DEFERRED path — the one whose depth buffer holds a
/// FRAGMENT-WRITTEN encode rather than hardware depth.
///
/// # This USED to name a known defect. It no longer does — and the distinction is the message
///
/// MEASURED on the first armed run of this fixture, and root-caused: `gbuffer_push_from_view` —
/// the projection the runner hands the particle draw on Deferred — is `marcher_view_proj_rows`,
/// whose `row2 == row3` (`boyko_render/src/view.rs`, "clip.z == clip.w (perspective divide row)").
/// Every rasterized vertex therefore has `SV_Position.z == clip.z / clip.w == 1.0` exactly, while
/// that path's depth buffer holds the euclidean `length(cam_eye - P) / MESH_DEPTH_T_MAX` its
/// G-buffer FS writes through `SV_Depth`. The particle pipeline is built `VK_COMPARE_OP_LESS`
/// there, so under the BASE shader pair every fragment failed — including over the sky, whose
/// cleared depth is not greater than 1.0.
///
/// It was never fixable host-side: `z_ndc` is a ratio of two affine functions of the world
/// position, and no such ratio equals a Euclidean norm. The fix is the `-D DEPTH_LINEAR` shader
/// pair this path now binds (`docs/PARTICLES-PLAN.md`, P2 item 1) — the fragment writes the depth
/// buffer's OWN encode, term for term the G-buffer producer's. **Particles are expected to render
/// here now, and the assertion below is expected to hold on all four paths.**
///
/// Kept, and still consulted by the failure message, because a trip on THIS path has a different
/// first suspect from a trip anywhere else: the depth contract is two answers (compare op +
/// shader pair) off one predicate, and if either half regressed the symptom is exactly the old
/// one — a scene that renders perfectly with no particles in it.
pub fn on_deferred_depth_encode_path() -> bool {
    matches!(
        std::env::var("BOYKO_RENDER_PATH").ok().as_deref().map(str::trim),
        None | Some("") | Some("deferred")
    )
}

/// One `FromStr` env read, or `None` when unset or unparsable — the `BOYKO_WIN` idiom every host
/// fixture in this crate uses.
fn env_parsed<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|s| s.parse().ok())
}

// ── The determinism witness ──────────────────────────────────────────────────────────

/// The measured clock inflow — the fixture's own refutation of its determinism premise.
///
/// Accumulates `Time::delta_secs()` (the quantity `ParticleClock::advance` consumes) and the frame
/// count. It reads `Time`, not `ParticleClock::steps()`, ON PURPOSE: the delta is written ONCE per
/// frame by the frame driver before any system runs, so every system observes the same value
/// whatever order the scheduler picks, whereas `steps()` changes value in the middle of the frame
/// (at `particle_tick_emitters`) and a witness racing that write would itself be nondeterministic.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LabClockWitness {
    /// ECS frames observed since boot.
    pub frames: u32,
    /// Substeps the fixture drove, summed from `ParticleClock::steps()` read at the site that
    /// WROTE it ([`lab_drive_clock`]) — not from a second, racing observer.
    pub substeps: u64,
    /// Frames whose `Time::delta_secs()` was NOT exactly zero. Must be 0: a non-zero virtual delta
    /// means the paused clock was un-paused by something, and the run's substep count is back to
    /// being a function of the wall clock.
    pub unpaused_frames: u32,
    /// Frames whose driven substep count was not exactly 1 — the accumulator misbehaving. Must
    /// be 0.
    pub off_rate_frames: u32,
    /// The shortest raw (unclamped) frame delta seen, in seconds. Reported only: with a paused
    /// clock the wall time no longer reaches the simulation, and this number is here to show by
    /// how much it USED to vary (0.1 ms to 34 ms on this host).
    pub min_real_secs: f64,
    /// The longest raw frame delta seen, in seconds.
    pub max_real_secs: f64,
}

impl Default for LabClockWitness {
    fn default() -> Self {
        Self {
            frames: 0,
            substeps: 0,
            unpaused_frames: 0,
            off_rate_frames: 0,
            min_real_secs: f64::INFINITY,
            max_real_secs: 0.0,
        }
    }
}

impl LabClockWitness {
    /// The shortest raw frame, in milliseconds.
    pub fn min_real_ms(&self) -> f64 {
        self.min_real_secs * 1000.0
    }

    /// The longest raw frame, in milliseconds.
    pub fn max_real_ms(&self) -> f64 {
        self.max_real_secs * 1000.0
    }

    /// Whether the run's simulated time is a function of the FRAME INDEX alone — the property the
    /// image pin rests on: the engine clock never delivered virtual time, every frame drove
    /// exactly one substep, and the two counts agree.
    pub fn is_deterministic(&self) -> bool {
        self.unpaused_frames == 0
            && self.off_rate_frames == 0
            && self.substeps == u64::from(self.frames)
    }
}

/// Re-arms [`spawn_per_frame`] BURST per frame on every emitter, ordered BEFORE
/// `particle_tick_emitters`.
///
/// The burst path, not `rate`, because the fold derives its continuous spawn count from
/// `clock.steps() · timestep` and this fixture's clock is advanced AFTER the fold — the fold's
/// `dt` is therefore zero and a rate would spawn nothing. `burst` is consumed exactly once by the
/// tick that reads it, so one write per frame is exactly `spawn_per_frame()` particles per frame,
/// with no dependence on any clock at all.
///
/// # This write is the knob, and it used to be a literal `1`
///
/// Because this system is ordered BEFORE the fold, its value — not [`setup`]'s seed — is what
/// every frame including frame 0 actually spawns. With `1` hardcoded here, `BOYKO_PARTICLE_RATE`
/// changed nothing at all: MEASURED 2026-08-20, a rate-8 run produced a dump byte-identical to
/// the rate-1 one (`sha256 60f39a3c…`), so gate #17's density runs could not be driven and would
/// have reported a 1-per-frame scene as an 8-per-frame one. Reading the same fn both sites read
/// is the repair; the DEFAULT is unchanged, so every existing pin renders the same bytes.
///
/// The env read is per frame rather than cached: this is a fixture, the value is wanted at the
/// site that uses it, and one `getenv` per frame is not measurable against a windowed present.
#[allow(clippy::needless_pass_by_value)]
pub fn lab_arm_burst(mut emitters: Query<&mut ParticleEmitter>) {
    let burst = spawn_per_frame();
    for emitter in emitters.iter_mut() {
        emitter.burst = burst;
    }
}

/// Advances the particle clock by exactly one substep, ordered AFTER `particle_tick_emitters`, and
/// records the witness.
///
/// AFTER, because the fold opens with its own `clock.advance(Time::delta_secs())` — zero on this
/// fixture's paused clock — which would otherwise overwrite `steps` back to 0 and leave the sim
/// with nothing to integrate. The runner reads `ParticleClock::steps()` once the whole ECS frame
/// has returned, so the value this system leaves behind is the one the `particle_sim` push
/// carries.
#[allow(clippy::needless_pass_by_value)]
pub fn lab_drive_clock(
    time: Res<Time>,
    mut clock: ResMut<ParticleClock>,
    mut witness: ResMut<LabClockWitness>,
) {
    clock.advance(LAB_SUBSTEP_SECS);
    let steps = clock.steps();

    let real = time.real_delta().as_secs_f64();
    witness.frames += 1;
    witness.substeps += u64::from(steps);
    if time.delta_secs() != 0.0 {
        witness.unpaused_frames += 1;
    }
    if steps != 1 {
        witness.off_rate_frames += 1;
    }
    witness.min_real_secs = witness.min_real_secs.min(real);
    witness.max_real_secs = witness.max_real_secs.max(real);
}

// ── The effect ───────────────────────────────────────────────────────────────────────

/// The fixture's effect — a spark-shaped additive burst authored for the gate #16 constraint
/// rather than for looks (see the module doc).
///
/// Every term that would make the image age-dependent is switched off: no gravity (the fan stays a
/// clean radial series), no drag (`damping == 1.0` exactly, not merely close), no spin (the
/// rotation multiplier bakes to the exact identity `(1, 0)`), and a FLAT size ramp.
pub fn lab_effect() -> ParticleEffect {
    let speed = launch_speed();
    // The contact parameters ride the SCENE knob, not the collision one: both live-fire runs then
    // upload a byte-identical effect table and the only difference between them is the sim module.
    // Unset (every existing pin's configuration), all three stay 0 and the row is P0's exactly.
    let collides = sdf_collider_armed();
    ParticleEffect {
        gravity: [0.0; 3],
        drag: 0.0,
        rot_speed: 0.0,
        lifetime_min: LAB_LIFETIME,
        lifetime_max: LAB_LIFETIME,
        speed_min: speed,
        speed_max: speed,
        size_base: billboard_size(),
        cone_cos: cone_cos(),
        collision_radius: if collides { LAB_COLLISION_RADIUS } else { 0.0 },
        restitution: if collides { LAB_RESTITUTION } else { 0.0 },
        friction: if collides { LAB_FRICTION } else { 0.0 },
        // White-hot for the particle's whole life: P0 is spawn-passthrough on colour, and `lit` is
        // 8-bit post-tonemap, so a dim key would land under the 2/255 additive floor.
        color_keys: [0xFFFF_FFFF; 4],
        color_times: [0.0, 0.333_333_34, 0.666_666_7, 1.0],
        size_keys: [1.0; 4],
        // 0 ⇒ the fragment shader's untextured arm: a flat white quad, no bindless sample. The
        // fixture must not depend on which texture happens to sit in bindless slot 0.
        tex_index: 0,
        blend_class: PARTICLE_BLEND_ADDITIVE,
        flags: 0,
        emitter_shape: PARTICLE_SHAPE_CONE,
    }
}

// ── The scene ────────────────────────────────────────────────────────────────────────

/// The fixture scene: a floor, the sun/sky pair, the camera, ONE emitter, and — when
/// `BOYKO_PARTICLE_OCCLUDER` is set — one opaque wall standing in front of the fan's left half.
#[allow(clippy::needless_pass_by_value)]
pub fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut effects: ResMut<Assets<ParticleEffect>>,
    dev: NonSendRes<GpuDevice>,
) {
    let floor = meshes.plane(dev.get(), 14.0);
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));

    if occluder_armed() {
        let wall = meshes.plane(dev.get(), OCCLUDER_SIZE);
        commands.spawn(MeshBundle::new(wall, Transform {
            translation: OCCLUDER_POS,
            rotation: WALL_ROTATION,
            scale: Vec3::ONE,
        }));
    }

    if sdf_collider_armed() {
        // Rung P1's collider — an `SdfPrimitive` entity, which is the ONE way geometry enters the
        // engine's field (principle 0: the per-entity component IS the store, and `collect_sdf_edits`
        // gathers it into the single edit list every field consumer reads). The particle sim reads
        // that same list at binding 10; there is no particle-side copy of this slab anywhere.
        //
        // It is a UNION with hard edges (`smoothness = 0`): a smooth blend would make the field
        // super-Lipschitz, which is exactly the regime where the skip bound's L factor starts to
        // matter — worth a fixture of its own, but not the one that has to show a bounce.
        commands.spawn(SdfPrimitive(SdfEdit::box_shape(
            SDF_COLLIDER_POS,
            SDF_COLLIDER_HALF,
            sdf_op::UNION,
            0.0,
        )));
    }

    // The effect row is minted through the domain API, not `Assets::add`, so it is PINNED — a
    // `ParticleEffectHandle` is a raw index with no generation, and an unpinned row could retire
    // under the live carrier.
    let effect = effects.register_effect(lab_effect());
    commands
        .spawn(ParticleEmitter {
            // `rate` stays at zero and [`lab_arm_burst`] drives the spawns — see its doc and the
            // module doc's determinism section. A non-zero rate here would add a `dt`-dependent
            // term back on top of the deterministic one.
            rate: 0.0,
            accumulator: 0.0,
            burst: spawn_per_frame(),
            speed_scale: 1.0,
        })
        .insert(Transform {
            translation: EMITTER_POS,
            rotation: EMITTER_ROTATION,
            scale: Vec3::ONE,
        })
        .insert(ParticleEffectHandle(effect.index()))
        // The third arming axis (D13): the config arms the subsystem, the component's presence
        // makes the entity an emitter, and this bit makes it emit NOW.
        .enable::<EmitterActive>();

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    let camera_pose =
        Affine3A::look_at_rh(CAMERA_EYE, CAMERA_TARGET, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: camera_pose.translation,
            rotation: Quat::from_mat3(camera_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// Composes the fixture app: `EnginePlugins` (which honours `BOYKO_RENDER_PATH` itself), the
/// particle subsystem composed BY HAND, the ARMED config, the paused engine clock, and the
/// fixture's own two drive systems.
///
/// # Why the subsystem is composed here and not by `add_plugin(ParticlePlugin)`
///
/// The fixture needs two ordering edges — burst-arm BEFORE the fold, clock-drive AFTER it — and an
/// edge needs the other system's `SystemKey`, which only its registration site holds. Everything
/// below mirrors `ParticlePlugin::build` one-for-one (six resources, three systems, the same
/// `refs → pack` edge); the plugin's own composition and its D17 containment contract are gated by
/// `boyko_render/tests/particle_containment.rs`, which is where that claim belongs.
///
/// The `insert_resource` calls come AFTER `add_plugins` on purpose — that is the documented
/// override order (`App::finish` inserts `Time` only IF ABSENT).
pub fn build_app(title: &'static str) -> App {
    let win = window_size();
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, win, win));
    app.add_startup_system(setup);

    // ARMED — the whole point of the fixture. `capacity` is boot-frozen: the runner reads it once,
    // after every plugin's `build`, to decide whether to create the GPU bundle at all.
    app.insert_resource(ParticleConfig {
        mode: ParticleMode::GpuUnlit,
        capacity: pool_capacity(),
        // Rung P1's arm. Boot-frozen: the runner reads it once, beside `capacity`, to pick which
        // `particle_sim` module the pipeline is built from. Rung P1b's census is a THIRD value on
        // the same axis, not a second knob over `Sdf`, so the fixture resolves both here.
        collision: collision_arming(),
    });
    // The subsystem's own clock, re-rated to the fixture's substep.
    app.insert_resource(ParticleClock::from_hz(LAB_STEP_HZ));
    // `ParticlePlugin::build`'s other four resources, verbatim.
    app.insert_resource(Assets::<ParticleEffect>::default());
    app.insert_resource(ParticleEmitScratch::default());
    app.insert_resource(ParticleEffectScratch::default());
    app.insert_resource(ParticleEffectRefs::default());

    // The engine clock, PAUSED before frame 0 — the wall clock never reaches the simulation (see
    // the module doc). A startup system could not do this: it runs after that frame's
    // `advance_with`, so frame 0's raw delta would already be in the accumulator.
    let mut time = Time::default();
    time.pause();
    app.insert_resource(time);

    app.insert_resource(LabClockWitness::default());
    app.add_systems_cfg(|b| {
        // A1, with the fixture's two drives bracketing it.
        let burst = b.add_system(lab_arm_burst).key();
        let tick = b.add_system(particle_tick_emitters).after(burst).key();
        b.add_system(lab_drive_clock).after(tick);

        // The refcount fold BEFORE the effect bake — `ParticlePlugin`'s own edge and its reason:
        // a +1/-1 moves the asset table's dirty generation and the bake's re-run gate reads it.
        let refs = b.add_system(particle_apply_effect_refs).key();
        b.add_system(particle_pack_effects).after(refs);
    });

    app
}

/// Prints the fixture's resolved configuration — the self-identifying boot line every host
/// fixture in this workspace emits, so a capture's env is readable from its own log rather than
/// from whoever ran it.
pub fn print_config(what: &str) {
    // `collide` prints the RESOLVED arm, not the raw `BOYKO_PARTICLE_COLLIDE` bit. A run armed by
    // `BOYKO_PARTICLE_STATS` alone builds a colliding module, so the raw bit would have printed
    // `false` on a run whose particles collide — a boot line that says the opposite of what the
    // pipeline does is worse than no boot line.
    println!(
        "{what}: path={} win={} spawn_per_frame={} speed={} size={} capacity={} occluder={} \
         sdf_collider={} collide={} substep={LAB_SUBSTEP_SECS}s hz={LAB_STEP_HZ} \
         capture_frame={CAPTURE_FRAME}",
        std::env::var("BOYKO_RENDER_PATH").unwrap_or_else(|_| "deferred (default)".into()),
        window_size(),
        spawn_per_frame(),
        launch_speed(),
        billboard_size(),
        pool_capacity(),
        occluder_armed(),
        sdf_collider_armed(),
        collision_arming().as_str(),
    );
}

// ── The image reader ─────────────────────────────────────────────────────────────────

/// A decoded 32-bpp BMP in TOP-DOWN screen order.
pub struct LabImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` BGRA texels, `y` increasing DOWNWARD.
    pub bgra: Vec<[u8; 4]>,
}

impl LabImage {
    /// Pixels whose maximum channel is at least `floor` — "not background-black".
    pub fn count_at_least(&self, floor: u8) -> usize {
        self.bgra.iter().filter(|p| p[0].max(p[1]).max(p[2]) >= floor).count()
    }

    /// Pixels whose three colour channels are ALL at least `floor` — the near-white core an
    /// additive billboard leaves on `lit`.
    ///
    /// A max-channel test would also count the sun-lit floor; requiring all three is what makes
    /// this a particle probe rather than a brightness probe.
    pub fn count_white(&self, floor: u8) -> usize {
        self.bgra.iter().filter(|p| p[0] >= floor && p[1] >= floor && p[2] >= floor).count()
    }

    /// [`Self::count_white`] restricted to a top-down rectangle, clamped to the image.
    pub fn count_white_in(&self, x0: u32, y0: u32, x1: u32, y1: u32, floor: u8) -> usize {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = self.bgra[(y * self.width + x) as usize];
                if p[0] >= floor && p[1] >= floor && p[2] >= floor {
                    n += 1;
                }
            }
        }
        n
    }

    /// The brightest single channel anywhere in the image.
    pub fn max_channel(&self) -> u8 {
        self.bgra.iter().map(|p| p[0].max(p[1]).max(p[2])).max().unwrap_or(0)
    }
}

/// Decodes the 32-bpp uncompressed BMP `boyko_app::host_dump::write_bmp` emits (54-byte header,
/// `BI_RGB`, POSITIVE height ⇒ bottom-up rows), returning TOP-DOWN rows.
///
/// # Panics
///
/// Panics with the path and the reason when the file is missing, truncated, or not that shape —
/// in a gate, an unreadable dump IS the failure, and a `Result` here would only be unwrapped.
pub fn read_bmp(path: &str) -> LabImage {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("invariant: the fixture dump is readable at {path}: {e}"));
    assert!(bytes.len() >= 54 && &bytes[0..2] == b"BM", "{path}: not a BMP");

    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let i32_at = |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);

    let data_offset = u32_at(10) as usize;
    assert_eq!(u16_at(28), 32, "{path}: expected 32 bpp");
    assert_eq!(u32_at(30), 0, "{path}: expected BI_RGB");

    let width = i32_at(18);
    let height_i = i32_at(22);
    assert!(width > 0 && height_i != 0, "{path}: degenerate extent {width}x{height_i}");
    let width = width as u32;
    let bottom_up = height_i > 0;
    let height = height_i.unsigned_abs();

    let row_bytes = (width as usize) * 4;
    let needed = data_offset + row_bytes * (height as usize);
    assert!(bytes.len() >= needed, "{path}: truncated ({} bytes, need {needed})", bytes.len());

    let mut bgra = vec![[0u8; 4]; (width as usize) * (height as usize)];
    for row in 0..height as usize {
        let src_row = if bottom_up { (height as usize) - 1 - row } else { row };
        let src = data_offset + src_row * row_bytes;
        for x in 0..width as usize {
            let o = src + x * 4;
            bgra[row * (width as usize) + x] = [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]];
        }
    }
    LabImage { width, height, bgra }
}

/// The SHA-256 of a file's bytes, lower-case hex — the same digest `scripts/golden.ps1` blesses a
/// `goldens/PINS.toml` row with, computed in-process so a fixture can PRINT the hash of the dump
/// it just produced.
///
/// A local implementation because this workspace takes no third-party dependency for a hash a
/// gate prints; it is ~40 lines of the published FIPS 180-4 compression function.
///
/// # Panics
///
/// Panics when the file cannot be read — see [`read_bmp`] for why that is the right shape here.
pub fn sha256_file(path: &str) -> String {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("invariant: the fixture dump is readable at {path}: {e}"));
    let digest = sha256(&bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// FIPS 180-4 SHA-256 over `msg`.
fn sha256(msg: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut padded = msg.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
