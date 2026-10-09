# Windowed Host (`boyko_app`)

`boyko_app` is the host layer: the OS window, the device boot chain, the windowed
frame loop, and the plugin that composes them. It answers *when* work runs and
*where* it goes on screen — [`boyko_render`](../rendering/overview.md) already
answers *what* to upload.

The point of this page is one claim: you assemble a correct, lit, shadowed 3D
scene in about thirty lines of ordinary ECS code, and the entire frame
discipline — GPU fences, per-frame interpolation, cascaded sun shadows — is baked
in by construction. You author *data*; the plugins do the rest.

This is a rendering-focused sibling to [Your First App](../getting-started/first-app.md).
That page builds a headless `App` with `run_n`; this one opens a window with
`EnginePlugins` and `run`.

## The pitch: one plugin, a handful of spawns

Two calls stand a window up and clear it every frame:

```rust,ignore
use boyko_app::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko clear", 800, 600));
    app.run();
}
```

That is the whole of [`examples/clear.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/examples/clear.rs).
No Vulkan SDK is required — the runner does not request the validation layer by
default, so an absent layer (the common case on end-user machines) cannot fail the
boot. Validation is opt-in: set `BOYKO_ENABLE_VALIDATION` and the runner requests
`VK_LAYER_KHRONOS_validation`, which must then be installed (the boot fails without
it rather than silently running unvalidated).

From there, everything is spawns. Add a floor, four cubes, a sun, and a camera in
a startup system and you have a rendered room. That is the hero example below.

## The hero example: a lit, shadowed room

[`examples/room.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/examples/room.rs)
is the "~30-line scene" milestone: a floor plane, four shadow-casting cubes, a
perspective camera, and ECS-owned lighting — an angled sun with cascaded shadow
maps, a sky ambient fill, and a warm point accent. Everything is assembled
through ECS spawns and `EnginePlugins`; there is no imperative render setup.

```rust,ignore
use boyko_app::prelude::*;

/// The sun direction TO the light — also the `-Z` the sun's transform faces, so
/// the transform-driven reconcile derives the same direction it was authored with.
const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko room", 800, 600));
    // Enable CSM. CsmPlugin's default is DISABLED (the 0%-gate); overwrite the
    // config AFTER add_plugins to turn sun shadows on. Three cascades here.
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    app.run();
}

/// Startup runs WITH the device present, so meshes register straight through the
/// world-resident GpuDevice + Assets<MeshGpu> (`plane` / `cube` come from MeshAssetsExt).
fn setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let floor = meshes.plane(dev.get(), 12.0);
    let cube = meshes.cube(dev.get(), 1.0);

    // The floor RECEIVES shadows only (no ShadowCaster marker).
    commands.spawn(MeshBundle::new(floor, Transform::IDENTITY));
    // The cubes CAST: the structural ShadowCaster marker routes them into the
    // cascade depth pass.
    for (x, z) in [(-2.0, -1.0), (0.0, -2.5), (1.8, -0.6), (0.9, 1.2)] {
        commands
            .spawn(MeshBundle::new(cube, Transform::from_translation(Vec3::new(x, 0.5, z))))
            .insert(ShadowCaster);
    }

    // The sun: an angled directional light. Orient the transform so local -Z
    // points TO the light; the reconcile derives the direction from the
    // propagated GlobalTransform.
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

    // A sky ambient fill (a cool sky over a warm ground).
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    // A warm point accent between the cubes (unshadowed in v1).
    commands.spawn(PointLightObject {
        transform: Transform::from_translation(Vec3::new(0.6, 1.6, -0.8)),
        global: GlobalTransform::IDENTITY,
        light: PointLight::new([0.6, 1.6, -0.8], [1.0, 0.72, 0.45], 220.0, 7.0),
    });

    // The camera at (0, 1.7, 6) looking at the origin.
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 800.0 / 600.0,
            near: 0.1,
            far: 100.0,
        },
    });
}
```

Run it:

```powershell
cargo run -p boyko-app --example room
```

### Walking through it, group by group

- **The plugin + the CSM knob.** `EnginePlugins::window(..)` composes the frame
  stack. `CsmConfig` is inserted *after* `add_plugins` — the sun-shadow cascades
  ship disabled by default, and this one line arms them.
- **Meshes.** `Assets<MeshGpu>` is the world's mesh asset table (a non-`Send`
  resource). The `MeshAssetsExt` methods `plane` and `cube` build vertex/index
  buffers on the resident `GpuDevice` and return `MeshHandle`s. Startup runs with
  the device present, so registration goes straight through.
- **Drawables.** `MeshBundle::new(handle, transform)` makes an entity *drawn*.
  Adding `ShadowCaster` makes it *cast*. The floor omits the marker, so it only
  receives — it can never cast a spurious whole-plane shadow.
- **Lights.** `DirectionalLightObject`, `SkyLight`, `PointLightObject` are
  ordinary spawned entities. Delete the sun spawn and the depth pass simply never
  records — there is no flag to flip.
- **The camera.** `CameraRig` carries a `Camera` and a `Projection`; spawning it
  makes that entity the view. The windowed host v1 is perspective only.

Everything above is a `commands.spawn`. No entity is "registered with the
renderer" by an imperative call.

## `EnginePlugins::window(title, w, h)`

`EnginePlugins` is the host composition plugin. Its
[`window`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/plugins.rs)
constructor takes a caption and a requested client size, and its `build`
([`plugins.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/plugins.rs))
composes the default frame stack. It adds these 18 plugins, in this order:

| Plugin | What it brings |
|--------|----------------|
| `ProfilerPlugin` | the profiler store, armed only by `BOYKO_PROFILE_ON` |
| `LogPlugin` | the logger's in-frame drain; `BOYKO_LOG` turns logging on |
| `CameraPlugin` | transform propagation, active-camera resolution, the visibility bridge |
| `Render3dPlugin` | the 3D instance pack |
| `AssetRefcountPlugin` | asset reference counting and validation |
| `LightingPlugin` | light reconcile, the GPU light table, the eviction hooks |
| `SsaoPlugin` | the SSAO config (off by default) |
| `CsmPlugin` | cascaded sun shadows (off by default) |
| `ShadowAtlasPlugin` | the spot/point shadow atlas (off by default) |
| `DdgiPlugin` | SDF DDGI global illumination (off by default) |
| `RayPlugin` | the ray-tracing backend selection (hardware rays only in `hwrt` builds) |
| `ShadowDenoisePlugin` | the ray-traced shadow denoiser config (off by default) |
| `AaPlugin` | anti-aliasing and the TAA config (off by default) |
| `RenderPathPlugin` | the render-path config (Deferred + both legs by default) |
| `HzbPlugin` | the depth-pyramid config (off by default) |
| `OcclusionPlugin` | two-phase occlusion culling (off by default) |
| `SdfPlugin` | the boot-static SDF edit path; a scene with no `SdfPrimitive` costs nothing |
| `ParticlePlugin` | GPU particles (off by default) |

It also inserts the light-table staging, the default `LightingConfig` /
`ClusterConfig` and the render scratch resources, orders `FixedSet::Snapshot` after `FixedSet::Gameplay` (the seam
that snapshots poses for interpolation), and finally installs the windowed frame
loop with `App::set_runner`.

Some capabilities are **opt-in** and NOT in the default set:

- **`FlyCameraPlugin`** — interactive input and a fly camera. Add it alongside
  `EnginePlugins`. See the viewer scene below.
- **`PhysicsPlugin`** — the in-house rigid-body solver, in the fixed schedule. See
  [Physics](../simulation/physics.md) and the `playground` example.
- **The UI plugins** — ECS-native widgets. The windowed host does not composite UI
  yet; see the status note in [UI Overview](../ui/overview.md).

Do not add any of the 18 plugins above yourself — `EnginePlugins` already composes
them, and adding a plugin twice panics (`boyko-B1801`).

## Capability is component presence

There is a single idea running through the whole example: **a capability is the
presence of a component, not a call you make.** You author data; systems query
for it structurally.

| To make an entity… | …author this |
|--------------------|--------------|
| drawn | a `MeshBundle` (mesh handle + transform) |
| cast a shadow | the `ShadowCaster` marker |
| a live SDF surface | an `SdfPrimitive(SdfEdit::…)` |
| the active view | a camera bundle (`CameraRig` / `FlyCameraBundle`) |
| interpolated | a `GpuTransform3D` pair (see *Frame discipline*) |

Omitting a component is the off switch — the floor with no `ShadowCaster` never
touches the depth pass. This is the same structural model the ECS uses
everywhere; toggling a capability *every frame* instead wants an
[enable tag](../concepts/enable-tags.md), and the storage cost of each choice is
covered in [Storage Trade-offs](../architecture/storage-tradeoffs.md).

## Frame discipline, baked in by construction

This is the load-bearing value. A correct real-time frame has to obey rules that
are easy to get wrong by hand: never write GPU memory the GPU is still reading;
show a smooth image between discrete simulation steps; keep the sun's shadow
cascades fitted to the camera as it moves. The windowed host enforces all three
for you. You never mint a frame token, touch a swapchain, or record a barrier.

- **GPU-fence-before-write.** The runner waits on the frame-in-flight fence
  *before* any host write to a mapped buffer, so a write can never race a read
  the GPU has not finished. The mechanism is a `FrameWriteToken`: the runner mints
  it once the fence is clear, and the token *type* is what the render crate's
  upload functions demand as proof — you cannot write GPU data without holding it.
  You never see this token in scene code; it exists so the correct order is the
  only order that compiles.
- **Fixed-sim, interpolated-render.** Put rate-sensitive gameplay in
  `FixedSet::Gameplay` and it runs at a fixed rate (64 Hz by default). A body that
  carries a `GpuTransform3D` pair opts into interpolation: the host snapshots its
  pose each substep and lerps between the previous and current pose at the render
  rate, so the image glides even when the simulation ticks in discrete jumps.
- **Cascaded sun shadows.** With CSM enabled, the host re-fits the shadow cascades
  to the camera frustum every frame and drives the depth pass over the live
  `ShadowCaster` set. You author the sun and the casters; the fit is automatic.

For the deeper render story — the render paths, the hybrid mesh↔SDF depth
bound, and how a frame actually flows through the GPU — see
[Rendering Overview](../rendering/overview.md). This page deliberately stops at
the user-facing seam.

## Configuring the renderer

Each render feature is chosen by an owner-set resource. `EnginePlugins` inserts
every one with its default, so you overwrite it **after** `add_plugins`; an insert
before it is replaced by the default.

```rust,ignore
use boyko_app::prelude::*;
use boyko_render::{AaConfig, AaMode, GeometryLegs, RenderPath, RenderPathConfig};

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("my game", 1280, 720));
    // All of these come AFTER add_plugins.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });
    app.insert_resource(AaConfig { mode: AaMode::Smaa });
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.run();
}
```

- **The render path is resolved once, at boot.** The runner reads
  `RenderPathConfig` (`RenderPath::{Deferred, Forward, ForwardPlus,
  VisibilityBuffer}` × `GeometryLegs::{Both, Mesh, Sdf}`) when it boots, and the
  choice is fixed for the run; there is no live toggle. A request the device cannot
  serve degrades with a logged reason. See [Render paths](../rendering/render-paths.md).
- **A launch-time override.** `BOYKO_RENDER_PATH` (`deferred`, `forward`,
  `forwardplus`, `vb`) and `BOYKO_GEOMETRY_LEGS` (`both`, `mesh`, `sdf`) override
  the default path while the plugins build, so any example can run in any path
  without an edit. An explicit `RenderPathConfig` inserted after `add_plugins`
  still wins. An unrecognized value is reported and falls back to the default.
- **SSAA is a boot-time choice too.** `EnginePlugins::window(..).with_ssaa_scale(2)`
  requests 2× supersampling; only `2` is honored, and a device that cannot fit it
  boots without it. With the scale left at its default, `BOYKO_AA=ssaa` requests it
  from the environment.
- **The other features are plain config overwrites:** `AaConfig` (FXAA, SMAA, TAA),
  `SsaoConfig`, `CsmConfig`, `ShadowConfig` (spot/point shadows), `DdgiConfig`,
  `ShadowDenoiseConfig` (`hwrt` builds), `OcclusionConfig`, `ParticleConfig`, and
  `LightingConfig` (exposure, tonemapper, clustered culling). The defaults and what
  each one turns on are tabled in
  [Rendering Overview](../rendering/overview.md#what-ships-today).
- **Present mode.** The windowed host presents with FIFO (v-synced) and exposes no
  present-mode knob yet. The backend swapchain itself also supports Immediate and
  Mailbox.
- **Validation.** `BOYKO_ENABLE_VALIDATION` arms the Vulkan validation layer (see
  above).

The renderer has no frame-time benchmark yet; see [Benchmarks](../reference/benchmarks.md)
for what is measured.

## The examples

The `boyko_app` examples run with `cargo run -p boyko-app --example <name>`. The first
five form a learning ladder, each adding one capability to the one before it.

| Example | What it shows |
|---------|---------------|
| `clear` | a window cleared to a neutral color every frame; the smallest possible host, proving boot and teardown |
| `room` | the hero scene above: floor, cubes, camera, sun and CSM shadows, all from spawns |
| `bounce` | a cube bouncing under fixed-timestep gameplay, drawn interpolated at the render rate (`GpuTransform3D`) |
| `viewer` | the room made fly-able with `FlyCameraPlugin` (WASD + mouse look) |
| `sdf_room` | the room plus one live SDF sphere composited into the same G-buffer: the hybrid mesh↔SDF path |
| `punctual_room` | the room with the spot/point shadow atlas enabled (`CastsPunctualShadow`) |
| `playground` | fly, shoot and knock a stack over: the in-house physics through `PhysicsPlugin` (run it with `--release`) |
| `showcase` | a fly-around scene using meshes, an SDF sphere, CSM and both punctual shadow kinds at once |
| `vb_lab` | a fly-around test bed for the Visibility Buffer path: varied meshes and PBR materials plus an SDF leg |
| `paradigm_lab` | one scene that renders in every `RenderPath × GeometryLegs` cell; pick the cell at launch |
| `shadow_denoise_eval` | an in-motion evaluation scene for the ray-traced shadow denoiser (`--features hwrt`) |

`_hud_probe` is an internal copy of `playground`, not a learning example. `scripts/run-scene.ps1` launches any example in a chosen render path, for
example `scripts\run-scene.ps1 -Scene paradigm_lab -Path vb -Legs both`.

### Interpolation: `bounce`

[`examples/bounce.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/examples/bounce.rs)
puts the integrator in `FixedSet::Gameplay` and attaches the interpolation pair
to the cube. The gameplay system writes `Transform` at 64 Hz; the host snapshots
and lerps for the render frame:

```rust,ignore
use boyko_app::prelude::*;
use boyko_ecs::ecs::core::app::CoreSchedule;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko bounce", 800, 600));
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    // Rate-sensitive gameplay runs in Fixed; the host's snapshot observes the
    // post-integrate pose, so the render lerp is one substep wide.
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        b.add_system(bounce).in_set(FixedSet::Gameplay);
    });
    app.run();
}
# fn setup() {}
# fn bounce() {}
```

The cube opts in by carrying a `GpuTransform3D` (seeded `prev == curr`):

```rust,ignore
# use boyko_app::prelude::*;
# let start = Transform::IDENTITY;
# let cube = MeshHandle::INVALID;
# fn f(mut commands: Commands) {
commands
    .spawn(MeshBundle::new(cube, start))
    .insert(ShadowCaster)
    .insert(GpuTransform3D::from_transform(&start));
# }
```

To jump a body discontinuously with no smear, teleport it —
`commands.entity(e).teleport_to(Transform::from_translation(..))` — which writes
the pose and snaps `prev = curr` for one frame (the `TeleportCommandsExt` sugar).

### Interactive input: `viewer`

[`examples/viewer.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/examples/viewer.rs)
is the room made interactive. Add `FlyCameraPlugin` alongside `EnginePlugins` and
spawn a `FlyCameraBundle` instead of a `CameraRig`:

```rust,ignore
use boyko_app::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko viewer", 800, 600));
    // The interactive input + fly-camera stack (opt-in — NOT in the default set).
    app.add_plugin(FlyCameraPlugin);
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    app.add_startup_system(setup);
    app.run();
}
# fn setup() {}
```

`FlyCameraPlugin`
([`fly.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/fly.rs))
composes the input ingest, registers the fly controller in `CameraSet::Control`
(so the view recomposes the same frame — no input lag), and wires an ECS-native
quit through the rebindable `FlyAction::Quit` (Escape by default). WASD flies,
`Space` / `E` rise, `Left Ctrl` / `Q` descend, and the mouse looks. See
[Input](input.md) for the action model behind it.

### Hybrid SDF: `sdf_room`

[`examples/sdf_room.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/examples/sdf_room.rs)
adds one live SDF sphere to the room with a single spawn — no brick bake, no
shader change:

```rust,ignore
# use boyko_app::prelude::*;
# fn f(mut commands: Commands) {
// A UNION-op sphere at radius 0.7, placed among the cubes. Its PRESENCE routes
// it into the marcher's edit list; the host uploads the list once on frame one.
commands.spawn(SdfPrimitive(SdfEdit::sphere([-0.9, 0.7, 0.4], 0.7, sdf_op::UNION, 0.0)));
# }
```

The sphere is direct-marched into the *same* G-buffer as the raster cubes
(min-combined with the mesh depth, so each correctly occludes the other) and lit
by the same sun with an analytic soft shadow. The SDF path is boot-static in v1:
the gather runs once, the write is one-shot, and a scene with no `SdfPrimitive`
costs nothing. See [SDF Rendering](../rendering/sdf.md) for the marcher and its
deferred ladder.

## Why this is ECS-native, not a framework on top

The windowed host is not a separate engine glued over the ECS. It is a stack of
plugins over the same `boyko_ecs` world you met in
[Your First App](../getting-started/first-app.md). Meshes, lights, cameras, SDF
primitives, and interpolation pairs are all ordinary components in the kernel's
own storage; the render, scene, and lighting systems are ordinary systems on the
kernel's own scheduler. `boyko_app` adds only the *host* concerns the ECS cannot
own by itself: the OS window, the device lifetime, and the frame sequencing that
mints the write token.

This is the engine's first principle — one unified engine, `boyko_ecs` as the one
SDK for both logic and data — reaching all the way to the window. The GPU buffers
are a derived view of your ECS data, never a parallel store you hand-manage.

## See also

- [Your First App](../getting-started/first-app.md) — the headless `App` basics
  this page builds on.
- [App & Plugins](plugins.md) — the `App`/`Plugin` model, the prelude, and
  composition rules.
- [Time & Fixed Timestep](time.md) — the fixed-sim clock behind interpolation.
- [Input](input.md) — the action model `FlyCameraPlugin` uses.
- [Rendering Overview](../rendering/overview.md) — the render paths, the
  hybrid mesh↔SDF path, what ships, and how a frame flows through the GPU.
- [Physics](../simulation/physics.md) — `PhysicsPlugin`, which the `playground`
  example adds.
- [Enable Tags](../concepts/enable-tags.md) and
  [Storage Trade-offs](../architecture/storage-tradeoffs.md) — capability as
  presence, and what each storage choice costs.
- Source: [`plugins.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/plugins.rs),
  [`fly.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/fly.rs),
  [`prelude.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_app/src/prelude.rs),
  and the examples in [`crates/boyko_app/examples/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_app/examples).
