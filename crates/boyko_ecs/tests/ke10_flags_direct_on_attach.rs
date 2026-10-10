//! **KE10 — `FLAGS_DIRECT`: per-component initial enable-bit states, applied on
//! attach.** The enable-bit twin of `REQUIRES_DIRECT`.
//!
//! # What this pins
//!
//! A component may declare, at registration time, the state each of a set of
//! enable tags should be in the moment that component is attached to an entity.
//! The table is
//! `crates/boyko_ecs/src/ecs/core/component/component_registry/flags.rs`
//! (`FLAGS_DIRECT`, `flags_direct_for`, `declares_flags`); the gate is
//! `ArchetypeFlags::FLAGS_ON_ATTACH`, OR-computed at archetype mint by
//! `ArchetypeFlags::insert_from_flag_declarations`; the application is
//! `EcsMaster::apply_attach_flags_all` / `apply_attach_flags_for`.
//!
//! The gate deliberately does NOT ride `insert_from_hooks`: both construction
//! walks reach that call only past an `is_signature_storage` screen, and section
//! 4 below is the red-first pin that a poolless (dense / bitset) declarer must
//! raise the bit all the same.
//!
//! # Two deliberate narrowings from the `REQUIRES_DIRECT` precedent
//!
//! * **No transitive closure.** There is no `FLAGS_ALL`, no memoized DFS, no
//!   `BUILDING` cycle stack. A required component pulls its own requires; a flag
//!   pulls nothing — it is one bit at one `(archetype, row)` address, and its id
//!   is `StorageKind::Bitset`, which carries no attach behaviour to recurse into.
//!   `flags_declared_by_a_flag_are_not_followed` pins that the narrowing is real
//!   rather than merely undocumented.
//! * **No ctor.** `RequiredCtor` exists to MATERIALIZE bytes into an uninit pool
//!   slot. A flag has no bytes, so the entry carries a plain `bool` and the whole
//!   module contains no `unsafe`.
//!
//! # The attach funnels covered, named
//!
//! Each has its own test, because they are different code with different
//! newly-attached sets — the KE11 measurement in this same rung is the standing
//! reminder that "the spawn path covers it" is not evidence about the insert
//! path:
//!
//! | funnel | test |
//! |---|---|
//! | `EcsMaster::create_entity` (direct API; `spawn_one` routes here) | `direct_spawn_applies_the_initial_state` |
//! | `SpawnAtCommand::apply` (deferred `Commands::spawn`) | `deferred_spawn_applies_the_initial_state` |
//! | `SpawnBatchCommand::apply` (`Commands::spawn_batch`) | `batch_spawn_applies_the_initial_state_to_every_row` |
//! | `migrate_entity_insert` (insert into an existing entity) | `insert_migration_applies_the_initial_state` |
//!
//! **NOT covered, and deliberately so:** clone and load. Neither carries enable
//! bits at all in v1 — `clone/mod.rs` states "a v1 clone does **not** carry the
//! enable/disable bit" and `boyko_serialize/src/load.rs` W1 SKIPS a bitset column
//! "matching the clone path". Enable-bit propagation through those two paths is
//! their own documented v1.1 follow-up; KE10 does not change either, so neither
//! silently gains nor silently loses behaviour here.
//!
//! # Registration route
//!
//! The author-facing producer is the Aether `flags (…)` group — rung R3, with
//! its value vocabulary still on owner ballot AB-13. These tests therefore drive
//! the table through `try_set_flags_direct`, the runtime registration twin (the
//! same shape `try_set_hooks` has for the `HOOKS` table). Without it the kernel
//! half would be unreachable and so untestable, which is the "a gate that cannot
//! fail" shape one level down.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    FlagDirectEntry, declares_flags, flags_direct_for, try_set_flags_direct,
};
use boyko_ecs::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};

// ════════════════════════════════════════════════════════════════════════════
// Fixture
// ════════════════════════════════════════════════════════════════════════════

/// The flag a declaring component wants SET on attach.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KVisible780;

/// The flag a declaring component wants explicitly CLEAR on attach. `off` is
/// not a no-op: it must clear a bit another component's `on` entry set.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KStunned781;

/// The declaring component. Its `flags (…)` group is registered by
/// `install_declarations` below.
#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct KHealth782 {
    hp: u32,
}

/// A second declaring component whose `on` entry targets the SAME flag
/// `KHealth782` clears — the ordering fixture.
#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct KCurse783 {
    v: u32,
}

/// Declares NOTHING. The zero-when-unused floor is written against this.
#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct KPlain784 {
    v: u32,
}

/// The anchor an insert migrates away from.
#[derive(Component, Clone, Copy, Default)]
#[repr(C)]
struct KAnchor785 {
    v: u32,
}

/// The flag the POOLLESS declarers below target. Kept disjoint from
/// `KVisible780` / `KStunned781` so section 4 can neither perturb nor be
/// perturbed by the ordering fixture above.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KWarded786;

/// A **dense** declarer — one of the two poolless storage kinds. Dense ids are
/// filtered out of the archetype signature and never gain a per-archetype
/// `ComponentPool`; their data lives in the global `DenseStore`.
#[derive(Component, Clone, Copy, Default)]
#[component(storage = "dense")]
#[repr(C)]
struct KDenseDecl787 {
    v: u32,
}

/// A **bitset** declarer — the other poolless kind. A ZST that is likewise
/// filtered out of the signature and owns no pool.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KBitsetDecl788;

#[derive(Bundle, Clone, Copy)]
struct BDense {
    a: KAnchor785,
    d: KDenseDecl787,
}

#[derive(Bundle, Clone, Copy)]
struct BHealth {
    h: KHealth782,
}

#[derive(Bundle, Clone, Copy)]
struct BPlain {
    p: KPlain784,
}

/// Registers `KHealth782`'s and `KCurse783`'s flag declarations exactly once per
/// process. `try_set_flags_direct` is write-once per id, so a repeat is a
/// no-op — every test may call this.
///
/// The resolvers are passed as fn ITEMS without parentheses, which is the whole
/// point of `FlagIdFn`: registration must not resolve a flag's id inside the
/// declaring component's own `component_id()` `OnceLock` init.
fn install_declarations() {
    // Mint KCurse783 FIRST so it takes the lower id. Signature order is
    // canonical (ascending `ComponentId`), so the lower id is applied first —
    // which puts `KHealth782`'s `off` for `KStunned781` LAST, the arrangement
    // `an_off_entry_clears_a_bit_another_declaration_set` needs to observe an
    // `off` entry actually writing. Measured: with the mint order reversed the
    // `off` ran first and the flag ended up SET, which is the same mechanism
    // read the other way round.
    let curse_id = KCurse783::component_id();
    let health_id = KHealth782::component_id();
    try_set_flags_direct(
        health_id.0,
        &[
            FlagDirectEntry {
                id_fn: KVisible780::component_id,
                initial: true,
            },
            FlagDirectEntry {
                id_fn: KStunned781::component_id,
                initial: false,
            },
        ],
    );
    try_set_flags_direct(
        curse_id.0,
        &[FlagDirectEntry {
            id_fn: KStunned781::component_id,
            initial: true,
        }],
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 1 — the registry itself
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn the_table_records_the_declaration_and_resolves_it_lazily() {
    install_declarations();
    let entries = flags_direct_for(KHealth782::component_id().0);
    assert_eq!(entries.len(), 2, "both declared entries are recorded, in order");
    assert_eq!(
        (entries[0].id_fn)(),
        KVisible780::component_id(),
        "the id resolver resolves to the declared flag when finally called"
    );
    assert!(entries[0].initial, "the first entry declared `on`");
    assert!(!entries[1].initial, "the second entry declared `off`");
    assert!(declares_flags(KHealth782::component_id().0));
}

/// Zero when unused, half 1: a component that declares nothing has an empty
/// slice and does not raise the predicate.
#[test]
fn a_component_with_no_declaration_reads_empty() {
    assert!(
        flags_direct_for(KPlain784::component_id().0).is_empty(),
        "no declaration ⇒ an empty slice, never a panic and never a default"
    );
    assert!(!declares_flags(KPlain784::component_id().0));
    // A `const` block: the claim is compile-time, and stating it at runtime is
    // what `clippy::assertions_on_constants` (a `-D warnings` lint here) rejects.
    // Zero-when-unused: the trait const stays false without a flags group, so
    // the derive-emitted `install_flags::<Self>` call const-folds away.
    const { assert!(!KPlain784::HAS_FLAGS) };
}

/// Zero when unused, half 2 — the load-bearing one. The gate bit is what makes
/// every attach path in a flag-free world cost one `u16` test, so an archetype
/// of non-declaring components must NOT raise it.
#[test]
fn the_archetype_gate_bit_is_raised_only_by_a_declaring_component() {
    install_declarations();
    let mut ecs = EcsMaster::new();

    let plain = ecs.create_archetype(&[KPlain784::component_id()]);
    let flags = ecs
        .archetype_master()
        .get_archetype(plain)
        .expect("archetype")
        .flags();
    assert!(
        !flags.contains(ArchetypeFlags::FLAGS_ON_ATTACH),
        "an archetype of non-declaring components must not raise FLAGS_ON_ATTACH \
         — this bit is the 0%-gate"
    );

    let declaring = ecs.create_archetype(&[KHealth782::component_id()]);
    let flags = ecs
        .archetype_master()
        .get_archetype(declaring)
        .expect("archetype")
        .flags();
    assert!(
        flags.contains(ArchetypeFlags::FLAGS_ON_ATTACH),
        "an archetype containing a declaring component must raise FLAGS_ON_ATTACH"
    );
}

/// The **no-transitive-closure** narrowing, made falsifiable. A flag is
/// `StorageKind::Bitset`; if a future edit ever grew a closure walk, a
/// declaration hung off the FLAG would start being followed. It must not be.
#[test]
fn flags_declared_by_a_flag_are_not_followed() {
    install_declarations();
    // Hang a declaration off the FLAG itself. Nothing may follow it: attaching
    // KHealth782 sets KVisible780's bit, and that is the end of the walk.
    try_set_flags_direct(
        KVisible780::component_id().0,
        &[FlagDirectEntry {
            id_fn: KStunned781::component_id,
            initial: true,
        }],
    );

    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KHealth782::component_id()]);
    let e = ecs.spawn_one(arch, KHealth782 { hp: 10 }).expect("spawn");

    assert!(ecs.is_enabled::<KVisible780>(e), "the direct entry applied");
    assert!(
        !ecs.is_enabled::<KStunned781>(e),
        "NO transitive closure: a declaration hanging off a FLAG is never \
         followed. KHealth782's own `off` entry for KStunned781 is what decides \
         this bit, and the flag's declaration contributes nothing"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 2 — the four attach funnels, one test each
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn direct_spawn_applies_the_initial_state() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KHealth782::component_id()]);
    let e = ecs.spawn_one(arch, KHealth782 { hp: 1 }).expect("spawn");

    assert!(
        ecs.is_enabled::<KVisible780>(e),
        "EcsMaster::create_entity must apply the declared `on` state"
    );
    assert!(
        !ecs.is_enabled::<KStunned781>(e),
        "and leave the declared `off` state clear"
    );
}

#[test]
fn deferred_spawn_applies_the_initial_state() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    let e: Entity =
        ecs.run_system(|mut cmds: Commands| cmds.spawn(BHealth { h: KHealth782 { hp: 1 } }).id());

    assert!(
        ecs.is_enabled::<KVisible780>(e),
        "SpawnAtCommand::apply (deferred Commands::spawn) must apply it too — \
         a different code path from the direct API"
    );
}

#[test]
fn batch_spawn_applies_the_initial_state_to_every_row() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    // `EcsMaster::spawn_batch` builds a `SpawnBatchCommand` and applies it
    // inline, so this is the `SpawnBatchCommand::apply` path itself.
    let entities: Vec<Entity> = ecs
        .spawn_batch((0..4u32).map(|i| BHealth {
            h: KHealth782 { hp: i },
        }))
        .expect("batch spawn");
    assert_eq!(entities.len(), 4, "the batch spawned four rows");
    for (i, &e) in entities.iter().enumerate() {
        assert!(
            ecs.is_enabled::<KVisible780>(e),
            "SpawnBatchCommand::apply must apply the initial state to row {i}. \
             This path fires NO on_add/on_insert hooks by design, but a flag's \
             initial state is what the component ARRIVES with, not a \
             notification — skipping it would make spawn_batch produce entities \
             in a different state than spawn"
        );
    }
}

#[test]
fn insert_migration_applies_the_initial_state() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KAnchor785::component_id()]);
    let e = ecs
        .spawn_one(arch, KAnchor785 { v: 1 })
        .expect("spawn the anchor");
    assert!(
        !ecs.is_enabled::<KVisible780>(e),
        "precondition: the anchor-only entity carries no flag yet"
    );

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(BHealth { h: KHealth782 { hp: 5 } });
    });

    assert!(
        ecs.is_enabled::<KVisible780>(e),
        "migrate_entity_insert must apply the initial state of a NEWLY-attached \
         component. This is a different call from the spawn path's, and the \
         spawn path cannot stand in for it (see the KE11 measurement)"
    );
}

/// The counterpart of the test above, and the reason the insert path uses the
/// newly-attached set rather than the whole target signature: a RETAINED
/// component's initial state must NOT be re-applied over a bit the game has
/// since toggled.
#[test]
fn an_insert_does_not_reapply_a_retained_components_initial_state() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KHealth782::component_id()]);
    let e = ecs.spawn_one(arch, KHealth782 { hp: 5 }).expect("spawn");
    assert!(ecs.is_enabled::<KVisible780>(e), "attached ⇒ on");

    // The game turns it off.
    ecs.disable::<KVisible780>(e);
    assert!(!ecs.is_enabled::<KVisible780>(e));

    // Insert an UNRELATED component. KHealth782 is retained, not re-attached.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(BPlain { p: KPlain784 { v: 9 } });
    });

    assert!(
        !ecs.is_enabled::<KVisible780>(e),
        "a retained component's initial state must NOT be re-applied by an \
         unrelated insert — that would silently undo a game-state toggle on \
         every archetype change"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 3 — `off` is an action, not an omission
// ════════════════════════════════════════════════════════════════════════════

/// An `off` entry must WRITE a clear, not be skipped as "the default is already
/// clear".
///
/// Written so the claim does not depend on the order two declarations are
/// applied in. A first draft did depend on it — two declaring components in one
/// archetype disagreeing about `KStunned781` — and it failed twice while the
/// mechanism was correct both times: the archetype's `component_ids` is not
/// canonically re-sorted, so the application order followed the order the ids
/// were handed to `create_archetype`, not their numeric order. Conflict
/// arbitration between two declarations is R3's grammar to define (and to decide
/// whether it is even expressible); this test deliberately asserts nothing about
/// it.
///
/// Instead the bit is set by the GAME first, and the declaring component is
/// attached afterwards. There is exactly one declaration in play, so the only
/// question left is whether `off` writes.
#[test]
fn an_off_entry_clears_an_already_set_bit() {
    install_declarations();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KAnchor785::component_id()]);
    let e = ecs
        .spawn_one(arch, KAnchor785 { v: 1 })
        .expect("spawn the anchor");

    ecs.enable::<KStunned781>(e);
    assert!(
        ecs.is_enabled::<KStunned781>(e),
        "precondition: the bit is SET before the declaring component attaches"
    );

    // KHealth782 declares `KStunned781 = off`.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(BHealth { h: KHealth782 { hp: 5 } });
    });

    assert!(ecs.is_enabled::<KVisible780>(e), "the same component's `on` applied");
    assert!(
        !ecs.is_enabled::<KStunned781>(e),
        "an `off` entry is a WRITE, not an omission: it must clear a bit that was \
         already set. A `flags (X = off)` implemented as 'skip — the default is \
         clear' leaves this true"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 4 — the gate bit over the POOLLESS storage kinds (dense, bitset)
// ════════════════════════════════════════════════════════════════════════════
//
// The campaign's signature defect class — *a mechanism that resolves
// per-archetype pools is blind to storage kinds that own no pool* — reaching
// KE10's own gate. `FLAGS_ON_ATTACH` used to be raised inside
// `ArchetypeFlags::insert_from_hooks`, and BOTH archetype-construction walks
// (`Archetype::create_by_ids` and `Archetype::register_component_inplace`, the
// live mint funnel) reach that call only PAST an `is_signature_storage` screen
// that `continue`s / `return`s for every dense and bitset id. So a poolless
// declarer never raised its own archetype's gate, and every attach funnel
// short-circuits on that bit — a silent, plausible, order-of-composition
// dependent wrong answer, since the same declaration DOES apply the moment a
// table sibling in the same archetype happens to raise the bit.
//
// The walk itself was never blind: `Archetype::component_ids` retains every id,
// poolless ones included, so `apply_attach_flags_all` sees them. Only the gate
// was wrong, which is why the fix is one call moved ABOVE the storage screen
// (`insert_from_flag_declarations`) rather than a change to the application.
//
// Section 1's `the_archetype_gate_bit_is_raised_only_by_a_declaring_component`
// structurally cannot see this: both of its declarers (`KHealth782`,
// `KCurse783`) are table components.

/// Registers the poolless declarations. Write-once per id like
/// `install_declarations`, so repeated calls are no-ops.
fn install_poolless_declarations() {
    try_set_flags_direct(
        KDenseDecl787::component_id().0,
        &[FlagDirectEntry {
            id_fn: KWarded786::component_id,
            initial: true,
        }],
    );
    try_set_flags_direct(
        KBitsetDecl788::component_id().0,
        &[FlagDirectEntry {
            id_fn: KWarded786::component_id,
            initial: true,
        }],
    );
}

/// The gate bit, at the live mint funnel (`EcsMaster::create_archetype` →
/// `ArchetypeBundle::add_archetype_from_components_fallible` →
/// `register_component_inplace`), for a **dense** declarer with no table
/// declarer beside it.
#[test]
fn a_dense_declarer_raises_the_archetype_gate_bit() {
    install_poolless_declarations();
    let mut ecs = EcsMaster::new();
    // Mint the poolless-carrying set FIRST: a dense id is filtered out of the
    // signature, so `[anchor, dense]` and `[anchor]` share a signature and the
    // second call would dedup onto the first archetype's `component_ids`.
    let arch = ecs.create_archetype(&[KAnchor785::component_id(), KDenseDecl787::component_id()]);
    let flags = ecs
        .archetype_master()
        .get_archetype(arch)
        .expect("archetype")
        .flags();
    assert!(
        flags.contains(ArchetypeFlags::FLAGS_ON_ATTACH),
        "a DENSE component that declares `flags (…)` must raise FLAGS_ON_ATTACH \
         on its own archetype. A clear bit here is the storage-kind blindness: \
         the gate is computed past the `is_signature_storage` screen, so a \
         poolless declarer is skipped and every attach funnel short-circuits"
    );
}

/// The same, for the other poolless kind. `create_archetype` accepts a bitset id
/// in its slice (it is filtered from the signature and kept in `component_ids`
/// — the shape `create_archetype_dedups_table_only_against_table_plus_bitset_tag`
/// already pins), so the gate must see it there too.
#[test]
fn a_bitset_declarer_raises_the_archetype_gate_bit() {
    install_poolless_declarations();
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[KAnchor785::component_id(), KBitsetDecl788::component_id()]);
    let flags = ecs
        .archetype_master()
        .get_archetype(arch)
        .expect("archetype")
        .flags();
    assert!(
        flags.contains(ArchetypeFlags::FLAGS_ON_ATTACH),
        "a BITSET component that declares `flags (…)` must raise FLAGS_ON_ATTACH \
         too — the screen the gate sat behind rejects both poolless kinds, so a \
         fix parameterised over dense alone would leave this one red"
    );
}

/// End-to-end through the deferred spawn funnel: the bit being raised is only
/// interesting because the initial state then actually lands.
#[test]
fn a_dense_declarer_applies_its_initial_state_on_spawn() {
    install_poolless_declarations();
    let mut ecs = EcsMaster::new();
    let e: Entity = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(BDense {
            a: KAnchor785 { v: 0 },
            d: KDenseDecl787 { v: 1 },
        })
        .id()
    });
    assert!(
        ecs.is_enabled::<KWarded786>(e),
        "a DENSE component's declared `on` state must be applied on attach, with \
         no table declarer in the archetype to raise the gate for it"
    );
}

/// The floor for the repair: raising the gate from a poolless id must not raise
/// it for a poolless id that declares NOTHING. Without this the "fix" could be
/// "raise the bit for every non-signature id", which would hand every dense
/// archetype in a flag-free world the per-attach walk the bit exists to avoid.
#[test]
fn a_poolless_non_declarer_leaves_the_gate_bit_clear() {
    install_poolless_declarations();
    let mut ecs = EcsMaster::new();
    // `KVisible780` is a bitset id that declares nothing of its own in this
    // section (section 1's `flags_declared_by_a_flag_are_not_followed` may hang
    // one off it, so use `KStunned781`, which no test ever declares against).
    let arch = ecs.create_archetype(&[KPlain784::component_id(), KStunned781::component_id()]);
    let flags = ecs
        .archetype_master()
        .get_archetype(arch)
        .expect("archetype")
        .flags();
    assert!(
        !flags.contains(ArchetypeFlags::FLAGS_ON_ATTACH),
        "a poolless id that declares nothing must leave the 0%-gate clear"
    );
}
