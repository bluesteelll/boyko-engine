//! Smoke check for `#[derive(Relationship)]` / `#[derive(RelationshipTarget)]`
//! (Relations v1, developer scratch).
//!
//! This file ONLY confirms the two derives EXPAND and COMPILE, and that the
//! relationship-aware `#[derive(Component)]` folds the C2 install metadata
//! (hooks + clone-remap + serialize-remap + a non-trivial layout fingerprint)
//! into `component_id()`. The full behavioral suite (R2 link/unlink/cascade, the
//! clone/serialize remap tripwires, the W3 install-probe, the R5 compile-fail
//! tests, the R6 cyclic cascade) is the TESTER's job — this is the developer's
//! "does the derive expand" gate, mirroring `hooks_install_for_child_of_and_children`.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    Cloneability, Serializability, get_clone_info, get_hooks, get_map_entities_fn,
    get_serialize_info,
};
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::relationship::{Relationship, RelationshipTarget};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Component, Relationship, RelationshipTarget};

/// The source-of-truth foreign key (the R2 `Likes` relation). `Clone, Copy` so the
/// autoref clone classification resolves `CloneViaFn` (a relationship source MUST be
/// `Clone` for its foreign key to remap on deep clone — BUG-RELATIONS-CLONE-1; the
/// derive now enforces this at compile time, matching the in-crate `ChildOf`).
#[derive(Component, Clone, Copy, Relationship)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

/// The reverse index (the R2 `LikedBy` relation). v1: `retain_empty` is mandatory.
/// `Default` is required by the `RelationshipTarget` supertrait bound.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

/// The `#[derive(Relationship)]` produced an `impl Relationship for Likes` with the
/// expected associated type + field accessors (the round-trip `Target::Source = Self`
/// type-checks at the bound).
#[test]
fn likes_impls_relationship() {
    let dummy = Entity::with_id(EntityId(7));
    let likes = <Likes as Relationship>::from_target(dummy);
    assert_eq!(likes.target(), dummy, "target() reads the FK field back");
    // ALLOW_SELF_REFERENTIAL keeps the default (`false`) — assert at compile time.
    const { assert!(!Likes::ALLOW_SELF_REFERENTIAL) };
}

/// The `#[derive(RelationshipTarget)]` produced an `impl RelationshipTarget for
/// LikedBy` with the policy consts + collection accessors.
#[test]
fn likedby_impls_relationship_target() {
    // `linked_despawn` / `retain_empty` flags → policy consts (compile-time).
    const { assert!(LikedBy::LINKED_DESPAWN) };
    const { assert!(LikedBy::RETAIN_EMPTY) };
    let t = <LikedBy as RelationshipTarget>::with_capacity(4);
    assert_eq!(t.len(), 0);
    assert!(t.is_empty());
}

/// The relationship-aware `#[derive(Component)]` folded the GENERIC hooks into
/// `register_hooks` — the source wires on_insert + on_replace, the target wires ONLY
/// on_replace (B7). Mirrors `hooks_install_for_child_of_and_children`.
#[test]
fn derive_installs_relationship_hooks() {
    let likes = get_hooks(Likes::component_id().0).expect("Likes hooks installed");
    assert!(likes.on_insert.is_some(), "Likes wires on_insert (link)");
    assert!(likes.on_replace.is_some(), "Likes wires on_replace (unlink)");
    assert!(likes.on_add.is_none(), "Likes must NOT wire on_add");
    assert!(likes.on_remove.is_none(), "Likes must NOT wire on_remove");

    let liked = get_hooks(LikedBy::component_id().0).expect("LikedBy hooks installed");
    assert!(liked.on_replace.is_some(), "LikedBy wires on_replace (cascade)");
    assert!(liked.on_add.is_none(), "LikedBy must NOT wire on_add (B7)");
    assert!(liked.on_insert.is_none(), "LikedBy must NOT wire on_insert (B7)");
    assert!(liked.on_remove.is_none(), "LikedBy must NOT wire on_remove");
}

/// C2: the source auto-emits the entity-remap clone metadata (CloneViaFn + the
/// foreign-key map_entities_fn); the target is Ignore (rebuilt via Link commands).
#[test]
fn derive_installs_relationship_clone_remap() {
    let likes = get_clone_info(Likes::component_id().0).expect("Likes clone info installed");
    assert_eq!(
        likes.cloneability,
        Cloneability::CloneViaFn,
        "Likes carries an Entity FK ⇒ CloneViaFn (so deep-clone remap runs)"
    );
    assert!(likes.clone_fn.is_some(), "Likes installs Some(clone_via_clone)");
    assert!(
        get_map_entities_fn(Likes::component_id().0).is_some(),
        "Likes auto-emits its map_entities_fn (the FK remap, B10)"
    );

    let liked = get_clone_info(LikedBy::component_id().0).expect("LikedBy clone info installed");
    assert_eq!(
        liked.cloneability,
        Cloneability::Ignore,
        "LikedBy is the reverse index ⇒ Ignore (rebuilt via Link commands, B12)"
    );
    assert!(liked.clone_fn.is_none(), "LikedBy installs no clone fn");
}

/// C2: the source auto-emits the serialize-remap metadata (SerializeViaFn + the
/// load-remap + a NON-TRIVIAL layout fingerprint computed from `Likes`'s real layout,
/// NOT `ChildOf`'s hard-coded transparent/offset-0 value).
#[test]
fn derive_installs_relationship_serialize_remap() {
    let info = get_serialize_info(Likes::component_id().0).expect("Likes serialize info installed");
    assert_eq!(
        info.serializability,
        Serializability::SerializeViaFn,
        "Likes carries an Entity FK ⇒ SerializeViaFn (the saved id is remapped on load)"
    );
    assert!(info.serialize_fn.is_some(), "Likes installs the WireBridge encoder");
    assert!(info.deserialize_fn.is_some(), "Likes installs the WireBridge decoder");
    assert!(
        info.map_entities_fn.is_some(),
        "Likes installs the load-direction FK remap (B11)"
    );
    assert_ne!(
        info.layout_fingerprint, 0,
        "the fingerprint is computed from Likes's real layout (not zero, not copied)"
    );
}
