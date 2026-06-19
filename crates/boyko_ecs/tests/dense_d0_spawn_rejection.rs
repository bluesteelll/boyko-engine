//! Dense plan D0 — out-of-crate spawn-rejection + signature-exclusion suite.
//!
//! D0 adds `StorageKind::Dense` as a NON-signature storage kind: a dense id is
//! registrable, classified `Dense`, excluded from every archetype signature, and
//! owns NO per-archetype `ComponentPool` (its global `DenseStore` lands in D1).
//! At D0 there is therefore nowhere to write a dense component's bytes during a
//! structural spawn, so the load-bearing soundness property is:
//!
//!   A spawn whose component list contains a dense id must be a CLEAN REJECTION
//!   (`Err`, no panic, no partial write, no EntityId leak) — NOT a silent
//!   success that drops the dense data.
//!
//! This file lives OUT-OF-CRATE so every assertion proves the property through
//! the public surface (`EcsMaster::{new,create_archetype,create_entity}`, the
//! `pub install_dense_storage_kind` derive-install wrapper, and the public
//! `storage_kind` / `is_signature_storage` readers). The dense id is classified
//! through the SAME public path the `#[component(storage = "dense")]` derive
//! emits (`install_dense_storage_kind::<C>(raw)`), so the test exercises the real
//! registration ordering, not a crate-private shortcut.
//!
//! # Fixture id allocation
//!
//! Fixed ids in the [360, 372) block, grep-verified free in the shared
//! out-of-crate test process (disjoint from the lib-test 320-360 fixtures, which
//! run in a different binary). Each dense fixture is a hand `impl Component` with
//! `STORAGE_IS_DENSE = true`; classification runs once per id via the public
//! `install_dense_storage_kind` (idempotent through `set_storage_kind`).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    self, ResidencyKind, StorageKind,
};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsError, EcsMaster};

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A dense-storage component (hand impl mirroring the derive's `storage =
/// "dense"` emission: `STORAGE_IS_DENSE = true`, default `Cpu` residency).
#[repr(C)]
#[derive(Clone, Copy)]
struct DenseHealth {
    hp: u32,
}

const DENSE_HEALTH_ID: ComponentId = ComponentId(360);

impl Component for DenseHealth {
    #[inline]
    fn component_id() -> ComponentId {
        DENSE_HEALTH_ID
    }
    const STORAGE_IS_DENSE: bool = true;
}

/// A normal table-storage data component (the sibling that DOES get a pool).
#[repr(C)]
#[derive(Clone, Copy)]
struct TablePos {
    x: u32,
}

const TABLE_POS_ID: ComponentId = ComponentId(361);

impl Component for TablePos {
    #[inline]
    fn component_id() -> ComponentId {
        TABLE_POS_ID
    }
}

/// Registers the fixture layouts + classifies the dense id through the public
/// derive-install path. Idempotent across the shared test process.
fn prime_fixtures() {
    component_registry::register_layout::<DenseHealth>(DENSE_HEALTH_ID.0);
    component_registry::register_layout::<TablePos>(TABLE_POS_ID.0);
    // The exact call the `#[component(storage = "dense")]` derive emits into the
    // `component_id()` OnceLock init closure.
    component_registry::install_dense_storage_kind::<DenseHealth>(DENSE_HEALTH_ID.0);
}

// ── 1. Classification round-trips through the public reader (the C1 #0 fix) ───

#[test]
fn dense_id_classifies_dense_and_excluded_from_signature() {
    prime_fixtures();

    assert_eq!(
        component_registry::storage_kind(DENSE_HEALTH_ID.0),
        StorageKind::Dense,
        "the public install path must classify the id as Dense (C1 #0 reader)"
    );
    assert!(
        !component_registry::is_signature_storage(component_registry::storage_kind(
            DENSE_HEALTH_ID.0
        )),
        "Dense is a NON-signature storage kind (excluded from every signature)"
    );
    assert!(
        component_registry::is_signature_storage(component_registry::storage_kind(TABLE_POS_ID.0)),
        "Table is the signature storage kind"
    );
    assert_eq!(
        component_registry::residency_class(DENSE_HEALTH_ID.0),
        ResidencyKind::Cpu,
        "a dense id is always Cpu-resident (W1)"
    );
}

// ── 2. THE spawn-rejection property: clean Err, no corruption ─────────────────

/// A multi-component spawn whose list contains a dense id is REJECTED cleanly:
/// the archetype resolved from `[Table, Dense]` excludes the dense id (so it has
/// no pool for it), and `create_entity` with the dense byte-slice present fails
/// the two-phase `can_push_entity_components` guard BEFORE any pool is mutated.
/// The result is `Err(ArchetypeRejectedEntity)` with NO panic and NO partial
/// write — and a subsequent NORMAL spawn into the same archetype still works.
#[test]
fn dense_bearing_spawn_is_cleanly_rejected_and_state_survives() {
    prime_fixtures();
    let mut ecs = EcsMaster::new();

    // Archetype resolved from a list CONTAINING the dense id: the dense bit is
    // filtered out of the signature, so the archetype's real signature is
    // {TablePos} and it owns ONLY a TablePos pool.
    let arch = ecs.create_archetype(&[TABLE_POS_ID, DENSE_HEALTH_ID]);

    // Baseline: a normal table-only push into this archetype succeeds. This is
    // the "subsequent normal spawn still works" anchor — captured BEFORE the
    // rejected attempt so we also prove no PRE-corruption.
    let pos0 = TablePos { x: 11 };
    let before = ecs
        .create_entity(arch, &[(TABLE_POS_ID, &pos0.x.to_ne_bytes())])
        .expect("table-only spawn must succeed into the table archetype");

    // The rejected attempt: spawn with BOTH a table byte-slice and a dense
    // byte-slice. The dense id has no pool in `arch`, so the push must be
    // rejected without writing anything.
    let pos1 = TablePos { x: 22 };
    let hp1 = DenseHealth { hp: 99 };
    let rejected = ecs.create_entity(
        arch,
        &[
            (TABLE_POS_ID, &pos1.x.to_ne_bytes()),
            (DENSE_HEALTH_ID, &hp1.hp.to_ne_bytes()),
        ],
    );

    // (a) It is an Err — a CLEAN rejection, not silent success-with-dropped-data.
    assert!(
        rejected.is_err(),
        "a spawn whose list contains a dense id must be rejected (no silent drop)"
    );
    assert_eq!(
        rejected.unwrap_err(),
        EcsError::ArchetypeRejectedEntity {
            archetype_id: arch
        },
        "the rejection is the two-phase can_push guard (ArchetypeRejectedEntity)"
    );

    // (b) No partial write: the archetype still holds exactly ONE entity (the
    // baseline). The rejected push touched no pool (two-phase commit) and leaked
    // no EntityId.
    assert!(
        ecs.has_entity(before),
        "the pre-existing entity survives the rejected spawn"
    );

    // (c) A subsequent NORMAL spawn still works — proves no state corruption /
    // no leaked id / no desynced pool from the rejected attempt.
    let pos2 = TablePos { x: 33 };
    let after = ecs
        .create_entity(arch, &[(TABLE_POS_ID, &pos2.x.to_ne_bytes())])
        .expect("a normal spawn must still succeed after a rejected dense spawn");
    assert!(ecs.has_entity(after), "the post-rejection entity is live");
    assert_ne!(
        before.id(),
        after.id(),
        "the rejected attempt did not leak/recycle an id into the live set"
    );

    // (d) The surviving rows carry the correct table data — the rejected dense
    // bytes were never written anywhere observable.
    let v_before = ecs
        .get_component::<TablePos>(before)
        .expect("before entity has TablePos")
        .x;
    let v_after = ecs
        .get_component::<TablePos>(after)
        .expect("after entity has TablePos")
        .x;
    assert_eq!(v_before, 11, "pre-existing row data intact");
    assert_eq!(v_after, 33, "post-rejection row data correct");
}

// ── 3. Single dense component → empty signature, push rejected ────────────────

/// An archetype resolved from a list of ONLY a dense id has an EMPTY signature
/// (the dense bit is filtered out) and owns no pools. A spawn that tries to push
/// the dense byte-slice into it is rejected; a zero-component push into the same
/// (empty) archetype succeeds — proving the archetype itself is well-formed, it
/// is specifically the dense byte-slice that is unaccepted.
#[test]
fn lone_dense_archetype_is_empty_and_rejects_dense_bytes() {
    prime_fixtures();
    let mut ecs = EcsMaster::new();

    // Resolving `[Dense]` filters to the EMPTY signature.
    let arch = ecs.create_archetype(&[DENSE_HEALTH_ID]);

    // A zero-component push into this archetype succeeds (it IS the empty
    // archetype — well-formed, just hosts no pools).
    let empty_entity = ecs
        .create_entity(arch, &[])
        .expect("a zero-component push into the empty archetype must succeed");
    assert!(ecs.has_entity(empty_entity));

    // Pushing the dense byte-slice is rejected (no pool for the dense id).
    let hp = DenseHealth { hp: 7 };
    let rejected = ecs.create_entity(arch, &[(DENSE_HEALTH_ID, &hp.hp.to_ne_bytes())]);
    assert!(
        rejected.is_err(),
        "pushing dense bytes into the (empty) lone-dense archetype must be rejected"
    );

    // The empty entity still lives after the rejected push.
    assert!(
        ecs.has_entity(empty_entity),
        "the empty-archetype entity survives the rejected dense push"
    );
}

// ── 4. A dense id mixed into a wider table archetype does not fragment ─────────

/// Two archetypes resolved from `[Table]` and `[Table, Dense]` are the SAME
/// archetype: the dense bit never enters the signature, so it cannot fragment
/// the archetype space (the core non-fragmentation premise of dense storage).
#[test]
fn dense_id_does_not_fragment_the_archetype_space() {
    prime_fixtures();
    let mut ecs = EcsMaster::new();

    let table_only = ecs.create_archetype(&[TABLE_POS_ID]);
    let table_plus_dense = ecs.create_archetype(&[TABLE_POS_ID, DENSE_HEALTH_ID]);

    assert_eq!(
        table_only, table_plus_dense,
        "[Table] and [Table, Dense] must resolve to the SAME archetype \
         (dense never fragments the signature)"
    );
}
