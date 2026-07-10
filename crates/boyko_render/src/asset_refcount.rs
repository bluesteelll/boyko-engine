//! [`apply_refcount_deltas`] — the per-frame fold from
//! [`RefcountDeltas`](boyko_scene::RefcountDeltas) (pushed by the
//! `MeshHandle`/`MaterialHandle` carrier hooks in `boyko_scene::render_caps`)
//! into the two GPU asset tables (asset-streaming plan F2 §1/§3, gen-checked
//! as of F5), plus [`validate_asset_refs`] (F5's best-effort staleness net)
//! and the [`AssetRefcountPlugin`] that wires the resources + both systems
//! into the app schedule.

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::asset::{AssetBacking, AssetLoadState, Assets, GEN_UNSYNCED};
use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{Commands, NonSendRes, NonSendResMut, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Resource;
use boyko_scene::{
    AssetRefKind, DeferredFree, FreeEntry, MaterialRefGen, MeshHandle, MeshRefGen, RefcountDeltas,
    RenderEnabled,
};

use crate::material::MaterialGpu;
use crate::mesh::MeshGpu;

/// Drains [`RefcountDeltas`] and folds each delta into the matching
/// `Assets<T>` table's refcount (asset-streaming plan F2 §1, gen-checked as of
/// F5): `+1` calls [`Assets::inc_ref`] and (regardless of its result — see
/// [`apply_one`]'s doc) re-syncs the carrier's `MeshRefGen`/`MaterialRefGen`
/// lane via [`Commands`]; `-1` calls [`Assets::dec_ref`] with the delta's
/// captured bind-generation. A `dec_ref` that returns a retire ticket is
/// enqueued into [`DeferredFree`] with a placeholder `retire_frame = 0` (F6
/// sets the real fence-gated value; F5 only enqueues — nothing drains this
/// queue yet).
///
/// Any other `delta` magnitude is unreachable from the hook wiring (every
/// pushed [`RefDelta`](boyko_scene::RefDelta) is `+1` or `-1`); a `debug_assert`
/// catches a future hook regression without costing anything in release.
///
/// # Schedule placement
///
/// Registered by [`AssetRefcountPlugin`] `.before(validate_asset_refs)` (both
/// systems share one [`App::add_systems_cfg`] closure, so the edge is
/// expressible — see the plugin's doc for why the further edge to the mesh/CSM
/// gathers is NOT a hard scheduler edge). The lane writes ride `Commands`,
/// flushed by the PER-SYSTEM apply window immediately after this system's body
/// returns (`boyko_ecs::ecs::core::schedule::schedule::Schedule::run`'s
/// dispatch loop calls `system.apply(world)` right after each system
/// completes — the concurrent-dispatch site at `schedule.rs:724` and the
/// solo/dispatcher-exclusive site at `schedule.rs:1130` both do this; NEITHER
/// batches every system's commands into one end-of-frame barrier), so
/// `validate_asset_refs` always observes THIS frame's lane writes. A future
/// stage-boundary-flush refactor (batching `Commands` across a whole stage
/// before any apply) would break this same-frame lane visibility — anyone
/// touching that dispatch loop should re-verify this contract.
pub fn apply_refcount_deltas(
    mut deltas: ResMut<RefcountDeltas>,
    mut free: ResMut<DeferredFree>,
    mut material_assets: ResMut<Assets<MaterialGpu>>,
    mut mesh_assets: NonSendResMut<Assets<MeshGpu>>,
    mut cmd: Commands,
) {
    if deltas.is_empty() {
        return;
    }
    for delta in deltas.drain() {
        match delta.kind {
            AssetRefKind::Mesh => {
                if let Some(g) =
                    apply_one(&mut mesh_assets, &mut free, delta.kind, delta.slot, delta.gen_, delta.delta)
                {
                    cmd.entity(delta.entity).insert(MeshRefGen(g));
                }
            }
            AssetRefKind::Material => {
                if let Some(g) = apply_one(
                    &mut material_assets,
                    &mut free,
                    delta.kind,
                    delta.slot,
                    delta.gen_,
                    delta.delta,
                ) {
                    cmd.entity(delta.entity).insert(MaterialRefGen(g));
                }
            }
        }
    }
}

/// Folds one delta into `assets`, routing a resulting retire ticket into
/// `free`. Generic over the two concrete `AssetBacking` types
/// (`MeshGpu`/`MaterialGpu`) so [`apply_refcount_deltas`] shares one body for
/// both branches — monomorphized, no dynamic dispatch. Returns `Some(gen)` on
/// a `+1` delta (the attach-time generation the caller stamps into the
/// type-specific lane — this function cannot name `MeshRefGen`/
/// `MaterialRefGen` itself, since both branches share one generic body);
/// `None` on a `-1` delta (nothing to stamp) or an unreachable magnitude.
///
/// # Unconditional lane stamp on a refused `+1` (F5 blocker fix)
///
/// [`Assets::inc_ref`] refuses (returns `false`, no mutation) a carrier that
/// binds an already-`Retiring` slot — the sole resurrection hazard (see its
/// doc). This function returns `Some(generation)` regardless of that bool.
/// Leaving the lane at [`GEN_UNSYNCED`] instead would (a) make
/// `validate_asset_refs` SKIP the carrier (it trusts `GEN_UNSYNCED` as
/// "freshly bound"), never disabling a carrier that in fact bound a dead
/// slot, AND (b) make the carrier's EVENTUAL `-1` decrement also carry
/// `GEN_UNSYNCED`, bypassing `dec_ref`'s gen-check and corrupting whichever
/// tenant has since reused the slot. Stamping unconditionally closes both:
/// `validate_asset_refs` sees `state_of_index != Loaded` and disables the
/// carrier; the eventual `-1` carries the ATTACH-time generation, which
/// mismatches the reused slot's current one, so `dec_ref` suppresses it. The
/// slot's real refcount still never rose (`inc_ref` refused) — the F5/F6
/// boundary stays airtight (see `Assets::inc_ref`'s doc for the full
/// argument). `try_generation` (not the panicking `generation`) guards the
/// OOR case — a malformed carrier holding a never-minted index must not panic.
#[inline]
fn apply_one<T: AssetBacking>(
    assets: &mut Assets<T>,
    free: &mut DeferredFree,
    kind: AssetRefKind,
    slot: u32,
    gen_: u32,
    delta: i32,
) -> Option<u32> {
    match delta {
        1 => {
            let _incremented = assets.inc_ref(slot);
            assets.try_generation(slot)
        }
        -1 => {
            if assets.dec_ref(slot, gen_).is_some() {
                free.push(FreeEntry { kind, slot, retire_frame: 0 });
            }
            None
        }
        other => {
            debug_assert!(
                false,
                "apply_refcount_deltas: RefDelta magnitude must be +1/-1, got {other} \
                 (a hook regression — every carrier hook pushes exactly +1 or -1)"
            );
            None
        }
    }
}

/// Tracks the last `free_epoch` [`validate_asset_refs`] observed on the mesh
/// store — the O(1) early-out oracle (asset-streaming plan F5 Decision 6).
/// `Default` starts at 0, matching a fresh `Assets::<MeshGpu>::free_epoch`.
///
/// Mesh-only: `validate_asset_refs` no longer reads the material store at all
/// (see that fn's doc for why the material arm was removed) — there is
/// nothing for a `mat` cursor to gate.
#[derive(Debug, Default, Clone, Copy, Resource)]
pub struct ValidateCursor {
    /// Last `Assets::<MeshGpu>::free_epoch()` observed.
    pub mesh: u64,
}

/// Deferred disable of [`RenderEnabled`] for a mesh row [`validate_asset_refs`]
/// found stale this frame (asset-streaming plan F5 Decision 6) — keyed by
/// [`EntityId`], mirroring `boyko_scene::visibility_sync`'s
/// `SetRenderEnabledById`: a read-only query yields only `EntityId` (there is
/// no `QueryData for Entity` and no world-resolving `SystemParam` in this
/// kernel — see that fn's doc), so the live, generation-correct `Entity` is
/// re-resolved at apply time via [`EcsMaster::get_entity`]. A dead/stale id (a
/// despawn racing this frame) is a silent no-op — the same contract as the
/// kernel's own `EnableTagCommand`.
struct DisableStaleMeshCommand {
    /// The stale row's entity id, read from the matched archetype's entity-id
    /// column at gather time (`Query::iter_entities`).
    id: EntityId,
}

impl Command for DisableStaleMeshCommand {
    fn apply(self, world: &mut EcsMaster) {
        let Some(entity) = world.get_entity(self.id) else {
            return;
        };
        world.disable::<RenderEnabled>(entity);
    }
}

/// Best-effort staleness net for `MeshHandle`/`MaterialHandle` carriers
/// (asset-streaming plan F5 Decision 6) — the SOLE backstop for a bare-slot
/// carrier that has fallen out of sync with its bound store row. A
/// well-formed carrier's own refcount keeps its slot alive and never goes
/// stale (see `boyko_scene::render_caps`'s "Refcount hook wiring" doc); the
/// carriers this system catches are contract violations (the W1 rebind gap, a
/// stale weak `Handle` copy held outside a carrier) that the durable guards —
/// refcount (F2) + the `dec_ref` gen-check (F5 Decision 4) — already render
/// non-corrupting. This system only adds visual cleanliness on top.
///
/// # DISABLE-ONLY by design in F5 — no re-enable path; MESH-ONLY (W1 fix)
///
/// This system only ever DISABLES a mesh row (`disable::<RenderEnabled>`) —
/// it never re-enables one. This is latent-but-correct today: every in-tree
/// load is synchronous (`Assets::add` → `Loaded` immediately, never via
/// `reserve`/`fill`), so no carrier ever binds a `Loading` slot; and no
/// in-tree scene retires-and-reuses a slot (F6, the first reuse, has not
/// landed), so `free_epoch` never advances on a golden scene and this
/// system's per-row loop never runs (see the early-out below). Before async
/// `reserve`/`fill` streaming is exercised (F6/F7), TWO things are HARD
/// PREREQUISITES (`docs/ASSET-STREAMING-PLAN.md`'s "HARD PREREQ before async
/// streaming" section): (a) a `Loading → Loaded` RE-ENABLE path — `fill` must
/// bump a validation epoch and this system must gain an enable arm; and (b)
/// DECOUPLING staleness from user visibility — reusing `RenderEnabled` here
/// fights `visibility_sync` (both drive that same bit); a future rung needs a
/// separate `RenderStale` `EnableTag` the gather also filters on, instead of
/// layering onto `RenderEnabled`. (Bevy PR #18734 is the same-frame
/// handle-swap race this whole mechanism defends against.)
///
/// Material staleness is handled SOLELY by the `dec_ref` gen-check at despawn
/// (no render effect: the raster hardcodes material 0 until F8 wires
/// per-instance material into the shader) — a stale weak material carrier can
/// no longer corrupt a reused slot's refcount, but this system does NOT read
/// or write `MaterialHandle`/`MaterialRefGen` at all. An earlier revision of
/// this rung ALSO substituted a stale material row with the pinned default
/// (id 0) directly here; that was REMOVED (W1 blocking fix, post-review):
/// `dec_ref(slot, gen)` on a MATCHING-gen `Loading`/`Failed` row (a
/// resurrection carrier whose `inc_ref` was refused, so the row never left
/// `Loading`/`Failed` — see `Assets::inc_ref`'s doc) does NOT hit the
/// gen-mismatch guard and instead PROCEEDS to a real zero-crossing decrement,
/// silently retiring a row this system had no business retiring (and leaking
/// the returned `RetireTicket`, since this call site never enqueued it into
/// [`DeferredFree`]) — latent-dead while F6 has not landed, but F5 is meant
/// to be the hard, permanent gate, and this activates exactly when F6/F7 do.
/// The VISIBLE substitution (point a stale material at the pinned default) is
/// DEFERRED to F8, which has the `Entity`-in-query / `RenderStale`
/// infrastructure this needs to do it safely — see
/// `docs/ASSET-STREAMING-PLAN.md`'s "HARD PREREQ before async streaming" (d).
///
/// # `free_epoch` early-out — O(1) on every churn-free/golden frame
///
/// One `u64` load + compare against [`ValidateCursor::mesh`]; if the mesh
/// store's `free_epoch` has not advanced since the last observation, this
/// returns immediately — no query iteration, no command. `free_epoch` bumps
/// only on [`Assets::remove`] or a [`Assets::dec_ref`] zero-crossing (a real
/// (un)load), so a static/golden scene NEVER advances it — this is the
/// byte-identity argument's load-bearing fact.
///
/// # On a churn frame — O(visible) dense `u32`-compares, no random access
///
/// One pass over `(MeshHandle, MeshRefGen)` for every `Enabled<RenderEnabled>`
/// row (dense, archetype-order, L1-resident): `MeshRefGen(GEN_UNSYNCED)` means
/// "bound this frame, not yet synced" and is trusted (skipped — the sibling
/// `apply_refcount_deltas` system, `.before` this one, guarantees a real
/// binding is NEVER left at `GEN_UNSYNCED` past this point — see
/// `apply_one`'s doc); otherwise a gen-mismatch or non-`Loaded` state disables
/// the row.
///
/// # Raw carrier-index read sites downstream of this system
///
/// Because bare-slot carriers give up a gen-keyed map's free staleness
/// safety, THIS system is the sole backstop for the MESH side — every raw
/// `MeshHandle.0` read site in the render crate (`mesh_draw.rs`,
/// `csm_caster.rs`) is documented as relying on running downstream of this
/// system within the same frame (`apply → validate → gather`). There is no
/// symmetric material backstop today (see the DISABLE-ONLY / MESH-ONLY
/// section above) — a raw `MaterialHandle.0` resolve has no live consumer
/// pre-F8 (the raster hardcodes material 0), so nothing currently depends on
/// one.
///
/// # A renderable missing its ref-gen lane is SILENTLY SKIPPED
///
/// `#[require(MeshRefGen)]` / `#[require(MaterialRefGen)]` materialize the
/// lane on every `Commands::spawn`/`insert`-driven path (the `Bundle`
/// required-component expansion). They do NOT materialize on the raw
/// archetype-deserialize path (`boyko_ecs::ecs::core::serialize::load_writer::load_archetype`
/// calls `EcsMaster::create_archetype` directly with the FILE's own saved
/// component-id list — no `Bundle`/require expansion runs). A `MeshHandle`
/// row loaded from a save file that predates this lane (or was otherwise
/// captured without it) would silently fail to match `q_mesh`'s tuple query
/// (an AND-match on both components) and never be checked here — no panic,
/// no disable, just invisible exclusion from validation. Latent today (no
/// such legacy save file exists in-tree; a same-build save/load round-trip
/// serializes the lane like any other live column, since it IS present in the
/// archetype by the time anything gets saved) — flagged for the reviewer as
/// a version-skew edge the serialization rungs (S0-S3) did not anticipate.
// SystemParams are consumed by-value by the SystemParam contract.
#[allow(clippy::needless_pass_by_value)]
pub fn validate_asset_refs(
    q_mesh: Query<(&MeshHandle, &MeshRefGen), Enabled<RenderEnabled>>,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut cursor: ResMut<ValidateCursor>,
    mut cmd: Commands,
) {
    let new_epoch = mesh_assets.free_epoch();
    debug_assert!(
        new_epoch >= cursor.mesh,
        "invariant: Assets::free_epoch is monotonic non-decreasing (observed {new_epoch}, cursor {})",
        cursor.mesh
    );
    if new_epoch == cursor.mesh {
        return;
    }

    for (id, (&MeshHandle(slot), &MeshRefGen(g))) in q_mesh.iter_entities() {
        if g == GEN_UNSYNCED {
            continue;
        }
        let stale = mesh_assets.try_generation(slot) != Some(g)
            || mesh_assets.state_of_index(slot) != Some(AssetLoadState::Loaded);
        if stale {
            cmd.add(DisableStaleMeshCommand { id });
        }
    }

    cursor.mesh = new_epoch;
}

/// Wires the asset-streaming refcount pipeline into the app schedule
/// (asset-streaming plan F2 §1/§3, F5's validation): inserts the queue
/// resources ([`RefcountDeltas`], [`DeferredFree`], [`ValidateCursor`]) the
/// carrier hooks and both systems share, and registers
/// [`apply_refcount_deltas`] `.before(validate_asset_refs)`.
///
/// # The apply → validate edge is expressible; the validate → gather edge is NOT
///
/// Both systems are registered in the SAME [`App::add_systems_cfg`] closure
/// here, so the `SystemKey`-based `.before` edge between them is directly
/// expressible. The FURTHER edge this rung's design calls for — validation
/// running before `boyko_render::gather_mesh_draws` /
/// `gather_shadow_casters` — is **not** expressible from inside this plugin:
/// those systems are registered by a LATER, separate
/// `App::add_systems_cfg` closure in the composing host
/// (`boyko_app::plugins::EnginePlugins::build`), and a `SystemKey` cannot be
/// obtained for a system that does not exist yet at this plugin's build time
/// (mirrors the documented `CsmPlugin`/`ShadowAtlasPlugin`/`LightingPlugin`
/// cross-plugin limitation — "a `.after(key)` edge needs the target's
/// `SystemKey`, only obtainable inside the target's own builder closure").
/// The correctness this gap could threaten is bounded exactly like those:
/// `EnginePlugins::build` already composes `AssetRefcountPlugin` BEFORE the
/// mesh/CSM gather closure (add-order), and — as this system's own doc notes
/// — its churn-frame effect (disable a stale mesh row) is a bounded,
/// self-correcting one-frame-at-most visual transient, never a soundness
/// hazard (the durable refcount/gen-check guards do not depend on this
/// system's timing at all). Closing this gap
/// with a hard scheduler edge (e.g. a `add_asset_validate_systems(&mut
/// ScheduleBuilder) -> SystemKey` helper the host calls directly inside its
/// own gather closure, mirroring `add_gpu_transform_pack`) is host-composition
/// work, out of this crate's scope.
#[derive(Default)]
pub struct AssetRefcountPlugin;

impl Plugin for AssetRefcountPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RefcountDeltas::default());
        app.insert_resource(DeferredFree::default());
        app.insert_resource(ValidateCursor::default());
        app.add_systems_cfg(|b| {
            let apply = b.add_system(apply_refcount_deltas).key();
            b.add_system(validate_asset_refs).after(apply);
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

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1);
        assert!(free.is_empty(), "refcount 0->1 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1);
        assert!(free.is_empty(), "refcount 1->2 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1);
        assert!(free.is_empty(), "refcount 2->1 must not enqueue a retire");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1);
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

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, 1);

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1);
        assert_eq!(free.entries().len(), 1, "the first zero-crossing decrement enqueues once");

        let _ = apply_one(&mut assets, &mut free, AssetRefKind::Material, slot, GEN_UNSYNCED, -1);
        assert_eq!(
            free.entries().len(),
            1,
            "a second decrement on an already-Retiring slot must not enqueue again"
        );
    }
}
