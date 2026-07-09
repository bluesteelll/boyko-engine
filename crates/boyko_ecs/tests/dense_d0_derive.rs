//! Dense plan D0 — `#[component(storage = "dense")]` derive-emission smoke test.
//!
//! Proves the derive arm end-to-end through the REAL macro (not a hand `impl`):
//!   * `#[component(storage = "dense")]` compiles and sets
//!     `Component::STORAGE_IS_DENSE = true` (the trait default is `false`);
//!   * the minted id classifies as `StorageKind::Dense` once `component_id()`
//!     runs the `OnceLock` install closure (the derive emits
//!     `install_dense_storage_kind::<Self>`);
//!   * the dense id is excluded from a signature it is listed in;
//!   * a NON-dense `#[derive(Component)]` keeps the `STORAGE_IS_DENSE = false`
//!     default (the 0%-gate — a world with no dense type is byte-identical).
//!
//! Out-of-crate so the derive's emitted `boyko_ecs::...` paths are exercised at
//! their real downstream-crate resolution.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self, StorageKind};
use boyko_ecs::prelude::EcsMaster;
use boyko_macros::Component;

/// A dense component minted through the real derive arm.
#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
#[derive(Clone, Copy)]
struct DerivedDense {
    energy: u32,
}

/// A plain table component (the 0%-gate control).
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct DerivedPlain {
    x: u32,
}

// The derived consts are compile-time facts, so assert them at compile time
// (stronger than a runtime `assert!`, and avoids `clippy::assertions_on_constants`).
//
// * the dense derive arm sets `STORAGE_IS_DENSE = true`;
// * a plain derive keeps the `STORAGE_IS_DENSE = false` default (the 0%-gate);
// * a plain derive keeps `STORAGE_IS_BITSET = false` (the dense and bitset arms
//   are mutually exclusive — `storage` may be set at most once).
const _: () = assert!(<DerivedDense as Component>::STORAGE_IS_DENSE);
const _: () = assert!(!<DerivedPlain as Component>::STORAGE_IS_DENSE);
const _: () = assert!(!<DerivedPlain as Component>::STORAGE_IS_BITSET);

#[test]
fn derive_storage_dense_classifies_and_excludes_from_signature() {
    // Force the `component_id()` OnceLock init closure to run — this is where the
    // derive's `install_dense_storage_kind::<Self>` classification lands.
    let dense_id = DerivedDense::component_id();
    let plain_id = DerivedPlain::component_id();

    assert_eq!(
        component_registry::storage_kind(dense_id.0),
        StorageKind::Dense,
        "the derive must classify the minted id as StorageKind::Dense"
    );
    assert_eq!(
        component_registry::storage_kind(plain_id.0),
        StorageKind::Table,
        "the plain derive's id stays at the Table default"
    );

    // When listed alongside a table id, the derived dense id is filtered out of
    // the signature, but the table id remains — so the two archetypes coincide.
    let mut ecs = EcsMaster::new();
    let table_only = ecs.create_archetype(&[plain_id]);
    let table_plus_dense = ecs.create_archetype(&[plain_id, dense_id]);
    assert_eq!(
        table_only, table_plus_dense,
        "a derived dense id must not fragment the archetype space"
    );
}
