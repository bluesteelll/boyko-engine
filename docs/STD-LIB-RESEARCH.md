# boyko-engine Standard Library — Feature Catalog (owner alignment menu)

This is a **menu**, not an implementation plan. It maps the cross-engine survey (Unity/Unreal/Godot/Bevy/flecs/EnTT) onto boyko's existing ECS, expressing every "object category" the owner asked for as **component composition** — never a God-class. Legend per item: **what it is · ECS-native shape · status · priority**.

Status: ✅ have · 🟡 partial (exists but not shareable / not ECS-native) · ❌ missing.
Priority: **CORE** (nothing else works without it) · **IMPORTANT** (needed for a real game-object kit) · **OPTIONAL** (scaffolding / later).

---

## (A) Spatial core — Transform / math / propagation

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **Math types** | `Vec2/3/4`, `Quat`, `Mat3/4`, affine, with full ops | A shared `boyko_math` crate (or `boyko_utils::math`); `#[repr(C)]`, SIMD-aligned, POD | 🟡 `Vec3/Quat/Mat3` exist **private to `boyko_physics`** (`math.rs:29/213/378`); **no `Vec2/Vec4/Mat4/affine`** | **CORE** |
| **`Transform`** | Local pose relative to parent: translation + rotation + scale | `#[derive(Component)]` POD `{ translation: Vec3, rotation: Quat, scale: Vec3 }` (decomposed, designer-friendly) | ❌ | **CORE** |
| **`GlobalTransform`** | Cached world-space pose, fast for render/physics/GPU upload | `#[derive(Component)]` storing a single packed affine (`Affine3A`-style `Mat3×Vec3`) — represents shear from non-uniform chains | ❌ | **CORE** |
| **Transform propagation system** | Composes `GlobalTransform = parent.global ∘ local` over the hierarchy | A late-stage system over `ChildOf`/`Children`; roots get `global = local`; dirty-bitset / change-detection gated | ❌ (hierarchy exists, carries **no transform semantics** — `hierarchy/mod.rs`) | **CORE** |
| **2D transform path** | 2D pose for sprite/quad games (the demo's `Position{x,y}`) | Either `Transform` used as a 2D subset (z=0) or a `Transform2D` variant | 🟡 demo-local `Position`/`Velocity` only | OPTIONAL (see fork: 2D vs 3D) |
| **`GlobalTransform` helper** | On-demand single-entity world pose (avoid 1-frame lag) | A `SystemParam`/fn walking ancestors for one entity | ❌ | OPTIONAL |

---

## (B) Camera

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **`Camera`** | Marks an entity as a view source; carries render-order + active flag | `#[derive(Component)]` `{ order: i32, is_active: bool, viewport: Option<Viewport> }`; pose comes from its `GlobalTransform` | ❌ (two **disjoint hardcoded** view sources outside ECS: demo `CameraUniform` `camera.rs:19`; marcher `CompositePushConstants` `compute.rs:994`) | **CORE** |
| **`Projection`** | Perspective vs orthographic params | `#[derive(Component)]` enum `Perspective{fov,aspect,near,far}` / `Orthographic{size,near,far}` | ❌ | **CORE** |
| **Active-camera resolution** | Which camera the renderer uses (no implicit "first wins" search) | Explicit: `ActiveCamera` resource (entity handle) or `is_active` field + a tiny resolver system writing a `ViewUniform` resource (view/proj matrix) | ❌ | **CORE** |
| **`ViewUniform` / view-proj derivation** | The matrix the renderer consumes | A resource derived each frame from active `Camera`+`Projection`+`GlobalTransform` (replaces hand-fed per-backend params) | 🟡 raw matrices exist per-backend, not derived from an ECS camera | **IMPORTANT** |
| **`Viewport` / render target** | Sub-rect / target for split-screen, PiP, offscreen | `Viewport{rect}` field/component + per-camera `RenderTarget` handle; multiple active cameras sorted by `order` | ❌ | OPTIONAL (see fork: multi-viewport) |
| **Camera controller** | Input-driven camera movement (orbit/fly/follow) | A system reading `boyko_input` `ActionState` → mutates camera `Transform` | 🟡 input is mature (`ActionState`), **no camera to drive** | OPTIONAL |
| **View-target blend** | Cinematic blend between cameras | A system lerping the `ViewUniform` between two camera entities | ❌ | OPTIONAL |

---

## (C) Rendering capability — the "this entity is drawn" component set

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **Renderable / mesh handle** | Binds an entity to drawable geometry | `#[derive(Component)]` `MeshHandle`/`Renderable` (id into a mesh/asset table) — opt-in, so non-spatial entities pay nothing | 🟡 **no general one**; three unrelated reps: demo 2D `GpuInstance` (24 B, `instance.rs:45`), `DeviceColumnHandle` (GPU column storage, not a per-entity mesh), SDF `SdfEdit` stream | **IMPORTANT** |
| **Material handle** | Per-entity material binding | `#[derive(Component)]` `MaterialHandle(MaterialId)` bridging to `MaterialGpu` (`material.rs:31`, `MaterialId(u16)`) | 🟡 `MaterialGpu`/`MaterialId` exist; **no per-entity component links a row to a material** | **IMPORTANT** |
| **GPU-instance bridge** | Zero-copy upload of renderable rows to the GPU | Reuse `GpuInstance` + `for_each_chunk` SoA→GPU path; generalize beyond 2D quads | ✅ mechanism exists (demo); 🟡 2D-only shape | **IMPORTANT** |
| **`Visibility`** | User intent: show/hide this entity | `#[derive(Component)]` enum `{ Inherited, Hidden, Visible }` (user-writable) | ❌ (**no `Visible`/`Visibility` anywhere**) | **IMPORTANT** |
| **`InheritedVisibility`** | Hierarchy-propagated visibility (strict-AND up the tree) | Engine-written `#[derive(Component)]` bool, propagated like `GlobalTransform` | ❌ | OPTIONAL |
| **`ViewVisibility` / culling** | Final per-view "drawn this frame" result after frustum/layer cull | Engine-written bool, computed by a parallel `check_visibility` pass | ❌ (L1 froxel cull exists GPU-side, not as an ECS visibility component) | OPTIONAL |
| **`RenderLayers` / cull mask** | Branch-free render filtering (layers ∩ camera cull mask) | `#[derive(Component)]` `u32` on renderables + `u32` on cameras; bitwise-AND filter | ❌ | OPTIONAL |
| **Lights** | Light source components | `DirectionalLight`/`PointLight`/`SpotLight`/`SkyLight` PODs | ✅ done (`boyko_render/light.rs:213/226/241/266`), folded to GPU table by `collect_lights` (Principle-0 clean). **Caveat:** positions are self-contained `[f32;3]`, **not derived from `Transform`** | (reuse / reconcile) |

---

## (D) Physics capability — the "this entity simulates / collides" component set

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **`RigidBody`** | Dynamics state (pos/vel/rot/angvel) | `#[derive(Component)]` HOT POD | ✅ (`components.rs:29`) | (reuse) |
| **`RigidBodyMass`** | Mass / inertia / material (cold) | `#[derive(Component)]` COLD POD; `BodyType{Static,Kinematic,Dynamic}` | ✅ (`:71`, body modes already explicit) | (reuse) |
| **`Collider`** | Shape + layer/mask | `#[derive(Component)]` `{ shape, layer, mask }` (Sphere/Box) | ✅ (`:148`) | (reuse) |
| **`Contact`** | Gameplay-facing collision snapshot | `#[derive(Component)]` `{ manifold, other: EntityId }` | ✅ (`:166`) | (reuse) |
| **Body-mode markers** | Express static/dynamic/kinematic as query-partitionable | Distinct markers / archetypes vs the `body_type` enum field (Unreal `ECollisionEnabled` taxonomy) — a refinement, not a rebuild | 🟡 currently a `BodyType` enum field on `RigidBodyMass` | OPTIONAL |
| **`Trigger` / sensor** | Overlap-only volume, no contact resolution (Unity/Unreal/Godot all ship one) | `Collider` + a `Sensor` marker (or `Collider.mode = QueryOnly`) | ❌ (no sensor flag) | **IMPORTANT** |
| **Math reconciliation** | Physics `Vec3/Quat/Mat3` must become the shared math | Lift `boyko_physics/math.rs` → shared crate; physics + `Transform` + lights speak one math vocabulary | 🟡 physics has its **own private** math (ties to A) | **CORE** (blocks A) |

---

## (E) OBJECT CATEGORIES — composable bundles/presets (the core owner ask)

The owner's "ready-made object that can/cannot use physics/rendering" = **which components the bundle contains**. No inheritance, no God-object: capability is **set membership**, and "cannot use X" simply means "the X-component is absent, so the X-system skips it." Shipped as named `#[derive(Bundle)]` presets (warm path hits the static bundle cache, Phase 8.5) — and, longer-term, as a Required-Components closure (Bevy's `#[require]`, the modern replacement for bundles).

| Preset | Composition (the exact "can/cannot") | Status of parts | Pri |
|---|---|---|---|
| **`SpatialBundle`** | `Transform` + `GlobalTransform` (+ `Visibility`) — the base every placed object shares | ❌ (needs A) | **CORE** |
| **`StaticProp`** | `Transform` + `GlobalTransform` + `MeshHandle` + `MaterialHandle` — **rendered, NO physics** (physics systems never see it: no `RigidBody`/`Collider`) | 🟡 render parts partial | **IMPORTANT** |
| **`StaticCollider`** | `Transform` + `Collider` + `RigidBodyMass{Static}` — **collides, NOT rendered** (no mesh) | 🟡 collider ✅, transform ❌ | **IMPORTANT** |
| **`DynamicBody`** | `Transform` + `GlobalTransform` + `MeshHandle` + `MaterialHandle` + `RigidBody` + `RigidBodyMass{Dynamic}` + `Collider` — **rendered AND simulated** (the full "physical visible object") | 🟡 physics ✅, render+transform partial | **IMPORTANT** |
| **`Trigger`** | `Transform` + `Collider` + `Sensor` — **detects overlaps, no resolution, not rendered** | 🟡 needs `Sensor` (D) | **IMPORTANT** |
| **`KinematicBody`** | `Transform` + `RigidBody` + `RigidBodyMass{Kinematic}` + `Collider` (+ optional mesh) — script-moved, pushes dynamics but isn't pushed | ✅ parts exist | OPTIONAL |
| **`CameraRig`** | `Transform` + `GlobalTransform` + `Camera` + `Projection` (+ `ActiveCamera`) — a placeable, parentable view | ❌ (needs B) | **IMPORTANT** |
| **`LightObject`** | `Transform` + `GlobalTransform` + one of `Directional/Point/SpotLight` — light reads pose from `GlobalTransform` (fixes the self-contained-position caveat in C) | 🟡 lights ✅, transform link ❌ | **IMPORTANT** |
| **`SceneRoot`** | `Transform` + `GlobalTransform` + `Children` — a transform-only grouping/pivot node (empty archetype + transform) | 🟡 hierarchy ✅, transform ❌ | OPTIONAL |
| **`RigidBodyBundle`** | existing physics bundle (body+mass+collider) | ✅ (`components.rs:182`) | (reuse / extend with transform) |

**This table IS the answer to the owner's question:** "an object that can be placed in physics and can/cannot use physics/rendering" is fully expressed by which of `{Transform, MeshHandle+MaterialHandle, RigidBody+RigidBodyMass+Collider, Sensor, Camera, Light}` the bundle carries. Each capability is independent and additive.

---

## (F) Identity / Visibility / Lifecycle

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **`Name`** | Human-readable label (debug, editor, lookup) | `#[derive(Component)]` interned-string newtype (NOT a hot field; cold) | ❌ (**missing entirely**) | **IMPORTANT** |
| **Marker tags** | Zero-data type identity (`PlayerTag`, `Enemy`) | ZST `#[derive(Component)]`, signature/table backend | ✅ static + dynamic (Phase 22) | (reuse) |
| **Enable-bit tags** | High-churn O(1) toggle, no migration (`Visible`, `Stunned`) | `#[component(storage="bitset")]` ZST; `Enabled<T>`/`Disabled<T>` filters | ✅ `EnableTag` (Phase 22) | (reuse — strong candidate backend for `Hidden`/`Disabled`) |
| **Visible/Hidden** | Per-entity show/hide toggle | EITHER `Visibility` enum component (C) OR an `EnableTag` bit — pick by toggle frequency | ❌ as a named std type | **IMPORTANT** |
| **Hierarchy** | Parent/child relation (DOTS `Parent`/`Child`, Unreal attachment, Godot tree) | `ChildOf(Entity)` source-of-truth + reactive `Children` | ✅ (Phase 19, `hierarchy/mod.rs`) | (reuse — transform propagation rides on top) |
| **Spawn (direct / deferred)** | Create entities | `EcsMaster::create_entity` / `Commands::spawn(bundle)` | ✅ | (reuse) |
| **Despawn + recursive** | Destroy entity + descendants (Godot `queue_free`, Unreal `Destroy`) | `delete_entity` / `Commands::despawn`; `Children` hook cascades | ✅ (Phase 19) | (reuse) |
| **Lifecycle hooks/observers** | React to add/insert/replace/remove (Unreal BeginPlay/EndPlay, Unity OnEnable) | `on_add`/`on_remove` hooks + runtime observers; no virtual dispatch | ✅ (Phase 14a/14b) | (reuse) |
| **Enable/disable cascade** | Hierarchy-propagated active state (Unity `activeInHierarchy`) | A propagation pass (like visibility) or flattened effective-bit | ❌ | OPTIONAL |

---

## (G) Gameplay scaffolding (optional — scope decision)

| Item | What it is | ECS-native shape | Status | Pri |
|---|---|---|---|---|
| **Time / fixed timestep** | Game clock + fixed loop + interpolation alpha | `Res<Time>`, `Res<FixedTime>`, `CoreSchedule::Fixed` | ✅ (Phase 20) | (reuse) |
| **Input → actions** | Buttons/axes for controllers | `ActionState<A>` resource (SoA, bitset-hot) | ✅ (`boyko_input`) | (reuse) |
| **States / App / Plugins** | Game states + frame loop + modular setup | `States`, `App`, `Plugin`, `add_plugins` | ✅ (Phases 17/18) | (reuse) |
| **Controller/Pawn split** | Decouple "brain" from "body" (Unreal possession) | A `Controls`/`PossessedBy` **relation** component (reuses hierarchy-style machinery) | ❌ (relations parked) | OPTIONAL |
| **GameMode / GameState / session** | Singletons (Unreal) | `Resource`s | ✅ (Resources exist) | (reuse) |
| **Prefab / instantiate** | Reusable templates (Unity prefab, Godot PackedScene, flecs `IsA`) | Template-blit into a fresh archetype row (boyko static-bundle-cache style) + `ChildOf` remap — NOT live IsA indirection | 🟡 serialization plan covers POB column-blit + `ChildOf` remap (`docs/SERIALIZATION-PLAN.md`) | OPTIONAL (see fork) |
| **Scene assembly** | Spawn-a-whole-tree ergonomics (replaces demo's hand-wired `SpawnIds` + raw `create_entity` byte-packing) | A spawn API over bundles + hierarchy | 🟡 demo does it manually (`modes.rs`) | OPTIONAL |

## Biggest gaps (ordered)
- #1 SHARED MATH LIBRARY — the root dependency of everything. Vec3/Quat/Mat3 exist but are PRIVATE to boyko_physics; Vec2/Vec4/Mat4/affine do not exist anywhere. No Transform, Camera, or GlobalTransform can ship until one shared, SIMD-aligned math vocabulary exists (lift + extend physics's math.rs).
- #2 TRANSFORM + GLOBALTRANSFORM COMPONENTS — there is NO engine-level position/rotation/scale component at all. Spatial data is ad-hoc per consumer (physics has its own; lights carry self-contained positions; the demo has a bespoke 2D Position{x,y}). This is the single most foundational missing GAMEPLAY piece — the owner's 'object that can be placed' has nowhere to store its placement.
- #3 TRANSFORM PROPAGATION SYSTEM — the ChildOf/Children hierarchy carries NO transform semantics (only link/unlink + recursive despawn). World-from-local composition over the tree is missing; without it parenting is structural-only and render/physics can't read world pose.
- #4 CAMERA (component + active-camera + view/proj derivation) — no Camera component, no active-camera concept, no view/projection in the ECS. Two disjoint hardcoded view sources live OUTSIDE the ECS (demo CameraUniform, marcher CompositePushConstants), hand-fed per backend. A game cannot frame a scene from an ECS entity.
- #5 GENERAL RENDERABLE COMPONENT SET (mesh handle + material handle + Visibility) — no engine-level 'this entity is drawn' component. Three unrelated representations exist (demo 2D GpuInstance, GPU-resident DeviceColumnHandle storage, SDF SdfEdit stream); none is a per-entity mesh/material binding. Renderable bundles (StaticProp/DynamicBody) cannot be assembled until this exists.
- #6 OBJECT-CATEGORY BUNDLES (the owner's literal ask) — StaticProp/DynamicBody/Trigger/CameraRig/LightObject/SpatialBundle. Blocked on #1–#5; the bundle MECHANISM (Bundle derive + static cache) is ready, so this is assembly-once-the-parts-exist. A Trigger/Sensor flag in physics is the one missing PHYSICS part for this.
- #7 Name + Visibility/Hidden marker components — trivially missing leaf types (no Name, no Visible/Visibility anywhere). Cheap to add; Visibility can ride the existing EnableTag bitset for O(1) toggling.

## Already in boyko
REUSE — DO NOT REBUILD. boyko already has a mature ECS substrate; the standard-library work is almost entirely the SPATIAL/CAMERA/RENDERING-capability layer (groups A/B/C) plus two trivial leaf components (Name, Visibility). Concretely already shipped and to be reused as-is:

- PHYSICS COMPONENTS (group D, mature): RigidBody, RigidBodyMass (with explicit BodyType{Static,Kinematic,Dynamic}), Collider (shape+layer/mask), Contact, RigidBodyBundle — all real #[derive(Component)] columns with hot/cold split (crates/boyko_physics/src/components.rs:29/71/148/166/182). Dense race-free solver state in resources.rs. Only gaps: a Sensor/Trigger flag and wiring pose to a shared Transform.
- LIGHTING COMPONENTS (group C, mature): DirectionalLight/PointLight/SpotLight/SkyLight PODs folded to a GPU table by collect_lights (crates/boyko_render/src/light.rs:213/226/241/266) — Principle-0 clean. Caveat to reconcile: positions are self-contained [f32;3], not yet read from a Transform.
- HIERARCHY (Phase 19): ChildOf (source-of-truth FK) + reactive Children, hook-maintained, recursive despawn cascade (crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs:86/119). Transform propagation rides ON TOP of this — do not build a new parent/child mechanism.
- TAGS + ENABLE-BIT (Phase 22): static ZST tags, dynamic name-keyed tags, and the EnableTag paged-bitset backend with O(1) toggle + Enabled<T>/Disabled<T> filters. Visible/Hidden/Disabled should reuse EnableTag, not invent a new toggle.
- BUNDLES + STATIC BUNDLE CACHE (Phase 8.5): named #[derive(Bundle)] + sub-ns warm archetype cache + MAX_BUNDLE_ARITY=16. Object-category presets (group E) are ASSEMBLED with this existing mechanism — the menu is new, the machinery is not. Prefab/instantiate later reuses the same blit + ChildOf-remap.
- LIFECYCLE HOOKS/OBSERVERS (Phase 14a/14b): on_add/insert/replace/remove hooks + runtime observers, no virtual dispatch — covers Unity OnEnable / Unreal BeginPlay-EndPlay without any new system.
- SCHEDULER + ORDERING (Phase 9/15/16): conflict-graph parallel executor, before/after/in_set, run_if, par_iter/for_each_chunk SoA→GPU. Transform propagation and visibility passes slot in as ordered stages here; the 1-frame-lag is solved by explicit ordering primitives that already exist.
- TIME / INPUT / STATES / APP / RESOURCES (Phases 20/17/18 + boyko_input): Res<Time>/Res<FixedTime> + CoreSchedule::Fixed, ActionState<A> (mature, ready to drive a camera controller), States/NextState, App/Plugin/add_plugins, Resources (= GameMode/GameState singletons). All of group G's 'existing' rows.
- MATH SEED (partial): Vec3/Quat/Mat3 with full ops exist but PRIVATE to boyko_physics (crates/boyko_physics/src/math.rs:29/213/378). These are the seed to LIFT into a shared crate (not rewrite) and extend with the missing Vec2/Vec4/Mat4/affine.

Net: groups D, F-hierarchy/tags/lifecycle/spawn-despawn, and G-existing are DONE. The build is concentrated in A (spatial), B (camera), C (general renderable + Visibility), the E presets that compose them, and the Name/Visibility leaves.

## Scope forks (for owner)

### Fork 1: Math library: lift boyko_physics's in-house Vec3/Quat/Mat3 to a shared crate and extend it, or take a dependency (e.g. glam)?
- Why: Everything in groups A, B, C-lights, and D speaks math. There are currently TWO problems at once: (1) the types are private to boyko_physics, and (2) Vec2/Vec4/Mat4/affine don't exist anywhere. A Transform/GlobalTransform/Camera cannot ship until one shared math vocabulary exists. This is the literal root dependency of the whole standard library.
- Options: In-house: lift physics math.rs into a new boyko_math crate, add the missing types (Vec2/Vec4/Mat4/Affine3A), keep #[repr(C)]+SIMD-aligned POD, full ops | Dependency: add glam (de-facto Rust gamedev math, used by Bevy), rewrite physics to use it | Hybrid: in-house POD storage types with a thin optional glam interop behind a feature
- Recommendation: In-house (boyko_math), lifting and extending physics's math.rs. Reasoning: the engine is in-house-first and perf-first; physics ALREADY proved an in-house Vec3/Quat/Mat3 that is bit-exact and SIMD-tuned (the O9 x8 SDF kernel, signed-zero ties). A glam dependency would force either a rewrite of the proven physics kernels or a lossy conversion layer at the physics boundary, and it cedes control of layout/alignment/SIMD exactly where boyko's whole thesis is layout control. The missing types (Vec2/4, Mat4, affine) are mechanical to add. This is the one fork where 'no one ships their own math' is irrelevant — boyko already ships its own, successfully.

### Fork 2: Transform representation: separate local Transform + cached GlobalTransform (Bevy/DOTS), or a single world-space transform computed on demand?
- Why: Determines storage cost (one component vs two), the existence of a propagation system, and the intra-frame staleness contract. It also dictates how render/GPU-upload and physics read pose: a cached affine GlobalTransform is one contiguous SIMD-friendly read; on-demand world means a hierarchy walk per consumer.
- Options: Two components: Transform (local, decomposed T/R/S) + GlobalTransform (cached affine), reconciled by a late propagation system — Bevy/Unity-DOTS/Godot-lazy model | Single Transform interpreted as world-space, no hierarchy composition (flat scenes only) | Single local Transform + compute world on demand via a helper (no cached column)
- Recommendation: Two components (Transform + GlobalTransform) with an eager dirty-bitset propagation system in a late stage. Reasoning: render, GPU instance upload, lighting, and physics all want a ready-to-consume world matrix as ONE sequential read — caching it as a packed affine is the cache-optimal choice and matches the existing zero-AoS-copy for_each_chunk SoA→GPU path. boyko's batched scheduler (Phase 9/15 ordering) makes an eager end-of-frame propagation pass natural, unlike Godot's lazy-on-read (which fits a per-node engine, not a batched one). The known 1-frame-lag footgun is resolved by EXPLICIT stage ordering (propagation before render/GPU upload, after physics writeback), not luck — boyko already has the ordering primitives. Gate propagation on change detection (Phase 10) so static subtrees cost nothing. Store GlobalTransform as affine (not re-decomposed T/R/S) to represent non-uniform-scale shear correctly.

### Fork 3: Object categories: ship as named #[derive(Bundle)] presets only, or also build a Required-Components closure (Bevy #[require]) as the idiomatic path?
- Why: This is the mechanism behind the owner's central ask ('ready-made object that can/cannot use physics/rendering'). Bundles already exist and hit the static bundle cache, but Bevy explicitly RETIRED bundles as the primary preset mechanism because the author must list the full dependency closure by hand (error-prone, no dependency inheritance). Required Components auto-insert the dependency closure and are archetype-cached (effectively free).
- Options: Bundles only: ship StaticProp/DynamicBody/Trigger/CameraRig/LightObject as named #[derive(Bundle)] presets now (lowest effort, parts already exist) | Required Components: build a #[require(...)]-style closure in boyko_macros (e.g. Mesh requires Transform; PointLight requires Transform), then presets compose naturally | Both, phased: bundles now for the category menu, Required-Components later as the idiomatic dependency layer
- Recommendation: Both, phased — bundles NOW, Required-Components soon. Reasoning: the owner wants a category menu for alignment today, and bundles + the static bundle cache (Phase 8.5) already deliver that with zero new machinery — ship StaticProp/DynamicBody/Trigger/CameraRig/LightObject immediately. But Required Components is the single highest-value modern ECS pattern (the unanimous Bevy conclusion) and is exactly boyko's 0%-gate, archetype-cached style; the proc-macro infra exists in boyko_macros. The killer feature it adds: 'Mesh requires Transform', 'PointLight requires Transform' enforced automatically, so a renderable/light can never exist without a pose — which directly fixes the lights-have-self-contained-positions caveat. Do NOT make bundles the permanent primary abstraction; make them the convenience layer over the require-closure (Bevy's exact migration). Enforce one lint Bevy learned the hard way: a required component's Default must be valid before its producer runs (the GlobalTransform-identity-until-propagated bug class).

### Fork 4: Dimensionality: 2D + 3D both first-class, or 3D-only with 2D as a z=0 subset?
- Why: The demo (the only live consumer) is 2D quads with a bespoke Position{x,y}; the real renderer (boyko_rhi_vulkan SDF marcher) is 3D. Camera, Transform, projection, and culling all differ in shape between 2D and 3D. Shipping two parallel type families (Transform2D/Transform3D, Camera2D/Camera3D) doubles the surface; a unified 3D type used for 2D adds a wasted axis but one vocabulary.
- Options: 3D-first, 2D as subset: one Transform/Camera/Projection (3D); 2D games set z=0 and use Orthographic projection | Dual first-class: Transform2D/Transform3D + Camera2D/Camera3D families (Godot/Bevy approach) | 2D-first then 3D later (matches what the demo exercises today)
- Recommendation: 3D-first, with 2D expressed as the z=0 / orthographic subset of the same types. Reasoning: the engine's REAL renderer and the mature physics kernels are 3D (Vec3/Quat/Mat3, SDF marcher); building 2D-first would mean re-deriving everything for 3D and maintaining two families. A single 3D Transform + Orthographic projection covers the demo's 2D quads (the survey confirms Unity/Unreal do exactly this — 2D is orthographic 3D), at the cost of one unused axis per 2D entity (negligible: 4 bytes, and the hot GPU path already packs its own 2D GpuInstance separately). Defer a dedicated Transform2D unless a 2D-heavy title proves the wasted axis matters — that is a measurable, later decision, not a now decision.

### Fork 5: Gameplay scaffolding scope: how much of group (G) is in v1 vs deferred — specifically Controller/Pawn relations, prefab/instantiate, and scene assembly?
- Why: These are the line between 'a scene/object kit' and 'a gameplay framework'. They depend on machinery that is parked (relations) or plan-only (serialization/prefab). Pulling them into v1 risks scope-creeping the foundational spatial/camera/render work that everything else needs first. The owner's standing rule: foundations before APIs.
- Options: Foundations-only v1: ship A+B+C+D+E+F (spatial, camera, render capability, physics markers, object bundles, Name/Visibility); defer ALL of G except what already exists (Time/Input/States/App) | Foundations + prefab: add template-blit instantiate (serialization plan is already written) so users can save/spawn whole object trees | Full framework: also add Controller/Pawn possession relations and a scene-assembly API
- Recommendation: Foundations-only v1. Reasoning: this is the owner's 'foundations before APIs' rule applied directly — Transform/Camera/Renderable are the slow inner loop everything sits on, and they don't exist yet. Controller/Pawn needs the relations subsystem (explicitly PARKED in memory) and a wrong relation design would be expensive to unwind; prefab/instantiate is plan-only (serialization) and is a natural SECOND wave once bundles + transform propagation are proven (template-blit reuses the static-bundle-cache + ChildOf-remap that v1 establishes). Scene assembly is sugar over v1's bundles+hierarchy and should be written against a stable foundation, not concurrently. Keep G to what's already shipped (Time, Input, States, App, Resources-as-GameMode). Promote prefab/instantiate to an explicit Wave 2 so the owner sees it's sequenced, not dropped.
