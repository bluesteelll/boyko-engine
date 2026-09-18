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

The crate ships two TGS-Soft solvers with the same contact model. They differ in the
order in which they sweep the contacts:

- **`ColoredSoftStepSolver` — the default.** The type alias `DefaultRigidSolver`
  names it. It sweeps the contacts color by color over a constraint graph (see
  [Scaling](#scaling--the-performance-paths)), with the AVX2 cohort kernel on
  (`PhysicsConfig::simd_solve` defaults to `true`).
- **`SoftStepSolver` — the reference.** It sweeps the contacts in the deterministic
  manifold order. It stays in the tree as the oracle the colored solve is checked
  against, and a world runs it only when it names it.

For each step, both run a velocity-level sequential-impulse solve with:

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

Both solvers **own integration**: each integrates simulated dynamic bodies inside its
own substep loop, so the pipeline's standalone integrate stage is gated off (see
[the pipeline](#the-pipeline)).

### The default and the reference differ in value, not in validity

The colored sweep visits the contacts in a different order than the manifold sweep,
so the two solvers converge to **different, equally valid floats**. They are compared
by tolerance acceptance gates (stacking, penetration, friction, restitution), never
by bits. Two consequences follow:

- Moving a world from one solver to the other changes its simulation values.
- A world or a replay pinned to the reference must name `SoftStepSolver` explicitly
  (see [the pipeline](#the-pipeline)).

Each solver is bit-deterministic run to run on its own; see
[Determinism](#determinism--the-contract).

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
| `NoopSolver` | The foundation seam-prover: `is_noop()` is `true`, the solve is skipped, the pipeline degenerates to integrate-only. | `false` |
| `SoftStepSolver` | The reference TGS-Soft solver (above). Solves in manifold order. Runs only when a world names it. | `true` |
| `ColoredSoftStepSolver` | The default (`DefaultRigidSolver`). Solves in graph-color order, with the parallel and SIMD paths (below). Its trait `solve` is empty: the `physics_solve_colored` stage drives it, because the trait signature carries no constraint graph. | `true` |

`DefaultRigidSolver` is a type alias for `ColoredSoftStepSolver`, not a fourth solver.

## The pipeline

Physics is wired into a schedule as a block of ordinary systems, registered in a
fixed order via `.after(...)`. The body-only pipeline (`add_physics_systems::<S>`)
runs the stages below. The solver **type** `S` picks the solve stage, once, at
wire-up:

```mermaid
flowchart LR
    A[physics_integrate] --> B[physics_gather]
    B --> P[select_broadphase]
    P --> C[physics_broadphase]
    C --> D[physics_narrowphase]
    D -->|"S = ColoredSoftStepSolver (default)"| G[physics_build_graph]
    G --> E[physics_solve_colored]
    D -->|"any other S"| R["physics_solve_step::&lt;S&gt;"]
    E --> F[physics_apply]
    R --> F
```

- **integrate** — `par_iter_mut` over simulated dynamic bodies: gravity, position
  advance, quaternion advance. **Gated off** when the solver owns integration (both
  TGS solvers do), so the solver is the *sole* integrator and bodies are never
  double-integrated. This gate is the `IntegrationMode::SolverOwned` vs
  `IntegrationMode::Foundation` resource, derived from `owns_integration()` at wire-
  up time.
- **gather** — snapshots `(RigidBody, RigidBodyMass, Collider)` in row order into
  the dense `SolverScratch`, derives each body's world inverse inertia, and stamps
  the step `dt` from the fixed clock (`FixedTime`) into `PhysicsConfig`.
- **select_broadphase** — a cold density policy. In the default
  `BroadphaseSelectMode::Manual` it only counts bodies and never changes
  `PhysicsConfig::broadphase`.
- **broadphase** — emits candidate pairs in deterministic `(min, max)` order.
- **narrowphase** — produces `Manifold`s for the overlapping pairs.
- **build_graph** — registered only for the colored solver. Partitions this step's
  manifolds into islands and greedy-colors them so no color shares a dynamic body.
- **solve** — `physics_solve_colored` for the colored solver. For any other `S`,
  `physics_solve_step::<S>`: `if solver.is_noop() { return } else { S::solve(...) }`.
  The two stages never both run, and `NoopSolver` can never be paired with the
  colored stage.
- **apply** — writes the solved snapshot back through `Mut<RigidBody>`, but only for
  touched rows.

Physics is meant to live in the **fixed-timestep schedule** — the gather reads the
fixed step's `dt`, never a per-render-frame delta. See [Time and the fixed
timestep](../app/time.md).

```rust,ignore
use boyko_physics::{add_physics_systems, DefaultRigidSolver};

// Inserts the physics resources on `world` and registers the pipeline stages on
// `builder`, returning the stage handles. (The caller owns `builder.build`.)
// `DefaultRigidSolver` wires the constraint graph and the colored solve.
let keys = add_physics_systems::<DefaultRigidSolver>(&mut builder, &mut world);
```

Rust has no default type parameters on functions, so the alias `DefaultRigidSolver`
is the one place the default is named. To run the reference solver instead, name it:

```rust,ignore
use boyko_physics::{add_physics_systems, SoftStepSolver};

// The reference manifold-order solve: no constraint graph, no colored stage.
// Its floats differ from the default's, so a world pinned to it must say so here.
let keys = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
```

Wiring variants extend this block, each opt-in. Every entry picks the solve stage
from `S` the same way, so the scene-sync, SDF and soft-body variants all run the
colored solve when given `DefaultRigidSolver`:

| Function | Adds |
|----------|------|
| `add_physics_systems::<S>` | the body-only pipeline (above) |
| `add_physics_systems_with_scene_sync::<S>` | the same, wrapped in `Transform ⇄ RigidBody` pose sync |
| `add_physics_sdf::<S>` | a body-vs-SDF narrowphase stage + an (empty) `SdfField` to fill |
| `add_physics_colored::<S>` | the constraint-graph stage for any `S`; with a non-colored solver it is a partition only and the solve is unchanged |
| `add_physics_colored_solve` | kept for existing callers; forwards to `add_physics_systems::<ColoredSoftStepSolver>` |
| `add_physics_soft::<S>` | the XPBD soft-body pass |
| `add_physics_soft_colored::<S>` | the colored-parallel soft-body pass |

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

## Scaling — the performance paths

These are the production-scale levers from the physics optimization campaign. The
colored solve is the default solver, and two AVX2 kernels are **on by default**:
`simd_solve` and `simd`. Both kernels are pure speed paths: each lane mirrors the
scalar op sequence exactly (no FMA, no `rsqrt`/`rcp`), so turning one off changes
performance, never a result bit. On a non-AVX2 build both are no-ops.

The rest are **default-off and opt-in**: `broadphase = Grid`, `parallel_broadphase`,
`parallel_solve` and `sleeping`. Each one's off state leaves the default path
byte-identical (the campaign's 0%-gate).

- **Grid broadphase** (`broadphase = BroadphaseKind::Grid`, default `AllPairs`) — a
  uniform-grid CSR counting-sort replacing the O(n²) all-pairs loop. Its pair set is
  bit-identical to all-pairs after the same feasibility filter and `(min, max)`
  sort. `parallel_broadphase` (default off) fans the candidate emit across the
  threadpool.
- **Constraint coloring** — islands + greedy graph coloring so no color shares a
  dynamic body; the enabler for parallel and SIMD solving. The colored solve
  consumes it, so every world on the default solver builds it.
- **Colored solve** — the default solver (`DefaultRigidSolver`, through any
  `add_physics_*` entry). A Gauss-Seidel sweep across colors over SoA contact
  columns. Its converged values differ from the reference solver's; they are
  validated against tolerance acceptance gates, stay bit-deterministic run-to-run,
  and never move a static body.
  - `simd_solve` (**default on**) widens each color's sweep over cohorts of 8
    body-disjoint manifold-groups with an AVX2 kernel. It is bit-identical to the
    scalar colored oracle.
  - `parallel_solve` (default off) runs each color's groups across workers, with a
    barrier between colors. It is bit-identical to the single-threaded colored
    result for any worker count.
- **SIMD integrate/inertia** (`simd`, **default on**) — AVX2 width-only kernels for
  the per-substep inertia refresh and the gravity integrate loop. The SIMD output is
  **bit-identical** to scalar — toggling it changes performance, never the result.
- **Sleeping** (`sleeping`, default off) — per-island deactivation: an island below a
  speed² threshold for a debounce window freezes and skips its solve/integrate, while
  the gather still walks every row (so warm keys stay valid and a new contact wakes
  the island the same frame). Unlike the other levers, it changes results on
  purpose — a frozen island stops integrating — so its gate is "rest state with
  sleeping on equals rest state with sleeping off, to ε". Two combinations the
  default solver makes reachable have no gate yet: sleeping with SDF contacts is
  unmeasured, and with soft↔rigid coupling a soft→rigid reaction does not wake a
  sleeping body.

`simd_solve`, `parallel_solve` and `sleeping` act only inside the colored solve. With
the reference `SoftStepSolver` they are silent no-ops.

> **What is measured, and what is not.** Two figures from
> [`benches/colored_solve.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/benches/colored_solve.rs)
> back the default. The AVX2 cohort kernel runs the colored step **1.96×** faster
> than the scalar colored arm, on that bench's production-shaped sphere pile
> (one-point manifolds, not box–box contacts). The scalar colored solve, graph build
> included, runs **1.059×** faster than the reference solver at ~1k contacts and
> **1.131×** at ~10k. The figures come from different bench groups, and neither
> measures a whole default world. The campaign's other headline numbers (grid
> broadphase crossover, ~workers× on the colored solve) remain *targets to
> validate*, not claimed benchmark results.

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

Each solver is bit-deterministic run to run: a fixed sweep order (graph colors for
the default, manifold order for the reference), fixed point order,
normal-before-friction, fixed substep/relax counts, fixed float op order, no
atomics, no `fast-math`.

On the default colored solve, the speed switches never change a bit. The AVX2 cohort
kernel (`simd_solve`, on by default), the scalar colored oracle, and `parallel_solve`
at any worker count all produce the same result — bit-identical across `{1 thread,
N threads, SIMD-on, SIMD-off}`. The disjoint-body coloring makes each body's
accumulation independent of which worker runs which group, and the warm-start store
is forced into canonical order regardless of solve-dispatch order. So a replay does
not depend on `simd_solve`. At scale, the release run of
`tests/default_world_pyramid_determinism.rs` steps a 1240-box pyramid for 120 frames.
Its per-frame hash of every `RigidBody` bit matches run to run, at 1, 2 and 8
workers with `parallel_solve` off and on, and against the scalar colored oracle.

The two solvers do **not** match each other bit for bit (see
[above](#the-default-and-the-reference-differ-in-value-not-in-validity)), so a
replay pinned to one solver must run under that solver.

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
  [`solver/colored.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/solver/colored.rs#L1),
  [`resources.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/resources.rs#L1),
  [`systems.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/systems.rs#L1),
  [`plugin.rs`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_physics/src/plugin.rs#L159)
