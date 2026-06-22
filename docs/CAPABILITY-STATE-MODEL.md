# Engine-Wide Capability + State Model

**Status:** owner-approved convention (this session). Normative for EVERY subsystem
(render, lighting, physics, GUI, audio, AI, anything future). Implementation is
phased; conformance status is tracked at the bottom.

The engine models "what an entity *can* do, and whether it is *doing it right now*"
on **exactly two axes**, plus one cold non-axis.

## The two axes (+ cold metadata)

| | Mechanism | In archetype signature? | Reorders iteration? | Toggle cost | Use for |
|---|---|---|---|---|---|
| **Axis 1 — PERMANENT capability** | **component presence** (column / ZST marker / dense column) | yes | yes (on add/remove) | spawn-time; **zero cost for the incapable** (never iterated) | "principally cannot do X" |
| **Axis 2 — RUNTIME on/off** | **`EnableTag` bitset** (`#[component(storage = "bitset")]`) | **no** | **no** | **O(1) bit flip, no migration** | "can do X, currently off/on" |
| (non-axis) **cold METADATA** | `#[repr(u8)]` enum / field | yes (a cold column) | n/a (never the gate) | — | authoring intent, save/load discriminant |

## The decision rule (every subsystem answers these)

1. **"Can an entity that will NEVER participate in X exist?"** → gate participation
   by **Axis 1** (component presence). That entity simply lacks X's component → X's
   queries match zero of its archetypes → **branch-free, zero-iteration**. "Cannot
   do X" = **absence**, never a `disabled` flag the system iterates over.
2. **"Among entities that CAN do X, does on/off flip during gameplay?"** → that flip
   is **Axis 2** (an `EnableTag` bit), filtered `Enabled<Tag>` / `Disabled<Tag>`.
   Never a marker component (archetype migration per flip), never an enum-field the
   system iterates-and-branches over.
3. **"Is there an authoring/serialization discriminant the system reads but does NOT
   gate on?"** → cold `#[repr(u8)]` metadata, bridged into Axis 2 by a
   `Changed<Meta>`-gated sync system (the `visibility_sync` pattern). The enum is
   **never** the iteration gate.

## The determinism rule (load-bearing, not stylistic)

- An `EnableTag` bit lives at `(archetype, row)` and is **NOT** in the archetype
  signature → flipping it never moves a row between archetypes → **never reorders
  archetype-row iteration**. This is what makes Axis 2 safe for order-dependent
  deterministic systems (the physics contact solve: float add is non-associative,
  so contact order — derived from gather/archetype order — determines the bit-exact
  result).
- A **marker COMPONENT** for the same state would split the archetype → toggling =
  archetype **migration** AND reorders iteration → **bit-divergence** in an
  order-dependent solve. ⇒ a marker component is correct ONLY for **permanent/rare**
  capability (Axis 1); a frequently-toggled state is ALWAYS a bitset (Axis 2).
- Industry corroboration: flecs `ecs_enable_id` and Unity DOTS enableable components
  are bitset toggles with no archetype move (= boyko `EnableTag`); Bevy's `Disabled`
  is a migration marker (the slow path we avoid); Box2D documents that contact array
  order determines the solver result.

## Performance model

- **Axis 1:** the incapable are **never visited** — zero instructions, zero cache
  traffic. This is the ECS substrate, not a feature.
- **Axis 2:** **O(1) toggle** (one atomic bit op in the apply window). Query
  treatment is **iterate-but-skip** (per-row bit test), *not* zero-iterate. Known
  cost: an `Enabled<Tag>`-filtered query forfeits the `for_each_chunk` SoA fast path
  (the enable filter is non-archetypal, per-row). Accept per-subsystem (render
  already does); a chunk-aware vectorizable enable filter is a possible future kernel
  enhancement.

## Per-subsystem conformance

| Subsystem | Axis 1 (presence) | Axis 2 (bitset) | Metadata | Status |
|---|---|---|---|---|
| **Render** | `MeshHandle`/`MaterialHandle` + dense `Gpu3dInstance` | `RenderEnabled` (`Enabled<RenderEnabled>`) | `Visibility` byte → bit via `visibility_sync` | **EXEMPLAR — conforms** |
| **Physics** | `RigidBody` (integrable) / `Collider` (collidable) / `Kinematic` marker | `Simulated` bit (currently-dynamic; O(1) static↔dynamic flip) | `BodyType` enum (authoring/serde only) | **IN PROGRESS** |
| **Lighting** | light-component presence | `LightEnabled` bit (+ a dirty channel to re-run `collect_lights`) | — | **PLANNED (Axis-2 gap)** |
| **GUI** | `Focusable`/`Interaction`/`FocusPolicy` | EnableTag visible/hovered/pressed bits | `Interaction` byte | **conforms** |
| **S6 bundles** | each bundle carries the right components; excludes bitset/dense fields | (applied post-spawn) | `Visibility` byte | **conforms** |

### Physics — the 5 cases (the hard determinism case)

| Case | Axis 1 components | Axis 2 `Simulated` bit |
|---|---|---|
| no physics | — (render-only / logic) | — |
| collision-only obstacle | `Collider` (+ `RigidBody`, `inv_mass`=0 for v1) | cleared |
| runtime-toggleable static↔dynamic | `RigidBody` + `Collider` | flips O(1) |
| dynamic | `RigidBody` + `Collider` | set |
| kinematic (user-driven) | `Kinematic` marker (+ `RigidBody`/`Collider`) | — (driven from `Transform`) |

- **Encoding (A), mandatory:** `physics_gather` is UNCHANGED — it still gathers
  EVERY body in the same order; the `Simulated` bit gates the **integrate step +
  value-write**, NOT gather membership. (Encoding B — filtering gather by the bit —
  is DISQUALIFIED: removing rows shifts dense `BodyIndex` → reorders contacts →
  bit-divergence.) Because the bit is an `EnableTag` (no archetype split), the gather
  array's membership and order are identical to today ⇒ the contact solve is
  **byte-identical** to the pre-change engine. **Zero perf cost** (no EntityId sort —
  run-to-run determinism already holds; layout-stable gather is unnecessary).
- `physics_integrate`: `Query<(&mut RigidBody, &RigidBodyMass), Enabled<Simulated>>`;
  a permanent-static body with no `RigidBody` is zero-iterate. `scene_sync` splits on
  `Enabled<Simulated>` / `Disabled<Simulated>`. Static bodies still collide
  (`Collider` presence; `inv_mass`=0 branchless in the solve).

## Owner-confirmed decisions

- Scope now: close the real gaps (physics, lighting); render/GUI/bundles already conform.
- **No perf cost** without necessity → no EntityId-sort / no layout-stable gather (the
  `EnableTag` bit is byte-identical for the static↔dynamic flip; run-to-run
  determinism suffices for lockstep/replay).
- collision-only-without-`RigidBody` = no for v1; sleeping/`IslandSleep` stays a
  separate solver-energy state; per-subsystem bits (no single master `EntityEnabled`);
  hierarchical `InheritedVisibility` propagation + `IslandSleep` off the `Resource`
  side-store = deferred later phases.
