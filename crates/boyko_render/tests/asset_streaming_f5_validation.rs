//! Asset-streaming plan F5 — cross-crate integration coverage for the
//! generation-lane validation net: [`apply_refcount_deltas`]'s unconditional
//! lane-stamp-on-a-refused-inc (the DELTA-1 blocker fix), mesh/material lane
//! independence, [`validate_asset_refs`]'s `free_epoch` early-out, and its
//! disable/substitute reaction to a force-reused (simulated F6) slot.
//!
//! Mirrors `asset_refcount_integration.rs`'s harness shape (a raw `EcsMaster`
//! plus direct `run_system` calls — the established ad-hoc-system test idiom,
//! see `boyko_ecs/tests/phase14a_hooks_firing.rs`) but additionally wires
//! [`validate_asset_refs`] and [`ValidateCursor`], and seeds BOTH
//! `Assets<MeshGpu>` and `Assets<Material>` with real rows — this suite's
//! mesh coverage needs actual mesh slots, unlike `asset_refcount_integration.rs`'s
//! material-only scope (see that file's doc for why `MeshGpu` was skipped
//! there).
//!
//! # `MeshGpu` without a device
//!
//! `MeshGpu` owns RHI buffers, but nothing in this file touches a real
//! device — [`dummy_mesh_gpu`] constructs one with a `VkBuffer::NULL` handle
//! (`BoundBuffer`'s fields are all `pub`) and no test ever calls a Vulkan
//! function on it. This is legal at the Rust type level: every assertion
//! below only moves the record through `Assets<MeshGpu>`'s store
//! (add/remove/inc_ref/dec_ref/try_generation/state_of_index) and the
//! `validate_asset_refs`/`apply_refcount_deltas` systems under test, neither
//! of which ever dereferences a device handle. Not a GPU/golden test — pure
//! CPU-side data-structure exercise, same class as `asset_refcount_integration.rs`.
//!
//! # No private-field access
//!
//! `Assets<T>`'s `refcount` column is crate-private to `boyko_ecs` — same
//! constraint as `asset_refcount_integration.rs` (see that file's doc).
//! Assertions here go through PUBLIC F5 cross-crate probes instead:
//! [`Assets::state_of_index`] (`None` for a non-Loaded/OOR row),
//! [`Assets::try_generation`]/[`Assets::generation`], and the
//! `MeshRefGen`/`MaterialRefGen` lane values read directly off an entity —
//! the exact same signals `validate_asset_refs` itself relies on, and the
//! observable surface a real `boyko_render` consumer reads.
//!
//! # Key gate: the DELTA-1 unconditional lane-stamp
//!
//! [`refused_inc_still_stamps_the_lane_unconditionally_on_a_retiring_mesh_slot`]
//! is the direct regression guard for the F5 blocker fix documented on
//! `apply_one` (`boyko_render::asset_refcount`): a carrier that binds an
//! ALREADY-`Retiring` slot has its `inc_ref` refused, yet its
//! `MeshRefGen`/`MaterialRefGen` lane must still be stamped to the slot's
//! REAL (Retiring) generation — never left at `GEN_UNSYNCED` — or
//! `validate_asset_refs` would wrongly trust it as "freshly bound" and never
//! disable it.

use boyko_ecs::ecs::core::asset::{AssetLoadState, Assets, GEN_UNSYNCED};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_render::asset_refcount::{ValidateCursor, validate_asset_refs};
use boyko_render::{Material, MeshGpu, RenderEpoch, apply_refcount_deltas};
use boyko_scene::{DeferredFree, MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, RefcountDeltas, RenderEnabled};

use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;

/// A device-inert `MeshGpu` — see this file's module doc ("`MeshGpu` without
/// a device") for why a `VkBuffer::NULL`-handled dummy is sound here.
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer { buffer: VkBuffer::NULL, offset: 0, size: 0, mapped: None };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 0,
        index_type: IndexType::Uint16,
        vertex_count: 0,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
    }
}

/// Builds an `EcsMaster` with the full F2+F5 refcount/validation pipeline's
/// resources inserted (mirrors `AssetRefcountPlugin::build`, minus the
/// App/Plugin scaffolding — see `asset_refcount_integration.rs`'s identical
/// `world_with` for the established idiom). Both `material_assets` and
/// `mesh_assets` are caller-seeded (rows already minted) BEFORE they move
/// into the world, since only the caller knows which slot(s) to reference.
fn world_with(material_assets: Assets<Material>, mesh_assets: Assets<MeshGpu>) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(RefcountDeltas::default());
    ecs.insert_resource(DeferredFree::default());
    ecs.insert_resource(ValidateCursor::default());
    // Asset-streaming plan F6: `apply_refcount_deltas` now reads `RenderEpoch` to
    // stamp a real fence-gated `retire_frame` — mirrors `AssetRefcountPlugin::build`.
    ecs.insert_resource(RenderEpoch::default());
    ecs.insert_resource(material_assets);
    ecs.insert_non_send_resource(mesh_assets);
    // Prime both ids (installs the on_insert/on_replace hooks) before any spawn —
    // the established idiom (see phase14a_hooks_firing.rs's T2/T7 tests).
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    ecs
}

fn despawn(ecs: &mut EcsMaster, e: Entity) {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
}

// ════════════════════════════════════════════════════════════════════════════
// KEY GATE — the DELTA-1 unconditional lane-stamp-on-refused-inc.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn refused_inc_still_stamps_the_lane_unconditionally_on_a_retiring_mesh_slot() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let slot = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut ecs = world_with(Assets::<Material>::with_reserved(4), mesh_assets);

    // Drive the slot to Retiring: a single owner attaches, then despawns.
    let a: Entity = ecs.run_system(move |mut cmds: Commands| cmds.spawn(MeshHandle(slot)).id());
    ecs.run_system(apply_refcount_deltas);
    despawn(&mut ecs, a);
    ecs.run_system(apply_refcount_deltas);

    assert_eq!(
        ecs.non_send_resource::<Assets<MeshGpu>>().state_of_index(slot),
        None,
        "the sole owner's despawn must have retired the slot (test precondition)"
    );
    let retiring_gen = ecs
        .non_send_resource::<Assets<MeshGpu>>()
        .try_generation(slot)
        .expect("a Retiring row is still in-range");

    // A SECOND carrier binds the SAME (now-Retiring) slot number — the
    // resurrection hazard F5 Decision 5 refuses.
    let b: Entity = ecs.run_system(move |mut cmds: Commands| cmds.spawn(MeshHandle(slot)).id());
    assert_eq!(
        ecs.get_component::<MeshRefGen>(b),
        Some(&MeshRefGen(GEN_UNSYNCED)),
        "a freshly spawned carrier's lane defaults to GEN_UNSYNCED (test precondition)"
    );

    ecs.run_system(apply_refcount_deltas);

    // inc_ref was refused: the slot must still read as non-Loaded (not
    // resurrected) — see `Assets::inc_ref`'s F5/F6 boundary doc.
    assert_eq!(
        ecs.non_send_resource::<Assets<MeshGpu>>().state_of_index(slot),
        None,
        "a refused inc_ref must not resurrect the Retiring slot to Loaded"
    );

    // THE KEY REGRESSION GUARD (F5 blocker fix / DELTA-1): the carrier's lane
    // must be stamped to the slot's (Retiring) generation regardless of the
    // refusal — leaving it at GEN_UNSYNCED would make `validate_asset_refs`
    // trust it as "freshly bound" and skip disabling a carrier that in fact
    // bound a dead slot (see `apply_one`'s doc for the full argument, and
    // this file's module doc for why refcount itself cannot be re-checked
    // here directly).
    let stamped =
        ecs.get_component::<MeshRefGen>(b).copied().expect("MeshHandle #[require]s MeshRefGen");
    assert_ne!(
        stamped.0, GEN_UNSYNCED,
        "apply_one must stamp the lane even when inc_ref is refused (F5 blocker fix)"
    );
    assert_eq!(
        stamped.0, retiring_gen,
        "the stamped generation must be the slot's actual (Retiring) generation"
    );
}

/// Material twin of the mesh gate above — same DELTA-1 fix, same
/// generic-over-`AssetBacking` `apply_one` body.
#[test]
fn refused_inc_still_stamps_the_lane_unconditionally_on_a_retiring_material_slot() {
    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets, Assets::<MeshGpu>::with_reserved(4));

    let a: Entity = ecs.run_system(move |mut cmds: Commands| cmds.spawn(MaterialHandle(slot)).id());
    ecs.run_system(apply_refcount_deltas);
    despawn(&mut ecs, a);
    ecs.run_system(apply_refcount_deltas);

    assert_eq!(
        ecs.resource::<Assets<Material>>().state_of_index(u32::from(slot)),
        None,
        "the sole owner's despawn must have retired the slot (test precondition)"
    );
    let retiring_gen = ecs
        .resource::<Assets<Material>>()
        .try_generation(u32::from(slot))
        .expect("a Retiring row is still in-range");

    let b: Entity = ecs.run_system(move |mut cmds: Commands| cmds.spawn(MaterialHandle(slot)).id());
    assert_eq!(ecs.get_component::<MaterialRefGen>(b), Some(&MaterialRefGen(GEN_UNSYNCED)));

    ecs.run_system(apply_refcount_deltas);

    assert_eq!(
        ecs.resource::<Assets<Material>>().state_of_index(u32::from(slot)),
        None,
        "a refused inc_ref must not resurrect the Retiring slot to Loaded"
    );
    let stamped = ecs
        .get_component::<MaterialRefGen>(b)
        .copied()
        .expect("MaterialHandle #[require]s MaterialRefGen");
    assert_ne!(
        stamped.0, GEN_UNSYNCED,
        "apply_one must stamp the lane even when inc_ref is refused (F5 blocker fix)"
    );
    assert_eq!(stamped.0, retiring_gen);
}

// ════════════════════════════════════════════════════════════════════════════
// Two-lane independence.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn mesh_churn_leaves_material_lane_untouched_and_vice_versa() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let slot_mesh_a = mesh_assets.add(dummy_mesh_gpu()).index();
    let slot_mesh_b = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot_mat_a = material_assets.add(Material::default()).index() as u16;
    let slot_mat_b = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets, mesh_assets);

    let e: Entity = ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(MeshHandle(slot_mesh_a)).insert(MaterialHandle(slot_mat_a)).id()
    });
    ecs.run_system(apply_refcount_deltas);

    let mesh_gen_before = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present");
    let mat_gen_before = ecs.get_component::<MaterialRefGen>(e).copied().expect("lane present");
    assert_ne!(mesh_gen_before.0, GEN_UNSYNCED, "the initial apply must have synced the mesh lane");
    assert_ne!(mat_gen_before.0, GEN_UNSYNCED, "the initial apply must have synced the material lane");

    // Churn the MESH side only: rebind to a different mesh slot.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(MeshHandle(slot_mesh_b));
    });
    ecs.run_system(apply_refcount_deltas);

    assert_eq!(
        ecs.get_component::<MaterialRefGen>(e).copied(),
        Some(mat_gen_before),
        "mesh-side churn must not perturb the MaterialRefGen lane"
    );
    let mesh_gen_after_mesh_churn = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present");
    // NOTE: comparing against `mesh_gen_before` directly (inequality) would be
    // WRONG — `slot_mesh_a` and `slot_mesh_b` are both FRESH rows (never
    // reused), so both start at generation 0 and the lane VALUE can
    // coincidentally match even though the SLOT changed. The real invariant
    // is that the lane tracks slot_mesh_b's actual current generation.
    assert_eq!(
        mesh_gen_after_mesh_churn.0,
        ecs.non_send_resource::<Assets<MeshGpu>>().generation(slot_mesh_b),
        "the mesh rebind must have re-synced MeshRefGen to the NEW slot's actual generation"
    );

    // Churn the MATERIAL side only: rebind to a different material slot.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(MaterialHandle(slot_mat_b));
    });
    ecs.run_system(apply_refcount_deltas);

    assert_eq!(
        ecs.get_component::<MeshRefGen>(e).copied(),
        Some(mesh_gen_after_mesh_churn),
        "material-side churn must not perturb the MeshRefGen lane"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// validate_asset_refs — O(1) early-out on a stable epoch.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn validate_asset_refs_is_a_no_op_on_a_stable_epoch() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let slot_mesh = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot_mat = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets, mesh_assets);

    let e: Entity = ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(MeshHandle(slot_mesh))
            .insert(MaterialHandle(slot_mat))
            .enable::<RenderEnabled>()
            .id()
    });
    ecs.run_system(apply_refcount_deltas);

    let mesh_handle_before = ecs.get_component::<MeshHandle>(e).copied();
    let mat_handle_before = ecs.get_component::<MaterialHandle>(e).copied();
    let mesh_gen_before = ecs.get_component::<MeshRefGen>(e).copied();
    let mat_gen_before = ecs.get_component::<MaterialRefGen>(e).copied();
    let enabled_before = ecs.is_enabled::<RenderEnabled>(e);
    assert!(enabled_before, "test precondition: the row starts enabled");

    // Neither store has retired anything since the fresh `ValidateCursor`
    // Default (both epochs 0) — the O(1) early-out path.
    ecs.run_system(validate_asset_refs);

    assert_eq!(
        ecs.get_component::<MeshHandle>(e).copied(),
        mesh_handle_before,
        "a stable-epoch validate must not touch MeshHandle"
    );
    assert_eq!(
        ecs.get_component::<MaterialHandle>(e).copied(),
        mat_handle_before,
        "a stable-epoch validate must not touch MaterialHandle"
    );
    assert_eq!(
        ecs.get_component::<MeshRefGen>(e).copied(),
        mesh_gen_before,
        "a stable-epoch validate must not touch MeshRefGen"
    );
    assert_eq!(
        ecs.get_component::<MaterialRefGen>(e).copied(),
        mat_gen_before,
        "a stable-epoch validate must not touch MaterialRefGen"
    );
    assert_eq!(
        ecs.is_enabled::<RenderEnabled>(e),
        enabled_before,
        "a stable-epoch validate must not touch RenderEnabled"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Force-reuse staleness — the "simulate F6" scenario.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn a_force_reused_mesh_slot_is_disabled_by_validate_without_refcount_corruption() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let h = mesh_assets.add(dummy_mesh_gpu());
    let slot = h.index();

    let mut ecs = world_with(Assets::<Material>::with_reserved(4), mesh_assets);

    let e: Entity =
        ecs.run_system(move |mut cmds: Commands| cmds.spawn(MeshHandle(slot)).enable::<RenderEnabled>().id());
    ecs.run_system(apply_refcount_deltas);
    assert!(ecs.is_enabled::<RenderEnabled>(e), "test precondition: starts enabled");
    let stale_gen = ecs.get_component::<MeshRefGen>(e).copied().expect("lane present").0;
    assert_ne!(stale_gen, GEN_UNSYNCED, "the apply must have synced the lane (test precondition)");

    // Simulate F6 (not yet landed): an external actor force-frees the slot,
    // bypassing the refcount-driven Retiring path entirely, and the freed row
    // gets reused (LIFO) by a fresh, unrelated mint — exactly the
    // "retired-and-reused underneath a lost/stale ref" hazard `dec_ref`'s
    // gen-check and `validate_asset_refs` jointly defend against.
    ecs.non_send_resource_mut::<Assets<MeshGpu>>().remove(h);
    let new_h = ecs.non_send_resource_mut::<Assets<MeshGpu>>().add(dummy_mesh_gpu());
    assert_eq!(new_h.index(), slot, "LIFO reuse must hand the freed row back at the same index");
    assert_ne!(
        new_h.generation(),
        stale_gen,
        "the reused row's generation must differ from the stale carrier's bound one"
    );

    ecs.run_system(validate_asset_refs);

    assert!(
        !ecs.is_enabled::<RenderEnabled>(e),
        "the stale carrier's row must be disabled once validate observes the gen mismatch"
    );

    // No refcount corruption: the entity's eventual detach (a gen-mismatched
    // -1) must be suppressed by `dec_ref`'s gen-check, never retiring the NEW
    // tenant that now legitimately owns the slot.
    despawn(&mut ecs, e);
    ecs.run_system(apply_refcount_deltas);
    assert!(
        ecs.resource::<DeferredFree>().is_empty(),
        "the stale carrier's despawn-triggered -1 must be suppressed by the gen-check, not \
         retire the slot's NEW tenant"
    );
    assert_eq!(
        ecs.non_send_resource::<Assets<MeshGpu>>().state_of_index(new_h.index()),
        Some(AssetLoadState::Loaded),
        "the new tenant must remain untouched and Loaded"
    );
}
