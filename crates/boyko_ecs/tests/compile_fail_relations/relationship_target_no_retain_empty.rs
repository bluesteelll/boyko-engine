// Relations v1 (R5 / W1): a `#[relationship_target(...)]` WITHOUT the mandatory
// `retain_empty` flag is rejected (RETAIN_EMPTY = true is mandatory in v1;
// remove-on-empty is deferred to v1.1).
//
// Expected diagnostic: the macro rejects the missing `retain_empty` flag.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Relationship, RelationshipTarget};

#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

// MISSING `retain_empty` — must be a compile error (v1 mandatory-true).
#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn)]
struct LikedBy(Vec<Entity>);

fn main() {}
