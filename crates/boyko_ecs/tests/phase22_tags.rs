//! Phase 22 (Tags) — Wave 1B integration tests: the dynamic-tag registry
//! surface (plan D3/D8/W3).
//!
//! Lives OUT-OF-CRATE deliberately (plan W3): `pub(crate)` access is
//! impossible here by construction, so every assertion below proves public
//! reachability, not just in-crate semantics.
//!
//! NOTE: the budget-exhaustion test lives in `phase22_tags_exhaustion.rs`
//! (its own test binary, hence its own process). Draining the process-global
//! `NEXT_ID` to `MAX_COMPONENTS` is permanent (the registry is write-once);
//! any sibling typed mint afterwards (`register_new` via
//! `#[derive(Component)]`) would panic — including the Wave-2A additions to
//! THIS file, which need typed components and attaches.

use boyko_ecs::ecs::core::component::component_registry::{self, TagId};
use boyko_ecs::ecs::core::component::hooks::{ComponentHooks, HooksError};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;

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
// Wave 2A: deferred tests (sanctioned by the plan's post-approval note 2 —
// the attach surface `add_tag` does not exist yet, and the id-keyed internal
// attach paths are not public).
//
// 1. `h1_three_case_staleness_gate` (plan D8 / W2) — case (ii) is covered
//    TODAY by `register_hooks_after_archetype_creation_is_rejected_as_already_archetyped`
//    above (archetype creation, not attach, flips EVER_ARCHETYPED); the
//    remaining cases need the attach surface:
//    (i)   mint → register_hooks_by_id → attach  ⇒ the hook fires;
//    (iii) the hook bits are present in the ArchetypeFlags of an archetype
//          created after registration.
//
// 2. `w3_reachability_chain` (plan D8 / W3):
//    register_tag → tag.component_id() → register_hooks_by_id + add_observer
//    → add_tag → assert hook AND observer fired.
//
// Wave-2A authors: this binary must stay exhaustion-free (see the module
// docs) — typed `#[derive(Component)]` mints panic once NEXT_ID hits the
// ceiling, so keep the budget-draining test in `phase22_tags_exhaustion.rs`.
// ════════════════════════════════════════════════════════════════════════════
