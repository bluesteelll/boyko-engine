//! std-lib S4-follow-up gate suite — the END-TO-END `visibility_sync` → pack
//! pipeline (boyko_render half).
//!
//! This crate may name BOTH the scene vocabulary (`boyko_scene`: `Visibility` /
//! `RenderEnabled` / `GlobalTransform` / `MaterialHandle` / `visibility_sync`)
//! and the GPU record (`boyko_render`: `Gpu3dInstance` / `sync_gpu_3d_instances`),
//! so the cross-crate end-to-end gate lives here: a `Visibility::Hidden` entity is
//! actually EXCLUDED from the `Gpu3dInstance` pack (its instance is never
//! refreshed), and flipping it back to `Visible` re-includes it — the whole point
//! of the S4 follow-up (`Visibility::Hidden` ALONE now hides).
//!
//! The bit-level mechanics (`Changed`-gating, 0%-work, manual-toggle-not-fought)
//! are proven in `boyko_scene/tests/visibility_sync_gates.rs`; here we prove the
//! observable RENDER consequence by running the real two-system chain
//! `visibility_sync` → `sync_gpu_3d_instances` and reading the packed column.
//!
//! # Ordering contract (the cross-crate edge the docs require)
//!
//! `visibility_sync`'s deferred `RenderEnabled` toggle flips at the apply window
//! AFTER its body returns, so for a Hidden entity to be excluded the SAME frame,
//! `visibility_sync` (and its apply window) must run BEFORE
//! `sync_gpu_3d_instances` (which filters `Enabled<RenderEnabled>`). This test
//! wires that edge EXPLICITLY (`sync_gpu_3d_instances.after(visibility_sync)`),
//! exactly the contract `TransformPlugin` / `visibility_sync` document for a host
//! that adds `Render3dPlugin` alongside `TransformPlugin`.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Mut, Query};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_math::{Affine3A, Mat3, Vec3};

use boyko_render::gpu3d_instance::Gpu3dInstance;
use boyko_render::gpu3d_system::sync_gpu_3d_instances;

use boyko_scene::render_caps::Visibility;
use boyko_scene::transform::GlobalTransform;
use boyko_scene::visibility_sync::visibility_sync;
use boyko_scene::MaterialHandle;

use bytemuck::Zeroable;

// ── byte helper ─────────────────────────────────────────────────────────────

/// Views a `#[repr(C/transparent/u8)]` POD as raw bytes for the spawn path.
///
/// # Safety
/// `T` is a fixed-layout `#[repr]` component whose byte image is a valid
/// serialization for its pool (holds for every component spawned here).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only.
    // `T` is fixed-layout, matching the pool's stored layout; the slice borrows
    // `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn global_from(matrix3: Mat3, translation: Vec3) -> GlobalTransform {
    GlobalTransform(Affine3A { matrix3, translation })
}

// ── pending Visibility write (drives a Changed inside the schedule) ──────────

#[derive(boyko_macros::Resource, Default)]
struct PendingVis {
    target: Option<Entity>,
    value: Visibility,
}

#[allow(clippy::needless_pass_by_value)]
fn apply_pending_vis(mut pending: ResMut<PendingVis>, mut q: Query<Mut<Visibility>>) {
    let Some(target) = pending.target.take() else { return };
    let v = pending.value;
    for (id, mut vis) in q.iter_entities_mut() {
        if id == target.id() {
            *vis = v;
        }
    }
}

fn queue_vis(world: &mut EcsMaster, target: Entity, value: Visibility) {
    let pv = world.resource_mut::<PendingVis>();
    pv.target = Some(target);
    pv.value = value;
}

// ── harness ──────────────────────────────────────────────────────────────────

/// A renderable carrying the pack inputs PLUS the durable `Visibility` byte.
fn renderable_vis_archetype(world: &mut EcsMaster) -> ArchetypeId {
    world.create_archetype(&[
        GlobalTransform::component_id(),
        MaterialHandle::component_id(),
        Gpu3dInstance::component_id(),
        Visibility::component_id(),
    ])
}

/// Spawns a renderable with a distinctive transform, a zeroed `Gpu3dInstance`,
/// and the given `Visibility`. The `RenderEnabled` bit starts CLEAR (a bitset tag
/// is unset until toggled) — `visibility_sync` is what sets it for a visible row.
fn spawn_renderable_vis(
    world: &mut EcsMaster,
    arch: ArchetypeId,
    global: GlobalTransform,
    mat: u16,
    vis: Visibility,
) -> Entity {
    let zero = Gpu3dInstance::zeroed();
    let mh = MaterialHandle(mat);
    world
        .create_entity(
            arch,
            &[
                (GlobalTransform::component_id(), as_bytes(&global)),
                (MaterialHandle::component_id(), as_bytes(&mh)),
                (Gpu3dInstance::component_id(), as_bytes(&zero)),
                (Visibility::component_id(), as_bytes(&vis)),
            ],
        )
        .expect("invariant: renderable+visibility archetype accepts its four columns")
}

/// Builds the real per-frame chain: `apply_pending_vis` (mutator) →
/// `visibility_sync` (byte → bit) → `sync_gpu_3d_instances` (pack), all ordered.
/// This is the host's documented add-order (visibility bridge BEFORE the pack).
fn build_pipeline(world: &mut EcsMaster) -> Schedule {
    world.insert_resource(PendingVis::default());
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let mutate = b.add_system(apply_pending_vis).key();
    let bridge = b.add_system(visibility_sync).after(mutate).key();
    b.add_system(sync_gpu_3d_instances).after(bridge);
    b.build(world)
}

fn inst(world: &EcsMaster, e: Entity) -> Gpu3dInstance {
    *world.get_component::<Gpu3dInstance>(e).expect("Gpu3dInstance lives")
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — a Visibility::Hidden entity is EXCLUDED from the pack (the byte alone
//          hides it; no manual disable needed); a Visible one IS packed.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn hidden_visibility_excludes_from_pack_visible_is_packed() {
    let mut world = EcsMaster::new();
    let arch = renderable_vis_archetype(&mut world);

    // Distinctive translations so a packed write is unmistakable.
    let visible = spawn_renderable_vis(
        &mut world,
        arch,
        global_from(Mat3::IDENTITY, Vec3::new(1.0, 0.0, 0.0)),
        1,
        Visibility::Visible,
    );
    let hidden = spawn_renderable_vis(
        &mut world,
        arch,
        global_from(Mat3::IDENTITY, Vec3::new(9.0, 9.0, 9.0)),
        2,
        Visibility::Hidden,
    );

    let mut sched = build_pipeline(&mut world);
    // Frame 0: visibility_sync reconciles both spawns' Added Visibility (enable
    // the Visible row, leave the Hidden row clear) in its apply window, THEN the
    // pack — ordered after it — packs only the enabled row.
    sched.run(&mut world);

    assert_eq!(
        inst(&world, visible).translation,
        [1.0, 0.0, 0.0],
        "the Visible row was packed from its GlobalTransform"
    );
    assert_eq!(
        inst(&world, hidden),
        Gpu3dInstance::zeroed(),
        "Visibility::Hidden ALONE excludes the row from the pack — its Gpu3dInstance stays \
         zeroed (the S4-follow-up: setting the byte hides, no manual disable needed)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — toggle BOTH directions through the pack: Hidden → Visible re-includes
//          (the row starts being packed); Visible → Hidden excludes it again.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn pack_inclusion_tracks_visibility_both_directions() {
    let mut world = EcsMaster::new();
    let arch = renderable_vis_archetype(&mut world);
    let e = spawn_renderable_vis(
        &mut world,
        arch,
        global_from(Mat3::IDENTITY, Vec3::new(2.0, 3.0, 4.0)),
        7,
        Visibility::Hidden,
    );

    let mut sched = build_pipeline(&mut world);

    // Frame 0: spawned Hidden → bit clear → NOT packed (stays zeroed).
    sched.run(&mut world);
    assert_eq!(
        inst(&world, e),
        Gpu3dInstance::zeroed(),
        "spawned Hidden ⇒ excluded from the pack (zeroed)"
    );

    // Hidden → Visible: the byte change drives the bit ON, so the pack — running
    // after the bridge — now includes it and writes its transform.
    queue_vis(&mut world, e, Visibility::Visible);
    sched.run(&mut world);
    assert_eq!(
        inst(&world, e).translation,
        [2.0, 3.0, 4.0],
        "Hidden → Visible ⇒ the row is RE-INCLUDED and packed from its transform"
    );

    // Visible → Hidden: the bit goes OFF; the pack excludes it. We first STOMP the
    // packed instance to a sentinel so that "not refreshed" is observable — if the
    // pack still touched it, the sentinel would be overwritten by the transform.
    {
        let mut g = world.get_component_mut::<Gpu3dInstance>(e).expect("inst lives");
        g.translation = [123.0, 123.0, 123.0];
    }
    queue_vis(&mut world, e, Visibility::Hidden);
    sched.run(&mut world);
    assert_eq!(
        inst(&world, e).translation,
        [123.0, 123.0, 123.0],
        "Visible → Hidden ⇒ EXCLUDED again: the pack does NOT refresh the row (the \
         sentinel survives, proving the Hidden row was skipped)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — a MOVING Hidden entity is never refreshed (Hidden truly excludes from
//          the GlobalTransform-driven pack, the render consequence of S4).
// ════════════════════════════════════════════════════════════════════════════

/// Drives the GlobalTransform-write inside the schedule so the pack (had the row
/// been enabled) would observe a new transform. `Some` ⇒ apply this frame.
#[derive(boyko_macros::Resource, Default)]
struct PendingMove {
    target: Option<Entity>,
    global: GlobalTransform,
}

#[allow(clippy::needless_pass_by_value)]
fn apply_pending_move(mut pending: ResMut<PendingMove>, mut q: Query<Mut<GlobalTransform>>) {
    let Some(target) = pending.target.take() else { return };
    let g = pending.global;
    for (id, mut gt) in q.iter_entities_mut() {
        if id == target.id() {
            *gt = g;
        }
    }
}

#[test]
fn moving_hidden_entity_is_never_packed() {
    let mut world = EcsMaster::new();
    world.insert_resource(PendingVis::default());
    world.insert_resource(PendingMove::default());
    let arch = renderable_vis_archetype(&mut world);
    let e = spawn_renderable_vis(
        &mut world,
        arch,
        global_from(Mat3::IDENTITY, Vec3::new(0.0, 0.0, 0.0)),
        3,
        Visibility::Hidden,
    );

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let mv = b.add_system(apply_pending_move).key();
    let pv = b.add_system(apply_pending_vis).after(mv).key();
    let bridge = b.add_system(visibility_sync).after(pv).key();
    b.add_system(sync_gpu_3d_instances).after(bridge);
    let mut sched = b.build(&mut world);

    // Frame 0: spawned Hidden → bit clear → not packed.
    sched.run(&mut world);
    assert_eq!(inst(&world, e), Gpu3dInstance::zeroed(), "spawned Hidden starts unpacked");

    // MOVE the Hidden entity each of several frames; the pack must NEVER refresh
    // its instance (the row's bit is clear, the byte never changed to Visible).
    for f in 1..=4 {
        let p = world.resource_mut::<PendingMove>();
        p.target = Some(e);
        p.global = global_from(Mat3::IDENTITY, Vec3::new(f as f32, f as f32, f as f32));
        sched.run(&mut world);
        assert_eq!(
            inst(&world, e),
            Gpu3dInstance::zeroed(),
            "frame {f}: a MOVING Hidden entity is never packed — its instance stays zeroed"
        );
    }

    // Now SHOW it (Visibility::Visible) and move it once more: the pack picks it
    // up (anti-vacuity — the pack is not simply broken).
    queue_vis(&mut world, e, Visibility::Visible);
    {
        let p = world.resource_mut::<PendingMove>();
        p.target = Some(e);
        p.global = global_from(Mat3::IDENTITY, Vec3::new(5.0, 6.0, 7.0));
    }
    sched.run(&mut world);
    assert_eq!(
        inst(&world, e).translation,
        [5.0, 6.0, 7.0],
        "once shown (Visible), the row IS packed from its current transform (anti-vacuity)"
    );
}
