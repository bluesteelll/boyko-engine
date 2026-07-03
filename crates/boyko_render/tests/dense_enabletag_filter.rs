//! Host plan R5 — the dense × EnableTag composition proof (the migrated path).
//!
//! `snap_apply` and the per-row-branching `pack_gpu_transforms` iterate a DENSE
//! component (`GpuTransform3D`, `#[component(storage = "dense")]`) filtered by the
//! `SnapInterpolation` bitset `EnableTag`. The plan's day-one design (a bitset
//! `EnableTag`, `Enabled<SnapInterpolation>` filter + an `IsEnabled` per-row read)
//! is now the SHIPPED design: the kernel dense × enable-query feature closed the
//! hole where `Query<&mut GpuTransform3D, Enabled<Tag>>` silently yielded zero rows.
//!
//! This test proves the two production accessors over the dense pair filtered by the
//! `SnapInterpolation` EnableTag:
//!
//! * `snap_apply` — the POSITIVE `Enabled<SnapInterpolation>` filter (visits EXACTLY
//!   the enabled dense rows);
//! * `pack_gpu_transforms` — the per-row `IsEnabled<SnapInterpolation>` read (reports
//!   the bit per row WITHOUT dropping a row, so the pack keeps its single `&mut`
//!   query — a two-`&mut`-query split would trip the filter-agnostic write-vs-write
//!   conflict).
//!
//! Both use `iter` / `iter_mut` (the archetype-walking cursors that honor the per-row
//! bit), never the dense `.get()` fast path — so no dense null-deref.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::iters::query::IsEnabled;
use boyko_ecs::ecs::core::system::Commands;

use boyko_macros::Bundle;
use boyko_math::Vec3;

use boyko_render::{GpuTransform3D, SnapInterpolation};

use boyko_scene::transform::Transform;

/// A `(Transform, GpuTransform3D)` spawn payload — the table pose alongside the
/// dense pair, the exact shape the pack / `snap_apply` iterate. The
/// `SnapInterpolation` EnableTag is toggled (enabled), not inserted.
#[derive(Bundle)]
struct PairBundle {
    transform: Transform,
    pair: GpuTransform3D,
}

/// Spawns one entity carrying a table `Transform` + the dense `GpuTransform3D` pair
/// seeded at `x`, with the `SnapInterpolation` EnableTag bit ENABLED iff `snap`.
fn spawn_pair(world: &mut EcsMaster, x: f32, snap: bool) {
    let t = Transform::from_translation(Vec3::new(x, 0.0, 0.0));
    let pair = GpuTransform3D::from_transform(&t);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(PairBundle { transform: t, pair });
        if snap {
            e.enable::<SnapInterpolation>();
        }
    });
}

/// The two production accessor paths over a dense pair filtered by the
/// `SnapInterpolation` EnableTag:
///
/// * `Query<&mut GpuTransform3D, Enabled<SnapInterpolation>>::iter_mut()`
///   (snap_apply's shape) visits EXACTLY the enabled rows — the dense × enable
///   feature bounds the driver to the enable column and per-row-trims disabled rows;
/// * `Query<(&mut GpuTransform3D, IsEnabled<SnapInterpolation>), ()>::iter_mut()`
///   (pack_gpu_transforms' shape) reports the bit PER ROW without dropping any row.
#[test]
fn dense_pair_filtered_by_enabletag_iterates_only_enabled_rows() {
    let mut world = EcsMaster::new();

    // Two enabled, three disabled — distinct x so a mis-filtered row is caught.
    spawn_pair(&mut world, 10.0, true);
    spawn_pair(&mut world, 11.0, true);
    spawn_pair(&mut world, 20.0, false);
    spawn_pair(&mut world, 21.0, false);
    spawn_pair(&mut world, 22.0, false);

    // snap_apply's shape — the Enabled filter: only the two enabled rows, each
    // carrying its enabled x in curr.pos.x (the collapse the snap would apply).
    let mut enabled_xs: Vec<f32> = {
        let mut q = world.query::<&mut GpuTransform3D, Enabled<SnapInterpolation>>();
        q.iter_mut().map(|p| p.curr.pos[0]).collect()
    };
    enabled_xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        enabled_xs,
        vec![10.0, 11.0],
        "Enabled<SnapInterpolation> visits exactly the two enabled dense rows"
    );

    // pack_gpu_transforms' shape — the per-row IsEnabled: every dense row, each with
    // its bit. The enabled rows (x = 10, 11) report true; the rest false.
    let mut per_row: Vec<(f32, bool)> = {
        let mut q = world.query::<(&mut GpuTransform3D, IsEnabled<SnapInterpolation>), ()>();
        q.iter_mut().map(|(p, on)| (p.curr.pos[0], on)).collect()
    };
    per_row.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    assert_eq!(
        per_row,
        vec![(10.0, true), (11.0, true), (20.0, false), (21.0, false), (22.0, false)],
        "IsEnabled<SnapInterpolation> reports the bit per dense row (the pack branch)"
    );

    // The sum is every dense row (no row dropped by either path).
    assert_eq!(
        world.query::<&GpuTransform3D, ()>().dense_iter().count(),
        5,
        "both production paths visit all five dense pairs"
    );
}

/// A per-row toggle is reflected across queries WITHOUT an archetype migration (the
/// EnableTag advantage over the retired table tag): disabling an enabled row drops it
/// from the `Enabled` filter and flips its `IsEnabled` read, while its dense pair
/// stays put (no structural move).
#[test]
fn per_row_disable_is_reflected_without_migration() {
    let mut world = EcsMaster::new();
    spawn_pair(&mut world, 10.0, true);

    assert_eq!(
        world.query::<&GpuTransform3D, Enabled<SnapInterpolation>>().iter().count(),
        1,
        "the enabled row is visible before the disable"
    );

    // Disable the sole row via the direct master API (a per-row bit toggle — no
    // archetype migration). The id-keyed deferred disable is exercised by
    // snap_apply's own test; here we assert the query reflection directly.
    let id = {
        let q = world.query::<&GpuTransform3D, ()>();
        let mut it = q.dense_iter();
        it.next().expect("one pair").0
    };
    let entity = world.get_entity(id).expect("the row is live");
    world.disable::<SnapInterpolation>(entity);

    assert_eq!(
        world.query::<&GpuTransform3D, Enabled<SnapInterpolation>>().iter().count(),
        0,
        "the disabled row drops from the Enabled filter (per-row trim)"
    );
    // The dense pair is still present (the disable did not migrate/remove it).
    assert_eq!(
        world.query::<&GpuTransform3D, ()>().dense_iter().count(),
        1,
        "the dense pair survives the bit disable (no archetype migration)"
    );
}
