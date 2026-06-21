All critical claims are now verified:
- `#[require(...)]` is already shipped (macro parses it, full test suite exists) — S8 confirmed mis-scoped.
- Lifecycle hooks exist (on_add/on_insert at structural sites) — usable for the dynamic-body-root guard.
- `for_each_chunk` yields no entity ids; pure-dense `(EntityId, &T)` iterator exists but only single-column read; mixed entity-yielding iteration does not exist publicly.
- Mat3 is row-major; Mat4 proposed column-major — real bridge hazard.
- `boyko_serialize` is whole-world only — S7 materially under-scoped.

I have everything needed to revise. Producing the full plan.

---

# boyko-engine Standard Library — Phased Implementation Plan (v2)

Owner-locked scope (D1–D5) is the design contract; nothing below re-litigates it. Spine kept (S1→S8). **S5 (physics⇄Transform sync + Sensor) is reordered to run right after S2** and is documented as depending only on S2 (pose sync needs `Transform`+`GlobalTransform`, not the renderer S3/S4). No other reorder.

**A new dependency phase S0 is inserted before S2** to close the critic's #1 blocking gap: per-row entity identity is not exposed as public `QueryData`/iteration in this kernel, yet propagation, body↔Transform sync, and the cold single-entity walk all require it. S0 promotes entity-yielding iteration to a first-class kernel feature (Principle 0) rather than deferring it. S0 is small and self-contained but is a real prerequisite, not bundled into S2.

## Crate graph (verified acyclic)

`boyko_render`/`boyko_rhi_vulkan` carry pose as raw `[f32;3]` and do not depend on physics' math; only `boyko_physics` uses `boyko_physics::math`. `boyko_macros` must NOT depend on `boyko_ecs` (the cycle the plan respects). `boyko_math` becomes a leaf root; a new `boyko_scene` crate holds `Transform`/`GlobalTransform`/`Camera`/render-caps so `boyko_ecs` stays a pure kernel.

```
boyko_ecs (kernel, no math)         boyko_math (leaf: Vec2/3/4,Quat,Mat3/4,Affine3A)
      ▲           ▲                        ▲      ▲        ▲
      │           └──────────┐             │      │        │
boyko_scene (Transform/GlobalTransform/Camera/render-caps/bundles/Name/Visibility)
      ▲           ▲                 ▲
      │           │                 │
boyko_physics  boyko_render   boyko_demo / consumers
(scene+math)      (scene+math)
```

## Schedule placement contract (resolves the multi-schedule gap — applies to S2/S3/S4/S5)

The engine runs a **fixed schedule** (physics: `physics_gather/integrate/solve/apply` read `FixedTime`, run 0..N times per frame) and a **per-frame schedule** (render/GPU upload/lights). Phase 20.1 already does GPU-side `mix(prev, pos, alpha)` interpolation. The full pose chain spans BOTH schedules, so `.after()`/`.before()` (which order within ONE schedule) is insufficient on its own; cross-schedule ordering is enforced by **which schedule each system is registered in** plus intra-schedule edges.

Canonical placement (the no-desync proof rests on this table):

| System | Schedule | Intra-schedule order |
|---|---|---|
| `sync_transform_to_body` (Static/Kinematic in) | **Fixed**, first | before `physics_gather` |
| `physics_gather/integrate/solve/apply` | **Fixed** | existing pipeline order |
| `sync_body_to_transform` (Dynamic out) | **Fixed**, last | after `physics_apply` |
| `propagate_transforms` | **Per-frame** | after the fixed schedule has fully advanced this frame; `.before(resolve_active_camera)` |
| `resolve_active_camera` → `ViewUniform` | **Per-frame** | `.after(propagate_transforms)` |
| `light_reconcile` → light components | **Per-frame** | `.after(propagate_transforms)`, `.before(collect_lights)` |
| `sync_gpu_instances` | **Per-frame** | `.after(propagate_transforms)` |
| `collect_lights` (unchanged) | **Per-frame** | after `light_reconcile` |

**Interpolation reconciliation (Phase 20.1).** `Transform`/`GlobalTransform` hold the **latest substep** pose (the fixed schedule writes `Transform` on its last sub-step via `sync_body_to_transform`; the very next per-frame `propagate_transforms` composes it). GPU-side `mix(prev, pos, alpha)` interpolation is owned entirely by the existing demo `GpuInstance` 2D path and stays untouched — `propagate_transforms` does NOT participate in prev/pos double-buffering. The 3D record (S4) ships WITHOUT GPU interpolation in v1 (stated explicitly in S4), so there is no second prev-shuffle writer to reconcile. This is the deliberate v1 cut; a 3D-interpolation phase is future work.

Because the fixed schedule fully completes before per-frame propagation each frame, every pose datum has exactly one writer per schedule window: `sync_body_to_transform` (fixed) is the sole `Transform` writer for Dynamic roots; gameplay/`sync_transform_to_body` is the sole `RigidBody` writer for Static/Kinematic; `propagate_transforms` (per-frame) is the sole `GlobalTransform` writer. No two systems write the same pose concurrently.

---

## S0 — Kernel feature: entity-yielding iteration (`iter_entities` / chunk entity-base) — PREREQUISITE

**GOAL.** Expose per-row entity identity through the public Query API so propagation, body↔Transform sync, and the cold ancestor walk can be written at all. Today `entity_ids: Vec<EntityId>` is a private, row-indexed `Archetype` field with only an internal fetch-scratch `entity_ids` pointer and a pure-dense `(EntityId, &T)` iterator; mixed `(Transform, GlobalTransform)` iteration cannot yield the entity. This is a first-class kernel capability (Principle 0: a capability a subsystem needs becomes a kernel feature, not a per-crate adapter), not a `boyko_scene` workaround.

**Crate.** `boyko_ecs` only. Files: `iters/query/query.rs` (new public methods), `iters/query/iter.rs` / `chunk_iter.rs` (entity-base threading — the base pointer already exists in the driver).

**API added.** Two minimal, monomorphization-direct additions (no `dyn`, no boxing, no extra archetype walk — they thread the *already-cached* `entity_ids` base that the fetch path computes via `arch_ref.entity_ids_slice().as_ptr()`):

```rust
impl<'s, D: QueryData, F: QueryFilter> Query<'s, D, F> {
    /// Per-row iterator yielding (EntityId, D::Item) — the entity base is the
    /// archetype's already-cached entity_ids column; zero extra archetype walk.
    pub fn iter_entities(&self) -> QueryIterEntities<'_, 's, D, F> where D: ReadOnlyQueryData;
    pub fn iter_entities_mut(&mut self) -> QueryIterEntitiesMut<'_, 's, D, F>;

    /// Chunk variant: f(entity_slice: &[EntityId], chunk: D::ChunkItem).
    /// entity_slice base = arch.entity_ids_slice(); same archetype loop as for_each_chunk.
    pub fn for_each_chunk_entities<Func>(&mut self, f: Func)
        where D: ChunkedQueryData, F: ArchetypalQueryFilter,
              Func: for<'c> FnMut(&'c [EntityId], D::ChunkItem<'c>);
}
```

**Why this shape.** The per-archetype driver already loads `arch_ref.entity_ids_slice().as_ptr()` into the fetch (verified: `data.rs:559/596/874/913`). Yielding it costs one extra slice/pointer per archetype, not per row — the hot loop is unchanged (same stride, same `D::Item` gather). `for_each_chunk_entities` is the propagation/sync workhorse: the entity-id slice and the component chunk are parallel arrays over the same row range, so a `for i in 0..len { let e = ents[i]; … }` loop is SoA-sequential on both.

**Performance.** Per-row variant: identical inner loop to `iter`/`iter_mut` + one `*ents.add(row)` load (a sequential read of a `usize` column already in L1 from the prior fetch). Chunk variant: identical to `for_each_chunk` + one base-pointer capture per archetype. **0%-gate:** `iter`/`iter_mut`/`for_each_chunk` (the non-entity variants) are byte-identical — the new methods are additive, the existing ones do not route through them.

**GATES.**
- Unit: `iter_entities` yields exactly the live entities of each matched archetype in slot order; the yielded `D::Item` matches `iter` for the same row.
- Property: for a random world, `{ (e, ptr_of(item)) }` from `iter_entities` == the set from `iter` joined with `entity_ids_slice`.
- 0%-gate: `iter`/`for_each_chunk` benches within noise of pre-S0 baseline (additive methods, no shared codepath regression).
- Miri-TB: the entity-base raw read in `iter_entities_mut` (shared `entity_ids` slice read alongside `&mut` component access — disjoint columns, must be argued; see SAFETY below).

**SAFETY invariant (documented in code).** The `entity_ids` column and the component column are **distinct `ComponentPool`/archetype allocations**; reading `entity_ids[row]` (shared) while holding `&mut component[row]` is a read of a disjoint allocation, not an alias. Stated as a `// SAFETY:` block; Miri-TB is the oracle.

**Dependencies.** None (pure kernel). Strictly precedes S2/S5.

---

## S1 — `boyko_math` (lift + extend; PRESERVE physics bit-determinism)

**GOAL.** The single SIMD-aligned POD math vocabulary, with **byte-identical physics behavior** after migration.

**New crate.** `crates/boyko_math/` — `lib.rs`, `vec.rs`, `quat.rs`, `mat.rs`, `affine.rs`.

**Changed crates.** Workspace `Cargo.toml` members += `crates/boyko_math`. `boyko_physics/Cargo.toml` += dep.

**Call-site migration folded into S1 (resolves the open-ended-shim finding).** The ~30 physics call-sites (`crate::math::{Mat3,Quat,Vec3}`) are migrated to `boyko_math::` **within S1**, with the byte-identical physics suite as the safety net. The math unit tests move to `boyko_math` (single home — no duplication). A **transitional 1-line re-export** `pub use boyko_math::{Vec2,Vec3,Vec4,Quat,Mat3,Mat4,Affine3A};` is left in `boyko_physics/src/math.rs` ONLY for the doc-link paths and is removed in the same phase once `cargo check` is green with direct imports — it is a within-phase mechanical step, not a deferred cleanup item. The phase is not "done" until `boyko_physics/src/math.rs` is either deleted or contains only the determinism-anchoring doc comments with no type definitions.

**Lift discipline (bit-determinism — CRITICAL).** `Vec3`/`Quat`/`Mat3` source is moved **verbatim, character-for-character** (same field order, same `#[repr(C)]`, same operation order). Confirmed load-bearing facts:
- `Vec3::normalize`/`Quat::normalize` use `len_sq.sqrt().recip()` (verified `math.rs:105`) = exact `sqrt` then reciprocal, NOT a hardware `rsqrt`. MUST stay literally `len_sq.sqrt().recip()`.
- **Forbidden crate-wide:** `f32::mul_add`/FMA intrinsics, `*_rsqrt`/`*_rcp` approximations, `to_intrinsic` fast paths, `#[target_feature(enable="fma")]` on any math fn, `fast-math`/`float_algebraic` on any type.
- `Quat::integrate` order (build ω̂ → one Hamilton product → scale → add → normalize, verified) and signed-zero tie behavior of `clamp_symmetric`/`abs` preserved (O9 SDF + signed-zero ties depend on them).

**FMA-suppression mechanism (resolves "no FMA is not automatic").** "No FMA" is enforced by a concrete mechanism, not by source convention alone:
1. No `-Cllvm-args=-fp-contract=fast` and no `fast-math` feature anywhere in the workspace profile (default LLVM `fp-contract` is `on` only for explicit `fma` source calls, which are forbidden — separate-statement `a*b` then `+c` is NOT contracted under default Rust codegen). 
2. No `#[target_feature(enable="fma")]` on any `boyko_math` fn (FMA contraction requires the feature to be enabled on the function).
3. Nightly paths: `float_algebraic`/fast-math intrinsics are NOT used in `boyko_math`.
4. **One-time asm gate** (recorded in RESULTS): disassembly of `Vec3::normalize`, `Quat::integrate`, AND the new `Mat4::perspective_rh`/`Affine3A::inverse` contains `sqrtss` + div/`mulss` and **no** `vfmadd*`/`rsqrtss`. A future toolchain change that introduces contraction is caught here.

**New types (signatures + conventions).**
```rust
#[repr(C)] #[derive(Clone,Copy,Debug,Default,PartialEq)]
pub struct Vec2 { pub x: f32, pub y: f32 }

// SIMD lane: one xmm; GPU/std140 vec4 lane.
#[repr(C, align(16))] #[derive(Clone,Copy,Debug,Default,PartialEq)]
pub struct Vec4 { pub x: f32, pub y: f32, pub z: f32, pub w: f32 }

// COLUMN-major 4x4 (WGSL mat4x4 convention): cols[i] is column i.
// Matches the demo CameraUniform.view_proj (column-major) → direct upload.
#[repr(C, align(16))] #[derive(Clone,Copy,Debug,PartialEq)]
pub struct Mat4 { pub cols: [Vec4; 4] }

// Packed affine: linear 3x3 (carries non-uniform-scale SHEAR) + translation.
// matrix3 reuses the lifted ROW-MAJOR Mat3 (rows[i] is row i) — see the
// convention contract below. Vec4-padded layout chosen for size/SIMD (see S2).
#[repr(C, align(16))] #[derive(Clone,Copy,Debug,PartialEq)]
pub struct Affine3A { pub matrix3: Mat3, pub translation: Vec3 }
```

**Matrix-convention contract (resolves the row-major/column-major bridge hazard — CRITICAL).** The lifted `Mat3` is **row-major** (verified `math.rs:369`, `rows[i]` is row `i`; `from_quat`/`mul`/`transpose`/`mul_vec` all row-major). `Affine3A.matrix3` is therefore **row-major** and reuses the existing row-major `Mat3` ops verbatim — `Affine3A::transform_point(p) = matrix3.mul_vec(p) + translation` and `Affine3A::mul(parent,child)` reuses row-major `Mat3::mul` for the linear part (no new linear-algebra code on the affine path, eliminating one transpose-bug surface). `Mat4` is **column-major** to match WGSL. The ONLY convention boundary is `Affine3A::to_mat4()`/`Mat4::from_affine()`, which performs the explicit row-major-3×3 → column-major-4×4 transpose-and-embed in ONE clearly-commented function. Basis extraction for the camera (S3) reads **rows** of the row-major `Mat3` (a row-major Mat3's rotation rows are the world-space basis vectors for a rigid transform) — NOT "columns" — closing the S3 framing error.

**Methods (signatures).** `Affine3A`: `IDENTITY`, `from_translation_rotation_scale(Vec3,Quat,Vec3)`, `mul(self, Affine3A)->Affine3A` (parent ∘ child, reuses row-major `Mat3::mul`), `transform_point(Vec3)->Vec3`, `transform_vector(Vec3)->Vec3`, `inverse()->Affine3A`, `to_mat4()->Mat4`. `Mat4`: `IDENTITY`, `perspective_rh(fov_y,aspect,near,far)`, `orthographic_rh(...)`, `mul`, `from_affine(Affine3A)`. These are new code, not on the physics determinism path, but obey the same FMA-free contract.

**`Affine3A::inverse()` for general affine.** v1 implements the **general** affine inverse (3×3 inverse of the row-major `matrix3` via adjugate/determinant + `-inv·t`) so a scaled/sheared transform inverts correctly. The camera path (S3) constrains its input to rigid+uniform-scale and may use the cheaper transpose form, but the general method exists for correctness elsewhere. Cost noted in S3.

**GATES.**
- **Bit-determinism (load-bearing):** full `cargo test -p boyko_physics` byte-for-byte unchanged before/after migration; every `.to_bits()`-equality test (O9 x8 SDF, signed-zero ties) green with zero edits.
- **Convention gate (transpose-bug catcher):** `Mat4::from_affine(Affine3A::from_trs(t, r, s)).transform_point(p)` ≈ `affine.transform_point(p)` for several `t/r/s/p` **including a non-uniform scale (2,1,1) composed with a 90°-about-Z rotation** — identity/uniform-scale tests CANNOT catch a transpose, so the non-uniform+rotated case is mandatory. Plus: the resulting `view_proj` matches a hand-computed column-major reference.
- **asm gate:** as above (one-time, recorded).
- **0%-gate:** physics micro-benches within noise (same instructions ⇒ same perf).
- **Miri:** `boyko_math` is pure-safe except `inverse` (no `unsafe` expected); Miri runs the new-type unit tests.

**Dependencies.** None (root). S0 and S1 are independent and may proceed in parallel.

---

## S2 — `Transform` + `GlobalTransform` + propagation system

**GOAL.** Foundational spatial components + alloc-free, dirty-gated world-from-local composition over the existing `ChildOf`/`Children` tree, using S0's entity-yielding iteration.

**New crate.** `crates/boyko_scene/` (members += it). Deps: `boyko_ecs`, `boyko_math`, `boyko_macros`. Files: `transform.rs`, `propagation.rs`, `plugin.rs`, `lib.rs`.

**Components.**
```rust
// LOCAL pose relative to parent (designer-facing, decomposed). Gameplay-written,
// SCALAR access — NOT a SIMD-load target (resolves the alignment finding), so
// 40B / natural f32 align is correct; not over-padded.
#[repr(C)] #[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,   // 12 B
    pub rotation: Quat,      // 16 B
    pub scale: Vec3,         // 12 B  -> 40 B
}
impl Default for Transform { /* translation ZERO, rotation IDENTITY, scale ONE */ }

// Cached WORLD pose as a packed affine. SIZE/ALIGN const-asserted (see below).
#[repr(C, align(16))] #[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform(pub Affine3A);
impl Default for GlobalTransform { /* Affine3A::IDENTITY */ }
```

**Size correction + const-assert (resolves the wrong-64B claim).** `Affine3A { matrix3: Mat3 (rows:[Vec3;3]=36B), translation: Vec3 (12B) }` = **48 B payload**; `align(16)` rounds size to 48 B (already a multiple of 16). The earlier "64 B / one cache line" claim was wrong. v1 ships the **48 B `Mat3`-of-`Vec3` packed form** (NOT a Vec4-column-padded 64 B form): the affine is read as one ≤48 B sequential block straddling at most two cache lines, and the linear part reuses the row-major `Mat3` ops verbatim (Vec4-padding the rows would diverge from the lifted `Mat3`, reintroducing a convention/codepath split for zero hot-loop benefit since propagation is scalar affine compose, not SIMD matrix mul). The SIMD/GPU-direct justification is **dropped** for `GlobalTransform` (it is read scalar in propagation; the GPU record in S4 is a separate packed layout). Layout pinned by `const { assert!(size_of::<GlobalTransform>() == 48 && align_of::<GlobalTransform>() == 16) }`, house-style (cf. `light.rs`, `CameraUniform`). `Transform` pinned by `const { assert!(size_of::<Transform>() == 40) }`.

**Default-validity lint (D5/Bevy lesson).** `GlobalTransform::default() == IDENTITY` is a *valid* pose, so a renderable/light spawned this frame renders at the origin for ≤1 frame before propagation runs — documented, acceptable (NOT a NaN/garbage default). Pre-satisfies the "required component's Default must be valid before its producer runs" rule.

### Propagation algorithm (resolves the aliasing + cost-model + determinism findings)

A single per-frame system `propagate_transforms`, alloc-free, with a **real O(changed) dirty mechanism** (not O(all) per-node bit-testing).

**Dirty-source mechanism (resolves the "claimed O(changed), actually O(all)" finding).** The cost model does NOT rely on visiting every node to test a `Changed` bit. Instead:

1. **Roots pass** — `Query<(&Transform, &mut GlobalTransform), (Without<ChildOf>, Changed<Transform>)>::for_each_chunk` (archetypal `Changed` filter is per-row; for the root recompose we use the per-row `iter_entities_mut` with a `Changed`-gated body). Recompose `global.0 = Affine3A::from_translation_rotation_scale(...)` ONLY for changed roots. A static root's archetype is still visited at the archetype level, but the per-row work is skipped by the `Changed` tick test, which is a **contiguous `Tick`-column compare** (SIMD-friendly, ~bytes-of-tick-column per archetype, quantified below).

2. **Dirty-subtree descent** — descent into `Children` happens ONLY from a root/parent that **either** was itself recomposed this run **or** has `Changed<Transform>` on a descendant. To avoid O(all) child visitation, the descent is **seeded from a dirty set**: a parent recomposed in step 1 pushes its `Children` onto the work frontier; a `Changed<Transform>` on a deep child is discovered via a **per-archetype "any changed this run" summary** — the engine already maintains per-row change ticks; the propagation reads each *child archetype's* max-tick-vs-system-tick once per archetype (O(archetypes), not O(entities)) to decide whether that archetype contains any dirty child, and only then walks it. Archetypes with no dirty child this run are skipped wholesale.

   - **Honest cost statement.** Per frame, a fully-static scene costs: O(root-archetypes) `Changed`-tick column scans + O(child-archetypes) per-archetype summary checks. This is **O(archetypes) + O(changed-rows)**, NOT O(entities). For a 1M-static-entity scene in a handful of archetypes, this is a few contiguous `Tick`-column passes (each `Tick` is 4 B; a 1M-row archetype's tick column is 4 MB — but the per-archetype summary collapses it to one max-compare when the engine tracks an archetype-level changed summary; where it does not, the per-row scan is a single linear SIMD-friendly pass over a 4 MB tick column, ~well within streaming bandwidth and far cheaper than 1M affine composes). **The 0%-gate measures wall-time + bytes-touched vs entity count for a static scene** (not merely "affine composes == 0"), so the real traffic is the thing under test. *(If the kernel does not already expose a per-archetype changed-summary, S2 adds reading the archetype's existing change-tick high-water mark — a read of existing metadata, no new storage.)*

**Aliasing / addressing in the descent (resolves the parent-read/child-write CRITICAL).** The hierarchy pass reads a parent's already-computed `GlobalTransform` and writes a child's `GlobalTransform` — same component column, different rows. This is expressed via a **raw read-view into the `GlobalTransform` pool** obtained once at system start (a `*mut` base for writes, a `*const` base for parent reads — same allocation, distinct rows), addressed **by row through the entity→slot→row mapping** that S0's entity access + the kernel's `slot_of(entity)` provide. The descent is **single-threaded in v1** (parallelism deferred, documented), so there is exactly one writer.

- **SAFETY invariant (stated, Miri-TB is the HARD gate).** In a tree, a node and its parent are **always distinct entities ⇒ distinct slots ⇒ distinct rows ⇒ non-overlapping byte ranges** in the `GlobalTransform` pool. A child's `GlobalTransform` write therefore never aliases the parent's `GlobalTransform` read. The descent processes each node exactly once, parent strictly before child (topological), so a parent's `GlobalTransform` is fully written before any child reads it. Cycles are impossible (the `ChildOf`/`Children` invariant + the existing cycle guard). This is a raw-pointer descent with a documented `// SAFETY:` block; **Miri-TB on the descent is a blocking gate** (the project's repeated lesson: Miri-TB caught soundness bugs that critic+review approved). The roots pass remains `for_each_chunk`/`par`-eligible (disjoint rows) and is kept separate from the raw-pointer hierarchy descent.

- **Addressing.** `GlobalTransform` is addressed by entity via the kernel's `slot_of(entity) → row` (the dense fetch path already uses `store.slot_of(entity)`, verified `data.rs:631/671`), then `base.add(row)`. The descent walks `Children` (entity ids) → `slot_of` → `row` → `base.add(row)`. The on-stack frontier (`[Entity; CASCADE_FANOUT_INLINE=32]`, reused from the hierarchy convention) holds entity ids, with the alloc-free wider-fan-out fallback mirroring `Children::on_replace`.

**Determinism (resolves the Children-order finding).** **GlobalTransform output is provably invariant to sibling visit order**: each child's `GlobalTransform` depends ONLY on its own `Transform` and its parent's (already-finalized) `GlobalTransform` — never on a sibling. Therefore unspecified/`swap_remove`-perturbed sibling order in `Children` does not affect any node's final value. The plan **drops every order-dependent assertion**: the property test compares **final per-entity values**, never traversal/emission order. No pose-derived quantity feeds back into the physics solver (sync is one-directional per the S5 table), so physics bit-determinism is untouched.

**2D-as-subset (D3).** Same `Transform` with `translation.z = 0`, rotation about Z only, `scale.z = 1`; `GlobalTransform` composes identically (z-lane inert). No `Transform2D`. 2D consumers read `global.0.translation.xy()`. Wasted axis 4 B/entity — negligible; the hot 2D GPU path (`GpuInstance`, 24 B) packs its own 2D rep.

**Single-entity helper (cold).**
```rust
pub fn compute_global_transform(world: &EcsMaster, e: Entity) -> Affine3A; // walks ChildOf ancestors via S0 access
```

**GATES.**
- Unit: identity Transform → identity GlobalTransform; root composition; 2-deep & 3-deep chain == hand-computed affine; **non-uniform parent scale × rotated child produces correct shear** (the affine-not-TRS test, transpose-sensitive); reparent updates next run; `Without<ChildOf>` root path.
- Property: random tree, `compute_global_transform(e)` == propagated `GlobalTransform(e)` for every entity — **final values, order-independent** (two implementations agree on values).
- **0%-gate (real cost):** static scene wall-time + bytes-touched flat vs entity count (NOT just affine-composes==0); a debug counter asserts zero affine composes when nothing changed AND a bench shows the per-frame cost is O(archetypes), not O(entities).
- **Miri-TB (HARD):** the raw-pointer hierarchy descent (parent-read/child-write disjointness) AND `iter_entities_mut`.
- const-assert: `size_of::<GlobalTransform>()==48`, `size_of::<Transform>()==40`.

**Dependencies.** S0, S1.

---

## S5 — Physics ⇄ Transform pose sync + `Sensor`/`Trigger` (depends only on S0, S2)

**This phase resolves CRITICAL DESIGN POINT #1.**

**GOAL.** One canonical world pose, no duplicated/desyncable pose data (Principle 0).

**Pose source-of-truth.** `BodyType` lives in the cold `RigidBodyMass` column (verified `components.rs:89`), so sync queries carry `&RigidBodyMass`.
- **Dynamic:** the physics `RigidBody.{position,rotation}` is the authoritative integrated world pose (it is what the solver integrates and round-trips). `Transform`/`GlobalTransform` are **downstream**: after `physics_apply`, `sync_body_to_transform` copies pose **into** the entity's spatial components.
- **Static/Kinematic:** gameplay owns `Transform`; `sync_transform_to_body` copies the other direction, `Transform` → `RigidBody`, before the gather.

Never a duplicated authoritative pose; direction is `BodyType`-selected; one writer per pose per schedule window (see the schedule table).

**Sync systems (use S0 entity access).**
```rust
// FIXED schedule, before physics_gather. Static/Kinematic only.
fn sync_transform_to_body(q: Query<(&Transform, &mut RigidBody, &RigidBodyMass)>) // gated body_type != Dynamic
// FIXED schedule, after physics_apply. Dynamic ROOTS only.
fn sync_body_to_transform(q: Query<(&mut Transform, &RigidBody, &RigidBodyMass), Without<ChildOf>>) // gated body_type == Dynamic
```

**Field-granular copy (resolves the decompose-recompose drift finding).** `sync_body_to_transform` is a **pure field copy**, NOT a decompose of an Affine3A:
```
transform.translation = body.position;   // bit-exact copy
transform.rotation    = body.rotation;   // bit-exact copy (no re-normalize)
// transform.scale UNTOUCHED
```
The physics-authoritative bits reach `GlobalTransform` unrounded (propagation composes them; for a Dynamic root with identity-scale, `GlobalTransform.translation` **bit-equals** `RigidBody.position`).

**Change-detection reconciliation (resolves the "write-back defeats the dirty-gate" MAJOR).** A naive every-frame `Mut` write to `Transform` would set `Changed<Transform>` every frame for all dynamic bodies, defeating the static-scene dirty-gate. Resolution: **value-gated write** — `sync_body_to_transform` writes through the `Mut` guard ONLY when the integrated pose actually differs from the current `Transform` (`body.position != transform.translation || body.rotation != transform.rotation`, bit-compare). A **resting/sleeping** dynamic body (the engine already has `IslandSleep`) produces an unchanged pose → no `Mut` deref → no `Changed` tick bump → its subtree is skipped by propagation. This reconciles the 0%-gate with the physics common case: a scene of resting dynamic bodies pays only the O(archetypes) tick scan, not per-frame recompose.
- **Determinism gate:** a falling (moving) dynamic body's `GlobalTransform.translation` **bit-equals** `RigidBody.position` after sync+propagation (bit-exact for the position copy; the eps form is only for composed multi-level chains where float compose is involved).

**Scheduler conflict edges (resolves the scheduler-edges finding).** Both sync systems are stated as explicit conflict-graph constraints, not prose: `sync_transform_to_body` reads `Transform`+`RigidBodyMass`, writes `RigidBody` → conflicts with any system touching those in the same fixed stage, so the scheduler serializes it before `physics_gather` (which reads `RigidBody`). `sync_body_to_transform` reads `RigidBody`+`RigidBodyMass`, writes `Transform` → serialized after `physics_apply` (which writes `RigidBody`) and before any per-frame `Transform` reader. These are registered as `.after()`/`.before()` edges within the fixed schedule (intra-schedule, valid).

**Dynamic-bodies-must-be-roots enforcement (resolves the "doc note, no guard" MAJOR — promoted to a kernel guard).** A parented Dynamic body would get its WORLD pose written into a LOCAL `Transform`, then double-composed by propagation — silent corruption. Enforced THREE ways (not documentation):
1. The `sync_body_to_transform` query carries `Without<ChildOf>` (verified-feasible filter) so a parented Dynamic body is **structurally excluded** from the write — it cannot be silently mis-synced.
2. A **lifecycle hook/observer** (the engine has `on_add`/`on_insert` hooks, verified) fires when a `ChildOf` is added to an entity that has `RigidBody` + `BodyType::Dynamic`, emitting a `debug_assert!`/one-time warn ("Dynamic body parented; v1 does not support parented dynamics"). This is first-class kernel enforcement, not a comment.
3. **Gate:** a test that spawns a parented Dynamic body and asserts (a) it is excluded from `sync_body_to_transform` and (b) the hook fires the assert in debug.

**`Sensor` marker.**
```rust
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Sensor; // ZST: a Collider with Sensor reports overlaps but is skipped by contact resolution.
```
Integration: the gather adds a per-row `is_sensor` bit (read from archetype membership of `Sensor`, branch-free); `physics_solve_step` skips impulse application for pairs where either body is a sensor while still emitting the `Contact`/overlap snapshot. **0%-gate:** a world with no `Sensor` component id minted raises no sensor bit → gather/solve byte-identical to today.

**GATES.** Unit: dynamic body's `Transform` follows integrated `RigidBody`; static body's authored `Transform` drives `RigidBody`; falling dynamic root's `GlobalTransform.translation` **bit-equals** `RigidBody.position`; resting dynamic body produces NO `Changed<Transform>` (value-gate test). Sensor: overlapping sensor emits a `Contact` but applies **zero** impulse (velocities unchanged bit-for-bit). Parented-dynamic guard (above). 0%-gate: no-sensor and no-physics worlds unchanged; resting-dynamic scene pays O(archetypes) propagation. Determinism: physics suite stays green.

**Dependencies.** S0, S2 (and S1 transitively). Independent of S3/S4.

---

## S3 — `Camera` + `Projection` + active-camera + `ViewUniform` + renderer wiring

**GOAL.** An ECS entity drives the view; the renderer reads a derived `ViewUniform` instead of hardcoded per-backend camera sources.

**Crate.** `boyko_scene/camera.rs`. Renderer wiring: `boyko_render` (new `view.rs`), `boyko_rhi_vulkan` (the composite push-constant fill site).

**Components / resources.**
```rust
#[repr(C)] #[derive(Component, Clone, Copy, Debug)]
pub struct Camera { pub order: i32, pub is_active: bool, pub viewport: Option<Viewport> }
#[repr(C)] #[derive(Clone, Copy, Debug)]
pub struct Viewport { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

#[repr(C)] #[derive(Component, Clone, Copy, Debug)]
pub enum Projection {
    Perspective { fov_y: f32, aspect: f32, near: f32, far: f32 },
    Orthographic { half_height: f32, aspect: f32, near: f32, far: f32 },
}

// Resolved each frame: the renderer's single source of truth. Carries BOTH the
// view_proj matrix (raster/demo path) AND the decomposed eye/basis/fov (marcher
// push-constant path) — see the wiring note (resolves the view-source finding).
#[repr(C, align(16))] #[derive(Resource, Clone, Copy, Debug)]
pub struct ViewUniform {
    pub view_proj: Mat4,     // column-major, GPU-ready (demo CameraUniform)
    pub inv_view: Mat4,      // world-pos reconstruction (lights/marcher)
    pub camera_pos: Vec4,    // eye world pos (xyz), w free
    pub cam_forward: Vec4,   // marcher basis (xyz), normalized
    pub cam_right:   Vec4,
    pub cam_up:      Vec4,
    pub fov_y: f32, pub aspect: f32, pub near: f32, pub far: f32, // marcher scalars
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct ActiveCamera(pub Option<Entity>); // explicit; no implicit "first wins"
```

**Resolver system (`resolve_active_camera`, per-frame, `.after(propagate_transforms)`).**
- **Alloc-free active-camera selection (resolves the per-frame-alloc finding).** Selection is **iterate-and-track-max** over the camera query (no `collect`/`sort_by`): if `ActiveCamera(Some(e))`, use it; else single pass tracking the highest-`order` `is_active` camera in registers. Zero allocation.
- **Camera-transform invariant (resolves the sheared-camera findings).** A camera's `GlobalTransform` is **constrained to rigid + uniform-scale**. `resolve_active_camera` `debug_assert!`s the `matrix3` is orthonormal-up-to-uniform-scale (rows mutually orthogonal, equal length) — a sheared/non-uniform camera parent is **caught**, not silently distorted. Under that invariant: `view = global.0.inverse()` uses the cheap rigid inverse (transpose of the rotation + `-Rᵀt`); `proj = Mat4::perspective_rh/orthographic_rh`; `view_proj = proj * view` (Mat4 column-major mul, in place into the `ViewUniform` Resource — no realloc).
- **Marcher basis (resolves the view-source mismatch + row/col convention).** `eye = global.0.translation`; the orthonormal basis is read from the **rows of the row-major `Mat3`** (per the S1 convention contract — for a rigid transform the rotation rows are the world basis): `cam_forward/right/up` are derived and normalized (the `debug_assert` guarantees they are already unit under the invariant; normalization is belt-and-suspenders). `fov_y/aspect` from `Projection`. The marcher consumes eye+basis+fov (NOT view_proj) — `ViewUniform` carries both forms, so the demo gets `view_proj` and the marcher gets the decomposed lanes. No transpose in the hot path; the one row→basis read is documented.

**Renderer wiring.**
1. **Marcher (`CompositePushConstants`):** the struct, its `CAM_MODE_PERSPECTIVE`/`CAM_MODE_ORTHO` lanes (`cam_eye/cam_forward/cam_right/cam_up`), and the const-asserted offsets are **unchanged** — only the *fill source* moves from the hand-fed basis to `ViewUniform`'s decomposed lanes. The ORTHO golden path keeps `CompositePushConstants::ortho` for the bit-frozen fixture (gated by a `Projection::Orthographic` active camera) so golden tests stay byte-exact.
2. **Demo (`CameraUniform`):** `view_proj` from `ViewUniform.view_proj` (both column-major → direct copy). The Phase-20.1 `alpha` field is appended by the demo's upload, unchanged.

**2D-as-subset (D3).** A 2D game spawns a `CameraRig` with `Projection::Orthographic` + a `Transform` at `z>0` looking down −Z; `resolve_active_camera` produces an ortho `view_proj` equivalent to the demo's `ortho_fit`. No `Camera2D`.

**GATES.** Unit: perspective/ortho `view_proj` match reference matrices; `view = global.inverse()` round-trips (`view * global == IDENTITY` for a rigid camera); active-camera resolution order (explicit > order > is_active); sheared/non-uniform camera trips the `debug_assert`. Integration: marcher fed from `ViewUniform` renders the same image as the hand-fed basis for the golden perspective scene (`l1_divergence_probe`/composite anchors); ORTHO golden byte-exact. **Per-frame alloc gate:** `resolve_active_camera` allocates 0 (iterate-and-track-max). 0%-gate: one-camera scene resolves O(1). Miri: any new `unsafe` in the upload path.

**Dependencies.** S0, S1, S2.

---

## S4 — Render-capability set + GlobalTransform-driven GPU upload + lights read pose

**GOAL.** The "this entity is drawn" set, world-pose-driven instance upload, lights deriving position/direction from `GlobalTransform`.

**Crate.** `boyko_scene/render_caps.rs`; `boyko_render` gains upload/reconcile systems; a new `Gpu3dInstance` dense component (see below).

**Components.**
```rust
#[repr(transparent)] #[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshHandle(pub u32);
#[repr(transparent)] #[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialHandle(pub u16);
#[repr(u8)] #[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility { #[default] Inherited, Visible, Hidden }
```
**Visibility backend (F).** `Visibility` is user intent; high-churn show/hide rides the existing `EnableTag` bitset (O(1), no migration) — documented as the recommended per-frame-toggle path. `InheritedVisibility`/`ViewVisibility` explicitly **deferred** (not v1).

**GPU instance upload (resolves the 24B-const-assert + prev_pos CRITICAL).** The existing 2D `GpuInstance` (24 B `repr(C)`, const-asserted `size==24 align==4`, every field a WGSL `@location`, with the Phase-20.1 single-writer `prev_pos` shuffle for GPU `mix(prev,pos,alpha)`) is **untouched and byte-frozen** — its 0%-gate is re-affirmed. The 3D world pose does NOT fit 24 B, so v1 introduces a **new, separate dense component** `Gpu3dInstance` with its OWN layout contract:
```rust
// NEW dense component — its own const-asserted layout + WGSL attribute contract.
// translation (12B) + packed rotation+scale OR affine columns (see below).
#[repr(C)] #[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Gpu3dInstance { /* world_pos: Vec3, plus packed linear part; material: u16; pad */ }
const _: () = assert!(size_of::<Gpu3dInstance>() == /* pinned */ && align_of::<Gpu3dInstance>() == /* pinned */);
```
- **Principle-0 honesty (resolves "the column IS the buffer" contradiction).** `Gpu3dInstance` **is** the dense component column AND the GPU vertex/instance buffer (no parallel `std::Vec` mirror). But because the SOURCE of truth is `GlobalTransform` (a different column), `sync_gpu_instances` is an **explicit pack-into-GPU-column system** — one sequential `Affine3A` read + one packed `Gpu3dInstance` write per visible row, alloc-free. This is correctly stated as a transform/write, NOT zero-copy: the zero-copy `for_each_chunk` SoA→GPU **upload** still applies to the `Gpu3dInstance` column itself (column → GPU is `cast_slice`), but `GlobalTransform` → `Gpu3dInstance` is a pack step. The earlier "the column is the buffer AND carries the affine via zero-copy from GlobalTransform" was self-contradictory; this resolves it.
- **3D interpolation story (explicit).** `Gpu3dInstance` ships **without** GPU-side `mix(prev,pos,alpha)` interpolation in v1 (so there is no second prev-shuffle writer to reconcile with Phase 20.1; the 2D path's single writer is untouched). 3D interpolation is a future phase. Stated, not silently dropped.
- `Visibility::Hidden` rows skipped branch-free via the `EnableTag`/archetype gate.

**Lights read pose (resolves the light reconcile CRITICAL/MAJOR + axis/sign).** A `light_reconcile` system runs **before `collect_lights`** (per-frame, `.after(propagate_transforms)`). For a light entity with `GlobalTransform`:
- **Value-gated write (resolves the collect_lights-rebuild finding).** reconcile writes `position`/`direction` into the light component ONLY when the `GlobalTransform`-derived value differs from the current stored value (bit-compare), AND is itself gated on `Changed<GlobalTransform>`. A **static parented light** therefore does NOT perpetually dirty `collect_lights` — it pays zero. `collect_lights` (verified `Changed`-gated, rebuilds the whole `MAX_LIGHTS` table on any change) only rebuilds when a light actually moved.
- **Axis/sign convention (resolves the under-specified finding).** Local forward axis is **−Z** (engine convention). For directional/spot, `direction` stays **"direction TO the light"** (matching the untouched `from_directional`/`from_spot` bake): `direction = normalize(global.0.matrix3.mul_vec(local_forward))` where `local_forward = (0,0,-1)` and the result is interpreted as the existing "to-light" convention — byte-compatible with `collect_lights`. (`matrix3.mul_vec` is the row-major op per S1; for point lights, `position = global.0.translation`.)
- A light **without** `GlobalTransform` keeps its self-contained position/direction (back-compat). `collect_lights` is **unchanged**.

**GATES.** Unit: `MeshHandle`/`MaterialHandle`/`Visibility` round-trip; `Hidden` excluded from upload; **a light rotated by a known Quat yields the hand-computed to-light direction** (sign/axis assertion); a moved light's GPU entry tracks its `Transform`; **a static light WITH `GlobalTransform` produces NO `collect_lights` rebuild** (value-gate); a light WITHOUT `GlobalTransform` is byte-identical to today. Integration: parented light orbiting its parent updates direction. const-assert: `Gpu3dInstance` size/align pinned; 2D `GpuInstance` 24 B re-affirmed unchanged. 0%-gate: no-`MeshHandle` scene skips the upload system; static lights pay nothing. Miri: the `Gpu3dInstance` pack + `cast_slice` upload path.

**Dependencies.** S0, S1, S2, S3.

---

## S6 — Object-category bundles + `Name`

**GOAL.** The owner's category menu as named `#[derive(Bundle)]` presets over the now-existing parts (Bundle derive rejects tuple/generic bundles — verified; Phase-8.5 static cache; `MAX_BUNDLE_ARITY=16`).

**Crate.** `boyko_scene/bundles.rs`, `boyko_scene/identity.rs`.

**Leaf component.**
```rust
#[repr(transparent)] #[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name(pub NameId);   // NameId = u32 into a SETUP-ONLY string interner
```
**Name interner Principle-0 boundary (resolves the scope-guard finding).** The interner is **setup/cold-only metadata** (consistent with the project's "globals are metadata" audit): interning happens at spawn/setup, never per-frame; no hot-path lookup consults it. It mirrors the existing global-registry metadata pattern. Stated explicitly so it cannot drift into a parallel data system. (`Visibility` already shipped in S4.)

**Bundles (all named structs).**
```rust
#[derive(Bundle)] pub struct SpatialBundle { transform: Transform, global: GlobalTransform, visibility: Visibility }
#[derive(Bundle)] pub struct StaticProp    { transform, global, mesh: MeshHandle, material: MaterialHandle, visibility } // rendered, no physics
#[derive(Bundle)] pub struct DynamicBody   { transform, global, mesh, material, body: RigidBody, mass: RigidBodyMass, collider: Collider, visibility } // arity 8 ≤ 16
#[derive(Bundle)] pub struct Trigger       { transform, global, collider: Collider, sensor: Sensor }
#[derive(Bundle)] pub struct CameraRig     { transform, global, camera: Camera, projection: Projection }
// LightObject is NOT generic (derive rejects generics): ship 3 concrete bundles.
#[derive(Bundle)] pub struct DirectionalLightObject { transform, global, light: DirectionalLight }
#[derive(Bundle)] pub struct PointLightObject       { transform, global, light: PointLight }
#[derive(Bundle)] pub struct SpotLightObject        { transform, global, light: SpotLight }
```

**Bundle-vs-require interaction (forward-looks to S8).** S6 ships bundles as the v1 instancing story. Whether these are later **replaced by** require-closures (S8) or **kept as convenience over** them is an explicit owner decision surfaced in S8 (it changes what S6 ultimately ships). v1: bundles stand alone.

**GATES.** Unit: each bundle spawns exactly its component set (archetype membership asserts); warm-path spawn hits the static bundle cache (no per-spawn rebuild). Integration: a `DynamicBody` falls (physics) and renders at its world pose (full-pipeline smoke). 0%-gate: bundle spawn within noise of equivalent manual multi-insert. No new `unsafe`.

**Dependencies.** S2, S3, S4, S5.

---

## S7 — Prefab / instantiate (NET-NEW subtree machinery — re-scoped)

**GOAL.** Spawn a whole object tree from a reusable template via column-blit + `ChildOf` remap.

**Re-scope (resolves the "already shipped" CRITICAL — it is FALSE for the operation S7 needs).** Verified: `boyko_serialize` exposes ONLY whole-world `save_world`/`load_world`; its format is archetype-grouped over the **whole world**; the row-write driver (`load_writer`) and the pool/archetype/entity-master write primitives are **crate-private in `boyko_ecs`**; the `ChildOf` remap runs only inside the full-world loader. There is **no** subtree `capture(world, root)`, no in-memory partial image, no partial-instantiate into a live world. **S7 is therefore NET-NEW machinery, not a reuse.** It is re-scoped honestly as a phase that adds crate-public primitives to `boyko_ecs`:

**New machinery (with realistic effort).**
1. **Subtree selection** — `Prefab::capture(world, root)` walks the `ChildOf`/`Children` subtree (via S0 entity access), collecting the root's descendants and grouping their rows by archetype. NEW.
2. **In-memory `Prefab` image type** — an archetype-grouped byte image (POB columns blitted via `copy_nonoverlapping`; `Entity`-bearing columns recorded with their `serialize_fn`/`map_entities_fn` per the existing C4 contract). The column-blit byte engine (the POB `memcpy`-per-column kernel) IS shared with `boyko_serialize` (same primitive), but the subtree-scoped image type is NEW.
3. **Partial instantiate into a live world** — `instantiate(world, &prefab)` allocates fresh archetype rows (NOT a fresh-world load), blits columns, runs a **scoped `ChildOf` remap** (saved internal parent id → fresh entity) so internal links rewrite to the new instance; external refs follow the existing C4 contract. NEW (the existing remap is full-world-load-only).
4. **Crate-public primitives** — the minimal `boyko_ecs` write primitives (fresh-row allocation, column base for blit, entity-id append) are promoted to **crate-public** (or a `boyko_ecs`-internal `pub(crate)` surfaced via a thin scene-facing API) WITHOUT leaking archetype internals into the public type signatures (the `Prefab` opaque type is the boundary). Stated as a requirement.

```rust
pub struct Prefab { /* opaque: archetype-grouped byte image + ChildOf column metadata */ }
impl Prefab { pub fn capture(world: &EcsMaster, root: Entity) -> Prefab; }
pub fn instantiate(world: &mut EcsMaster, prefab: &Prefab) -> Entity; // new root, ChildOf-remapped
```
`Transform`/`GlobalTransform` are POB (no `Entity`) → blitted; `GlobalTransform` is recomputed by the next `propagate_transforms` (blitted value is a valid stale pose, not garbage — Default-validity holds).

**Owner-facing scope note.** S7 carries non-trivial NET-NEW kernel work (crate-public primitives + scoped remap). If the owner prefers a leaner v1, the documented alternative is to **ship bundles (S6) as the v1 instancing story and defer prefab** — surfaced as an explicit scope call, not a silent assumption. The plan defaults to building S7 but flags the descope option.

**GATES.** Unit: capture+instantiate a 3-deep tree → structure identical, internal `ChildOf` remapped to fresh entities, external refs per C4; blitted `Transform` correct, `GlobalTransform` recomputed after propagation. Property/fuzz: reuse the existing loader-fuzz harness shape (Err-or-valid-never-UB) for malformed templates. 0%-gate: POB columns take the bulk-`memcpy` path (assert via the `columns_blitted` stat). Miri (curated): the blit + remap unsafe.

**Dependencies.** S0, S2 (Transform POB), S6 (bundles define templates). NOT a serialization-reuse — flagged above.

---

## S8 — Required-Components wiring (`#[require(...)]`) — THIN WIRING PHASE (re-scoped)

**Re-scope (resolves the mis-scoped-as-macro-work CRITICAL).** Verified: `#[require(...)]` is **ALREADY SHIPPED** — the `Component` derive declares `attributes(component, require, entities)`; `RequiresSpec`/`RequireEntry` parse all forms; `ctor_fns_codegen`/`codegen`/`install_codegen` emit the constructors and install path; a full test suite exists (`required_components.rs`, `required_property.rs`, `required_insert_fire.rs`, `required_spawn_batch_ub.rs`, `required_cycle.rs`, `compile_fail_require/*`). **S8 is NOT macro-build work.** It is a thin phase: (1) add `#[require(...)]` declarations to the new scene components; (2) decide bundle-vs-require; (3) gate-test archetype equivalence.

**Wiring.**
```rust
#[derive(Component)] #[require(Transform, GlobalTransform)] pub struct MeshHandle(...);
#[derive(Component)] #[require(Transform, GlobalTransform)] /* on each light component */
#[derive(Component)] #[require(Transform, GlobalTransform)] pub struct Camera{...} // + Projection
```
At insert, the shipped require closure auto-inserts the required components ONLY if absent (no double-insert, archetype computed with the full closure once, cached). Enforces "a renderable/light/camera can never exist without a pose."

**Owner decision surfaced (the genuine open call).** Whether `SpatialBundle`/`StaticProp`/… (S6) are **REPLACED by** require-closures (Bevy's migration) or **KEPT as convenience over** them is an explicit owner-facing scope decision — it changes what S6 ultimately ships. The plan recommends KEEP-as-convenience for v1 (bundles are a one-call ergonomic; requires guarantee the invariant) but defers the final call to the owner.

**Compatibility gate (resolves "confirm require composes with the bundle static cache").** A gate verifies the shipped require precedence (first-DFS / W1) composes correctly with the Phase-8.5 bundle static cache: inserting `MeshHandle` alone yields the **identical archetype** to spawning via `StaticProp` (so the cache is not bypassed and no extra archetype churn occurs).

**GATES.** Unit: `MeshHandle` alone auto-inserts `Transform`+`GlobalTransform`; manual supply does NOT double-insert; archetype identical via bundle vs require-closure (the compatibility gate). 0%-gate: require expansion is the same archetype-cached path as bundles (no extra migration). Miri: the insert-closure path (covered by the shipped suite; re-run for the scene components).

**Dependencies.** S6 (bundles to relate to requires), all component types. The `#[require]` machinery itself is already shipped.

---

## Cross-cutting: critical design points (explicit resolutions)

1. **Pose source-of-truth (#1):** S5 — `BodyType`-selected direction; Dynamic = RigidBody→Transform (field-copy, value-gated) after apply, Static/Kinematic = Transform→RigidBody before gather; one writer per pose per schedule window (schedule table); parented-dynamic guarded THREE ways (`Without<ChildOf>` filter + lifecycle-hook assert + test).
2. **Math-lift bit-determinism (#2):** S1 — verbatim lift; no FMA/rsqrt/rcp introduced; FMA-suppression mechanism stated (no fp-contract=fast, no target_feature fma, no float_algebraic) + asm gate over new ops too; full physics suite byte-for-byte.
3. **GlobalTransform packed affine + cost model (#3):** S1/S2 — `Affine3A{row-major Mat3, Vec3}`=**48 B** (corrected, const-asserted) carries shear; propagation cost is **O(archetypes)+O(changed)** (honestly stated, not O(changed) alone), value-gated by `Changed` + per-archetype summary; 0%-gate measures wall-time/bytes vs entity count.
4. **2D-as-subset (#4):** S2/S3 — same `Transform` (z=0) + `Projection::Orthographic`; no `Transform2D`/`Camera2D`.
5. **Crate graph (#5):** `boyko_math` leaf root; new `boyko_scene`; `boyko_ecs` stays a pure kernel; no cycles (`boyko_macros` ↛ `boyko_ecs` respected).
6. **Entity identity in queries (NEW, was a blocker):** S0 — first-class `iter_entities`/`for_each_chunk_entities` kernel feature; threads the already-cached `entity_ids` base; additive, 0%-gate on existing iterators.
7. **Hierarchy aliasing (NEW resolution):** S2 — raw-pointer descent with a stated disjoint-rows SAFETY invariant (distinct entities ⇒ distinct slots ⇒ distinct rows), single-threaded, Miri-TB as HARD gate.
8. **Matrix convention (NEW resolution):** S1 — row-major `Mat3` reused on the affine path verbatim; column-major `Mat4` only at the single `to_mat4`/`from_affine` boundary; camera basis from row-major rows; transpose-sensitive non-uniform+rotated gate.
9. **3D GPU instance vs 2D 24 B frozen path (NEW resolution):** S4 — separate `Gpu3dInstance` dense component with its own const-asserts; 2D `GpuInstance` untouched; `sync_gpu_instances` is an explicit pack (not zero-copy from GlobalTransform); 3D interpolation deferred (no second prev-shuffle writer).

## Phase dependency order

`{S0, S1}` (independent roots, parallel) → S2 (needs S0+S1) → `{S5 (needs S0+S2), S3 (needs S2)}` (parallel after S2) → S4 (needs S3) → S6 (needs S2–S5) → S7 (needs S6, NET-NEW) → S8 (thin wiring, needs S6; `#[require]` already shipped).

## Schedule placement (consolidated)

Fixed: `sync_transform_to_body` → physics pipeline → `sync_body_to_transform`. Per-frame (after fixed fully advances): `propagate_transforms` → `resolve_active_camera` / `light_reconcile` / `sync_gpu_instances` → `collect_lights`. Cross-schedule ordering enforced by schedule membership; intra-schedule by `.after()`/`.before()` conflict edges.

## Files touched (summary)

- **New crates:** `crates/boyko_math/` (S1), `crates/boyko_scene/` (S2–S7).
- **Changed:** workspace `Cargo.toml` (members); `boyko_ecs` (S0 entity-yielding iteration in `iters/query/`; S7 crate-public subtree primitives); `boyko_physics/Cargo.toml` + `src/math.rs` (call-site migration + shim removal, S1), `src/systems.rs`+`plugin.rs` (S5 sync ordering, parented-dynamic hook, Sensor gate), `src/components.rs` (Sensor / gather `is_sensor` bit); `boyko_render` (new `view.rs` S3; `Gpu3dInstance` + `sync_gpu_instances` + `light_reconcile` S4); `boyko_rhi_vulkan/src/compute.rs` (composite push-constant fill source → `ViewUniform`-derived, struct unchanged); `boyko_demo` (`CameraUniform` fed from `ViewUniform`); `boyko_scene` (`#[require]` declarations, S8).

## Plan readiness checklist

- Goal/metrics per phase: ✔ (cost models, 0%-gates, bit-exact gates stated).
- Every decision justified via perf/cache/parallelism: ✔.
- Data structures: `repr`/align/size const-asserts stated (GlobalTransform 48 B corrected, Transform 40 B, Gpu3dInstance pinned, 2D GpuInstance frozen): ✔.
- Hot/cold split: ✔ (Transform hot/scalar, RigidBodyMass cold carries BodyType, Affine3A one read).
- No `dyn`/`Box`/`Mutex` in hot path: ✔ (S0 additive monomorphized; propagation raw-ptr; sync field-copy).
- Multithreading: ✔ (schedule table; one writer per pose per window; roots pass disjoint-row parallel; hierarchy descent single-threaded v1 with stated SAFETY; Miri-TB hard gate).
- Correctness/edge cases: ✔ (parented-dynamic guarded; resting-body value-gate; sheared-camera debug_assert; cycle-impossible descent; default-validity).
- Unsafe invariants stated: ✔ (S0 entity-base disjoint allocation; S2 descent disjoint rows; S4 cast_slice; S7 blit+remap).
- Integration/compat: ✔ (Arena/ComponentPool/UnitId untouched; `#[require]` shipped; serialize NOT reused for S7 — flagged).
- Validation (unit/property/bench/Miri/debug_assert): ✔ per phase.

## Changes from review

- **Added S0 (entity-yielding iteration kernel feature).** Resolves Critic-1 C1 (CRITICAL): `Entity`/`EntityId` is not public `QueryData`; only a private `entity_ids` fetch-base and a pure-dense `(EntityId,&T)` iterator exist (verified `data.rs`, `query.rs`). S0 promotes `iter_entities`/`for_each_chunk_entities` as a first-class kernel feature (Principle 0), additive with a 0%-gate, threading the already-cached `entity_ids` base. Propagation/sync/cold-walk now have a concrete mechanism.
- **Hierarchy-pass aliasing fully specified.** Resolves Critic-1 C2 + Critic-3 (CRITICAL): raw-pointer descent over the `GlobalTransform` pool with a stated SAFETY invariant (distinct entities ⇒ distinct slots ⇒ distinct rows ⇒ non-aliasing parent-read/child-write), entity→slot→row addressing via the kernel's `slot_of`, single-threaded v1, Miri-TB as a HARD gate. The roots pass stays disjoint-row parallel.
- **Determinism claim corrected.** Resolves Critic-1 C3 (CRITICAL): proved `GlobalTransform` output is invariant to sibling visit order (each child depends only on parent); dropped all order-dependent assertions; property test now compares final per-entity values only. `Children` `swap_remove` perturbation is now irrelevant.
- **Cost model made honest.** Resolves Critic-2 C3 (CRITICAL): replaced the overstated "O(changed)" with **O(archetypes)+O(changed)**, added a per-archetype changed-summary to skip static subtrees without per-node visitation; the 0%-gate now measures wall-time/bytes-touched vs entity count, not "affine composes == 0".
- **Matrix convention pinned.** Resolves Critic-2 C2 + Critic-3 minor (CRITICAL): `Affine3A.matrix3` is row-major (reuses lifted `Mat3` verbatim — no new linear algebra on the affine path); `Mat4` column-major only at the single `to_mat4`/`from_affine` boundary; camera basis from row-major **rows** (fixing the S3 "columns" framing); transpose-sensitive non-uniform-scale + rotation gate added.
- **GPU instance contradiction resolved.** Resolves Critic-2 C1 (CRITICAL): 2D `GpuInstance` (24 B, const-asserted, prev_pos single-writer) stays byte-frozen; introduced a separate `Gpu3dInstance` dense component with its own const-asserts; `sync_gpu_instances` is an explicit pack-from-`GlobalTransform` step (NOT zero-copy from GlobalTransform — the self-contradiction removed); 3D GPU interpolation explicitly deferred (no second prev-shuffle writer).
- **S7 re-scoped honestly.** Resolves Critic-3 C1 (CRITICAL): `boyko_serialize` is whole-world only; row-write driver is crate-private; the `ChildOf` remap is full-world-load-only (verified). S7 is now NET-NEW machinery (subtree capture, in-memory image, partial live-world instantiate, scoped remap, crate-public primitives), with a documented descope-to-bundles option.
- **S8 re-scoped as thin wiring.** Resolves Critic-3 C2 (CRITICAL): `#[require(...)]` is already shipped (verified macro + test suite); S8 is now declarations + the bundle-vs-require owner decision + an archetype-equivalence/static-cache compatibility gate — not macro-build work.
- **Sync ↔ change-detection reconciled.** Resolves Critic-1/Critic-2 MAJOR: `sync_body_to_transform` is a value-gated field copy (bit-compare position/rotation; scale untouched; no decompose-recompose drift), so a resting/sleeping dynamic body produces no `Changed<Transform>` and the dirty-gate holds for the common physics case.
- **Light reconcile value-gated.** Resolves Critic-1/Critic-2 MAJOR + minor: reconcile writes only on actual value change AND is `Changed<GlobalTransform>`-gated, so a static parented light does not rebuild `collect_lights`; axis (−Z local forward) and sign ("direction TO the light") pinned; byte-identical-without-GlobalTransform gate kept.
- **Parented-dynamic guarded by the kernel.** Resolves Critic-1/Critic-3 MAJOR: `Without<ChildOf>` query filter + a lifecycle-hook `debug_assert`/warn + a gate test, replacing the doc-only limitation.
- **Multi-schedule placement specified.** Resolves Critic-3 MAJOR: a schedule-placement table (fixed vs per-frame) for every pose/render/light system; Phase-20.1 interpolation reconciled (Transform holds latest substep; 3D record has no interpolation in v1); cross-schedule ordering by schedule membership, intra by `.after()`.
- **S3 per-frame alloc + camera invariant.** Resolves Critic-2 MAJOR + minor: active-camera selection is alloc-free iterate-and-track-max; camera `GlobalTransform` constrained to rigid+uniform-scale with a `debug_assert`; `ViewUniform` carries BOTH `view_proj` (demo/raster) AND decomposed eye/basis/fov (marcher) — fixing the "view_proj direct upload to the marcher" mismatch; `inverse()` uses the cheap rigid form under the invariant (general affine inverse exists in S1 for other callers).
- **FMA-suppression mechanism stated.** Resolves Critic-2 MAJOR: concrete enforcement (no fp-contract=fast, no `target_feature(fma)`, no float_algebraic) + asm gate extended to the new `Mat4`/`Affine3A` ops, not just normalize/integrate.
- **GlobalTransform size corrected + const-asserts added.** Resolves Critic-3 minor: 48 B (not 64 B); SIMD/one-cache-line justification dropped for the scalar-read `GlobalTransform`; `Transform` stated scalar-written (40 B/align4 correct, not over-padded); both layouts const-asserted.
- **Physics math shim folded into S1.** Resolves Critic-3 minor: call-site migration + shim removal happen within S1 (determinism suite as the net), math tests get a single home in `boyko_math` — no open-ended deferred cleanup.
- **Name interner bounded.** Resolves Critic-3 minor: explicitly setup/cold-only metadata, no hot-path lookup, mirroring the global-registry pattern.