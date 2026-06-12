//! Phase 20.1 prev-correctness tests (plan §Metrics G7, tests T3/T4/T6/T7).
//!
//! The GPU lerp `mix(prev_pos, pos, alpha)` is only as honest as the
//! `prev_pos` discipline (D2/D3): exactly one per-substep shuffle site
//! (`sync_gpu_instance`) and one seed site (`GpuInstance::new`). These tests
//! drive the REAL native `SimRunner` (thread pool + schedule + transition
//! pass) headlessly and assert the invariant **bitwise**:
//!
//! * T3 — Particles: after one further substep, every row's `prev_pos` equals
//!   the prior substep's packed `pos`; rows whose `Position` moved (the
//!   independent ★n11 witness) have `pos != prev_pos`.
//! * T4 — Physics: across substeps, `prev_pos` equals the prior substep's
//!   packed `pos` for every ball — proving `sync_ball_gpu` / `tint_collided`
//!   (the field-granular writers, D3) never re-shuffle or clobber prev; tinted
//!   rows changed `color` only (★n10: the flash is re-derived via
//!   `GpuInstance::pack_rgba8`, not duplicated bytes).
//! * T6 — spawn seed: a click-path-shaped direct `create_entity` spawn has
//!   `prev_pos == pos` before any substep runs.
//! * T7 — proptest (light): over random substep/idle frame sequences, "prev
//!   equals pos-of-previous-pack" holds after every pack, and idle frames
//!   leave the record bit-identical.
//!
//! # Miri
//!
//! `#![cfg(not(miri))]`: drives `Schedule::run` (worker dispatch via
//! `Scope::spawn`, the Phase-9.1 Tree-Borrows deferral), like
//! `tests/mode_switch.rs` and `tests/sim_smoke.rs`.
#![cfg(not(miri))]

use proptest::prelude::*;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_demo::render::instance::GpuInstance;
use boyko_demo::sim::bundles::ParticleBundle;
use boyko_demo::sim::components::{ParticleTag, Position, Radius, Velocity};
use boyko_demo::sim::modes::{BALL_COUNT, Mode, PARTICLE_COUNT};
use boyko_demo::sim::resources::{InputState, PhysicsParams, SimParams};
use boyko_demo::sim::runner::SimRunner;
use boyko_demo::sim::systems::physics::COLLISION_FLASH_COLOR;

/// One engine fixed step (64 Hz, Phase 20) as an f32 display delta. A power-
/// of-two fraction, so from_secs_f32 converts it EXACTLY to 15,625,000 ns —
/// each step() call below expends exactly one substep with zero remainder.
const FIXED_DT: f32 = 1.0 / 64.0;

/// Builds a headless world + runner with the sim resources (the mode_switch.rs
/// harness shape). Frame 1's `runner.step` fires the synthesized
/// `on_enter(Particles)` and spawns the particle cloud.
fn setup() -> (EcsMaster, SimRunner) {
    let mut world = EcsMaster::with_capacity(PARTICLE_COUNT, 3);
    world.insert_resource(InputState::default());
    world.insert_resource(SimParams::default());

    let pool = ThreadPoolBuilder::new().num_threads(4).build();
    let runner = SimRunner::new(pool, &mut world);
    (world, runner)
}

/// Collects `(Position, GpuInstance)` pairs in archetype row order — stable
/// across substeps because no structural change happens between them.
fn collect_particles(world: &mut EcsMaster) -> Vec<(Position, GpuInstance)> {
    let mut out = Vec::new();
    world
        .query::<(&Position, &GpuInstance), ()>()
        .for_each_chunk(|(positions, gpus): (&[Position], &[GpuInstance])| {
            out.extend(positions.iter().copied().zip(gpus.iter().copied()));
        });
    out
}

/// Collects `(Position, Radius, GpuInstance)` ball rows in archetype row
/// order. The `Radius` join restricts the rows to balls.
fn collect_balls(world: &mut EcsMaster) -> Vec<(Position, f32, GpuInstance)> {
    let mut out = Vec::new();
    world
        .query::<(&Position, &Radius, &GpuInstance), ()>()
        .for_each_chunk(
            |(positions, radii, gpus): (&[Position], &[Radius], &[GpuInstance])| {
                for ((p, r), g) in positions.iter().zip(radii).zip(gpus) {
                    out.push((*p, r.0, *g));
                }
            },
        );
    out
}

/// Bitwise equality of two `[f32; 2]` (NaN-proof, rounding-proof — the G7 bar).
fn bits_eq(a: [f32; 2], b: [f32; 2]) -> bool {
    a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits()
}

/// T3 — Particles: after exactly one further substep, every row's `prev_pos`
/// is BITWISE the prior substep's packed `pos`; rows that moved (witnessed
/// independently via `Position`, ★n11) have `pos != prev_pos`.
#[test]
fn particles_prev_is_prior_substep_pos_bitwise() {
    let (mut world, mut runner) = setup();

    // Frame 1: spawn + first substep + first pack.
    let steps = runner.step(&mut world, FIXED_DT);
    assert_eq!(steps, 1, "exact fixed-dt frame expends exactly one substep");

    let before = collect_particles(&mut world);
    assert_eq!(before.len(), PARTICLE_COUNT, "full particle set spawned");

    // Exactly one further substep (and one further pack).
    let steps = runner.step(&mut world, FIXED_DT);
    assert_eq!(steps, 1, "exact fixed-dt frame expends exactly one substep");

    let after = collect_particles(&mut world);
    assert_eq!(after.len(), PARTICLE_COUNT, "population stable across substeps");

    let mut moved = 0usize;
    for ((p0, g0), (p1, g1)) in before.iter().zip(&after) {
        // G7 binding assertion: the shuffle made prev the prior packed pos.
        assert!(
            bits_eq(g1.prev_pos, g0.pos),
            "prev_pos must be the prior substep's packed pos bitwise \
             (prev {:?} vs prior pos {:?})",
            g1.prev_pos,
            g0.pos
        );
        // ★n11: motion witnessed independently of the GPU mirror.
        if p1.x.to_bits() != p0.x.to_bits() || p1.y.to_bits() != p0.y.to_bits() {
            moved += 1;
            assert!(
                !bits_eq(g1.pos, g1.prev_pos),
                "a row whose Position moved must have pos != prev_pos"
            );
        }
    }
    // Random nonzero spawn velocities: effectively every particle moves.
    assert!(
        moved > PARTICLE_COUNT / 2,
        "integration must move most particles ({moved} of {PARTICLE_COUNT})"
    );
}

/// T4 — Physics: across substeps every ball's `prev_pos` equals the prior
/// substep's packed `pos` bitwise (so the field-granular `sync_ball_gpu` /
/// `tint_collided` writers never touched prev), and tinted rows changed
/// `color` only (pos still mirrors `Position`, scale still `radius *
/// ball_size`).
#[test]
fn physics_writers_do_not_clobber_prev_and_tint_is_color_only() {
    const SUBSTEPS: usize = 10;

    let (mut world, mut runner) = setup();

    // Frame 1 enters Particles (synthesized initial transition); queue the
    // switch and step: transition + first Physics substep (balls spawned,
    // integrated, packed — fresh rows seed prev then shuffle the same substep).
    runner.step(&mut world, FIXED_DT);
    world.set_next_state(Mode::Physics);
    runner.step(&mut world, FIXED_DT);
    assert_eq!(*world.state::<Mode>(), Mode::Physics, "state is Physics");

    let ball_size = world.resource::<PhysicsParams>().ball_size;
    let flash = GpuInstance::pack_rgba8(COLLISION_FLASH_COLOR);

    let mut snap = collect_balls(&mut world);
    assert_eq!(snap.len(), BALL_COUNT, "full ball set spawned");

    let mut tinted_seen = false;
    for substep in 0..SUBSTEPS {
        let steps = runner.step(&mut world, FIXED_DT);
        assert_eq!(steps, 1, "exact fixed-dt frame expends exactly one substep");

        let cur = collect_balls(&mut world);
        assert_eq!(cur.len(), snap.len(), "ball population stable");

        for (row, ((p, r, g), (_, _, g_before))) in cur.iter().zip(&snap).enumerate() {
            // G7 binding assertion: prev == the prior substep's packed pos —
            // neither sync_ball_gpu nor tint_collided re-shuffled/clobbered it.
            assert!(
                bits_eq(g.prev_pos, g_before.pos),
                "substep {substep}, ball {row}: prev_pos {:?} != prior packed pos {:?}",
                g.prev_pos,
                g_before.pos
            );
            if g.color == flash {
                // ★n10 tint check: color changed, NOTHING else — pos still
                // mirrors the post-solve Position and scale is still the
                // sync_ball_gpu product, both bitwise.
                tinted_seen = true;
                assert!(
                    bits_eq(g.pos, [p.x, p.y]),
                    "substep {substep}, ball {row}: tint must not move pos"
                );
                assert_eq!(
                    g.scale.to_bits(),
                    (r * ball_size).to_bits(),
                    "substep {substep}, ball {row}: tint must not touch scale"
                );
            }
        }
        snap = cur;
    }

    // 4 000 balls at ~0.8 packing fraction collide constantly: at least one
    // flash across 10 substeps is structurally guaranteed in practice.
    assert!(
        tinted_seen,
        "expected at least one collision flash across {SUBSTEPS} substeps"
    );
}

/// T6 — spawn seed: a click-path-shaped spawn (direct `create_entity`, the
/// `app.rs` `ParticleSpawner::spawn_one` mirror) has `prev_pos == pos` bitwise
/// BEFORE any substep runs — the seed funnel (D1/D8).
#[test]
fn click_path_spawn_seeds_prev_equal_to_pos() {
    const SPAWNS: usize = 16;

    let mut world = EcsMaster::with_capacity(64, 2);
    let archetype = world.bundle_archetype_id_for::<ParticleBundle>();
    let pos_id = Position::component_id();
    let vel_id = Velocity::component_id();
    let gpu_id = GpuInstance::component_id();
    let tag_id = ParticleTag::component_id();

    for i in 0..SPAWNS {
        let pos = Position {
            x: 1.25 * i as f32,
            y: -0.5 * i as f32,
        };
        let vel = Velocity { x: 3.0, y: -4.0 };
        // The exact seed app.rs's spawn_one writes.
        let gpu = GpuInstance::new([pos.x, pos.y], 0.6, [80, 160, 255, 255]);
        world
            .create_entity(
                archetype,
                &[
                    (pos_id, bytemuck::bytes_of(&pos)),
                    (vel_id, bytemuck::bytes_of(&vel)),
                    (gpu_id, bytemuck::bytes_of(&gpu)),
                    // ZST tag (Phase 22): the marker contributes no bytes.
                    (tag_id, &[]),
                ],
            )
            .expect("invariant: capacity 64 holds 16 spawns");
    }

    let rows = collect_particles(&mut world);
    assert_eq!(rows.len(), SPAWNS, "every spawn landed");
    for (p, g) in &rows {
        assert!(
            bits_eq(g.prev_pos, g.pos),
            "freshly spawned row must have prev_pos == pos (got prev {:?}, pos {:?})",
            g.prev_pos,
            g.pos
        );
        assert!(
            bits_eq(g.pos, [p.x, p.y]),
            "seed pos must mirror the spawned Position"
        );
    }
}

/// Frame-delta choices for T7: idle (0 substeps), a third of a step (0 or 1
/// substeps as the accumulator fills), and a full step (exactly 1 substep).
/// Every choice is <= the timestep, so `steps` is always 0 or 1 and the
/// per-frame property is fully observable (no unobservable intermediate pack).
fn frame_dt(choice: u8) -> f32 {
    match choice {
        0 => 0.0,
        1 => FIXED_DT / 3.0,
        _ => FIXED_DT,
    }
}

/// Collects the GpuInstance column only (T7's observable).
fn collect_gpu(world: &mut EcsMaster) -> Vec<GpuInstance> {
    let mut out = Vec::new();
    world
        .query::<&GpuInstance, ()>()
        .for_each_chunk(|chunk: &[GpuInstance]| out.extend_from_slice(chunk));
    out
}

proptest! {
    // 8 cases: each spawns the full 100 k particle cloud through a real
    // runner — "light" per the plan's T7 row (coverage over wall time).
    #![proptest_config(ProptestConfig {
        cases: 8,
        ..ProptestConfig::default()
    })]

    /// T7 — over random substep/idle frame sequences, after every pack
    /// `prev_pos` equals the previous pack's `pos` (bitwise), and frames that
    /// expended no substep leave the whole record bit-identical (the pack did
    /// not run).
    #[test]
    fn prev_equals_pos_of_previous_pack(choices in proptest::collection::vec(0u8..3, 4..12)) {
        let (mut world, mut runner) = setup();

        // Frame 1: spawn + first pack establishes the baseline snapshot.
        let steps = runner.step(&mut world, FIXED_DT);
        prop_assert_eq!(steps, 1);
        let mut snap = collect_gpu(&mut world);
        prop_assert_eq!(snap.len(), PARTICLE_COUNT);

        for &choice in &choices {
            let steps = runner.step(&mut world, frame_dt(choice));
            prop_assert!(steps <= 1, "dt <= timestep can expend at most one substep");

            let cur = collect_gpu(&mut world);
            prop_assert_eq!(cur.len(), snap.len());

            let mut violations = 0usize;
            for (now, before) in cur.iter().zip(&snap) {
                let ok = if steps == 0 {
                    // Idle frame: no pack ran; the record is untouched.
                    bits_eq(now.pos, before.pos)
                        && bits_eq(now.prev_pos, before.prev_pos)
                        && now.scale.to_bits() == before.scale.to_bits()
                        && now.color == before.color
                } else {
                    // One pack ran: prev is the previous pack's pos.
                    bits_eq(now.prev_pos, before.pos)
                };
                if !ok {
                    violations += 1;
                }
            }
            prop_assert_eq!(
                violations, 0,
                "{} rows violated the prev/pack invariant after a {}-substep frame",
                violations, steps
            );
            snap = cur;
        }
    }
}
