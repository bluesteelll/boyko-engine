//! **KE11 — `#[require]` of an id whose storage kind owns NO per-archetype pool.**
//! MEASUREMENT, not a fix: the disposition is owner ballot **AB-6**.
//!
//! # The class
//!
//! `StorageKind::Dense` and `StorageKind::Bitset` are both **signature-excluded**
//! — `ArchetypeMaster` filters them out of every archetype signature via
//! `is_signature_storage`, so neither owns a per-archetype `ComponentPool`. Two
//! sites on the required-components path resolve a pool for a required id
//! **without screening the storage kind first**, and consume the resulting
//! `None` through an `.expect` whose message blames the archetype-expansion
//! contract:
//!
//! | # | site | the call |
//! |---|---|---|
//! | 1 | `BundleColumnCache::resolve_required_missing` (`bundle/bundle_column_cache.rs`) | `archetype.component_pools().pool_id_for(entry.component_id).expect("invariant: the archetype was expanded with every required id …")` |
//! | 2 | the `has_requires` required-ctor pass inside `migrate_entity_insert` (`commands/migration_helpers.rs`) | `tgt!().component_pools_mut().get_pool_mut(req_id).expect("invariant: target hosts every required id (expanded archetype)")` |
//!
//! Their sibling `BundleColumnCache::resolve_and_cache` **does** screen — it
//! diverts a `StorageKind::Dense` id to `DENSE_POOL_SENTINEL` before resolving.
//! Neither of the two above has that screen.
//!
//! # What was MEASURED here (2026-08-30, rung R1)
//!
//! All four combinations panic, and each lands where the code reading predicted.
//! Captured from the run of this file, `--nocapture`:
//!
//! ```text
//! site1 … bitset_flag … panicked at crates\boyko_ecs\src\ecs\core\bundle\bundle_column_cache.rs:410:22
//! site1 … dense       … panicked at crates\boyko_ecs\src\ecs\core\bundle\bundle_column_cache.rs:410:22
//! site2 … bitset_flag … panicked at crates\boyko_ecs\src\ecs\core\commands\migration_helpers.rs:728:22
//! site2 … dense       … panicked at crates\boyko_ecs\src\ecs\core\commands\migration_helpers.rs:728:22
//! ```
//!
//! **Site 2 had never been confirmed before this run.** R0's census measured
//! site 1 for both kinds with a throwaway spawn probe and recorded site 2 as
//! "STILL unconfirmed", warning that a spawn-only suite would pin site 1 twice
//! and report site 2 as covered. It is now confirmed, for both poolless kinds,
//! through the insert path.
//!
//! The two `.expect` messages are distinct enough to discriminate the sites on
//! their own ("the archetype was expanded with every required id" vs "target
//! hosts every required id"), which is why each `should_panic` pins the message
//! of ITS site rather than a shared substring.
//!
//! # Why the two sites need SEPARATE tests
//!
//! They are **two different calls on two different paths**, and the spawn path
//! never reaches site 2 — it dies at site 1 first. Site 1 is reached only from
//! `SpawnAtCommand::apply` / `SpawnBatchCommand::apply` (the only two callers of
//! `resolve_and_cache`); site 2 is reached only by an **insert into an existing
//! entity**, which migrates through `migrate_entity_insert` and never consults
//! the `BundleColumnCache`. A suite that only spawns pins site 1 twice and
//! silently reports site 2 as covered.
//!
//! # Why both poolless KINDS
//!
//! The backlog row originally said "a bitset tag", which is half the class:
//! `Dense` is signature-excluded on the same predicate and reaches the same two
//! unscreened calls. Each site is therefore parameterised over both kinds — four
//! defect tests, plus two table-storage controls.
//!
//! # Disposition — NOT decided here
//!
//! Ballot **AB-6**: (a) parse refusal — which would *narrow* the ratified
//! `storage = table | dense` × `requires` surface; (b) a dense required-ctor
//! (construct-and-commit) route; (c) leave it KNOWN-OPEN with a documented hook
//! workaround. These tests demonstrate the panic under all three: under (a) and
//! (c) the kernel behaviour is unchanged and they keep passing; under (b) they
//! go RED the moment the panic stops, which is exactly the signal a landing fix
//! should have to acknowledge.
//!
//! # The controls are load-bearing
//!
//! `table_required_id_*_does_not_panic` are not decoration. A `should_panic`
//! test passes on ANY panic, including one from a broken fixture, so without a
//! green control that the *same* fixture shape works for a `Table` required id,
//! these four could be pinning a typo.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};

// ════════════════════════════════════════════════════════════════════════════
// Required targets — one per storage kind
// ════════════════════════════════════════════════════════════════════════════

/// DENSE required target: owns a global `DenseStore`, is excluded from every
/// archetype signature, and therefore owns no per-archetype `ComponentPool`.
#[derive(Component, Default, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct KDense770 {
    x: u32,
}

/// BITSET required target: an enable bit. Signature-excluded like the dense
/// target, and it owns no bytes at all.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KFlag771;

/// TABLE required target: the control kind. Owns a per-archetype pool, so both
/// sites resolve it exactly as the `.expect` messages assume.
#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct KTable772 {
    x: u32,
}

// ════════════════════════════════════════════════════════════════════════════
// Requiring components — one per required target
// ════════════════════════════════════════════════════════════════════════════

/// Requires the DENSE target.
#[derive(Component, Default, Clone, Copy)]
#[require(KDense770)]
#[repr(C)]
struct KReqDense773 {
    v: u32,
}

/// Requires the BITSET target.
#[derive(Component, Default, Clone, Copy)]
#[require(KFlag771)]
#[repr(C)]
struct KReqFlag774 {
    v: u32,
}

/// Requires the TABLE target (the control).
#[derive(Component, Default, Clone, Copy)]
#[require(KTable772)]
#[repr(C)]
struct KReqTable775 {
    v: u32,
}

/// Payload the entity already carries, so that an `insert` is a real MIGRATION
/// (`{Anchor}` → `{Anchor, Req, Required}`) and not the in-place replace fast
/// path — the in-place path never reaches site 2.
#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct KAnchor776 {
    v: u32,
}

#[derive(Bundle, Clone, Copy)]
struct BDense {
    a: KReqDense773,
}

#[derive(Bundle, Clone, Copy)]
struct BFlag {
    a: KReqFlag774,
}

#[derive(Bundle, Clone, Copy)]
struct BTable {
    a: KReqTable775,
}

// ════════════════════════════════════════════════════════════════════════════
// Drivers — one per SITE, shared by every kind
// ════════════════════════════════════════════════════════════════════════════

/// Site 1 driver: a deferred `Commands::spawn`, which routes through
/// `SpawnAtCommand::apply` → `BundleColumnCache::resolve_and_cache` →
/// `resolve_required_missing`. The `BundleColumnCache` is consulted on the
/// FIRST spawn of a given `(Bundle, world)` pair, which is what this is.
fn spawn_bundle<B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    world.run_system(move |mut cmds: Commands| cmds.spawn(bundle).id())
}

/// Site 2 driver: spawn an anchored entity, then `insert` the require-bearing
/// bundle into it. The source archetype is `{KAnchor776}` and the target is
/// `{KAnchor776, Req…}` (+ the required id if it is signature storage), so the
/// insert is a genuine migration through `migrate_entity_insert` and reaches the
/// `has_requires` constructor pass. It does NOT consult the `BundleColumnCache`,
/// so site 1 cannot shadow the measurement.
fn insert_bundle_into_anchored<B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) {
    let arch = world.create_archetype(&[KAnchor776::component_id()]);
    let e = world
        .spawn_one(arch, KAnchor776 { v: 7 })
        .expect("spawn the anchor entity");
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(bundle);
    });
}

// ════════════════════════════════════════════════════════════════════════════
// Controls — the same two drivers over a TABLE required id must NOT panic
// ════════════════════════════════════════════════════════════════════════════

/// Control for site 1. If this reds, the four `should_panic` tests below could
/// be pinning a fixture defect rather than the KE11 mechanism.
#[test]
fn table_required_id_on_spawn_does_not_panic() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, BTable { a: KReqTable775 { v: 1 } });
    assert!(
        world.has_component(e, KTable772::component_id()),
        "control: a TABLE required id is constructed on the spawn path"
    );
}

/// Control for site 2 — the same shape through the migration path.
#[test]
fn table_required_id_on_insert_does_not_panic() {
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[KAnchor776::component_id()]);
    let e = world
        .spawn_one(arch, KAnchor776 { v: 7 })
        .expect("spawn the anchor entity");
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(BTable { a: KReqTable775 { v: 2 } });
    });
    assert!(
        world.has_component(e, KTable772::component_id()),
        "control: a TABLE required id is constructed on the INSERT path"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SITE 1 — `BundleColumnCache::resolve_required_missing`, both poolless kinds
// ════════════════════════════════════════════════════════════════════════════

/// `#[require]` of a DENSE component, spawned. Site 1's `.expect` blames the
/// archetype-expansion contract; the actual cause is that a dense id owns no
/// pool to expand into.
#[test]
#[should_panic(expected = "the archetype was expanded with every required id")]
fn site1_spawn_require_dense_panics_with_the_expansion_invariant() {
    let mut world = EcsMaster::new();
    let _ = spawn_bundle(&mut world, BDense { a: KReqDense773 { v: 1 } });
}

/// `#[require]` of a BITSET flag, spawned. Same site, same misleading message —
/// this is the half the backlog row originally named.
#[test]
#[should_panic(expected = "the archetype was expanded with every required id")]
fn site1_spawn_require_bitset_flag_panics_with_the_expansion_invariant() {
    let mut world = EcsMaster::new();
    let _ = spawn_bundle(&mut world, BFlag { a: KReqFlag774 { v: 1 } });
}

// ════════════════════════════════════════════════════════════════════════════
// SITE 2 — the `migrate_entity_insert` required-ctor pass, both poolless kinds
// ════════════════════════════════════════════════════════════════════════════

/// `#[require]` of a DENSE component, INSERTED into an existing entity. This is
/// the site the spawn probe could never reach.
#[test]
#[should_panic(expected = "target hosts every required id")]
fn site2_insert_require_dense_panics_with_the_target_hosts_invariant() {
    let mut world = EcsMaster::new();
    insert_bundle_into_anchored(&mut world, BDense { a: KReqDense773 { v: 1 } });
}

/// `#[require]` of a BITSET flag, INSERTED into an existing entity.
#[test]
#[should_panic(expected = "target hosts every required id")]
fn site2_insert_require_bitset_flag_panics_with_the_target_hosts_invariant() {
    let mut world = EcsMaster::new();
    insert_bundle_into_anchored(&mut world, BFlag { a: KReqFlag774 { v: 1 } });
}
