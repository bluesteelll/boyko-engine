//! GUI P7b — cursor-ray PICK + depth-test OCCLUSION integration tests (HEADLESS,
//! NO GPU). Exercises the REAL `ui_world_pick_system` over a real `EcsMaster`.
//!
//! Test matrix (GUI-P7B-PLAN §4, pick 10-14 / occlusion 15-19 / zero-overhead
//! 20-23):
//!  10. pick hits the nearest pickable under the cursor.
//!  11. pick miss → HoveredWorldEntity(None).
//!  12. cursor inactive → None AND all UiWorldOccluded cleared, early return.
//!  13. gate_pick_resolves_to_anchor_root (the §1 contract): UiPickable on the
//!      SCENE entity → pick → visibility shows the root tracking it (end-to-end).
//!  14. layer mask: a non-matching-layer pickable is skipped.
//!  15. occluder in front → UiWorldOccluded set.
//!  16. clear path → bit cleared.
//!  17. depth_test == false → never occluded.
//!  18. self-target excluded (EntityAnchor); + a WorldPos case where a front
//!      pickable DOES occlude (O1).
//!  19. gate_layout_skips_occluded_root: an occluded root is skipped by layout.
//!  20. struct/layout const asserts (build-confirmed; see the note at the bottom).
//!  21. gate_no_pickable: anchors, no pickables → always-walk stays write-free.
//!  22. depth populated: a visible anchor's UiWorldProjection.depth == ndc_z∈[0,1].
//!  23. gate_no_stale_occluded_bit (C2 guard): occlude → bit set; remove occluder
//!      → bit cleared (the stranded-bit guard).
//!
//! The camera is at the world origin, identity rotation (forward -z); a pickable
//! at (0,0,-Z) sits dead-ahead, so the VIEWPORT-CENTER cursor ray hits it. Entity
//! counts are SMALL (a handful) so the Miri pass completes.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_input::PhysicalInput;
use boyko_math::{Affine3A, Mat4, Quat, Vec3, Vec4};
use boyko_scene::transform::GlobalTransform;
use boyko_scene::{Transform, ViewUniform};

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiSafeArea, UiViewport};
use boyko_ui::units::Unit;
use boyko_ui::world::components::{UiPickShape, UiPickable, UiWorldOccluded};
use boyko_ui::world::{
    HoveredWorldEntity, UiWorldAnchor, UiWorldHidden, UiWorldHoverState, UiWorldProjection,
    WorldScaleMode, WorldTarget, ui_world_pick_system, ui_world_project_system,
    ui_world_visibility_system,
};

// ════════════════════════════════════════════════════════════════════════════
// Shared constants + the camera-at-origin view
// ════════════════════════════════════════════════════════════════════════════

const VP_W: f32 = 1600.0;
const VP_H: f32 = 900.0;
const ASPECT: f32 = VP_W / VP_H;
const FOV_Y: f32 = core::f32::consts::FRAC_PI_2; // 90°
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

/// The pixel coordinates of the viewport center (the cursor ray that, under the
/// origin camera, casts straight down -z).
const CENTER_PX: f64 = (VP_W as f64) * 0.5;
const CENTER_PY: f64 = (VP_H as f64) * 0.5;

fn origin_view() -> ViewUniform {
    ViewUniform {
        view_proj: Mat4::perspective_rh(FOV_Y, ASPECT, NEAR, FAR),
        inv_view: Mat4::IDENTITY,
        camera_pos: Vec4::new(0.0, 0.0, 0.0, 1.0),
        cam_forward: Vec4::new(0.0, 0.0, -1.0, 0.0),
        cam_right: Vec4::new(1.0, 0.0, 0.0, 0.0),
        cam_up: Vec4::new(0.0, 1.0, 0.0, 0.0),
        fov_y: FOV_Y,
        aspect: ASPECT,
        near: NEAR,
        far: FAR,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Harness — pick (+ optionally project / visibility) over a real EcsMaster
// ════════════════════════════════════════════════════════════════════════════

/// A world seeded for the origin camera with `ViewUniform` / `UiViewport` /
/// `PhysicalInput` (cursor at center, active) / `HoveredWorldEntity` /
/// `UiWorldHoverState`, driving the pick system once per `run`.
struct PickHarness {
    world: EcsMaster,
}

impl PickHarness {
    fn new() -> Self {
        let mut world = EcsMaster::new();
        world.insert_resource(origin_view());
        world.insert_resource(UiViewport {
            width: VP_W,
            height: VP_H,
            scale_factor: 1.0,
            generation: 0,
        });
        let mut physical = PhysicalInput::new();
        physical.cursor_pos = [CENTER_PX, CENTER_PY];
        physical.cursor_inside = true;
        physical.window_focused = true;
        world.insert_resource(physical);
        world.insert_resource(HoveredWorldEntity::default());
        world.insert_resource(UiWorldHoverState::default());
        Self { world }
    }

    /// Drives one frame: PROJECT then PICK, mirroring the production schedule
    /// (both run each frame after the camera/transform producers; world/mod.rs
    /// schedule contract). The occlusion pass early-outs on a `!visible` root (it
    /// casts no meaningless behind-eye ray), so a root must be PROJECTED — its
    /// `UiWorldProjection.visible` set — before the occlusion pass will test it.
    /// Running project here makes every occlusion gate exercise the real path.
    fn run(&mut self) {
        self.world.run_system(ui_world_project_system);
        self.world.run_system(ui_world_pick_system);
    }

    fn set_cursor_inside(&mut self, inside: bool) {
        self.world.resource_mut::<PhysicalInput>().cursor_inside = inside;
    }

    fn set_window_focused(&mut self, focused: bool) {
        self.world.resource_mut::<PhysicalInput>().window_focused = focused;
    }

    fn hovered(&self) -> Option<Entity> {
        self.world.resource::<HoveredWorldEntity>().0
    }

    fn is_occluded(&self, root: Entity) -> bool {
        self.world.is_enabled::<UiWorldOccluded>(root)
    }

    /// Spawns a SCENE entity (a pickable target) with a `GlobalTransform` at `pos`
    /// and a `UiPickable` of the given shape + layers. The pick reads
    /// `GlobalTransform` directly (no propagation is wired here).
    fn spawn_pickable(&mut self, pos: Vec3, shape: UiPickShape, layers: u32) -> Entity {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(
            pos,
            Quat::IDENTITY,
            Vec3::ONE,
        ));
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(UiPickable { shape, layers });
            ec.insert(gt);
            ec.id()
        })
    }

    /// A unit-sphere pickable (radius 0.5) on all layers at `pos`.
    fn spawn_sphere(&mut self, pos: Vec3, radius: f32) -> Entity {
        self.spawn_pickable(pos, UiPickShape::Sphere { radius }, u32::MAX)
    }

    /// A scene entity WITHOUT a UiPickable (a tracked target for an EntityAnchor).
    fn spawn_scene_entity(&mut self, pos: Vec3) -> Entity {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(
            pos,
            Quat::IDENTITY,
            Vec3::ONE,
        ));
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(Transform::from_translation(pos));
            ec.insert(gt);
            ec.id()
        })
    }

    /// Spawns a world-anchor UI root tracking `target` (EntityAnchor) with the
    /// given `depth_test`.
    fn spawn_entity_anchor_root(&mut self, target: Entity, depth_test: bool) -> Entity {
        let anchor = UiWorldAnchor {
            target: WorldTarget::EntityAnchor(target),
            depth_test,
            ..Default::default()
        };
        spawn_anchor_root(&mut self.world, anchor)
    }

    /// Spawns a world-anchor UI root at a fixed `WorldPos` with the given
    /// `depth_test`.
    fn spawn_world_pos_root(&mut self, pos: [f32; 3], depth_test: bool) -> Entity {
        let anchor = UiWorldAnchor {
            target: WorldTarget::WorldPos(pos),
            depth_test,
            ..Default::default()
        };
        spawn_anchor_root(&mut self.world, anchor)
    }

    fn despawn(&mut self, e: Entity) {
        despawn_via_commands(&mut self.world, e);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 10. pick hits nearest
// ════════════════════════════════════════════════════════════════════════════

/// Two pickables on the center ray at different depths → the NEARER one wins.
#[test]
fn gate_pick_hits_nearest() {
    let mut h = PickHarness::new();
    let near = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    let _far = h.spawn_sphere(Vec3::new(0.0, 0.0, -9.0), 0.5);
    h.run();
    assert_eq!(h.hovered(), Some(near), "the nearer pickable on the cursor ray is hovered");
}

// ════════════════════════════════════════════════════════════════════════════
// 11. pick miss → None
// ════════════════════════════════════════════════════════════════════════════

/// A cursor ray that misses every bound → HoveredWorldEntity(None).
#[test]
fn gate_pick_miss_none() {
    let mut h = PickHarness::new();
    // A pickable far off to the side of the world; the center ray misses it.
    let _p = h.spawn_sphere(Vec3::new(50.0, 0.0, -5.0), 0.5);
    h.run();
    assert_eq!(h.hovered(), None, "a center ray that misses all bounds hovers nothing");
}

// ════════════════════════════════════════════════════════════════════════════
// 12. cursor inactive → None + all occlusion cleared (early return)
// ════════════════════════════════════════════════════════════════════════════

/// `cursor_inside == false` → hover None AND every root's UiWorldOccluded cleared,
/// regardless of any occluder. Same for `window_focused == false`.
#[test]
fn gate_cursor_inactive_none_and_occlusion_cleared() {
    let mut h = PickHarness::new();
    // A depth_test root + an occluder in front of it; an active run would set the bit.
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    let _occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    let _hit = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);

    // First, an ACTIVE run sets hover + (with the occluder) the bit, proving the
    // inactive path actually clears something.
    h.run();
    assert!(h.hovered().is_some(), "an active run hovers the front pickable");
    assert!(h.is_occluded(root), "an active run sets the occlusion bit (occluder in front)");

    // Cursor leaves the window → hover None + bit cleared, early return.
    h.set_cursor_inside(false);
    h.run();
    assert_eq!(h.hovered(), None, "cursor-outside clears the hover");
    assert!(!h.is_occluded(root), "cursor-outside clears every occlusion bit (C2)");

    // Window loses focus (cursor back inside) → same clear-all behavior.
    h.set_cursor_inside(true);
    h.run();
    assert!(h.is_occluded(root), "re-active run re-sets the bit");
    h.set_window_focused(false);
    h.run();
    assert_eq!(h.hovered(), None, "unfocused clears the hover");
    assert!(!h.is_occluded(root), "unfocused clears every occlusion bit (C2)");
}

// ════════════════════════════════════════════════════════════════════════════
// 13. gate_pick_resolves_to_anchor_root — the §1 contract (end-to-end)
// ════════════════════════════════════════════════════════════════════════════

/// UiPickable on the SCENE entity S (the EntityAnchor target). Picking S writes
/// Some(S) into HoveredWorldEntity; the visibility system then SHOWS the root that
/// tracks S (UiWorldHidden clear) and hides other entity-tracking roots. Proves
/// the contract: UiPickable belongs on the scene entity, not the UI root.
#[test]
fn gate_pick_resolves_to_anchor_root() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    world.insert_resource(origin_view());
    world.insert_resource(UiViewport { width: VP_W, height: VP_H, scale_factor: 1.0, generation: 0 });
    let mut physical = PhysicalInput::new();
    physical.cursor_pos = [CENTER_PX, CENTER_PY];
    world.insert_resource(physical);
    world.insert_resource(HoveredWorldEntity::default());
    world.insert_resource(UiWorldHoverState::default());

    // S: a pickable scene entity dead-ahead on the cursor ray.
    let s_pos = Vec3::new(0.0, 0.0, -5.0);
    let s = {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(s_pos, Quat::IDENTITY, Vec3::ONE));
        spawn_via_commands(&mut world, move |cmds| {
            let mut ec = cmds.spawn(UiPickable { shape: UiPickShape::Sphere { radius: 1.0 }, layers: u32::MAX });
            ec.insert(gt);
            ec.id()
        })
    };
    // A SECOND scene entity OFF the ray (so its root must end hidden).
    let other = {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(Vec3::new(50.0, 0.0, -5.0), Quat::IDENTITY, Vec3::ONE));
        spawn_via_commands(&mut world, move |cmds| {
            let mut ec = cmds.spawn(Transform::from_translation(Vec3::new(50.0, 0.0, -5.0)));
            ec.insert(gt);
            ec.id()
        })
    };

    // The UI roots: one tracks S, one tracks `other`.
    let root_s = spawn_anchor_root(&mut world, UiWorldAnchor { target: WorldTarget::EntityAnchor(s), ..Default::default() });
    let root_other = spawn_anchor_root(&mut world, UiWorldAnchor { target: WorldTarget::EntityAnchor(other), ..Default::default() });

    // pick → visibility in a real schedule.
    let mut builder = ScheduleBuilder::new(pool);
    let pick = builder.add_system(ui_world_pick_system).key();
    builder.add_system(ui_world_visibility_system).after(pick);
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);

    assert_eq!(world.resource::<HoveredWorldEntity>().0, Some(s), "picking S writes Some(S) into the hover seam");
    assert!(!world.is_enabled::<UiWorldHidden>(root_s), "the root tracking the picked scene entity S is SHOWN");
    assert!(world.is_enabled::<UiWorldHidden>(root_other), "the root tracking the non-picked entity is hidden");
}

// ════════════════════════════════════════════════════════════════════════════
// 14. layer mask
// ════════════════════════════════════════════════════════════════════════════

/// A pickable whose `layers` does not intersect the pick mask (u32::MAX) is
/// skipped. Since the pick mask is u32::MAX (pick-all), only `layers == 0` is fully
/// masked out; assert a `layers == 0` pickable is never hovered even dead-ahead,
/// while an all-layers one behind it IS.
#[test]
fn gate_layer_mask_excludes_zero_layer() {
    let mut h = PickHarness::new();
    // A `layers == 0` pickable NEAR on the ray (would win on depth if not masked).
    let _masked = h.spawn_pickable(Vec3::new(0.0, 0.0, -4.0), UiPickShape::Sphere { radius: 0.5 }, 0);
    // An all-layers pickable FARTHER on the ray.
    let visible = h.spawn_pickable(Vec3::new(0.0, 0.0, -9.0), UiPickShape::Sphere { radius: 0.5 }, u32::MAX);
    h.run();
    assert_eq!(
        h.hovered(),
        Some(visible),
        "the layers==0 pickable is masked out; the all-layers one (farther) is hovered"
    );
}

/// Sanity (non-vacuous): the SAME masked pickable WOULD win if it were on all
/// layers — proving the mask is what excluded it, not its depth/position.
#[test]
fn gate_layer_mask_non_vacuous() {
    let mut h = PickHarness::new();
    let near_all = h.spawn_pickable(Vec3::new(0.0, 0.0, -4.0), UiPickShape::Sphere { radius: 0.5 }, u32::MAX);
    let _far = h.spawn_pickable(Vec3::new(0.0, 0.0, -9.0), UiPickShape::Sphere { radius: 0.5 }, u32::MAX);
    h.run();
    assert_eq!(h.hovered(), Some(near_all), "with all-layers the near pickable wins (the mask was the cause)");
}

// ════════════════════════════════════════════════════════════════════════════
// 15. occluder in front → bit set
// ════════════════════════════════════════════════════════════════════════════

/// A depth_test root whose anchor point has a nearer UiPickable surface between it
/// and the eye → UiWorldOccluded set.
#[test]
fn gate_occlusion_occluder_in_front() {
    let mut h = PickHarness::new();
    // Anchor point at z=-8 (tracked entity, NOT pickable, so no self-issue).
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    // An occluder pickable at z=-4 (between eye and the anchor point).
    let _occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    h.run();
    assert!(h.is_occluded(root), "a nearer pickable in front of the anchor occludes the root");
}

// ════════════════════════════════════════════════════════════════════════════
// 16. clear path → bit cleared
// ════════════════════════════════════════════════════════════════════════════

/// With NOTHING in front of the anchor point, the depth_test root is NOT occluded.
#[test]
fn gate_occlusion_clear_path() {
    let mut h = PickHarness::new();
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    // A pickable BEHIND the anchor point (z=-12, farther than the anchor at -8).
    let _behind = h.spawn_sphere(Vec3::new(0.0, 0.0, -12.0), 0.5);
    h.run();
    assert!(!h.is_occluded(root), "a pickable behind the anchor does not occlude it");
}

// ════════════════════════════════════════════════════════════════════════════
// 17. depth_test == false → never occluded
// ════════════════════════════════════════════════════════════════════════════

/// A `depth_test == false` root with an occluder in front is NEVER occluded
/// (always-on-top overlay).
#[test]
fn gate_occlusion_depth_test_false_never() {
    let mut h = PickHarness::new();
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, false); // depth_test OFF
    let _occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    h.run();
    assert!(!h.is_occluded(root), "a depth_test==false root is never occluded (overlay)");
}

// ════════════════════════════════════════════════════════════════════════════
// 18. self-target excluded (EntityAnchor); WorldPos occludes (O1)
// ════════════════════════════════════════════════════════════════════════════

/// An EntityAnchor root whose tracked target IS itself a pickable: the target's
/// own surface in front of (around) its anchor point does NOT occlude its label
/// (self-exclusion). With NO other occluder the bit stays clear.
#[test]
fn gate_occlusion_self_target_excluded() {
    let mut h = PickHarness::new();
    // The tracked entity is pickable (a big sphere at its own anchor point).
    let target = h.spawn_sphere(Vec3::new(0.0, 0.0, -8.0), 2.0);
    let root = h.spawn_entity_anchor_root(target, true);
    h.run();
    assert!(
        !h.is_occluded(root),
        "an EntityAnchor's own pickable surface does not self-occlude its label"
    );
}

/// Non-vacuous companion to self-exclusion: a SECOND, non-self occluder in front
/// of the same anchor point DOES occlude (so the exclusion above is the cause).
#[test]
fn gate_occlusion_self_excluded_but_other_occludes() {
    let mut h = PickHarness::new();
    let target = h.spawn_sphere(Vec3::new(0.0, 0.0, -8.0), 2.0);
    let root = h.spawn_entity_anchor_root(target, true);
    // A DIFFERENT pickable strictly nearer than the anchor point.
    let _other = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    h.run();
    assert!(h.is_occluded(root), "a non-self occluder in front DOES occlude (self-exclusion isolated)");
}

/// O1: a WorldPos anchor has NO self-exclusion — a front pickable on its anchor
/// point DOES occlude its label.
#[test]
fn gate_occlusion_worldpos_no_self_exclusion() {
    let mut h = PickHarness::new();
    // A fixed WorldPos anchor at z=-8.
    let root = h.spawn_world_pos_root([0.0, 0.0, -8.0], true);
    // A pickable in front of it (z=-4).
    let _occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    h.run();
    assert!(h.is_occluded(root), "a WorldPos anchor has no self-exclusion; a front pickable occludes it (O1)");
}

// ════════════════════════════════════════════════════════════════════════════
// 19. gate_layout_skips_occluded_root
// ════════════════════════════════════════════════════════════════════════════

/// An occluded root is SKIPPED by the layout pass (no ComputedRect written to the
/// projected origin); once the occluder is gone the root is laid out. Driven
/// through a real schedule: project → pick → layout discovery → layout apply.
#[test]
fn gate_layout_skips_occluded_root() {
    let mut h = LayoutHarness::new();
    // A depth_test root tracking an on-screen, in-front anchor point.
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    // An occluder in front of the anchor point.
    let occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);

    h.run();
    assert!(h.is_occluded(root), "the root is occluded with the occluder present");
    assert!(h.projection(root).visible, "the anchor itself is on-screen / in-front (visible projection)");
    let rect_occluded = h.rect(root);
    // While occluded the rect was NOT positioned at the projected center.
    let (cx, cy) = (VP_W * 0.5, VP_H * 0.5); // anchor on the -z axis ⇒ projects to center
    assert!(
        (rect_occluded.x - cx).abs() > 1.0 || (rect_occluded.y - cy).abs() > 1.0,
        "an occluded root is not laid out at the projected origin (rect {rect_occluded:?})"
    );

    // Remove the occluder; the root is no longer occluded → laid out at center.
    h.despawn(occ);
    h.tick();
    h.run();
    assert!(!h.is_occluded(root), "removing the occluder clears the bit");
    h.run(); // one settle frame for the project→layout one-frame seam
    let rect_vis = h.rect(root);
    approx(rect_vis.x, cx, 1.0e-2, "un-occluded root rect at projected center x");
    approx(rect_vis.y, cy, 1.0e-2, "un-occluded root rect at projected center y");
}

// ════════════════════════════════════════════════════════════════════════════
// 21. gate_no_pickable — always-walk stays write-free
// ════════════════════════════════════════════════════════════════════════════

/// Anchors present, NO UiPickable: the occlusion walk still runs and every root is
/// re-derived to not-occluded; a still second run leaves the hover unchanged and
/// writes nothing new (set-if-changed). Proves C2's always-walk stays write-free.
#[test]
fn gate_no_pickable_walk_is_write_free() {
    let mut h = PickHarness::new();
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);

    // No UiPickable in the world at all.
    assert!(
        h.world.query_entities(&[UiPickable::component_id()]).is_empty(),
        "no pickables ⇒ the pick loop never picks"
    );

    h.run();
    assert_eq!(h.hovered(), None, "no pickables ⇒ nothing hovered");
    assert!(!h.is_occluded(root), "no pickables ⇒ the always-walk re-derives the root to not-occluded");

    // Several MORE still runs: the always-walk re-derives the bit each time but the
    // observable state is invariant — the hover stays None and the root stays
    // not-occluded (the set-if-changed + disable-already-clear are write-free for
    // the unchanged value). NOTE: `EcsMaster` exposes no per-RESOURCE changed-tick
    // accessor, so write-freedom is asserted via the value-stability contract the
    // set-if-changed guard guarantees, not via a tick read.
    for i in 0..4 {
        h.run();
        assert_eq!(h.hovered(), None, "still run #{i}: hover stays None");
        assert!(!h.is_occluded(root), "still run #{i}: root stays not-occluded (always-walk re-derives, no flip)");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 22. depth populated — UiWorldProjection.depth == ndc_z ∈ [0,1]
// ════════════════════════════════════════════════════════════════════════════

/// After the project system runs, a visible anchor's UiWorldProjection.depth is
/// the projected ndc_z and lies in [0, 1] (the field that was previously
/// discarded; P7b/W1).
#[test]
fn gate_depth_field_populated_in_unit_range() {
    let mut world = EcsMaster::new();
    world.insert_resource(origin_view());
    world.insert_resource(UiViewport { width: VP_W, height: VP_H, scale_factor: 1.0, generation: 0 });

    let root = spawn_anchor_root(&mut world, UiWorldAnchor { target: WorldTarget::WorldPos([0.0, 0.0, -10.0]), ..Default::default() });
    world.run_system(ui_world_project_system);

    let proj = *world.get_component::<UiWorldProjection>(root).expect("anchor has a projection");
    assert!(proj.visible, "an in-front on-screen anchor is visible");
    assert!(
        (0.0..=1.0).contains(&proj.depth),
        "the projected depth (ndc_z) is in [0,1]: {}",
        proj.depth
    );
    // It is NOT the default far-plane neutral 1.0 (a real projection was written).
    assert!(proj.depth < 1.0, "a real in-frustum depth is < the far-plane neutral (1.0): {}", proj.depth);
}

// ════════════════════════════════════════════════════════════════════════════
// 23. gate_no_stale_occluded_bit — the C2 regression guard
// ════════════════════════════════════════════════════════════════════════════

/// Occlude a root (occluder in front, run pick → bit set), then DESPAWN the
/// occluder and run pick again → the bit is CLEARED. The stranded-bit guard
/// (gate-5 eviction class): a pickable that despawned after setting a bit must not
/// leave the root permanently occluded.
#[test]
fn gate_no_stale_occluded_bit_after_despawn() {
    let mut h = PickHarness::new();
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    let occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);

    h.run();
    assert!(h.is_occluded(root), "the root is occluded with the occluder present");

    // Despawn the occluder; the next pick run must re-derive the bit to clear.
    h.despawn(occ);
    h.run();
    assert!(!h.is_occluded(root), "despawning the occluder clears the (would-be) stale bit (C2)");
}

/// The cursor-moves-off variant of the C2 guard: occlude, then move the cursor so
/// the system stays active but re-derives the bit — still re-walked + cleared once
/// the occluder is gone. (Distinct from the inactive-path clear in test 12.)
#[test]
fn gate_no_stale_occluded_bit_after_occluder_moves() {
    let mut h = PickHarness::new();
    let target = h.spawn_scene_entity(Vec3::new(0.0, 0.0, -8.0));
    let root = h.spawn_entity_anchor_root(target, true);
    // Spawn the occluder, occlude, then move it off the eye→anchor line by editing
    // its GlobalTransform (the occlusion ray is eye→anchor, fixed).
    let occ = h.spawn_sphere(Vec3::new(0.0, 0.0, -4.0), 0.5);
    h.run();
    assert!(h.is_occluded(root), "occluded while the occluder sits on the eye→anchor line");

    // Move the occluder far to the side (off the eye→anchor ray).
    if let Some(mut gt) = h.world.get_component_mut::<GlobalTransform>(occ) {
        gt.0.translation = Vec3::new(50.0, 0.0, -4.0);
    }
    h.run();
    assert!(!h.is_occluded(root), "moving the occluder off the eye→anchor line clears the bit (C2)");
}

// ════════════════════════════════════════════════════════════════════════════
// 20. struct/layout const asserts — confirmed at BUILD time
// ════════════════════════════════════════════════════════════════════════════

/// The component layout pins (`size_of::<UiWorldProjection>() == 24`, align 4;
/// `UiPickable == 20`, `UiPickShape == 16`; `UiWorldAnchor == 56`) are `const _`
/// assertions inside `components.rs`, so they are enforced when this crate (and
/// hence this test) compiles. This test merely re-states a couple at runtime as a
/// living, greppable record alongside the matrix; if `components.rs`'s asserts
/// ever weaken, the build fails before this runs.
#[test]
fn gate_struct_layout_pins() {
    assert_eq!(size_of::<UiWorldProjection>(), 24, "UiWorldProjection is 24 B (depth field added)");
    assert_eq!(align_of::<UiWorldProjection>(), 4, "UiWorldProjection align 4");
    assert_eq!(size_of::<UiPickable>(), 20, "UiPickable is 20 B");
    assert_eq!(align_of::<UiPickable>(), 4, "UiPickable align 4");
    assert_eq!(size_of::<UiPickShape>(), 16, "UiPickShape is 16 B");
    assert_eq!(size_of::<UiWorldAnchor>(), 56, "UiWorldAnchor unchanged at 56 B");
}

// ════════════════════════════════════════════════════════════════════════════
// LayoutHarness — project → pick → layout discovery → layout apply (real Schedule)
// ════════════════════════════════════════════════════════════════════════════

/// Wires `ui_world_project_system` → `ui_world_pick_system` → `ui_layout_discovery`
/// → `ui_layout_apply` so an occluded root's layout-skip is observable. The view +
/// cursor are still (the Changed-window vehicle, like the p7a SchedHarness).
struct LayoutHarness {
    world: EcsMaster,
    schedule: Schedule,
    ticker: Schedule,
}

fn noop_exclusive(_world: &mut EcsMaster) {}

impl LayoutHarness {
    fn new() -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();
        world.insert_resource(origin_view());
        world.insert_resource(UiViewport { width: VP_W, height: VP_H, scale_factor: 1.0, generation: 0 });
        let mut physical = PhysicalInput::new();
        physical.cursor_pos = [CENTER_PX, CENTER_PY];
        world.insert_resource(physical);
        world.insert_resource(HoveredWorldEntity::default());
        world.insert_resource(UiWorldHoverState::default());
        world.insert_resource(UiSafeArea::default());
        world.insert_resource(LayoutScratch::with_seeds());

        let mut builder = ScheduleBuilder::new(pool);
        let proj = builder.add_system(ui_world_project_system).key();
        let pick = builder.add_system(ui_world_pick_system).after(proj).key();
        let disc = builder.add_system(ui_layout_discovery).after(pick).key();
        builder.add_system(ui_layout_apply).after(disc);
        let schedule = builder.build(&mut world);

        let mut tbuilder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
        tbuilder.add_system(noop_exclusive);
        let mut ticker = tbuilder.build(&mut world);
        ticker.run(&mut world); // lift the tick off ZERO before any spawn

        Self { world, schedule, ticker }
    }

    fn tick(&mut self) {
        self.ticker.run(&mut self.world);
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn is_occluded(&self, root: Entity) -> bool {
        self.world.is_enabled::<UiWorldOccluded>(root)
    }

    fn spawn_scene_entity(&mut self, pos: Vec3) -> Entity {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(pos, Quat::IDENTITY, Vec3::ONE));
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(Transform::from_translation(pos));
            ec.insert(gt);
            ec.id()
        })
    }

    fn spawn_entity_anchor_root(&mut self, target: Entity, depth_test: bool) -> Entity {
        spawn_anchor_root(&mut self.world, UiWorldAnchor {
            target: WorldTarget::EntityAnchor(target),
            depth_test,
            scale_mode: WorldScaleMode::ScreenSpace,
            ..Default::default()
        })
    }

    fn spawn_sphere(&mut self, pos: Vec3, radius: f32) -> Entity {
        let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(pos, Quat::IDENTITY, Vec3::ONE));
        spawn_via_commands(&mut self.world, move |cmds| {
            let mut ec = cmds.spawn(UiPickable { shape: UiPickShape::Sphere { radius }, layers: u32::MAX });
            ec.insert(gt);
            ec.id()
        })
    }

    fn despawn(&mut self, e: Entity) {
        despawn_via_commands(&mut self.world, e);
    }

    fn projection(&self, e: Entity) -> UiWorldProjection {
        *self.world.get_component::<UiWorldProjection>(e).expect("anchor has a projection")
    }

    fn rect(&self, e: Entity) -> ComputedRect {
        *self.world.get_component::<ComputedRect>(e).expect("anchor root has a ComputedRect")
    }
}

#[track_caller]
fn approx(a: f32, b: f32, eps: f32, what: &str) {
    assert!((a - b).abs() <= eps, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

// ════════════════════════════════════════════════════════════════════════════
// Shared spawn helpers (the p7a Arc<Mutex<..>> probe pattern)
// ════════════════════════════════════════════════════════════════════════════

/// A definite-size root layout so `layout_root` measures a real box.
fn definite_root_layout() -> UiLayout {
    UiLayout {
        width: Unit::Px(120.0),
        height: Unit::Px(40.0),
        ..Default::default()
    }
}

/// Spawns a world-anchored UI ROOT carrying `anchor` (+ a definite UiLayout,
/// UiRoot, ComputedRect). `#[require(UiWorldProjection)]` auto-inserts the
/// projection column.
fn spawn_anchor_root(world: &mut EcsMaster, anchor: UiWorldAnchor) -> Entity {
    spawn_via_commands(world, move |cmds| {
        let mut ec = cmds.spawn(anchor);
        ec.insert(definite_root_layout());
        ec.insert(UiRoot);
        ec.insert(ComputedRect::default());
        ec.id()
    })
}

/// The Commands + Arc<Mutex<Option<Entity>>> probe: spawn inside a one-shot system,
/// harvest the handle after the apply window.
fn spawn_via_commands<F>(world: &mut EcsMaster, f: F) -> Entity
where
    F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let f = Mutex::new(Some(f));
    world.run_system(move |mut cmds: Commands| {
        let f = f.lock().unwrap().take().expect("spawn closure runs once");
        let e = f(&mut cmds);
        *probe.lock().unwrap() = Some(e);
    });
    let e = sink.lock().unwrap().expect("spawned handle");
    assert!(world.has_entity(e), "spawned entity is live after apply");
    e
}

/// Despawns `e` via a one-shot Commands system (the apply window removes it).
fn despawn_via_commands(world: &mut EcsMaster, e: Entity) {
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert!(!world.has_entity(e), "entity is gone after the despawn apply window");
}
