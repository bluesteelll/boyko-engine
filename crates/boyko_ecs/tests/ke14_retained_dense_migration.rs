//! **KE14 — the retained-dense path in `migrate_entity_insert` and its three
//! siblings.** Five defects, none of them memory-safety (Miri clean), all of
//! them reachable from plain Rust the moment `#[require]` over a dense storage
//! kind became an advertised capability (KE11 / ballot AB-6).
//!
//! Row and full statement: [`docs/aether-v2/KERNEL-BACKLOG.md`](../../../docs/aether-v2/KERNEL-BACKLOG.md)
//! §KE14.
//!
//! # The two mechanisms, stated once
//!
//! **`Archetype::component_ids` is a DECLARATION RECORD, not a membership
//! oracle.** A dense id is filtered out of the archetype *signature* but
//! RETAINED in the mint-time id list (deliberately — KE10's attach-flag walk
//! depends on that retention). Archetype identity keys on the FILTERED
//! signature mask, so two id lists that differ only in dense ids collapse onto
//! ONE archetype and whichever list minted it first decides what the list
//! records. Every loop that resolves a `ComponentPool` out of that list is
//! therefore one dedup away from asking for a pool that cannot exist. That is
//! **D1**, and it has four faces:
//!
//! | face | call site |
//! |---|---|
//! | insert, Step-1 retained copy | `migration_helpers.rs` `invariant: retained component must exist in source` |
//! | remove, retained gather | `invariant: target ⊂ source` |
//! | tag attach, retained copy | `invariant: source hosts its own component id` |
//! | tag detach, retained copy | same message, the detach twin |
//!
//! **`DenseStore::arch_presence` is seeded only by value-WRITING sites.** It is
//! the sole candidate seed a dense query with no table positive term has, and
//! it is documented as having no false negatives. A dense member merely
//! *retained* across a migration leaves the destination archetype unmarked, so
//! the entity keeps the component in the store and stops being enumerated —
//! a silent wrong answer. That is **D2**, and it has the same four faces.
//!
//! # Why the control at the top of this file is load-bearing
//!
//! Every D1 test here asserts *"this does not panic"*. Such a test passes for
//! any reason at all, including a fixture that never created the phantom the
//! defect needs — which is precisely how `ke11_require_poolless_storage_kind`'s
//! own insert driver is blind to it (`insert_bundle_into_anchored` builds a
//! dense-free source archetype). The control asserts the phantom EXISTS in the
//! fixture before any of the four faces is exercised.
//!
//! # The D2 tests need a fresh world, and the reason is not hygiene
//!
//! `arch_presence` is CONSERVATIVE: a bit is set on insert and never cleared.
//! A D2 test whose destination archetype has ever hosted a dense member in the
//! same world is green no matter what the kernel does. Each test below builds
//! its own `EcsMaster::new()` and composes the destination so that no
//! dense-carrying entity ever occupied it. **Do not share a fixture between
//! them** — a "share the world" refactor greens the file silently.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{FlagDirectEntry, try_set_flags_direct};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};
use proptest::prelude::*;

/// The value `K14Dense`'s required-ctor writes. Not zero and not a plausible
/// allocator residue, so "the ctor ran" is distinguishable from "the slot
/// happened to be zeroed".
const DENSE_CTOR_SENTINEL: u32 = 0x00C0_FFEE;

// ════════════════════════════════════════════════════════════════════════════
// Fixture components
// ════════════════════════════════════════════════════════════════════════════

/// The DENSE subject. Signature-excluded, owns no per-archetype pool, lives in
/// the global `DenseStore`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
#[repr(C)]
struct K14Dense {
    x: u32,
}

impl Default for K14Dense {
    fn default() -> Self {
        Self {
            x: DENSE_CTOR_SENTINEL,
        }
    }
}

/// Requires the dense subject. Its presence is what puts the dense id into an
/// archetype's declaration list.
#[derive(Component, Default, Clone, Copy)]
#[require(K14Dense)]
#[repr(C)]
struct K14Req {
    v: u32,
}

/// Table payload every fixture entity carries, so an insert is a genuine
/// migration rather than the in-place replace fast path.
#[derive(Component, Default, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
struct K14Pay {
    p: u32,
}

#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct K14A {
    v: u32,
}

#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct K14B {
    v: u32,
}

#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct K14C {
    v: u32,
}

/// The flag the poolless declarer targets.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct K14Warded;

/// A DENSE component that declares an initial flag state. Used by the two D4
/// tests; it deliberately has no table sibling that would raise
/// `FLAGS_ON_ATTACH` on the destination archetype, because the destination's
/// flag word is exactly the wrong oracle for a signature-excluded id.
#[derive(Component, Default, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct K14FlagDense {
    v: u32,
}

/// Requires the flag-declaring dense component, and declares nothing itself.
#[derive(Component, Default, Clone, Copy)]
#[require(K14FlagDense)]
#[repr(C)]
struct K14ReqFlag {
    v: u32,
}

// ── Bundles ─────────────────────────────────────────────────────────────────

#[derive(Bundle, Clone, Copy)]
struct BWide {
    a: K14A,
    b: K14B,
    r: K14Req,
}

#[derive(Bundle, Clone, Copy)]
struct BNarrow {
    a: K14A,
    r: K14Req,
}

#[derive(Bundle, Clone, Copy)]
struct BJustB {
    b: K14B,
}

#[derive(Bundle, Clone, Copy)]
struct BJustC {
    c: K14C,
}

#[derive(Bundle, Clone, Copy)]
struct BPayDense {
    p: K14Pay,
    d: K14Dense,
}

#[derive(Bundle, Clone, Copy)]
struct BPayBDense {
    p: K14Pay,
    b: K14B,
    d: K14Dense,
}

#[derive(Bundle, Clone, Copy)]
struct BPayOnly {
    p: K14Pay,
}

#[derive(Bundle, Clone, Copy)]
struct BDenseOnly {
    d: K14Dense,
}

#[derive(Bundle, Clone, Copy)]
struct BAnchor {
    a: K14A,
}

#[derive(Bundle, Clone, Copy)]
struct BReq {
    r: K14Req,
}

#[derive(Bundle, Clone, Copy)]
struct BReqFlag {
    r: K14ReqFlag,
}

#[derive(Bundle, Clone, Copy)]
struct BFlagDense {
    d: K14FlagDense,
}

// ════════════════════════════════════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════════════════════════════════════

/// Registers `K14FlagDense`'s `flags (K14Warded = on)` declaration. Write-once
/// per id, so every test may call it.
fn install_flag_declaration() {
    try_set_flags_direct(
        K14FlagDense::component_id().0,
        &[FlagDirectEntry {
            id_fn: K14Warded::component_id,
            initial: true,
        }],
    );
}

fn spawn<B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    world.run_system(move |mut cmds: Commands| cmds.spawn(bundle).id())
}

fn insert<B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static>(
    world: &mut EcsMaster,
    entity: Entity,
    bundle: B,
) {
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(bundle);
    });
}

fn remove_component<C: Component>(world: &mut EcsMaster, entity: Entity) {
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).remove::<C>();
    });
}

/// The declaration list of `entity`'s current archetype.
fn declaration_ids(world: &EcsMaster, entity: Entity) -> Vec<ComponentId> {
    let arch_id = world
        .entity_archetype_id(entity)
        .expect("entity must be live");
    world
        .archetype_master()
        .get_archetype(arch_id)
        .expect("archetype must be live")
        .all_component_ids()
        .to_vec()
}

/// The number of entities a dense-only query enumerates.
///
/// `Query<&K14Dense>` has NO table positive term, so its candidate archetypes
/// come from `DenseStore::arch_presence` alone — this is the seeded path D2
/// breaks. A mixed query (`(&K14Pay, &K14Dense)`) would be bounded by the table
/// include mask instead and could not see the defect.
fn dense_enumerated(world: &mut EcsMaster) -> usize {
    world.query::<&K14Dense, ()>().iter().count()
}

/// `true` iff the dense STORE still holds `entity` — the exact membership
/// oracle, and the half that stays correct while enumeration goes wrong.
fn dense_member(world: &EcsMaster, entity: Entity) -> bool {
    world.has_component(entity, K14Dense::component_id())
}

// ════════════════════════════════════════════════════════════════════════════
// Control — the fixture really mints the phantom
// ════════════════════════════════════════════════════════════════════════════

/// **Must be green before AND after the fix.** The four D1 tests below assert
/// "no panic"; without this they could all pass over a fixture in which no
/// archetype ever recorded the dense id, i.e. over nothing at all.
///
/// After the id-list split this reads `all_component_ids()` — the same list
/// under its true name.
#[test]
fn the_wide_archetype_records_the_dense_id_in_its_declaration_list() {
    let mut world = EcsMaster::new();
    let wide = spawn(
        &mut world,
        BWide {
            a: K14A { v: 1 },
            b: K14B { v: 2 },
            r: K14Req { v: 3 },
        },
    );
    let ids = declaration_ids(&world, wide);
    assert!(
        ids.contains(&K14Dense::component_id()),
        "the spawn archetype must RETAIN the required dense id in its declaration \
         list (KE10 depends on that retention). Without it there is no phantom and \
         every D1 test below is vacuous. Got {ids:?}"
    );
    assert!(
        !ids.contains(&K14Pay::component_id()),
        "sanity: the list is this archetype's own, not a global one"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D1 — the reachable panic, four faces
// ════════════════════════════════════════════════════════════════════════════

/// D1, insert face. The three-step repro from the register, verbatim: a WIDE
/// bundle mints `{A, B, Req}` with the dense id in its declaration list; a
/// NARROW bundle gives a second entity `{A, Req}`; inserting `(B,)` into the
/// narrow entity resolves a target by FILTERED signature mask and gets the wide
/// archetype back — declaration list and all.
#[test]
fn insert_into_a_narrow_entity_that_dedups_onto_a_dense_bearing_archetype() {
    let mut world = EcsMaster::new();
    spawn(
        &mut world,
        BWide {
            a: K14A { v: 1 },
            b: K14B { v: 2 },
            r: K14Req { v: 3 },
        },
    );
    let narrow = spawn(
        &mut world,
        BNarrow {
            a: K14A { v: 4 },
            r: K14Req { v: 5 },
        },
    );

    insert(&mut world, narrow, BJustB { b: K14B { v: 6 } });

    assert!(
        dense_member(&world, narrow),
        "the entity keeps its dense membership across the table migration"
    );
    assert_eq!(
        dense_enumerated(&mut world),
        2,
        "both entities are dense members and must both be enumerated"
    );
    // Reviewer O1: same argument as the remove face — assert the table INSERT
    // landed, or a no-op "fix" satisfies every assertion above.
    assert!(
        world.has_component(narrow, K14B::component_id()),
        "the inserted TABLE component must be present — otherwise the test \
         cannot tell a real migration from a skipped one"
    );
}

/// D1, remove face. `without_component_archetype_id` builds the kept set from
/// the SIGNATURE, so the target resolves onto an archetype minted earlier from
/// a list that still names the dense id.
#[test]
fn remove_from_an_entity_in_a_dense_bearing_archetype() {
    let mut world = EcsMaster::new();
    // Mint `{A, Req}` FIRST so the removal's target dedups onto it.
    spawn(
        &mut world,
        BNarrow {
            a: K14A { v: 1 },
            r: K14Req { v: 2 },
        },
    );
    let wide = spawn(
        &mut world,
        BWide {
            a: K14A { v: 3 },
            b: K14B { v: 4 },
            r: K14Req { v: 5 },
        },
    );

    remove_component::<K14B>(&mut world, wide);

    assert!(
        dense_member(&world, wide),
        "removing a TABLE component must not disturb dense membership"
    );
    // Reviewer O1: "did not panic" is satisfied by a repair that turns the
    // remove into a NO-OP, which is the cheaper regression. Pin the table op
    // itself so a silently-skipped migration is red rather than green.
    assert!(
        !world.has_component(wide, K14B::component_id()),
        "the TABLE remove must actually have happened — a fix that merely stops \
         panicking by skipping the migration keeps the assertion above green"
    );
}

/// D1, tag-attach face. Any dynamic-tag attach onto an entity whose archetype
/// declaration list names a dense id walks that list resolving pools.
#[test]
fn add_tag_to_an_entity_carrying_a_dense_component() {
    let mut world = EcsMaster::new();
    let e = spawn(
        &mut world,
        BNarrow {
            a: K14A { v: 1 },
            r: K14Req { v: 2 },
        },
    );
    let tag = world.register_tag("ke14_attach_tag");

    world.add_tag(e, tag);

    assert!(world.has_tag(e, tag), "the tag attached");
    assert!(dense_member(&world, e), "dense membership survived the attach");
}

/// D1, tag-detach face. Reaching it needs a source archetype whose declaration
/// list names the dense id while the ENTITY need not be a member — the phantom
/// alone is enough, which is the point. `create_archetype` mints
/// `{Pay, tag, Dense}` up front; the attach then dedups onto it by filtered
/// mask, and the detach walks the list it recorded.
#[test]
fn remove_tag_from_an_entity_carrying_a_dense_component() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("ke14_detach_tag");
    // `create_archetype` takes the id list verbatim into the declaration record,
    // and the migration helpers debug-assert canonical (ascending) order on it —
    // the order a bundle-minted or merged list always has. A hand-written slice
    // has to sort itself; `register_tag` mints at test time, so the tag's id is
    // not positionally predictable.
    let mut ids = [
        K14Pay::component_id(),
        tag.component_id(),
        K14Dense::component_id(),
    ];
    ids.sort_unstable_by_key(|c| c.0);
    world.create_archetype(&ids);

    let e = spawn(&mut world, BPayOnly { p: K14Pay { p: 1 } });
    world.add_tag(e, tag);
    assert!(
        declaration_ids(&world, e).contains(&K14Dense::component_id()),
        "precondition: the attach deduped onto the pre-minted phantom-bearing \
         archetype — without this the detach walks a dense-free list and the test \
         measures nothing"
    );

    world.remove_tag(e, tag);

    assert!(!world.has_tag(e, tag), "the tag detached");
}

// ════════════════════════════════════════════════════════════════════════════
// D2 — the silent wrong answer, four faces plus the required shape
// ════════════════════════════════════════════════════════════════════════════

/// D2, insert face, with a PLAIN dense bundle member — no `#[require]`
/// anywhere. The required shape is merely the one the register reached first;
/// this is the general case.
///
/// Fresh world; the destination `{Pay, B}` never hosted a dense member.
#[test]
fn retained_dense_member_survives_a_table_insert() {
    let mut world = EcsMaster::new();
    let e = spawn(
        &mut world,
        BPayDense {
            p: K14Pay { p: 1 },
            d: K14Dense { x: 7 },
        },
    );
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "precondition: the spawn seeded the source archetype"
    );

    insert(&mut world, e, BJustB { b: K14B { v: 2 } });

    assert!(
        dense_member(&world, e),
        "control: the STORE still holds the component…"
    );
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "…and the entity must still be ENUMERATED. Zero here is D2: the store and \
         the query disagree, because nothing re-seeded `arch_presence` for the \
         archetype the entity migrated INTO"
    );
}

/// D2, remove face. Fresh world; the destination `{Pay}` is minted by this
/// removal and never hosted a dense member.
#[test]
fn retained_dense_member_survives_a_table_remove() {
    let mut world = EcsMaster::new();
    let e = spawn(
        &mut world,
        BPayBDense {
            p: K14Pay { p: 1 },
            b: K14B { v: 2 },
            d: K14Dense { x: 7 },
        },
    );
    assert_eq!(dense_enumerated(&mut world), 1, "precondition");

    remove_component::<K14B>(&mut world, e);

    assert!(dense_member(&world, e), "control: the STORE still holds it");
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "the entity must still be enumerated after a TABLE remove"
    );
}

/// D2, tag-attach face. The dense component is inserted through the in-place
/// replace path (which seeds `{Pay}` only), so the attach destination
/// `{Pay, tag}` is genuinely unseeded.
#[test]
fn retained_dense_member_survives_a_tag_attach() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("ke14_d2_attach_tag");
    let e = spawn(&mut world, BPayOnly { p: K14Pay { p: 1 } });
    insert(&mut world, e, BDenseOnly { d: K14Dense { x: 7 } });
    assert_eq!(dense_enumerated(&mut world), 1, "precondition");

    world.add_tag(e, tag);

    assert!(dense_member(&world, e), "control: the STORE still holds it");
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "the entity must still be enumerated after a tag ATTACH"
    );
}

/// D2, tag-detach face. The entity occupied `{Pay}` before it was ever a dense
/// member, so the detach destination is unseeded even though the entity has
/// been there before.
#[test]
fn retained_dense_member_survives_a_tag_detach() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("ke14_d2_detach_tag");
    let e = spawn(&mut world, BPayOnly { p: K14Pay { p: 1 } });
    world.add_tag(e, tag);
    insert(&mut world, e, BDenseOnly { d: K14Dense { x: 7 } });
    assert_eq!(dense_enumerated(&mut world), 1, "precondition");

    world.remove_tag(e, tag);

    assert!(dense_member(&world, e), "control: the STORE still holds it");
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "the entity must still be enumerated after a tag DETACH"
    );
}

/// D2 in the register's own wording: *a correctly constructed required dense
/// component vanishes from every dense query on the next insert.*
#[test]
fn a_constructed_required_dense_member_survives_a_second_insert() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, BAnchor { a: K14A { v: 1 } });
    insert(&mut world, e, BReq { r: K14Req { v: 2 } });
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "precondition: the required ctor ran and seeded its target archetype"
    );

    insert(&mut world, e, BJustC { c: K14C { v: 3 } });

    assert!(dense_member(&world, e), "control: the STORE still holds it");
    assert_eq!(
        dense_enumerated(&mut world),
        1,
        "the constructed requirement must survive the NEXT insert"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// D4 / D4b — declared flags dropped
// ════════════════════════════════════════════════════════════════════════════

/// D4. The destination archetype `{A, ReqFlag}` carries NO flag declarer in its
/// signature, so its `FLAGS_ON_ATTACH` word is clear — and that word gates the
/// walk that would apply the DENSE declarer's initial state. The archetype flag
/// word is a sound over-approximation for signature ids only; for a
/// signature-excluded id it is simply the wrong oracle.
#[test]
fn flags_declared_by_a_constructed_required_dense_component_are_applied_on_insert() {
    install_flag_declaration();
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, BAnchor { a: K14A { v: 1 } });

    insert(&mut world, e, BReqFlag { r: K14ReqFlag { v: 2 } });

    assert!(
        world.has_component(e, K14FlagDense::component_id()),
        "precondition: the requirement was constructed"
    );
    assert!(
        world.is_enabled::<K14Warded>(e),
        "the constructed required DENSE component's declared `on` state must be \
         applied. A clear bit is D4: the dense attach-flag walk sits inside a gate \
         on the TARGET ARCHETYPE's flag word, which a signature-excluded id never \
         contributes to"
    );
}

/// D4b, found while designing the D4 repair and on the same path as D3: the
/// in-place replace has no attach-flag pass AT ALL, so a dense bundle member
/// newly added there loses its declared state too.
#[test]
fn flags_declared_by_a_dense_bundle_member_are_applied_on_replace_in_place() {
    install_flag_declaration();
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, BAnchor { a: K14A { v: 1 } });

    // Bundle is entirely dense ⇒ the merged signature equals the source ⇒
    // `InsertCommand` takes the in-place replace fast path, not a migration.
    insert(&mut world, e, BFlagDense { d: K14FlagDense { v: 2 } });

    assert!(
        world.has_component(e, K14FlagDense::component_id()),
        "precondition: the dense member landed in its store"
    );
    assert!(
        world.is_enabled::<K14Warded>(e),
        "a newly-added dense bundle member's declared `on` state must be applied on \
         the in-place replace path too. A clear bit is D4b: that path calls no \
         attach-flag pass whatsoever"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The D2 invariant, stated directly, over orders no hand-written test reaches
// ════════════════════════════════════════════════════════════════════════════

/// One step of the random migration sequence.
#[derive(Debug, Clone, Copy)]
enum Step {
    InsertB,
    InsertC,
    RemoveB,
    RemoveC,
    AddTag,
    RemoveTag,
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::InsertB),
        Just(Step::InsertC),
        Just(Step::RemoveB),
        Just(Step::RemoveC),
        Just(Step::AddTag),
        Just(Step::RemoveTag),
    ]
}

proptest! {
    /// **The D2 invariant, asserted as itself**: the set a dense query
    /// enumerates equals the set the dense STORE holds, after EVERY structural
    /// step. `has_component` routes a dense id to the store's `e2s` map, which
    /// is the exact membership oracle; the query is bounded by `arch_presence`,
    /// which is the seed D2 fails to maintain. The two must not diverge.
    ///
    /// A fresh world per case, for the `arch_presence`-is-never-cleared reason
    /// this file's header gives.
    #[test]
    fn dense_enumeration_agrees_with_membership_under_random_migrations(
        steps in prop::collection::vec(step_strategy(), 1..12)
    ) {
        let mut world = EcsMaster::new();
        let tag = world
            .tag_by_name("ke14_prop_tag")
            .unwrap_or_else(|| world.register_tag("ke14_prop_tag"));

        // Two entities: one a dense member from birth, one never a member. The
        // second is what makes an over-approximating "fix" (mark every
        // archetype) fail rather than pass.
        let member = spawn(
            &mut world,
            BPayDense { p: K14Pay { p: 1 }, d: K14Dense { x: 7 } },
        );
        let outsider = spawn(&mut world, BPayOnly { p: K14Pay { p: 2 } });

        for step in steps {
            for e in [member, outsider] {
                match step {
                    Step::InsertB => insert(&mut world, e, BJustB { b: K14B { v: 1 } }),
                    Step::InsertC => insert(&mut world, e, BJustC { c: K14C { v: 1 } }),
                    Step::RemoveB => remove_component::<K14B>(&mut world, e),
                    Step::RemoveC => remove_component::<K14C>(&mut world, e),
                    Step::AddTag => world.add_tag(e, tag),
                    Step::RemoveTag => world.remove_tag(e, tag),
                }
            }

            let held = usize::from(dense_member(&world, member))
                + usize::from(dense_member(&world, outsider));
            prop_assert_eq!(
                dense_enumerated(&mut world),
                held,
                "after {:?} the dense query and the dense store disagree",
                step
            );
        }
    }
}
