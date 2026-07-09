> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# `boyko_physics` Foundation — the Manifold Currency + Swappable `RigidSolver` Seam

> **Status: REVISED (architect → critic APPROVED_WITH_NITS → revise; 3 IMPORTANT must-fixes resolved).**
> Branch `ecs`, 2026-06-17. **The "Post-Critic Binding Revisions" section at the BOTTOM is AUTHORITATIVE**
> where it conflicts with the decisions below (it pins the write-back addressing scheme, the determinism
> precondition, and the Phase-4 rebase dependency).
> Implements [docs/RENDER-PHYSICS-GPU-PLAN.md](../RENDER-PHYSICS-GPU-PLAN.md) §8 (physics) + §10 (metrics).
> **FOUNDATION/SEAM ONLY** per the binding directive "render/physics = only base, foundation for extension".
> ADD-ONLY: a new `boyko_ecs`-dependent crate, **zero core edit** (unlike the GPU Phase-4 seams).

## Goal

Ship the physics seam: the universal contact **currency** (`Manifold`), a **swappable, zero-`dyn`-on-hot-path
`RigidSolver` trait**, a **no-op default solver** (proves the seam compiles + integrates), the physics
**components** (`RigidBody`/`Collider`/`Contact`) as ordinary `#[derive(Component)]` CPU-archetype columns with
a Phase-10-ready hot/cold layout, the **physics-step system wiring** a user adds to their `Schedule`, and a
**determinism/apply-order contract**. The real Soft-Step solver, SDF-native collision, and any external
(Rapier/Jolt) backend are OUT of scope — but the seam is designed so each slots in without reshaping.

**Perf posture:** the no-op solver does no work → no hot loop to optimize yet. The deliverable's perf burden is
(a) the layout is correct now (SoA/hot-cold-split-ready columns), (b) dispatch is zero-`dyn` on the hot path
(principle 1), (c) the 0%-gate: a schedule with no physics systems pays nothing (separate crate, not linked);
the no-op step early-outs.

## Constraints
- New crate `crates/boyko_physics/`, deps `boyko-ecs`/`boyko-macros`/`boyko-utils`/`boyko-threadpool`, NO graphics.
- Consumes the existing public ECS API only (`#[derive(Component/Resource/Bundle)]`, `Query<D,F>`, `Res`/`ResMut`,
  `Mut<T>`, `Commands`, `Schedule`/`ScheduleBuilder` `.add_system().run_if().after().key()`, `EcsMaster`).
- 8 INVIOLABLE principles; load-bearing here: #1 (no `dyn` on hot path → solver dispatch), #2/#3 (DOD+cache →
  component layout), #5 (min-alloc → `Manifold` fixed-capacity, preallocated step buffers).
- §8 directives: Manifold = universal currency; `RigidSolver` swappable; components ordinary; default Soft-Step
  deferred to Phase 10; determinism ties to a stable apply order; "shared BVH = share the CODE not the
  instance"; the no-third-party rule is graphics/core-ONLY so `boyko_physics` MAY later take a Rust dep.

## Decisions

### D1 — `Manifold` is a fixed-capacity `#[repr(C)]` 2-point contact record (the universal currency)
`Manifold { points: [ContactPoint; MAX_CONTACT_POINTS], normal: Vec2, body_a, body_b, count: u8 }`, POD, `Copy`,
≤128 B (2 CLs). `MAX_CONTACT_POINTS = 2` (Box2D-v3 verified max for 2D convex-convex) as a `const` so a future
3D variant flips it to 4 with the array following. Shared `normal` lives once in the header (Box2D rationale),
not per point. Impulses are NOT in the manifold (they are the solver's warm-start state — keeps the manifold a
pure collision→solver currency). 2D-first via a `math` alias (3D = type swap, no API break).
Rejected: `Vec<ContactPoint>` per manifold (per-pair heap alloc, fails #5); `SmallVec` (branch+dep for a case
that provably never spills past 2); impulses-in-manifold (Box2D keeps them in solver constraint state).

### D2 — Solver dispatch = generic `S: RigidSolver` monomorphized step system (NOT `dyn`/enum/fn-ptr)
The step system `physics_solve_step::<S>` is generic; the solver instance is a `Resource` (`ResMut<S>`); the
user picks the backend at schedule-build time (`add_system(physics_solve_step::<NoopSolver>)`). Monomorphization
→ a **direct, inlinable** `S::solve` call, zero vtable. The trait is **deliberately NOT object-safe** (documents
that `dyn RigidSolver` is unsupported), mirroring the RHI §4 "static dispatch, not object-safe" precedent.
- `dyn` rejected: an inlining firewall across the solver boundary (the solver's per-contact loop is the Phase-10
  hot loop) + `Box<dyn>` alloc. Principle 1.
- enum rejected: closes the set — an external backend (Rapier/Jolt, a Phase-10 D10 goal) can't add a variant
  without editing `boyko_physics`; + a `match` per call + all-variants I-cache bloat.
- fn-ptr rejected: indirect call + same inlining firewall + loses the typed solver-state `Resource` (warm-start
  buffers the Phase-10 solver must own).
- generic chosen: zero-overhead AND swappable AND open-for-external simultaneously; solver state rides `S`.
Trade-off: each registered backend monomorphizes the step system (bounded code-size, paid per backend type, not
per call). A game ships ONE solver; the demo/tests use 2-3.

### D3 — The physics step is a fixed pipeline of ordinary systems; added via a `PhysicsPlugin`-shaped free fn
There is no `Plugin` trait in the engine (the demo wires systems via a fn taking `&mut ScheduleBuilder`), so
`add_physics_systems::<S>(&mut builder, &mut world) -> PhysicsStageKeys` is the faithful idiom. The pipeline:
1. `physics_integrate` — `Query<&mut RigidBody>` `par_iter_mut`: gravity + `pos += vel·dt` (the only real-work
   stage in the foundation; SoA hot loop, NOT gated by the no-op solver).
2. `physics_broadphase` — fill `ContactPairs` in deterministic `(low_id, high_id)` order (foundation: stub /
   tiny all-pairs; real BVH/grid = Phase 10).
3. `physics_narrowphase` — produce `Manifold`s into the `Manifolds` resource (foundation: real circle-circle for
   the seam test, or empty for pure no-op — see OQ1).
4. `physics_solve_step::<S>` — `if solver.is_noop() { return }` else `S::solve(..)`.
5. `physics_apply` — write solved state back through `Mut<RigidBody>` for touched rows (precise change-tick), at
   the apply window.
Ordering pinned via `.after(...)` → deterministic intra-step order; returns the stage keys so the user can
order their own systems against the block.
Rejected: a monolithic exclusive `physics_step` (kills `.before`/`.after` granularity + forces frame
serialization); a `Plugin` trait (doesn't exist; scope creep).

### D4 — Determinism = stable broadphase pair-ordering + single-threaded solve + apply-window write-back
(1) Candidate pairs emitted in a content-defined stable order — sorted by `(min(id_a,id_b), max(id_a,id_b))`,
NOT hash/pointer order (float add is non-associative → contact iteration order must be deterministic).
(2) Solve is single-threaded over the ordered manifold buffer (a pair `(a,b)` writes BOTH rows → parallel
pair-solve races; island-parallel is Phase-10+ with explicit partitioning — demo §G12).
(3) Write-back at the apply window (`running == 0`) — the deterministic, race-free tick point (§5.3 precedent).
The contract is STATED now (and partially tested) even though the solve is a stub, so the Phase-10 broadphase is
written against it. Cost: an `O(P log P)` pair sort (P = pairs ≪ entities after a real broadphase; P=0 for the
no-op foundation → free).

### D5 — `RigidBody`/`Collider`/`Contact` component layout: hot/cold split, SoA-ready
- `RigidBody` — HOT integrator state only (`position`, `linear_velocity`, `rotation`, `angular_velocity`), its
  own SoA column.
- `RigidBodyMass` — COLD mass/material as a SEPARATE component/column (`inv_mass`, `inv_inertia`, `restitution`,
  `friction`, `body_type`) → never pollutes the integrate loop's cache lines (the archetype-column model makes
  this free; `Query<&mut RigidBody>` streams only the hot column).
- `Collider` — zero-`dyn` tagged union `ColliderShape { Circle{radius}, Aabb{half_extents}, .. }` + layer/mask
  `u32` for broadphase filtering.
- `Contact` — OPTIONAL gameplay-facing queryable component (carries a `Manifold` snapshot + `other`); NOT the
  solve input (the solve reads the dense `Manifolds` resource buffer, sequential, not scattered components).
- `RigidBodyBundle` (`#[derive(Bundle)]`) bundles body+mass+collider so the user spawns with one bundle.
Rejected: one fat `RigidBody` (pollutes hot cache lines); `Box<dyn Shape>` colliders (heap+vtable, #1);
coupling solve to per-entity `Contact` (scattered random access vs the dense resource buffer).

## Key types (abridged — see the architect output for full bodies)
```rust
#[repr(C)] pub struct Vec2 { pub x: f32, pub y: f32 }
pub const MAX_CONTACT_POINTS: usize = 2;
#[repr(C)] pub struct ContactPoint { anchor_a: Vec2, anchor_b: Vec2, separation: f32, feature_id: u32 } // 24 B
#[repr(C)] pub struct Manifold { points: [ContactPoint; 2], normal: Vec2, body_a: EntityId, body_b: EntityId, count: u8, _pad } // 80 B ≤ 128

#[repr(C)] #[derive(Component)] pub struct RigidBody { position: Vec2, linear_velocity: Vec2, rotation: f32, angular_velocity: f32 } // 24 B hot
#[repr(C)] #[derive(Component)] pub struct RigidBodyMass { inv_mass, inv_inertia, restitution, friction: f32, body_type: BodyType } // cold
#[repr(C)] #[derive(Component)] pub struct Collider { shape: ColliderShape, layer: u32, mask: u32 }
#[repr(C)] #[derive(Component)] pub struct Contact { manifold: Manifold, other: EntityId }
#[derive(Bundle)] pub struct RigidBodyBundle { body: RigidBody, mass: RigidBodyMass, collider: Collider }

#[derive(Resource)] pub struct PhysicsConfig { gravity: Vec2, substeps: u32 /* reserved for Phase 10 */ }
#[derive(Resource, Default)] pub struct ContactPairs { pairs: Vec<(EntityId, EntityId)> } // reused, sorted
#[derive(Resource, Default)] pub struct Manifolds { manifolds: Vec<Manifold> }           // reused, dense
#[derive(Resource, Default)] pub struct SolverScratch { /* reused snapshot + touched bitset */ }

pub trait RigidSolver: Resource + 'static {           // NOT object-safe; static dispatch
    fn solve(&mut self, config: &PhysicsConfig, manifolds: &[Manifold], scratch: &mut SolverScratch);
    #[inline] fn is_noop(&self) -> bool { false }
}
pub struct NoopSolver; // is_noop()==true; solve(){} — the default

pub fn physics_integrate(q: Query<&mut RigidBody>, cfg: Res<PhysicsConfig>, dt: Res<FixedTime>);
pub fn physics_broadphase(/* colliders */ pairs: ResMut<ContactPairs>);
pub fn physics_narrowphase(/* */ pairs: Res<ContactPairs>, manifolds: ResMut<Manifolds>);
pub fn physics_solve_step<S: RigidSolver>(solver: ResMut<S>, cfg: Res<PhysicsConfig>, manifolds: Res<Manifolds>, scratch: ResMut<SolverScratch>);
pub fn physics_apply(q: Query<Mut<RigidBody>>, scratch: Res<SolverScratch>);
pub fn add_physics_systems<S: RigidSolver + Default>(builder: &mut ScheduleBuilder, world: &mut EcsMaster) -> PhysicsStageKeys;
```

## Multithreading
Only `physics_integrate` is parallel (`par_iter_mut` over disjoint rows — sound, the engine's verified model).
The solve is single-threaded (D4). No new sync points (rides the apply-window barrier). No atomics in the
foundation. All types `Send + Sync` (POD/`Vec`-resources; `NoopSolver` is a unit). Data-race freedom: integrate
disjoint-rows; solve is one system serialized by the conflict graph on its `ResMut`s; apply at the apply window.

## Integration
Zero core edits. One line to root `Cargo.toml` `members`. Components are ordinary `#[derive(Component)]` → normal
`ComponentPool` columns in normal CPU archetypes (post-Phase-X.B `len + row_ptr(i)` model). New modules:
`crates/boyko_physics/{Cargo.toml, src/{lib,math,manifold,components,resources,solver,systems,plugin}.rs}`.
**Merge ordering note:** Phase 4 modifies the `Component` derive (adds `RESIDENCY` default + emits
`install_residency_class`). Physics components built against the old derive recompile cleanly against the new one
(macros run at compile time; `RESIDENCY` defaults to `Cpu`). Merge Phase 4 first, then rebase + merge physics.

## Implementation waves
1. Crate scaffold + Cargo wiring + `lib.rs` re-exports; `math.rs` (`Vec2`, ops, `MAX_CONTACT_POINTS`); `manifold.rs`
   (`ContactPoint`/`Manifold` + `size_of ≤ 128` assert).
2. `components.rs` (all components + `RigidBodyBundle`); `resources.rs` (preallocated buffers, `with_capacity`).
3. `solver.rs` (`RigidSolver` trait NOT object-safe + `NoopSolver`); `systems.rs` (the 5 stages incl. the
   `(low_id, high_id)` deterministic pair-sort contract + `is_noop` early-out + `Mut` touched-row write-back).
4. `plugin.rs` (`add_physics_systems::<S>` wiring the 5 stages `.after(...)` in order + inserting resources +
   `S::default()`; returns `PhysicsStageKeys`).
5. Tests + doc pass.
Waves 1-2 are leaf; 3 needs 1-2; 4 needs 3.

## Validation
- `manifold_layout` — `size_of::<Manifold>() ≤ 128`, `#[repr(C)]` offsets, `count` range, `MAX_CONTACT_POINTS==2`.
- `components_round_trip` — spawn `RigidBodyBundle`, query back, values survive.
- `noop_step_runs_in_schedule` — real `Schedule` + `add_physics_systems::<NoopSolver>`, N bodies, run several
  frames, assert gravity·dt integration + no panic.
- `seam_implementable_by_second_solver` — a test-local `DummySolver` (zeroes velocities) implements `RigidSolver`,
  runs in a schedule, effect observable (**the seam-swappability proof**).
- `deterministic_pair_order` — fixed body set → `ContactPairs.pairs` sorted `(low_id, high_id)` identically across
  two runs.
- `static_dispatch_no_dyn` — grep/compile assert: no `dyn RigidSolver` anywhere.
- proptest `integrate_is_deterministic` — two `physics_integrate` runs over the same random input are bit-identical.
- criterion `bench_noop_step` (N=100k, integrate-dominated, < 5 ns/entity, solve/apply ≈ 0) + `bench_0pct_gate`.
- `debug_assert!`: `Manifold.count ≤ MAX`; `ContactPairs.pairs` sorted+dedup'd; apply snapshot-count == query
  row-count; `RigidBodyMass.inv_mass ≥ 0`.

## Open questions (for the critic)
1. Narrowphase stub depth — ship a REAL circle-circle narrowphase (exercises the full currency end-to-end for the
   seam test) vs a pure empty stub? Lean: real circle-circle (trivial, test-set-gated; no-op solver still ignores
   the manifolds). Confirm in-scope vs over-building.
2. `Contact` as a component vs an event (Phase 12 `EventReader`/`EventWriter`). Lean: component-only for the
   foundation, events as a Phase-10 addition.
3. `PhysicsStageKeys` struct-of-keys vs a config closure. Lean: named-keys struct (matches the demo `.key()` idiom).
4. 2D-first (`Vec2`, `MAX_CONTACT_POINTS=2`) vs setting up 4-wide 3D now. Lean: 2D-first (the demo is 2D);
   const+alias make 3D mechanical.
5. `PhysicsConfig.substeps` reserved now vs added in Phase 10. Lean: reserve it (non-breaking field add now is
   cheaper than a later ABI change).

> Sources: Box2D-v3 `b2Manifold` (2 points + shared normal + pointCount); Erin Catto, Contact Manifolds, GDC 2007.
> Reference: `boyko_demo/src/sim/systems/physics.rs` (the working CPU-archetype physics step this seam generalizes).

---

# Post-Critic Binding Revisions (AUTHORITATIVE)

> Resolutions to the critic's 3 IMPORTANT must-fixes. Where these conflict with D1–D5, THESE WIN. The critic
> verified every other API claim against live code (D2 generic dispatch, D5 hot/cold split, integration idioms
> all SOUND); only these three needed resolution.

## IM-1 — write-back addressing: the solver is ROW-INDEX-keyed (the demo's proven pattern); `EntityId` is gameplay-facing only.

The critic found the `Manifold` was `EntityId`-keyed (D1) while `physics_apply(Query<Mut<RigidBody>>)` is
row-iteration-keyed — contradictory. RESOLVED in favor of the **working reference's** scheme
(`boyko_demo/physics.rs apply_ball_motion`), which is sequential + cache-friendly + needs no exclusive scatter:

- A **gather** stage (folded into `physics_broadphase`, or a leading `physics_gather`) snapshots
  `Query<(Entity, &RigidBody, &RigidBodyMass)>` **in archetype-row order** into a dense
  `SolverScratch.bodies: Vec<BodyState>` (BodyState = the hot integrator fields + cold mass needed by the
  solve) PLUS a parallel `SolverScratch.entities: Vec<EntityId>` (row→entity map, for the gameplay `Contact`).
- The dense **row index is `BodyIndex(u32)`**. Broadphase emits pairs as `(BodyIndex, BodyIndex)`; narrowphase
  produces `Manifold`s keyed by `BodyIndex`. **`Manifold.body_a/body_b` are `BodyIndex(u32)`, NOT `EntityId`**
  (the manifold is a per-step transient solver input, not a stable cross-frame identity).
- The solver mutates `SolverScratch.bodies[idx]` in place + sets `SolverScratch.touched` (a bitset **indexed by
  `BodyIndex` = row**, answering the critic's "indexed by what?" — by row).
- `physics_apply(q: Query<Mut<RigidBody>>, scratch: Res<SolverScratch>)` does `q.iter_mut().enumerate()`: for
  row `i`, if `touched[i]`, write `scratch.bodies[i]` back through `Mut<RigidBody>` (precise change-tick). This
  is correct UNDER the **"no structural change between gather and apply" invariant** — stated explicitly and
  `debug_assert!(scratch.bodies.len() == &lt;query row count&gt;, "...")` (the demo's `:289` invariant, now named).
  Since the whole physics pipeline runs within one schedule pass and no stage spawns/despawns, the invariant
  holds; a user inserting a structural command mid-pipeline is a documented misuse.
- The gameplay-facing **`Contact` component carries `other: EntityId`** (resolved via `scratch.entities[idx]`
  during the gather) — that is the ONLY place `EntityId` appears in the physics data. The "universal currency"
  is still the `Manifold`; it is addressed by the per-step dense index, with the stable `EntityId` projected
  out for gameplay queries.
- **Phase-10 fit**: the dense, row-indexed `SolverScratch.bodies` IS the ideal SoA constraint buffer a
  Soft-Step solver wants (sequential warm-start over a packed array) — no reshape needed; the solver gathers
  once (already done) and scatters once (already done) at the seam boundary.

`Manifold` shape update: `body_a: BodyIndex, body_b: BodyIndex` (each `#[repr(transparent)] u32`) instead of
`EntityId`. This SHRINKS the manifold (u32 vs usize per body) → more 2-CL headroom (addresses MINOR-3's "3D
leaves no slack" note: u32 indices give back 8 B). Keep the `const { assert!(size_of::<Manifold>() <= 128) }`.

## IM-2 — determinism precondition (D4), stated.

Pair/solve order determinism keys on the dense `BodyIndex` = **archetype row order**, which is deterministic
**only under a deterministic spawn/despawn order**. The engine's entity-id counter is a `Relaxed` atomic
shared by parallel `Commands` workers (`entity_master.rs:141`), so under parallel `Commands::spawn` the id (and
hence row) a given logical entity receives depends on worker scheduling — NOT reproducible run-to-run. Add to
D4: *"Deterministic-across-runs physics requires a deterministic spawn order (serialized spawning, or a
content-defined ordering key independent of id/row). The foundation's single-threaded tests satisfy this; a
content-defined key is the Phase-10+ path if parallel-spawn determinism is ever required."* No code change —
an honest contract. (Keying on row order rather than `EntityId` does not change this: both derive from spawn
order.)

## IM-3 — Phase-4 rebase claim downgraded to a stated dependency.

The "Merge ordering note" is downgraded from asserted fact to: *"This ASSUMES Phase 4 ships `RESIDENCY` as a
**defaulted associated const** (mirroring the existing `HAS_HOOKS` const-fold pattern in
`boyko_macros/src/lib.rs:303`), NOT a required `#[component(...)]` attribute or a new required trait method.
Verify against the Phase-4 plan before rebasing."* (The Phase-4 plan DOES specify `const RESIDENCY:
ResidencyKind = ResidencyKind::Cpu` defaulted — so the assumption holds — but the physics plan must not state a
cross-phase fact it cannot itself verify; it states the dependency.)

## MINORs folded
- **MINOR-1**: `add_physics_systems::<S>` registers systems + inserts resources and **returns** `PhysicsStageKeys`;
  it does NOT call `builder.build(world)` (that consumes the builder and is the caller's job — `runner.rs:325`).
- **MINOR-2**: `physics_apply` wraps the whole `RigidBody` in one `Mut`, so `Changed<RigidBody>` fires for every
  moving body every frame (position is written each step). This is a **documented choice** (simpler foundation);
  a later refinement may split `Position`/`Velocity` into separate components (the demo's scheme) so
  `Changed<Velocity>` tracks collisions specifically. Not a bug; documented.
- **MINOR-3**: `Manifold` size is a hard `const { assert!(size_of::<Manifold>() <= 128) }` (not a doc comment);
  with `BodyIndex(u32)` keys (IM-1) the 3D `MAX_CONTACT_POINTS=4` flip has comfortable 2-CL headroom.
