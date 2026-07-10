//! [`apply_refcount_deltas`] — the per-frame fold from
//! [`RefcountDeltas`](boyko_scene::RefcountDeltas) (pushed by the
//! `MeshHandle`/`MaterialHandle` carrier hooks in `boyko_scene::render_caps`)
//! into the two GPU asset tables (asset-streaming plan F2 §1/§3), plus the
//! [`AssetRefcountPlugin`] that wires the two resources + this system into the
//! app schedule.

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::asset::{AssetBacking, Assets};
use boyko_ecs::ecs::core::system::{NonSendResMut, ResMut};
use boyko_scene::{AssetRefKind, DeferredFree, FreeEntry, RefcountDeltas};

use crate::material::MaterialGpu;
use crate::mesh::MeshGpu;

/// Drains [`RefcountDeltas`] and folds each delta into the matching
/// `Assets<T>` table's refcount (asset-streaming plan F2 §1): `+1` calls
/// [`Assets::inc_ref`], `-1` calls [`Assets::dec_ref`]. A `dec_ref` that
/// returns a retire ticket is enqueued into [`DeferredFree`] with a
/// placeholder `retire_frame = 0` (F6 sets the real fence-gated value; F2
/// only enqueues — nothing drains this queue yet).
///
/// Any other `delta` magnitude is unreachable from the F2 hook wiring (every
/// pushed [`RefDelta`](boyko_scene::RefDelta) is `+1` or `-1`); a `debug_assert`
/// catches a future hook regression without costing anything in release.
///
/// # Schedule placement
///
/// Registered as a plain per-frame system by [`AssetRefcountPlugin`], with no
/// ordering edge to any other system: nothing in F2 yet reads
/// [`DeferredFree`] or depends on a row's `Retiring` transition (F5's
/// `validate_asset_refs` and F6's `retire_deferred_frees` are the future
/// consumers) — `Assets::get_by_index` resolves a `Retiring` row exactly like
/// a `Loaded` one (F2), so this system's placement within the frame does not
/// yet affect what the render's gather observes.
pub fn apply_refcount_deltas(
    mut deltas: ResMut<RefcountDeltas>,
    mut free: ResMut<DeferredFree>,
    mut material_assets: ResMut<Assets<MaterialGpu>>,
    mut mesh_assets: NonSendResMut<Assets<MeshGpu>>,
) {
    if deltas.is_empty() {
        return;
    }
    for delta in deltas.drain() {
        match delta.kind {
            AssetRefKind::Mesh => apply_one(&mut mesh_assets, &mut free, delta.kind, delta.slot, delta.delta),
            AssetRefKind::Material => {
                apply_one(&mut material_assets, &mut free, delta.kind, delta.slot, delta.delta)
            }
        }
    }
}

/// Folds one delta into `assets`, routing a resulting retire ticket into
/// `free`. Generic over the two concrete `AssetBacking` types
/// (`MeshGpu`/`MaterialGpu`) so [`apply_refcount_deltas`] shares one body for
/// both branches — monomorphized, no dynamic dispatch.
#[inline]
fn apply_one<T: AssetBacking>(
    assets: &mut Assets<T>,
    free: &mut DeferredFree,
    kind: AssetRefKind,
    slot: u32,
    delta: i32,
) {
    match delta {
        1 => assets.inc_ref(slot),
        -1 => {
            if assets.dec_ref(slot).is_some() {
                free.push(FreeEntry { kind, slot, retire_frame: 0 });
            }
        }
        other => {
            debug_assert!(
                false,
                "apply_refcount_deltas: RefDelta magnitude must be +1/-1, got {other} \
                 (a hook regression — every carrier hook pushes exactly +1 or -1)"
            );
        }
    }
}

/// Wires the asset-streaming refcount pipeline into the app schedule
/// (asset-streaming plan F2 §1/§3): inserts the two queue resources
/// ([`RefcountDeltas`], [`DeferredFree`]) the carrier hooks and
/// [`apply_refcount_deltas`] share, and registers the system.
///
/// # No ordering edge (yet)
///
/// F2 has no consumer that depends on this system's placement relative to
/// any other (see [`apply_refcount_deltas`]'s doc) — so this plugin adds no
/// `.before`/`.after` edge. F5 (`validate_asset_refs`) and F6
/// (`retire_deferred_frees`) are the rungs that will need one.
#[derive(Default)]
pub struct AssetRefcountPlugin;

impl Plugin for AssetRefcountPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RefcountDeltas::default());
        app.insert_resource(DeferredFree::default());
        app.add_systems_cfg(|b| {
            b.add_system(apply_refcount_deltas);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::AssetRefcountPlugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `apply_one` is generic over `AssetBacking`; `MaterialGpu` (a local,
    /// device-free `AssetBacking` type — `NEEDS_TEARDOWN = false`) exercises it
    /// without any device dependency. `MeshGpu` cannot be used here: it needs a
    /// real device to construct a value, which this unit test has none of.
    ///
    /// `Assets::add` mints refcount 0 (an unattached load has no owner yet —
    /// see `Assets::add`'s doc), so the sequence below `inc_ref`s TWICE before
    /// decrementing, to reach the SAME 1->2->1->0 trace the name describes
    /// without ever calling `dec_ref` on an already-0 row (which would trip
    /// its `debug_assert!(count > 0, ...)` — a real caller-precondition
    /// violation, not a property this test should exercise).
    #[test]
    fn apply_one_inc_then_dec_to_zero_enqueues_a_free_entry() {
        let mut assets = Assets::<MaterialGpu>::with_reserved(4);
        let handle = assets.add(MaterialGpu::default());
        let slot = handle.index();
        let mut free = DeferredFree::default();

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, 1);
        assert!(free.is_empty(), "refcount 0->1 must not enqueue a retire");

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, 1);
        assert!(free.is_empty(), "refcount 1->2 must not enqueue a retire");

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, -1);
        assert!(free.is_empty(), "refcount 2->1 must not enqueue a retire");

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, -1);
        assert_eq!(free.entries().len(), 1, "refcount 1->0 must enqueue exactly one retire");
        assert_eq!(free.entries()[0].slot, slot);
    }

    /// `Assets::add` mints refcount 0 (see the sibling test's doc) — `inc_ref`
    /// once first so the first `dec_ref` below is the genuine 1->0
    /// zero-crossing decrement this test targets, not an underflow on an
    /// already-0 row.
    #[test]
    fn apply_one_double_dec_past_zero_is_idempotent() {
        let mut assets = Assets::<MaterialGpu>::with_reserved(4);
        let handle = assets.add(MaterialGpu::default());
        let slot = handle.index();
        let mut free = DeferredFree::default();

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, 1);

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, -1);
        assert_eq!(free.entries().len(), 1, "the first zero-crossing decrement enqueues once");

        apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, -1);
        assert_eq!(
            free.entries().len(),
            1,
            "a second decrement on an already-Retiring slot must not enqueue again"
        );
    }
}
