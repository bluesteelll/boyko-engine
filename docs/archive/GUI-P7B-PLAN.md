> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# GUI P7b — World-space / diegetic UI: 3D cursor-ray pick + depth-test occlusion

Status: PLAN (implementation-ready, critic round 1 applied). Branch `ecs`. Builds on
P7a (shipped, commit `6518bb5`). This document VALIDATES the locked P7a-substrate
decisions against the real code, fills the integration gaps, and is the step-by-step
impl brief.

All file:line citations below were confirmed against the working tree at plan time.

---

## Critic round 1 — resolutions (all findings ACCEPTED + applied)

The architecture-critic returned CHANGES REQUESTED. Every finding is applied below;
the three open questions were resolved by the orchestrator and folded in. Praised
parts kept intact: the core ray-gen identity, the `+0.5` pixel-center distinction, the
ortho-honesty caveat, the "`UiPickable` on the SCENE entity not the UI root" contract +
its `gate_pick_resolves_to_anchor_root` test, the `resolve_anchor_point` refactor, and
"no new `unsafe`".

| ID | Finding | Resolution (where) |
|---|---|---|
| **C1** | `camera_ray` must NOT rely on a `view.aspect == vp_w/vp_h` invariant for ray direction. | §2.2: `aspect = vp_w / vp_h` derived from the handed viewport extent, mirroring the marcher's `CompositeCamera::Perspective.aspect = w/h` (compute.rs:1080, `coarse_ray` 2924-2928). `view.aspect` is NOT read for the ray. |
| **C2** | Stale-set-bit hazard (gate-5 eviction class) if the occlusion walk is ever skipped. | §2.4 + §5: the occlusion pass ALWAYS walks every `UiWorldAnchor` root every pick-system run and re-derives each root's `UiWorldOccluded` bit unconditionally (enable if occluded, disable otherwise). The "skip when no pickables" opt is DROPPED. Cursor-inactive / no-cursor path still clears (disables) every root's bit + `HoveredWorldEntity = None`. Stated guarantee: **no code path leaves a set `UiWorldOccluded` bit un-revisited.** |
| **W1** | `UiWorldProjection` struct block showed the wrong field set/order. | §2.3: all six fields in real order `screen_x, screen_y, scale, fade, depth, visible` (`visible` STAYS LAST; `depth: f32` inserted before it); `size_of == 24`, `align == 4`; `Default` includes `depth`. Struct block + write-site block made consistent. |
| **W2** | Origin-inside-sphere returned `Some(0.0)` even on a zero/degenerate dir; the "zero dir → None" claim was false. | §2.1: explicit degenerate-ray guard at the TOP of BOTH `ray_sphere` and `ray_aabb` (`if dir.length_squared() <= MIN { return None }`); near-unit `debug_assert!` kept as the caller contract; §6 claim made true. |
| **W3** | Scale extraction was vague ("max-axis element"). | §2.3/§2.4/§6: `s = max(‖col_0‖, ‖col_1‖, ‖col_2‖)` where the column norm of the ROW-MAJOR `Mat3 { rows: [Vec3;3] }` is `‖col_i‖ = (rows[0][i], rows[1][i], rows[2][i]).length()` (NOT max-abs-element); `debug_assert!` the three column norms within eps (uniform-scale contract), mirroring `debug_assert_camera_rigid` (camera.rs:422). Applied to the Sphere radius and the AABB half-extents. |
| **W4** | Cross-check golden under-specified. | §2.7 + test 9: marcher `CompositeCamera::Perspective` built with `aspect = w/h` (matching `host_camera_from_view`, camera_drives_render_gpu.rs:278); feed `px+0.5`/`py+0.5` into `camera_ray`; EPS ~1e-6 (justified); assert ≥1 off-center off-axis pixel in EACH quadrant. |
| **O1** | `WorldPos` anchor self-exclusion. | §2.4 + §6 docs: a `WorldTarget::WorldPos` anchor has NO self-exclusion (`self_target = None`); stated as defensible in the occlusion / `UiPickable` docs. |
| **O2** | A fixed world-unit occlusion bias is scale-meaningless. | §2.4/§6: occluded iff `t < dA * (1.0 - REL_BIAS)` with `REL_BIAS = 1e-3` (relative / scale-invariant); rationale documented. |
| **O3** | Wrong file paths. | §2.4/§7/§11: corrected to `crates/boyko_ui/src/interaction/focus.rs` etc. |

---

## 0. Delta — what P7a provides vs what P7b adds

### P7a already provides (do NOT redesign)
- `UiWorldAnchor` root component (`crates/boyko_ui/src/world/components.rs:71-101`,
  size 56 / align 8). Carries `target: WorldTarget`, `offset: [f32;3]`,
  `depth_test: bool` (STORED, currently UNUSED — P7b consumes it), `scale_mode`,
  fade fields. `#[require(UiWorldProjection)]`.
- `UiWorldProjection` per-frame result (`components.rs:144-166`, size 20 / align 4,
  `set_if_neq` Changed-gate). Author-never-writes. **P7b adds a `depth: f32` field.**
- `HoveredWorldEntity(pub Option<Entity>)` Resource (`components.rs:176-177`) — THE
  seam P7b's pick writes. Already consumed by `ui_world_visibility_system`.
- `UiWorldCulled` (frustum, project-owned) + `UiWorldHidden` (hover, visibility-owned)
  bitset EnableTags (`components.rs:188-200`, `#[component(storage = "bitset")]`,
  O(1) toggle). The layout pass skips a root with EITHER set.
- `ui_world_project_system` (`crates/boyko_ui/src/world/project.rs:200`) — exclusive;
  enumerates anchors, projects, writes `UiWorldProjection` `set_if_neq`, toggles
  `UiWorldCulled`. `project_world_to_screen` (`project.rs:84`) computes `ndc_z` into
  `ProjectedPoint.ndc_z` (`project.rs:66`) but DISCARDS it (never copied into
  `UiWorldProjection`).
- `ui_world_visibility_system` (`crates/boyko_ui/src/world/visibility.rs:74`) — exclusive;
  reads `HoveredWorldEntity`, matches each anchor's `EntityAnchor(target)`, toggles
  `UiWorldHidden` (`visibility.rs:98-102`).
- The layout-skip guard (`crates/boyko_ui/src/layout.rs:289-293`):
  `is_enabled::<UiWorldCulled>` / `is_enabled::<UiWorldHidden>` → `return` before any
  `ComputedRect` write.

### P7b adds
1. `boyko_math`: a `Ray { origin, dir }` type + `ray_sphere` + `ray_aabb` analytic
   intersection (nearest positive `t`).
2. `boyko_scene::camera`: `camera_ray(view, px, py, vp_w, vp_h) -> Ray` — the inverse
   of `project_world_to_screen`, bit-mirroring the marcher's perspective ray-gen.
3. `boyko_ui::world`:
   - `UiPickable { shape: UiPickShape }` component (`UiPickShape = Sphere | Aabb`).
   - `UiWorldOccluded` bitset EnableTag (third layout-skip authority).
   - `ui_world_pick_system` (exclusive) — cursor ray → nearest `UiPickable` hit →
     `HoveredWorldEntity`; ALSO computes per-root depth-test occlusion into
     `UiWorldOccluded` (one bounds iteration; see D5).
   - `UiWorldProjection.depth: f32` field (= `ProjectedPoint.ndc_z`) + asserts 24/4.
4. `boyko_ui::layout`: add `|| occluded` to the skip guard.
5. `boyko_ui` registration/scheduling (host-owned, per P7a's contract).
6. `boyko_render`: a CPU cross-check golden (`camera_ray` ↔ `composite_pixel_ray`).

No new render pipeline code (proven below, INVESTIGATE #3). No new `unsafe` (proven
below, INVESTIGATE #6).

---

## 1. Locked-decision validation + the one contract DEFECT to fix in docs

The seven locked decisions (D1–D5) are sound against the real code. One subtle
**contract hazard** must be stated loudly in the impl, or the feature silently no-ops:

> **DEFECT / HAZARD (documentation-level, not a design defect): `UiPickable` lives on
> the SCENE entity, never on the UI root.** `ui_world_visibility_system` un-hides a root
> ONLY when that root's `WorldTarget::EntityAnchor(target)` equals `HoveredWorldEntity.0`
> (`visibility.rs:95-98`). Therefore the pick MUST report the **scene entity that an
> anchor tracks** — i.e. `UiPickable` + `GlobalTransform` must sit on the same entity
> referenced by some `UiWorldAnchor { target: EntityAnchor(that_entity), .. }`. If an
> implementer puts `UiPickable` on the UI root entity instead, the pick will write that
> root's id into `HoveredWorldEntity`, `visibility.rs:95` (`EntityAnchor` match) will
> never fire, and NOTHING is ever shown.**

This is consistent with D2 ("NOT coupled to physics colliders", "iterated with
`GlobalTransform`") and D4 — the picked entity is the world object, and the anchor
tracks it. The plan's docs + a dedicated test (`gate_pick_resolves_to_anchor_root`)
make the contract explicit. `WorldPos` anchors are intentionally NOT hover-driven
(`visibility.rs:93-97`), matching the "pick a thing in the world" model.

No other defect found. The y-flip, the `ndc = coord/extent*2-1` round-trip, the
`fov_y == 0.0` ortho sentinel, the EnableTag absence-default (`is_enabled` on an
entity lacking the bitset component → `false` = "not occluded", the correct default)
all check out.

---

## 2. Per-file change list (exact signatures, formulas, ordering)

### 2.1 `boyko_math` — ray vocabulary

**New file** `crates/boyko_math/src/ray.rs` (register `pub mod ray;` + re-export in
`crates/boyko_math/src/lib.rs` next to the existing `vec`/`mat` re-exports).

```rust
/// A parametric ray `origin + t * dir` (t >= 0). `dir` is expected normalized for
/// the returned `t` to read as a Euclidean distance, but the intersectors do not
/// require it (they return `t` in `dir`-length units either way).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// World-space ray origin.
    pub origin: Vec3,
    /// Ray direction (normalized for `t` to be a distance).
    pub dir: Vec3,
}

impl Ray {
    /// Constructs a ray from an origin and a direction.
    pub const fn new(origin: Vec3, dir: Vec3) -> Self;
    /// The point at parameter `t`: `origin + dir * t`.
    #[inline]
    pub fn at(self, t: f32) -> Vec3;
}

/// Nearest non-negative intersection `t` of `ray` with the sphere `(center, radius)`,
/// or `None` (miss, the whole sphere behind the origin, or a degenerate `dir`). An
/// origin INSIDE the sphere returns `t == 0.0` (it is "at" the surface from inside —
/// the pick treats an inside hit as a hit at distance 0), EXCEPT on a zero/near-zero
/// `dir`, which returns `None` (the degenerate-ray guard, W2).
pub fn ray_sphere(ray: Ray, center: Vec3, radius: f32) -> Option<f32>;

/// Nearest non-negative intersection `t` of `ray` with the axis-aligned box
/// `[center - half_extents, center + half_extents]`, or `None` (incl. a degenerate
/// `dir`). Origin-inside returns `t == 0.0` (except on a zero/near-zero `dir` → `None`,
/// the W2 guard). Slab method; component divides guarded against a zero `dir`
/// component (an axis-parallel ray that misses returns `None`).
pub fn ray_aabb(ray: Ray, center: Vec3, half_extents: Vec3) -> Option<f32>;
```

**Math — `ray_sphere`** (geometric form, branch-light). `RAY_DIR_MIN_SQ: f32 = 1.0e-12`
(a tiny eps² below which `dir` is treated as degenerate):
```
if ray.dir.length_squared() <= RAY_DIR_MIN_SQ -> None   // W2: degenerate-ray guard (release, load-bearing)
oc      = ray.origin - center
b       = oc · ray.dir          // dir assumed unit; see normalization note
c       = oc·oc - radius*radius
disc    = b*b - c               // assumes |dir| == 1; for general dir use a = dir·dir
if disc < 0 -> None             // miss
sq      = disc.sqrt()
t0      = -b - sq               // near root
t1      = -b + sq               // far root
if t0 >= 0 -> Some(t0)          // ahead, outside
if t1 >= 0 -> Some(0.0)         // origin inside the sphere -> distance 0
None                            // both behind -> miss
```
The **release** degenerate guard is load-bearing: it makes the §6 / test-5 "zero dir →
None" claim TRUE (without it, an origin-inside-sphere with a zero `dir` would return
`Some(0.0)` — the bug the critic found, W2). The near-unit `debug_assert!((ray.dir.
length_squared() - 1.0).abs() < 1e-3)` is kept as the CALLER CONTRACT (the pick always
passes a normalized `dir` from `camera_ray`), but the release guard is what guarantees
no spurious `Some` on a degenerate ray. (If a general-`dir` variant is ever needed,
divide by `a = dir·dir` — not needed for P7b.)

**Math — `ray_aabb`** (slab method, the existing engine convention is finite-only).
`RAY_DIR_MIN_SQ: f32 = 1.0e-12` (shared with `ray_sphere`):
```
if ray.dir.length_squared() <= RAY_DIR_MIN_SQ -> None   // W2: degenerate-ray guard (release, load-bearing)
inv  = 1/dir component-wise   (a zero component -> ±inf, handled by the min/max)
lo   = (center - half_extents - origin) * inv
hi   = (center + half_extents - origin) * inv
tmin = max(min(lo.x,hi.x), min(lo.y,hi.y), min(lo.z,hi.z))
tmax = min(max(lo.x,hi.x), max(lo.y,hi.y), max(lo.z,hi.z))
if tmax < 0 || tmin > tmax -> None     // box behind, or no slab overlap (miss)
if tmin >= 0 -> Some(tmin)             // entry ahead
Some(0.0)                               // origin inside -> distance 0
```
The W2 guard is the FIRST step and is load-bearing in release: a zero/near-zero `dir`
returns `None` BEFORE the slab math, so an origin-INSIDE-box with a degenerate `dir`
returns `None` (not `Some(0.0)`). A degenerate ray that slips a finite-but-tiny `dir`
past the guard still yields non-finite/huge `inv` → `tmax < tmin` → `None` on an
axis-parallel miss; an exact axis-parallel-through-the-box still intersects via the
other two finite slabs. `debug_assert!(half_extents.x >= 0.0 && ..)`. Keep the near-unit
`debug_assert!` on `dir` here too (caller contract), same as `ray_sphere`.

`#[inline]` on `Ray::at` (trivial cross-crate); intersectors get plain `#[inline]`
(small, called per-bound from another crate, must be visible to LTO). No
`#[inline(always)]` (unmeasured; principle 7).

Vocabulary already present and reused: `Vec3::{new, dot, length_squared, length,
normalize, is_finite}` (`vec.rs:171-233`), `Sub`/`Mul<f32>`/`Add` for `Vec3`
(`vec.rs:299-336`), `f32::min/max/clamp`. No new `Vec3` methods required.

### 2.2 `boyko_scene::camera` — `camera_ray`

**Add to** `crates/boyko_scene/src/camera.rs` (a free fn near `ViewUniform`):

```rust
/// The world-space cursor ray for logical pixel `(px, py)` at logical viewport
/// extent `(vp_w, vp_h)` under `view` — the exact inverse of
/// [`project_world_to_screen`](../../boyko_ui/...) and a bit-mirror of the SDF
/// marcher's perspective ray-gen (`composite_ray`).
///
/// PERSPECTIVE (`view.fov_y != 0.0`): origin = eye, dir = normalized
/// `forward + right*sx + up*sy`, with `aspect = vp_w / vp_h` DERIVED FROM THE HANDED
/// VIEWPORT EXTENT (C1) — NOT `view.aspect`. This bit-mirrors the marcher, whose
/// `CompositeCamera::Perspective.aspect` is set to `w/h` from the push constants
/// (`compute.rs:1080`), so the pick ray and the marcher pixel ray agree regardless of
/// any `view.aspect` staleness. ORTHOGRAPHIC (`view.fov_y == 0.0`): best-effort from
/// the view basis; see the marcher-ortho-fixture caveat in §2.2 / §5 (the marcher's
/// ortho arm uses FIXED legacy constants, NOT the camera).
///
/// `px`/`py` are CONTINUOUS logical-pixel samples (a cursor position, NOT a pixel
/// center): `ndc_x = px / vp_w * 2 - 1` with NO `+0.5`. Passing `px + 0.5` for an
/// integer pixel `px` reproduces the marcher's pixel-CENTER ray exactly (the
/// cross-check golden uses that to compare apples to apples).
#[inline]
pub fn camera_ray(view: &ViewUniform, px: f32, py: f32, vp_w: f32, vp_h: f32) -> Ray;
```

**Math — PERSPECTIVE arm** (mirrors the marcher's `composite_ray` / `coarse_ray`
perspective arm, `crates/boyko_rhi_vulkan/src/compute.rs:2918-2937`, substituting
continuous `px` and `ViewUniform` lanes for the integer pixel + `CompositeCamera`
payload). **`aspect` is derived locally as `vp_w / vp_h` (C1), NOT read from
`view.aspect`** — the marcher's payload `aspect` is itself `w/h` (compute.rs:1080), so
deriving it here from the same viewport extent is the faithful mirror and is immune to
a stale `view.aspect`:
```
aspect = vp_w / vp_h                        // C1: derive from the handed extent, NOT view.aspect
ndc_x = (px / vp_w) * 2 - 1                 // NO +0.5 (continuous cursor sample)
ndc_y = -((py / vp_h) * 2 - 1)              // y-flip, identical to the marcher
tan_half = (view.fov_y * 0.5).tan()
sx = ndc_x * aspect * tan_half              // aspect = vp_w/vp_h (NOT view.aspect)
sy = ndc_y * tan_half
f = view.cam_forward.xyz(); r = view.cam_right.xyz(); u = view.cam_up.xyz()
dir = (f + r*sx + u*sy).normalize()         // Vec3::normalize (guarded recip-sqrt)
origin = view.camera_pos.xyz()
Ray { origin, dir }
```
(Do NOT add or rely on any `view.aspect == vp_w/vp_h` invariant for the ray direction.
`vp_w`/`vp_h` are the logical viewport extent the caller already hands `camera_ray`;
the same pair feeds the marcher's `w`/`h`.)
Note: `Vec3::normalize` (`vec.rs:226`) guards a zero `dir` → `Vec3::ZERO` (the
marcher's raw `sqrt`+divide does NOT). For a valid camera `dir` is never zero, so on
the pixels the cross-check golden exercises they agree to f32 epsilon; the guard only
diverges on a degenerate camera, where the marcher would emit a non-finite ray anyway.
**This divergence is documented, not a bug** (the pick wants robustness; the marcher
wants GPU bit-parity). The cross-check golden epsilon is chosen to tolerate it (it
tests valid forward cameras only).

**Math — ORTHOGRAPHIC arm (best-effort, `view.fov_y == 0.0`):**
The marcher's ortho arm uses fixed `SDF_HALF_EXTENT`/`SDF_CAM_Z` constants
(`compute.rs:1314-1319`), NOT the camera — so a marcher-accurate ortho pick is
impossible from `ViewUniform` alone. P7b's ortho ray is camera-driven (correct for a
camera-driven ortho raster path, mismatched with the marcher fixture):
```
ndc_x = (px / vp_w) * 2 - 1
ndc_y = -((py / vp_h) * 2 - 1)
// ortho half-extents are not stored as scalars on ViewUniform; reconstruct the
// ray plane from inv_view + the projection is out of scope for the pick math core.
// Best-effort: origin offset along the camera right/up by ndc * (unit extent),
// dir = forward. The caller is documented to treat ortho pick as approximate.
origin = eye + r*ndc_x + u*ndc_y
dir    = f
```
**Caveat stated in the doc-comment and §5:** ortho pick is approximate (unit extent
placeholder) and does NOT match the marcher's ortho fixture. P7b targets PERSPECTIVE
(the screenshot scene is perspective). A future phase that adds ortho half-extents to
`ViewUniform` upgrades this arm. `boyko_scene` gains a use of `boyko_math::ray::Ray`
(already depends on `boyko_math` — `camera.rs:37`), NOT on the vulkan backend.

`#[inline]` (trivial, cross-crate, must be LTO-visible).

### 2.3 `boyko_ui::world::components` — new components + the `depth` field

**Add to** `crates/boyko_ui/src/world/components.rs`:

```rust
/// The shape of a [`UiPickable`] bound, in the entity's LOCAL frame (transformed
/// by its `GlobalTransform` at pick time). `#[repr(C)]` POD enum.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiPickShape {
    /// A sphere of `radius`, centered at the entity's world translation.
    Sphere { radius: f32 },
    /// An axis-aligned box of `half_extents`, centered at the world translation.
    Aabb { half_extents: [f32; 3] },
}

/// Marks a SCENE entity (NOT a UI root) as cursor-ray pickable for world-space UI.
///
/// The pick ([`ui_world_pick_system`]) ray-tests this bound and writes the nearest
/// hit into [`HoveredWorldEntity`]; the existing
/// [`ui_world_visibility_system`](super::visibility::ui_world_visibility_system)
/// then shows the [`UiWorldAnchor`] root whose `EntityAnchor(target)` equals the
/// picked entity. THEREFORE `UiPickable` MUST sit on the entity an anchor tracks,
/// alongside a `GlobalTransform`. NOT coupled to `boyko_physics` colliders
/// (`boyko_ui` does not depend on `boyko_physics` — Principle 0: the pick bound is
/// a first-class UI component, not a borrowed physics primitive).
///
/// The shape is local; the pick applies the `GlobalTransform`'s translation + a UNIFORM
/// scale to the bound. The scale is the conservative per-axis bound `s = max(‖col_0‖,
/// ‖col_1‖, ‖col_2‖)` of the transform's linear part (the COLUMN norms of the
/// ROW-MAJOR `Mat3 { rows: [Vec3;3] }`: `‖col_i‖ = (rows[0][i], rows[1][i],
/// rows[2][i]).length()` — NOT a max-abs element, W3). A uniform-scaled target has
/// equal column norms (`debug_assert!`ed within eps, mirroring `debug_assert_camera_
/// rigid`, camera.rs:422); a non-uniform-scaled target conservatively uses the largest
/// (the bound never shrinks below the true shape on any axis). True OBB picking is out
/// of scope.
///
/// O1: a `UiWorldAnchor` whose `target` is `WorldTarget::WorldPos` has NO self-exclusion
/// in the occlusion pass (`self_target = None`) — there is no scene entity to exclude,
/// so any pickable in front of the fixed point may occlude its label. Defensible: a
/// `WorldPos` label is "a point in the air", not "a label on object X".
///
/// `#[repr(C)]`, POD `Copy`, its own SoA column.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiPickable {
    /// The pick bound, in the entity's local frame.
    pub shape: UiPickShape,
    /// Optional pick-layer mask (a bitset of layers this target lives on); the
    /// pick's `layer_mask` ANDs against it. `u32::MAX` (default) = "all layers".
    pub layers: u32,
}

impl Default for UiPickable {
    /// A unit-sphere pickable on all layers (the simplest "a clickable point").
    fn default() -> Self {
        Self { shape: UiPickShape::Sphere { radius: 0.5 }, layers: u32::MAX }
    }
}

/// The DEPTH-TEST occlusion EnableTag, OWNED by [`ui_world_pick_system`]'s
/// occlusion pass (D5). A bitset tag (`#[component(storage = "bitset")]`): O(1)
/// toggle, no archetype migration — IDENTICAL backend to [`UiWorldCulled`] /
/// [`UiWorldHidden`].
///
/// Set on a `depth_test == true` world-anchor root whose anchor point is occluded
/// by a nearer [`UiPickable`] surface (a CPU proxy against the SAME bounds the pick
/// ray-tests — see the P7b plan; this is NOT a GPU depth-buffer test). The layout
/// pass skips a root with this bit set. Independent of `UiWorldCulled` /
/// `UiWorldHidden` (a third authority over a distinct bit, so none race).
/// `depth_test == false` roots are NEVER set (always-on-top overlay).
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct UiWorldOccluded;
```

**Modify `UiWorldProjection`** (`components.rs:144-166`): insert the `depth: f32` field
BEFORE the existing `visible: bool` (which STAYS LAST), and bump the asserts. All SIX
fields, in real declaration order:
```rust
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiWorldProjection {
    /// Screen-space x (logical px) of the projected anchor point.
    pub screen_x: f32,
    /// Screen-space y (logical px) of the projected anchor point.
    pub screen_y: f32,
    /// Distance-driven UI scale (M1).
    pub scale: f32,
    /// Distance-driven fade in `[0, 1]` (M1).
    pub fade: f32,
    /// NDC z of the anchor point (= `ProjectedPoint.ndc_z`, previously discarded, W1).
    /// Default `1.0` = the far plane / "nearest-neutral": a far-plane / no-op depth
    /// that never reads as "in front" of real geometry, so a default-constructed
    /// projection cannot spuriously win a future z-order. Forward-looking: a future
    /// GPU-depth UI pass / a CPU z-order can consume it. Unread by the P7a/P7b CPU core
    /// beyond being stored.
    pub depth: f32,
    /// In-frustum visibility (STAYS LAST — author-never-writes).
    pub visible: bool,
}
const _: () = assert!(size_of::<UiWorldProjection>() == 24); // was 20
const _: () = assert!(align_of::<UiWorldProjection>() == 4);

// `depth` defaults to the far-plane-neutral `1.0`, so `Default` is hand-written rather
// than derived (a derived `Default` would give `depth: 0.0` = the NEAR plane, which a
// future z-order would read as "in front of everything" — the wrong neutral).
impl Default for UiWorldProjection {
    fn default() -> Self {
        Self {
            screen_x: 0.0,
            screen_y: 0.0,
            scale: 1.0,
            fade: 1.0,
            depth: 1.0,   // far-plane / nearest-neutral (W1)
            visible: false,
        }
    }
}
```
`visible: bool` stays last; inserting `depth: f32` before it keeps natural f32 packing
(4×f32 + 1×f32 = 20, + bool + 3 pad = 24). The struct block and the write-site block
(below) are now consistent: both list `depth` immediately before `visible`. NOTE: if the
existing `UiWorldProjection` already hand-rolls (or derives) `Default` with different
`scale`/`fade` neutrals, preserve those existing neutrals and ONLY add `depth: 1.0` —
the load-bearing change is `depth`'s far-plane-neutral value, not the others.

**Modify the `ui_world_project_system` write site** (`project.rs:287-293`): copy
`pp.ndc_z` into the new field so `depth` is populated. Field order matches the struct
(EDIT 8): `depth` immediately before `visible`:
```rust
let projection = UiWorldProjection {
    screen_x: pp.x,
    screen_y: pp.y,
    scale,
    fade,
    depth: pp.ndc_z,   // NEW: the previously-discarded ndc_z (W1)
    visible: pp.visible,
};
```
(And `mark_invisible` at `project.rs:317-324` leaves `depth` at its prior value via the
read-modify-write — fine; an invisible root is skipped regardless.)

### 2.4 `boyko_ui::world::pick` — the pick + occlusion system

**Add to** `crates/boyko_ui/src/world/` (new file `pick.rs`, re-exported in
`world/mod.rs:35-40` block alongside the others). One exclusive system carrying BOTH
the pick AND the occlusion (D5: one bounds iteration, the pick already has the ray
machinery + the bounds enumeration):

```rust
/// The cursor-ray pick + depth-test occlusion system (P7b), exclusive.
///
/// Reads `PhysicalInput` + `UiViewport` + `ViewUniform`. If the cursor is inactive
/// (outside / unfocused) it writes `HoveredWorldEntity(None)` (set-if-changed) AND
/// walks EVERY `UiWorldAnchor` root to DISABLE (clear) its `UiWorldOccluded` bit, then
/// returns (C2: the clear-all path leaves no stale set bit). Otherwise:
///  1. PICK: builds the cursor ray, ray-tests every `UiPickable` + `GlobalTransform`
///     entity, picks the NEAREST positive-`t` hit (layer-masked), and writes its
///     entity into `HoveredWorldEntity` (set-if-changed); no hit -> `None`.
///  2. OCCLUSION: walks EVERY `UiWorldAnchor` root EVERY run and re-derives its
///     `UiWorldOccluded` bit UNCONDITIONALLY. For a `depth_test == true` root it casts
///     eye->anchor-point and ENABLES the bit iff some `UiPickable` surface (excluding
///     the anchor's own `EntityAnchor` target) is hit at `t < dA * (1 - REL_BIAS)`,
///     else DISABLES it. A `depth_test == false` root is always DISABLED. Because the
///     bit is re-derived (enable-or-disable) for every root on every run, NO code path
///     can leave a set `UiWorldOccluded` bit un-revisited (C2 — covers pickable-despawn,
///     cursor-inactive, and a `depth_test` flip; the gate-5 eviction class).
///
/// Schedule: `.after(resolve_active_camera)` (fresh `ViewUniform`),
/// `.after(propagate_transforms)` (fresh `GlobalTransform`),
/// `.before(ui_world_visibility_system)` (it consumes `HoveredWorldEntity`).
#[allow(clippy::needless_pass_by_ref_mut)] // enable/disable/is_enabled are &mut, opaque to clippy
pub fn ui_world_pick_system(world: &mut EcsMaster);
```

**Algorithm (steps + complexity + cache + branching):**

Let `P` = number of `UiPickable` entities, `R` = number of `UiWorldAnchor` roots
(both O(tens) in practice — world UI is sparse).

1. Snapshot `view = *world.resource::<ViewUniform>()`, `viewport =
   *world.resource::<UiViewport>()`, `physical = world.resource::<PhysicalInput>().clone()`
   (mirror `focus.rs:152-157`; `PhysicalInput` is `Clone`, `#[repr(C, align(64))]`).
2. `cursor_active = physical.cursor_inside && physical.window_focused`
   (`crates/boyko_ui/src/interaction/focus.rs:158`). If `!cursor_active`:
   `set_hovered_if_changed(world, None)`, then **walk EVERY `UiWorldAnchor` root and
   `world.disable::<UiWorldOccluded>(root)` unconditionally** (C2 — no skip-when-no-
   pickables shortcut; the walk must clear every root's bit so a bit set on a prior
   active frame cannot survive the cursor leaving the window), then `return`. O(R) ≈ tens
   roots; `disable` writes nothing when the bit is already clear, so the inactive path
   stays cheap.
3. `scale = if viewport.scale_factor > 0.0 { viewport.scale_factor } else { 1.0 }`;
   `cursor = [(physical.cursor_pos[0] / scale as f64) as f32,
   (physical.cursor_pos[1] / scale as f64) as f32]` (LOGICAL px,
   `crates/boyko_ui/src/interaction/focus.rs:170-184`).
4. `ray = camera_ray(&view, cursor[0], cursor[1], viewport.width, viewport.height)`.
5. PICK: `let pickables = world.query_entities(&[UiPickable::component_id()]);`
   For each `e`: `let Some(pk) = world.get_component::<UiPickable>(e).copied() else
   continue;` `let Some(gt) = world.get_component::<GlobalTransform>(e).copied() else
   continue;` (a `UiPickable` without a `GlobalTransform` is a setup error →
   `debug_assert!` + skip). Layer gate: `if pk.layers & layer_mask == 0 { continue; }`
   (default mask = `u32::MAX`). Compute world center = `gt.translation()`, and the
   uniform scale `s` as the CONSERVATIVE column-norm bound (W3):
   ```
   let m = gt.affine().matrix3;                  // ROW-MAJOR Mat3 { rows: [Vec3;3] }
   let r = &m.rows;
   let c0 = Vec3::new(r[0].x, r[1].x, r[2].x).length();   // ‖col_0‖
   let c1 = Vec3::new(r[0].y, r[1].y, r[2].y).length();   // ‖col_1‖
   let c2 = Vec3::new(r[0].z, r[1].z, r[2].z).length();   // ‖col_2‖
   debug_assert!(within_eps(c0, c1) && within_eps(c0, c2),
       "UiPickable target must be uniform-scaled");        // mirrors debug_assert_camera_rigid (camera.rs:422)
   let s = c0.max(c1).max(c2);                              // conservative uniform bound (NOT max-abs element)
   ```
   Ray-test by shape: `ray_sphere(ray, center, radius * s)` or
   `ray_aabb(ray, center, Vec3::new(half_extents) * s)`. Track `(best_t, best_e)` = the
   `min` over positive `t`. Result = `Some(best_e)` or `None`.
   `set_hovered_if_changed(world, result)`.
   - Complexity O(P), one linear `query_entities` Vec + per-entity random `get_component`
     (sparse-map lookup — the same access pattern `project`/`visibility` already make,
     `project.rs:233`/`visibility.rs:90`). Branching: one shape `match` + one positive-`t`
     compare per entity. No allocation beyond the `query_entities` Vec (§5).
6. OCCLUSION (ALWAYS walks every root — C2): `let roots = world.query_entities(
   &[UiWorldAnchor::component_id()]);`. For EACH `root`: `let Some(anchor) =
   world.get_component::<UiWorldAnchor>(root).copied() else continue;`. The bit is
   re-derived (enable-or-disable) on EVERY root EVERY run; there is no path that leaves
   a set bit un-revisited.
   - If `!anchor.depth_test`: `world.disable::<UiWorldOccluded>(root)` (always-on-top
     overlay); continue.
   - Early-out (still re-derives the bit, never skips it): if the root's
     `UiWorldProjection.visible == false` it is already culled →
     `world.disable::<UiWorldOccluded>(root)` and continue (this DISABLES the bit — it
     is a re-derivation to the "not occluded" value, not a skip; a behind-eye anchor is
     never occluded-tested with a meaningless ray, but its bit is still cleared).
   - Resolve the anchor point IDENTICALLY to `project.rs:247-269` via the shared
     `resolve_anchor_point` helper (§7): `base =` match `WorldTarget::WorldPos(p) => p`
     | `EntityAnchor(t) =>` (`get_component::<GlobalTransform>(t)` -> translation, or —
     if dangling — `world.disable::<UiWorldOccluded>(root)` and continue; treat as not
     occluded, NEVER leave a stale set bit); `point = base + anchor.offset`.
   - `eye = view.camera_pos.xyz()`; `to_anchor = point - eye`; `dA = to_anchor.length()`;
     `occ_ray = Ray::new(eye, to_anchor.normalize())` (a zero `dA` — anchor AT the eye —
     yields a degenerate dir → `ray_*` return `None` via the W2 guard → not occluded;
     `disable` the bit).
   - Self-exclusion (O1): `let self_target = match anchor.target { WorldTarget::
     EntityAnchor(t) => Some(t), WorldTarget::WorldPos(_) => None };` — an `EntityAnchor`
     excludes its own tracked surface; a `WorldPos` anchor has NO self-exclusion
     (`None`), so any pickable in front of the fixed point may occlude its label.
   - Re-walk the CACHED `pickables` Vec from step 5 (do NOT re-query). For each `e` with
     `Some(e) != self_target`, transform its bound by the SAME column-norm scale `s`
     (W3, exactly as step 5) and `if let Some(t) = ray_test(occ_ray, ...)` with
     `t < dA * (1.0 - REL_BIAS)` → `occluded = true`; **break** (first occluder suffices).
   - `if occluded { world.enable::<UiWorldOccluded>(root) } else {
     world.disable::<UiWorldOccluded>(root) }` — every branch writes the bit; none leaves
     it un-revisited (C2).
   - Complexity O(R·P) worst case; R,P ~ tens → low hundreds of ray-AABBs/frame, off any
     per-entity hot loop. Early-break on the first occluder → common case O(R + hits).
     Cache: the `pickables` Vec is reused (no re-alloc); per-entity `get_component`
     random as above. `enable`/`disable` to the already-correct value writes nothing
     (O(1) bit-test), so the still-cursor path stays cheap despite the unconditional walk.

OCCLUSION BIAS (O2 — RELATIVE / scale-invariant): occluded iff
`t < dA * (1.0 - REL_BIAS)` with `const REL_BIAS: f32 = 1.0e-3`. A fixed world-unit
bias is meaningless at a different world scale (1e-3 units is huge in a millimeter scene,
nothing in a kilometer scene); the relative bias scales with the eye→anchor distance, so
the slop is always ~0.1% of the label range. (Equivalent robust form if a tiny absolute
floor is wanted near `dA == 0`: `t < dA - max(ABS_FLOOR, dA * REL_BIAS)`; for P7b the
pure-relative form suffices since the `dA == 0` case is already handled by the W2 guard.)

**`set_hovered_if_changed`** is a private helper mirroring the `set_if_neq` discipline
the rest of the world systems use:
```rust
#[inline]
fn set_hovered_if_changed(world: &mut EcsMaster, value: Option<Entity>) {
    let r = world.resource_mut::<HoveredWorldEntity>();
    if r.0 != value { r.0 = value; }   // HoveredWorldEntity is PartialEq; the
                                       // visibility system has its OWN Changed-gate,
                                       // so a redundant write is harmless but we gate
                                       // anyway to keep the resource's change-tick clean.
}
```

`#[inline]` only on the trivial helpers; the system body itself takes NO inline
attribute (it is a scheduled fn, called once/frame — inlining it is meaningless and
would bloat the caller; principle 7).

### 2.5 `boyko_ui::layout` — extend the skip guard (INVESTIGATE #1)

**Edit** `crates/boyko_ui/src/layout.rs`:
- Import (line ~66): add `UiWorldOccluded` to
  `use crate::world::components::{UiWorldAnchor, UiWorldCulled, UiWorldHidden,
  UiWorldProjection};` → `{..., UiWorldHidden, UiWorldOccluded, UiWorldProjection};`.
- `layout_root` skip guard (`layout.rs:289-293`), insert one read + extend the OR:
```rust
let culled = world.is_enabled::<UiWorldCulled>(root);     // L289 (unchanged)
let hidden = world.is_enabled::<UiWorldHidden>(root);     // L290 (unchanged)
let occluded = world.is_enabled::<UiWorldOccluded>(root); // NEW
if !proj.visible || culled || hidden || occluded {        // EDIT: + `|| occluded`
    return;
}
```
This reuses the IDENTICAL `is_enabled::<T>(root)` mechanism (confirmed: `enable` /
`disable` / `is_enabled` on `&mut EcsMaster`, `enable_tag_api.rs:87/95/113`); a
skipped root writes no `ComputedRect` for itself or its subtree (the `return`
precedes `write_rect` at `layout.rs:305` and `position_node` at `layout.rs:306`), so
"occluded" == "not laid out" == "not emitted" (INVESTIGATE #3, proven below).

### 2.6 `boyko_ui` registration / scheduling (INVESTIGATE #4)

**Finding:** `UiPlugin::build` (`crates/boyko_ui/src/plugin.rs:84-128`) does NOT
register world components or resources — the world UI is **host-owned** (registered +
scheduled by the host app/test directly), matching P7a's documented contract
(`world/mod.rs:22-29`). Components self-register lazily via `#[derive(Component)]`
(`T::component_id()`); resources are `world.insert_resource(...)`'d by the host.

**Registration edits:**
- `crates/boyko_ui/src/world/mod.rs:35-40` — extend the `pub use components::{...}` to
  export `UiPickable, UiPickShape, UiWorldOccluded` and `pub use pick::ui_world_pick_system;`
  (mirror the existing `pub use project::{...}` / `pub use visibility::{...}` lines).
  Add `mod pick;` next to `mod project;`/`mod visibility;`.
- Host wiring (the reference is the P7a test harness `tests/p7a_world_anchor.rs:678-688`):
  - `world.insert_resource(HoveredWorldEntity::default());` (already required by
    `ui_world_visibility_system`; P7a tests insert it where they use visibility).
  - No resource needed for the pick beyond `PhysicalInput` / `UiViewport` /
    `ViewUniform`, which the host already inserts for focus/project.
  - Schedule (capture `SystemKey`s, `system_config.rs:55/66/77`; `add_system` accepts
    an exclusive `fn(&mut EcsMaster)`, `schedule_builder.rs:166`):
    ```rust
    let cam  = builder.add_system(resolve_active_camera).key();
    let prop = builder.add_system(propagate_transforms).key();
    let proj = builder.add_system(ui_world_project_system).after(prop).key();
    let pick = builder.add_system(ui_world_pick_system).after(cam).after(prop).key();
    let vis  = builder.add_system(ui_world_visibility_system).after(pick).key();
    // ... ui_layout_discovery .after(proj).after(vis) ... ui_layout_apply ...
    ```
  - **Schedule contract addendum (to add to `world/mod.rs` doc-comment):** register
    `ui_world_pick_system` `.after(resolve_active_camera)` + `.after(propagate_transforms)`
    + `.before(ui_world_visibility_system)`. It must also run BEFORE the layout
    discovery/apply (so the same-frame layout sees the fresh `UiWorldOccluded`); since
    it precedes `ui_world_visibility_system` which itself precedes layout, this holds
    transitively. (`ui_world_pick_system` and `ui_world_project_system` are independent
    — both read the same snapshots, neither writes the other's outputs — they may run
    in either relative order; `UiWorldOccluded` and `UiWorldCulled` are distinct bits.)

`UiPlugin` itself needs NO change unless the project later decides `UiPlugin` should
own world-UI scheduling (out of scope; P7a deliberately left it host-owned).

### 2.7 `boyko_render` — the cross-check golden (D3, INVESTIGATE #3 confirms no other render change)

`boyko_render` is the only crate that sees BOTH `boyko_scene::camera::camera_ray` AND
`boyko_rhi_vulkan::compute::composite_pixel_ray`. Add a `#[cfg(test)]` golden in
`crates/boyko_render/tests/` (e.g. `p7b_camera_ray_crosscheck.rs`) — NO production
render code:

```rust
// Build a forward perspective ViewUniform (eye, basis, fov, aspect), then build the
// equivalent marcher camera via `host_camera_from_view(&view, w, h)`
// (camera_drives_render_gpu.rs:270) — which sets CompositeCamera::Perspective {
//   eye, forward, right, up, tan_half_fov = (view.fov_y*0.5).tan(),
//   aspect = (w as f32) / (h as f32),   // W4: aspect = w/h, matches camera_ray's vp_w/vp_h
// }.
// Pixel set MUST include >=1 OFF-CENTER, OFF-AXIS pixel in EACH of the four quadrants
// (e.g. (w/4, h/4), (3w/4, h/4), (w/4, 3h/4), (3w/4, 3h/4)) PLUS the center + corners,
// so a y-flip sign error or an aspect (w/h vs h/w) error cannot hide behind symmetry.
// For each integer pixel (px,py):
//   let (ro_m, rd_m) = composite_pixel_ray(px, py, w, h, cam);          // marcher (pixel CENTER, +0.5 folded in)
//   let r = camera_ray(&view, px as f32 + 0.5, py as f32 + 0.5, w as f32, h as f32); // +0.5 to match the marcher pixel CENTER
//   assert ro_m ~= r.origin (eye) and rd_m ~= r.dir within EPS.
```
**EPS ~1e-6** (W4). Justification for the tight epsilon: a FORWARD perspective camera
never produces a near-zero pre-normalize `dir`, so `Vec3::normalize`'s zero-guard
recip-sqrt branch is NEVER taken on these pixels — it computes the identical
`sqrt`/divide the marcher does. The ONLY divergence is f32 rounding in the
basis-combine (`forward + right*sx + up*sy`) and the normalize, which is bounded well
below 1e-6 for the small basis-combine of a unit-basis forward camera. (A degenerate /
backward camera could trip the guard and diverge — the golden tests forward cameras
only, by construction.) This closes the drift gap WITHOUT `boyko_ui`/`boyko_scene`
depending on the vulkan backend (the dependency lives only in `boyko_render`'s test).

**INVESTIGATE #3 conclusion (no render change needed):** the GPU emit path is
`ComputedRect`-driven end to end — rect quads via `UiUploadSystem::pack_sort_upload`
→ `pack_ui_instance` (`crates/boyko_render/src/ui/upload.rs:149-174`,
`crates/boyko_render/src/ui/pack.rs:53-62`, folds `input.rect`), glyphs via
`emit_glyphs` keyed off `node.rect` (`crates/boyko_ui/src/text/emit.rs:100-114`), and
the host gather `host_upload_frame_from_world` (`upload.rs:241-260`) only produces
`UiNode`s for nodes it finds. A root the layout pass skips writes no `ComputedRect`
for itself or its subtree → no `UiNode` → no GPU instance. `boyko_render` has ZERO
references to `UiWorldProjection` or the cull/hide/occlude tags (it observes the
*absence* of nodes, not the bit). Adding `depth: f32` to `UiWorldProjection` is read
by NOTHING in the emit path (the field is purely additive; `.fade` is likewise still
unapplied — deferred per `components.rs:157-159`). **So: no new render code; the
existing P5a/P6b emit path renders exactly what the layout produced.**

---

## 3. Multithreading model

`ui_world_pick_system` is **exclusive** (`fn(&mut EcsMaster)`) — it runs alone in the
apply window, like `ui_world_project_system` / `ui_world_visibility_system` /
`propagate_transforms`. No shared mutable state, no atomics, no cross-thread data: it
reads three resources (snapshot-copied / cloned), walks two `query_entities` Vecs,
and writes one resource + bitset bits, all single-threaded. The borrow protocol is the
"snapshot the resource by `Copy`/`Clone` so no resource borrow is held across the
per-entity `&mut`-calls" pattern P7a already uses (`project.rs:201-205`,
`focus.rs:152-157`). Trivially data-race free (no concurrency). `Send`/`Sync` of the
new components/resource: `UiPickable`/`UiPickShape`/`UiWorldOccluded` are POD `Copy`
(auto `Send + Sync`); `HoveredWorldEntity` already is.

---

## 4. The CPU test matrix (golden list for the tester)

All tests are CPU/headless (no GPU). Crate placement in parentheses.

**Ray math (`boyko_math` unit tests):**
1. `ray_sphere`: hit ahead (t > 0 = entry distance), miss (disc < 0), behind
   (sphere fully behind origin → None), tangent (disc == 0 → single t), origin-inside
   (→ t == 0). Numeric: a unit sphere at `(0,0,-5)`, ray `+(-z)` → `t ≈ 4`.
2. `ray_aabb`: hit ahead, miss, behind, axis-parallel through-box (one slab inf,
   other two finite → hit), axis-parallel miss (→ None), origin-inside (→ t == 0),
   exactly-grazing a face/edge.
3. `nearest-of-many`: two spheres on the ray at t=3 and t=7 → the intersector returns
   3; mixed sphere+aabb nearest.
4. `Ray::at(t)` == `origin + dir*t`.
5. degenerate `dir == ZERO` → `ray_aabb`/`ray_sphere` return `None` (non-finite
   guarded), no panic, no NaN escape.

**`camera_ray` (`boyko_scene` unit tests):**
6. ROUND-TRIP: `camera_ray` ↔ `project_world_to_screen`. For a forward perspective
   view, pick a world point in front, `project_world_to_screen` → `(sx, sy)`; then
   `camera_ray(view, sx, sy, w, h)` must produce a ray that passes through that world
   point (point lies on `origin + t*dir` for some t>0, within eps). This is THE
   correctness anchor (both use `ndc = coord/extent*2-1` + the same y-flip).
7. center pixel (`px=w/2, py=h/2`) → ray dir ≈ `cam_forward` (within eps).
8. `fov_y == 0.0` ortho sentinel takes the ortho arm (dir ≈ `cam_forward`, origin
   offset by ndc) — documented-approximate; assert it does not panic / NaN.

**Cross-check (`boyko_render` test):**
9. `camera_ray(view, px+0.5, py+0.5, w, h)` ≈ the marcher ray
   `composite_pixel_ray(px, py, w, h, host_camera_from_view(&view, w, h))` (marcher
   built with `aspect = w/h`, W4) for: center + four corners + at least ONE off-center
   off-axis pixel in EACH of the four quadrants (so a y-flip or aspect error cannot hide
   behind symmetry); forward perspective camera; EPS ~1e-6 (justified — forward cameras
   never hit the normalize zero-guard, divergence bounded by f32 basis-combine rounding).

**Pick system (`boyko_ui` integration tests):**
10. `gate_pick_hits_nearest`: two `UiPickable` scene entities on the cursor ray at
    different depths → `HoveredWorldEntity` == the nearer one.
11. `gate_pick_miss_none`: cursor ray misses all bounds → `HoveredWorldEntity(None)`.
12. `gate_cursor_inactive_none`: `cursor_inside == false` (or `window_focused ==
    false`) → `HoveredWorldEntity(None)`, occlusion all cleared, system early-returns.
13. `gate_pick_resolves_to_anchor_root` (THE contract test for the §1 hazard): a scene
    entity `S` with `UiPickable` + `GlobalTransform`, a `UiWorldAnchor { target:
    EntityAnchor(S) }` root `Rt`; pick `S` → `HoveredWorldEntity(Some(S))` → after
    `ui_world_visibility_system`, `Rt`'s `UiWorldHidden` is cleared (shown) and other
    roots are hidden. (Proves pick→hover→visibility end to end, and that `UiPickable`
    on the SCENE entity is the correct placement.)
14. `gate_layer_mask`: a pickable on a non-matching layer is skipped.

**Occlusion (`boyko_ui` integration tests):**
15. `gate_occlusion_occluder_in_front`: a `depth_test == true` root, an occluder
    `UiPickable` between eye and anchor point (t < dA - BIAS) → `UiWorldOccluded` set.
16. `gate_occlusion_clear_path`: occluder removed / behind the anchor → bit cleared.
17. `gate_occlusion_depth_test_false_never`: `depth_test == false` root with an
    occluder in front → bit NEVER set (always clear).
18. `gate_occlusion_self_target_excluded`: the anchor's own `EntityAnchor` target
    surface in front of (around) the anchor point does NOT occlude its own label.
19. `gate_layout_skips_occluded_root` (mirror the existing
    `gate3_layout_skips_culled_root`, `tests/p7a_world_anchor.rs:307`): an occluded
    root's `ComputedRect` stays at spawn default (not positioned), becomes positioned
    once `UiWorldOccluded` is cleared.

**Struct/layout + zero-overhead:**
20. `const _` asserts: `size_of::<UiWorldProjection>() == 24`, `align == 4`;
    `size_of::<UiWorldAnchor>() == 56` UNCHANGED; `UiPickable`/`UiPickShape` size/align
    pinned (house style).
21. `gate_zero_overhead_no_pickable`: a world with anchors but NO `UiPickable` → pick's
    pickable query is empty → `HoveredWorldEntity` stays `None`, no occlusion set, a
    second run with a still cursor writes nothing (set-if-changed). Assert the
    HoveredWorldEntity change-tick does not advance on the still path.
22. `depth` field populated: after `ui_world_project_system`, a visible anchor's
    `UiWorldProjection.depth` == the projected `ndc_z` (in `[0,1]`).

**Soundness:** run the full P7b suite under `cargo +nightly miri test` for the
`boyko_ui` pick tests (all-safe API; Miri confirms no accidental UB via the ECS calls).
No `unsafe` is introduced (§10), so Miri is a belt-and-suspenders pass, not a gate on a
new unsafe block.

---

## 5. Determinism / zero-overhead (INVESTIGATE #5)

- **No `UiPickable` in the world** → `query_entities(&[UiPickable::component_id()])`
  returns an empty Vec → the pick loop does nothing → `HoveredWorldEntity` stays `None`
  (set-if-changed suppresses the redundant write). **The occlusion root-walk is NOT
  skipped** (C2 — the prior "skip when no pickables" optimization is DROPPED): with zero
  pickables every root simply re-derives to "not occluded" and `disable`s its bit, which
  writes nothing when the bit is already clear. This is the load-bearing guarantee:
  **every root's `UiWorldOccluded` bit is re-derived on every pick-system run; no code
  path leaves a set bit un-revisited** — so a pickable that despawned after setting a bit,
  or a cursor that left the window, can never strand a root permanently occluded (the
  gate-5 eviction class). The walk is O(R) ≈ tens roots, the toggle is O(1) and write-free
  when correct, so the still frame stays cheap even though it always walks.
- **Still cursor** → `cursor_active` true but the ray is identical; `set-if-changed` on
  `HoveredWorldEntity` writes nothing if the hovered entity is unchanged; the occlusion
  bits are re-derived (always-walk) but `enable`/`disable` to the same value is a no-op
  bit write (O(1), no migration). The downstream `ui_world_visibility_system` and
  `layout` both have their OWN Changed-gates (`visibility.rs:79`, the layout discovery
  pass), so a still frame does no relayout.
- **Allocations:** two `query_entities` Vecs/frame (pickables + roots) — the SAME
  tradeoff `project`/`visibility` already make (`project.rs:230`, `visibility.rs:88`,
  both documented "O(tens) roots, off the hot path"). No per-entity allocation, no
  `format!`/`String`/`Box`/`HashMap`. A future phase can hoist both into a retained
  scratch (the zero-alloc `query_entities_buf(ids, &mut buf, &mut arch_ids)` exists,
  used by `focus.rs:213`) if anchor/pickable churn ever warrants it — noted, NOT done
  in P7b (it would add a scratch resource the host must insert; parity with P7a's
  current allocating pattern is intentional).
- **Determinism:** pure f32 arithmetic, fixed iteration order over `query_entities`
  (stable per archetype layout), `min`-by-`t` nearest selection is order-independent
  for distinct depths; equal-`t` ties resolve to the first encountered (a documented,
  deterministic tie-break — equal-depth picks are a setup ambiguity).

---

## 6. Edge cases & risks (and how each is handled)

| Case | Handling |
|---|---|
| Degenerate ray (`dir == 0`, e.g. zero-extent viewport, or an anchor AT the eye) | `camera_ray` uses `Vec3::normalize` (guards → `ZERO`); BOTH `ray_sphere` and `ray_aabb` have an explicit FIRST-STEP release guard `if dir.length_squared() <= RAY_DIR_MIN_SQ { return None }` (W2) → a zero/near-zero dir ALWAYS returns `None`, including the origin-inside-sphere/box case (which would otherwise return `Some(0.0)`). No NaN escapes; `debug_assert!` on `viewport.width/height > 0` (mirror `focus.rs`/`project.rs` aspect guard) + the near-unit `debug_assert!` on `dir` as the caller contract. |
| Behind-camera anchor (occlusion) | The projection already marks such a root `!visible` → `UiWorldCulled` set → layout skips it regardless. The occlusion pass short-circuits when `UiWorldProjection.visible == false` (recommended early-out), so it never casts a meaningless behind-eye ray. |
| Dangling `EntityAnchor` target (despawned / no `GlobalTransform`) | `resolve_anchor_point` returns `None` (mirrors `project.rs:258-261`); the occlusion pass treats it as "not occluded" (clears the bit) — the project system already marks it invisible/culled the same frame, so layout skips it via `UiWorldCulled`. No stale point used. |
| Non-uniform scale on a pick bound | The pick uses `s = max(‖col_0‖, ‖col_1‖, ‖col_2‖)` — the largest COLUMN norm of the row-major `Mat3 { rows: [Vec3;3] }` linear part (`‖col_i‖ = (rows[0][i], rows[1][i], rows[2][i]).length()`, NOT a max-abs element, W3). Conservative — the bound never shrinks below the true shape on any axis; a sphere stays a sphere, an AABB grows isotropically. A `debug_assert!` checks the three column norms are within eps (uniform-scale contract, mirroring `debug_assert_camera_rigid`, camera.rs:422). Documented in `UiPickable`'s doc-comment; non-uniform-scaled targets are a setup choice, not a correctness bug. (True OBB picking is deferred — out of scope.) |
| Pickable without `GlobalTransform` | `debug_assert!` + skip (a setup error; release-safe). |
| Self-occlusion (anchor's own target surface "in front of" its label) | An `EntityAnchor(t)` anchor EXCLUDES its own target by id: `self_target = Some(t)`, the occlusion loop skips `Some(e) == self_target`, so a label about object X is never hidden by X's own surface. A `WorldTarget::WorldPos` anchor has NO self-exclusion (`self_target = None`, O1) — there is no scene entity to exclude; any pickable in front of the fixed point may occlude its label (defensible: a `WorldPos` label is "a point in the air", not "a label on object X"). |
| Ortho camera (`fov_y == 0.0`) | `camera_ray` takes the best-effort ortho arm (unit-extent placeholder; documented). The marcher's ortho arm uses FIXED legacy constants (`compute.rs:1314-1319`), so ortho pick does NOT match the marcher fixture — DOCUMENTED mismatch; P7b targets perspective (the screenshot scene). |
| Equal-depth pick tie | First-encountered wins (deterministic, documented). |
| Occlusion-bias flicker at a label coincident with a surface | A RELATIVE / scale-invariant bias `REL_BIAS = 1e-3` (occluded iff `t < dA * (1 - REL_BIAS)`, O2) — a fixed world-unit bias is meaningless at a different world scale; the relative form is always ~0.1% of the eye→anchor distance. Tuned in `gate_occlusion_*` tests. |

---

## 7. Integration summary

- **New deps:** `boyko_scene` already depends on `boyko_math` (`camera.rs:37`); `camera_ray`
  uses `boyko_math::ray::Ray` — no new crate edge. `boyko_ui` already depends on
  `boyko_math` + `boyko_scene` + `boyko_input` — `pick.rs` uses all three; no new edge.
  `boyko_render` already depends on both `boyko_scene` and `boyko_rhi_vulkan` — the
  cross-check golden is a test-only consumer; no new edge.
- **Shared anchor-point resolver:** factor `fn resolve_anchor_point(world, anchor) ->
  Option<[f32;3]>` (the `project.rs:247-269` logic) into a shared `world` helper used by
  BOTH `ui_world_project_system` and `ui_world_pick_system`'s occlusion pass, so the
  projected point and the occlusion ray can never drift apart (a single source of truth
  for "where is this anchor in the world"). This is a small refactor of existing code,
  perf-neutral (same arithmetic), and removes a latent divergence risk. The occlusion
  pass separately derives `self_target` from `anchor.target` (`EntityAnchor(t) =>
  Some(t)`, `WorldPos(_) => None`, O1) — `resolve_anchor_point` returns only the POINT;
  self-exclusion is the caller's concern.
- **Existing API changes:** `UiWorldProjection` grows a field (24 B; the size assert +
  the `project.rs` write site update); `project_world_to_screen`'s caller writes `depth`.
  No signature change to any public fn. No change to `UiWorldAnchor` (its `depth_test`
  field was already present and unused — P7b consumes it).
- **Affected modules:** `boyko_math` (+`ray`), `boyko_scene::camera` (+`camera_ray`),
  `boyko_ui::world` (+`pick`, +components, `UiWorldProjection.depth`), `boyko_ui::layout`
  (skip guard), `boyko_ui::world::mod` (re-exports + schedule-contract doc),
  `boyko_render` (test golden).

---

## 8. Implementation plan (ordered steps for the developer)

1. **`boyko_math`**: add `src/ray.rs` (`Ray`, `Ray::at`, `ray_sphere`, `ray_aabb`) with
   the formulas in §2.1; `pub mod ray;` + re-export in `lib.rs`; unit tests 1–5 (§4).
2. **`boyko_scene::camera`**: add `camera_ray` (§2.2, perspective + ortho-best-effort
   arms); unit tests 6–8.
3. **`boyko_ui::world::components`**: add `UiPickShape`, `UiPickable`, `UiWorldOccluded`;
   add `UiWorldProjection.depth` + bump asserts to 24/4 (§2.3); update the
   `project.rs:287-293` write site to copy `pp.ndc_z`; layout-pin asserts (test 20).
4. **`boyko_ui::world`**: factor `resolve_anchor_point` shared helper (§7); add
   `src/world/pick.rs` with `ui_world_pick_system` (pick + occlusion, §2.4) +
   `set_hovered_if_changed`; `mod pick;` + re-exports in `world/mod.rs`; extend the
   schedule-contract doc-comment.
5. **`boyko_ui::layout`**: import `UiWorldOccluded`, add `|| occluded` to the skip guard
   (§2.5).
6. **Host wiring**: insert `HoveredWorldEntity` (if the host schedule didn't already)
   and add `ui_world_pick_system` with the ordering in §2.6 (reference test harness).
7. **Tests**: `boyko_ui` integration tests 10–22 (§4); `boyko_render` cross-check golden
   (test 9); run the `boyko_ui` pick suite under Miri (§4 soundness).
8. **Build gates**: `cargo check --all-targets`, `cargo clippy --all-targets -- -D
   warnings` (the new `&mut EcsMaster` system carries `#[allow(clippy::
   needless_pass_by_ref_mut)]`), `cargo test --all-targets`, then the orchestrator-owned
   offscreen screenshot (existing render path — owner is the visual oracle).

---

## 9. Deferred: GPU depth-buffer occlusion (explicit)

**GPU depth-buffer occlusion is DEFERRED; P7b ships a CPU proxy.** Rationale (one line):
the pick already ray-tests these exact bounds, so occlusion reuses the SAME geometry
(pick and occlusion can never disagree) and the SAME EnableTag visibility plumbing (the
layout pass already skips `UiWorldCulled`/`UiWorldHidden`; `UiWorldOccluded` is a third
bit on that guard) — at zero risk to the frozen UI rect/text goldens and zero exposure
to the crash-prone GPU path. The CPU proxy is honestly NOT a per-pixel scene-depth test:
it occludes a root when a nearer `UiPickable` surface lies between the eye and the anchor
POINT (the same point the projection uses), not when arbitrary scene geometry covers the
label's pixels. The `UiWorldProjection.depth` field is added now (the previously-discarded
`ndc_z`) so a FUTURE GPU-depth UI pass / a CPU z-sort can consume it without a layout
change.

---

## 10. Soundness — NO new `unsafe` (INVESTIGATE #6)

Every P7b operation is a SAFE public ECS / math API: `query_entities`, `get_component`,
`resource` / `resource_mut`, `enable` / `disable` / `is_enabled`, plus pure-f32 ray
math. There is **no new `unsafe` block** in any P7b file. (The `set_if_neq` /
`get_component_mut` write site already exists; we only add an `f32` field to its
payload.) Miri (§4) runs as defense-in-depth over the ECS API calls. If a future
zero-alloc rewrite adopts `query_entities_buf` it remains a safe API.

---

## 11. Open questions

1. **Layer-mask source:** the pick's `layer_mask` (which layers the cursor picks) — is
   it a constant `u32::MAX` for P7b, or a `Resource` the host sets? Recommendation:
   ship `u32::MAX` (pick-all) in P7b with `UiPickable.layers` honored, and add a
   `UiPickLayerMask(u32)` resource only when a real multi-layer use case appears (YAGNI;
   `UiPickable.layers` already supports per-target exclusion). Defaulting to all-layers
   keeps the still path identical to "no masking".
2. **RESOLVED (C2):** the occlusion pass ALWAYS walks every root every run and
   re-derives its bit; there is no "skip the pass" path. The `!visible` case is handled
   by re-deriving the bit to "not occluded" (`disable`) and continuing — it DISABLES the
   bit rather than skipping the root, so no stale set bit can survive (the gate-5
   eviction class). The `get_component::<UiWorldProjection>` read for `visible` is one
   read amortized by the anchor fetch.
3. **`UiWorldOccluded` and `#[require]`:** like `UiWorldCulled`/`UiWorldHidden`, it is
   NOT `#[require]`d on `UiWorldAnchor` (absence = bit clear = "not occluded", the
   correct default; `is_enabled` on an entity lacking the bitset component returns
   `false`). Confirmed against the existing tags' non-required pattern. No `#[require]`
   edit to `UiWorldAnchor`.
