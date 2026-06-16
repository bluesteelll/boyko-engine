//! Entity cloning (Feature 3) — `EntityCloner`, the typestate builder, the
//! materialization (Algorithm A), and the deep `ChildOf`-subtree clone
//! (Algorithm B).
//!
//! Reflection-free, fn-ptr-driven, relationship-aware (deep/shallow over
//! `ChildOf`). 0% cost when unused: the clone metadata lives in parallel cold
//! tables (`CLONE` / `MAP_ENTITIES` in `component_registry`), read ONLY from this
//! module; `ComponentLayout` is unchanged (TRIPWIRE 2), and a program that never
//! clones is byte-identical (registration-time delta only).
//!
//! # Entry points
//!
//! The shipped (v1) public surface is on
//! [`EcsMaster`](crate::ecs::core::ecs_master::ecs_master::EcsMaster)
//! (`clone_and_spawn` / `clone_and_spawn_with` / `clone_subtree`) and
//! [`Commands`](crate::ecs::core::system::params::commands::Commands)
//! (`clone_and_spawn` — deferred). [`EntityCloner`] is the reusable config object.
//!
//! `clone_components` / `move_components` (clone-onto / move-onto an existing
//! target entity) are **deferred to v1.1** — not yet implemented.
//!
//! # EnableTag state is NOT cloned (v1)
//!
//! An EnableTag (`#[component(storage = "bitset")]`) has no `ComponentPool`; its
//! presence is a per-row enable bit, not pool bytes. A v1 clone does **not** carry
//! the enable/disable bit: a bitset id is skipped during materialization (see
//! `materialize.rs`, W1), so the clone lands in an archetype **without** the tag.
//! Cloning an entity that carries an EnableTag is sound — it produces a valid clone,
//! just without the tag. Preserving the enable-state across a clone is a **v1.1
//! follow-up** (read the source row's enable bit, re-apply it on the target after
//! materialization).

pub mod cloner;
pub mod deep;
pub mod map;
pub mod materialize;

pub use cloner::{EntityCloner, EntityClonerBuilder, OptIn, OptOut};
pub use map::EntityCloneMap;

/// Hard cap on the number of nodes a single deep-subtree clone visits — a cycle
/// tripwire (only `ChildOf` self-reference is guarded elsewhere; a deeper cycle
/// would otherwise loop forever). Sized generously: a real subtree never
/// approaches it. Same exposure class as the despawn-cascade depth cap.
pub(crate) const MAX_CLONE_SUBTREE_NODES: usize = 1 << 20;
