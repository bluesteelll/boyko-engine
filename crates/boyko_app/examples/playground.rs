//! The interactive PHYSICS playground: fly, shoot, and knock a stack over.
//!
//! The first windowed scene in this repo that runs the in-house rigid solver
//! ([`boyko_physics`]) under the host — via
//! [`PhysicsPlugin`](boyko_physics::PhysicsPlugin), the `App`-facing wiring of the
//! pipeline the determinism tests drive by hand. Everything here is authored the
//! ordinary way: ECS spawns + `EnginePlugins` + `FlyCameraPlugin` + `PhysicsPlugin`,
//! with real PBR material folders off disk.
//!
//! Run (release — a 120 Hz solve over ~90 bodies is not a debug-build workload):
//!
//! ```text
//! cargo run --release -p boyko-app --example playground
//! ```
//!
//! # Controls
//!
//! * `W`/`A`/`S`/`D` — fly; `Space`/`E` — rise; `Left Ctrl`/`Q` — descend; mouse — look.
//! * **LMB — shoot a ball** (hold for automatic fire, one every
//!   [`SHOT_INTERVAL`] s). The newest [`MAX_BALLS`] balls live; the oldest is
//!   despawned to make room.
//! * `R` — rebuild the pyramid and clear every ball.
//! * `Esc` — quit.
//!
//! The same list is drawn on screen — see "the on-screen key hints" below.
//!
//! # What is in the scene
//!
//! * A textured arena — floor, four walls, four pillars, a ramp — all STATIC bodies
//!   (`RigidBody` + `inv_mass == 0` + `Simulated` clear, the documented immovable
//!   contact-surface shape).
//! * A five-level pyramid of [`PYRAMID_CUBES`] dynamic cubes, plus a few loose
//!   crates, all with shape-derived inertia tensors.
//! * A sun with CSM cascades, a sky fill, and a point + spot pair carrying
//!   `CastsPunctualShadow`.
//!
//! Every dynamic body carries the `GpuTransform3D` interpolation pair, so the 120 Hz
//! fixed solve is drawn smoothly at the render rate (the engine's `FixedSet::Gameplay`
//! -> `FixedSet::Snapshot` seam, which `PhysicsPlugin` joins).
//!
//! # Materials
//!
//! Read from `assets/materials/<name>/pbr/` (repo-relative, resolved off
//! `CARGO_MANIFEST_DIR`) through [`load_material_folder`] — the same albedo /
//! normal / metallic-roughness / AO convention `tests/pbr_material_showcase.rs`
//! documents. A missing slot falls back to the material's scalar channel, so a
//! folder that is absent entirely still renders (untextured), never panics.
//!
//! # The on-screen key hints, and why they are a QUAD
//!
//! The hint panel is a camera-parented textured quad, not a `boyko_ui` widget. That is a
//! measured constraint, not a preference: the ECS UI stack exists and is complete, but it
//! **cannot reach the windowed screen today** — `boyko_app` names no `boyko_ui`, and the
//! windowed entry point (`Renderer::render_gbuffer_frame` → `record_gbuffer`) has no `ui`
//! parameter at all. The only path that records a UI pass is `present_sampled(.., ui:
//! Option<&UiPass>)`, which nothing outside `boyko_render`'s own GPU goldens calls.
//! Wiring it up is an engine change (an RHI signature + the frame loop's borrow split +
//! a layering decision, since `boyko_ui` and `boyko_render` are deliberately forbidden to
//! name each other outside test targets), not a scene change.
//!
//! So the panel goes through the path that IS live: the text is rasterised ONCE at
//! startup from the engine's own MTSDF bake (`boyko_fontbake::bake_font`) into an RGBA8
//! image, uploaded as an ordinary texture, and drawn on a quad that rides the camera.
//! The same image feeds the albedo AND emissive slots, so the glyphs glow (the deferred
//! resolve multiplies the material's emissive by the emissive texture's luminance) and
//! the panel stays readable in shadow. `BOYKO_HUD=off` removes it.
//!
//! # The two downloaded models
//!
//! `assets/models/anime_girl/` and `assets/models/alien/` (both from rigmodels.com, both
//! `.glb`) stand in the arena by default; `BOYKO_MODELS=off` leaves them out. They are
//! decoded by the same in-house `.glb` reader everything else uses — the girl is RIGGED,
//! so she comes in through `GlbMeshLoader::decode_static_pose` (her bind pose; the engine
//! plays no animation), and the alien's primitives mix `u8` and `u16` indices, which the
//! decoder learned to read for exactly this file.
//!
//! They arrive TEXTURED, through `GlbMeshLoader::decode_scene_static_pose`: one entity per
//! glTF primitive, each with the material that primitive names, and the file's embedded
//! images decoded and uploaded as ordinary engine textures. A glTF splits primitives BY
//! MATERIAL, so this is also why they are several entities and not one — concatenating
//! them (what `AssetLoader::decode` does) is precisely the step that would throw the
//! material assignment away.
//!
//! One caveat, stated because it is visible: images are decoded through the in-house PNG
//! decoder, and this engine has no JPEG decoder. The anime model's 28 images are all PNG,
//! so it is fully textured; the alien mixes PNG and JPEG, and the JPEG slot falls back to
//! its material's base-colour factor with a printed line saying so.
//!
//! # Bring your own mesh (`BOYKO_MESH`)
//!
//! `BOYKO_MESH=<path>` drops one downloaded model into the arena. Both in-house
//! decoders are wired: **`.glb`** (binary glTF 2.0 — `boyko_render::loaders::GlbMeshLoader`)
//! and **`.obj`** (`boyko_render::loaders::ObjMeshLoader`). Prefer `.glb`: it carries an
//! index buffer, UVs and real tangents, and it bakes each node's transform. Uncompressed
//! only — Draco / meshopt, sparse accessors, skins, morph targets and animation are
//! refused loudly rather than silently half-decoded.
//!
//! | var | meaning | default |
//! |-----|---------|---------|
//! | `BOYKO_MESH` | path to a `.glb` / `.obj` | none (no prop) |
//! | `BOYKO_MESH_POS` | `x,y,z` — where it STANDS (`y` is the ground it rests on) | `-3,0,-1` |
//! | `BOYKO_MESH_SCALE` | uniform scale | `1.0` |
//! | `BOYKO_MESH_YAW` | yaw in degrees | `0` |
//! | `BOYKO_MESH_MAT` | material folder under `assets/materials/` | the steel tint |
//! | `BOYKO_MESH_DYNAMIC` | make it a shootable rigid body instead of scenery | unset |
//!
//! The model is RE-CENTRED on its own bounding box at load and then lifted so its lowest
//! vertex rests on `BOYKO_MESH_POS`'s `y`. A downloaded model's origin is as often its
//! centre as its feet as some point off in space, and re-centring makes all three place
//! identically — it also makes the collider exact instead of conservative.
//!
//! **Collision is that bounding box, not the triangles.** The solver's shapes are sphere
//! and oriented box (`ColliderShape`) — this engine has no triangle-mesh collider — so a
//! crate, a barrel or a pillar collides exactly, while a chair collides as the box around
//! its legs.
//!
//! # Headless capture
//!
//! `BOYKO_HOST_DUMP=<path.bmp>` captures one settled frame (and then exits), and
//! `BOYKO_WINDOW_FRAMES=<n>` exits after `n` frames — both inherited from the runner.
//! Pair either with this scene's own `BOYKO_LOCK_LOOK=1`, which zeroes the mouse-look
//! sensitivity so a capture with nobody at the keyboard frames the scene the scene
//! chose, not wherever stray OS mouse motion left the camera.

use core::f32::consts::PI;
use core::time::Duration;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Mut, Query, With};
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::asset::AssetLoader;
use boyko_ecs::ecs::core::time::Time;
use boyko_input::{GameplaySet, MouseButton, PhysicalInput};
use boyko_macros::{Bundle, Component, Resource};
use boyko_math::Mat3;
use boyko_physics::PhysicsPlugin;
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::resources::{BroadphaseKind, PhysicsConfig};
use boyko_fontbake::{AtlasKind, BakedFont, TtfFace, bake_font};
use boyko_render::loaders::{GlbMeshLoader, ObjMeshLoader, PngTextureLoader};
use boyko_render::mesh::Vertex;
use boyko_render::mesh_data::MeshData;
use boyko_render::texture_data::TextureData;
use boyko_render::{
    BindlessTextureTable, ColorSpace, Material, MaterialGpu, MaterialTextures, TextureAssetsExt,
    TextureGpu, generate_tangents, load_material_folder, upload_texture_2d,
};
use boyko_scene::CameraSet;

// ════════════════════════════════════════════════════════════════════════════════
// Tunables
// ════════════════════════════════════════════════════════════════════════════════

/// The sun direction TO the light (the engine's familiar showcase sun).
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

/// Window size, and the camera aspect derived from it.
const WIN_W: u32 = 1280;
/// See [`WIN_W`].
const WIN_H: u32 = 800;

/// Fixed-step rate. 120 Hz (rather than the engine default 64 Hz) halves `h` for the
/// stacking contacts, which is what keeps a five-level pyramid from breathing at rest.
const FIXED_HZ: f64 = 120.0;

/// Live projectile ceiling. A ring, not a limit that stops the gun: firing past it
/// despawns the oldest ball. Also the instance budget's guard rail — the whole scene
/// stays far under the host's 1024-instance ring.
const MAX_BALLS: usize = 64;

/// Minimum seconds between shots while LMB is held (a single click always fires
/// immediately — the cooldown starts at zero).
const SHOT_INTERVAL: f32 = 0.11;

/// Projectile radius (world units) and muzzle speed (units/s).
///
/// MEASURED, not guessed. The gun fires along the camera's own forward, so the aim and
/// the arc compound: at 28 m/s from the original `-0.18` rad default pose, the ball
/// ate 2.25 m of aim plus 1.0 m of gravity over the 12.5 m to the stack and hit the
/// FLOOR every time, a metre and a half short. 30 m/s with the near-level default pose
/// below arrives with ~0.3 m of total drop.
///
/// A substep advances the ball `30 / (120 * 4) = 0.063` m — under a third of its radius,
/// so the sphere cannot tunnel through a 0.5 m cube between contacts.
const BALL_RADIUS: f32 = 0.22;
/// See [`BALL_RADIUS`].
const BALL_SPEED: f32 = 30.0;
/// Projectile mass. Heavier than a pyramid cube on purpose — a ball that bounces off
/// the stack is a worse toy than one that goes through it. MEASURED at 1.4: the stack
/// topples and slides; at 2.4 it detonated, throwing cubes five metres up.
const BALL_MASS: f32 = 1.4;

/// Pyramid geometry: `PYRAMID_LEVELS` square layers of `CUBE_SIZE` cubes, `L`-th layer
/// (from the bottom, 0-based) being `(PYRAMID_LEVELS - L)` on a side.
const PYRAMID_LEVELS: usize = 5;
/// See [`PYRAMID_LEVELS`].
const CUBE_SIZE: f32 = 0.5;
/// 5² + 4² + 3² + 2² + 1² = 55 — the square pyramidal number, computed rather than
/// typed so the array size and the spawn loop cannot disagree.
const PYRAMID_CUBES: usize = {
    let mut n = 0;
    let mut level = 0;
    while level < PYRAMID_LEVELS {
        let side = PYRAMID_LEVELS - level;
        n += side * side;
        level += 1;
    }
    n
};
/// Loose crates scattered around the arena (dynamic, 1 m, brick).
const CRATES: usize = 6;
/// Every dynamic prop this scene can own at once, pyramid + crates.
const MAX_PROPS: usize = PYRAMID_CUBES + CRATES;

/// The key-hint lines drawn on the HUD panel, top to bottom.
///
/// ASCII only: the baked glyph set is exactly the characters that appear here (see
/// [`bake_hint_texture`]), and the fixture face is a Latin font.
const HUD_LINES: [&str; 3] = [
    "WASD MOVE    MOUSE LOOK    SPACE/E UP    CTRL/Q DOWN",
    "LMB SHOOT    R RESET    ESC QUIT",
    "BOYKO ENGINE PLAYGROUND",
];

/// How far in front of the eye the panel rides.
///
/// The panel's SIZE is not authored — it is derived from the rasterised image so that one
/// texel lands on one screen pixel (see [`world_per_pixel`]). Authoring a width instead
/// was the bug: at 1.05 m the 712-texel image was minified to 659 pixels, the sampler
/// dropped into mip 1, and the text went soft.
const HUD_DISTANCE: f32 = 1.15;
/// Vertical offset from the view centre, world units at [`HUD_DISTANCE`] (negative = down).
const HUD_DROP: f32 = -0.42;
/// Glyph em size in texels in the rasterised panel image — and, because the quad is sized
/// 1:1 against it, the glyph's height in SCREEN pixels at the default window size.
const HUD_GLYPH_PX: f32 = 28.0;

/// The two downloaded models in `assets/models/` (git-ignored), as
/// `(folder, target height in metres, x, z, yaw°)`.
///
/// The HEIGHT is authored, not the scale: a downloaded model arrives in whatever units its
/// author used (these two differ by a factor of ~30), so the loader measures the bounding
/// box and derives the scale that makes the model that tall. Authoring a scale factor
/// instead would mean re-tuning it per download.
const SHOWCASE_MODELS: [(&str, f32, f32, f32, f32); 2] = [
    ("anime_girl", 1.70, -2.6, -2.2, 25.0),
    ("alien", 2.05, 2.9, -2.4, -20.0),
];

/// Arena half-extent (the floor is `2 * ARENA_HALF` on a side) and wall height.
const ARENA_HALF: f32 = 16.0;
/// See [`ARENA_HALF`].
const WALL_HEIGHT: f32 = 6.0;

// ════════════════════════════════════════════════════════════════════════════════
// Scene state
// ════════════════════════════════════════════════════════════════════════════════

/// Marks the camera-parented hint panel (see the module doc). One entity; the follow
/// systems need a filter that names it and nothing else.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HudPanel;

/// Where the hint panel should sit this frame — the hand-off between the camera READ and
/// the panel WRITE (see [`hud_read_camera`] for why they cannot be one system).
#[derive(Resource, Clone, Copy, Debug, Default)]
struct HudTarget {
    /// World position of the panel's centre.
    translation: Vec3,
    /// Panel orientation (the camera's, so the quad faces the eye).
    rotation: Quat,
}

/// A physics-only static body: pose + collider, no mesh.
///
/// The showcase models are several drawable entities (one per glTF primitive) but ONE
/// collider, so the body cannot ride on a drawable — it is its own entity.
#[derive(Bundle)]
struct StaticBodyBundle {
    /// Local pose (the model's placement).
    transform: Transform,
    /// Cached world pose, filled by `propagate_transforms`.
    global: GlobalTransform,
    /// HOT integrator state — `inv_mass == 0`, never integrated.
    body: RigidBody,
    /// COLD mass / material.
    mass: RigidBodyMass,
    /// The box around the whole model.
    collider: Collider,
}

/// A one-field bundle attaching the `GpuTransform3D` interpolation pair.
///
/// A dense component's self-`Bundle` is deliberately suppressed by the derive, so a
/// named bundle is the attach path (the same shape `examples/bounce.rs` uses).
#[derive(Bundle)]
struct PairOnly {
    /// The prev/curr pose pair, seeded `prev == curr`.
    pair: GpuTransform3D,
}

/// The scene's runtime handles + spawned-entity registry.
///
/// Mesh and material assets are uploaded ONCE at boot (the host drains
/// `upload_mesh_assets` / `upload_material_assets` after startup), so every asset is
/// registered in [`setup`] and only ENTITIES are spawned later. This resource is how
/// the shoot/reset systems reach those boot-time handles.
///
/// Both entity registries are fixed-size arrays, not `Vec`s: the ceilings are
/// compile-time constants, so the storage is preallocated and a shot costs no
/// allocation (Principle 5).
#[derive(Resource)]
struct Playground {
    /// Projectile mesh (a UV sphere with a generated tangent basis).
    ball_mesh: MeshHandle,
    /// Projectile material (polished gold).
    ball_material: MaterialHandle,
    /// Pyramid-cube mesh ([`CUBE_SIZE`] on a side).
    cube_mesh: MeshHandle,
    /// Pyramid-cube material (gold).
    cube_material: MaterialHandle,
    /// Loose-crate mesh (1 m on a side).
    crate_mesh: MeshHandle,
    /// Loose-crate material (brick).
    crate_material: MaterialHandle,
    /// The live projectile ring; `next_ball` is the slot the next shot claims.
    balls: [Option<Entity>; MAX_BALLS],
    /// See [`Playground::balls`].
    next_ball: usize,
    /// Every dynamic prop currently alive (pyramid cubes + crates), so `R` can
    /// despawn exactly what it spawned — by `Entity`, generation included.
    props: [Option<Entity>; MAX_PROPS],
    /// Seconds left before the gun may fire again (see [`SHOT_INTERVAL`]).
    cooldown: f32,
}

impl Default for Playground {
    fn default() -> Self {
        Self {
            ball_mesh: MeshHandle(0),
            ball_material: MaterialHandle(0),
            cube_mesh: MeshHandle(0),
            cube_material: MaterialHandle(0),
            crate_mesh: MeshHandle(0),
            crate_material: MaterialHandle(0),
            balls: [None; MAX_BALLS],
            next_ball: 0,
            props: [None; MAX_PROPS],
            cooldown: 0.0,
        }
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko playground", WIN_W, WIN_H));
    // Interactive input + fly camera (also the source of `PhysicalInput`, which the
    // gun reads for its LMB edge).
    app.add_plugin(FlyCameraPlugin);
    // The rigid solve, in `CoreSchedule::Fixed`, joined to `FixedSet::Gameplay` so the
    // engine's interpolation snapshot observes each substep's POST-solve pose.
    app.add_plugin(PhysicsPlugin::new());
    // Tune the config the plugin just inserted, FIELD BY FIELD rather than by inserting
    // a fresh one over it. `PhysicsPlugin::build` writes `colored` / `soft_body` /
    // `soft_rigid_coupling` / `broadphase` to match the pipeline it registered, and a
    // whole-value insert here would silently reset them to `PhysicsConfig::default()` —
    // a scene that later asked for `.soft()` would get the soft STAGE with the soft
    // FLAG off, which is a no-op nobody would think to look for.
    {
        let cfg = app.world_mut().resource_mut::<PhysicsConfig>();
        cfg.gravity = Vec3::new(0.0, -9.81, 0.0);
        // MEASURED on this machine (144 Hz, 700-frame runs, quiet box). 8 substeps with
        // the all-pairs broadphase: median frame 7.02 ms, p95 11.66 ms, 5.6 % of frames
        // over 1.5x the median — i.e. one dropped vsync interval in eighteen, which is
        // exactly what "small jitter" looks like. 4 substeps + the O2 grid broadphase:
        // median 6.94 ms (the refresh interval itself), p95 8.72 ms, 3.1 % — and the
        // stack still settles to a standstill (apex drift 1.7e-4 m over 0.25 s).
        // ⚠ Those numbers were taken on the reference `SoftStepSolver`, which
        // `PhysicsPlugin::new()` built until the A8 merge made it the default world's
        // colored solve (`DefaultRigidSolver`, owner decision 2026-09-18). They have not
        // been re-measured on the colored solve.
        cfg.substeps = 4;
        // The grid's candidate set is bit-identical to all-pairs (the O2 campaign gate),
        // so this is pure cost, never a behaviour change.
        cfg.broadphase = BroadphaseKind::Grid;
    }
    app.set_fixed_timestep(Duration::from_secs_f64(1.0 / FIXED_HZ));

    // Both shadow systems default DISABLED (the 0%-gate) — arm them.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });

    app.insert_resource(Playground::default());
    app.insert_resource(HudTarget::default());
    app.add_startup_system(setup);
    // Both gameplay systems join `GameplaySet`, which `InputPlugin` orders AFTER its
    // `update_action_state` ingest — so they read THIS frame's input snapshot.
    app.add_systems_cfg(|b| {
        b.add_system(shoot_system).in_set(GameplaySet);
        b.add_system(reset_system).in_set(GameplaySet);
        // The panel rides the camera, so it must be placed AFTER the fly controller has
        // written this frame's camera `Transform` and BEFORE `propagate_transforms` /
        // `resolve_active_camera` recompose the world poses — exactly the gap between the
        // two camera sets. Set-to-set edges, because the controller lives in another
        // plugin whose `SystemKey` is not visible here.
        let hud_read = b
            .add_system(hud_read_camera)
            .after_set(CameraSet::Control)
            .before_set(CameraSet::Resolve)
            .key();
        b.add_system(hud_place_panel)
            .after(hud_read)
            .after_set(CameraSet::Control)
            .before_set(CameraSet::Resolve);
    });
    app.run();
}

// ════════════════════════════════════════════════════════════════════════════════
// Startup — every mesh, material and static body
// ════════════════════════════════════════════════════════════════════════════════

/// Builds the whole scene. Startup runs WITH the device present, which is what makes
/// the mesh/texture registration legal here (and only here).
#[allow(clippy::needless_pass_by_value)]
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
    mut pg: ResMut<Playground>,
) {
    // ── Materials: the three real PBR folders in `assets/materials/`, minted into four
    //    materials — the crates re-tint the industrial set rather than re-uploading it.
    let industrial = load_slots(&mut textures, &mut bindless, dev.get(), "industrial-walls");
    let brick = load_slots(&mut textures, &mut bindless, dev.get(), "alley-brick-wall");
    let gold = load_slots(&mut textures, &mut bindless, dev.get(), "light-gold");
    let floor_material = material_from(&mut materials, industrial, [1.0, 1.0, 1.0, 1.0], 0.0, 0.85);
    let brick_material = material_from(&mut materials, brick, [1.0, 1.0, 1.0, 1.0], 0.0, 0.92);
    let gold_material = material_from(&mut materials, gold, [1.0, 1.0, 1.0, 1.0], 1.0, 0.28);
    // Steel-blue crates: same plates as the floor, tinted and polished so a loose prop
    // never reads as another chunk of brick wall at a glance.
    let crate_material =
        material_from(&mut materials, industrial, [0.78, 0.84, 0.98, 1.0], 0.25, 0.45);
    // CLAY: untextured, mid-grey, matte. The downloaded models carry their own UVs but the
    // engine imports no glTF textures yet, so anything textured would tile a wall pattern
    // across a face — a clay render says "no textures" instead of pretending.
    let clay_material = materials.add(Material::new([0.62, 0.60, 0.58, 1.0], 0.0, 0.72, 0.4, [0.0; 3], 0));
    let clay_material = MaterialHandle::from_handle(clay_material);
    materials.pin(u32::from(clay_material.0));

    // ── Meshes. The UV scale is tiles PER METRE, so texel density is a property of the
    //    material rather than of each mesh's size: the 32 m floor at 0.5 lays a 2 m
    //    plate, and the 0.5 m cube at 2.0 lays one tile per face — instead of one
    //    texture sheet stretched over whatever the object happens to measure.
    let floor_half = Vec3::new(ARENA_HALF, 0.5, ARENA_HALF);
    let floor_mesh = register_box(&mut meshes, dev.get(), floor_half, 0.5);
    let wall_ns = Vec3::new(ARENA_HALF, WALL_HEIGHT * 0.5, 0.5);
    let wall_ns_mesh = register_box(&mut meshes, dev.get(), wall_ns, 0.5);
    let wall_ew = Vec3::new(0.5, WALL_HEIGHT * 0.5, ARENA_HALF);
    let wall_ew_mesh = register_box(&mut meshes, dev.get(), wall_ew, 0.5);
    let pillar_half = Vec3::new(0.6, 2.5, 0.6);
    let pillar_mesh = register_box(&mut meshes, dev.get(), pillar_half, 0.7);
    let ramp_half = Vec3::new(3.0, 0.25, 2.0);
    let ramp_mesh = register_box(&mut meshes, dev.get(), ramp_half, 0.7);
    let cube_half = Vec3::new(CUBE_SIZE * 0.5, CUBE_SIZE * 0.5, CUBE_SIZE * 0.5);
    let cube_mesh = register_box(&mut meshes, dev.get(), cube_half, 2.0);
    let crate_half = Vec3::new(0.5, 0.5, 0.5);
    let crate_mesh = register_box(&mut meshes, dev.get(), crate_half, 1.0);
    let (ball_verts, ball_indices) = uv_sphere(BALL_RADIUS, 24, 32);
    let ball_mesh = meshes.register_mesh(dev.get(), &ball_verts, &ball_indices);

    // ── PIN every asset (asset-streaming F2, `Assets::pin`). This is load-bearing, and
    //    the bug it fixes is invisible until you delete something:
    //
    //    the mesh/material tables are REFCOUNTED by carrier presence — a `MeshHandle`
    //    inserted on an entity is `+1`, its despawn `-1` — and a row that reaches zero
    //    goes `Retiring` and is then freed. `inc_ref` on a `Retiring` row returns
    //    `false` ("a Retiring slot's refcount can never rise again",
    //    `assets.rs:840`), so the row NEVER comes back.
    //
    //    A handle cached in a resource is NOT a reference. MEASURED before this pin:
    //    pressing `R` despawned all 55 cubes at once, the cube mesh + gold material
    //    retired mid-flush, and the 55 freshly-spawned cubes drew NOTHING — the census
    //    still counted 55 simulated bodies while the screen showed bare floor. Same for
    //    the ball mesh: after one reset the gun fired invisible bullets.
    //
    //    Every asset here is scene-lifetime, so every one is pinned. The runner pins
    //    the default material slot 0 the same way (`boyko_app/src/runner.rs:342`).
    for mesh in [
        floor_mesh,
        wall_ns_mesh,
        wall_ew_mesh,
        pillar_mesh,
        ramp_mesh,
        cube_mesh,
        crate_mesh,
        ball_mesh,
    ] {
        meshes.pin(mesh.0);
    }
    for material in [floor_material, brick_material, gold_material, crate_material] {
        materials.pin(u32::from(material.0));
    }

    pg.ball_mesh = ball_mesh;
    pg.ball_material = gold_material;
    pg.cube_mesh = cube_mesh;
    pg.cube_material = gold_material;
    pg.crate_mesh = crate_mesh;
    pg.crate_material = crate_material;

    // ── The arena. The floor slab's TOP face sits at y == 0 (centre at -half.y), so
    //    the whole scene can be authored against a y == 0 ground plane.
    spawn_static_box(
        &mut commands,
        floor_mesh,
        floor_material,
        floor_half,
        Transform::from_translation(Vec3::new(0.0, -floor_half.y, 0.0)),
        false,
    );
    for (mesh, half, x, z) in [
        (wall_ns_mesh, wall_ns, 0.0, -ARENA_HALF),
        (wall_ns_mesh, wall_ns, 0.0, ARENA_HALF),
        (wall_ew_mesh, wall_ew, -ARENA_HALF, 0.0),
        (wall_ew_mesh, wall_ew, ARENA_HALF, 0.0),
    ] {
        spawn_static_box(
            &mut commands,
            mesh,
            brick_material,
            half,
            Transform::from_translation(Vec3::new(x, WALL_HEIGHT * 0.5, z)),
            true,
        );
    }
    for (x, z) in [(-9.0, -9.0), (9.0, -9.0), (-9.0, 9.0), (9.0, 9.0)] {
        spawn_static_box(
            &mut commands,
            pillar_mesh,
            brick_material,
            pillar_half,
            Transform::from_translation(Vec3::new(x, pillar_half.y, z)),
            true,
        );
    }
    // A tilted slab to shoot balls up and over the stack.
    spawn_static_box(
        &mut commands,
        ramp_mesh,
        floor_material,
        ramp_half,
        Transform {
            translation: Vec3::new(7.5, 0.9, 2.5),
            rotation: quat_rot_z(-0.28),
            scale: Vec3::ONE,
        },
        true,
    );

    // ── The dynamic props.
    spawn_props(&mut commands, &mut pg);

    // ── Lighting: sun + CSM, a sky fill, and BOTH punctual shadow kinds.
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.95, 0.88], 4.6),
    });
    // A cooler, dimmer fill than the key: enough that the shaded sides of the gold
    // read, dim enough that the sun still carves the stack out of the floor.
    commands.spawn(SkyLight::new([0.30, 0.36, 0.46], [0.14, 0.13, 0.12]));
    commands
        .spawn(PointLightObject {
            transform: Transform::from_translation(Vec3::new(5.0, 4.5, 3.0)),
            global: GlobalTransform::IDENTITY,
            light: PointLight::new([5.0, 4.5, 3.0], [1.0, 0.76, 0.48], 160.0, 14.0),
        })
        .insert(CastsPunctualShadow);
    let spot_pos = Vec3::new(-5.5, 6.0, 1.5);
    let spot_target = Vec3::new(0.0, 0.6, -5.0);
    let spot_rot = Quat::from_mat3(
        Affine3A::look_at_rh(spot_pos, spot_target, Vec3::new(0.0, 1.0, 0.0)).matrix3,
    );
    let spot_dir = (spot_target - spot_pos).normalize();
    commands
        .spawn(SpotLightObject {
            transform: Transform { translation: spot_pos, rotation: spot_rot, scale: Vec3::ONE },
            global: GlobalTransform::IDENTITY,
            light: SpotLight::new(
                [spot_pos.x, spot_pos.y, spot_pos.z],
                [spot_dir.x, spot_dir.y, spot_dir.z],
                [0.6, 0.75, 1.0],
                120.0,
                16.0,
                18.0,
                32.0,
            ),
        })
        .insert(CastsPunctualShadow);

    // ── The key-hint panel (see the module doc for why it is a quad and not a widget).
    if !matches!(std::env::var("BOYKO_HUD").as_deref(), Ok("off")) {
        spawn_hud_panel(&mut commands, &mut meshes, &mut materials, &mut textures, &mut bindless, dev.get());
    }

    // ── The two downloaded models in `assets/models/` (see the module doc).
    if !matches!(std::env::var("BOYKO_MODELS").as_deref(), Ok("off")) {
        spawn_showcase_models(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut textures,
            &mut bindless,
            dev.get(),
            clay_material,
        );
    }

    // ── The owner's own model, if `BOYKO_MESH` names one (see the module doc).
    spawn_env_mesh(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut textures,
        &mut bindless,
        dev.get(),
        crate_material,
    );

    // ── The fly camera, framing the pyramid down -Z (yaw 0 looks down -Z). The pose is
    //    near-level ON PURPOSE — the gun fires along this forward, so the default view
    //    is also the default aim (see [`BALL_SPEED`]).
    //
    //    `BOYKO_LOCK_LOOK=1` zeroes the look sensitivity. An unattended dump run has no
    //    one holding the mouse still, and the OS delivers whatever raw motion happens
    //    to arrive while the window is up: MEASURED, three consecutive
    //    `BOYKO_HOST_DUMP` captures of this scene framed three different directions,
    //    two of them straight into the floor. The knob makes an automated capture
    //    reproducible; it changes nothing about an interactive run.
    let sensitivity = if std::env::var_os("BOYKO_LOCK_LOOK").is_some() {
        0.0
    } else {
        FlyCamera::DEFAULT_SENSITIVITY
    };
    commands.spawn(FlyCameraBundle {
        transform: Transform::from_translation(Vec3::new(0.0, 1.9, 6.5)),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 58.0 * PI / 180.0,
            aspect: WIN_W as f32 / WIN_H as f32,
            near: 0.1,
            far: 160.0,
        },
        fly: FlyCamera { yaw: 0.0, pitch: -0.04, speed: 7.0, sensitivity },
    });

    println!(
        "playground: {PYRAMID_CUBES} pyramid cubes + {CRATES} crates, up to {MAX_BALLS} balls. \
         LMB = shoot, R = reset, Esc = quit."
    );
}

// ════════════════════════════════════════════════════════════════════════════════
// Gameplay systems
// ════════════════════════════════════════════════════════════════════════════════

/// Fires a ball from the camera on LMB, rate-limited to one every [`SHOT_INTERVAL`].
///
/// Reads the LEVEL bit (not the edge) and gates on the cooldown, so a tap fires once
/// immediately and a hold fires a stream — one branch covering both.
#[allow(clippy::needless_pass_by_value)]
fn shoot_system(
    mut commands: Commands,
    input: Res<PhysicalInput>,
    time: Res<Time>,
    mut pg: ResMut<Playground>,
    cams: Query<(&FlyCamera, &Transform)>,
) {
    pg.cooldown = (pg.cooldown - time.delta_secs()).max(0.0);
    if !input.mouse_held(MouseButton::Left) || pg.cooldown > 0.0 {
        return;
    }
    // The camera's own basis, rebuilt from the controller's accumulators rather than
    // decomposed out of the rotation quaternion — this is the SAME expression
    // `fly_step` uses to build the view, so the shot can never disagree with the
    // crosshair by a convention.
    let Some((fly, transform)) = cams.iter().next() else {
        return;
    };
    let (sy, cy) = fly.yaw.sin_cos();
    let (sp, cp) = fly.pitch.sin_cos();
    let forward = Vec3::new(sy * cp, sp, -cy * cp).normalize();

    // Muzzle the ball clear of the near plane so it is never spawned inside the view.
    let origin = transform.translation + forward * (BALL_RADIUS + 0.7);
    let ball = spawn_ball(&mut commands, &pg, origin, forward * BALL_SPEED);

    let slot = pg.next_ball;
    if let Some(old) = pg.balls[slot].take() {
        commands.despawn(old);
    }
    pg.balls[slot] = Some(ball);
    pg.next_ball = (slot + 1) % MAX_BALLS;
    pg.cooldown = SHOT_INTERVAL;
}

/// `R` — despawns every ball and every dynamic prop, then rebuilds the pyramid.
///
/// Uses the stored `Entity` handles (generation included), not a re-query: a despawn
/// keyed on a stale generation is exactly the bug a registry avoids.
#[allow(clippy::needless_pass_by_value)]
fn reset_system(mut commands: Commands, input: Res<PhysicalInput>, mut pg: ResMut<Playground>) {
    let pressed = KeyCode::KeyR
        .dense_index()
        .is_some_and(|i| input.keys_just_pressed.get(i));
    if !pressed {
        return;
    }
    for slot in 0..MAX_BALLS {
        if let Some(e) = pg.balls[slot].take() {
            commands.despawn(e);
        }
    }
    pg.next_ball = 0;
    for slot in 0..MAX_PROPS {
        if let Some(e) = pg.props[slot].take() {
            commands.despawn(e);
        }
    }
    spawn_props(&mut commands, &mut pg);
}

// ════════════════════════════════════════════════════════════════════════════════
// Spawn helpers
// ════════════════════════════════════════════════════════════════════════════════

/// Spawns the pyramid + the loose crates and records their handles in `pg.props`.
///
/// Called from [`setup`] and again on every `R` reset, so the two paths cannot drift.
fn spawn_props(commands: &mut Commands<'_>, pg: &mut Playground) {
    let mut slot = 0usize;
    let half = Vec3::new(CUBE_SIZE * 0.5, CUBE_SIZE * 0.5, CUBE_SIZE * 0.5);
    // Levels bottom-up; layer `level` is `side x side` cubes, centred over the one
    // below it. The 4 % lateral gap keeps the initial state penetration-free — a
    // stack that starts overlapping starts by exploding.
    let pitch = CUBE_SIZE * 1.04;
    for level in 0..PYRAMID_LEVELS {
        let side = PYRAMID_LEVELS - level;
        let offset = (side as f32 - 1.0) * 0.5;
        for i in 0..side {
            for j in 0..side {
                let t = Transform::from_translation(Vec3::new(
                    (i as f32 - offset) * pitch,
                    CUBE_SIZE * 0.5 + level as f32 * CUBE_SIZE,
                    -5.0 + (j as f32 - offset) * pitch,
                ));
                let e = spawn_dynamic_box(commands, pg.cube_mesh, pg.cube_material, half, t, 1.0);
                pg.props[slot] = Some(e);
                slot += 1;
            }
        }
    }
    // Loose crates: two small stacks off to the side, something to knock about that is
    // not the pyramid.
    let crate_half = Vec3::new(0.5, 0.5, 0.5);
    for k in 0..CRATES {
        let stack = k / 3;
        let level = k % 3;
        let t = Transform {
            // Clear of the showcase models (see `SHOWCASE_MODELS`) — the crates used to
            // stand exactly where the left-hand model does, and buried it.
            translation: Vec3::new(
                -6.2 + stack as f32 * 1.7,
                0.5 + level as f32 * 1.02,
                0.4 - stack as f32 * 0.9,
            ),
            rotation: quat_rot_y(0.12 * k as f32),
            scale: Vec3::ONE,
        };
        let e = spawn_dynamic_box(commands, pg.crate_mesh, pg.crate_material, crate_half, t, 3.0);
        pg.props[slot] = Some(e);
        slot += 1;
    }
    debug_assert_eq!(slot, MAX_PROPS, "invariant: every prop slot is filled exactly once");
}

/// An immovable contact surface: drawable, collidable, never integrated.
///
/// `inv_mass == 0` + `inv_inertia == Mat3::ZERO` + the `Simulated` bit left CLEAR is
/// the documented static shape (`components::Simulated`). The pose flows Transform ->
/// RigidBody through `sync_transform_to_body`, so the authored `Transform` stays the
/// single source of truth.
fn spawn_static_box(
    commands: &mut Commands<'_>,
    mesh: MeshHandle,
    material: MaterialHandle,
    half: Vec3,
    transform: Transform,
    caster: bool,
) {
    let mut bundle = MeshBundle::new(mesh, transform);
    bundle.material = material;
    let id = commands
        .spawn(bundle)
        .insert(RigidBodyBundle {
            body: RigidBody {
                position: transform.translation,
                rotation: transform.rotation,
                ..RigidBody::default()
            },
            mass: RigidBodyMass {
                inv_inertia: Mat3::ZERO,
                inv_mass: 0.0,
                restitution: 0.0,
                friction: 0.6,
            },
            collider: Collider {
                shape: ColliderShape::Box { half_extents: half },
                layer: 1,
                mask: !0,
            },
        })
        .id();
    if caster {
        // Structural capability, so it is inserted rather than flagged: the ground
        // slab is a RECEIVER only. A 32 m caster would push the CSM fit out to cover
        // itself and shadow-acne its own top face for no visible gain.
        commands.entity(id).insert(ShadowCaster);
    }
}

/// A simulated box prop: drawable, collidable, integrated, interpolated.
fn spawn_dynamic_box(
    commands: &mut Commands<'_>,
    mesh: MeshHandle,
    material: MaterialHandle,
    half: Vec3,
    transform: Transform,
    mass: f32,
) -> Entity {
    let mut bundle = MeshBundle::new(mesh, transform);
    bundle.material = material;
    commands
        .spawn(bundle)
        .insert(RigidBodyBundle {
            body: RigidBody {
                position: transform.translation,
                rotation: transform.rotation,
                ..RigidBody::default()
            },
            mass: RigidBodyMass {
                inv_inertia: box_inv_inertia(half, mass),
                inv_mass: 1.0 / mass,
                // Low restitution + high friction: a crate that bounces or slides
                // reads as polystyrene, and a tower of those never settles.
                restitution: 0.02,
                friction: 0.8,
            },
            collider: Collider {
                shape: ColliderShape::Box { half_extents: half },
                layer: 1,
                mask: !0,
            },
        })
        .insert(ShadowCaster)
        .insert(PairOnly { pair: GpuTransform3D::from_transform(&transform) })
        .enable::<Simulated>()
        .id()
}

/// A projectile: a sphere body launched at `velocity`.
fn spawn_ball(
    commands: &mut Commands<'_>,
    pg: &Playground,
    origin: Vec3,
    velocity: Vec3,
) -> Entity {
    let transform = Transform::from_translation(origin);
    let mut bundle = MeshBundle::new(pg.ball_mesh, transform);
    bundle.material = pg.ball_material;
    commands
        .spawn(bundle)
        .insert(RigidBodyBundle {
            body: RigidBody {
                position: origin,
                linear_velocity: velocity,
                ..RigidBody::default()
            },
            mass: RigidBodyMass {
                inv_inertia: sphere_inv_inertia(BALL_RADIUS, BALL_MASS),
                inv_mass: 1.0 / BALL_MASS,
                // See the module note on ballistics: a livelier ball turns every miss
                // into a vertical launch, because the impact's normal impulse also
                // saturates the friction cone and kills the forward component.
                restitution: 0.12,
                friction: 0.22,
            },
            collider: Collider {
                shape: ColliderShape::Sphere { radius: BALL_RADIUS },
                layer: 1,
                mask: !0,
            },
        })
        .insert(ShadowCaster)
        .insert(PairOnly { pair: GpuTransform3D::from_transform(&transform) })
        .enable::<Simulated>()
        .id()
}

// ════════════════════════════════════════════════════════════════════════════════
// Inertia
// ════════════════════════════════════════════════════════════════════════════════

/// Inverse inertia tensor of a solid box with `half` extents and `mass`, in the
/// body's LOCAL frame.
///
/// LOCAL is the operative word: the solver refreshes each substep's effective tensor
/// as `R · I⁻¹ · Rᵀ` from this column (`solver::simd::refresh_inertia_scalar`), so
/// what belongs here is the constant body-frame tensor, not a world-space snapshot.
///
/// `I_xx = m/12 · ((2h_y)² + (2h_z)²) = m/3 · (h_y² + h_z²)`, and cyclically.
fn box_inv_inertia(half: Vec3, mass: f32) -> Mat3 {
    let third = mass / 3.0;
    let ix = third * (half.y * half.y + half.z * half.z);
    let iy = third * (half.x * half.x + half.z * half.z);
    let iz = third * (half.x * half.x + half.y * half.y);
    Mat3::from_rows(
        Vec3::new(1.0 / ix, 0.0, 0.0),
        Vec3::new(0.0, 1.0 / iy, 0.0),
        Vec3::new(0.0, 0.0, 1.0 / iz),
    )
}

/// Inverse inertia tensor of a solid sphere (`I = 2/5 · m · r²`, isotropic).
fn sphere_inv_inertia(radius: f32, mass: f32) -> Mat3 {
    let inv = 2.5 / (mass * radius * radius);
    Mat3::from_rows(
        Vec3::new(inv, 0.0, 0.0),
        Vec3::new(0.0, inv, 0.0),
        Vec3::new(0.0, 0.0, inv),
    )
}

// ════════════════════════════════════════════════════════════════════════════════
// Assets
// ════════════════════════════════════════════════════════════════════════════════

/// Loads one `assets/materials/<folder>/pbr/` texture set and reports which slots
/// resolved.
///
/// A missing / unreadable file leaves that slot at `0` and the material falls back to
/// its scalar channel; a folder that is absent entirely yields an all-zero set and an
/// untextured (but perfectly valid) material — [`load_material_folder`] names each
/// skipped slot on stderr and never panics.
fn load_slots(
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
    folder: &str,
) -> MaterialTextures {
    let mut dir = asset_path("assets/materials");
    dir.push(folder);
    dir.push("pbr");
    let slots = load_material_folder(textures, ctx, bindless, &dir);
    println!(
        "playground: {folder} -> albedo={} normal={} metal_rough={} ao={} (0 = fallback)",
        slots.albedo, slots.normal, slots.metal_rough, slots.ao
    );
    slots
}

/// Mints a material over an already-loaded texture set.
///
/// `base_color` MULTIPLIES the albedo texture, which is what lets the crates and the
/// floor share one upload of `industrial-walls` and still read as different objects —
/// a second `load_slots` of the same folder would upload every image again for nothing.
/// `metallic` / `roughness` are the SCALAR fallbacks: they drive the surface only where
/// the folder supplies no map (`light-gold`, for instance, ships no `ao.png`).
fn material_from(
    materials: &mut Assets<Material>,
    slots: MaterialTextures,
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
) -> MaterialHandle {
    let handle = materials.add(Material::with_textures(
        MaterialGpu::new(base_color, metallic, roughness, 0.5, [0.0; 3], 0),
        slots,
    ));
    MaterialHandle::from_handle(handle)
}

// ════════════════════════════════════════════════════════════════════════════════
// The key-hint HUD
// ════════════════════════════════════════════════════════════════════════════════

/// Rasterises [`HUD_LINES`] once and spawns the camera-parented panel quad.
///
/// Everything here is startup-only: the bake, the CPU compositing and the texture upload
/// all happen before the first frame, and the per-frame cost is one `Transform` write in
/// [`hud_follow_camera`].
fn spawn_hud_panel(
    commands: &mut Commands<'_>,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
) {
    let Some(image) = bake_hint_texture() else {
        eprintln!("playground: the hint font could not be baked — running without the HUD");
        return;
    };
    // ONE mip level, deliberately — NOT `register_texture`, which builds the full chain.
    //
    // The bindless sampler is trilinear with anisotropy and an unclamped max-LOD
    // (`boyko_rhi_vulkan/src/bindless.rs:173-191`), so the instant the panel is minified
    // even slightly the GPU blends in mip 1 and the glyph strokes go soft. A HUD is the
    // one surface that is never meant to be minified: a single level makes "sample mip 0"
    // the only thing the sampler can do, and the 1:1 sizing below makes that the right
    // thing.
    let (format, view_format) = image.color_space.formats();
    let texture = upload_texture_2d(ctx, image.width, image.height, &image.rgba8, format, view_format, 1)
        .expect("invariant: HUD panel texture upload (setup stage)");
    let slot = bindless.register(ctx, texture.view());
    // Into the world's texture table so the host tears it down at shutdown like every
    // other texture. Nothing else resolves the handle — the material addresses the image
    // by its BINDLESS SLOT, and texture rows carry no carrier refcount to retire.
    textures.add(TextureGpu {
        texture,
        bindless_slot: slot,
        width: image.width,
        height: image.height,
        mip_levels: 1,
    });
    // The SAME image in the albedo and emissive slots. The deferred resolve computes
    // `emissive = material.emissive * luminance(emissive_texture)`, so the bright glyphs
    // emit and the dark plate does not — the panel reads the same in sunlight and in the
    // shadow of a wall, with no second texture and no special-case shader.
    let handle = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 0.0, 1.0, 0.0, [1.6, 1.6, 1.6], 0),
        MaterialTextures { albedo: slot, normal: 0, metal_rough: 0, ao: 0, emissive: slot },
    ));
    let material = MaterialHandle::from_handle(handle);
    materials.pin(u32::from(material.0));

    // A camera-facing quad in the XY plane: +Z is its normal, which is what the follow
    // system aligns with the camera's own -Z forward. Its size is the image's size times
    // the world-units-per-screen-pixel at [`HUD_DISTANCE`] — so the panel covers exactly
    // as many pixels as it has texels, and every glyph edge the CPU rasteriser drew lands
    // on the pixel it was drawn for.
    let scale = world_per_pixel(HUD_DISTANCE);
    let (hw, hh) = (image.width as f32 * scale * 0.5, image.height as f32 * scale * 0.5);
    let mut verts = vec![
        Vertex::new([-hw, -hh, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
        Vertex::new([hw, -hh, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
        Vertex::new([hw, hh, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
        Vertex::new([-hw, hh, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ];
    // V grows DOWNWARD in the image, so the bottom-left corner samples v = 1.
    verts[0].uv = [0.0, 1.0];
    verts[1].uv = [1.0, 1.0];
    verts[2].uv = [1.0, 0.0];
    verts[3].uv = [0.0, 0.0];
    let indices = vec![0u32, 1, 2, 0, 2, 3];
    generate_tangents(&mut verts, &indices);
    let mesh = meshes.register_mesh(ctx, &verts, &indices);
    meshes.pin(mesh.0);

    let mut bundle = MeshBundle::new(mesh, Transform::IDENTITY);
    bundle.material = material;
    // No `ShadowCaster`: a panel floating in front of the eye must not stamp itself into
    // the sun cascades, and no physics — it is not part of the world.
    commands.spawn(bundle).insert(HudPanel);
}

/// World units one SCREEN pixel spans at `distance` down the view axis.
///
/// From the projection this scene authors: the view frustum is `2 · d · tan(fov_y / 2)`
/// tall at distance `d`, spread over [`WIN_H`] pixels. Pixels are square, so the same
/// number scales the horizontal axis. Exact while the render extent is the boot window
/// size (the host fixes the composite extent at boot); a resized window rescales the
/// presented image, which softens the 1:1 slightly rather than breaking it.
fn world_per_pixel(distance: f32) -> f32 {
    let fov_y = 58.0 * PI / 180.0;
    2.0 * distance * (fov_y * 0.5).tan() / WIN_H as f32
}

/// Rasterises [`HUD_LINES`] into an RGBA8 image: a dark plate with light glyphs.
///
/// The glyphs come from the engine's own MTSDF bake — [`bake_font`] over exactly the
/// characters [`HUD_LINES`] uses — decoded on the CPU with the standard median +
/// screen-px-range rule (the same decode the UI shader does on the GPU). Returns `None`
/// only when the fixture face cannot be read or parsed.
fn bake_hint_texture() -> Option<TextureData> {
    // The face: the packaged copy if there is one, else this repo's own fixture (which is
    // what a package copies). `BOYKO_HUD_FONT` overrides both.
    let packaged = asset_path("assets/fonts/Ubuntu-Light.ttf");
    let font_path = std::env::var("BOYKO_HUD_FONT").unwrap_or_else(|_| {
        if packaged.exists() {
            packaged.to_string_lossy().into_owned()
        } else {
            concat!(env!("CARGO_MANIFEST_DIR"), "/../boyko_fontbake/fixtures/Ubuntu-Light.ttf")
                .to_string()
        }
    });
    let bytes = std::fs::read(&font_path)
        .map_err(|e| eprintln!("playground: HUD font {font_path} could not be read: {e}"))
        .ok()?;
    let face = TtfFace::from_bytes(&bytes)?;

    // Bake exactly the glyph set the hints use — the atlas is proportional to it.
    let mut wanted: Vec<char> = HUD_LINES.concat().chars().filter(|c| *c != ' ').collect();
    wanted.sort_unstable();
    wanted.dedup();
    let font = bake_font(&face, &wanted, None);

    // Layout in texels. `plane` is em-relative to the baseline, so one em maps to
    // `HUD_GLYPH_PX` texels and every metric scales with it.
    let em = HUD_GLYPH_PX;
    let line_h = (font.meta.ascender_em - font.meta.descender_em + font.meta.line_gap_em) * em;
    let pad = em * 0.7;
    let widest = HUD_LINES
        .iter()
        .map(|line| line_width_em(&font, line) * em)
        .fold(0.0_f32, f32::max);
    let width = (widest + pad * 2.0).ceil().max(16.0) as u32;
    let height = (line_h * HUD_LINES.len() as f32 + pad * 2.0).ceil().max(16.0) as u32;

    // The plate: a dark, slightly blue-black backing (opaque — this G-buffer has no
    // alpha blending, so a "transparent" HUD would just be a hole).
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for px in rgba.as_chunks_mut::<4>().0 {
        *px = [10, 12, 16, 255];
    }
    // A one-texel border, so the panel reads as a plate rather than as a floating smear.
    for x in 0..width {
        for y in [0u32, 1, height - 2, height - 1] {
            let i = ((y * width + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[70, 78, 92, 255]);
        }
    }
    for y in 0..height {
        for x in [0u32, 1, width - 2, width - 1] {
            let i = ((y * width + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[70, 78, 92, 255]);
        }
    }

    for (row, line) in HUD_LINES.iter().enumerate() {
        // Baseline of this row, in texels from the image top.
        let baseline = pad + line_h * row as f32 + font.meta.ascender_em * em;
        let mut pen = pad;
        for ch in line.chars() {
            let Some(slot) = glyph_slot(&font, ch) else {
                pen += em * 0.32;
                continue;
            };
            let g = &font.glyphs[slot];
            draw_glyph(&mut rgba, width, height, &font, g, pen, baseline, em, row);
            pen += g.advance_em * em;
        }
    }

    Some(TextureData { width, height, rgba8: rgba, color_space: ColorSpace::Srgb })
}

/// Total advance of `line` in em (the layout pass's width query).
fn line_width_em(font: &BakedFont, line: &str) -> f32 {
    line.chars()
        .map(|ch| glyph_slot(font, ch).map_or(0.32, |slot| font.glyphs[slot].advance_em))
        .sum()
}

/// Dense glyph slot for `ch`, or `None` when the bake carries no such codepoint.
///
/// `cmap` is sorted by codepoint (the bake's own contract), so this is a binary search,
/// not a scan — it runs once per glyph per line at startup either way.
fn glyph_slot(font: &BakedFont, ch: char) -> Option<usize> {
    let cp = ch as u32;
    font.cmap
        .binary_search_by_key(&cp, |m| m.codepoint)
        .ok()
        .map(|i| font.cmap[i].slot as usize)
}

/// Composites one glyph into `rgba` at the given pen position.
///
/// The MTSDF decode is the standard one: `median(r, g, b)` is the signed distance in
/// `[0, 1]` with `0.5` on the contour, and multiplying `(sd - 0.5)` by the number of
/// screen texels one distance-range spans turns it into a coverage ramp exactly one texel
/// wide — antialiasing without supersampling.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    font: &BakedFont,
    g: &boyko_fontbake::GlyphMetrics,
    pen: f32,
    baseline: f32,
    em: f32,
    row: usize,
) {
    let (al, ab, ar, at) = (g.atlas[0], g.atlas[1], g.atlas[2], g.atlas[3]);
    if ar - al <= 0.0 || at - ab <= 0.0 {
        return; // advance-only glyph (space and friends carry no atlas entry)
    }
    // Destination quad, in image texels. `plane` is baseline-relative and Y-up; the image
    // is Y-down, hence the two sign flips.
    let dst_l = pen + g.plane[0] * em;
    let dst_r = pen + g.plane[2] * em;
    let dst_t = baseline - g.plane[3] * em;
    let dst_b = baseline - g.plane[1] * em;
    // One atlas texel spans this many destination texels — the AA ramp's slope.
    let px_range = ((dst_r - dst_l) / (ar - al)) * font.meta.distance_range_texels;
    // The third line is the dimmer caption; the hint lines are bright.
    let ink: [f32; 3] = if row + 1 == HUD_LINES.len() {
        [0.55, 0.62, 0.75]
    } else {
        [0.94, 0.96, 1.0]
    };

    let x0 = dst_l.floor().max(0.0) as u32;
    let x1 = (dst_r.ceil() as i64).clamp(0, width as i64) as u32;
    let y0 = dst_t.floor().max(0.0) as u32;
    let y1 = (dst_b.ceil() as i64).clamp(0, height as i64) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            // Destination texel centre → atlas texel (nearest; the atlas is baked at
            // 48 px/em and the panel draws at 26, so it is a minification — a bilinear
            // tap would only blur the distance field).
            let u = (x as f32 + 0.5 - dst_l) / (dst_r - dst_l);
            let v = (y as f32 + 0.5 - dst_t) / (dst_b - dst_t);
            let ax = (al + u * (ar - al)).clamp(0.0, font.meta.atlas_w as f32 - 1.0);
            // The baked `atlas = [left, bottom, right, top]` is in texels measured from
            // the image TOP (`boyko_ui::text::shape::quad_uv`'s contract), so `atlas[3]`
            // is the SMALLER texel-Y and maps to the quad's top edge. Getting this
            // backwards is not subtly wrong — it samples other glyphs' rows and prints
            // shredded letters. MEASURED, first try.
            let ay = (at + v * (ab - at)).clamp(0.0, font.meta.atlas_h as f32 - 1.0);
            let ai = ((ay as u32) * font.meta.atlas_w + ax as u32) as usize * 4;
            let Some(texel) = font.atlas.pixels.get(ai..ai + 4) else {
                continue;
            };
            let sd = match font.meta.kind {
                // MTSDF keeps the TRUE signed distance in alpha; MSDF has RGB only.
                AtlasKind::Mtsdf => texel[3] as f32 / 255.0,
                AtlasKind::Msdf => median3(
                    texel[0] as f32 / 255.0,
                    texel[1] as f32 / 255.0,
                    texel[2] as f32 / 255.0,
                ),
            };
            let coverage = ((sd - 0.5) * px_range + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let di = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                let dst = rgba[di + c] as f32 / 255.0;
                let lerped = dst + (ink[c] - dst) * coverage;
                rgba[di + c] = (lerped * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Median of three — the MSDF decode's corner-preserving reconstruction.
fn median3(a: f32, b: f32, c: f32) -> f32 {
    a.max(b).min(a.max(c)).max(b.min(c))
}

/// Computes where the panel should be, from the camera pose the fly controller just wrote.
///
/// # Why this is TWO systems and not one
///
/// The pose has to be carried through a resource because the scheduler's intra-system
/// access check is FILTER-BLIND: reading `&Transform` and writing `Mut<Transform>` in one
/// system is rejected as `ComponentWriteVsRead` even when `With`/`Without` make the two
/// queries provably disjoint (`filtered_access_set.rs:255-262` tests the component id, not
/// the filter). Splitting the read from the write is the engine's shape for this, and it
/// costs one 32-byte resource.
#[allow(clippy::needless_pass_by_value)]
fn hud_read_camera(cams: Query<&Transform, With<FlyCamera>>, mut target: ResMut<HudTarget>) {
    let Some(cam) = cams.iter().next() else {
        return;
    };
    let rot = cam.rotation;
    let forward = rot.rotate(Vec3::new(0.0, 0.0, -1.0));
    let up = rot.rotate(Vec3::new(0.0, 1.0, 0.0));
    target.translation = cam.translation + forward * HUD_DISTANCE + up * HUD_DROP;
    target.rotation = rot;
}

/// Writes the panel's pose from [`HudTarget`] — the write half of [`hud_read_camera`].
///
/// Value-gated, so a still camera does not bump the panel's changed tick every frame.
#[allow(clippy::needless_pass_by_value)]
fn hud_place_panel(target: Res<HudTarget>, mut panels: Query<Mut<Transform>, With<HudPanel>>) {
    for mut panel in panels.iter_mut() {
        if panel.translation != target.translation || panel.rotation != target.rotation {
            let panel = &mut *panel;
            panel.translation = target.translation;
            panel.rotation = target.rotation;
        }
    }
}

/// Loads the model named by `BOYKO_MESH` (if any) and spawns it as one prop.
///
/// Decoders are chosen by extension and called SYNCHRONOUSLY: `AssetLoader::decode` is a
/// pure `&[u8] -> MeshData`, so a scene that knows its file at startup needs neither the
/// `AssetServer` nor its async drain. Every failure path is a printed line and an early
/// return — a bad path or an unsupported `.glb` feature must not take the scene down.
#[allow(clippy::too_many_arguments)]
fn spawn_env_mesh(
    commands: &mut Commands<'_>,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
    fallback_material: MaterialHandle,
) {
    let Some(raw_path) = std::env::var_os("BOYKO_MESH") else {
        return;
    };
    let path = std::path::PathBuf::from(raw_path);
    let Some(mut data) = load_mesh_file(&path) else {
        return;
    };

    // The model's own bounds, in ITS units.
    let (lo, hi) = mesh_bounds(&data);
    // RE-CENTRE on the AABB. `Collider`'s box is centred on the BODY and carries no
    // offset field, so a model whose origin is (say) its feet would otherwise need a
    // collider grown to twice its height just to cover it. Moving the geometry instead
    // costs one pass at load and makes the box exact.
    recentre(&mut data, [
        (hi[0] + lo[0]) * 0.5,
        (hi[1] + lo[1]) * 0.5,
        (hi[2] + lo[2]) * 0.5,
    ]);

    let scale = env_f32("BOYKO_MESH_SCALE", 1.0).max(1e-4);
    let half = Vec3::new(
        (hi[0] - lo[0]) * 0.5 * scale,
        (hi[1] - lo[1]) * 0.5 * scale,
        (hi[2] - lo[2]) * 0.5 * scale,
    );

    println!(
        "playground: BOYKO_MESH {} -> {} verts, {} tris, world size {:.2} x {:.2} x {:.2}",
        path.display(),
        data.vertices.len(),
        data.indices.len() / 3,
        half.x * 2.0,
        half.y * 2.0,
        half.z * 2.0
    );

    let mesh = meshes.register_mesh(ctx, &data.vertices, &data.indices);
    // Pinned like every other scene asset — see the pin block in `setup`.
    meshes.pin(mesh.0);
    let material = match std::env::var("BOYKO_MESH_MAT") {
        Ok(folder) => {
            let slots = load_slots(textures, bindless, ctx, &folder);
            let handle = material_from(materials, slots, [1.0, 1.0, 1.0, 1.0], 0.0, 0.7);
            materials.pin(u32::from(handle.0));
            handle
        }
        Err(_) => fallback_material,
    };

    let ground = env_vec3("BOYKO_MESH_POS", Vec3::new(-3.0, 0.0, -1.0));
    let transform = Transform {
        // Re-centred geometry ⇒ the body centre is half a height above the ground.
        translation: Vec3::new(ground.x, ground.y + half.y, ground.z),
        rotation: quat_rot_y(env_f32("BOYKO_MESH_YAW", 0.0).to_radians()),
        scale: Vec3::new(scale, scale, scale),
    };

    if std::env::var_os("BOYKO_MESH_DYNAMIC").is_some() {
        // Shootable. Mass from a light-crate density over the box volume, clamped: a
        // 6 m statue should not be flicked across the arena, and a 20 cm trinket should
        // not weigh a tonne.
        let volume = (half.x * half.y * half.z * 8.0).max(0.001);
        let mass = (volume * 150.0).clamp(1.0, 500.0);
        spawn_dynamic_box(commands, mesh, material, half, transform, mass);
    } else {
        spawn_static_box(commands, mesh, material, half, transform, true);
    }
}

/// Reads and decodes one mesh file, or prints why it could not and returns `None`.
///
/// `.glb` takes the strict decode first and falls back to the BIND-POSE decode when the
/// file turns out to be rigged — announced on stderr, because "this model is posed, not
/// animated" is exactly the kind of thing that must not be silent (it is why the engine's
/// loader keeps the two entry points apart in the first place).
fn load_mesh_file(path: &std::path::Path) -> Option<MeshData> {
    let bytes = std::fs::read(path)
        .map_err(|e| eprintln!("playground: {} could not be read: {e}", path.display()))
        .ok()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let decoded = match ext.as_str() {
        "glb" => match GlbMeshLoader::decode(&bytes) {
            Ok(d) => Ok(d),
            Err(strict) => match GlbMeshLoader::decode_static_pose(&bytes) {
                Ok(d) => {
                    println!(
                        "playground: {} is rigged — taking its BIND POSE ({strict:?})",
                        path.display()
                    );
                    Ok(d)
                }
                Err(e) => Err(e),
            },
        },
        "obj" => ObjMeshLoader::decode(&bytes),
        other => {
            eprintln!(
                "playground: extension `{other}` is not one this engine decodes \
                 (.glb = binary glTF 2.0, .obj = Wavefront)"
            );
            return None;
        }
    };
    let data = decoded
        .map_err(|e| eprintln!("playground: {} failed to decode: {e:?}", path.display()))
        .ok()?;
    if data.vertices.is_empty() || data.indices.is_empty() {
        eprintln!("playground: {} decoded to an empty mesh", path.display());
        return None;
    }
    Some(data)
}

/// Spawns every model in [`SHOWCASE_MODELS`] that is present on disk.
///
/// A missing folder is a printed line, not a panic: the scene must still run in a checkout
/// where the (licence-restricted) models were not fetched.
#[allow(clippy::too_many_arguments)]
fn spawn_showcase_models(
    commands: &mut Commands<'_>,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
    fallback_material: MaterialHandle,
) {
    for (folder, target_height, x, z, yaw_deg) in SHOWCASE_MODELS {
        let mut path = asset_path("assets/models");
        path.push(folder);
        path.push("model.glb");
        if !path.exists() {
            println!("playground: no model at {} — skipping", path.display());
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("playground: {} could not be read: {e}", path.display());
                continue;
            }
        };
        // Strict first; the rest-shape decode is the announced fallback (see `load_mesh_file`).
        let scene = match GlbMeshLoader::decode_scene(&bytes) {
            Ok(sc) => sc,
            Err(strict) => match GlbMeshLoader::decode_scene_static_pose(&bytes) {
                Ok(sc) => {
                    println!("playground: {folder} is deformable — taking its REST SHAPE ({strict:?})");
                    sc
                }
                Err(e) => {
                    eprintln!("playground: {folder} failed to decode: {e:?}");
                    continue;
                }
            },
        };
        let mut scene = scene;
        if scene.parts.is_empty() {
            eprintln!("playground: {folder} decoded to no parts");
            continue;
        }

        // ── One bounding box over ALL parts: the model is placed and collided as a whole,
        //    so every part must take the SAME recentre and the SAME scale.
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for part in &scene.parts {
            let (plo, phi) = mesh_bounds(&part.mesh);
            for axis in 0..3 {
                lo[axis] = lo[axis].min(plo[axis]);
                hi[axis] = hi[axis].max(phi[axis]);
            }
        }
        let raw_height = (hi[1] - lo[1]).max(1e-6);
        let scale = target_height / raw_height;
        let centre = [
            (hi[0] + lo[0]) * 0.5,
            (hi[1] + lo[1]) * 0.5,
            (hi[2] + lo[2]) * 0.5,
        ];
        let half = Vec3::new(
            (hi[0] - lo[0]) * 0.5 * scale,
            (hi[1] - lo[1]) * 0.5 * scale,
            (hi[2] - lo[2]) * 0.5 * scale,
        );
        let transform = Transform {
            translation: Vec3::new(x, half.y, z),
            rotation: quat_rot_y(yaw_deg.to_radians()),
            scale: Vec3::new(scale, scale, scale),
        };

        // ── The images, decoded ONCE each and shared by every material that names them.
        //    `None` marks an image this engine cannot decode (JPEG today) — the material
        //    then keeps its scalar factor for that channel.
        let mut slots: Vec<Option<u32>> = Vec::with_capacity(scene.images.len());
        let mut skipped_mimes: Vec<String> = Vec::new();
        for (index, image) in scene.images.iter().enumerate() {
            slots.push(upload_glb_image(
                textures,
                bindless,
                ctx,
                image,
                glb_image_color_space(&scene, index),
                &mut skipped_mimes,
            ));
        }
        let textured = slots.iter().filter(|s| s.is_some()).count();

        let tris: usize = scene.parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
        let verts: usize = scene.parts.iter().map(|p| p.mesh.vertices.len()).sum();
        println!(
            "playground: {folder} -> {} parts, {verts} verts, {tris} tris, {textured}/{} images decoded, \
             height {raw_height:.2} -> {target_height:.2} m",
            scene.parts.len(),
            scene.images.len()
        );
        if !skipped_mimes.is_empty() {
            skipped_mimes.sort_unstable();
            skipped_mimes.dedup();
            println!(
                "playground: {folder} — {} image(s) left undecoded ({}); those channels use the \
                 material factor",
                scene.images.len() - textured,
                skipped_mimes.join(", ")
            );
        }

        // ── One drawable entity per PART, each with its own material.
        for part in &mut scene.parts {
            if part.mesh.indices.is_empty() {
                continue;
            }
            recentre(&mut part.mesh, centre);
            let mesh = meshes.register_mesh(ctx, &part.mesh.vertices, &part.mesh.indices);
            meshes.pin(mesh.0);
            let material = match part.material.and_then(|mi| scene.materials.get(mi)) {
                Some(m) => {
                    let handle = materials.add(Material::with_textures(
                        MaterialGpu::new(
                            m.base_color_factor,
                            m.metallic,
                            m.roughness,
                            0.5,
                            m.emissive_factor,
                            0,
                        ),
                        MaterialTextures {
                            albedo: m.base_color_image.and_then(|i| slots.get(i).copied().flatten()).unwrap_or(0),
                            normal: m.normal_image.and_then(|i| slots.get(i).copied().flatten()).unwrap_or(0),
                            metal_rough: m
                                .metallic_roughness_image
                                .and_then(|i| slots.get(i).copied().flatten())
                                .unwrap_or(0),
                            ao: m.occlusion_image.and_then(|i| slots.get(i).copied().flatten()).unwrap_or(0),
                            emissive: m.emissive_image.and_then(|i| slots.get(i).copied().flatten()).unwrap_or(0),
                        },
                    ));
                    let handle = MaterialHandle::from_handle(handle);
                    materials.pin(u32::from(handle.0));
                    handle
                }
                None => fallback_material,
            };
            let mut bundle = MeshBundle::new(mesh, transform);
            bundle.material = material;
            commands.spawn(bundle).insert(ShadowCaster);
        }

        // ── ONE physics body for the whole model: an immovable box around it. Separate from
        //    the drawables because a glTF's parts share one collider, not one each — and a
        //    body needs no mesh of its own.
        commands.spawn(StaticBodyBundle {
            transform,
            global: GlobalTransform::IDENTITY,
            body: RigidBody {
                position: transform.translation,
                rotation: transform.rotation,
                ..RigidBody::default()
            },
            mass: RigidBodyMass {
                inv_inertia: Mat3::ZERO,
                inv_mass: 0.0,
                restitution: 0.0,
                friction: 0.6,
            },
            collider: Collider {
                shape: ColliderShape::Box { half_extents: half },
                layer: 1,
                mask: !0,
            },
        });
    }
}

/// The colour space a glTF image must be sampled in, decided by which SLOT names it.
///
/// glTF is explicit about this and getting it wrong is not subtle: base-colour and emissive
/// are sRGB-encoded, while normal / metallic-roughness / occlusion are linear data. An
/// image named by both kinds of slot (rare, and wrong in the file) resolves as sRGB.
fn glb_image_color_space(scene: &boyko_render::loaders::glb::GlbScene, image: usize) -> ColorSpace {
    let srgb = scene.materials.iter().any(|m| {
        m.base_color_image == Some(image) || m.emissive_image == Some(image)
    });
    if srgb { ColorSpace::Srgb } else { ColorSpace::Linear }
}

/// Decodes ONE embedded glTF image and uploads it, returning its bindless slot.
///
/// `None` when the container is one this engine has no decoder for — the MIME string is
/// pushed into `skipped` so the caller can say which, once, instead of per image.
fn upload_glb_image(
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
    image: &boyko_render::loaders::glb::GlbImage,
    color_space: ColorSpace,
    skipped: &mut Vec<String>,
) -> Option<u32> {
    if image.bytes.is_empty() {
        return None;
    }
    // PNG is the one container this engine decodes in-house (`boyko_image`). Anything else
    // is named, not guessed at.
    if !image.mime.eq_ignore_ascii_case("image/png") {
        skipped.push(if image.mime.is_empty() { "<no mimeType>".to_string() } else { image.mime.clone() });
        return None;
    }
    let mut data = PngTextureLoader::decode(&image.bytes)
        .map_err(|e| eprintln!("playground: an embedded PNG failed to decode: {e:?}"))
        .ok()?;
    data.color_space = color_space;
    let handle = textures.register_texture(ctx, bindless, &data);
    Some(textures.texture(handle).bindless_slot)
}

/// Model-space AABB of a decoded mesh, as `(min, max)`.
fn mesh_bounds(data: &MeshData) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &data.vertices {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v.position[axis]);
            hi[axis] = hi[axis].max(v.position[axis]);
        }
    }
    (lo, hi)
}

/// Shifts every vertex so the mesh's AABB centre lands on the model origin — see the
/// re-centring note in [`spawn_env_mesh`].
fn recentre(data: &mut MeshData, centre: [f32; 3]) {
    for v in &mut data.vertices {
        for (axis, c) in centre.iter().enumerate() {
            v.position[axis] -= c;
        }
    }
}

/// Resolves a repo-root-relative asset path for BOTH a packaged build and a `cargo run`.
///
/// A shipped folder has `assets/` next to the executable; a developer run has it at the
/// repository root, which is where the compile-time path points. The exe-relative candidate
/// is tried FIRST and only when it exists, so a package can never silently read the
/// developer's tree (and a dev run is unaffected by a stale folder next to `target/`).
fn asset_path(rel: &str) -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(rel);
        if candidate.exists() {
            return candidate;
        }
    }
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(rel)
}

/// One `f32` env knob, falling back to `default` when unset or unparsable.
fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// One `x,y,z` env knob, falling back to `default` when unset or malformed.
fn env_vec3(key: &str, default: Vec3) -> Vec3 {
    let Ok(raw) = std::env::var(key) else {
        return default;
    };
    let mut it = raw.split(',').map(|part| part.trim().parse::<f32>());
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(x)), Some(Ok(y)), Some(Ok(z))) => Vec3::new(x, y, z),
        _ => {
            eprintln!("playground: {key}=`{raw}` is not `x,y,z` — using the default");
            default
        }
    }
}

/// Registers an axis-aligned box mesh of the given half-extents, UV-tiled at
/// `tiles_per_metre` (so a 32 m floor and a 0.5 m cube share one texel density).
fn register_box(
    meshes: &mut Assets<MeshGpu>,
    ctx: &boyko_rhi_vulkan::device::VulkanContext,
    half: Vec3,
    tiles_per_metre: f32,
) -> MeshHandle {
    let (verts, indices) = box_geometry(half, tiles_per_metre);
    meshes.register_mesh(ctx, &verts, &indices)
}

/// One face of [`box_geometry`]'s box: the outward normal, the four corners in fan
/// order, and the `(u, v)` world extents that corner order sweeps (so the UV tiles by
/// SIZE rather than per face).
struct BoxFace {
    /// Outward face normal (model space), carried per-corner so the G-buffer normal
    /// lane is face-correct — no vertex-normal averaging across an edge.
    normal: [f32; 3],
    /// The four corners, in the order the `(0,1,2)+(0,2,3)` fan below consumes.
    corners: [[f32; 3]; 4],
    /// World-space `(u, v)` span of this face, the UV tiling multiplier.
    extent: [f32; 2],
}

/// Six-face box geometry with per-face normals, world-scaled UVs, and a generated
/// tangent basis.
///
/// Corner order and the `(0,1,2)+(0,2,3)` fan are lifted from the engine's own
/// `cube_geometry`, so the winding matches every other mesh the raster pass sees; the
/// only additions are per-axis half-extents and the UV tiling.
fn box_geometry(half: Vec3, tiles_per_metre: f32) -> (Vec<Vertex>, Vec<u32>) {
    const COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const QUAD_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let (hx, hy, hz) = (half.x, half.y, half.z);
    let faces: [BoxFace; 6] = [
        BoxFace {
            normal: [1.0, 0.0, 0.0],
            corners: [[hx, -hy, -hz], [hx, hy, -hz], [hx, hy, hz], [hx, -hy, hz]],
            extent: [2.0 * hy, 2.0 * hz],
        },
        BoxFace {
            normal: [-1.0, 0.0, 0.0],
            corners: [[-hx, -hy, hz], [-hx, hy, hz], [-hx, hy, -hz], [-hx, -hy, -hz]],
            extent: [2.0 * hy, 2.0 * hz],
        },
        BoxFace {
            normal: [0.0, 1.0, 0.0],
            corners: [[-hx, hy, -hz], [-hx, hy, hz], [hx, hy, hz], [hx, hy, -hz]],
            extent: [2.0 * hz, 2.0 * hx],
        },
        BoxFace {
            normal: [0.0, -1.0, 0.0],
            corners: [[-hx, -hy, hz], [-hx, -hy, -hz], [hx, -hy, -hz], [hx, -hy, hz]],
            extent: [2.0 * hz, 2.0 * hx],
        },
        BoxFace {
            normal: [0.0, 0.0, 1.0],
            corners: [[-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz]],
            extent: [2.0 * hx, 2.0 * hy],
        },
        BoxFace {
            normal: [0.0, 0.0, -1.0],
            corners: [[hx, -hy, -hz], [-hx, -hy, -hz], [-hx, hy, -hz], [hx, hy, -hz]],
            extent: [2.0 * hx, 2.0 * hy],
        },
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (f, face) in faces.iter().enumerate() {
        for (c, corner) in face.corners.iter().enumerate() {
            let mut v = Vertex::new(*corner, face.normal, COLOR);
            v.uv = [
                QUAD_UV[c][0] * face.extent[0] * tiles_per_metre,
                QUAD_UV[c][1] * face.extent[1] * tiles_per_metre,
            ];
            vertices.push(v);
        }
        let base = (f * 4) as u32;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    generate_tangents(&mut vertices, &indices);
    (vertices, indices)
}

/// A UV sphere with outward normals, spherical UVs and a generated tangent basis.
///
/// The pole-fan triangles are skipped: at a pole the row collapses to one 3D point, so
/// one triangle per quad has zero area and poisons the pole-ring tangent basis. Same
/// construction (and same reason) as the shared test fixture `tests/common::uv_sphere`,
/// which an example cannot reach.
fn uv_sphere(radius: f32, stacks: u32, slices: u32) -> (Vec<Vertex>, Vec<u32>) {
    const COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * PI;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * PI);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, COLOR);
            vertex.uv = [j as f32 / slices as f32, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            if i != 0 {
                idx.extend_from_slice(&[a, b, a + 1]);
            }
            if i != stacks - 1 {
                idx.extend_from_slice(&[a + 1, b, b + 1]);
            }
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// Rotation of `angle` radians about `+Y` as a unit quaternion.
///
/// `boyko_math::Quat` has no axis-angle constructor and the engine's own scenes build
/// orientations through `look_at_rh`; for a pure yaw the half-angle form is the direct
/// (and exactly normalized) route.
fn quat_rot_y(angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    Quat::new(0.0, s, 0.0, c)
}

/// Rotation of `angle` radians about `+Z` (see [`quat_rot_y`]).
fn quat_rot_z(angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    Quat::new(0.0, 0.0, s, c)
}
