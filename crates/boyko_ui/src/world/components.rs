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
/// The [`Default`] (`visible == false`, `scale == 1.0`, `fade == 1.0`) lays a
/// root out at the origin, invisible, until the first projection runs.
///
/// `#[repr(C)]`, POD `Copy`, `PartialEq` so the project system can `set_if_neq`
/// it (the Changed-gate: a still anchor + still camera writes nothing).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Default)]
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
    /// `false` when the anchor is behind the camera or off-screen (the
    /// CPU-testable cull mirror; the layout pass skips a `!visible` world root).
    pub visible: bool,
}

const _: () = assert!(size_of::<UiWorldProjection>() == 20);
const _: () = assert!(align_of::<UiWorldProjection>() == 4);

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
