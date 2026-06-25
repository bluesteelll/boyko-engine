# Physics

`boyko_physics` is a **fully in-house** 3D rigid-body engine. There is no Rapier,
no Jolt, no parry — no third-party physics FFI at all. The solver is written from
first principles as a Temporal Gauss-Seidel "Soft Step" (TGS-Soft) sequential-
impulse scheme, in the Box2D-v3 lineage, and it runs as **ordinary ECS components
and systems on the engine's own threadpool**. A soft-body (XPBD) path lives in the
same crate.

The design choice that makes this work is the one the rest of the engine is built
on: physics is not a subsystem glued on the side with its own parallel data
structures. A body's state lives in `ComponentPool` columns; the per-step solve
state lives in dense, non-fragmenting kernel storage. "ECS-native" and "cache-
optimal" are the same thing here.

## Why fully in-house

An FFI physics backend forces a parallel data system: you keep your authoritative
transforms in the ECS, then mirror them into the foreign library's own body arrays
every frame, solve there, and copy back. That mirror is a second source of truth.
It also defeats cache locality (you pay a scatter/gather across the FFI boundary
every step) and it defeats determinism control (you inherit the backend's float
contraction and threading model).

boyko-engine instead owns every byte. The solver reads the dense
`SolverScratch` snapshot, mutates it in place, and writes back only the rows it
touched. Float behavior is pinned for **bit-determinism** — exact `sqrt` and `1/x`,
no `rsqrt`/`rcp`, no FMA contraction, no `fast-math` — so the same scene produces
the same result run-to-run, and (on the parallel/SIMD paths) bit-identically across
worker counts.

The seam is still open: an external backend *could* slot in by implementing the
[`RigidSolver`](#the-rigidsolver-seam) trait on its own resource, with no edit to
this crate. The shipped default is the in-house solver.

## The body model — capability is structural

A body is just a set of components. What kind of body it is comes from **which
components are present**, not a `BodyType` enum branch.

- `RigidBody` — the **hot** integrator state (position, linear velocity, rotation,
  angular velocity), in its own SoA column so the integrate loop streams a tight,
  cache-dense buffer.
- `RigidBodyMass` — the **cold** mass/material (inverse mass, inverse inertia
  *tensor*, restitution, friction), in a *separate* column so it never pollutes the
  integrate cache lines.
- `Collider` — a zero-`dyn` tagged-union shape (`ColliderShape::Sphere` or
  `ColliderShape::Box`, an oriented OBB) plus a `layer`/`mask` broadphase filter.

A permanent static body simply **does not carry** a `RigidBody` (structural skip —
the integrator never iterates it). An immovable contact surface carries
`RigidBody` with `inv_mass == 0`. There is no enum to branch on.

Runtime on/off is a **bit**, not a component swap:

- `Simulated` — an [enable-tag](../concepts/enable-tags.md) bit
  (`#[component(storage = "bitset")]`). A body whose `Simulated` bit is set
  integrates under gravity and is advanced by the solver. Clearing it "parks" the
  body — its pose freezes in place — with **O(1) toggle and no archetype
  migration**. The integrate stage and the solver both read this bit non-filteringly
  through `IsEnabled<Simulated>`, so flipping it never reorders the physics gather.
- `Kinematic` — another enable-tag bit, for a body moved by external control only.
  (Kinematic *motion* — actually advancing a kinematic body's pose from a target —
  is a documented deferral, not yet built.)

This is the engine-wide rule: **capability = component presence; runtime state = an
`IsEnabled<T>` bit.** It replaces the old `BodyType` enum entirely.

> Solver-state-in-a-side-`Vec` is exactly the anti-pattern that caused a real data
> race (the "SP4 race") in an earlier iteration. The fix — and the standing rule —
> is that durable per-body solve state lives in the kernel's own dense storage, not
> a `std::Vec` mirror.

### Spawning a body

```rust,ignore
use boyko_ecs::prelude::*;
use boyko_physics::{
    RigidBody, RigidBodyMass, Collider, ColliderShape, RigidBodyBundle, Simulated,
};
use boyko_physics::math::{Vec3, Mat3, Quat};

// `RigidBodyBundle` is a named `#[derive(Bundle)]` struct — a bare tuple is NOT a
// bundle. The three columns spawn together in one call.
let bundle = RigidBodyBundle {
    body: RigidBody {
        position: Vec3::new(0.0, 5.0, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,        // a default body has a valid identity quat
        angular_velocity: Vec3::ZERO,
    },
    mass: RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,     // unit-tensor placeholder
        inv_mass: 1.0,                   // 0.0 = immovable
        restitution: 0.5,
        friction: 0.3,
    },
    collider: Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    },
};

let entity = commands.spawn(bundle).id();
// The body does NOT simulate yet — the `Simulated` bit can't be a bundle field
// (a bitset tag has no column). Enable it (deferred, O(1), no migration):
commands.entity(entity).enable::<Simulated>();
```

For the common "I want a visible body that simulates immediately" case, the std-lib
provides the `DynamicBody` bundle (pose + render handles + physics columns) and a
`spawn_dynamic(commands, bundle)` helper that spawns it **and** enables `Simulated`
in one call. A non-blocking overlap volume is the `Trigger` bundle (a `Collider`
plus the `Sensor` marker — the solver detects the overlap and reports it, but
applies no impulse).

## The TGS-Soft solver

The shipped default solver is `SoftStepSolver`. For each step it runs a velocity-
level sequential-impulse solve over the deterministic manifold order:

- **Inertia tensor** — the cold `RigidBodyMass::inv_inertia` is the *world-space*
  inverse inertia tensor, so angular response is `Δω = inv_inertia · τ_world`,
  paired with the world-frame quaternion integrate.
- **2-DOF Coulomb friction cone** — a coupled two-tangent friction solve clamped to
  the normal impulse (not two independent 1-DOF axes).
- **Soft penetration recovery** — a soft-constraint bias (`contact_hertz` /
  `contact_damping`) pushes overlapping bodies apart smoothly instead of snapping,
  with a clamped maximum bias velocity so a deep initial overlap cannot launch a
  body.
- **Warm-starting** — each contact *point* persists its converged accumulated
  impulses (normal + 2 tangent) across frames in a double-buffered table, keyed by
  `(body_a, body_b, feature_id)`. This is what lets a stack rest instead of
  jittering apart under a fixed substep budget.
- **Restitution** — a single post-loop pass, gated by an approach-speed threshold
  so a body resting under gravity does not creep upward frame after frame.

The solver **owns integration**: it integrates simulated dynamic bodies inside its
own substep loop, so the pipeline's standalone integrate stage is gated off (see
[the pipeline](#the-pipeline)).

### Contact shapes

The narrowphase generates manifolds for:

- **sphere–sphere**, **sphere–box**, **box–box** (OBB, with feature-id-stable
  multi-point manifolds), and
- **body–vs–SDF** — a body resolved against the analytic signed-distance field, the
  *same* CPU-authoritative edit list the renderer draws (see
  [SDF rendering](../rendering/sdf.md)). SDF contacts use a sentinel `body_b` and
  ride the one-sided immovable-surface impulse path, with **zero GPU readback**. The
  SDF narrowphase is opt-in (`add_physics_sdf`).

## The `RigidSolver` seam

The solver is swappable behind a trait, and the dispatch is **static — deliberately
not object-safe**:

```rust,ignore
use boyko_physics::{PhysicsConfig, Manifold, SolverScratch};

pub trait RigidSolver: /* Resource + */ 'static {
    /// Resolve all contacts for one step: mutate `scratch.bodies` in place and
    /// flag `scratch.touched` for every row written.
    fn solve(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    );

    fn is_noop(&self) -> bool { false }          // skip the solve entirely
    fn owns_integration(&self) -> bool { false } // solver integrates internally
}
```

Because a `Resource` is `Sized`, `dyn RigidSolver` does not compile by design. The
step system is generic over `S: RigidSolver`; the solver rides as a `ResMut<S>` and
the user picks the backend at schedule-build time. Monomorphization makes the per-
contact loop a direct, inlinable call with **zero vtable** — the hot loop inlines
across the seam instead of being firewalled behind dynamic dispatch.

The crate ships three solvers:

| Solver | What it does | `owns_integration` |
|--------|--------------|--------------------|
| `NoopSolver` | The default seam-prover: `is_noop()` is `true`, the solve is skipped, the pipeline degenerates to integrate-only. | `false` |
| `SoftStepSolver` | The real TGS-Soft solver (above). Solves in manifold order. | `true` |
| `ColoredSoftStepSolver` | The colored/parallel/SIMD solve (below). | `true` |

## The pipeline

Physics is wired into a schedule as a block of ordinary systems, registered in a
fixed order via `.after(...)`. The body-only pipeline (`add_physics_systems::<S>`)
runs:

```mermaid
flowchart LR
    A[physics_integrate] --> B[physics_gather]
    B --> C[physics_broadphase]
    C --> D[physics_narrowphase]
    D --> E["physics_solve_step::&lt;S&gt;"]
    E --> F[physics_apply]
```

- **integrate** — `par_iter_mut` over simulated dynamic bodies: gravity, position
  advance, quaternion advance. **Gated off** when the solver owns integration (the
  TGS path), so the solver is the *sole* integrator and bodies are never double-
  integrated. This gate is the `IntegrationMode::SolverOwned` vs
  `IntegrationMode::Foundation` resource, derived from `owns_integration()` at wire-
  up time.
- **gather** — snapshots `(RigidBody, RigidBodyMass, Collider)` in row order into
  the dense `SolverScratch`, derives each body's world inverse inertia, and stamps
  the step `dt` from the fixed clock (`FixedTime`) into `PhysicsConfig`.
- **broadphase** — emits candidate pairs in deterministic `(min, max)` order.
- **narrowphase** — produces `Manifold`s for the overlapping pairs.
- **solve** — `if solver.is_noop() { return } else { S::solve(...) }`.
- **apply** — writes the solved snapshot back through `Mut<RigidBody>`, but only for
  touched rows.

Physics is meant to live in the **fixed-timestep schedule** — the gather reads the
fixed step's `dt`, never a per-render-frame delta. See [Time and the fixed
timestep](../app/time.md).

```rust,ignore
use boyko_physics::{add_physics_systems, SoftStepSolver};

// Inserts the physics resources on `world` and registers the six stages on
// `builder`, returning the stage handles. (The caller owns `builder.build`.)
let keys = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
```

Wiring variants extend this block, each opt-in:

| Function | Adds |
|----------|------|
| `add_physics_systems::<S>` | the body-only pipeline (above) |
| `add_physics_systems_with_scene_sync::<S>` | the same, wrapped in `Transform ⇄ RigidBody` pose sync |
| `add_physics_sdf::<S>` | a body-vs-SDF narrowphase stage + an (empty) `SdfField` to fill |
| `add_physics_colored::<S>` | builds the constraint-graph islands/coloring (partition only) |
| `add_physics_colored_solve` | the colored solve via `ColoredSoftStepSolver` |
| `add_physics_soft` | the XPBD soft-body pass |
| `add_physics_soft_colored` | the colored-parallel soft-body pass |

### Pose stays in one datum

With scene sync, the body's pose has exactly one writer per window: the solver
writes `RigidBody`, `sync_body_to_transform` mirrors it into `Transform`, and
`propagate_transforms` (in `boyko_scene`) derives `GlobalTransform`. There is no
parallel pose store — see [Transforms](transforms.md).

## Tuning

`PhysicsConfig` is the global tunable resource. Most fields are user-set; `dt` is
**not** — it is stamped each step from the fixed clock.

| Field | Default | Meaning |
|-------|---------|---------|
| `gravity` | `(0, -9.81, 0)` | constant acceleration on dynamic bodies |
| `substeps` | `4` | TGS substeps per step (the solver reads `h = dt / substeps`) |
| `relax_iterations` | `2` | bias-free relaxation passes per substep |
| `contact_hertz` | `30.0` | soft-constraint stiffness (penetration-recovery spring frequency) |
| `contact_damping` | `10.0` | soft-constraint damping ratio (heavily overdamped, for stable resting contact) |
| `dt` | stamped | the fixed step delta, written by the gather — a hand-set value is overwritten |

## Scaling — the opt-in performance paths

These are the production-scale levers from the physics optimization campaign. Every
one is **default-off and opt-in**, and each preserves the 0%-gate: a world that does
not opt in is byte-identical to the shipped scalar path. Where a path *does* change
the converged float values (the colored solve reorders the sweep), it is validated
against tolerance acceptance gates, stays bit-deterministic run-to-run, and never
moves a static body.

- **Grid broadphase** (`broadphase = BroadphaseKind::Grid`) — a uniform-grid CSR
  counting-sort replacing the O(n²) all-pairs loop. Its pair set is bit-identical
  to all-pairs after the same feasibility filter and `(min, max)` sort.
  `parallel_broadphase` fans the candidate emit across the threadpool.
- **Constraint coloring** — islands + greedy graph coloring so no color shares a
  dynamic body; the enabler for parallel and SIMD solving.
- **Colored solve** (`ColoredSoftStepSolver` via `add_physics_colored_solve`) — a
  Gauss-Seidel sweep across colors over SoA contact columns. `parallel_solve` runs
  each color across workers; `simd_solve` widens the per-color sweep with 8-lane
  AVX2 cohorts. Both are bit-identical to the single-threaded colored result for any
  worker count.
- **SIMD integrate/inertia** (`simd`) — AVX2 width-only kernels for the per-substep
  inertia refresh and integrate. Each lane mirrors the scalar op sequence exactly
  (no FMA, no `rsqrt`), so the SIMD output is **bit-identical** to scalar — toggling
  it changes performance, never the result.
- **Sleeping** (`sleeping`) — per-island deactivation: an island below a speed²
  threshold for a debounce window freezes and skips its solve/integrate, while the
  gather still walks every row (so warm keys stay valid and a new contact wakes the
  island the same frame).

> **Targets, not measured results.** The campaign's headline numbers (grid
> broadphase crossover, ~workers× on the colored solve, ~2–2.3× AVX2 on the contact
> solve) are *targets to validate*, not claimed benchmark results. This page does
> not quote a benchmark figure as a fact.

## Soft bodies (XPBD)

A separate, opt-in path (`add_physics_soft`, config `soft_body = true`) advances
`SoftBody` components by an XPBD position pass after the rigid solve. It is a
*strictly disjoint* integrator — it operates only on the soft-body columns and never
touches the rigid `SolverScratch`, so the rigid simulation is byte-identical whether
soft bodies are present or not.

A `SoftBody` is a particle cloud tied by distance constraints, stored **SoA by
axis** (`pos_x/y/z`, `prev_x/y/z`, `vel_x/y/z`, per-particle `inv_mass`) with an
immutable constraint topology, all preallocated and refilled in place (zero per-step
allocation). A particle with `inv_mass == 0.0` is pinned. Constructors validate the
topology up front so the solver never re-validates on the hot path:

```rust,ignore
use boyko_physics::SoftBody;

// A cloth/lattice from particle positions + edge constraints.
let soft = SoftBody::from_mesh(
    &positions,   // &[[f32; 3]]
    &inv_masses,  // &[f32]  (0.0 pins a particle)
    &edges,       // &[(u32, u32)] distance constraints
    None,         // rest lengths (None = current distance)
    0.0,          // XPBD compliance (0.0 = perfectly stiff)
    0.05,         // particle radius
).expect("invariant: valid soft-body topology");
```

Shipped soft-body capability, honestly: distance constraints (`from_mesh` /
`from_mesh_per_edge`); volume-constraint tetrahedra (`from_tet_mesh`); per-substep
viscous damping and a rest-velocity clamp; particle self-collision; two-way
soft↔rigid coupling; and a colored-parallel path. Each is its own opt-in flag on
`PhysicsConfig`.

## Determinism — the contract

The shipped scalar path is bit-deterministic: single-threaded over the
deterministic manifold order, fixed point order, normal-before-friction, fixed
substep/relax counts, fixed float op order, no atomics, no `fast-math`. The parallel
and SIMD opt-ins are bit-identical to it across `{1 thread, N threads, SIMD-on,
SIMD-off}` — the disjoint-body coloring makes each body's accumulation independent
of which worker runs which group, and the warm-start store is forced into canonical
order regardless of solve-dispatch order.

One precondition: dense-row order is the archetype row order, which is deterministic
across runs only under a deterministic spawn/despawn order. Single-threaded spawning
satisfies this.

## See also

- [Time and the fixed timestep](../app/time.md) — where physics runs
- [Enable-tags](../concepts/enable-tags.md) — the `Simulated`/`Kinematic` mechanism
- [Transforms](transforms.md) — the single-source-of-truth pose pipeline
- [Math](math.md) — the deterministic POD `Vec3` / `Quat` / `Mat3`
- [SDF rendering](../rendering/sdf.md) — the field physics shares for SDF contacts
- Source:
  [`boyko_physics/src/lib.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/lib.rs#L1),
  [`components.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/components.rs#L1),
  [`solver/mod.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/solver/mod.rs#L46),
  [`solver/soft_step.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/solver/soft_step.rs#L1),
  [`systems.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/systems.rs#L1),
  [`plugin.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/plugin.rs#L159)
