// Relations v1 (R5): a `#[relationship_target(...)]` with a PUBLIC collection
// field is rejected — the reverse index must not be writable by user code (the
// privacy fence; only the `*_risky` mutators may touch it, command-apply-only).
//
// Expected diagnostic: the collection field must be PRIVATE; remove the `pub`.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Relationship, RelationshipTarget};

#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

// PUBLIC field — must be a compile error (the reverse index is read-only to users).
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(pub Vec<Entity>);

fn main() {}
