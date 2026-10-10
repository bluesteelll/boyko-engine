//! **KM2 — `on_despawn` unlocked in `#[derive(Component)]`.**
//!
//! The kernel has carried the entity-level despawn hook since Feature 2:
//! `ComponentHooks::on_despawn` exists, `ArchetypeFlags::ON_DESPAWN_HOOK` is
//! OR-computed from it (`archetype_flags.rs`, `ArchetypeFlags::add_component`),
//! `trigger_on_despawn` dispatches it, and
//! `tests/feature2_observers_despawn_reentrancy.rs` proves it fires — through
//! the RUNTIME builder (`register_component_hooks::<T>().on_despawn(..)`).
//! Only the derive refused the key, with "deferred to Phase 14b".
//!
//! That refusal was guarding nothing: it was verified before the unlock by
//! running `feature2_observers_despawn_reentrancy` green on the unmodified tree.
//!
//! # Why the fire test is the one that matters
//!
//! A derive that ACCEPTS the key and wires it to nothing passes any test that
//! only checks "it compiles". So the acceptance test below is paired with two
//! tests that despawn a real entity and count fires, and one that reads the
//! dying entity's still-intact value (which also pins that the derive-installed
//! hook lands in the same pre-drop slot the runtime builder's does).
//!
//! # The relationship section is not incidental
//!
//! `#[derive(Component)]` generates `register_hooks` from EITHER the user's
//! `#[component(on_*)]` keys OR — for a relationship side — the relationship's
//! own generic hooks; the two were never merged. `reject_hook_collision`
//! refuses only the OWNED slots (`on_insert` / `on_replace`), and its own doc
//! comment states that "the other two slots (`on_add` / `on_remove`) are free …
//! so they compose without conflict". They did not: the whole user
//! `register_hooks` body was discarded for a relationship type, so a user
//! `on_add` on a `#[derive(Relationship)]` component compiled and never fired.
//! Unlocking `on_despawn` without fixing that would have added a third silently
//! dropped key. The merge landed with this rung; these tests were observed red
//! against the unlock-only tree.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::get_hooks;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::Relationship;
use boyko_macros::{Component, Relationship, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// 1 — the derive ACCEPTS the key and installs it in the right slot
// ════════════════════════════════════════════════════════════════════════════

static K2_FIRES: AtomicUsize = AtomicUsize::new(0);
static K2_SEEN: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Reads the dying entity's still-intact value, proving the derive-installed
/// hook lands in the pre-drop `on_despawn` slot and not somewhere adjacent.
unsafe fn k2_on_despawn(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    if let Some(v) = w.get_component::<K2Hp>(ctx.entity) {
        K2_SEEN.store(v.0 as usize, SEQ);
    }
    K2_FIRES.fetch_add(1, SEQ);
}

#[derive(Component, Clone, Copy)]
#[component(on_despawn = k2_on_despawn)]
#[repr(C)]
struct K2Hp(u32);

/// Derive test: the key parses, and the generated `register_hooks` writes the
/// `on_despawn` slot (and only that slot).
#[test]
fn derive_installs_the_on_despawn_slot() {
    let id = K2Hp::component_id();
    let hooks = get_hooks(id.0).expect("the derive installed a HOOKS entry");
    assert!(
        hooks.on_despawn.is_some(),
        "KM2: #[component(on_despawn = f)] must populate ComponentHooks::on_despawn"
    );
    assert!(
        hooks.on_add.is_none() && hooks.on_insert.is_none() && hooks.on_remove.is_none(),
        "the on_despawn key must not spill into the other slots"
    );
    // Compile-time claim, so a `const` block (clippy::assertions_on_constants is
    // `-D warnings` here): an on_despawn-only derive must still set HAS_HOOKS,
    // or the gated `install_hooks` call in `component_id()` never runs and the
    // slot asserted above would be empty.
    const { assert!(K2Hp::HAS_HOOKS) };
}

/// The test that matters: a derive that accepted the key and wired it to
/// nothing would pass the one above and fail this one.
#[test]
fn derive_on_despawn_fires_once_and_reads_the_intact_value() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[K2Hp::component_id()]);
    let e = ecs.spawn_one(arch, K2Hp(99)).expect("spawn");

    K2_FIRES.store(0, SEQ);
    K2_SEEN.store(usize::MAX, SEQ);
    assert_eq!(K2_FIRES.load(SEQ), 0, "no fire before the despawn");

    assert!(ecs.delete_entity(e), "despawn");
    assert_eq!(
        K2_FIRES.load(SEQ),
        1,
        "KM2 HEADLINE: the DERIVE-wired on_despawn fires exactly once per dying entity"
    );
    assert_eq!(
        K2_SEEN.load(SEQ),
        99,
        "the derive-wired hook fires PRE-drop — it read the intact value, exactly \
         as the runtime-builder path does"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 2 — HAS_HOOKS stays false, and the slot stays empty, without the key
// ════════════════════════════════════════════════════════════════════════════

/// Zero-when-unused floor: a plain derive must not gain an `on_despawn` slot,
/// an `install_hooks` call, or the `ON_DESPAWN_HOOK` archetype flag.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct K2Plain(u32);

#[test]
fn a_derive_without_the_key_installs_no_hooks_at_all() {
    let id = K2Plain::component_id();
    // Zero-when-unused, at compile time: a component that names no hook key
    // keeps `HAS_HOOKS = false`, so the install call const-folds away.
    const { assert!(!K2Plain::HAS_HOOKS) };
    assert!(
        get_hooks(id.0).is_none_or(|h| h.on_despawn.is_none()),
        "zero-when-unused: no on_despawn slot for a component that never asked for one"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 3 — relationship sides: the NON-OWNED hooks must survive the generated body
// ════════════════════════════════════════════════════════════════════════════

static K2_REL_DESPAWN: AtomicUsize = AtomicUsize::new(0);
static K2_REL_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn k2_rel_on_despawn(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    K2_REL_DESPAWN.fetch_add(1, SEQ);
}
unsafe fn k2_rel_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    K2_REL_ADD.fetch_add(1, SEQ);
}

/// A relationship SOURCE that also declares the two hooks the relationship does
/// NOT own. `reject_hook_collision` accepts this combination — its doc comment
/// says the non-owned slots "compose without conflict".
#[derive(Component, Clone, Copy, Relationship)]
#[component(on_add = k2_rel_on_add, on_despawn = k2_rel_on_despawn)]
#[relationship(target = K2LikedBy)]
struct K2Likes(pub Entity);

/// The reverse index for `K2Likes`.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = K2Likes, linked_despawn, retain_empty)]
struct K2LikedBy(Vec<Entity>);

/// The relationship's OWN hooks must still be wired — the merge must not
/// clobber the slots the relationship owns. This is the over-correction guard.
#[test]
fn relationship_still_owns_its_own_hook_slots() {
    let id = K2Likes::component_id();
    let hooks = get_hooks(id.0).expect("a relationship source installs hooks");
    assert!(
        hooks.on_insert.is_some() && hooks.on_replace.is_some(),
        "a Relationship source owns on_insert (link) + on_replace (unlink); the \
         user-hook merge must not drop them"
    );
    // The generated `impl Relationship` still type-checks and reads its FK back.
    let dummy = Entity::with_id(boyko_ecs::ecs::identifiers::primitives::EntityId(7));
    assert_eq!(<K2Likes as Relationship>::from_target(dummy).target(), dummy);
}

/// The user's NON-OWNED hooks must be present in the merged body.
#[test]
fn relationship_keeps_the_users_non_owned_hook_slots() {
    let id = K2Likes::component_id();
    let hooks = get_hooks(id.0).expect("a relationship source installs hooks");
    assert!(
        hooks.on_add.is_some(),
        "a user #[component(on_add = ..)] on a relationship side must survive: \
         reject_hook_collision explicitly documents on_add as a free slot that \
         'composes without conflict'. Before this rung the generated \
         relationship register_hooks REPLACED the user body, so it compiled and \
         never fired"
    );
    assert!(
        hooks.on_despawn.is_some(),
        "same for on_despawn, the key KM2 unlocks — unlocking it over the \
         un-merged body would have added a third silently dropped key"
    );
}

/// The behavioural half of the two above: the merged hooks actually FIRE.
#[test]
fn relationship_non_owned_hooks_fire() {
    let mut ecs = EcsMaster::new();
    let target_arch = ecs.create_archetype(&[K2LikedBy::component_id()]);
    let target = ecs
        .spawn_one(target_arch, K2LikedBy(Vec::new()))
        .expect("spawn the relation target");

    K2_REL_ADD.store(0, SEQ);
    K2_REL_DESPAWN.store(0, SEQ);

    let src_arch = ecs.create_archetype(&[K2Likes::component_id()]);
    let src = ecs
        .spawn_one(src_arch, K2Likes(target))
        .expect("spawn the relation source");
    assert_eq!(
        K2_REL_ADD.load(SEQ),
        1,
        "the user's on_add on a relationship side fires on attach"
    );

    assert!(ecs.delete_entity(src), "despawn the source");
    assert_eq!(
        K2_REL_DESPAWN.load(SEQ),
        1,
        "the user's on_despawn on a relationship side fires on despawn"
    );
}
