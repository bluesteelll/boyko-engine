# Architecture: #35 — CameraRig drives the on-screen ViewUniform (live render-view path)

Status: PLAN (implementation-ready, critic round 1 applied). Branch `ecs`. English only.

---

## Critic round 1 — resolutions

All findings ACCEPTED and patched in place. The critic CONFIRMED the handedness/look-at
chain end-to-end; these are test-rigor + spec-precision + one real investigation.

| ID | Finding | Resolution (where) |
|---|---|---|
| **C2 + Open-Q1** | `Quat::from_mat3` must be the EXACT algebraic inverse of `Mat3::from_quat`'s sign layout; transcribe the 9 elements + write all 4 Shepperd branches explicitly. | §6.2 rewritten: the `from_quat` layout transcribed verbatim from mat.rs:96-106 (index convention `m_ij ≡ rows[i].component(j)` pinned), the off-diagonal sum/difference identities derived, and all FOUR branches (trace + x/y/z-diagonal) written out with their exact off-diagonal patterns. This is the dev contract. |
| **C1** | Replace transpose-blind round-trip with a DIRECTIONAL assertion + cover all 4 branches with directional checks. | §7 math matrix rewritten: element-by-element `Mat3::from_quat(Quat::from_mat3(m_90z)) == m_90z` on a NON-symmetric matrix, a HAND-COMPUTED `from_mat3(m_90z).rotate(known_vec)` ground-truth, and the proptest extended to a directional check on the trace branch AND near-180° about X/Y/Z (all four branches). |
| **C3 + Open-Q2** | REAL investigation: does propagation compose a lone root, change-gated, with no one-frame lag? | INVESTIGATED (file:line in §0.5). FINDING: a lone root IS recomposed (`propagation.rs:264-272`) but the recompute is DIRTY-GATED (`propagation.rs:396-424`) and the world tick bumps only at `Schedule::run` frame start (`ecs_master.rs:3705-3708`), NOT on a bare `run_system` — so the bare 3-`run_system` test/example sequence has a real same-window-skip hazard. RESOLVED: §0.5 + D1 + §8 + §13 mandate a `world.bump_change_tick()` (or `Schedule`-frame) between the rig write and `propagate_transforms`, plus a §7 test that proves "this-frame pose → this-frame ViewUniform, no lag". |
| **W1** | look_at_rh pole fallback must change ONLY the source `up`, reuse the SAME cross order; assert det≈+1 on the degenerate cases. | §6.1 step 2 pins "fallback changes the up vector only; `right = cross(up,back)` then `true_up = cross(back,right)` order is NEVER reordered". §7 degenerate-guard cases now assert `det(matrix3) ≈ +1`. |
| **W2** | Pin `Mat3::from_columns` exact `rows` layout. | D2 + §6 + §7: `from_columns(c0,c1,c2)` produces `rows = [(c0.x,c1.x,c2.x),(c0.y,c1.y,c2.y),(c0.z,c1.z,c2.z)]` (transpose pattern) so `mul_vec(unit_axis_i) == c_i`. Test kept as the mechanical guard. |
| **O1** | Doc the two `scene.mvp` re-push sites. | D3 note: re-pushed at swapchain.rs:1356 (`record_scene`, the windowed `render_scene_frame` path the example drives) AND swapchain.rs:2542 (`record_gbuffer`, a raster path reached only by `render_gbuffer_frame`). `set_mvp` affects whichever path records next; the example drives 1356. |
| **O2** | Resolve the `#[require]` open question by mandating an explicit spawn list. | §4 + §13: the example/test spawn list is `Camera + Projection + OrbitCamera + Transform + GlobalTransform` EXPLICITLY. `#[require]`-chaining dropped from the critical path (pure ergonomic nicety). Open question 1 removed. |

**Verification facts gathered this round (file:line):**
- `Mat3::from_quat` sign layout TRANSCRIBED + CONFIRMED byte-exact (mat.rs:96-106).
- `Quat { x,y,z,w }` all f32 (quat.rs:20-31); `Quat::rotate` = `v + 2w(u×v) + 2(u×(u×v))`, active (quat.rs:100-105); NO `from_mat3` today.
- `Mat3` row-major `rows:[Vec3;3]` (mat.rs:26-31); `mul_vec` = per-row dot = `self·v` (mat.rs:57-63); NO `from_columns` today.
- `Affine3A { matrix3, translation }` (affine.rs:22-29); `inverse()` is GENERAL (mat.rs adjugate) not rigid-only (affine.rs:91-98); NO `look_at` today.
- `Vec3::normalize` returns `Vec3::ZERO` on zero input, not NaN (vec.rs:226-233); `cross` right-handed (vec.rs:186-192); NO `X/Y/Z` consts — build via `new` (vec.rs:156-167).
- `Transform { translation, rotation, scale }` (transform.rs:44-53); `to_affine` via `from_translation_rotation_scale` (transform.rs:106-108); NO `from_affine`.
- Lone-root recompose branch `propagation.rs:264-272`; dirty-gate `propagation.rs:396-424` (tick test `:412-419`); tick bump ONLY at `Schedule::run` frame start `ecs_master.rs:3705-3708`.
- `Scene.mvp` PRIVATE `[u8;64]` (`SCENE_MVP_BYTES=64`), set only in `Scene::new`, NO existing setter (swapchain.rs:3318/3328/3283/3344). Re-push sites: `record_scene` swapchain.rs:1356-1363 (vertex stage, offset 0, unconditional — windowed, `render_scene_frame` calls it at :1103); `record_gbuffer` swapchain.rs:2542-2549 (vertex, offset 0, unconditional — raster, only `render_gbuffer_frame` at :2256). `boyko_rhi_vulkan` deps = `boyko_rhi` + `boyko_sdf_math` only (Cargo.toml), NEITHER scene nor render.

---

## 0. Delta — what S3 gave vs what #35 adds

**S3 (shipped) gave a STATIC view pipeline.** A camera pose is hand-placed: a test writes
`GlobalTransform(Affine3A { matrix3, translation })` directly (see
`crates/boyko_render/tests/camera_drives_render_gpu.rs`), and the existing systems
turn that pose into a GPU-ready matrix:

```
[hand-placed pose] -> resolve_active_camera -> ViewUniform -> composite_*_from_view -> marcher
                                                            \-> view_proj_columns   -> raster MVP
```

S3 already owns: `Camera`/`Projection`/`ActiveCamera`/`ViewUniform`,
`ViewUniform::from_camera(global, projection)` (does `view = global^-1`, `view_proj = proj*view`,
basis from the COLUMNS of `matrix3` with convention right=+X, up=+Y, forward=-Z),
`resolve_active_camera(Query<(&Camera,&Projection,&GlobalTransform)>, Res<ActiveCamera>, ResMut<ViewUniform>)`,
and the `boyko_render::view` bridge (`composite_from_view`, `composite_perspective_from_view`,
`demo_view_proj_from_view`, `view_proj_columns`).

**#35 adds the MISSING CAMERA CONTROLLER and the LIVE ON-SCREEN seam.** Today there is
NO rig — every pose is a literal. #35 delivers:

1. **`OrbitCamera` component + `orbit_camera_system`** in `boyko_scene` — derives the
   camera's local `Transform` from `{ target, distance, yaw, pitch }` so the eye orbits the
   target on a sphere and looks AT it. (Principle 0: a component + a system on ECS storage,
   no parallel data store.)
2. **`look_at_rh` + the enabling `Quat::from_mat3`** in `boyko_math` — the one new math
   primitive family that turns `(eye, target, up)` into a rigid camera world rotation
   matching the `from_camera` column convention.
3. **`Scene::set_mvp`** in `boyko_rhi_vulkan` — a tiny additive per-frame MVP mutator so the
   windowed present path can be driven by the live `ViewUniform` each frame (the renderer
   already re-pushes `scene.mvp` every frame; only the public setter is missing).
4. **Two proofs**: (a) an offscreen `#[ignore]` SDF screenshot the orchestrator verifies on
   the RTX (the rig at two orbit angles -> the view rotates), and (b) a live windowed orbit
   example the OWNER runs (a cube orbited by the rig, on screen, with a known-angle readback).

After #35 the path is end-to-end LIVE:

```
OrbitCamera{yaw,pitch,...} -> orbit_camera_system -> Transform -> propagate_transforms ->
   GlobalTransform -> resolve_active_camera -> ViewUniform -> view_proj_columns ->
   Scene::set_mvp -> render_scene_frame -> PRESENT (windowed)   [and the marcher path for SDF]
```

---

## 0.5. C3 INVESTIGATION — does propagation compose a lone-root camera, change-gated, with NO one-frame lag?

**Question (load-bearing).** The rig writes the camera's local `Transform`; `resolve_active_camera`
reads `GlobalTransform` (camera.rs:485-486). Does a PARENTLESS root camera (a `Transform` +
`GlobalTransform`, no `ChildOf`, no `Children`) get its `GlobalTransform` recomputed from its OWN
`Transform` on each `propagate_transforms` run, change-gated on the rig's `&mut Transform` write,
with NO one-frame lag?

**FINDING (investigated, file:line cited).**
1. **A lone root IS recomposed.** `propagate_transforms` seeds each dirty entity; the root branch
   (`!world.has_component(entity, child_of_id)` — `propagation.rs:265`) reads the entity's
   `Transform` and writes `GlobalTransform = local.to_affine()` via `set_global_if_changed`
   (`propagation.rs:266-272`). A lone root needs no `Children`; it composes to its own
   `to_affine()` and the descent finds no children. (Proven by the existing `unparented_entity_is_a_root`
   / `root_composition_equals_to_affine` tests.)
2. **BUT the recompute is DIRTY-GATED.** The entity is only *visited* if `collect_dirty`
   collects it, i.e. only if its `Transform.changed_tick.is_newer_than(last_run, this_run)`
   (`propagation.rs:396-424`, archetype gate `:400-403`, per-row tick test `:412-419`). An
   unchanged root is not visited at all (the 0%-overhead property).
3. **The world change-tick bumps ONLY at `Schedule::run` frame start**
   (`bump_change_tick`, `ecs_master.rs:3705-3708`), NOT on a bare `world.run_system(...)`.
   `this_run = world.current_tick()` is read at `propagation.rs:225`; `last_run` is advanced to
   `this_run` at the END of each run (`propagation.rs:370`).
4. **The set-if-neq write gate (`propagation.rs:576-583`) is downstream-only** — it suppresses the
   *GlobalTransform* `changed_tick` when the value is unchanged, but never withholds a value that
   should be written. It does not cause a value lag.
5. **The existing camera tests sidestep all of this:** `camera_drives_render_gpu.rs`'s
   `CameraBundle` is `{ Camera, Projection, GlobalTransform }` with **NO `Transform`**, and it
   writes `GlobalTransform` DIRECTLY (`:203-264`); `propagate_transforms` is never called. So the
   rig is the FIRST consumer to route a camera through propagation.

**THE HAZARD (real, must be resolved).** In the tests/example we drive the systems via bare
`run_system` calls (no `Schedule`), so the tick is NOT bumped between them. If
`orbit_camera_system` writes `Transform` and then `propagate_transforms` runs in the SAME tick
window, the write's `changed_tick == this_run`, the half-open window `(last_run, this_run]`
INCLUDES it on the FIRST propagate run after a bump but can MISS it if a prior propagate already
advanced `last_run` to the same tick — i.e. a static "write once, propagate once, with no
intervening bump" sequence is NOT guaranteed to recompose. This would silently produce a stale
(or identity) `GlobalTransform` → wrong `ViewUniform` → the rig appears not to move the view.

**RESOLUTION (option a — chosen).** Mandate a tick bump between the rig write and propagation in
every test/example driver: call `world.bump_change_tick()` (the same primitive `Schedule::run`
uses at frame start, `ecs_master.rs:3705-3708`) AFTER spawning / AFTER the per-frame rig advance
and BEFORE `propagate_transforms`. This pushes the just-written `Transform` strictly inside the
next `(last_run, this_run]` window, so the lone root IS collected and recomposed THAT run — the
pose written this frame reaches `ViewUniform` this frame, NO one-frame lag. The per-frame order
becomes:
```
[advance rig fields] -> orbit_camera_system -> world.bump_change_tick() -> propagate_transforms -> resolve_active_camera
```
This is strictly the bare-`run_system` analogue of a real `Schedule::run` frame (which bumps once
at frame start and runs all systems in that one window — the writer and propagation land in the
SAME window, which is correct because `last_run` is the PREVIOUS frame's tick). A production
`CameraPlugin` registered into a `Schedule` therefore needs NO bump call — the frame boundary
supplies it; the manual bump is only for the bare-`run_system` test/example drivers.

**Why option a and not the alternatives.**
- *(b) Spawn-so-propagation-covers-it* alone is insufficient: spawning makes the FIRST frame dirty
  (insert tick), but every SUBSEQUENT per-frame `yaw += step` re-write hits the same
  same-window-skip hazard. The bump is needed every frame, not just at spawn.
- *(c) Write `GlobalTransform` directly from the rig* is rejected per D1 (propagation would
  overwrite it; breaks parenting). It also implies `resolve_active_camera` reads a rig-written
  `GlobalTransform` that bypasses the SOLE-writer invariant — a regression of the propagation
  contract. The rig writes LOCAL `Transform` (the correct ECS shape); the bump makes propagation
  observe it deterministically.

A §7 test ("GlobalTransform reflects a just-written Transform after one propagate_transforms for a
parentless camera, given an intervening bump") LOCKS this in.

---

## 1. Goal

Wire a live render-view path: a `CameraRig` (the orbit controller) drives the on-screen
`ViewUniform`, so moving the rig moves what the screen shows. Functionally: the camera can
ORBIT a target and the windowed present + the SDF marcher both follow it. Performance: the
rig is a pure per-camera-entity arithmetic kernel (no allocation, no `dyn`, no `HashMap`,
no branching beyond a pitch clamp and degenerate guards), runs in the existing
single-threaded scene-update window, and adds the new math as alloc-free FMA-free primitives.

---

## 2. Context and constraints

**Affected subsystems**

| Crate | Change | Kind |
|---|---|---|
| `boyko_math` | `Quat::from_mat3`, `Affine3A::look_at_rh` (+ a `Mat3::from_columns` helper) | additive, pure |
| `boyko_scene` | `OrbitCamera` component, `orbit_camera_system`, schedule order note | additive |
| `boyko_rhi_vulkan` | `Scene::set_mvp(&mut self, [u8; SCENE_MVP_BYTES])` | additive, 1 method |
| `boyko_render` | offscreen rig screenshot test + live windowed orbit example + cube mesh | tests/examples only |

**Invariants preserved**
- `ViewUniform::from_camera`'s column convention (`right=+X`, `up=+Y`, `forward=-Z`, basis =
  COLUMNS of `matrix3`) is the contract the rig must feed. `look_at_rh` is designed against it.
- `resolve_active_camera` debug-asserts the camera `GlobalTransform` is **rigid + uniform-scale**
  (`debug_assert_camera_rigid`, camera.rs:495). `look_at_rh` must produce an ORTHONORMAL basis.
- `propagate_transforms` is the SOLE writer of `GlobalTransform` per frame. The rig writes
  `Transform` (local), NOT `GlobalTransform`, so it must run BEFORE propagation.
- `boyko_rhi_vulkan` MUST NOT depend up on `boyko_scene`/`boyko_render` (layering wall —
  confirmed: its only `[dependencies]` are `boyko_rhi` + `boyko_sdf_math`).
- `boyko_math` discipline: FMA-free, exact `sqrt().recip()` (lib.rs:6-30). The new primitives
  follow it (a camera look-at is not on the physics determinism path, but the crate is uniform).

**Target performance metrics**
- `orbit_camera_system`: O(active camera entities) — in practice 1. Per camera: ~3 `sin`/`cos`,
  2 cross products, 3 normalizes, one `Quat::from_mat3` (one `sqrt`, a handful of mul/add).
  Single-digit hundreds of cycles, once per camera per frame. ZERO heap allocations.
  ZERO `dyn`. ZERO `HashMap`/`Vec` in the body.
- `Scene::set_mvp`: a 64-byte `copy_from_slice` into an owned field; no allocation, no GPU call.
- No change to the hot marcher/raster inner loops (they already consume `ViewUniform`/MVP bytes).

---

## 3. Key decisions

### Decision D1 — `OrbitCamera` is a component; `orbit_camera_system` is a PURE pose-deriver

**What.** Add to `boyko_scene::camera` (camera concern):

```rust
/// Orbit-camera RIG: the camera entity's pose is DERIVED from these fields by
/// [`orbit_camera_system`] — the eye orbits `target` on a sphere of radius
/// `distance`, oriented to look AT `target`. The rig is pure state: a caller
/// (an example loop, an input system, an animation) advances `yaw`/`pitch`;
/// the system only re-derives the `Transform`. Principle 0: a component on ECS
/// storage + a system, never a side data store.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// World-space point the camera orbits and looks at.
    pub target: [f32; 3],
    /// Orbit radius (eye distance from `target`). Guarded to a small positive
    /// minimum so a zero/negative radius cannot collapse the look-at (eye==target).
    pub distance: f32,
    /// Azimuth, radians. yaw=0 places the eye on +Z of `target`; +yaw sweeps +Z->+X.
    pub yaw: f32,
    /// Elevation, radians. pitch=0 is level; +pitch raises the eye toward +Y.
    /// CLAMPED to +/-(pi/2 - [`OrbitCamera::PITCH_EPS`]) so the look direction is
    /// never collinear with world-up (which would make the right axis degenerate).
    pub pitch: f32,
}
```

Constructor `OrbitCamera::new(target, distance, yaw, pitch) -> Self` and a const
`PITCH_LIMIT = FRAC_PI_2 - PITCH_EPS` (`PITCH_EPS = 1e-3`), `MIN_DISTANCE = 1e-4`.

The system:

```rust
/// Derives each [`OrbitCamera`] entity's local [`Transform`] from its rig fields
/// (eye on the orbit sphere, looking at `target`). PURE: rig fields -> pose. It does
/// NOT advance `yaw`/`pitch` itself — animation/input is the caller's loop. Runs
/// `.before(propagate_transforms)` so the same-frame `resolve_active_camera`
/// (`.after(propagate_transforms)`) sees the new pose.
pub fn orbit_camera_system(mut rigs: Query<(&OrbitCamera, &mut Transform)>);
```

**Why PURE (rig fields -> pose), not time-driven.** Determinism + testability: a pure function
of the rig fields is trivially asserted (given yaw/pitch/distance, the eye and the look matrix
are fixed). It removes a hidden `Res<Time>` dependency from the scheduler conflict graph and
keeps the orbit ANIMATION in the caller (the windowed example advances `yaw += dt*omega` between
frames; a future input system writes yaw from the mouse). This matches the project's
"capability = component presence" and "decide perf/architecture yourself" memory: the system
is the minimal pure kernel; motion is composed on top.

**Why a `Query<(&OrbitCamera, &mut Transform)>`** (not exclusive `&mut EcsMaster`): the rig
reads one component and writes a DISJOINT component on the SAME entity — no cross-entity
read/write (unlike `propagate_transforms`, which reads a parent row and writes a child row of
the same column and therefore MUST be exclusive). A non-exclusive `SystemParam` system is the
correct, parallel-friendly shape and mirrors `resolve_active_camera`.

**Alternatives rejected.**
- *Bake the orbit animation into the system (advance yaw by dt).* Rejected: couples the system
  to `Res<Time>`, makes it non-deterministic and harder to unit-test, and hard-codes one motion
  policy. The pure form composes with any driver.
- *Write `GlobalTransform` directly from the rig (skip `Transform`/propagation).* Rejected: it
  fights `propagate_transforms` (the sole `GlobalTransform` writer would overwrite it next
  frame for any camera that also has a `Transform`, which `Camera` requires), and it breaks the
  hierarchy story (a rig camera could not be parented). Writing local `Transform` is the
  correct ECS shape; for a ROOT camera `GlobalTransform == Transform.to_affine()` so the pose
  reaches `from_camera` unchanged.

**Trade-off.** The orbit MOTION lives in the caller, so "make the camera orbit" is two lines in
the example loop, not a turnkey component flag. Accepted: that is the price of a deterministic,
testable, policy-free kernel.

---

### Decision D2 — `look_at_rh` in `boyko_math`, ENABLED by a new `Quat::from_mat3`

**What.** Add to `boyko_math`:

```rust
// mat.rs — the missing column constructor (Mat3 is row-major; only from_rows exists today).
impl Mat3 {
    /// Builds a matrix whose COLUMNS are `c0, c1, c2` (transposes into the
    /// row-major storage). The basis convention `ViewUniform::from_camera` reads
    /// is column-major (`matrix3.mul_vec(local_axis)` selects a column), so a
    /// look-at basis is assembled here as columns.
    pub const fn from_columns(c0: Vec3, c1: Vec3, c2: Vec3) -> Self;
}

// quat.rs — the missing rotation-matrix -> quaternion conversion (Shepperd's method).
impl Quat {
    /// Constructs a unit quaternion from a PROPER orthonormal rotation matrix
    /// (`det ~ +1`). Uses the numerically stable largest-diagonal (Shepperd)
    /// branch selection. Bit-determinism: literal `sqrt`, no `rsqrt`.
    /// DEBUG-asserts orthonormality; on a degenerate (non-rotation) input it
    /// returns a normalized best effort rather than NaN.
    pub fn from_mat3(m: Mat3) -> Self;
}

// affine.rs — the new camera-world-transform constructor.
impl Affine3A {
    /// The RIGID camera WORLD transform looking from `eye` at `target` with world
    /// `up` hint, in the right-handed convention `ViewUniform::from_camera` expects:
    /// local right=+X, up=+Y, forward=-Z, stored as the COLUMNS of `matrix3`, so
    /// `self.inverse()` is the standard RH view matrix. Column 2 (camera +Z) =
    /// `normalize(eye - target)` (= -view_forward); column 0 = `normalize(cross(up, +Z))`;
    /// column 1 = `cross(+Z, right)`. GUARDS the degenerate `eye==target` and
    /// `up || forward` cases (substitutes a fallback axis, then re-derives `up`) so it
    /// never yields NaN.
    pub fn look_at_rh(eye: Vec3, target: Vec3, up: Vec3) -> Self;
}
```

**Why a NEW `Quat::from_mat3` is REQUIRED (the DEFECT in the locked D1/D2 — see section 9).** D1 says
the rig writes the camera's local `Transform`. `Transform` is decomposed TRS with a `Quat`
rotation (transform.rs:46-53). A look-at naturally produces a basis MATRIX. Today there is
**NO** `Quat::from_mat3` and **NO** `Transform::from_affine` — so a basis `Mat3` has NO path
into `Transform.rotation`. The enabling primitive is `Quat::from_mat3`. With it:
`orbit_camera_system` builds the basis via `look_at_rh` (or a basis `Mat3`), converts the
linear part to a `Quat`, and writes `Transform { translation: eye, rotation, scale: ONE }`.
`propagate_transforms` then folds that to `GlobalTransform`, and `from_camera` reads the basis
back from the COLUMNS — bit-consistent because `Quat::from_mat3` of `Mat3::from_quat` round-trips
a proper rotation.

**Why `look_at_rh` returns an `Affine3A`** (not just a `Quat`): the math primitive is the full
camera world transform (rotation + eye translation); the rig extracts `.matrix3` for the `Quat`
and `.translation` is the eye. A test can also assert `look_at_rh(...).inverse()` projects
`target` to the screen center directly, without the ECS round-trip. The column convention is
pinned to exactly what `from_camera` consumes (confirmed against camera.rs:304-318).

**Why Shepperd's method for `from_mat3`** (largest-diagonal branch): it is the standard
numerically stable matrix->quaternion conversion (avoids the catastrophic cancellation of the
naive `w = sqrt(1+trace)/2` when `trace ~ -1`). It is what glam/Bevy use under the hood.

**Alternatives rejected.**
- *Add `Transform::from_affine` and decompose the look-at affine into TRS.* Rejected: an affine
  decompose (extract scale, then quaternion) is strictly MORE work than `Quat::from_mat3` on an
  already-orthonormal basis, and `from_affine` is a broader API surface than the rig needs. We
  add the minimal primitive.
- *Store the pose as `GlobalTransform` directly and skip the `Quat`.* Rejected — see D1
  (propagation overwrite, hierarchy break).
- *Reuse glam.* Rejected — the engine is fully in-house (`boyko_math` is the math SDK); adding a
  dependency for one conversion violates the in-house discipline and the determinism rules.

**Trade-off.** `Quat::from_mat3` is a new ~30-line primitive carrying a branch (largest-diagonal
selection). It runs once per camera per frame (cold relative to any inner loop), so the branch
cost is irrelevant; the cost is one-time review of the conversion's correctness (covered by the
round-trip property test in section 7).

---

### Decision D3 — `Scene::set_mvp` is the live on-screen seam (additive, minimal)

**What.** Add ONE method to `boyko_rhi_vulkan::swapchain::Scene`:

```rust
impl Scene {
    /// Overwrites the per-frame vertex push-constant bytes (the MVP). The next
    /// [`Renderer::render_scene_frame`] re-pushes these bytes unconditionally
    /// (swapchain.rs cmd_push_constants in the per-frame record), so a per-frame
    /// update takes effect immediately. The 64 bytes are a column-major 4x4 `f32`
    /// matrix the vertex shader reads at offset 0.
    #[inline]
    pub fn set_mvp(&mut self, mvp: [u8; SCENE_MVP_BYTES]) {
        self.mvp = mvp;
    }
}
```

**Why.** INVESTIGATE finding: `Scene.mvp` is a PRIVATE field, set ONCE via `Scene::new`; there
is NO existing setter (confirmed: no `set_mvp`, no `pub fn set_*` on `Scene`). But
`render_scene_frame` re-pushes `scene.mvp` to the vertex stage at offset 0 on EVERY frame
(swapchain.rs:1356, in `record_scene`, `cmd_push_constants(..., scene.mvp.len(), scene.mvp.as_ptr())`,
unconditional). So a tiny `&mut self` byte-copy setter is the complete and correct live seam — the
renderer already does the re-push. No allocation, no GPU call, no signature change to
`render_scene_frame`.

**O1 — `scene.mvp` is re-pushed at TWO sites (both vertex stage, offset 0, unconditional):**
swapchain.rs:1356 (`record_scene`, called only by `render_scene_frame` at :1103 — the WINDOWED
present path the orbit example drives) AND swapchain.rs:2542 (`record_gbuffer`, called only by
`render_gbuffer_frame` at :2256 — a separate RASTER/G-buffer path, NOT reachable from
`render_scene_frame`). `set_mvp` mutates the single shared `Scene.mvp` field, so it takes effect on
whichever path records NEXT. The #35 example calls `render_scene_frame` -> the 1356 site.

**Alternatives rejected.**
- *Make `Scene.mvp` a `pub` field.* Rejected: leaks the raw 64-byte buffer and its push-constant
  contract into the public API; a named setter documents the "column-major 4x4 at offset 0"
  invariant and keeps the field private.
- *Rebuild the `Scene` each frame with a fresh `mvp_bytes()`.* Rejected: `Scene::new` consumes a
  pipeline + a bound vertex buffer; rebuilding per frame re-creates GPU objects every frame —
  a per-frame allocation/GPU-churn anti-pattern. The setter mutates 64 owned bytes.

**Trade-off.** None of substance — one additive method, no behavior change to existing callers
(`window_present_scene.rs` keeps using the `Scene::new` initial MVP). The hardcoded-MVP tests
are byte-identical.

The bytes are produced by the bridge: `view_proj_columns(view.view_proj) -> [[f32;4];4]`
(model = identity, object at origin), flattened column-major into `[u8; 64]`.

---

### Decision D4 — TWO proofs; the windowed orbit example lives in `boyko_render`

**What.**
- **(a) Offscreen rig screenshot** — `crates/boyko_render/tests/orbit_camera_drives_render_gpu.rs`,
  `#[ignore]` GPU test mirroring `camera_drives_render_gpu.rs` + `p7b_world_ui_screenshot.rs`:
  build an SDF scene + an ECS camera carrying an `OrbitCamera`; drive the LIVE systems
  `orbit_camera_system -> propagate_transforms -> resolve_active_camera`; bridge
  `composite_perspective_from_view`; `run_marcher` -> BMP. Render TWO orbit angles into ONE
  side-by-side image so "the rig rotates the view" is unmistakable. Plus non-ignored CPU setup
  asserts (the eye orbits; the view looks at the target).
- **(b) Live windowed orbit example** — `crates/boyko_render/examples/orbit_cube_window.rs`,
  `#[cfg(windows)]`, mirroring `window_present_scene.rs`: open a window, boot windowed Vulkan,
  build a `Scene` with a CUBE mesh + the existing scene VS/FS, then ORBIT: each frame advance
  `OrbitCamera.yaw`, run the ECS systems -> `ViewUniform` -> `Scene::set_mvp(view_proj_columns)` ->
  `render_scene_frame` -> present. Owner watches the cube orbit live. One readback at a known
  angle -> BMP for orchestrator cross-verification.

**Why `boyko_render` hosts BOTH** (the layering decision). The example needs BOTH `boyko_scene`
(the ECS camera systems -> `ViewUniform`) AND the windowed `Renderer`/`Scene`/`Surface`/
`Swapchain`/`Window` from `boyko_rhi_vulkan`. Confirmed deps:
- `boyko_render` depends on BOTH `boyko_rhi_vulkan` AND `boyko_scene` (Cargo.toml lines 27, 35).
  Its own comment: "the ONLY crate allowed to name both the graphics RHI and the graphics-pure
  ECS core."
- `boyko_rhi_vulkan` depends on NEITHER `boyko_scene` NOR `boyko_render` (only `boyko_rhi` +
  `boyko_sdf_math`). So it CANNOT host an example that uses the camera systems.

Therefore BOTH proofs go in `boyko_render` (tests/ and examples/), which is acyclic and already
sees both halves. The cube mesh + the orbit loop are example/test code (no engine change).

**Why an `examples/` binary for (b), not an `#[ignore]` windowed test.** The owner RUNS it
interactively (`cargo run -p boyko_render --example orbit_cube_window`) and watches the cube
spin; an example binary is the idiomatic "owner runs this" artifact and won't be swept by
`cargo test`. The offscreen (a) is the orchestrator-verified `#[ignore]` TEST (it asserts +
dumps a BMP). The windowed example ALSO does a one-frame readback -> BMP at a known angle so the
orchestrator can cross-verify the windowed path produced the right oblique view without a human
in the loop.

**Trade-off.** Two artifacts to maintain. Accepted: (a) is the machine-checkable I-verify gate;
(b) is the literal on-screen owner oracle — the project's standing owner-eval pattern needs both.

---

## 4. Data structures

```rust
// boyko_scene::camera  (additive)
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: [f32; 3],   // 12 B — world orbit/look-at point
    pub distance: f32,      //  4 B — orbit radius (guarded >= MIN_DISTANCE)
    pub yaw: f32,           //  4 B — azimuth rad (free range; wrapped by caller if desired)
    pub pitch: f32,         //  4 B — elevation rad (clamped +/-PITCH_LIMIT at read)
}
// 24 B, natural f32 alignment. POD, Copy. No hot/cold split needed (all fields are
// read together once per camera per frame). Pin its size with a const assert (house style).
const _: () = assert!(size_of::<OrbitCamera>() == 24);

impl OrbitCamera {
    pub const PITCH_EPS: f32 = 1.0e-3;
    pub const PITCH_LIMIT: f32 = core::f32::consts::FRAC_PI_2 - Self::PITCH_EPS;
    pub const MIN_DISTANCE: f32 = 1.0e-4;
    pub const fn new(target: [f32; 3], distance: f32, yaw: f32, pitch: f32) -> Self;
}
```

**Spawn list is EXPLICIT (O2).** The rig camera is spawned with all five components named
explicitly: `Camera + Projection + OrbitCamera + Transform + GlobalTransform`. The
`#[require]`-chaining dependency is DROPPED from the critical path (it is a pure ergonomic nicety
and its derive support is not load-bearing for #35). Rationale: `propagate_transforms`'s archetype
gate requires BOTH `Transform` AND `GlobalTransform` columns to be present
(`propagation.rs:400-403`) — the existing `CameraBundle` omits `Transform`, so the rig camera MUST
add it explicitly anyway. Listing all five removes any reliance on derive `#[require]` chaining and
makes the dirty-gate prerequisite self-evident at the spawn site. `OrbitCamera` MAY still carry a
`#[require(Transform, GlobalTransform)]` as a convenience, but the spawn list does not depend on it.

No new struct in `boyko_math` (methods on existing `Mat3`/`Quat`/`Affine3A`). No new struct in
`boyko_rhi_vulkan` (one method on `Scene`). The cube mesh is a `const [Vertex; N]` in the example.

---

## 5. Public API (signatures only)

```rust
// boyko_math
impl Mat3   { pub const fn from_columns(c0: Vec3, c1: Vec3, c2: Vec3) -> Self; }
impl Quat   { pub fn from_mat3(m: Mat3) -> Self; }
impl Affine3A { pub fn look_at_rh(eye: Vec3, target: Vec3, up: Vec3) -> Self; }

// boyko_scene::camera  (re-export OrbitCamera + orbit_camera_system from the crate root)
pub struct OrbitCamera { /* section 4 */ }
impl OrbitCamera { pub const fn new(target: [f32;3], distance: f32, yaw: f32, pitch: f32) -> Self; }
pub fn orbit_camera_system(rigs: Query<(&OrbitCamera, &mut Transform)>);

// boyko_rhi_vulkan::swapchain
impl Scene { pub fn set_mvp(&mut self, mvp: [u8; SCENE_MVP_BYTES]); }
```

No internal types leak. No `dyn`. No lifetimes beyond the `Query` param's elided ones.

---

## 6. Algorithms for critical paths

### 6.1 `Affine3A::look_at_rh(eye, target, up)`
Steps (convention pinned to `from_camera`: columns = right, up, +Z; camera looks down -Z):
1. `back = (eye - target)`. If `back.length_squared() < EPS^2` (eye==target), substitute
   `back = (0,0,1)` (a valid default +Z). Else `back = back.normalize()`.
2. `right = cross(up, back)`. If `right.length_squared() < EPS^2` (up || back — the pole), the
   fallback changes ONLY the SOURCE up vector: pick a `fallback_up` orthogonal to `back` (`+X` =
   `Vec3::new(1,0,0)` if `back` is not ~`±X`, else `+Z` = `Vec3::new(0,0,1)`), then recompute
   `right = cross(fallback_up, back)`. The CROSS ORDER IS NEVER REORDERED — it stays
   `right = cross(up_or_fallback, back)` then (step 3) `true_up = cross(back, right)`, so the basis
   chirality (and thus `det(matrix3) ≈ +1`) is identical in the nominal and the fallback case.
   `right = right.normalize()`.
3. `true_up = cross(back, right)` (already unit; normalize for safety). With `right = cross(up, back)`
   and `true_up = cross(back, right)`, the ordered triple `(right, true_up, back)` is right-handed:
   `cross(right, true_up) == back` and `det(from_columns(right, true_up, back)) == +1`.
4. `matrix3 = Mat3::from_columns(right, true_up, back)`; `translation = eye`.

Complexity O(1). Cache: scalar registers only. Branching: two cold guards (eye==target,
pole) — never taken on a valid orbit camera (the pitch clamp keeps `back` off the pole and
`MIN_DISTANCE` keeps eye!=target), so the branch predictor sees them as not-taken. SIMD: N/A
(per-camera scalar). Note `Vec3::normalize` returns `Vec3::ZERO` (not NaN) on a zero input, so
the guards are belt-and-suspenders: even without them, a degenerate camera yields a singular
`matrix3` that `from_camera` catches via `inverse() -> None -> identity view` (camera.rs:324).

### 6.2 `Quat::from_mat3(m)` — Shepperd, EXACT algebraic inverse of `Mat3::from_quat`

**Index convention (PINNED).** `m_ij ≡ m.rows[i].component(j)` = row `i`, column `j`. `Mat3` is
row-major (`rows: [Vec3; 3]`), so `m00 = rows[0].x`, `m01 = rows[0].y`, `m02 = rows[0].z`,
`m10 = rows[1].x`, `m11 = rows[1].y`, `m12 = rows[1].z`, `m20 = rows[2].x`, `m21 = rows[2].y`,
`m22 = rows[2].z`.

**The `Mat3::from_quat` layout this MUST invert (transcribed VERBATIM from mat.rs:96-106).**
For unit `q = (x, y, z, w)`:
```text
m00 = 1 - 2(yy+zz)   m01 = 2(xy - wz)    m02 = 2(xz + wy)
m10 = 2(xy + wz)     m11 = 1 - 2(xx+zz)  m12 = 2(yz - wx)
m20 = 2(xz - wy)     m21 = 2(yz + wx)    m22 = 1 - 2(xx+yy)
```
(`m_ij` index convention as above; `xx=x·x`, `xy=x·y`, `wz=w·z`, …). `Quat::from_mat3` is the
EXACT algebraic inverse of THIS sign layout — not a generic Shepperd from a textbook with a
possibly-different sign convention.

**Off-diagonal identities derived from that layout (the contract).** Subtracting/adding the
symmetric off-diagonal pairs of the layout above:
```text
m21 - m12 = 2(yz+wx) - 2(yz-wx) = 4·w·x        m21 + m12 = 4·y·z
m02 - m20 = 2(xz+wy) - 2(xz-wy) = 4·w·y        m02 + m20 = 4·x·z
m10 - m01 = 2(xy+wz) - 2(xy-wz) = 4·w·z        m10 + m01 = 4·x·y
```
The diagonal gives the trace `t = m00 + m11 + m22 = 3 - 4(xx+yy+zz) = 4·w·w - 1` (using
`xx+yy+zz+ww = 1`), and each diagonal element isolates one squared component, e.g.
`m00 - m11 - m22 = 4·xx - 1`. These six difference/sum identities are what each Shepperd branch
uses; the SIGNS are fixed by the transcribed layout (note the `w`-term order:
`+wx` is on `m21`, `+wy` on `m02`, `+wz` on `m10` — so the POSITIVE `w·k` term is always the
LOWER-left element of its pair, hence `m21-m12 = +4wx`, etc.).

**The four branches (Shepperd largest-diagonal; written out EXACTLY).** Select the branch by the
largest of `{t, m00, m11, m22}` for numerical stability (avoids `sqrt` of a near-zero), where
`t` is the trace. In each branch one component is `s/2` and the other three come from the
identities above divided by `2s`. `q = (x, y, z, w)`:

- **Trace branch** (largest is `t = m00+m11+m22`, i.e. small-angle / `w`-dominant):
  ```text
  s = sqrt(t + 1) · 2          // = 4w
  w = s / 4
  x = (m21 - m12) / s          // = 4wx / 4w = x
  y = (m02 - m20) / s          // = 4wy / 4w = y
  z = (m10 - m01) / s          // = 4wz / 4w = z
  ```
- **x-diagonal branch** (largest is `m00`, i.e. near-180° about X):
  ```text
  s = sqrt(1 + m00 - m11 - m22) · 2   // = 4x
  x = s / 4
  w = (m21 - m12) / s          // = 4wx / 4x = w
  y = (m01 + m10) / s          // = 4xy / 4x = y
  z = (m02 + m20) / s          // = 4xz / 4x = z
  ```
- **y-diagonal branch** (largest is `m11`, i.e. near-180° about Y):
  ```text
  s = sqrt(1 + m11 - m00 - m22) · 2   // = 4y
  y = s / 4
  w = (m02 - m20) / s          // = 4wy / 4y = w
  x = (m01 + m10) / s          // = 4xy / 4y = x
  z = (m12 + m21) / s          // = 4yz / 4y = z
  ```
- **z-diagonal branch** (largest is `m22`, i.e. near-180° about Z):
  ```text
  s = sqrt(1 + m22 - m00 - m11) · 2   // = 4z
  z = s / 4
  w = (m10 - m01) / s          // = 4wz / 4z = w
  x = (m02 + m20) / s          // = 4xz / 4z = x
  y = (m12 + m21) / s          // = 4yz / 4z = y
  ```
The off-diagonal PATTERN per branch is EXACT (not "the off-diagonal sums"): the trace branch and
the chosen-diagonal branch each use the specific DIFFERENCE pairs for the `w`-coupled component and
the specific SUM pairs for the two vector-vector-coupled components, with the signs above. The dev
implements these verbatim. After branch selection, NORMALIZE the result (cheap insurance against
input drift). `debug_assert!` the input is orthonormal (`|det| ≈ 1`, unit & mutually-orthogonal
rows) BEFORE the branch.

Complexity O(1): one `sqrt`, ~10 mul/add, one 4-way branch (cold, once per camera per frame). Cache:
registers only. SIMD: N/A. Determinism: literal `sqrt`/`sqrt().recip()` in the normalize, FMA-free.

### 6.3 `orbit_camera_system`
For each `(rig, transform)`:
1. `pitch = clamp(rig.pitch, -PITCH_LIMIT, +PITCH_LIMIT)`; `dist = rig.distance.max(MIN_DISTANCE)`.
2. `(sp, cp) = (sin(pitch), cos(pitch))`; `(sy, cy) = (sin(yaw), cos(yaw))`.
3. `offset = dist * (cp*sy, sp, cp*cy)`; `eye = target + offset`. (yaw=0,pitch=0 -> eye = target +
   (0,0,dist), camera on +Z looking -Z at target; yaw=pi/2 -> eye on +X; pitch->+ raises toward +Y.)
4. `world = Affine3A::look_at_rh(eye, target, up=(0,1,0))`.
5. `*transform = Transform { translation: world.translation, rotation: Quat::from_mat3(world.matrix3), scale: Vec3::ONE }`.

Complexity O(active cameras) — 1 in practice. Cache: sequential over the (tiny) `OrbitCamera`
column; `&mut Transform` write is one 40-byte store. Branching: one `clamp`, one `max`, the two
cold guards inside `look_at_rh`. SIMD: not worth it at N=1; if many rigs, the column iteration is
already SoA-friendly. ZERO allocation. The `sin`/`cos` are libm scalar calls (cold, once/frame).

**C3 wiring note.** `orbit_camera_system` writes the LOCAL `Transform` only. For
`propagate_transforms` to recompose the lone-root camera's `GlobalTransform` THIS frame (no lag),
a `world.bump_change_tick()` MUST run between this system and `propagate_transforms` in any
bare-`run_system` driver (§0.5). Under a real `Schedule::run` frame the bump is automatic.

### 6.4 Bridge to the windowed MVP (per frame, in the example)
`mvp_cols = view_proj_columns(view.view_proj)` (`[[f32;4];4]`, column-major, model=identity) ->
flatten to `[u8; 64]` (16 LE-f32, column 0 first) -> `scene.set_mvp(bytes)`. O(1), 64-byte copy.
The renderer re-pushes it next `render_scene_frame`.

---

## 7. CPU test matrix (for the tester)

**`boyko_math` (`crates/boyko_math/tests/look_at.rs` + quat/mat3 unit tests):**
- `look_at_rh` basis: from `eye=(0,0,5)`, `target=ORIGIN`, `up=+Y` -> column 2 (`back`) ~ `(0,0,1)`,
  forward (`-back`) points at target; basis orthonormal (`right·up ~ 0`, all unit) and
  right-handed: assert `cross(right, true_up) ~ back` AND `det(matrix3) ~ +1`.
- `look_at_rh` projects target to center: `look_at_rh(eye,target,up).inverse().to_mat4()` (the
  view) times a `proj` maps `target` to NDC ~ origin (xy ~ 0).
- **W1 degenerate-guard handedness:** for `eye==target` (zero `back`) AND `up || forward` (eye
  directly above target — the pole), the result is all-finite (no NaN) AND
  `det(matrix3) ~ +1` (the fallback reuses the same cross order, so chirality is preserved). Assert
  `det ~ +1` on BOTH degenerate cases, not just the nominal one.
- **C1 directional round-trip (NON-symmetric matrix):** let `m_90z` = the matrix of a 90° rotation
  about +Z (off-diagonals visibly asymmetric: `m01 = -1, m10 = +1`). Assert
  `Mat3::from_quat(Quat::from_mat3(m_90z))` equals `m_90z` ELEMENT-BY-ELEMENT within eps (a
  transpose would yield `q⁻¹` and fail). This is a directional check, NOT a transpose-blind
  `m == mᵀ`-tolerant one.
- **C1 ground-truth rotate (hand-computed, NOT a round-trip):** `Quat::from_mat3(m_90z).rotate(v)`
  for `v = (1,0,0)` equals the HAND-COMPUTED `(0,1,0)` (a +90° rotation about +Z sends +X to +Y),
  within eps. A transposed/inverted `from_mat3` would return `(0,-1,0)` and fail. (This also pins
  the `Quat::rotate` active-rotation convention, quat.rs:100-105.)
- **C1 proptest — ALL FOUR Shepperd branches with a DIRECTIONAL check.** For a random unit `Quat q`,
  `Quat::from_mat3(Mat3::from_quat(q))` ~ `q` or `-q` (double cover) within eps. EXTEND to a
  directional ground-truth on each branch: (i) the TRACE branch — a small-angle rotation `q_small`
  (e.g. 5° about an arbitrary axis): assert `from_mat3(from_quat(q_small)).rotate(v) ~ q_small.rotate(v)`
  for a fixed `v`; (ii)-(iv) a NEAR-180° rotation about each of X, Y, Z (179°, to land in the
  x/y/z-diagonal branch respectively): for each, assert `from_mat3(from_quat(q_180k)).rotate(v) ~
  q_180k.rotate(v)` ELEMENT-WISE for a fixed non-axis-aligned `v` — a directional check that
  exercises the chosen diagonal branch's exact off-diagonal sign pattern, not just `|trace|`.
- **W2 `Mat3::from_columns` exact layout:** `from_columns(c0,c1,c2)` produces
  `rows = [(c0.x,c1.x,c2.x), (c0.y,c1.y,c2.y), (c0.z,c1.z,c2.z)]` (the transpose pattern). Mechanical
  guard: `from_columns(c0,c1,c2).mul_vec(unit_axis_i) == c_i` for `i = 0,1,2` (column selection),
  confirming the column placement the basis convention needs.

**`boyko_scene` (`crates/boyko_scene/tests/orbit_camera.rs`):**
- Eye geometry: at `(yaw=0, pitch=0, dist=d)` the derived `Transform.translation` ==
  `target + (0,0,d)`; at `yaw=pi/2` the eye is on the `+X` side (`x~d, z~0`); at `pitch=+PITCH_LIMIT`
  the eye is near `+Y` of target.
- Pitch clamp: `rig.pitch = pi` is clamped to `+PITCH_LIMIT` (eye not at the pole; basis finite).
- Distance guard: `rig.distance = 0.0` clamps to `MIN_DISTANCE` (eye != target; finite pose).
- Look-at correctness via the FULL pipeline (run `orbit_camera_system -> propagate_transforms ->
  resolve_active_camera` on an `EcsMaster` with `ActiveCamera`+`ViewUniform`+
  `TransformPropagationScratch` resources): the resulting `ViewUniform.view_proj` projects
  `rig.target` (homogeneous) to NDC ~ origin (screen center), at multiple yaw/pitch.
- **C3 no-lag propagation (parentless camera):** spawn a lone-root camera (NO `ChildOf`/`Children`)
  with `Camera + Projection + OrbitCamera + Transform + GlobalTransform`; write its `Transform`
  (via `orbit_camera_system` at an oblique pose), call `world.bump_change_tick()`, then ONE
  `propagate_transforms` — assert `GlobalTransform.affine() == Transform.to_affine()` (the lone
  root was recomposed THIS run, no one-frame lag). Then re-write the rig (new yaw), bump, propagate
  again, and assert `GlobalTransform` tracked the SECOND pose (proves it is not a one-shot
  spawn-only recompose). NEGATIVE control: WITHOUT the bump between the second write and propagate,
  document that `GlobalTransform` may NOT update (the same-window-skip hazard of §0.5) — this test
  pins WHY the bump is mandatory.

**`boyko_render` (cross, the NON-ignored setup half of `orbit_camera_drives_render_gpu.rs`):**
- An `OrbitCamera` at an OBLIQUE pose (yaw~40 deg, pitch~25 deg) drives `resolve_active_camera` to a
  `ViewUniform` whose `view_proj` (and therefore `composite_perspective_from_view`'s push
  constants) DIFFER materially from the head-on (yaw=0,pitch=0) pose — assert the eye moved and
  the basis rotated (the rig changes the view). This guards the GPU `#[ignore]` body's premise on
  CPU so a mis-wired rig fails fast without a device.

---

## 8. Multithreading model

- `orbit_camera_system` is a NON-exclusive `SystemParam` system reading `&OrbitCamera` and
  writing `&mut Transform` on the SAME entity — disjoint columns, no cross-entity access. It is
  parallel-safe by the same argument as `resolve_active_camera`; the scheduler's conflict graph
  sees `Write<Transform>` and serializes it only against other `Transform` writers
  (`propagate_transforms`, which the order constraint already places after it).
- No shared mutable state, no atomics, no locks introduced. `OrbitCamera` is POD -> `Send + Sync`.
- The windowed example is single-threaded (one window, one present loop) — no concurrency claims.
- Data-race freedom: the rig touches only `(&OrbitCamera, &mut Transform)` of cameras; the same
  frame's `propagate_transforms` (the sole `GlobalTransform` writer) runs strictly AFTER it, and
  `resolve_active_camera` (the sole `ViewUniform` writer) runs strictly after propagation. The
  write-after-write and read-after-write chains are linearized by the schedule order in D1.

**Schedule order (the load-bearing constraint):**
```
orbit_camera_system  .before(propagate_transforms)
propagate_transforms                                  (writes GlobalTransform)
resolve_active_camera .after(propagate_transforms)    (writes ViewUniform)
```
In the tests/example there is no `Schedule`; systems are run in this exact order via
`EcsMaster::run_system`, with a `world.bump_change_tick()` inserted between the rig write and
propagation so the just-written `Transform` lands inside `propagate_transforms`'s
`(last_run, this_run]` dirty window (§0.5 — the lone-root recompose is dirty-gated at
propagation.rs:396-424; the tick bumps only at frame start, ecs_master.rs:3705-3708). The bare
order is:
```
[advance rig] -> orbit_camera_system -> world.bump_change_tick() -> propagate_transforms -> resolve_active_camera
```
A production `CameraPlugin` (out of #35 scope) registered in a `Schedule` needs NO explicit bump —
`Schedule::run` bumps once at frame start and runs the writer + propagation in that one window
(`last_run` = previous frame). It would add the `.before`/`.after` edges from this section.

---

## 9. DEFECT in the locked decisions (flagged, resolved in-plan)

**DEFECT-1 (D1 x D2 incompatibility, REAL).** D1 mandates the rig writes the camera's local
`Transform`; D2 mandates a `look_at_rh` math primitive. But `Transform.rotation` is a `Quat`
and there is **NO `Quat::from_mat3`** and **NO `Transform::from_affine`** in the codebase
(verified: quat.rs exposes only `new`/`normalize`/`mul`/`rotate`/`conjugate`/`integrate`;
transform.rs has `from_translation`/`from_rotation`/`from_scale`/`to_affine` but no affine->TRS).
A `look_at` that yields a basis `Mat3`/`Affine3A` therefore has NO path into `Transform`.

**Resolution (adopted in D2):** add the minimal enabling primitive `Quat::from_mat3` (Shepperd).
`look_at_rh` returns the camera-world `Affine3A`; the rig converts `world.matrix3` to a `Quat`
and writes `Transform { translation: world.translation, rotation, scale: ONE }`. This is strictly
less new surface than `Transform::from_affine` (no scale-extraction) and keeps the column
convention bit-consistent with `from_camera` (because `Quat::from_mat3` of `Mat3::from_quat`
round-trips a proper rotation — guarded by the section 7 round-trip property test).

**DEFECT-2 (terminology, minor).** The brief says "bridge `composite_from_view`"; both existing
GPU tests call **`composite_perspective_from_view`** (the perspective arm), because the SDF
screenshot scene is perspective. `composite_from_view` selects ortho when `view.fov_y == 0.0`,
which a perspective camera never is — so it would dispatch the same perspective path, but the
plan/tests use `composite_perspective_from_view` explicitly to match the proven harnesses
(camera_drives_render_gpu.rs, p7b_world_ui_screenshot.rs).

**DEFECT-3 (handedness silent-pass, latent).** `debug_assert_camera_rigid` (camera.rs:495)
checks orthonormality (equal-length, mutually-orthogonal rows) but NOT handedness. A transposed
basis (the classic row-major/column-major mix-up — `Mat3` is row-major, `from_rows` only) would
pass it yet invert the rotation. Mitigated by: (a) `Mat3::from_columns` is the explicit
column-placing constructor used by `look_at_rh`, and (b) the section 7 `look_at_rh` test asserts
right-handedness directly (`cross(right, true_up) ~ back`, `det ~ +1`) — not just orthonormality.

**No other defect found.** The D3 (`Scene::set_mvp`) and D4 (host = `boyko_render`) decisions are
correct against the verified deps and the verified per-frame re-push.

---

## 10. Integration

**Modules touched / created.**
- `boyko_math`: `mat.rs` (+`Mat3::from_columns`), `quat.rs` (+`Quat::from_mat3`),
  `affine.rs` (+`Affine3A::look_at_rh`). Re-exports already cover `Affine3A`/`Mat3`/`Quat`.
- `boyko_scene`: `camera.rs` (+`OrbitCamera`, +`orbit_camera_system`); re-export both from the
  crate root (`lib.rs`) next to `Camera`/`resolve_active_camera`.
- `boyko_rhi_vulkan`: `swapchain.rs` (+`Scene::set_mvp`). No other change; `render_scene_frame`
  unchanged (it already re-pushes `scene.mvp`).
- `boyko_render`: NEW `tests/orbit_camera_drives_render_gpu.rs`; NEW
  `examples/orbit_cube_window.rs`. No `src/` change (the `view.rs` bridge is reused as-is).

**Existing-API changes:** NONE that break callers. All additions. `window_present_scene.rs` is
untouched (still uses the `Scene::new` initial MVP). `from_camera`/`resolve_active_camera`/
`propagate_transforms` signatures unchanged.

**Compatibility with Arena/ComponentPool/UnitId:** `OrbitCamera` is a 24-byte POD component
stored in a `ComponentPool` column like any other (small size class; the adaptive chunking
handles it). No special storage. No dense/GPU-resident needs.

---

## 11. Implementation plan (for the developer)

1. **`boyko_math/src/mat.rs`** — add `Mat3::from_columns(c0,c1,c2)` (const; transpose into
   `from_rows`). Unit test: column selection.
2. **`boyko_math/src/quat.rs`** — add `Quat::from_mat3(m)` (Shepperd largest-diagonal,
   `debug_assert` orthonormality, normalize result). Unit + proptest round-trip (section 7).
3. **`boyko_math/src/affine.rs`** — add `Affine3A::look_at_rh(eye,target,up)` (section 6.1, both
   degenerate guards). Test file `boyko_math/tests/look_at.rs` (section 7).
4. **`boyko_scene/src/camera.rs`** — add `OrbitCamera` (section 4, size-pin const assert, `#[require]`),
   `OrbitCamera::new`, consts; add `orbit_camera_system` (section 6.3). Re-export from `lib.rs`.
   Test file `boyko_scene/tests/orbit_camera.rs` (section 7).
5. **`boyko_rhi_vulkan/src/swapchain.rs`** — add `Scene::set_mvp` (section 3, `#[inline]`, doc the
   column-major-at-offset-0 contract). No other change.
6. **`boyko_render/tests/orbit_camera_drives_render_gpu.rs`** — mirror
   `camera_drives_render_gpu.rs`: clone the buffer-layout helpers + `run_marcher` +
   `boot_or_skip`; build `EcsMaster` with `ActiveCamera`/`ViewUniform`/`TransformPropagationScratch`;
   spawn a camera entity with `OrbitCamera` (oblique pose); per pose run
   `orbit_camera_system -> propagate_transforms -> resolve_active_camera` (3x `run_system`),
   bridge `composite_perspective_from_view`, `run_marcher`; render TWO angles, `write_bmp`
   side-by-side; `#[ignore]` the GPU body, add the non-ignored CPU setup asserts (section 7 cross row).
7. **`boyko_render/examples/orbit_cube_window.rs`** — `#[cfg(windows)]`, mirror
   `window_present_scene.rs`: open window, boot windowed Vulkan, build the CUBE `Scene` (section 12),
   then the orbit loop (section 13). Per-frame: advance `OrbitCamera.yaw`, run the 3 systems, read
   `ViewUniform`, `view_proj_columns` -> `[u8;64]` -> `scene.set_mvp`, `render_scene_frame`,
   present. One known-angle readback -> BMP. Graceful-skip + bounded frames + reverse teardown.

---

## 12. The cube mesh (example)

A unit cube centered at the origin, side 1, as a `TriangleList`: 36 vertices (6 faces x 2
triangles x 3), each `Vertex { position: [f32;3], color: [f32;4] }` (the SAME 28-byte `#[repr(C)]`
`Vertex` and the SAME committed scene VS/FS `.spv` `window_present_scene.rs` uses — no new
shader). Per-face distinct color so rotation is visible (e.g. +X red, -X cyan, +Y green, -Y
magenta, +Z blue, -Z yellow). Winding CCW front-facing; the scene pipeline's
`depth_format: Some(Format::D32Sfloat)` (already set in the reference) plus the auto-synced depth
image (`scene.sync_depth`) gives correct occlusion as the cube turns. The mesh is a
`const CUBE: [Vertex; 36]` in the example. Object at origin -> MVP model = identity -> MVP =
`view_proj`.

---

## 13. The windowed orbit loop (example, spelled out)

```text
open Window("orbit_cube", W, H)                       // graceful-skip on Err
boot VulkanContext{ windowed: true }                  // graceful-skip
Surface::new(ctx, hinstance, hwnd)  [unsafe + SAFETY] // graceful-skip
Swapchain::new; map format -> rhi Format              // graceful-skip on unmapped format
Renderer::new
create+map cube VERTEX buffer; copy CUBE in           // [unsafe + SAFETY, copied verbatim]
create scene VS/FS modules; build pipeline (push_constant_bytes = SCENE_MVP_BYTES,
    depth D32Sfloat, vertex_layout = 28B/2attr); destroy shader modules
scene = Scene::new(pipeline, vbuf, 36, identity_mvp_bytes())
create staging readback buffer (W*H*4)

// ECS side (single EcsMaster, built once)
world = EcsMaster::new()
world.insert_resource(ActiveCamera::default())
world.insert_resource(ViewUniform::default())
world.insert_resource(TransformPropagationScratch::default())
spawn camera entity: Camera + Projection::Perspective{ fov, aspect=W/H, near, far }
                     + OrbitCamera::new(target=ORIGIN, distance=4.0, yaw=0, pitch=0.4)
                     + Transform + GlobalTransform   (explicit 5-component spawn list — O2)

for frame in 0..N (bounded, e.g. 360):                // owner can watch a full revolution
    window.pump_events(); window.refresh_size()
    if !window-open: break
    // advance the rig (animation lives HERE, not in the system)
    world.run_system(move |mut q: Query<&mut OrbitCamera>| for (_,r) in q { r.yaw += YAW_STEP; })
    world.run_system(orbit_camera_system)
    world.bump_change_tick()                          // C3: push the Transform write into propagation's dirty window (no one-frame lag)
    world.run_system(propagate_transforms)
    world.run_system(resolve_active_camera)
    let view = *world.resource::<ViewUniform>()
    let cols = view_proj_columns(view.view_proj)      // [[f32;4];4] column-major
    scene.set_mvp(flatten_le(cols))                   // [u8; 64]
    want_rb = (frame == KNOWN_ANGLE_FRAME && extent_stable)
    presented = unsafe { renderer.render_scene_frame(ctx, surface, swapchain,
                            &mut scene, w, h, CLEAR, rb) }    // [SAFETY copied verbatim]
    if want_rb && presented: write_bmp(known_angle.bmp, readback)

assert ctx.debug_state().total() == 0                 // zero validation messages
teardown REVERSE: drop(renderer) -> scene.destroy + destroy buffers -> drop(swapchain)
                  -> drop(surface) -> drop(ctx) -> drop(window)
```

The `YAW_STEP` advance is a closure `run_system` (mirrors `move_camera_to` in
`camera_drives_render_gpu.rs`). The rig system stays PURE; the motion is this one line.

---

## 14. Owner-run commands

**(a) Offscreen rig screenshot (orchestrator verifies on the RTX):**
```powershell
cargo test -p boyko_render --test orbit_camera_drives_render_gpu -- --ignored --test-threads=1 --nocapture
# then BMP -> PNG (PowerShell System.Drawing) from target/screenshots/orbit_camera_*.bmp, eyeball, show owner
```
Non-ignored CPU setup asserts run in the normal suite:
```powershell
cargo test -p boyko_render --test orbit_camera_drives_render_gpu
```

**(b) Live windowed orbit (OWNER runs, watches the cube spin):**
```powershell
cargo run -p boyko_render --example orbit_cube_window --release
# writes target/screenshots/orbit_cube_known_angle.bmp at the known frame for cross-verify
```

**Math/scene CPU gates (no GPU):**
```powershell
cargo test -p boyko_math  --test look_at
cargo test -p boyko_scene --test orbit_camera
```

---

## 15. Edge cases / risks and handling

| Risk | Handling |
|---|---|
| **Gimbal at the poles** (pitch -> +/-pi/2: back || world-up -> right axis = 0 -> NaN) | `pitch` clamped to +/-`PITCH_LIMIT` (=pi/2-1e-3) AT READ in `orbit_camera_system`; `look_at_rh` ALSO carries a pole fallback-axis guard (belt-and-suspenders). |
| **eye == target** (distance 0 -> back = 0 -> normalize -> 0 -> singular) | `distance.max(MIN_DISTANCE)` in the system; `look_at_rh` substitutes a default +Z on a zero `back`. Even unguarded, `from_camera` catches a singular `matrix3` via `inverse() -> None -> identity view`. |
| **Transposed basis silently valid** (DEFECT-3: rigid-assert checks orthonormality not handedness; `Mat3` is row-major) | `Mat3::from_columns` is the explicit column-placer; the `look_at_rh` test asserts handedness (`cross(right, true_up)~back`, `det~+1`), not just orthonormality. |
| **`Quat::from_mat3` numerical blow-up** at trace~-1 | Shepperd largest-diagonal branch (not the naive `sqrt(1+trace)`); proptest round-trip covers all four branches incl. 180 deg rotations. |
| **Windowed: no window / no GPU / no validation SDK / SRGB-only swapchain format** | Every fallible boot step graceful-SKIPs with `eprintln!("SKIP ...")` + `return` (mirrors `window_present_scene.rs`); the example simply exits cleanly. |
| **Windowed crash-proneness (GPU)** | Bounded frame count (N~360, not infinite); reverse-order teardown that waits device-idle (`drop(renderer)`); `#[cfg(windows)]` gate; readback only when extent is stable. Commit only after the owner's visual OK (standing pattern). |
| **`boyko_rhi_vulkan` layering wall** | The orbit example needs `boyko_scene` + the windowed `Renderer`; it lives in `boyko_render` (depends on BOTH), NEVER in `boyko_rhi_vulkan` (depends on neither). `Scene::set_mvp` is added IN `boyko_rhi_vulkan` (no upward dep — it's a pure byte setter). |
| **Swapchain recreated mid-frame** (`render_scene_frame -> Ok(false)`) | Skip the readback that frame (`want_rb && presented`); the depth image auto-syncs (`scene.sync_depth`). Same as the reference. |
| **Rig MVP stale because renderer doesn't re-push** | NOT a risk: confirmed `render_scene_frame` re-pushes `scene.mvp` every frame (swapchain.rs:1356); `set_mvp` before the call takes effect that frame. |

---

## 16. NO new `unsafe` in CPU/ECS code — confirmation

- `Mat3::from_columns`, `Quat::from_mat3`, `Affine3A::look_at_rh`, `OrbitCamera`,
  `orbit_camera_system`, `Scene::set_mvp` are **100% safe Rust** (scalar arithmetic + a field
  byte-copy). No `unsafe` anywhere in the math, the scene system, or the setter.
- The ONLY `unsafe` is in the windowed EXAMPLE, and it is COPIED GPU plumbing from
  `window_present_scene.rs` (Surface creation, `render_scene_frame`, the mapped-buffer vertex
  upload, `scene.destroy`/`destroy_buffer`) — each carries the SAME `// SAFETY:` comment verbatim
  (the window-outlives-surface invariant; the live-ctx/surface/swapchain/scene + host-visible
  staging invariant). No NEW unsafe reasoning is introduced.

---

## 17. Metrics and validation

**Mandatory unit tests:** section 7 in full (math: look_at orthonormality + handedness + target-to-center
+ degenerate guards; quat round-trip proptest all-4-branches; from_columns selection. scene: eye
geometry at yaw/pitch landmarks + pitch clamp + distance guard + full-pipeline look-at-center.
render: oblique-vs-headon view differs on CPU).

**Mandatory property tests:** `Quat::from_mat3` of `Mat3::from_quat` ~ id (double-cover) and the
reverse, over random unit quats / proper rotations.

**Mandatory benchmarks:** none required (the rig is once-per-camera-per-frame, off any inner
loop). OPTIONAL: a criterion bench of `orbit_camera_system` over N=10k synthetic rigs to confirm
linear scaling and zero allocation (the project's scaling-guard habit) — file as a follow-up, not
a gate.

**Mandatory `debug_assert!`:** `Quat::from_mat3` asserts the input is orthonormal (`|det|~1`,
rows unit & orthogonal); `orbit_camera_system` may `debug_assert!(transform finite)` after the
write; `look_at_rh` may `debug_assert!` the output basis is orthonormal & right-handed.

**Owner-eval oracle (standing pattern):** the offscreen BMP (orchestrator -> PNG -> owner) and the
live windowed cube (owner watches) are the visual gates; commit only after the owner's OK.

---

## 18. Open questions — RESOLVED (critic round 1)

All three prior open questions are resolved in-text:

1. **`#[require]` chaining on `OrbitCamera` — RESOLVED (O2).** No longer load-bearing. The spawn
   list is EXPLICIT: `Camera + Projection + OrbitCamera + Transform + GlobalTransform` (§4, §13).
   `#[require]`-chaining is dropped from the critical path (a pure ergonomic nicety). `OrbitCamera`
   may carry `#[require(Transform, GlobalTransform)]` as a convenience, but nothing depends on it.
2. **`up` hint as a constant vs a rig field — RESOLVED.** Hard-coded world-up `(0,1,0)` in
   `orbit_camera_system` for #35 (the orbit camera never rolls). A future free-look/roll camera
   promotes it to an `OrbitCamera.up: [f32;3]` field — explicitly OUT of #35 scope.
3. **Production `CameraPlugin` wiring — RESOLVED (deferred).** Out of #35 scope; the tests/example
   wire the order manually via `run_system` + an explicit `world.bump_change_tick()` between the
   rig write and propagation (§0.5, §8). A follow-up registers `orbit_camera_system` into the
   scene-update set with the `.before(propagate_transforms)` edge; under a `Schedule` the
   frame-start tick bump replaces the manual one.
