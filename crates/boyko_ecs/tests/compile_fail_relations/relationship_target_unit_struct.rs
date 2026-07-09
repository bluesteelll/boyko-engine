// Relations v1 (R5): a unit struct as a `#[relationship_target(...)]` is rejected
// — a relationship target needs exactly one collection field (a unit struct has
// none).
//
// Expected diagnostic: requires exactly one collection field; a unit struct has
// none.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Relationship, RelationshipTarget};

#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

// UNIT STRUCT target — no collection field. Must be a compile error.
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy;

fn main() {}
