//! Physics O11 SP4 — SERIAL bit-baseline gate (C1 guard byte-identity).
//!
//! This file proves the C1 per-endpoint write guard added to the SHARED leaf
//! kernels (`project_distance` / `project_volume` / `project_self_pair`) is
//! **byte-preserving** on the serial `physics_soft_step` path — the campaign
//! 0%-gate. The guard replaces an unconditional `pos[pinned] += nrm*(s*0.0)`
//! (a value-benign `+= ±0.0`) with a skip; per the plan C1-A proof, on every
//! finite-position state the two are bit-equal, so the serial soft world stays
//! byte-identical to the pre-guard SP1–SP3 behaviour.
//!
//! The oracle is a GOLDEN snapshot (`f32::to_bits` per `pos`/`vel` component)
//! captured from the pre-guard build and pinned here as a literal. The scene
//! exercises ALL THREE guarded kernels (distance edges, volume tets, and the
//! self-collision push) AND every pinned/dynamic mix that routes through the
//! guard:
//!   - dynamic–dynamic endpoints (guard true both sides),
//!   - dynamic–pinned (`inv_mass == +0.0`) endpoints (guard skips the pinned add),
//!   - a `-0.0` inverse mass (the C1-A signed-zero case: `is_dynamic_row` treats
//!     it as pinned, the guard skips it — `±0.0` are `==` under IEEE so the guard
//!     and the coloring route it identically).
//!
//! The NEGATIVE-finite inverse-mass C1-A case (out-of-contract-but-determinism-
//! safe: `is_dynamic_row(w) = w != 0.0` treats it as dynamic on BOTH paths) is
//! NOT exercised serially here — a negative `inv_mass` makes a distance/volume
//! `denom = wsum + α̃` non-positive, which trips the kernels' PRE-EXISTING
//! `debug_assert!(denom > 0.0)` (unrelated to the SP4 guard). The colored {1,N}
//! disjointness oracle (the tester's gate) covers the negative-mass routing.
//!
//! Driven via `run_system_once` on a hoisted `FunctionSystem` (no `Schedule::run`
//! deque), so the gate is Miri-clean and witnesses the kernel directly.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::physics_soft_step;
use boyko_physics::soft::SoftBody;

// ── Harness (mirrors soft_self_collision_sp3.rs) ──────────────────────────────

fn soft_driver() -> impl FnMut(&mut EcsMaster) {
    let mut sys = IntoSystem::into_system(physics_soft_step);
    move |world: &mut EcsMaster| {
        world.run_system_once(&mut sys);
    }
}

fn step_soft_n(world: &mut EcsMaster, n: usize) {
    let mut step = soft_driver();
    for _ in 0..n {
        step(world);
    }
}

fn spawn_soft(world: &mut EcsMaster, body: SoftBody) {
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, body)
        .expect("invariant: {SoftBody} archetype accepts a SoftBody");
}

fn read_soft(world: &mut EcsMaster) -> SoftBody {
    let q = world.query::<&SoftBody, ()>();
    let mut it = q.iter();
    it.next().expect("one soft body spawned").clone()
}

/// The full `(pos, vel)` bit snapshot (`to_bits` so `-0.0`/`NaN`/every ULP is
/// compared exactly).
fn full_bits(body: &SoftBody) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    (0..body.particle_count())
        .map(|i| {
            (
                body.pos_x[i].to_bits(),
                body.pos_y[i].to_bits(),
                body.pos_z[i].to_bits(),
                body.vel_x[i].to_bits(),
                body.vel_y[i].to_bits(),
                body.vel_z[i].to_bits(),
            )
        })
        .collect()
}

/// A soft body that exercises distance + volume + self-collision kernels with a
/// pinned / `-0.0` / negative-mass / dynamic endpoint mix.
///
/// Two tetrahedra sharing a face, with particle radii large enough that several
/// particles overlap (so the self-collision push fires), and a mix of inverse
/// masses across the guarded endpoints.
fn mixed_scene() -> SoftBody {
    // 5 particles: a shared-face tet pair (0,1,2,3) and (1,2,3,4).
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.7, 0.7, 0.7],
    ];
    // inv_mass mix: p0 pinned (+0.0), p1 dynamic, p2 `-0.0` (pinned per
    // is_dynamic_row — the C1-A signed-zero case), p3 dynamic, p4 dynamic.
    let inv_masses = [0.0f32, 1.0, -0.0, 0.5, 2.0];
    let edges = [(0u32, 1u32), (1, 2), (2, 3), (3, 0), (1, 4), (2, 4), (3, 4)];
    let tets = [(0u32, 1u32, 2u32, 3u32), (1, 2, 3, 4)];
    SoftBody::from_tet_mesh(
        &positions,
        &inv_masses,
        &edges,
        &tets,
        None,
        None,
        1.0e-6, // edge compliance
        1.0e-6, // tet compliance
        0.4,    // radius (cell = 0.8 <= smallest rest len 1.0; some particles overlap)
    )
    .expect("the mixed tet scene is well-formed")
}

fn install_cfg(world: &mut EcsMaster) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 4,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        self_collision_iters: 2,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
}

/// Runs the mixed scene a few steps and returns the `(pos, vel)` bit snapshot.
fn run_mixed() -> Vec<(u32, u32, u32, u32, u32, u32)> {
    let mut world = EcsMaster::new();
    install_cfg(&mut world);
    spawn_soft(&mut world, mixed_scene());
    step_soft_n(&mut world, 8);
    full_bits(&read_soft(&mut world))
}

/// The pre-guard serial golden, captured from the build IMMEDIATELY before the C1
/// guard was added (the mixed distance+volume+self-collision scene, 8 steps). The
/// guard is byte-preserving iff the post-guard run reproduces this exactly.
const SP4_SERIAL_GOLDEN: [(u32, u32, u32, u32, u32, u32); 5] = [
    (0, 0, 0, 0, 0, 0),
    (1065349880, 3112106254, 3162211885, 3131252735, 966614351, 3193757521),
    (0, 1065353216, 0, 0, 0, 0),
    (3154870860, 3108754354, 1065353286, 3186969383, 982534527, 938475519),
    (1060315321, 1060130575, 1060234920, 3142836223, 3189740671, 3180438143),
];

/// PRINTS the snapshot so the golden literal can be regenerated when the kernel
/// math legitimately changes. Run with `--nocapture`.
#[test]
fn print_serial_golden() {
    let bits = run_mixed();
    println!("SP4_SERIAL_GOLDEN = {bits:?}");
}

/// THE C1 serial bit-baseline: the guarded leaf kernels produce a `(pos, vel)`
/// snapshot BYTE-IDENTICAL to the pre-guard golden — the serial 0%-gate. Proves
/// the per-endpoint write guard `if is_dynamic_row(w) { add }` is a no-op on the
/// serial path (the pinned add it replaces was a value-benign `+= ±0.0`).
#[test]
fn serial_guard_is_byte_identical_to_pre_guard_golden() {
    let bits = run_mixed();
    assert_eq!(
        bits.as_slice(),
        SP4_SERIAL_GOLDEN.as_slice(),
        "C1 guard must be BYTE-IDENTICAL to the pre-guard serial behaviour (the 0%-gate)"
    );
}

/// Run-to-run determinism (independent of the golden): the serial mixed scene is
/// bit-stable across two fresh builds.
#[test]
fn serial_mixed_scene_is_run_to_run_bit_stable() {
    let a = run_mixed();
    let b = run_mixed();
    assert_eq!(a, b, "serial soft step must be run-to-run bit-deterministic");
}

/// The pinned (`+0.0`) and `-0.0` endpoints NEVER move (the guard removes the
/// `+= ±0.0` write entirely; the result is bit-equal to the unconditional add).
/// This is the load-bearing invariant the C1 guard preserves serially.
#[test]
fn pinned_and_signed_zero_endpoints_are_frozen() {
    let mut world = EcsMaster::new();
    install_cfg(&mut world);
    let scene = mixed_scene();
    // Snapshot the pinned (+0.0 = p0) and -0.0 (p2) start positions.
    let p0 = (scene.pos_x[0], scene.pos_y[0], scene.pos_z[0]);
    let p2 = (scene.pos_x[2], scene.pos_y[2], scene.pos_z[2]);
    spawn_soft(&mut world, scene);
    step_soft_n(&mut world, 8);
    let out = read_soft(&mut world);
    assert_eq!(
        (out.pos_x[0].to_bits(), out.pos_y[0].to_bits(), out.pos_z[0].to_bits()),
        (p0.0.to_bits(), p0.1.to_bits(), p0.2.to_bits()),
        "the +0.0-mass pinned particle must stay byte-frozen (guard removes its write)"
    );
    assert_eq!(
        (out.pos_x[2].to_bits(), out.pos_y[2].to_bits(), out.pos_z[2].to_bits()),
        (p2.0.to_bits(), p2.1.to_bits(), p2.2.to_bits()),
        "the -0.0-mass particle (pinned per is_dynamic_row) must stay byte-frozen"
    );
}
