//! World-space / diegetic UI components (GUI P7a).
//!
//! A world-anchored UI subtree (nameplate / health bar / 3D prompt) is a normal
//! [`UiRoot`](crate::components::UiRoot) carrying a [`UiWorldAnchor`]: the root's
//! screen origin is driven each frame by
//! [`ui_world_project_system`](super::project::ui_world_project_system) from a
//! fixed [`WorldTarget::WorldPos`] or a live
//! [`WorldTarget::EntityAnchor`]'s `GlobalTransform`, projected through the S3
//! camera. The projected result lands in [`UiWorldProjection`] — a SEPARATE
//! input the layout pass reads to seed the root origin, so the layout pass stays
//! the single `ComputedRect` writer (no pre-pass write race; the seam the P6a
//! [`UiAnchor`](crate::components::UiAnchor) precedent establishes).
//!
//! Principle 0: anchors are ECS components and projection / visibility are
//! systems over the engine's own storage — there is no parallel world-anchor
//! data system.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Resource};

/// The world point a [`UiWorldAnchor`] tracks.
///
/// `#[repr(C)]` enum: a fixed point or a live scene entity whose
/// `GlobalTransform.translation` is read each frame. The largest payload is
/// `[f32; 3]` (12 B); the discriminant + alignment fold into 16 B (const-asserted
/// on [`UiWorldAnchor`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldTarget {
    /// A FIXED world point `(x, y, z)`. The anchor never moves unless the
    /// component is rewritten.
    WorldPos([f32; 3]),
    /// A LIVE scene entity. The anchor reads the entity's
    /// [`GlobalTransform`](boyko_scene::transform::GlobalTransform)`.translation`
    /// each frame (so it tracks a moving target).
    EntityAnchor(Entity),
}

/// How an anchored subtree scales with camera distance.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WorldScaleMode {
    /// Constant pixel size regardless of depth (a billboarded HUD label). The
    /// default — the projected point moves, but the subtree keeps its laid-out
    /// pixel size.
    #[default]
    ScreenSpace,
    /// Perspective-scaled: the subtree shrinks with distance by
    /// `ref_distance / dist` (true Euclidean eye-distance, NOT clip `w` — see
    /// [`UiWorldAnchor::ref_distance`]).
    WorldScaled,
}

/// Marks a [`UiRoot`](crate::components::UiRoot) subtree as WORLD-ANCHORED
/// (GUI P7a). AUTHOR-OWNED, OPT-IN.
///
/// A root WITH this component is positioned by
/// [`ui_world_project_system`](super::project::ui_world_project_system) (which
/// writes [`UiWorldProjection`]), NOT by a screen-edge
/// [`UiAnchor`](crate::components::UiAnchor); the two root-positioning kinds are
/// mutually exclusive (a `debug_assert!` in the project system catches a root
/// carrying both). A `UiWorldAnchor` on a NON-root in-tree node is ignored
/// (mirrors the `UiAnchor` "roots only" rule).
///
/// It auto-inserts [`UiWorldProjection`] via `#[require(...)]` so an author can
/// never add the anchor without the projection column the project query / the
/// layout seam read.
///
/// `#[repr(C)]`, POD `Copy`; its own SoA column. HOT input, read once per anchor
/// per frame by the project system.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(UiWorldProjection)]
pub struct UiWorldAnchor {
    /// The world point or tracked entity.
    pub target: WorldTarget,
    /// World-space offset added to the target point before projection
    /// (e.g. `[0.0, 2.0, 0.0]` = 2 m above the target).
    pub offset: [f32; 3],
    /// Reserved for the P7b billboard quad orientation; the CPU core always
    /// faces the screen (the projected point IS the anchor).
    pub billboard: bool,
    /// `ScreenSpace` (constant px) vs `WorldScaled` (perspective shrink).
    pub scale_mode: WorldScaleMode,
    /// Whether render occludes the subtree against scene depth. P7b consumes
    /// this; it is STORED (forwarded) but unused by the P7a CPU core.
    pub depth_test: bool,
    /// Whether to compute the distance-fade factor into
    /// [`UiWorldProjection::fade`].
    pub fade_by_distance: bool,
    /// `WorldScaled` reference distance: the eye-distance at which
    /// `scale == 1.0` (`scale = ref_distance / dist`). Ignored for
    /// `ScreenSpace`. Must be finite and `> 0`.
    pub ref_distance: f32,
    /// Distance fade: `fade == 1.0` at or below this eye-distance. Ignored when
    /// `fade_by_distance` is false.
    pub fade_near: f32,
    /// Distance fade: `fade == 0.0` at or beyond this eye-distance. Must exceed
    /// `fade_near`. Ignored when `fade_by_distance` is false.
    pub fade_far: f32,
}

// Layout pin (house style): `WorldTarget`'s `EntityAnchor(Entity)` arm carries an
// 8-aligned `Entity` (a `usize` id + a `u32` generation) → a 16 B payload, so the
// enum is 8-aligned and 24 B (8 B discriminant slot + 16 B payload); the struct
// rounds to 56 B at align 8. A silent layout drift (a widened field) must fail the
// build rather than read as "correct".
const _: () = assert!(size_of::<UiWorldAnchor>() == 56);
const _: () = assert!(align_of::<UiWorldAnchor>() == 8);

impl Default for UiWorldAnchor {
    /// A fixed origin-point anchor, no offset, screen-space scale, no fade — the
    /// "constant-size HUD label pinned at the world origin" default.
    #[inline]
    fn default() -> Self {
        Self {
            target: WorldTarget::WorldPos([0.0, 0.0, 0.0]),
            offset: [0.0, 0.0, 0.0],
            billboard: true,
            scale_mode: WorldScaleMode::ScreenSpace,
            depth_test: false,
            fade_by_distance: false,
            ref_distance: 1.0,
            fade_near: 0.0,
            fade_far: 1.0,
        }
    }
}

/// The per-frame projection RESULT for a [`UiWorldAnchor`] root.
///
/// PRODUCED by
/// [`ui_world_project_system`](super::project::ui_world_project_system); READ by
/// the layout pass (origin seed + uniform subtree scale) and, later (P7b /
/// render), the fade. It is the single-writer seam that keeps `ComputedRect`
/// written ONLY by the layout pass (no pre-pass write race).
///
/// AUTHOR-NEVER-WRITES; auto-inserted by `UiWorldAnchor`'s `#[require(...)]`.
/// The [`Default`] (`visible == false`, `scale == 1.0`, `fade == 1.0`,
/// `depth == 1.0`) lays a root out at the origin, invisible, until the first
/// projection runs.
///
/// `#[repr(C)]`, POD `Copy`, `PartialEq` so the project system can `set_if_neq`
/// it (the Changed-gate: a still anchor + still camera writes nothing).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiWorldProjection {
    /// Projected screen-space top-left seed, logical px (+x right, +y down) —
    /// the SAME basis as [`ComputedRect`](crate::components::ComputedRect) /
    /// [`UiViewport`](crate::resources::UiViewport).
    pub screen_x: f32,
    /// Projected screen-space top-left seed, logical px (+y down).
    pub screen_y: f32,
    /// Uniform subtree scale (`1.0` for `ScreenSpace`; `ref_distance / dist`
    /// for `WorldScaled`). Always finite and `> 0`.
    pub scale: f32,
    /// Distance-fade factor in `[0, 1]` (`1.0` when `fade_by_distance` is off);
    /// monotonically non-increasing in eye-distance. The render path multiplies
    /// alpha by it (deferred — not applied by the P7a CPU core).
    pub fade: f32,
    /// NDC z of the anchor point (= `ProjectedPoint.ndc_z`, previously discarded,
    /// P7b/W1). Default `1.0` = the far plane / "nearest-neutral": a far-plane /
    /// no-op depth that never reads as "in front" of real geometry, so a
    /// default-constructed projection cannot spuriously win a future z-order.
    /// Forward-looking GPU-depth / z-order seam: a future GPU-depth UI pass or a
    /// CPU z-sort can consume it. The P7b occlusion is a CPU proxy and does NOT
    /// read this — it is stored only.
    pub depth: f32,
    /// `false` when the anchor is behind the camera or off-screen (the
    /// CPU-testable cull mirror; the layout pass skips a `!visible` world root).
    /// STAYS LAST — author-never-writes.
    pub visible: bool,
}

// Layout pin (house style): 5×f32 (20 B) + `bool` + 3 B tail pad = 24 B at
// align 4 (P7b grew it from 20 B by inserting `depth: f32` before `visible`). A
// silent layout drift must fail the build rather than read as "correct".
const _: () = assert!(size_of::<UiWorldProjection>() == 24);
const _: () = assert!(align_of::<UiWorldProjection>() == 4);

// `depth` defaults to the far-plane-neutral `1.0`, so `Default` is hand-written
// rather than derived: a derived `Default` would give `depth: 0.0` = the NEAR
// plane, which a future z-order would read as "in front of everything" — the
// wrong neutral. The other neutrals (`scale: 1.0`, `fade: 1.0`, `visible:
// false`) preserve the previous derived/documented semantics exactly.
impl Default for UiWorldProjection {
    /// A root at the origin, invisible, unit scale + fade, far-plane depth.
    #[inline]
    fn default() -> Self {
        Self {
            screen_x: 0.0,
            screen_y: 0.0,
            scale: 1.0,
            fade: 1.0,
            depth: 1.0,
            visible: false,
        }
    }
}

/// The "hovered 3D entity" input the FUTURE P7b GPU cursor-ray pick populates.
///
/// In P7a it is set DIRECTLY (a headless test, or a stub). A `Resource`
/// (engine-owned storage — Principle 0). `Some(entity)` names the hovered SCENE
/// entity; [`ui_world_visibility_system`](super::visibility::ui_world_visibility_system)
/// resolves it to the world-anchor root tracking that entity and enables that
/// root's subtree (disabling the previously-hovered one). `None` hides all
/// hover-driven roots.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct HoveredWorldEntity(pub Option<Entity>);

/// The FRUSTUM-cull EnableTag, OWNED by the project system (Decision 3 / M4
/// two-tag split). A bitset tag (`#[component(storage = "bitset")]`): toggling
/// it is O(1) — no archetype migration, no structural bump.
///
/// Set on a world-anchor root when the anchor is behind the camera or
/// off-screen; the layout pass skips a root with this bit set. Independent of
/// [`UiWorldHidden`] (the hover-driven hide), so the frustum-cull and the
/// show/hide authorities never race the same bit. A root is laid out only when
/// NEITHER bit is set.
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct UiWorldCulled;

/// The HOVER-driven hide EnableTag, OWNED by the visibility system (Decision 3 /
/// M4 two-tag split). A bitset tag: O(1) toggle.
///
/// Set on a world-anchor root that is NOT the currently-hovered one (so it is
/// hidden), cleared on the hovered one. Independent of [`UiWorldCulled`]. The
/// layout pass skips a root with this bit set.
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct UiWorldHidden;

/// The shape of a [`UiPickable`] bound, in the entity's LOCAL frame (transformed
/// by its `GlobalTransform` at pick time, GUI P7b). `#[repr(C)]` POD enum.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiPickShape {
    /// A sphere of `radius`, centered at the entity's world translation.
    Sphere {
        /// The sphere radius in the entity's local frame (scaled by the uniform
        /// transform scale at pick time).
        radius: f32,
    },
    /// An axis-aligned box of `half_extents`, centered at the world translation.
    Aabb {
        /// Per-axis half-extents in the entity's local frame (scaled by the
        /// uniform transform scale at pick time).
        half_extents: [f32; 3],
    },
}

/// Marks a SCENE entity (NOT a UI root) as cursor-ray pickable for world-space UI
/// (GUI P7b).
///
/// The pick (`ui_world_pick_system`) ray-tests this bound and writes the nearest
/// hit into [`HoveredWorldEntity`]; the existing
/// [`ui_world_visibility_system`](super::visibility::ui_world_visibility_system)
/// then shows the [`UiWorldAnchor`] root whose `EntityAnchor(target)` equals the
/// picked entity. THEREFORE `UiPickable` MUST sit on the entity an anchor tracks,
/// alongside a `GlobalTransform`. A `UiPickable` placed on the UI ROOT instead
/// would never match any `EntityAnchor` (the visibility system matches the picked
/// id against `WorldTarget::EntityAnchor`), so nothing would ever show. NOT
/// coupled to `boyko_physics` colliders (`boyko_ui` does not depend on
/// `boyko_physics` — Principle 0: the pick bound is a first-class UI component,
/// not a borrowed physics primitive).
///
/// The shape is local; the pick applies the `GlobalTransform`'s translation + a
/// UNIFORM scale to the bound. The scale is the conservative per-axis bound
/// `s = max(‖col_0‖, ‖col_1‖, ‖col_2‖)` of the transform's linear part (the
/// COLUMN norms of the ROW-MAJOR `Mat3 { rows: [Vec3; 3] }`:
/// `‖col_i‖ = (rows[0][i], rows[1][i], rows[2][i]).length()` — NOT a max-abs
/// element). A uniform-scaled target has equal column norms; a non-uniform-scaled
/// target conservatively uses the largest (the bound never shrinks below the true
/// shape on any axis). True OBB picking is out of scope.
///
/// O1: a [`UiWorldAnchor`] whose `target` is [`WorldTarget::WorldPos`] has NO
/// self-exclusion in the occlusion pass (`self_target = None`) — there is no
/// scene entity to exclude, so a front pickable on (or near) the fixed point DOES
/// occlude its label. Defensible: a `WorldPos` label is "a point in the air", not
/// "a label on object X".
///
/// `#[repr(C)]`, POD `Copy`, its own SoA column.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiPickable {
    /// The pick bound, in the entity's local frame.
    pub shape: UiPickShape,
    /// Pick-layer mask (a bitset of layers this target lives on); the pick's
    /// `layer_mask` ANDs against it. `u32::MAX` (default) = "all layers".
    pub layers: u32,
}

// Layout pin (house style): `UiPickShape` is a tagged union — the `Aabb` arm
// carries `[f32; 3]` (12 B) and the discriminant rounds the enum to 16 B at
// align 4; `+ layers: u32` (4 B) makes the struct 20 B at align 4. A silent
// layout drift (a widened payload) must fail the build rather than read as
// "correct".
const _: () = assert!(size_of::<UiPickShape>() == 16);
const _: () = assert!(align_of::<UiPickShape>() == 4);
const _: () = assert!(size_of::<UiPickable>() == 20);
const _: () = assert!(align_of::<UiPickable>() == 4);

impl Default for UiPickable {
    /// A unit-sphere pickable on all layers (the simplest "a clickable point").
    #[inline]
    fn default() -> Self {
        Self {
            shape: UiPickShape::Sphere { radius: 0.5 },
            layers: u32::MAX,
        }
    }
}

/// The DEPTH-TEST occlusion EnableTag, OWNED by `ui_world_pick_system`'s
/// occlusion pass (GUI P7b, Decision 5). A bitset tag
/// (`#[component(storage = "bitset")]`): O(1) toggle, no archetype migration —
/// IDENTICAL backend to [`UiWorldCulled`] / [`UiWorldHidden`].
///
/// Set on a `depth_test == true` world-anchor root whose anchor point is occluded
/// by a nearer [`UiPickable`] surface (a CPU PROXY against the SAME bounds the
/// pick ray-tests — this is NOT a GPU depth-buffer test). The layout pass skips a
/// root with this bit set. Independent of [`UiWorldCulled`] (frustum,
/// project-owned) and [`UiWorldHidden`] (hover, visibility-owned): a third
/// authority over a distinct bit, so the three never race a shared bit.
/// `depth_test == false` roots are NEVER set (always-on-top overlay).
///
/// O1: a [`WorldTarget::WorldPos`] anchor has NO self-exclusion in occlusion — a
/// front pickable on its anchor point DOES occlude it (there is no scene entity
/// to exclude; see [`UiPickable`]).
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct UiWorldOccluded;
