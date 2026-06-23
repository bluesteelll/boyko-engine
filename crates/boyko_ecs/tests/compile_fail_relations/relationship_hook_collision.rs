// Relations v1 (R5 / Decision 4): a `#[relationship(...)]` source OWNS the
// `on_insert` / `on_replace` hook slots (it wires the generic link/unlink). A
// user `#[component(on_insert = ...)]` alongside it would silently lose to (or
// double-install with) the generic hook — the macro rejects it loudly.
//
// Expected diagnostic: the relationship owns the hook slot; remove the
// `#[component(on_insert=...)]`.

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::{Component, Relationship, RelationshipTarget};

unsafe fn user_on_insert(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {}

// COLLISION: `#[relationship]` + `#[component(on_insert=...)]` on the same type.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
#[component(on_insert = user_on_insert)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, linked_despawn, retain_empty)]
struct LikedBy(Vec<Entity>);

fn main() {}
