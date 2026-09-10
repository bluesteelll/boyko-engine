//! KE14 D1 — the `ON_*_OBSERVER` archetype bits answer DECLARATION membership,
//! gated in RELEASE SEMANTICS through the public `Archetype::flags()` accessor.
//!
//! # What this file gates, and what it deliberately does NOT claim
//!
//! `ArchetypeMaster::debug_assert_observer_flags_consistent` is a
//! `#[cfg(debug_assertions)]` tripwire that compares each archetype's
//! `ON_{kind}_OBSERVER` bit against a recompute. It is a good gate — it caught
//! the KE13/KE14 lane's first attempt at moving these sites to
//! `table_component_ids()`, eight tests across `dense_d2_reentrancy` and
//! `dense_d2_routing`. But it compiles to NOTHING in release, so an invariant
//! whose only witness is that tripwire is unwitnessed in every shipping build.
//! Every assertion below is the TEST's own and survives `--release`.
//!
//! **A claim this file does not make.** The KE14 D1 brief supposed a worse,
//! silent second face: that reading `table_component_ids()` in
//! `remove_observer`'s sibling recompute would clear the bit while a dense
//! observer was still registered, and dense observers would then stop firing.
//! That face was checked against every fire site and **does not exist**. Each
//! flag-gated fire block iterates a TABLE-ONLY id set —
//! `spawn_at_command`/`materialize` read `table_component_ids` directly,
//! `migration_helpers`' `bundle_ids` is filled only on the non-dense arm of
//! `for_each_component_bytes`, `insert_command` `continue`s on `is_dense(cid)`
//! — and every DENSE fire rides a separate block documented "NOT gated by
//! archetype flags" (`dense_insert_and_fire`, `dense_remove_and_fire`,
//! `dense_despawn_fire_and_tombstone`, the `dense_fire_buf` drains, and
//! `materialize_dense_memberships`). No dense observer consults the bit, so no
//! list choice can silence one. The two `assert_eq!`s on fire counts below are
//! kept as POSITIVE pins on that reachability, not as the regression gate.
//!
//! # The invariant that is gated
//!
//! Seed and consumers must read the SAME set, and that set is the declaration
//! record. The two mint seeds OR `ArchetypeFlags::insert_from_observers` over
//! the full id list — `create_archetype`'s caller slice and
//! `add_existing_archetype`'s `all_component_ids()` — and that OR sits above any
//! `is_signature_storage` screen. (The screen belongs to `insert_from_hooks`,
//! which seeds the unrelated `ON_*_HOOK` bits; the pre-fix comment on the
//! tripwire cited it, which is how a table-list recompute came to look like an
//! oracle for the observer bits.) The two maintenance walks must therefore also
//! read the declaration record, and must test membership with
//! `Archetype::declares_component_id`, not `has_component_id` — the latter is a
//! signature-mask test, and `filtered_signature_mask` drops every `Dense` id by
//! construction, so no dense `cid` can ever satisfy it.
//!
//! | test | site | pre-fix shape | observable effect |
//! |---|---|---|---|
//! | `dense_declarer_keeps_the_observer_bit_when_a_table_sibling_observer_is_removed` | `remove_observer`'s `any_sibling` recompute | `table_component_ids()` | bit CLEARED though a declared component still observes |
//! | `registering_an_observer_on_a_dense_component_raises_the_bit_on_an_existing_archetype` | `add_observer`'s add-first walk | `has_component_id` | bit never RAISED on an archetype minted first |
//!
//! Both are divergences from the state the two seeds produce, i.e. exactly the
//! inconsistency the tripwire fires on — restated as a release-live assertion.
//! Neither is reachable from `dense_d2_routing`, whose helper registers every
//! observer BEFORE the first spawn and never removes one.
//!
//! # Why the DECLARATION direction and not the TABLE one
//!
//! Because both gated loops and both seeds already agree on it, and because it
//! is the conservative side of the only asymmetry that exists. The bit is a
//! GATE: an extra raise costs one pass of `fire_*_observers` over the table ids,
//! each an early-out on an empty registry list, and can never produce a wrong
//! fire (dispatch is keyed by `cid`). A missing raise loses fires outright. The
//! table list is the tighter answer for today's fire-site shapes; the
//! declaration list stays correct if any future gated loop carries a dense id —
//! which is the KE10 regression verbatim, in this same file, where a
//! declaration-wide gate raised only for pool-owning ids left a dense declarer's
//! `flags (…)` group silently unapplied.

// Test oracle model: the `Arc<Mutex<_>>` here is a cross-thread observation
// channel for the spawned entity handle, not engine data.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// 16-byte POD dense payload, mirroring `dense_d2_routing`'s `DPos`.
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct DPos {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

const P: DPos = DPos { x: 1.0, y: 2.0, z: 3.0, w: 4.0 };

/// Spawns `make()` through `Commands` and returns the live handle.
fn spawn<B: boyko_ecs::ecs::core::bundle::Bundle + Send + Sync>(
    ecs: &mut EcsMaster,
    make: impl Fn() -> B + Send + Sync + 'static,
) -> Entity {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        probe.lock().expect("lock").push(cmds.spawn(make()).id());
    });
    sink.lock().expect("lock")[0]
}

/// Reads `entity`'s current archetype's flag word through the public accessor —
/// the same read `ke10_flags_direct_on_attach` uses, and the reason `flags()`
/// exists: a gate nothing can observe is a gate nothing can falsify.
fn archetype_flags_of(ecs: &EcsMaster, entity: Entity) -> ArchetypeFlags {
    let id = ecs.entity_archetype_id(entity).expect("entity is live");
    ecs.archetype_master()
        .get_archetype(id)
        .expect("archetype exists")
        .flags()
}

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — `remove_observer`'s sibling recompute must SEE the dense declarer.
//
// One archetype declaring a table component and a dense component, each with an
// `on_insert` observer. Removing the TABLE one is the LAST observer of its
// `(Insert, cid)` pair, so the remove-last recompute walk runs.
//
// Pre-fix, `any_sibling` scanned `table_component_ids()` — a list holding only
// `A1Tag`, whose observer had just been removed — concluded "nothing here
// observes Insert", and CLEARED `ON_INSERT_OBSERVER` on an archetype one of
// whose DECLARED components still has an Insert observer. That is precisely the
// state the mint seed would never produce, and the assertion below reads it
// directly, in release as in debug.
// ════════════════════════════════════════════════════════════════════════════

static A1_TABLE_INSERT: AtomicUsize = AtomicUsize::new(0);
static A1_DENSE_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn a1_table_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    A1_TABLE_INSERT.fetch_add(1, SEQ);
}
unsafe fn a1_dense_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    A1_DENSE_INSERT.fetch_add(1, SEQ);
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A1Tag(u32);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct A1Dense(DPos);

#[derive(Bundle)]
struct A1Bundle {
    t: A1Tag,
    d: A1Dense,
}

#[test]
fn dense_declarer_keeps_the_observer_bit_when_a_table_sibling_observer_is_removed() {
    let mut ecs = EcsMaster::new();

    let table_obs = ecs.observe_on_insert::<A1Tag>(a1_table_insert);
    let _dense_obs = ecs.observe_on_insert::<A1Dense>(a1_dense_insert);

    // First spawn mints the archetype, so the OBS-SEED walk over the declaration
    // record raises `ON_INSERT_OBSERVER`. This is the state the recompute below
    // must not contradict.
    let e0 = spawn(&mut ecs, || A1Bundle { t: A1Tag(1), d: A1Dense(P) });
    assert!(
        archetype_flags_of(&ecs, e0).contains(ArchetypeFlags::ON_INSERT_OBSERVER),
        "the mint seed raises ON_INSERT_OBSERVER over the declaration record"
    );
    assert_eq!(A1_TABLE_INSERT.load(SEQ), 1, "table on_insert fires on the seeding spawn");
    assert_eq!(A1_DENSE_INSERT.load(SEQ), 1, "dense on_insert fires on the seeding spawn");

    // Remove the LAST observer of `(Insert, A1Tag)` → the recompute walk runs
    // over every archetype declaring `A1Tag`, including this one.
    assert!(ecs.remove_observer(table_obs), "the table observer was registered");

    assert!(
        archetype_flags_of(&ecs, e0).contains(ArchetypeFlags::ON_INSERT_OBSERVER),
        "KE14 D1: `remove_observer`'s sibling recompute must read the DECLARATION \
         record (`all_component_ids`), where the still-observed dense id lives — \
         not `table_component_ids`, which excludes every dense id by construction \
         and so clears a bit the mint seed would have raised"
    );

    // Positive reachability pin (NOT the gate): the dense observer keeps firing.
    // It would keep firing under either list, because every dense fire path is
    // flag-free — see this file's header.
    A1_TABLE_INSERT.store(0, SEQ);
    A1_DENSE_INSERT.store(0, SEQ);
    let _e1 = spawn(&mut ecs, || A1Bundle { t: A1Tag(2), d: A1Dense(P) });
    assert_eq!(A1_DENSE_INSERT.load(SEQ), 1, "the dense observer still fires");
    assert_eq!(A1_TABLE_INSERT.load(SEQ), 0, "the removed table observer does not fire");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — `add_observer`'s add-first walk must SEE the dense declarer.
//
// Mint the archetype FIRST, so no seed can help, then register the dense
// observer. Pre-fix, the walk's membership test was `has_component_id`, which
// reads the signature mask — and `filtered_signature_mask` drops every `Dense`
// id — so the walk matched NO archetype and the bit was never raised, leaving
// the world in a state the seed can never produce.
// ════════════════════════════════════════════════════════════════════════════

static A2_DENSE_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn a2_dense_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    A2_DENSE_INSERT.fetch_add(1, SEQ);
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct A2Tag(u32);

#[derive(Component)]
#[component(storage = "dense")]
#[repr(C)]
struct A2Dense(DPos);

#[derive(Bundle)]
struct A2Bundle {
    t: A2Tag,
    d: A2Dense,
}

#[test]
fn registering_an_observer_on_a_dense_component_raises_the_bit_on_an_existing_archetype() {
    let mut ecs = EcsMaster::new();

    // Mint the archetype BEFORE any observer exists: the OBS-SEED cannot set the
    // bit, so only `add_observer`'s walk can.
    let e0 = spawn(&mut ecs, || A2Bundle { t: A2Tag(1), d: A2Dense(P) });
    assert!(
        !archetype_flags_of(&ecs, e0).contains(ArchetypeFlags::ON_INSERT_OBSERVER),
        "no observer is registered yet, so the bit must be clear — without this \
         the next assertion could not distinguish a raise from a stuck bit"
    );

    A2_DENSE_INSERT.store(0, SEQ);
    let _dense_obs = ecs.observe_on_insert::<A2Dense>(a2_dense_insert);

    assert!(
        archetype_flags_of(&ecs, e0).contains(ArchetypeFlags::ON_INSERT_OBSERVER),
        "KE14 D1: `add_observer`'s add-first walk must test DECLARATION membership \
         (`declares_component_id`), not signature membership (`has_component_id`), \
         which no dense id can ever satisfy — otherwise an archetype minted before \
         the observer keeps a bit contradicting the one a fresh mint would get"
    );

    // Positive reachability pin (NOT the gate), as in Test 1.
    let _e1 = spawn(&mut ecs, || A2Bundle { t: A2Tag(2), d: A2Dense(P) });
    assert_eq!(A2_DENSE_INSERT.load(SEQ), 1, "the dense observer fires on the next spawn");
}
