//! `Bundle` impls for the two hierarchy components ([`ChildOf`] / [`Children`]),
//! Phase 22 D7.
//!
//! # Why a macro mirror instead of `#[derive(Component)]`'s Bundle emission
//!
//! `boyko-macros` is a **dev-dependency** of `boyko-ecs` (Cargo.toml), so its
//! derive macros are unavailable to library `src/` code. Downstream components
//! get `impl Bundle for Self` from `#[derive(Component)]` automatically; engine
//! components defined here use [`impl_self_bundle!`] — the in-crate, hand-written
//! mirror of that exact emission (see `bundle/self_bundle.rs` for the macro and
//! its SAFETY accounting).
//!
//! The Phase-19 `ChildOfBundle` / `ChildrenBundle` 1-field newtypes are deleted:
//! with `ChildOf` / `Children` being `Bundle`s themselves, every former
//! `ChildOfBundle(ChildOf(parent))` call site now passes the component directly
//! through the same audited `merged_archetype_id` / `migrate_entity_insert` /
//! `InsertCommand` machinery.

use crate::ecs::core::bundle::self_bundle::impl_self_bundle;
use crate::ecs::core::hierarchy::{ChildOf, Children};

impl_self_bundle!(ChildOf);
impl_self_bundle!(Children);
