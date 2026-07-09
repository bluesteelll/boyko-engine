//! Phase 22 (Tags) — Wave 1B + Wave 2A integration tests: the dynamic-tag
//! registry surface (plan D3/D8/W3) and the dynamic attach / detach migration
//! surface (plan D9/O3).
//!
//! Lives OUT-OF-CRATE deliberately (plan W3): `pub(crate)` access is
//! impossible here by construction, so every assertion below proves public
//! reachability, not just in-crate semantics.
//!
//! NOTE: the budget-exhaustion test lives in `phase22_tags_exhaustion.rs`
//! (its own test binary, hence its own process). Draining the process-global
//! `NEXT_ID` to `MAX_COMPONENTS` is permanent (the registry is write-once);
//! any sibling typed mint afterwards (`register_new` via
//! `#[derive(Component)]`) would panic — including the Wave-2A tests in THIS
//! file, which need typed components and attaches. Do NOT mint to the
//! ceiling here.
//!
//! # Why `static` counters (Wave 2A hook / observer tests)
//!
//! A `HookFn` / `ObserverFn` is a bare `unsafe fn` pointer — it cannot
//! capture. Each test owns private `static AtomicUsize` counters plus its own
//! uniquely-named tag(s), so concurrently-running tests never observe one
//! another's fires (the registries are process-wide in this binary).

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component_registry::{self, TagId};
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::{ComponentHooks, HookContext, HooksError};
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Name-keyed idempotency (plan D3 mint protocol, step 1)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn same_name_twice_returns_same_tag_id() {
    let mut world = EcsMaster::new();
    let first = world
        .try_register_tag("phase22_idem")
        .expect("budget must be available for this test binary's handful of mints");
    let second = world
        .try_register_tag("phase22_idem")
        .expect("an interned name is a success, never None");
    assert_eq!(first, second, "same name must intern to the same TagId");

    // The panicking sugar resolves to the same interned id.
    let third = world.register_tag("phase22_idem");
    assert_eq!(first, third, "register_tag must be idempotent per name too");

    // tag_by_name resolves without minting.
    assert_eq!(
        world.tag_by_name("phase22_idem"),
        Some(first),
        "tag_by_name must resolve an interned name to its TagId"
    );
}

#[test]
fn tag_by_name_never_minted_is_none() {
    let world = EcsMaster::new();
    assert_eq!(
        world.tag_by_name("phase22_never_minted"),
        None,
        "tag_by_name must not mint and must return None for an unknown name"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Two names never alias (plan O2 — idempotency is NAME-keyed, never
// TypeId-keyed: every dynamic tag shares the sentinel TypeId)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn two_names_never_alias() {
    let mut world = EcsMaster::new();
    let a = world.register_tag("phase22_alias_a");
    let b = world.register_tag("phase22_alias_b");
    assert_ne!(a, b, "distinct names must mint distinct TagIds");
    assert_ne!(
        a.component_id(),
        b.component_id(),
        "distinct tags must occupy distinct ComponentId slots"
    );
}

#[test]
fn tags_are_process_global_across_worlds() {
    let mut world_a = EcsMaster::new();
    let world_b = EcsMaster::new();
    let tag = world_a.register_tag("phase22_cross_world");
    assert_eq!(
        world_b.tag_by_name("phase22_cross_world"),
        Some(tag),
        "tags are process-global metadata: a mint through any world is visible to all"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// TagId -> ComponentId bridge (plan W3) — compiles and round-trips out of crate
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn tag_id_component_id_bridge_round_trips_out_of_crate() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_bridge");

    // All three public bridge forms agree.
    let via_method: ComponentId = tag.component_id();
    let via_from: ComponentId = ComponentId::from(tag);
    let via_into: ComponentId = tag.into();
    assert_eq!(via_method, via_from, "From<TagId> must equal component_id()");
    assert_eq!(via_method, via_into, "Into must route through the same bridge");

    // The bridge is const-usable (plan: `pub const fn component_id`).
    const fn bridge_in_const_context(tag: TagId) -> ComponentId {
        tag.component_id()
    }
    assert_eq!(bridge_in_const_context(tag), via_method);

    // Round trip: the bridged id resolves to the minted dynamic-tag layout.
    let layout = component_registry::get_layout(via_method.0)
        .expect("a minted TagId's ComponentId must resolve in the global registry");
    assert!(layout.is_zst(), "dynamic tags are size-0 layouts (D2/D3)");
    assert_eq!(layout.size, 0, "dynamic-tag layout size must be 0");
    assert_eq!(layout.alignment, 1, "dynamic-tag layout alignment must be 1");
    assert!(layout.drop_fn.is_none(), "dynamic tags carry no drop glue");
    assert_eq!(
        layout.type_name, "phase22_bridge",
        "the interned tag name doubles as the layout's type_name"
    );
}

/// The id-keyed hook surface is publicly reachable with a bridged `TagId`
/// (the W3 point: without the bridge, `register_hooks_by_id` would be
/// unreachable out of crate for dynamic tags). The behavioral H1 cases that
/// need an attach are deferred — see the Wave-2A block below.
#[test]
fn register_hooks_by_id_is_publicly_reachable_with_a_bridged_tag_id() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_hooks_by_id");
    let cid: ComponentId = tag.into();

    // Fresh tag, never attached: registration succeeds.
    assert_eq!(
        component_registry::register_hooks_by_id(cid, ComponentHooks::default()),
        Ok(()),
        "hooks for a freshly minted, never-attached tag must register"
    );

    // Write-once: a second registration is rejected, not silently merged.
    assert_eq!(
        component_registry::register_hooks_by_id(cid, ComponentHooks::default()),
        Err(HooksError::AlreadyRegistered { component_id: cid }),
        "the HOOKS table is write-once per id"
    );
}

/// H1 staleness gate, case (ii) of the plan-D8 triple (pulled forward from
/// Wave 2A): archetype CREATION — not attach — freezes the archetype's
/// `ArchetypeFlags` hook bits and flips the process-global `EVER_ARCHETYPED`
/// bit, so hooks registered afterwards must be rejected (they would silently
/// never fire in the already-created archetype). No `add_tag` surface needed:
/// `get_or_create_archetype` is public today.
#[test]
fn register_hooks_after_archetype_creation_is_rejected_as_already_archetyped() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_h1_case_ii");
    let cid: ComponentId = tag.component_id();

    // Create (not attach) an archetype containing the tag's column.
    let _archetype_id = world.get_or_create_archetype(&[cid]);

    assert_eq!(
        component_registry::register_hooks_by_id(cid, ComponentHooks::default()),
        Err(HooksError::AlreadyArchetyped { component_id: cid }),
        "archetype creation freezes hook flags: late registration must be rejected"
    );

    // Case (ii) also pins the Display contract wording (the H1 gate exists
    // precisely to turn a compile-but-lie into a named, actionable error).
    let err = component_registry::register_hooks_by_id(cid, ComponentHooks::default())
        .expect_err("the gate is sticky: EVER_ARCHETYPED never clears");
    assert!(
        err.to_string().contains("mint -> register hooks -> first attach"),
        "AlreadyArchetyped's Display must name the registration contract, got: {err}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Wave 2A — dynamic attach / detach (plan D9 / O3 / step 7)
// ════════════════════════════════════════════════════════════════════════════

/// Typed data component for the data-archetype attach / detach fixtures.
/// Lazily minted id (`register_new`) — never collides with explicit
/// `register_layout` slots used by other test binaries.
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct W2Data(u32);

#[derive(Bundle)]
struct W2DataBundle {
    d: W2Data,
}

// ── spawn_empty → add_tag → query-visible; remove_tag → back-to-empty ───────

/// Direct-API round trip: empty ↔ tagged. Pins the O3 pair — attach FROM the
/// empty archetype (zero retained columns) and detach back INTO the exact
/// same empty `ArchetypeId` (exact-mask cache) — plus `has_tag`
/// positive / negative and id-keyed query visibility.
#[test]
fn direct_empty_tagged_round_trip() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_round_trip_direct");
    let other = world.register_tag("phase22_w2a_round_trip_other");

    let e = world.spawn_empty();
    let empty_arch = world.get_entity_archetype_id(e).expect("live entity");
    assert!(!world.has_tag(e, tag), "fresh empty entity carries no tag");

    // Attach FROM the empty archetype (zero-retained shape, O3).
    world.add_tag(e, tag);
    assert!(world.has_tag(e, tag), "add_tag must make has_tag true");
    assert!(!world.has_tag(e, other), "has_tag must be per-tag, not per-entity");
    let tagged_arch = world.get_entity_archetype_id(e).expect("live entity");
    assert_ne!(empty_arch, tagged_arch, "attach must migrate out of the empty archetype");
    assert_eq!(
        world.query_entities(&[tag.component_id()]),
        vec![e],
        "the tagged entity must be visible to an id-keyed component query"
    );

    // Detach back INTO the empty archetype (detach-to-empty, O3).
    world.remove_tag(e, tag);
    assert!(!world.has_tag(e, tag), "remove_tag must clear has_tag");
    assert!(world.has_entity(e), "the entity survives with zero components");
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(empty_arch),
        "detaching the last tag must route back to the SAME empty archetype id \
         (exact-mask cache)"
    );
    assert!(
        world.query_entities(&[tag.component_id()]).is_empty(),
        "a detached entity must vanish from the tag's id-keyed query"
    );

    // Round trip again — the warm path reuses both cached archetypes.
    world.add_tag(e, tag);
    assert!(world.has_tag(e, tag));
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(tagged_arch),
        "re-attach must land in the SAME tagged archetype id (exact-mask cache)"
    );
}

/// Deferred round trip through `Commands` / `EntityCommands`: spawn_empty +
/// add_tag chained in one system, remove_tag in a second — the
/// `AddTagCommand` / `RemoveTagCommand` apply path delegates to the direct
/// API, so semantics must match `direct_empty_tagged_round_trip`.
#[test]
fn deferred_empty_tagged_round_trip() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_round_trip_deferred");

    world.run_system(move |mut cmds: Commands| {
        cmds.spawn_empty().add_tag(tag);
    });
    assert_eq!(world.entity_count(), 1, "deferred spawn_empty must materialize");
    let e = world.iter_entities().next().expect("one entity exists");
    assert!(world.has_tag(e, tag), "deferred add_tag must attach on apply (FIFO after the spawn)");
    assert_eq!(world.query_entities(&[tag.component_id()]), vec![e]);

    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove_tag(tag);
    });
    assert!(!world.has_tag(e, tag), "deferred remove_tag must detach on apply");
    assert!(world.has_entity(e), "the entity survives detach-to-empty");
    assert!(world.query_entities(&[tag.component_id()]).is_empty());
}

// ── data-archetype attach / detach (non-zero retained columns) ──────────────

/// Attaching / detaching a tag on a DATA-bearing entity must keep the entity
/// visible to typed data queries (direct `EcsMaster::query` API) with its
/// bytes intact — the retained-column memcpy of the attach / detach
/// migrations is byte-exact.
#[test]
fn tagged_entity_stays_visible_to_data_queries_with_bytes_intact() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_data_visibility");

    world.run_system(|mut cmds: Commands| {
        cmds.spawn(W2DataBundle { d: W2Data(7) });
    });
    let e = world.iter_entities().next().expect("one entity exists");

    // Attach: non-zero retained set (the W2Data column rides the migration).
    world.add_tag(e, tag);
    assert!(world.has_tag(e, tag));
    {
        let view = world.query::<&W2Data, ()>();
        let collected: Vec<u32> = view.iter().map(|d: &W2Data| d.0).collect();
        assert_eq!(
            collected,
            vec![7],
            "a tag attach must keep the entity visible to data queries with bytes intact"
        );
    }

    // Detach: the data column rides back.
    world.remove_tag(e, tag);
    assert!(!world.has_tag(e, tag));
    {
        let view = world.query::<&W2Data, ()>();
        let collected: Vec<u32> = view.iter().map(|d: &W2Data| d.0).collect();
        assert_eq!(collected, vec![7], "a tag detach must keep the data column intact");
    }
}

// ── dead-entity no-ops ───────────────────────────────────────────────────────

/// `add_tag` / `remove_tag` / `has_tag` on a despawned (stale-generation)
/// handle are silent no-ops — including against a recycled slot (plan D9).
/// The deferred path inherits the contract (despawn racing an enqueued tag
/// op within one drain is legitimate).
#[test]
fn dead_entity_tag_ops_are_no_ops() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_dead_entity");

    let e = world.spawn_empty();
    assert!(world.delete_entity(e), "despawn of a live entity succeeds");

    assert!(!world.has_tag(e, tag), "has_tag on a dead handle is false");
    world.add_tag(e, tag); // must not panic, must not resurrect
    assert!(!world.has_tag(e, tag));
    assert_eq!(world.entity_count(), 0, "add_tag must not resurrect a dead entity");
    world.remove_tag(e, tag); // must not panic

    // Recycled slot: the stale handle's generation mismatches — ops through
    // it must never touch the new occupant.
    let e2 = world.spawn_empty();
    world.add_tag(e, tag);
    assert!(
        !world.has_tag(e2, tag),
        "a stale handle must never tag the recycled entity"
    );

    // Deferred: despawn + add_tag on the same entity in one queue (FIFO) —
    // the AddTagCommand applies against a dead entity and no-ops.
    let e3 = world.spawn_empty();
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e3).despawn().add_tag(tag);
    });
    assert!(!world.has_entity(e3), "despawn applied first (FIFO)");
    assert_eq!(world.entity_count(), 1, "only the recycled-slot entity survives");
}

// ════════════════════════════════════════════════════════════════════════════
// Wave 2A — hook / observer firing at the new attach / detach sites
// (plan D8 fire-site ledger; Phase-14b lesson: count sites, then test each)
// ════════════════════════════════════════════════════════════════════════════

// ── H1 case (i): mint → register_hooks_by_id → attach ⇒ the hook fires ──────

static H1I_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn h1i_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    H1I_ADD.fetch_add(1, SEQ);
}

#[test]
fn h1_case_i_hooks_registered_before_first_attach_fire() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_h1_case_i");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks { on_add: Some(h1i_on_add), ..Default::default() },
    )
    .expect("fresh tag, never archetyped: registration must succeed (contract order)");

    let e = world.spawn_empty();
    world.add_tag(e, tag);
    assert_eq!(
        H1I_ADD.load(SEQ),
        1,
        "mint → register → attach: the id-keyed on_add hook must fire on the attach"
    );
}

// ── H1 case (iii): hook bits are baked into archetypes created after
//    registration (behavioral pin: the bits are durable archetype state) ─────

static H1III_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn h1iii_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    H1III_ADD.fetch_add(1, SEQ);
}

#[test]
fn h1_case_iii_hook_bits_baked_into_archetype_created_after_registration() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_h1_case_iii");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks { on_add: Some(h1iii_on_add), ..Default::default() },
    )
    .expect("fresh tag, never archetyped");

    // First attach CREATES the hosting archetype AFTER registration — the
    // creation funnel OR-computes the hook bit into its ArchetypeFlags.
    let e1 = world.spawn_empty();
    world.add_tag(e1, tag);
    assert_eq!(H1III_ADD.load(SEQ), 1, "fires for the archetype-creating attach");

    // Second attach routes into the SAME pre-existing archetype: the hook
    // fires again ⇒ the bit is durable archetype state, not a creation-time
    // side effect.
    let e2 = world.spawn_empty();
    world.add_tag(e2, tag);
    assert_eq!(
        H1III_ADD.load(SEQ),
        2,
        "the hook bit baked at creation must keep firing for later attaches"
    );
}

// ── H1 gate: EVER_ARCHETYPED flips via the add_tag path itself ───────────────

#[test]
fn ever_archetyped_flips_on_first_hosting_archetype_via_add_tag() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_h1_flip_via_attach");
    let cid: ComponentId = tag.component_id();

    let e = world.spawn_empty();
    world.add_tag(e, tag); // first hosting archetype is created HERE

    assert_eq!(
        component_registry::register_hooks_by_id(cid, ComponentHooks::default()),
        Err(HooksError::AlreadyArchetyped { component_id: cid }),
        "the attach path's archetype creation must flip the H1 staleness bit"
    );
}

// ── W3 reachability chain: register_tag → component_id() →
//    register_hooks_by_id + add_observer → add_tag → BOTH fire ───────────────

static W3_HOOK: AtomicUsize = AtomicUsize::new(0);
static W3_OBSERVER: AtomicUsize = AtomicUsize::new(0);

unsafe fn w3_on_add_hook(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    W3_HOOK.fetch_add(1, SEQ);
}
unsafe fn w3_on_add_observer(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Add, "observer ctx.kind matches the registered kind");
    W3_OBSERVER.fetch_add(1, SEQ);
}

#[test]
fn w3_reachability_chain_hook_and_observer_both_fire_on_attach() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_w3_chain");
    let cid: ComponentId = tag.component_id();

    // The full public chain (plan W3): every step uses only out-of-crate
    // surfaces — the bridge, the id-keyed hook entry, the id-keyed observer.
    component_registry::register_hooks_by_id(
        cid,
        ComponentHooks { on_add: Some(w3_on_add_hook), ..Default::default() },
    )
    .expect("fresh tag, never archetyped");
    world.add_observer(ObserverKind::Add, cid, w3_on_add_observer);

    let e = world.spawn_empty();
    world.add_tag(e, tag);

    assert_eq!(W3_HOOK.load(SEQ), 1, "the id-keyed hook must fire on attach");
    assert_eq!(W3_OBSERVER.load(SEQ), 1, "the id-keyed observer must fire on attach");
}

// ── In-place re-add (present tag): on_replace + on_insert, NO on_add ─────────

static RE_ADD: AtomicUsize = AtomicUsize::new(0);
static RE_REPLACE: AtomicUsize = AtomicUsize::new(0);
static RE_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn re_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    RE_ADD.fetch_add(1, SEQ);
}
unsafe fn re_on_replace(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    RE_REPLACE.fetch_add(1, SEQ);
}
unsafe fn re_on_insert(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    RE_INSERT.fetch_add(1, SEQ);
}

/// Re-adding a present tag takes the in-place replace semantics (plan D8):
/// `on_replace` + `on_insert` fire, `on_add` does NOT, and the entity does
/// not migrate. Covers the `retag_in_place` fire site (direct AND deferred
/// routes share it via the command's delegation).
#[test]
fn re_add_present_tag_fires_replace_and_insert_in_place() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_re_add_in_place");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks {
            on_add: Some(re_on_add),
            on_replace: Some(re_on_replace),
            on_insert: Some(re_on_insert),
            ..Default::default()
        },
    )
    .expect("fresh tag, never archetyped");

    let e = world.spawn_empty();
    world.add_tag(e, tag); // first attach: on_add + on_insert
    assert_eq!((RE_ADD.load(SEQ), RE_REPLACE.load(SEQ), RE_INSERT.load(SEQ)), (1, 0, 1));
    let tagged_arch = world.get_entity_archetype_id(e).expect("live entity");

    world.add_tag(e, tag); // present tag: in-place replace semantics (direct)
    assert_eq!(
        (RE_ADD.load(SEQ), RE_REPLACE.load(SEQ), RE_INSERT.load(SEQ)),
        (1, 1, 2),
        "re-add must fire on_replace + on_insert and must NOT fire on_add"
    );
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(tagged_arch),
        "in-place re-add must not migrate the entity"
    );

    // Deferred route: AddTagCommand delegates to the same in-place path.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).add_tag(tag);
    });
    assert_eq!(
        (RE_ADD.load(SEQ), RE_REPLACE.load(SEQ), RE_INSERT.load(SEQ)),
        (1, 2, 3),
        "the deferred re-add must take the same in-place replace semantics"
    );
}

// ── Detach fires on_replace THEN on_remove against the dying row ────────────

static RM_SEQ: AtomicUsize = AtomicUsize::new(0);
static RM_REPLACE_AT: AtomicUsize = AtomicUsize::new(0);
static RM_REMOVE_AT: AtomicUsize = AtomicUsize::new(0);

unsafe fn rm_on_replace(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    RM_REPLACE_AT.store(RM_SEQ.fetch_add(1, SEQ) + 1, SEQ);
}
unsafe fn rm_on_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    RM_REMOVE_AT.store(RM_SEQ.fetch_add(1, SEQ) + 1, SEQ);
}

/// `remove_tag` fires `on_replace` then `on_remove` (the SAFETY-2 ordering)
/// for the detached tag — here on the detach-TO-EMPTY shape (the tag is the
/// entity's only component), pinning the O3 hook coverage.
#[test]
fn remove_tag_fires_replace_then_remove() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_detach_fires");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks {
            on_replace: Some(rm_on_replace),
            on_remove: Some(rm_on_remove),
            ..Default::default()
        },
    )
    .expect("fresh tag, never archetyped");

    let e = world.spawn_empty();
    world.add_tag(e, tag);
    assert_eq!((RM_REPLACE_AT.load(SEQ), RM_REMOVE_AT.load(SEQ)), (0, 0));

    world.remove_tag(e, tag); // detach-to-empty
    let (replace_at, remove_at) = (RM_REPLACE_AT.load(SEQ), RM_REMOVE_AT.load(SEQ));
    assert_eq!(replace_at, 1, "on_replace must fire exactly once, first");
    assert_eq!(remove_at, 2, "on_remove must fire exactly once, after on_replace");
    assert!(!world.has_tag(e, tag));
    assert!(world.has_entity(e), "detach-to-empty keeps the entity alive");
}

// ── Deferred command routes fire the same hooks (Phase-14b lesson) ───────────

static DEF_ADD: AtomicUsize = AtomicUsize::new(0);
static DEF_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn def_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    DEF_ADD.fetch_add(1, SEQ);
}
unsafe fn def_on_remove(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    DEF_REMOVE.fetch_add(1, SEQ);
}

/// The deferred `AddTagCommand` / `RemoveTagCommand` apply sites fire hooks —
/// the exact gap class the Phase-14b tester caught (observers silent for the
/// whole `Commands` API). The commands delegate to the direct API, so this
/// pins the delegation, not a parallel implementation.
#[test]
fn deferred_add_and_remove_tag_fire_hooks() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_deferred_fires");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks {
            on_add: Some(def_on_add),
            on_remove: Some(def_on_remove),
            ..Default::default()
        },
    )
    .expect("fresh tag, never archetyped");

    world.run_system(move |mut cmds: Commands| {
        cmds.spawn_empty().add_tag(tag);
    });
    assert_eq!(DEF_ADD.load(SEQ), 1, "deferred add_tag must fire on_add at apply");

    let e = world.iter_entities().next().expect("one entity exists");
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove_tag(tag);
    });
    assert_eq!(DEF_REMOVE.load(SEQ), 1, "deferred remove_tag must fire on_remove at apply");
}

// ── Replace / Remove / Insert OBSERVERS at the new fire sites ────────────────
//    (the ON_*_OBSERVER flag bits are gated separately from ON_*_HOOK — an
//    observer-only registration must still fire; the hook tests above cannot
//    catch a dropped observer branch)

static OBS_REPLACE: AtomicUsize = AtomicUsize::new(0);
static OBS_REMOVE: AtomicUsize = AtomicUsize::new(0);
static OBS_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn obs_on_replace(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Replace, "observer ctx.kind matches the registered kind");
    OBS_REPLACE.fetch_add(1, SEQ);
}
unsafe fn obs_on_remove(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Remove, "observer ctx.kind matches the registered kind");
    OBS_REMOVE.fetch_add(1, SEQ);
}
unsafe fn obs_on_insert(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Insert, "observer ctx.kind matches the registered kind");
    OBS_INSERT.fetch_add(1, SEQ);
}

/// Replace + Remove (plus Insert, for completeness) OBSERVER coverage at the
/// new tag fire sites. Registration is observer-ONLY (no hooks at all) through
/// the `TagId` → `ComponentId` bridge, so every fire below proves the
/// `ON_*_OBSERVER` flag bits gate independently of their `ON_*_HOOK` siblings:
///
/// * fresh attach (`migrate_entity_attach_ids`): Insert fires; Replace and
///   Remove do NOT (no old value exists yet);
/// * present-tag re-add (`retag_in_place`): Replace + Insert fire;
/// * detach (`migrate_entity_detach_ids`): Replace + Remove fire.
#[test]
fn tag_fire_sites_fire_replace_remove_and_insert_observers() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("phase22_w2a_observer_kinds");
    let cid: ComponentId = tag.component_id();

    world.add_observer(ObserverKind::Replace, cid, obs_on_replace);
    world.add_observer(ObserverKind::Remove, cid, obs_on_remove);
    world.add_observer(ObserverKind::Insert, cid, obs_on_insert);

    let counts = || (OBS_REPLACE.load(SEQ), OBS_REMOVE.load(SEQ), OBS_INSERT.load(SEQ));

    let e = world.spawn_empty();
    world.add_tag(e, tag); // fresh attach — migrate_entity_attach_ids
    assert_eq!(
        counts(),
        (0, 0, 1),
        "fresh attach must fire the Insert observer only (no replace yet)"
    );

    world.add_tag(e, tag); // present tag — retag_in_place
    assert_eq!(
        counts(),
        (1, 0, 2),
        "in-place re-add must fire the Replace + Insert observers"
    );

    world.remove_tag(e, tag); // detach — migrate_entity_detach_ids
    assert_eq!(
        counts(),
        (2, 1, 2),
        "detach must fire the Replace + Remove observers"
    );
}
