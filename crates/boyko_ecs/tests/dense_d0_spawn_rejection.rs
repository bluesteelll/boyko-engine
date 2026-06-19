//! Dense plan D0/D2 — out-of-crate signature-exclusion + spawn-routing suite.
//!
//! D0 adds `StorageKind::Dense` as a NON-signature storage kind: a dense id is
//! registrable, classified `Dense`, excluded from every archetype signature, and
//! owns NO per-archetype `ComponentPool` (its global `DenseStore` is owned by the
//! per-world `DenseRegistry`).
//!
//! D2 ROUTES the structural ops: a `create_entity` list that contains a dense id
//! is PARTITIONED — the table subset is written into the archetype and the dense
//! subset is routed to its global `DenseStore` (no migration). The D0-era
//! "dense byte in a spawn list ⇒ clean `Err` rejection" contract is therefore
//! SUPERSEDED by D2: the dense bytes are now ACCEPTED and stored, not rejected.
//! The load-bearing soundness property at D2 is:
//!
//!   A spawn whose component list contains a dense id SUCCEEDS — the table subset
//!   lands in the archetype, the dense subset lands in its `DenseStore`, no
//!   archetype fragmentation, no partial write, no EntityId leak.
//!
//! This file lives OUT-OF-CRATE so every assertion proves the property through
//! the public surface (`EcsMaster::{new,create_archetype,create_entity,
//! dense_contains}`, the `pub install_dense_storage_kind` derive-install wrapper,
//! and the public `storage_kind` / `is_signature_storage` readers). The dense id
//! is classified through the SAME public path the `#[component(storage =
//! "dense")]` derive emits (`install_dense_storage_kind::<C>(raw)`), so the test
//! exercises the real registration ordering, not a crate-private shortcut.
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
use boyko_ecs::prelude::EcsMaster;

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

// ── 2. THE D2 spawn-routing property: dense subset routed, no corruption ───────

/// A multi-component spawn whose list contains a dense id SUCCEEDS at D2: the
/// archetype resolved from `[Table, Dense]` excludes the dense id (so it has no
/// pool for it), and `create_entity` PARTITIONS the input — the table subset is
/// written into the archetype, the dense subset is routed to its global
/// `DenseStore` (no migration). NO panic, NO partial write, NO EntityId leak —
/// and the table data + dense membership are both observable afterwards.
#[test]
fn dense_bearing_spawn_routes_and_state_survives() {
    prime_fixtures();
    let mut ecs = EcsMaster::new();

    // Archetype resolved from a list CONTAINING the dense id: the dense bit is
    // filtered out of the signature, so the archetype's real signature is
    // {TablePos} and it owns ONLY a TablePos pool.
    let arch = ecs.create_archetype(&[TABLE_POS_ID, DENSE_HEALTH_ID]);

    // Baseline: a normal table-only push into this archetype succeeds. Captured
    // BEFORE the dense spawn so we also prove no PRE-corruption.
    let pos0 = TablePos { x: 11 };
    let before = ecs
        .create_entity(arch, &[(TABLE_POS_ID, &pos0.x.to_ne_bytes())])
        .expect("table-only spawn must succeed into the table archetype");

    // The D2 routed spawn: BOTH a table byte-slice and a dense byte-slice. The
    // dense id has no pool in `arch`; D2 routes it to its `DenseStore` (the
    // table subset still writes the pool). The whole spawn SUCCEEDS.
    let pos1 = TablePos { x: 22 };
    let hp1 = DenseHealth { hp: 99 };
    let routed = ecs
        .create_entity(
            arch,
            &[
                (TABLE_POS_ID, &pos1.x.to_ne_bytes()),
                (DENSE_HEALTH_ID, &hp1.hp.to_ne_bytes()),
            ],
        )
        .expect("a D2 spawn whose list contains a dense id must SUCCEED (routed, not rejected)");

    // (a) The dense subset landed in the global DenseStore (membership recorded).
    assert!(
        ecs.dense_contains(routed, DENSE_HEALTH_ID),
        "the dense subset is routed to its DenseStore (D2), not dropped"
    );

    // (b) The table subset landed in the archetype pool (the routed entity keeps
    // its table component, unmigrated).
    assert_eq!(
        ecs.get_component::<TablePos>(routed)
            .expect("routed entity has TablePos")
            .x,
        22,
        "the table subset of a dense-bearing spawn is written to the pool"
    );

    // (c) No partial write / no leaked id: the baseline entity still lives and a
    // subsequent NORMAL spawn still works with a distinct id.
    assert!(ecs.has_entity(before), "the pre-existing entity survives");
    let pos2 = TablePos { x: 33 };
    let after = ecs
        .create_entity(arch, &[(TABLE_POS_ID, &pos2.x.to_ne_bytes())])
        .expect("a normal spawn must still succeed after a routed dense spawn");
    assert!(ecs.has_entity(after), "the post-routing entity is live");
    assert_ne!(before.id(), after.id(), "no leaked/recycled id");
    assert_ne!(before.id(), routed.id(), "the routed dense spawn took a fresh id");

    // (d) The surviving rows carry the correct table data.
    assert_eq!(
        ecs.get_component::<TablePos>(before).expect("before TablePos").x,
        11,
        "pre-existing row data intact"
    );
    assert_eq!(
        ecs.get_component::<TablePos>(after).expect("after TablePos").x,
        33,
        "post-routing row data correct"
    );
}

// ── 3. Single dense component → empty signature, dense bytes routed ────────────

/// An archetype resolved from a list of ONLY a dense id has an EMPTY signature
/// (the dense bit is filtered out) and owns no pools. At D2 a spawn that supplies
/// the dense byte-slice into it SUCCEEDS — the empty-signature archetype hosts the
/// entity row and the dense subset is routed to its global `DenseStore`.
#[test]
fn lone_dense_archetype_is_empty_and_routes_dense_bytes() {
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

    // D2: supplying the dense byte-slice SUCCEEDS — the table subset is empty
    // (empty-signature archetype) and the dense subset is routed to the store.
    let hp = DenseHealth { hp: 7 };
    let routed = ecs
        .create_entity(arch, &[(DENSE_HEALTH_ID, &hp.hp.to_ne_bytes())])
        .expect("a lone-dense spawn into the empty archetype must SUCCEED (D2 routed)");
    assert!(
        ecs.dense_contains(routed, DENSE_HEALTH_ID),
        "the lone dense subset is routed to its DenseStore (D2)"
    );

    // The empty entity still lives.
    assert!(
        ecs.has_entity(empty_entity),
        "the empty-archetype entity survives the routed dense spawn"
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
