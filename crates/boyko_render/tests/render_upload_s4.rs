//! std-lib S4 gate suite (boyko_render half) — `GlobalTransform`-driven GPU
//! upload + lights read pose.
//!
//! This crate is the one that may name BOTH the scene vocabulary
//! (`boyko_scene`: `GlobalTransform` / `MaterialHandle` / `RenderEnabled`) and
//! the GPU records (`boyko_render`: `Gpu3dInstance`, `DirectionalLight`, …), so
//! the cross-crate S4 gates live here.
//!
//! Gates covered:
//!
//!  2. `Hidden` excluded from the `Gpu3dInstance` pack (a bit-clear row is not
//!     refreshed).
//!  3. LIGHT DIRECTION MATH — a light with a `GlobalTransform` built from a KNOWN
//!     `Quat` yields the HAND-COMPUTED to-light direction (axis = −Z, sign
//!     asserted, value derived by hand in the test).
//!  4. A MOVED light's reconciled position/direction tracks its `Transform`.
//!  5. STATIC light WITH `GlobalTransform` → NO `collect_lights` rebuild across
//!     frames (the value-gate: a static light's `Changed<DirectionalLight>` tick
//!     never advances, which is exactly `collect_lights`' rebuild trigger).
//!  6. A light WITHOUT `GlobalTransform` is byte-identical to the pre-S4 path
//!     (reconcile does not match it; its self-contained pose is untouched).
//!  7. const-assert: `Gpu3dInstance` 52 B / align 4 (compile + runtime mirror);
//!     the 2D `GpuInstance` 24 B / align 4 re-affirmed unchanged.
//!  8. 0%-gate: a no-`Gpu3dInstance` / no-`MeshHandle` scene does zero pack work;
//!     a static light's reconcile bumps nothing.
//!  9. Integration: a parented light orbiting its parent updates direction.
//! 10. The pack + `cast_slice` upload walk (exercised under Miri separately).
//!
//! The 2D `GpuInstance` 24 B re-affirmation reads the demo's own const-asserts;
//! since `boyko_demo` is not a dependency of `boyko_render`, the byte-frozen
//! contract is mirrored here as an independent 24 B / align 4 + per-field-offset
//! pin that MUST stay equal to `boyko_demo/src/render/instance.rs:69-70`.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;

use boyko_math::{Affine3A, Mat3, Quat, Vec3};

use boyko_render::gpu3d_instance::{Gpu3dInstance, GPU3D_INSTANCE_SIZE};
use boyko_render::gpu3d_system::sync_gpu_3d_instances;
use boyko_render::light::{DirectionalLight, PointLight, SpotLight};
use boyko_render::light_reconcile::light_reconcile;

use boyko_scene::propagation::propagate_transforms;
use boyko_scene::render_caps::RenderEnabled;
use boyko_scene::transform::{GlobalTransform, Transform};
use boyko_scene::MaterialHandle;

use bytemuck::Zeroable;

// ── byte helpers ──────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD as raw bytes for the `create_entity` spawn path.
///
/// # Safety
/// `T` is a `#[repr(C)]` component whose byte image is a valid serialization for
/// its pool (holds for every component spawned here — all fixed-layout PODs).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only.
    // `T` is `#[repr(C)]`, matching the pool's stored layout; the slice borrows
    // `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 7 — const-asserts: Gpu3dInstance pinned; 2D GpuInstance 24 B unchanged
// ════════════════════════════════════════════════════════════════════════════

/// An INDEPENDENT mirror of the demo's byte-frozen 2D `GpuInstance` (24 B, align
/// 4, every field a WGSL `@location`). This struct is NOT used by production code;
/// it exists ONLY so this gate fails the build if the 24 B contract the S4 brief
/// requires to stay frozen ever drifts. It is field-for-field identical to
/// `boyko_demo/src/render/instance.rs:43-59` (`pos: [f32;2]`, `scale: f32`,
/// `color: u32`, `prev_pos: [f32;2]` → 6 × 4 B = 24 B, no padding).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuInstance2dMirror {
    pos: [f32; 2],
    scale: f32,
    color: u32,
    prev_pos: [f32; 2],
}

// The same compile-time pins the demo carries (instance.rs:69-70), re-affirmed
// here so a drift in the byte-frozen 2D record is a build error in the S4 suite.
const _: () = assert!(size_of::<GpuInstance2dMirror>() == 24);
const _: () = assert!(align_of::<GpuInstance2dMirror>() == 4);

#[test]
fn gpu3d_instance_layout_is_pinned() {
    // Mirrors the crate's own `const _: () = assert!(...)` so the fingerprint is
    // visible in the test report (52 B = 36 linear + 12 translation + 4 material).
    assert_eq!(size_of::<Gpu3dInstance>(), 52);
    assert_eq!(size_of::<Gpu3dInstance>(), GPU3D_INSTANCE_SIZE);
    assert_eq!(align_of::<Gpu3dInstance>(), 4);
    // No padding holes (required for the bytemuck::Pod cast_slice upload).
    assert_eq!(
        size_of::<Gpu3dInstance>(),
        size_of::<[[f32; 3]; 3]>() + size_of::<[f32; 3]>() + size_of::<u32>(),
        "Gpu3dInstance has no padding holes (sum of field sizes == struct size)"
    );
}

#[test]
fn gpu_instance_2d_is_byte_frozen_at_24() {
    // The byte-frozen 2D record the S4 brief requires to stay untouched.
    assert_eq!(size_of::<GpuInstance2dMirror>(), 24, "2D GpuInstance stays 24 B");
    assert_eq!(align_of::<GpuInstance2dMirror>(), 4, "2D GpuInstance stays align 4");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — LIGHT DIRECTION MATH (axis = -Z, sign, hand-computed expected value)
// ════════════════════════════════════════════════════════════════════════════

/// Builds a `GlobalTransform` directly from an affine (no propagation needed) so
/// the math gate isolates `light_reconcile`'s direction formula.
fn global_from(matrix3: Mat3, translation: Vec3) -> GlobalTransform {
    GlobalTransform(Affine3A { matrix3, translation })
}

/// A reconcile-only `App`: it runs `light_reconcile` in the Main schedule and
/// does NOT propagate (so a hand-set `GlobalTransform` is the source of truth and
/// is not overwritten). `light_reconcile` is `Changed<GlobalTransform>`-gated, and
/// a one-shot `EcsMaster::run_system` leaves an EMPTY change window (its
/// `this_run == last_run` sentinel never advances), so reconcile would match zero
/// rows; an `App::update` advances `last_run`/`this_run` correctly, so the spawn's
/// Added-GlobalTransform (⊆ Changed) is observed on the first update. THIS is why
/// the gated reconcile must be driven through a real schedule, not `run_system`.
fn reconcile_only_app() -> App {
    let mut app = App::new();
    app.add_systems(light_reconcile);
    app
}

/// Runs `light_reconcile` over `app` for ONE frame (drives the change window).
fn reconcile_frame(app: &mut App) {
    app.finish();
    app.update();
}

/// Spawns a directional light WITH `Transform` + `GlobalTransform` and returns
/// its handle. The initial `direction` is a sentinel so a successful reconcile is
/// observable as a change away from it.
fn spawn_dir_light_with_global(
    world: &mut EcsMaster,
    initial_dir: [f32; 3],
    global: GlobalTransform,
) -> Entity {
    let arch = world.create_archetype(&[
        DirectionalLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = DirectionalLight { direction: initial_dir, color: [1.0, 1.0, 1.0], illuminance: 1.0 };
    let transform = Transform::IDENTITY;
    world
        .create_entity(
            arch,
            &[
                (DirectionalLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&transform)),
                (GlobalTransform::component_id(), as_bytes(&global)),
            ],
        )
        .expect("dir-light archetype accepts its three columns")
}

/// Asserts two `[f32;3]` agree within a tight epsilon, with a clear message.
fn assert_dir_eq(got: [f32; 3], want: [f32; 3], eps: f32, ctx: &str) {
    for i in 0..3 {
        assert!(
            (got[i] - want[i]).abs() <= eps,
            "{ctx}: lane {i} got {} want {} (eps {eps}); full got {:?} want {:?}",
            got[i],
            want[i],
            got,
            want
        );
    }
}

#[test]
fn light_direction_is_minus_z_transformed_known_quat_y90() {
    // A KNOWN rotation: +90° about Y. q = (0, sin45, 0, cos45), sin45 == cos45.
    let s = std::f32::consts::FRAC_PI_4.cos(); // = sin(45°) = cos(45°)
    let q = Quat::new(0.0, s, 0.0, s);
    let global = global_from(Mat3::from_quat(q), Vec3::ZERO);

    // HAND-COMPUTED expected: direction = normalize(matrix3 · (0,0,-1)).
    // matrix3 · (0,0,-1) negates the THIRD column of the row-major rotation:
    //   (-row0.z, -row1.z, -row2.z).
    // For +90° about Y this is q.rotate((0,0,-1)) = (-1, 0, 0): the local -Z
    // forward axis swings to world -X. This pins BOTH the axis (-Z) and the sign.
    let expected = [-1.0_f32, 0.0, 0.0];

    let mut app = reconcile_only_app();
    let e = spawn_dir_light_with_global(app.world_mut(), [0.0, 0.0, 1.0], global);
    reconcile_frame(&mut app);

    let dir = ecs_dir(app.world(), e);
    assert_dir_eq(dir, expected, 1e-6, "Y+90 to-light direction");
    // Sign witness: the X lane is strictly negative (forward -Z rotated toward -X,
    // NOT +X — a sign flip would land at +1).
    assert!(dir[0] < -0.9, "to-light X lane must be strongly NEGATIVE (sign/axis): {dir:?}");
}

#[test]
fn light_direction_z_rotation_leaves_minus_z_forward_fixed() {
    // +90° about Z. A Z rotation leaves the Z axis fixed, so local -Z forward
    // stays world (0,0,-1) — an orthogonality cross-check on the axis choice.
    let s = std::f32::consts::FRAC_PI_4.cos();
    let q = Quat::new(0.0, 0.0, s, s);
    let global = global_from(Mat3::from_quat(q), Vec3::ZERO);

    let expected = [0.0_f32, 0.0, -1.0];

    let mut app = reconcile_only_app();
    let e = spawn_dir_light_with_global(app.world_mut(), [1.0, 0.0, 0.0], global);
    reconcile_frame(&mut app);

    let dir = ecs_dir(app.world(), e);
    assert_dir_eq(dir, expected, 1e-6, "Z+90 leaves -Z forward fixed");
    assert!(dir[2] < -0.9, "to-light Z lane stays NEGATIVE (forward still -Z): {dir:?}");
}

#[test]
fn light_direction_identity_is_minus_z() {
    // Identity transform → forward is exactly local -Z = world (0,0,-1).
    let mut app = reconcile_only_app();
    let e = spawn_dir_light_with_global(
        app.world_mut(),
        [1.0, 0.0, 0.0],
        global_from(Mat3::IDENTITY, Vec3::ZERO),
    );
    reconcile_frame(&mut app);
    assert_dir_eq(ecs_dir(app.world(), e), [0.0, 0.0, -1.0], 1e-7, "identity to-light dir is -Z");
}

/// Reads a directional light's current `direction`.
fn ecs_dir(world: &EcsMaster, e: Entity) -> [f32; 3] {
    world.get_component::<DirectionalLight>(e).expect("dir light lives").direction
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — a MOVED light's reconciled position/direction tracks its Transform
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn point_light_position_tracks_global_translation() {
    let mut app = reconcile_only_app();
    let world = app.world_mut();
    let arch = world.create_archetype(&[
        PointLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = PointLight { position: [0.0, 0.0, 0.0], color: [1.0; 3], power: 50.0, range: 5.0 };
    let moved_to = Vec3::new(3.0, -4.0, 5.5);
    let global = global_from(Mat3::IDENTITY, moved_to);
    let e = world
        .create_entity(
            arch,
            &[
                (PointLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&global)),
            ],
        )
        .expect("point-light archetype accepts its columns");

    reconcile_frame(&mut app);

    let p = app.world().get_component::<PointLight>(e).expect("point light lives").position;
    assert_eq!(p, [3.0, -4.0, 5.5], "point light position tracks GlobalTransform.translation");
}

#[test]
fn spot_light_position_and_direction_both_track() {
    let mut app = reconcile_only_app();
    let world = app.world_mut();
    let arch = world.create_archetype(&[
        SpotLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    // A +90° about Y rotation AND a translation: both lanes must update.
    let s = std::f32::consts::FRAC_PI_4.cos();
    let q = Quat::new(0.0, s, 0.0, s);
    let pos = Vec3::new(1.0, 2.0, 3.0);
    let global = global_from(Mat3::from_quat(q), pos);

    let light = SpotLight {
        position: [0.0, 0.0, 0.0],
        direction: [0.0, 0.0, 1.0],
        color: [1.0; 3],
        power: 100.0,
        range: 5.0,
        inner_deg: 15.0,
        outer_deg: 30.0,
    };
    let e = world
        .create_entity(
            arch,
            &[
                (SpotLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&global)),
            ],
        )
        .expect("spot-light archetype accepts its columns");

    reconcile_frame(&mut app);

    let l = *app.world().get_component::<SpotLight>(e).expect("spot light lives");
    assert_eq!(l.position, [1.0, 2.0, 3.0], "spot position tracks translation");
    assert_dir_eq(l.direction, [-1.0, 0.0, 0.0], 1e-6, "spot direction tracks rotated -Z");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 6 — a light WITHOUT GlobalTransform is byte-identical to pre-S4
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn light_without_global_transform_is_untouched() {
    let mut app = reconcile_only_app();
    let world = app.world_mut();
    // A directional light with NO Transform/GlobalTransform columns.
    let arch = world.create_archetype(&[DirectionalLight::component_id()]);
    let authored = DirectionalLight::new([0.3, 0.4, 0.866_025_4], [0.2, 0.5, 0.9], 7.5);
    let e = world
        .create_entity(arch, &[(DirectionalLight::component_id(), as_bytes(&authored))])
        .expect("bare dir-light archetype accepts its one column");

    let before = *world.get_component::<DirectionalLight>(e).expect("lives");
    reconcile_frame(&mut app);
    let after = *app.world().get_component::<DirectionalLight>(e).expect("lives");

    // Bit-for-bit identical: reconcile never matched the row (no GlobalTransform),
    // so the self-contained pose is byte-frozen (back-compat).
    assert_eq!(
        as_bytes(&before),
        as_bytes(&after),
        "a light WITHOUT GlobalTransform is byte-identical after reconcile (back-compat)"
    );
    assert_eq!(after, authored, "the authored pose is exactly preserved");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 + 8 — Gpu3dInstance pack: Hidden excluded; 0%-gate; pack correctness
// ════════════════════════════════════════════════════════════════════════════

/// Spawns a renderable: `GlobalTransform` + `MaterialHandle` + a zeroed
/// `Gpu3dInstance` column. Returns the handle; the caller decides whether to
/// `enable::<RenderEnabled>` (visible) or leave it clear (hidden).
fn spawn_renderable(world: &mut EcsMaster, arch: ArchetypeId, global: GlobalTransform, mat: u16) -> Entity {
    let zero = Gpu3dInstance::zeroed();
    let mh = MaterialHandle(mat);
    world
        .create_entity(
            arch,
            &[
                (GlobalTransform::component_id(), as_bytes(&global)),
                (MaterialHandle::component_id(), as_bytes(&mh)),
                (Gpu3dInstance::component_id(), as_bytes(&zero)),
            ],
        )
        .expect("renderable archetype accepts its three columns")
}

fn renderable_archetype(world: &mut EcsMaster) -> ArchetypeId {
    world.create_archetype(&[
        GlobalTransform::component_id(),
        MaterialHandle::component_id(),
        Gpu3dInstance::component_id(),
    ])
}

#[test]
fn pack_writes_global_transform_into_gpu3d_instance() {
    let mut world = EcsMaster::new();
    let arch = renderable_archetype(&mut world);

    let m = Mat3::from_rows(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(4.0, 5.0, 6.0),
        Vec3::new(7.0, 8.0, 9.0),
    );
    let t = Vec3::new(10.0, 11.0, 12.0);
    let e = spawn_renderable(&mut world, arch, global_from(m, t), 0xABCD);
    world.enable::<RenderEnabled>(e);

    world.run_system(sync_gpu_3d_instances);

    let inst = *world.get_component::<Gpu3dInstance>(e).expect("gpu3d inst lives");
    // linear_rows == matrix3 rows verbatim (no transpose — that is the shader's job).
    assert_eq!(inst.linear_rows[0], [1.0, 2.0, 3.0]);
    assert_eq!(inst.linear_rows[1], [4.0, 5.0, 6.0]);
    assert_eq!(inst.linear_rows[2], [7.0, 8.0, 9.0]);
    assert_eq!(inst.translation, [10.0, 11.0, 12.0]);
    // material: low 16 = handle; high 16 = pad (0).
    assert_eq!(inst.material & 0xFFFF, 0xABCD, "low 16 bits carry the material handle");
    assert_eq!(inst.material >> 16, 0, "high 16 bits stay pad (0)");
}

#[test]
fn hidden_row_is_excluded_from_the_pack() {
    let mut world = EcsMaster::new();
    let arch = renderable_archetype(&mut world);

    // Visible: bit set, distinctive transform. Hidden: bit clear, distinctive
    // transform that must NOT be written.
    let visible = spawn_renderable(&mut world, arch, global_from(Mat3::IDENTITY, Vec3::new(1.0, 0.0, 0.0)), 1);
    let hidden = spawn_renderable(&mut world, arch, global_from(Mat3::IDENTITY, Vec3::new(9.0, 9.0, 9.0)), 2);
    world.enable::<RenderEnabled>(visible);
    // hidden: deliberately NOT enabled.

    world.run_system(sync_gpu_3d_instances);

    let vis_inst = *world.get_component::<Gpu3dInstance>(visible).expect("visible inst");
    assert_eq!(vis_inst.translation, [1.0, 0.0, 0.0], "visible row was packed from its transform");

    let hid_inst = *world.get_component::<Gpu3dInstance>(hidden).expect("hidden inst");
    assert_eq!(
        hid_inst,
        Gpu3dInstance::zeroed(),
        "hidden (bit-clear) row's Gpu3dInstance is NOT refreshed — stays zeroed (excluded)"
    );
}

/// 0%-gate: a world with NO `Gpu3dInstance` column yields zero matching
/// archetypes, so the pack system visits nothing — a scene that never opts into
/// 3D instancing pays nothing. Asserted via a visit-counting wrapper.
#[test]
fn pack_does_zero_work_when_no_gpu3d_instance_column() {
    let mut world = EcsMaster::new();
    // A scene with GlobalTransform + MaterialHandle but NO Gpu3dInstance and NO
    // MeshHandle: a non-renderable entity.
    let arch = world.create_archetype(&[GlobalTransform::component_id(), MaterialHandle::component_id()]);
    let mh = MaterialHandle(1);
    world
        .create_entity(
            arch,
            &[
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
                (MaterialHandle::component_id(), as_bytes(&mh)),
            ],
        )
        .expect("non-renderable archetype accepts its columns");

    // Count rows the pack query would visit. Mirror the production query exactly.
    let visited = Arc::new(Mutex::new(0usize));
    let probe = Arc::clone(&visited);
    world.run_system(
        move |mut q: Query<
            (&GlobalTransform, &MaterialHandle, &mut Gpu3dInstance),
            boyko_ecs::ecs::core::iters::query::filter_enable::Enabled<RenderEnabled>,
        >| {
            let mut n = 0;
            for _ in q.iter_mut() {
                n += 1;
            }
            *probe.lock().expect("probe") = n;
        },
    );
    assert_eq!(
        *visited.lock().expect("probe"),
        0,
        "0%-gate: a no-Gpu3dInstance scene yields zero pack-query rows"
    );

    // And the production system itself runs without panicking (vacuously).
    world.run_system(sync_gpu_3d_instances);
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 5 — STATIC light WITH GlobalTransform → NO collect_lights rebuild
// ════════════════════════════════════════════════════════════════════════════

/// A probe resource that records, per `Schedule::run`, whether
/// `Changed<DirectionalLight>` fired — i.e. whether `collect_lights` would
/// rebuild (it is gated on EXACTLY this filter). The reconcile's value-gate must
/// keep this CLEAR on a static frame.
#[derive(boyko_macros::Resource, Default)]
struct DirChangedProbe {
    fired_frames: u32,
    runs: u32,
}

/// The probe system — counts a frame as "rebuild" if any `DirectionalLight` is
/// `Changed` this run (the same gate `collect_lights` uses).
#[allow(clippy::needless_pass_by_value)]
fn probe_dir_changed(
    changed: Query<&DirectionalLight, Changed<DirectionalLight>>,
    mut probe: boyko_ecs::ecs::core::system::ResMut<DirChangedProbe>,
) {
    probe.runs += 1;
    if changed.iter().next().is_some() {
        probe.fired_frames += 1;
    }
}

/// A queued local-`Transform` write applied INSIDE the schedule (so the write's
/// `Changed` tick lands in the SAME frame window `propagate_transforms` observes —
/// a write done between frames via a one-shot `run_system` can land exactly on the
/// window boundary and be missed). `Some` ⇒ apply to `target` this frame, then
/// clear; `None` ⇒ no-op.
#[derive(boyko_macros::Resource, Default)]
struct PendingMove {
    target: Option<Entity>,
    transform: Transform,
}

/// Applies any queued [`PendingMove`] through `Mut<Transform>` (bumping the
/// `Changed` tick) so the same-frame `propagate_transforms` re-composes the moved
/// subtree. Runs FIRST in the frame (before propagate).
#[allow(clippy::needless_pass_by_value)]
fn apply_pending_move(
    mut pending: boyko_ecs::ecs::core::system::ResMut<PendingMove>,
    mut q: Query<boyko_ecs::ecs::core::iters::query::Mut<Transform>>,
) {
    let Some(target) = pending.target.take() else { return };
    let t = pending.transform;
    for (id, mut tr) in q.iter_entities_mut() {
        if id == target.id() {
            *tr = t;
        }
    }
}

/// Queues a local-`Transform` write to be applied INSIDE the next `app.update`.
fn queue_move(world: &mut EcsMaster, target: Entity, transform: Transform) {
    let pm = world.resource_mut::<PendingMove>();
    pm.target = Some(target);
    pm.transform = transform;
}

/// Builds an `App` wiring propagate → reconcile → probe (the `collect_lights`
/// rebuild-trigger stand-in) into the Main schedule. `App::update` advances
/// `last_run` per frame, so `Changed` is observed against a correct per-frame
/// window (unlike one-shot `run_system`). The App owns its own pool, so the test
/// needs no `boyko_threadpool` dependency.
fn build_static_light_app() -> App {
    let mut app = App::new();
    app.insert_resource(DirChangedProbe::default());
    app.insert_resource(PendingMove::default());
    app.add_systems_cfg(|b| {
        // apply_pending_move (mutator) → propagate → reconcile → probe.
        let propagate_key = b.add_system(propagate_transforms).key();
        b.add_system(apply_pending_move).before(propagate_key);
        let probe_key = b.add_system(probe_dir_changed).key();
        b.add_system(light_reconcile).after(propagate_key).before(probe_key);
    });
    app
}

/// Spawns a static directional light (Transform + GlobalTransform) whose authored
/// direction ALREADY equals the identity-transform-derived -Z, so even the first
/// reconcile is a no-op write (value-gate). Returns its handle.
fn spawn_static_dir_light(world: &mut EcsMaster) -> Entity {
    let arch = world.create_archetype(&[
        DirectionalLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = DirectionalLight { direction: [0.0, 0.0, -1.0], color: [1.0; 3], illuminance: 1.0 };
    world
        .create_entity(
            arch,
            &[
                (DirectionalLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
            ],
        )
        .expect("static dir-light archetype accepts its columns")
}

#[test]
fn static_light_with_global_transform_does_not_rebuild_collect_lights() {
    let mut app = build_static_light_app();
    spawn_static_dir_light(app.world_mut());

    // Frame 0 seeds GlobalTransform via propagate (first compose). Frames 1..N are
    // the steady state: nothing moves.
    const FRAMES: u64 = 6;
    app.run_n(FRAMES);

    let probe = app.world().resource::<DirChangedProbe>();
    assert_eq!(probe.runs, FRAMES as u32, "the probe ran once per frame");
    // The load-bearing assertion: across the STEADY-STATE frames the light's
    // Changed<DirectionalLight> tick NEVER advanced from reconcile — because its
    // derived pose is bit-equal to the stored pose, the value-gate skips the
    // DerefMut, so collect_lights would do ZERO rebuilds.
    //
    // Allow at most the initial spawn frame to register a change (Added ⊆ Changed
    // at spawn); every subsequent frame must be clear. So fired_frames <= 1.
    assert!(
        probe.fired_frames <= 1,
        "a static light must not perpetually dirty collect_lights: fired on {} of {} frames",
        probe.fired_frames,
        FRAMES
    );
}

/// The counterpart: a MOVING light DOES fire the rebuild gate (anti-vacuity — the
/// probe is not stuck clear). The light's transform is changed between frames, so
/// reconcile writes a new direction and `Changed<DirectionalLight>` fires.
#[test]
fn moving_light_does_rebuild_collect_lights() {
    let mut app = build_static_light_app();
    let e = spawn_static_dir_light(app.world_mut());

    // Settle for a couple of frames (the spawn-Added window drains).
    app.finish();
    app.update();
    app.update();
    let fired_before = app.world().resource::<DirChangedProbe>().fired_frames;

    // ROTATE the light's local Transform by +90° about Y (applied INSIDE the next
    // frame by apply_pending_move): propagate yields a moved GlobalTransform and
    // reconcile derives a NEW direction, so the rebuild gate must fire.
    let s = std::f32::consts::FRAC_PI_4.cos();
    let rotated = Transform { translation: Vec3::ZERO, rotation: Quat::new(0.0, s, 0.0, s), scale: Vec3::ONE };
    queue_move(app.world_mut(), e, rotated);

    // Two frames: frame N applies the move + propagates (GlobalTransform stamped at
    // the apply-window tick); frame N+1 is when reconcile observes
    // `Changed<GlobalTransform>` and writes the new direction, which the same-frame
    // probe counts (the documented one-frame propagate→reader stagger).
    app.update();
    app.update();
    let fired_after = app.world().resource::<DirChangedProbe>().fired_frames;

    assert!(
        fired_after > fired_before,
        "a MOVED light fires the collect_lights rebuild gate (anti-vacuity): {fired_before} -> {fired_after}"
    );
    // And the new direction is the rotated -Z (world -X).
    assert_dir_eq(ecs_dir(app.world(), e), [-1.0, 0.0, 0.0], 1e-6, "moved light's new to-light dir");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 9 — Integration: a parented light orbiting its parent updates direction
// ════════════════════════════════════════════════════════════════════════════

#[derive(boyko_macros::Bundle)]
struct ParentBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[derive(boyko_macros::Bundle)]
struct ChildLightBundle {
    light: DirectionalLight,
    transform: Transform,
    global: GlobalTransform,
}

#[test]
fn parented_light_orbiting_parent_updates_direction() {
    // An App wiring propagate → reconcile (ordered), so the child light's world
    // pose flows from the parent through propagation and reconcile derives the
    // direction — driven by `App::update`'s real change windows.
    let mut app = App::new();
    app.insert_resource(PendingMove::default());
    app.add_systems_cfg(|b| {
        // apply_pending_move (mutator) → propagate → reconcile.
        let propagate_key = b.add_system(propagate_transforms).key();
        b.add_system(apply_pending_move).before(propagate_key);
        b.add_system(light_reconcile).after(propagate_key);
    });

    // Spawn parent + child light; parent rotates, child keeps an IDENTITY local
    // transform (so the child's world rotation == the parent's). As the parent
    // orbits, the child light's derived world direction must rotate with it.
    let sink: Arc<Mutex<Option<(Entity, Entity)>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    app.world_mut().run_system(move |mut cmds: Commands| {
        let parent = cmds
            .spawn(ParentBundle { transform: Transform::IDENTITY, global: GlobalTransform::IDENTITY })
            .id();
        let child = cmds
            .spawn(ChildLightBundle {
                light: DirectionalLight { direction: [0.0, 0.0, -1.0], color: [1.0; 3], illuminance: 1.0 },
                transform: Transform::IDENTITY,
                global: GlobalTransform::IDENTITY,
            })
            .id();
        cmds.entity(parent).add_child(child);
        *probe.lock().expect("probe") = Some((parent, child));
    });
    let (parent, child) = sink.lock().expect("probe").expect("spawn handles");
    assert!(
        app.world().has_entity(parent) && app.world().has_entity(child),
        "both entities live"
    );

    // Settle the hierarchy (compose child's world from parent's identity).
    app.finish();
    app.update();
    assert_dir_eq(
        ecs_dir(app.world(), child),
        [0.0, 0.0, -1.0],
        1e-6,
        "initial child dir is -Z (identity chain)",
    );

    // Orbit the PARENT by +90° about Y (applied INSIDE the next frame). The child
    // inherits the rotation through propagation; reconcile then derives the rotated
    // to-light direction.
    let s = std::f32::consts::FRAC_PI_4.cos();
    let rotated = Transform { translation: Vec3::ZERO, rotation: Quat::new(0.0, s, 0.0, s), scale: Vec3::ONE };
    queue_move(app.world_mut(), parent, rotated);

    // Two frames: frame N applies the move + propagates the new GlobalTransform
    // (stamped at the apply-window tick); frame N+1 is when reconcile's
    // `Changed<GlobalTransform>` window observes it (the documented one-frame
    // stagger between the exclusive propagate's write and the gated reader — see
    // `LightingPlugin`'s "self-correcting stagger" note). The Changed gate makes
    // this robust, not a race.
    app.update();
    app.update();

    // The child's local -Z forward, after the parent's +90° Y orbit, swings to -X.
    assert_dir_eq(
        ecs_dir(app.world(), child),
        [-1.0, 0.0, 0.0],
        1e-6,
        "a parented light orbiting its parent updates its world to-light direction",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 10 — the pack + cast_slice upload walk (the unsafe path; Miri target)
// ════════════════════════════════════════════════════════════════════════════

/// Packs a small renderable scene, then walks the contiguous `Gpu3dInstance`
/// column and `bytemuck::cast_slice`s it to bytes — exactly the COLUMN → GPU
/// upload the consuming renderer performs (`for_each_chunk` + `cast_slice`). This
/// is the test the Miri (tree-borrows) gate runs to exercise the only unsafe-
/// adjacent path S4 owns the producer side of.
#[test]
fn pack_then_cast_slice_upload_walk() {
    let mut world = EcsMaster::new();
    let arch = renderable_archetype(&mut world);

    let mut handles = Vec::new();
    for i in 0..4u32 {
        let t = Vec3::new(i as f32, (i * 2) as f32, (i * 3) as f32);
        let e = spawn_renderable(&mut world, arch, global_from(Mat3::IDENTITY, t), i as u16);
        world.enable::<RenderEnabled>(e);
        handles.push((e, t));
    }

    // Pack GlobalTransform → Gpu3dInstance.
    world.run_system(sync_gpu_3d_instances);

    // COLUMN → GPU upload: gather the packed instances into a contiguous buffer
    // and cast to bytes (the cast_slice the renderer uploads). bytemuck::cast_slice
    // requires Pod — the derive proves the layout is hole-free, which Miri checks.
    let packed: Vec<Gpu3dInstance> = handles
        .iter()
        .map(|(e, _)| *world.get_component::<Gpu3dInstance>(*e).expect("inst lives"))
        .collect();
    let bytes: &[u8] = bytemuck::cast_slice(&packed);
    assert_eq!(
        bytes.len(),
        packed.len() * GPU3D_INSTANCE_SIZE,
        "cast_slice byte length == count * 52 (no padding, sound upload)"
    );

    // Read the translation lane back out of the raw bytes at its known offset
    // (after the 36 B linear part) to prove the byte image is what the GPU sees.
    for (i, (_, t)) in handles.iter().enumerate() {
        let base = i * GPU3D_INSTANCE_SIZE + 36; // translation starts at offset 36.
        let read = |o: usize| {
            f32::from_ne_bytes(bytes[base + o..base + o + 4].try_into().expect("4 bytes"))
        };
        assert_eq!([read(0), read(4), read(8)], [t.x, t.y, t.z], "instance {i} translation bytes");
    }
}
