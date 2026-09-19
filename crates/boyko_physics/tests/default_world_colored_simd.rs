//! G1 — the default physics world is the colored solve with the O7 AVX2 cohort
//! kernel on (owner decision, 2026-09-18).
//!
//! Every leg is evaluated and every failure is reported, so one run shows each leg
//! that is red rather than only the first.
//!
//! - (a) `PhysicsConfig::default().simd_solve` is `true`.
//! - (b) `add_physics_systems::<ColoredSoftStepSolver>` wires the constraint graph
//!   and the colored solve stage: `build_graph` is set, the colored solver counts a
//!   solved step per frame, contacts exist every frame, and a dropped dynamic box
//!   loses height over 10 steps. Before the change this entry registered the
//!   generic `physics_solve_step`, whose `ColoredSoftStepSolver::solve` is a no-op
//!   while the solver still claimed integration, so every body froze (B4).
//! - (c) `DefaultRigidSolver` is `ColoredSoftStepSolver` by `TypeId`, and the wired
//!   world carries `simd_solve == true` and `IntegrationMode::SolverOwned`.
//! - (d) The build has AVX2 on x86-64. With (c) that puts both inputs of the one
//!   scalar/AVX2 dispatch fork (`ColoredSoftStepSolver::solve_color_dispatch`) on:
//!   the `cfg` and the runtime flag. It does NOT observe the instruction stream.
//! - (e) Every other entry — sdf, soft(uncoupled), soft(coupled), soft_colored and
//!   scene_sync — wired with `::<DefaultRigidSolver>` passes (b)'s behavioural
//!   check, not only the `build_graph` structure: the O4 graph-only shape also has
//!   `build_graph` set, and a stage whose params fail to resolve is only seen by
//!   building and running the schedule.
//! - (f) The reference `::<SoftStepSolver>` and the foundation `::<NoopSolver>`
//!   stay on the generic solve: `build_graph` is `None`.
//!
//! A second test, [`physics_plugin_default_solver_tripwire`], carries the merge
//! obligation for the render line's `PhysicsPlugin`: it is VACUOUS on this line
//! (no `PhysicsPlugin` exists here) and goes live the moment one is merged in.
//!
//! This file spins up `boyko_threadpool` (intractable under Miri), so it is
//! `cfg(not(miri))`.

#![cfg(not(miri))]

use std::any::TypeId;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::{
    PhysicsStageKeys, add_physics_sdf, add_physics_soft, add_physics_soft_colored,
    add_physics_systems, add_physics_systems_with_scene_sync,
};
use boyko_physics::resources::{IntegrationMode, Manifolds, PhysicsConfig};
use boyko_physics::solver::{ColoredSoftStepSolver, DefaultRigidSolver, NoopSolver, SoftStepSolver};

/// Steps each wired world runs.
const STEPS: usize = 10;

/// Fixed timestep of every wired world.
const DT: f32 = 1.0 / 60.0;

/// Starting height of the dropped box — far enough above the floor that it
/// cannot land within [`STEPS`], so "it lost height" is gravity, not a contact.
const DROP_Y: f32 = 3.0;

/// A physics wiring entry with its solver type fixed.
type Wire = fn(&mut ScheduleBuilder, &mut EcsMaster) -> PhysicsStageKeys;

/// Returns the bytes of a `#[repr(C)]` POD value for the raw `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; the slice views its
    // `size_of::<T>()` bytes read-only for the duration of the borrow, which is
    // the exact layout the component pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

fn spawn_box(world: &mut EcsMaster, position: Vec3, half_extents: Vec3, inv_mass: f32) {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: if inv_mass == 0.0 { Mat3::ZERO } else { Mat3::IDENTITY },
        inv_mass,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Box { half_extents },
        layer: 1,
        mask: 1,
    };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns");
    world.enable::<Simulated>(e);
}

/// What one wired world did over [`STEPS`].
struct Outcome {
    keys: PhysicsStageKeys,
    /// Height of the dropped box before and after the run.
    drop_y: (f32, f32),
    /// `ColoredSoftStepSolver::solved_steps()` after the run, or `None` when the
    /// world holds no colored solver.
    solved_steps: Option<u64>,
    /// Frames in which the narrowphase produced at least one manifold.
    frames_with_contacts: usize,
    /// The wired world's `PhysicsConfig::simd_solve`.
    simd_solve: bool,
    /// The wired world's integration mode.
    integration: IntegrationMode,
}

/// Spawns a static floor, one box resting on it (so the solve sees contacts every
/// frame) and one box dropped from [`DROP_Y`], wires the pipeline with `wire`, and
/// runs [`STEPS`] frames.
fn run_wired(wire: Wire) -> Outcome {
    let mut world = EcsMaster::new();
    spawn_box(&mut world, Vec3::new(0.0, -0.5, 0.0), Vec3::new(10.0, 0.5, 10.0), 0.0);
    // Slight overlap with the floor so a manifold exists from the first frame.
    spawn_box(&mut world, Vec3::new(-3.0, 0.49, 0.0), Vec3::new(0.5, 0.5, 0.5), 1.0);
    spawn_box(&mut world, Vec3::new(3.0, DROP_Y, 0.0), Vec3::new(0.5, 0.5, 0.5), 1.0);

    let mut builder = ScheduleBuilder::new(serial_pool());
    let keys = wire(&mut builder, &mut world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(DT)));
    let mut schedule: Schedule = builder.build(&mut world);

    let drop_y_of = |world: &mut EcsMaster| {
        let q = world.query::<&RigidBody, ()>();
        q.iter()
            .map(|b| b.position)
            .find(|p| p.x > 1.0)
            .expect("invariant: the dropped box is spawned at x = 3")
            .y
    };
    let before = drop_y_of(&mut world);
    let mut frames_with_contacts = 0;
    for _ in 0..STEPS {
        schedule.run(&mut world);
        if !world.resource::<Manifolds>().manifolds().is_empty() {
            frames_with_contacts += 1;
        }
    }
    let after = drop_y_of(&mut world);
    Outcome {
        keys,
        drop_y: (before, after),
        solved_steps: world
            .try_resource::<ColoredSoftStepSolver>()
            .map(ColoredSoftStepSolver::solved_steps),
        frames_with_contacts,
        simd_solve: world.resource::<PhysicsConfig>().simd_solve,
        integration: *world.resource::<IntegrationMode>(),
    }
}

/// Checks that `outcome` is a world whose colored solve ran: the graph stage is
/// wired, the colored solver solved every step, contacts existed every frame, and
/// the dropped box fell.
fn check_colored_ran(label: &str, outcome: &Outcome, failures: &mut Vec<String>) {
    if outcome.keys.build_graph.is_none() {
        failures.push(format!("{label}: build_graph is None (no constraint-graph stage)"));
    }
    match outcome.solved_steps {
        Some(n) if n >= STEPS as u64 => {}
        other => failures.push(format!(
            "{label}: ColoredSoftStepSolver::solved_steps() = {other:?}, expected >= {STEPS}"
        )),
    }
    if outcome.frames_with_contacts != STEPS {
        failures.push(format!(
            "{label}: contacts in {} of {STEPS} frames (the resting box must touch every frame)",
            outcome.frames_with_contacts
        ));
    }
    let (before, after) = outcome.drop_y;
    if after >= before {
        failures.push(format!(
            "{label}: the dropped box did not fall (y {before} -> {after}); the bodies are frozen"
        ));
    }
}

#[test]
fn default_world_wires_colored_simd() {
    let mut failures: Vec<String> = Vec::new();

    // (a) the owner's flag default.
    if !PhysicsConfig::default().simd_solve {
        failures.push("(a) PhysicsConfig::default().simd_solve is false".to_owned());
    }

    // (b) the colored solver type argument wires the colored pipeline.
    let colored = run_wired(add_physics_systems::<ColoredSoftStepSolver>);
    check_colored_ran("(b) add_physics_systems::<ColoredSoftStepSolver>", &colored, &mut failures);

    // (c) the alias names the colored solver, and its world runs the SIMD solve and
    // owns integration.
    if TypeId::of::<DefaultRigidSolver>() != TypeId::of::<ColoredSoftStepSolver>() {
        failures.push("(c) DefaultRigidSolver is not ColoredSoftStepSolver".to_owned());
    }
    let default_world = run_wired(add_physics_systems::<DefaultRigidSolver>);
    if !default_world.simd_solve {
        failures.push("(c) the DefaultRigidSolver world has simd_solve == false".to_owned());
    }
    if default_world.integration != IntegrationMode::SolverOwned {
        failures.push(format!(
            "(c) the DefaultRigidSolver world has {:?}, expected SolverOwned",
            default_world.integration
        ));
    }

    // (d) the compile-time input of the dispatch fork — the exact `cfg` it tests.
    if !cfg!(all(target_arch = "x86_64", target_feature = "avx2")) {
        failures.push(
            "(d) this build lacks x86_64 + avx2, so the AVX2 cohort kernel is compiled out \
             (the x86-64-v3 baseline in .cargo/config.toml did not reach it)"
                .to_owned(),
        );
    }

    // (e) every other entry, on the default solver, runs the colored solve.
    let entries: [(&str, Wire); 5] = [
        ("(e) add_physics_sdf::<DefaultRigidSolver>", add_physics_sdf::<DefaultRigidSolver>),
        ("(e) add_physics_soft::<DefaultRigidSolver>(false)", |b, w| {
            add_physics_soft::<DefaultRigidSolver>(b, w, false)
        }),
        ("(e) add_physics_soft::<DefaultRigidSolver>(true)", |b, w| {
            add_physics_soft::<DefaultRigidSolver>(b, w, true)
        }),
        (
            "(e) add_physics_soft_colored::<DefaultRigidSolver>",
            add_physics_soft_colored::<DefaultRigidSolver>,
        ),
        (
            "(e) add_physics_systems_with_scene_sync::<DefaultRigidSolver>",
            add_physics_systems_with_scene_sync::<DefaultRigidSolver>,
        ),
    ];
    for (label, wire) in entries {
        check_colored_ran(label, &run_wired(wire), &mut failures);
    }

    // (f) the reference and the foundation solvers keep the generic solve.
    let generic: [(&str, Wire); 2] = [
        ("(f) add_physics_systems::<SoftStepSolver>", add_physics_systems::<SoftStepSolver>),
        ("(f) add_physics_systems::<NoopSolver>", add_physics_systems::<NoopSolver>),
    ];
    for (label, wire) in generic {
        let outcome = run_wired(wire);
        if outcome.keys.build_graph.is_some() {
            failures.push(format!("{label}: build_graph is set; it must stay uncolored"));
        }
        if outcome.solved_steps.is_some() {
            failures.push(format!("{label}: a ColoredSoftStepSolver resource was inserted"));
        }
    }

    assert!(failures.is_empty(), "G1 legs red:\n  {}", failures.join("\n  "));
}

/// The line prefixes that declare a `PhysicsPlugin` type.
const DECLARATIONS: [&str; 3] = [
    "pub struct PhysicsPlugin",
    "pub(crate) struct PhysicsPlugin",
    "struct PhysicsPlugin",
];

/// Collects every `.rs` file under `dir`, recursively.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("invariant: directory entry is readable").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// The merge obligation for the render line's App-level `PhysicsPlugin`, made
/// mechanical (critique W1).
///
/// On the render line `PhysicsPlugin<S = SoftStepSolver>` and its `new()` /
/// `Default` build the reference solver, so a merge that keeps them would leave
/// every App on the scalar reference while every test here stays green. This test
/// scans this crate's source for a `PhysicsPlugin` declaration and, when one
/// exists, requires its default type parameter to be `DefaultRigidSolver` and no
/// `new()` / `Default` impl to be pinned to `SoftStepSolver`.
///
/// ⚠ VACUOUS on this line: no `PhysicsPlugin` is declared here, so the test prints
/// that and passes. It goes live, without an edit, the moment one is merged in. It
/// checks the declaration's text, not the behaviour; the behavioural leg is G1(e)
/// re-run through `PhysicsPlugin::new()` after the merge.
#[test]
fn physics_plugin_default_solver_tripwire() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&src, &mut files);

    // The walk itself must have found this crate's wiring, or "no declaration"
    // would be an empty scan rather than a finding.
    let plugin_rs = std::fs::read_to_string(src.join("plugin.rs"))
        .expect("invariant: the physics crate has src/plugin.rs");
    assert!(
        files.iter().any(|f| f.ends_with("plugin.rs"))
            && plugin_rs.contains("pub fn add_physics_systems<"),
        "the source walk did not reach src/plugin.rs's add_physics_systems"
    );

    let mut declarations = Vec::new();
    let mut failures = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", file.display()));
        let mut offset = 0;
        for (n, raw) in text.split_inclusive('\n').enumerate() {
            let line_start = offset;
            offset += raw.len();
            let t = raw.trim();
            let at = format!("{}:{}", file.display(), n + 1);
            let is_decl = DECLARATIONS.iter().any(|p| t.starts_with(p));
            if is_decl {
                // The header runs to the body or the terminator, so a generic list
                // split over several lines is read whole.
                let rest = &text[line_start..];
                let header = &rest[..rest.find(['{', ';']).unwrap_or(rest.len())];
                if !header.contains("= DefaultRigidSolver") {
                    failures.push(format!(
                        "{at}: `{}` — the default type parameter must be DefaultRigidSolver",
                        header.split_whitespace().collect::<Vec<_>>().join(" ")
                    ));
                }
                declarations.push(at.clone());
            }
            if t.starts_with("impl PhysicsPlugin<SoftStepSolver>")
                || t.starts_with("impl Default for PhysicsPlugin<SoftStepSolver>")
            {
                failures.push(format!(
                    "{at}: `{}` — PhysicsPlugin::new()/Default must build DefaultRigidSolver",
                    t.trim_end()
                ));
            }
        }
    }

    if declarations.is_empty() {
        eprintln!(
            "physics_plugin_default_solver_tripwire: VACUOUS — no PhysicsPlugin is declared in \
             {} ({} files scanned); it goes live when the render line's PhysicsPlugin is merged",
            src.display(),
            files.len()
        );
    }
    assert!(
        failures.is_empty(),
        "PhysicsPlugin default is not the colored solve:\n  {}",
        failures.join("\n  ")
    );
}
