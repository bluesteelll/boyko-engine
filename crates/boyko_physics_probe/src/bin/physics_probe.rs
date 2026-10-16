//! `physics_probe` — IB's game-shaped probe (IB P0r).
//!
//! A binary built the way a game that ships `boyko-physics` is built: a normal-dependency
//! `[[bin]]` of a package that depends on `boyko-physics` and `boyko-ecs`, and on Windows on
//! `boyko-app`, all at default features (its `Cargo.toml` states the shape and why). Cargo
//! therefore resolves every engine library unit of this binary exactly as it does for a windowed
//! game, and `ug15 unit` (V11) compares those units with the parity runner's
//! (`crates/boyko_physics_parity`): the probe is the reference nothing in the runner's package can
//! perturb unseen.
//!
//! # Branches
//!
//! * `physics_probe <N>`, headless, on every platform: an [`App`] with [`PhysicsPlugin::new`] and
//!   Jolt's `PyramidScene` placement (1240 boxes on a static floor, the runner's `jolt` scene),
//!   stepped for exactly `N` fixed steps — one per frame, each frame driven with the fixed
//!   timestep — then one line `physics_probe: frames N bodies 1240 pose_hash 0x…`. `N` comes from
//!   argv, so nothing folds the loop away. The hash is FNV-1a 64 over every dynamic body's
//!   `RigidBody` as 13 little-endian `f32` bit patterns, in spawn order (the runner's pose
//!   record).
//! * `physics_probe --windowed`, Windows only: the same scene in the windowed host, the
//!   playground's `main` shape (`EnginePlugins::window`, [`PhysicsPlugin::new`], `App::run`). Off
//!   Windows it is a usage error. Everything only this branch uses lives in `mod windowed`, so a
//!   build without the host compiles no item it would leave unused.
//!
//! Both branches register [`PhysicsPlugin`] exactly once.
//!
//! Exit codes: 0 ok, 2 usage.

use std::process::ExitCode;

use boyko_ecs::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_physics::{
    Collider, ColliderShape, Mat3, PhysicsPlugin, Quat, RigidBody, RigidBodyBundle, RigidBodyMass,
    Simulated, Vec3,
};

/// `cBoxSize`: the pitch between neighbouring boxes in a layer (Jolt `PyramidScene.h`).
const BOX_SIZE: f32 = 2.0;
/// Half a box edge.
const HALF_BOX: f32 = 0.5 * BOX_SIZE;
/// The vertical gap between layers in Jolt's placement.
const LAYER_GAP: f32 = 0.5;
/// `cPyramidHeight`.
const PYRAMID_HEIGHT: i32 = 15;
/// The boxes Jolt's placement loop spawns at [`PYRAMID_HEIGHT`].
const PYRAMID_BODIES: usize = 1240;
/// The static floor's half extents.
const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(50.0, 1.0, 50.0);
/// Jolt's default friction.
const FRICTION: f32 = 0.2;
/// A 2 m box of mass 8.
const BOX_INV_MASS: f32 = 0.125;
/// Its inverse inertia about each principal axis.
const BOX_INV_INERTIA: f32 = 0.1875;
/// `f32` fields of one body's pose record.
const POSE_FIELDS: usize = 13;
/// FNV-1a 64 offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// The usage exit code.
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--windowed" => windowed_main(),
        [n] => match n.parse::<u64>() {
            Ok(frames) if frames > 0 => headless(frames),
            _ => usage(&format!("`{n}` is not a frame count > 0")),
        },
        _ => usage("expected one argument"),
    }
}

#[cold]
fn usage(why: &str) -> ExitCode {
    eprintln!("physics_probe: {why}\nusage: physics_probe <N>    (headless: N fixed steps, then the pose hash)\n       physics_probe --windowed (Windows: the windowed game)");
    ExitCode::from(EXIT_USAGE)
}

#[cfg(windows)]
fn windowed_main() -> ExitCode {
    windowed::run()
}

#[cfg(not(windows))]
fn windowed_main() -> ExitCode {
    usage("--windowed needs the Windows host")
}

/// The headless branch: `frames` fixed steps of the pyramid, then its pose hash.
fn headless(frames: u64) -> ExitCode {
    let mut app = App::new();
    app.add_plugin(PhysicsPlugin::new());
    let boxes = spawn_pyramid(app.world_mut());
    app.finish();
    let step = app.world().resource::<FixedTime>().timestep();
    for frame in 0..frames {
        app.update_with_delta(step);
        let ran = app.world().resource::<FixedTime>().steps_this_frame();
        assert!(ran == 1, "invariant: a frame driven with the fixed timestep runs one fixed step; frame {frame} ran {ran}");
    }
    let hash = pose_hash(app.world(), &boxes);
    println!("physics_probe: frames {frames} bodies {} pose_hash 0x{hash:016x}", boxes.len());
    ExitCode::SUCCESS
}

/// Spawns the static floor and Jolt's pyramid; returns the dynamic bodies in spawn order.
fn spawn_pyramid(world: &mut EcsMaster) -> Vec<Entity> {
    let mut bundles = Vec::with_capacity(PYRAMID_BODIES + 1);
    bundles.push(body(Vec3::new(0.0, -1.0, 0.0), false));
    // Jolt's placement loop, index for index.
    for i in 0..PYRAMID_HEIGHT {
        let lo = i / 2;
        let hi = PYRAMID_HEIGHT - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + LAYER_GAP) * i as f32,
                    -(PYRAMID_HEIGHT as f32) + BOX_SIZE * k as f32 + odd,
                );
                bundles.push(body(position, true));
            }
        }
    }
    let mut spawned = world.spawn_batch(bundles).expect("invariant: the RigidBodyBundle archetype accepts a batch of 1241");
    let boxes = spawned.split_off(1);
    assert!(boxes.len() == PYRAMID_BODIES, "invariant: Jolt's placement loop spawns {PYRAMID_BODIES} boxes, got {}", boxes.len());
    for &e in &boxes {
        world.enable::<Simulated>(e);
    }
    boxes
}

/// One box at rest; the floor is the one that is not `dynamic` (no `Simulated` bit, no mass).
fn body(position: Vec3, dynamic: bool) -> RigidBodyBundle {
    let (mass, half_extents) = if dynamic {
        (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(BOX_INV_INERTIA, BOX_INV_INERTIA, BOX_INV_INERTIA)),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: FRICTION,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        )
    } else {
        (RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: FRICTION }, FLOOR_HALF_EXTENTS)
    };
    RigidBodyBundle {
        body: RigidBody { position, linear_velocity: Vec3::ZERO, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO },
        mass,
        collider: Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 },
    }
}

/// FNV-1a 64 over every dynamic body's pose record, in spawn order.
fn pose_hash(world: &EcsMaster, boxes: &[Entity]) -> u64 {
    let mut h = FNV_OFFSET;
    for &e in boxes {
        let b = world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
        let fields: [f32; POSE_FIELDS] = [
            b.position.x,
            b.position.y,
            b.position.z,
            b.linear_velocity.x,
            b.linear_velocity.y,
            b.linear_velocity.z,
            b.rotation.x,
            b.rotation.y,
            b.rotation.z,
            b.rotation.w,
            b.angular_velocity.x,
            b.angular_velocity.y,
            b.angular_velocity.z,
        ];
        for byte in fields.iter().flat_map(|f| f.to_bits().to_le_bytes()) {
            h = (h ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// The windowed game. Every item only this branch needs is here, so a build without the host
/// (`cfg(not(windows))`) has nothing left unused.
#[cfg(windows)]
mod windowed {
    use std::process::ExitCode;

    use boyko_app::EnginePlugins;
    use boyko_ecs::App;
    use boyko_physics::PhysicsPlugin;

    /// The window's title.
    const TITLE: &str = "boyko physics probe";
    /// The window's client width.
    const WIDTH: u32 = 1280;
    /// The window's client height.
    const HEIGHT: u32 = 720;

    /// The playground's `main` shape with the pyramid in it.
    pub(super) fn run() -> ExitCode {
        let mut app = App::new();
        app.add_plugins(EnginePlugins::window(TITLE, WIDTH, HEIGHT));
        app.add_plugin(PhysicsPlugin::new());
        super::spawn_pyramid(app.world_mut());
        app.run();
        ExitCode::SUCCESS
    }
}
