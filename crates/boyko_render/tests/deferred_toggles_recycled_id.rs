//! H-06 in `boyko_render`'s two deferred bit toggles (rung A9b — the class
//! rung A9 fixed in `boyko_scene::visibility_sync`, found by name at two more
//! sites): a deferred toggle pending for `E` must NOT land on `F`, spawned
//! on `E`'s recycled id before the toggle drains. Each site's command carries
//! the full `Entity` (id + generation) captured when the intent was formed,
//! and the kernel's `live_inland` resolve at apply drops a stale generation.
//!
//! The two sites and the two harnesses:
//!
//! 1. **`validate_asset_refs`'s stale-mesh mark** (`asset_refcount.rs`,
//!    `SetStaleCommand`, which SETS the `RenderStale` bit on a stale carrier —
//!    since the asset-validate prereqs the verdict is its own bit, not a clear
//!    of `RenderEnabled`). The system reads `NonSendRes<Assets<MeshGpu>>`,
//!    which declares universal access, so it resolves to `CpuExclusive` and the
//!    scheduler runs it dispatcher-solo with its apply INLINE after its body
//!    (`schedule.rs`'s exclusive path). No other system's apply can land
//!    between its body and its apply, so the concurrent-`killer` recipe of
//!    `visibility_sync_gates` gate 5 cannot stage the window on a schedule: the
//!    window is closed by exclusivity today. It is not closed by the command's
//!    key, which is what this gate pins — through the kernel's own body/apply
//!    split (`EcsMaster::run_system_once` runs the body and leaves the deferred
//!    queue unflushed; `System::apply` drains it), the exact split the
//!    concurrent path exposes to every other system and the split the site
//!    would fall into the moment the `NonSendRes` went away.
//! 2. **`snap_apply`'s one-frame disable** (`snap_interpolation.rs`,
//!    `DisableSnap`). A concurrent system, so gate 5's recipe applies verbatim:
//!    a `killer` system with disjoint access, added FIRST, on a ONE-worker pool
//!    (FIFO injector, FIFO completions, insertion-stable Kahn ⇒ `killer`'s
//!    queue drains before `snap_apply`'s, by construction), `F` spawned on the
//!    dispatcher at apply (`EcsMaster::spawn_empty` pops the recycled stack the
//!    despawn one command earlier pushed), and a probe command at the head of
//!    `killer`'s queue that reads `E`'s bit — still SET there, so `snap_apply`'s
//!    disable was pending across the recycle and not applied to a live `E`.
//!
//! Both gates pin the recycle they rely on (`F.id == E.id`, generation + 1),
//! run the fresh-id twin (`F` allocated BEFORE `E`'s despawn), and assert the
//! plan's equality: `F`'s bit must not depend on whether `F` recycled `E`'s id.
//! `F` is a `spawn_empty` row in both: an `EnableTag` bit is per row in any
//! archetype. At site 1 a landed stale mark would SET a `RenderStale` bit `F`
//! never had (it was never validated); at site 2 a landed stale disable would
//! CLEAR the `SnapInterpolation` bit `F` set at spawn.
//!
//! # `MeshGpu` without a device
//!
//! Same as `asset_streaming_f5_validation.rs`: a `VkBuffer::NULL`-handled dummy
//! only ever moves through `Assets<MeshGpu>`'s store; no Vulkan call is made.

use boyko_ecs::ecs::core::asset::{Assets, GEN_UNSYNCED};
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem, ResMut, System};
use boyko_macros::{Bundle, Resource};
use boyko_math::Vec3;
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_render::asset_refcount::{RenderStale, ValidateCursor, validate_asset_refs};
use boyko_render::snap_interpolation::snap_apply;
use boyko_render::{
    GpuTransform3D, Material, MeshGpu, RenderEpoch, SnapInterpolation, TrsPacked,
    apply_refcount_deltas,
};
use boyko_scene::transform::Transform;
use boyko_scene::{DeferredFree, MeshHandle, MeshRefGen, RefcountDeltas, RenderEnabled};

/// Which order the despawn of `E` and the spawn of `F` happen in — the ONLY
/// thing that differs between the recycled run and the fresh-id control.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum ClaimOrder {
    /// Despawn `E`, then spawn `F`: `F` pops `E`'s slot (same id, generation + 1).
    #[default]
    Recycled,
    /// Spawn `F`, then despawn `E`: the free stack is empty when `F` is
    /// allocated, so `F` mints a fresh id.
    Fresh,
}

// ════════════════════════════════════════════════════════════════════════════
// Site 1 — `validate_asset_refs`'s stale-mesh mark (`SetStaleCommand` SETS the `RenderStale` bit).
// ════════════════════════════════════════════════════════════════════════════

/// A device-inert `MeshGpu` — see the module doc.
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer {
        buffer: VkBuffer::NULL,
        offset: 0,
        size: 0,
        mapped: None,
        block: 0,
    };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 0,
        index_type: IndexType::Uint16,
        vertex_count: 0,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
        local_min: [f32::INFINITY; 3],
        local_max: [f32::NEG_INFINITY; 3],
    }
}

/// A world holding one `RenderEnabled` mesh carrier `E` whose bound slot has
/// been force-reused underneath it (the F5 "simulate F6" recipe): its lane
/// generation mismatches the slot's, and the store's `free_epoch` has advanced
/// past the fresh `ValidateCursor`, so `validate_asset_refs` walks the row and
/// finds it stale.
fn stale_mesh_world() -> (EcsMaster, Entity) {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let h = mesh_assets.add(dummy_mesh_gpu());
    let slot = h.index();

    let mut world = EcsMaster::new();
    world.insert_resource(RefcountDeltas::default());
    world.insert_resource(DeferredFree::default());
    world.insert_resource(ValidateCursor::default());
    world.insert_resource(RenderEpoch::default());
    world.insert_resource(Assets::<Material>::with_reserved(4));
    world.insert_non_send_resource(mesh_assets);
    let _ = MeshHandle::component_id();

    let e: Entity = world.run_system(move |mut cmds: Commands| {
        cmds.spawn(MeshHandle(slot)).enable::<RenderEnabled>().id()
    });
    world.run_system(apply_refcount_deltas);
    assert!(
        world.is_enabled::<RenderEnabled>(e),
        "precondition: E starts enabled"
    );
    let bound = world
        .get_component::<MeshRefGen>(e)
        .copied()
        .expect("MeshHandle #[require]s MeshRefGen")
        .0;
    assert_ne!(
        bound, GEN_UNSYNCED,
        "precondition: the apply synced E's lane"
    );

    world.non_send_resource_mut::<Assets<MeshGpu>>().remove(h);
    let reused = world
        .non_send_resource_mut::<Assets<MeshGpu>>()
        .add(dummy_mesh_gpu());
    assert_eq!(
        reused.index(),
        slot,
        "precondition: LIFO reuse hands the freed row back at the same index"
    );
    assert_ne!(
        reused.generation(),
        bound,
        "precondition: E's bound generation is now stale"
    );

    (world, e)
}

/// `IntoSystem::into_system` with the same bounds `EcsMaster::run_system`
/// uses, so the marker infers from the fn item.
fn system_of<F, M, Out>(f: F) -> F::System
where
    F: IntoSystem<(), Out, M>,
    F::System: System<Out = Out>,
{
    F::into_system(f)
}

/// What one staged validation window left behind.
#[derive(Debug)]
struct ValidateOutcome {
    e: Entity,
    /// `None` when the window despawned nothing (the toggle-is-live control).
    f: Option<Entity>,
    /// `is_enabled::<RenderStale>` on `E` after the body ran and BEFORE the
    /// apply — `false` says the stale mark is deferred, not applied by the body
    /// (the anti-vacuity pin of the split).
    e_stale_before_apply: bool,
    /// `is_enabled::<RenderStale>` on `E` after the apply (a dead `E` reads
    /// `false`).
    e_stale: bool,
    /// `is_enabled::<RenderStale>` on `F` after the apply (`false` if no `F`).
    f_stale: bool,
}

/// One window: `validate_asset_refs`'s body runs against a live, stale `E`
/// and queues its stale mark; then, before that queue drains, `E` is despawned
/// and `F` spawned in the requested order with its `RenderStale` bit CLEAR (a
/// fresh row, never validated); then the queue drains. `order == None` runs the
/// control in which nothing is despawned.
fn run_validate_window(order: Option<ClaimOrder>) -> ValidateOutcome {
    let (mut world, e) = stale_mesh_world();

    let mut validate = system_of(validate_asset_refs);
    // Body only: `run_system_once` does not flush the deferred buffers.
    world.run_system_once(&mut validate);
    let e_stale_before_apply = world.is_enabled::<RenderStale>(e);

    let f = order.map(|order| {
        let f = match order {
            ClaimOrder::Recycled => {
                assert!(world.delete_entity(e), "recycled: E is live and deletes");
                world.spawn_empty()
            }
            ClaimOrder::Fresh => {
                let f = world.spawn_empty();
                assert!(world.delete_entity(e), "fresh: E is live and deletes");
                f
            }
        };
        assert!(
            !world.is_enabled::<RenderStale>(f),
            "F's stale bit is CLEAR before the drain (a fresh row, never validated)"
        );
        f
    });

    // The drain: the stale mark queued for E lands now.
    validate.apply(&mut world);

    ValidateOutcome {
        e,
        f,
        e_stale_before_apply,
        e_stale: world.is_enabled::<RenderStale>(e),
        f_stale: f.is_some_and(|f| world.is_enabled::<RenderStale>(f)),
    }
}

#[test]
fn stale_mesh_disable_pending_for_a_despawned_entity_does_not_clear_its_recycled_ids_bit() {
    // Anti-vacuity 1 — the window really carries a live, DEFERRED stale mark
    // for E: the body leaves E's `RenderStale` bit CLEAR, the apply sets it.
    let live = run_validate_window(None);
    assert!(live.f.is_none(), "control: nothing was spawned");
    assert!(
        !live.e_stale_before_apply,
        "control: the body only QUEUES the stale mark (the split stages a real window)"
    );
    assert!(
        live.e_stale,
        "control: the apply sets the stale carrier's bit — the mark under test is live"
    );

    // The recycled run.
    let a = run_validate_window(Some(ClaimOrder::Recycled));
    let fa = a.f.expect("recycled run: F was spawned");
    assert!(
        !a.e_stale_before_apply,
        "recycled run: the stale mark was pending when E was despawned"
    );
    // Anti-vacuity 2 — the recycle happened as staged: F sits on E's id, one
    // generation up.
    assert_eq!(
        fa.id(),
        a.e.id(),
        "recycled run: F must take E's id (spawned after E's despawn)"
    );
    assert_eq!(
        fa.generation(),
        a.e.generation() + 1,
        "recycled run: F carries E's slot generation + 1"
    );
    assert_ne!(
        fa, a.e,
        "recycled run: F and E are distinct handles on one id"
    );

    // THE H-06 ASSERTION. F is a fresh row that has never been validated;
    // E's pending stale mark, keyed by E, must not reach it.
    assert!(
        !a.f_stale,
        "H-06 (validate_asset_refs): a stale mark pending for the despawned E landed on F, spawned \
         on E's recycled id before the drain — F ({fa:?}) was never validated and reads STALE",
    );

    // The fresh-id twin: identical window, F allocated BEFORE E's despawn.
    let b = run_validate_window(Some(ClaimOrder::Fresh));
    let fb = b.f.expect("fresh run: F was spawned");
    assert_ne!(
        fb.id(),
        b.e.id(),
        "fresh run: F must NOT take E's id (nothing was recycled yet)"
    );
    assert_eq!(
        fb.generation(),
        0,
        "fresh run: a minted id starts at generation 0"
    );
    assert!(
        !b.f_stale,
        "fresh run: F was never validated and reads clean"
    );

    // The plan's equality.
    assert_eq!(
        a.f_stale, b.f_stale,
        "H-06 (validate_asset_refs): F's bit must not depend on whether F recycled E's id"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Site 2 — `snap_apply`'s one-frame disable (`DisableSnap`).
// ════════════════════════════════════════════════════════════════════════════
//
// The interleaving under test is: `snap_apply` enqueues the disable for E
// while E is live → E is despawned and F is registered on E's id → the disable
// drains. Only the apply window can stage the middle step, so the harness runs
// `killer` CONCURRENTLY with `snap_apply` (no ordering edge; disjoint access:
// `ResMut<KillRequest>` + `Commands` vs `Query<(&Transform, &mut
// GpuTransform3D), Enabled<SnapInterpolation>>` + `Commands`) and lets the
// drain order do the rest — `visibility_sync_gates` gate 5's argument, which
// this file does not repeat: `killer` added FIRST on a ONE-worker pool drains
// first, by construction. `killer`'s queue opens with `ProbeEBit`, which reads
// E's snap bit at `killer`'s apply: SET means `snap_apply`'s disable had not
// applied yet, i.e. it was pending across the recycle (the drain-order pin —
// the `enable_generation` probe of gate 5 cannot serve here, because F's
// enable in the empty archetype allocates a column in either order).

/// A `(Transform, GpuTransform3D)` spawn payload — the pack's shape; the
/// `SnapInterpolation` bit is toggled, not inserted.
#[derive(Bundle)]
struct PairBundle {
    transform: Transform,
    pair: GpuTransform3D,
}

/// The frame's structural request: `killer` consumes `doomed` on the frame it
/// runs; the probe and `SpawnFAtApply` report back through it.
#[derive(Resource, Default)]
struct KillRequest {
    doomed: Option<Entity>,
    order: ClaimOrder,
    /// `E`'s snap bit as read by `ProbeEBit` at the head of `killer`'s apply.
    e_bit_at_kill: Option<bool>,
    spawned: Option<Entity>,
}

/// Reads `E`'s snap bit at the head of `killer`'s apply — the drain-order pin.
struct ProbeEBit {
    e: Entity,
}

impl Command for ProbeEBit {
    fn apply(self, world: &mut EcsMaster) {
        let bit = world.is_enabled::<SnapInterpolation>(self.e);
        world.resource_mut::<KillRequest>().e_bit_at_kill = Some(bit);
    }
}

/// Spawns `F` through the DISPATCHER's allocator, at apply time (a worker-side
/// `Commands::spawn` claims its id at ENQUEUE, before any despawn of this frame
/// has applied, so it cannot stage the same-frame reuse), and SETS its snap bit
/// in the same apply so a landed stale disable would show as a cleared bit.
struct SpawnFAtApply;

impl Command for SpawnFAtApply {
    fn apply(self, world: &mut EcsMaster) {
        let f = world.spawn_empty();
        world.enable::<SnapInterpolation>(f);
        world.resource_mut::<KillRequest>().spawned = Some(f);
    }
}

/// Concurrent with `snap_apply`. Enqueues the probe, then the despawn of `E`
/// and the apply-time spawn of `F` in the requested order.
#[allow(clippy::needless_pass_by_value)]
fn killer(mut commands: Commands, mut req: ResMut<KillRequest>) {
    let Some(e) = req.doomed.take() else { return };
    commands.add(ProbeEBit { e });
    match req.order {
        ClaimOrder::Recycled => {
            commands.despawn(e);
            commands.add(SpawnFAtApply);
        }
        ClaimOrder::Fresh => {
            commands.add(SpawnFAtApply);
            commands.despawn(e);
        }
    }
}

/// What one snap frame left behind.
#[derive(Debug)]
struct SnapOutcome {
    e: Entity,
    /// `None` when the frame despawned nothing (the toggle-is-live control).
    f: Option<Entity>,
    /// `E`'s snap bit as `killer`'s apply saw it (`None` on the control frame).
    e_bit_at_kill: Option<bool>,
    /// `is_enabled::<SnapInterpolation>` on `E` after the frame (a dead `E`
    /// reads `false`).
    e_enabled: bool,
    /// `is_enabled::<SnapInterpolation>` on `F` after the frame (`false` if no `F`).
    f_enabled: bool,
}

/// One frame: `E` is a flagged body (its snap bit SET, so `snap_apply` yields it
/// and queues its one-frame disable), then `killer` and `snap_apply` run
/// concurrently on a one-worker pool. `order == None` runs the control frame in
/// which nothing is despawned.
fn run_snap_frame(order: Option<ClaimOrder>) -> SnapOutcome {
    let mut world = EcsMaster::new();
    world.insert_resource(KillRequest::default());

    let transform = Transform::from_translation(Vec3::new(50.0, 0.0, 0.0));
    let pair = GpuTransform3D {
        prev: TrsPacked::from_transform(&Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))),
        curr: TrsPacked::from_transform(&transform),
    };
    let e: Entity = world.run_system(move |mut cmds: Commands| {
        cmds.spawn(PairBundle { transform, pair })
            .enable::<SnapInterpolation>()
            .id()
    });
    assert!(
        world.is_enabled::<SnapInterpolation>(e),
        "precondition: E's snap bit is SET"
    );
    {
        let req = world.resource_mut::<KillRequest>();
        req.doomed = order.map(|_| e);
        req.order = order.unwrap_or_default();
    }

    // ONE worker + `killer` added FIRST: the dispatch and drain order the
    // header derives.
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(killer);
    b.add_system(snap_apply);
    let mut sched = b.build(&mut world);
    sched.run(&mut world);

    let req = world.resource::<KillRequest>();
    let (f, e_bit_at_kill) = (req.spawned, req.e_bit_at_kill);
    SnapOutcome {
        e,
        f,
        e_bit_at_kill,
        e_enabled: world.is_enabled::<SnapInterpolation>(e),
        f_enabled: f.is_some_and(|f| world.is_enabled::<SnapInterpolation>(f)),
    }
}

#[test]
fn snap_disable_pending_for_a_despawned_entity_does_not_clear_its_recycled_ids_bit() {
    // Anti-vacuity 1 — the frame really carries a live disable for E: with
    // nothing despawned, E's snap bit is CLEAR after the frame.
    let live = run_snap_frame(None);
    assert!(live.f.is_none(), "control: nothing was spawned");
    assert!(
        live.e_bit_at_kill.is_none(),
        "control: killer had nothing to do"
    );
    assert!(
        !live.e_enabled,
        "control: snap_apply's one-frame disable clears E's bit — the disable under test is live"
    );

    // The recycled run.
    let a = run_snap_frame(Some(ClaimOrder::Recycled));
    let fa = a.f.expect("recycled run: SpawnFAtApply registered F");
    // Drain-order pin: E's bit was still SET when killer's queue drained, so
    // snap_apply's disable was pending across the recycle (not applied to a
    // live E before the despawn — which would make this run green for the
    // wrong reason).
    assert_eq!(
        a.e_bit_at_kill,
        Some(true),
        "recycled run: killer's queue must drain before snap_apply's (E's bit still SET at the probe)"
    );
    // Anti-vacuity 2 — the recycle happened as staged: F sits on E's id, one
    // generation up.
    assert_eq!(
        fa.id(),
        a.e.id(),
        "recycled run: F must take E's id (killer drained before the toggle)"
    );
    assert_eq!(
        fa.generation(),
        a.e.generation() + 1,
        "recycled run: F carries E's slot generation + 1"
    );
    assert_ne!(
        fa, a.e,
        "recycled run: F and E are distinct handles on one id"
    );

    // THE H-06 ASSERTION. F set its own snap bit at spawn and has never been
    // snapped; E's pending disable, keyed by E, must not reach it.
    assert!(
        a.f_enabled,
        "H-06 (snap_apply): a stale disable pending for the despawned E landed on F, spawned on E's \
         recycled id in the same frame — F ({fa:?}) set its bit at spawn, was never snapped, and \
         reads DISABLED",
    );

    // The fresh-id twin: identical frame, F allocated BEFORE E's despawn.
    let b = run_snap_frame(Some(ClaimOrder::Fresh));
    let fb = b.f.expect("fresh run: SpawnFAtApply registered F");
    assert_eq!(
        b.e_bit_at_kill,
        Some(true),
        "fresh run: same drain order as the recycled run"
    );
    assert_ne!(
        fb.id(),
        b.e.id(),
        "fresh run: F must NOT take E's id (nothing was recycled yet)"
    );
    assert_eq!(
        fb.generation(),
        0,
        "fresh run: a minted id starts at generation 0"
    );
    assert!(
        b.f_enabled,
        "fresh run: F set its bit at spawn and was never snapped"
    );

    // The plan's equality.
    assert_eq!(
        a.f_enabled, b.f_enabled,
        "H-06 (snap_apply): F's bit must not depend on whether F recycled E's id"
    );
}
