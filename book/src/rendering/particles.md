# GPU Particles

> Particles simulated and drawn entirely on the GPU. Emitters are ECS entities; the particles
> themselves live only in GPU buffers. Off by default.

## What it is

At thousands of spawns per frame, routing each particle through `Commands` would cost far more CPU
time than the GPU needs to emit it. So in `boyko_render` a **particle is not an entity**; an
**emitter** is. Each frame:

1. `particle_tick_emitters` folds every active emitter (up to `MAX_EMITTERS`, 256) into one table
   of emit requests.
2. On the GPU, a kickoff pass, an emit pass and a simulation pass run, all as indirect dispatches,
   so the per-frame cost follows the number of live particles, not the pool size.
3. The survivors are drawn as camera-facing, unlit billboards with one indirect indexed draw per
   blend class, composited into the lit image.

Particles are not queryable, observable or serialisable: there is no CPU copy of them. Emitters,
effects and the per-frame staging use the engine's own storage.

This works on all four render paths.

## Turning it on

`EnginePlugins` adds `ParticlePlugin` with `ParticleMode::Off`. Arm it before `run`:

```rust,ignore
use boyko_app::prelude::*;
use boyko_render::{ParticleCollision, ParticleConfig, ParticleMode, ParticleSortMode};

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("particles", 1280, 720));
    app.insert_resource(ParticleConfig {
        mode: ParticleMode::GpuUnlit,
        capacity: 262_144,                // the pool size; fixed at boot
        collision: ParticleCollision::Off, // or `Sdf`
        sort: ParticleSortMode::None,      // or `Radix` for back-to-front alpha
    });
    app.add_startup_system(setup);
    app.run();
}
# fn setup() {}
```

| Field | Default | Meaning |
|-------|---------|---------|
| `mode` | `Off` | `GpuUnlit` simulates and draws. `Off` allocates nothing and records nothing. |
| `capacity` | `PARTICLE_DEFAULT_CAPACITY` (262 144) | The most live particles. It bounds memory only; spawns past it are clamped and counted. |
| `collision` | `Off` | `Sdf` makes particles collide with the SDF shapes in the scene. |
| `sort` | `None` | `Radix` sorts alpha-blended particles back to front with an 8-bit depth key. |

All four are read once at boot.

## Effects and emitters

An effect is a `ParticleEffect` asset: how particles are born, how they move and how they look.
An emitter is an entity with a `ParticleEmitter` component, a pose and a `ParticleEffectHandle`,
and the `EmitterActive` enable tag switches it on.

```rust,ignore
use boyko_app::prelude::*;
use boyko_ecs::prelude::ResMut;
use boyko_render::{
    EmitterActive, PARTICLE_BLEND_ADDITIVE, PARTICLE_SHAPE_CONE, ParticleEffect,
    ParticleEffectHandle, ParticleEffectsExt, ParticleEmitter,
};

fn setup(mut commands: Commands, mut effects: ResMut<Assets<ParticleEffect>>) {
    // `register_effect` pins the row: an emitter's handle is a bare index.
    let sparks = effects.register_effect(ParticleEffect {
        gravity: [0.0, -9.81, 0.0],
        lifetime_min: 0.6,
        lifetime_max: 1.2,
        speed_min: 2.0,
        speed_max: 4.0,
        cone_cos: 0.8,                       // cos of the half-angle of a cone around the emitter's +Z
        emitter_shape: PARTICLE_SHAPE_CONE,
        blend_class: PARTICLE_BLEND_ADDITIVE,
        ..ParticleEffect::default()
    });

    commands
        .spawn(ParticleEmitter { rate: 200.0, accumulator: 0.0, burst: 0, speed_scale: 1.0 })
        .insert(Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)))
        .insert(ParticleEffectHandle(sparks.index()))
        .enable::<EmitterActive>();
}
```

`ParticleEmitter` fields:

| Field | Meaning |
|-------|---------|
| `rate` | Continuous spawns per second. Fractions carry over in `accumulator`, so 0.4/s spawns two particles every five seconds. |
| `burst` | One-shot spawns added to the next tick, then reset to zero. |
| `speed_scale` | Multiplies the effect's speed range for this emitter. |

`ParticleEmitter` requires `Transform`, `GlobalTransform` and `ParticleEffectHandle`, so an entity
cannot be an emitter without a pose and an effect. Toggling `EmitterActive` is an O(1) bit flip
with no archetype move; see [Enable Tags](../concepts/enable-tags.md).

`ParticleEffect` fields, in groups:

| Group | Fields |
|-------|--------|
| Birth | `emitter_shape` (`POINT`, `SPHERE`, `CONE`, `BOX`), `cone_cos`, `speed_min` / `speed_max`, `lifetime_min` / `lifetime_max` |
| Motion | `gravity`, `drag`, `rot_speed` |
| Look | `size_base` and `size_keys` (a four-key size ramp over the lifetime, at keys 0, 1/3, 2/3 and 1); `color_keys[0]` (packed RGBA8, held for the whole life); `tex_index` (a bindless texture slot, `0` = untextured) |
| Blending | `blend_class`: `PARTICLE_BLEND_ADDITIVE` or `PARTICLE_BLEND_ALPHA` |
| Collision | `collision_radius`, `restitution`, `friction` (used when `ParticleConfig::collision` is `Sdf`) |

Up to `MAX_EFFECTS` (256) effects can be registered. The size ramp is evaluated once per particle in
the simulation pass, not per vertex. Colour does not change over a particle's life yet: only the
first colour key is drawn, and `color_times` and the other colour keys are uploaded but not read.

## Timing

The particle system has its own fixed-rate clock, `ParticleClock`, at `PARTICLE_DEFAULT_HZ` (64 Hz)
by default. It advances from the engine's `Time`, so pausing the game or changing its speed
applies to particles too. It does not use the `Fixed` schedule: registering a rendering system
there would change how every event type in the app is delivered. See
[Time & Fixed Timestep](../app/time.md).

## Limits

- **Unlit.** Particles are not lit and cast no shadows.
- **SDF collisions only.** Particles collide with SDF shapes, not with meshes or physics bodies.
- **Sorting is per class.** Only the alpha class is sorted, and only with `ParticleSortMode::Radix`.
- **No CPU access** to individual particles.

## See also

- [Enable Tags](../concepts/enable-tags.md): `EmitterActive`.
- [SDF Rendering](sdf.md): the shapes particles collide with.
- [Shader eDSL](shader-edsl.md): the particle shaders are generated from Rust.
- Source: [`particle.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/particle.rs),
  [`particle_config.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/particle_config.rs),
  [`particle_effect.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/particle_effect.rs).
