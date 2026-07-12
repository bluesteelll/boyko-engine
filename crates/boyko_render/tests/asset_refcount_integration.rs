//! Asset-streaming plan F2 §1/§3 — ECS integration coverage for the refcount
//! lifetime driver: `MaterialHandle` carrier hooks (`boyko_scene::render_caps`)
//! pushing `RefDelta`s into `RefcountDeltas`, folded by `apply_refcount_deltas`
//! (this crate) into `Assets<Material>`, retiring into `DeferredFree`.
//!
//! `MaterialHandle` (a bare `u16` carrier, no `#[require(...)]`) is the vehicle
//! — `MeshHandle` mirrors the identical two-hook wiring (see
//! `boyko_scene::render_caps`'s module doc) but its asset (`MeshGpu`) owns RHI
//! buffers and cannot be minted without a real device. `Material` is POD
//! (`Default`, no device handle) and needs none. `Assets<MeshGpu>` is still
//! inserted EMPTY (a `NonSendResMut<Assets<MeshGpu>>` is one of
//! `apply_refcount_deltas`'s four system params, resolved unconditionally) so
//! the system's parameters are satisfiable; no `MeshHandle` ever fires in
//! these tests, so it stays untouched.
//!
//! # No private-field access
//!
//! `Assets<T>`'s `refcount` column is crate-private to `boyko_ecs` — this
//! integration test crate cannot read it directly. Every assertion instead
//! goes through the PUBLIC `DeferredFree` retire-enqueue signal: "refcount hit
//! zero" is observed as "exactly one `FreeEntry` for that slot now exists",
//! and "still alive" as "no `FreeEntry` for that slot exists yet" — the same
//! signal a real streaming consumer (a future F6 drain) would read.
//!
//! # Key gate: #4 (shared-handle single-decrement)
//!
//! [`shared_handle_despawn_one_decrements_by_exactly_one_not_two`] is the
//! direct validator of `Assets::dec_ref`'s doc'd "only `on_replace` decrements"
//! deviation from a literal `on_insert`/`on_replace`/`on_remove` three-hook
//! wiring (see `render_caps.rs`'s module doc): if a regression wired
//! `on_remove` to ALSO push a `-1`, a despawn while other owners remain would
//! decrement refcount by 2 instead of 1, and this test's mid-loop
//! `DeferredFree::is_empty()` assertion would catch the resulting premature
//! retire before the last owner is even gone.

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_render::{Material, MeshGpu, RenderEpoch, apply_refcount_deltas};
use boyko_scene::{DeferredFree, MaterialHandle, RefcountDeltas};

/// Builds an `EcsMaster` with the F2 refcount pipeline's resources inserted
/// (mirrors `AssetRefcountPlugin::build`, minus the App/Plugin scaffolding — a
/// raw `EcsMaster` + direct `run_system` calls is the established ad-hoc-system
/// test harness; see `boyko_ecs/tests/phase14a_hooks_firing.rs`). `material_assets`
/// is caller-seeded (rows already minted) BEFORE it moves into the world, since
/// only the caller knows which slot(s) to reference from a `MaterialHandle`.
fn world_with(material_assets: Assets<Material>) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(RefcountDeltas::default());
    ecs.insert_resource(DeferredFree::default());
    // Asset-streaming plan F6: `apply_refcount_deltas` now reads `RenderEpoch` to
    // stamp a real fence-gated `retire_frame` — mirrors `AssetRefcountPlugin::build`.
    ecs.insert_resource(RenderEpoch::default());
    ecs.insert_resource(material_assets);
    ecs.insert_non_send_resource(Assets::<MeshGpu>::default());
    // Prime the id (installs the on_insert/on_replace hooks) before any spawn —
    // the established idiom (see phase14a_hooks_firing.rs's T2/T7 tests).
    let _ = MaterialHandle::component_id();
    ecs
}

fn despawn(ecs: &mut EcsMaster, e: Entity) {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
}

// ════════════════════════════════════════════════════════════════════════════
// #1 + #3 — a single owner's insert drives refcount to 1; its despawn drives
//           refcount to 0, transitioning the slot to Retiring and enqueuing a
//           DeferredFree entry.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn single_owner_insert_then_despawn_drives_refcount_to_one_then_retiring() {
    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets);

    let e: Entity = ecs.run_system(move |mut cmds: Commands| cmds.spawn(MaterialHandle(slot)).id());
    ecs.run_system(apply_refcount_deltas);
    assert!(
        ecs.resource::<DeferredFree>().is_empty(),
        "a single insert (refcount 0->1) must not retire the slot"
    );

    despawn(&mut ecs, e);
    ecs.run_system(apply_refcount_deltas);

    let entries = ecs.resource::<DeferredFree>().entries();
    assert_eq!(
        entries.len(),
        1,
        "the sole owner's despawn (refcount 1->0) must retire exactly the one slot"
    );
    assert_eq!(
        entries[0].slot, slot as u32,
        "the retired slot must be the one the despawned entity carried"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// #2 — an in-place rebind (`insert` on the SAME entity, same archetype ⇒
//      on_replace(OLD) then on_insert(NEW)) balances old-decrement /
//      new-increment: NOT a double-decrement on old (which would kill it out
//      from under a still-live second owner), NOT a double-increment on new
//      (which would take two despawns, not one, to retire it).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn inplace_rebind_balances_old_decrement_and_new_increment() {
    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot_old = material_assets.add(Material::default()).index() as u16;
    let slot_new = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets);

    // Two owners on slot_old: A keeps it alive across B's rebind.
    let (a, b): (Entity, Entity) = ecs.run_system(move |mut cmds: Commands| {
        let a = cmds.spawn(MaterialHandle(slot_old)).id();
        let b = cmds.spawn(MaterialHandle(slot_old)).id();
        (a, b)
    });
    ecs.run_system(apply_refcount_deltas);
    assert!(
        ecs.resource::<DeferredFree>().is_empty(),
        "two fresh inserts on slot_old must not retire anything"
    );

    // Rebind B in place: slot_old -1 (on_replace, dying value), slot_new +1
    // (on_insert, new value) — one Commands::insert call.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(b).insert(MaterialHandle(slot_new));
    });
    ecs.run_system(apply_refcount_deltas);
    assert!(
        ecs.resource::<DeferredFree>().is_empty(),
        "balanced rebind: slot_old (still owned by A, 2->1) must NOT retire — a double-decrement \
         here would wrongly kill an asset out from under A's still-live MaterialHandle"
    );

    // slot_new must now hold EXACTLY refcount 1 (not 0 — the insert must have
    // fired at all; not 2 — a double-increment): despawning its ONLY owner (B)
    // must retire it after exactly this ONE despawn.
    despawn(&mut ecs, b);
    ecs.run_system(apply_refcount_deltas);
    let entries = ecs.resource::<DeferredFree>().entries();
    assert_eq!(
        entries.len(),
        1,
        "slot_new's sole owner (B) despawning must retire slot_new exactly once"
    );
    assert_eq!(entries[0].slot, slot_new as u32, "the retired slot must be slot_new, not slot_old");

    // slot_old must still be alive (owned by A) — despawning A now must be the
    // thing that FINALLY retires it, proving the rebind's decrement on slot_old
    // was exactly -1 (not -2, which would have retired it prematurely above).
    despawn(&mut ecs, a);
    ecs.run_system(apply_refcount_deltas);
    let entries = ecs.resource::<DeferredFree>().entries();
    assert_eq!(entries.len(), 2, "A's despawn must retire slot_old — the second retire recorded");
    assert!(
        entries.iter().any(|e| e.slot == slot_old as u32),
        "slot_old must now appear among the retired entries"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// #4 — THE KEY GATE: a shared handle (N entities on ONE slot). Despawning ONE
//      of N must decrement refcount by exactly 1 (not 2); the slot only
//      transitions to Retiring after the LAST owner is despawned.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn shared_handle_despawn_one_decrements_by_exactly_one_not_two() {
    let mut material_assets = Assets::<Material>::with_reserved(4);
    let slot = material_assets.add(Material::default()).index() as u16;

    let mut ecs = world_with(material_assets);

    const N: usize = 3;
    let entities: Vec<Entity> = ecs.run_system(move |mut cmds: Commands| {
        (0..N).map(|_| cmds.spawn(MaterialHandle(slot)).id()).collect()
    });
    ecs.run_system(apply_refcount_deltas);
    assert!(
        ecs.resource::<DeferredFree>().is_empty(),
        "N fresh inserts on ONE shared slot must not retire it (refcount == N > 0)"
    );

    // Despawn N-1 of the N owners, one at a time, applying after each. A
    // premature retire here means a despawn decremented refcount by 2 (the
    // on_replace+on_remove double-hook regression this test guards against),
    // saturating the count to 0 before the LAST owner is even gone.
    for &e in &entities[..N - 1] {
        despawn(&mut ecs, e);
        ecs.run_system(apply_refcount_deltas);
        assert!(
            ecs.resource::<DeferredFree>().is_empty(),
            "slot must stay alive while at least one owner remains — a retire here means a single \
             despawn decremented refcount by 2, not 1"
        );
    }

    // The LAST owner's despawn must be the one that retires the slot.
    despawn(&mut ecs, entities[N - 1]);
    ecs.run_system(apply_refcount_deltas);
    let entries = ecs.resource::<DeferredFree>().entries();
    assert_eq!(entries.len(), 1, "exactly one retire must happen, on the LAST owner's despawn");
    assert_eq!(entries[0].slot, slot as u32);
}
