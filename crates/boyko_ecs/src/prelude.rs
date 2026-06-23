//! Common `boyko-ecs` imports, collapsed into one glob: `use boyko_ecs::prelude::*;`.
//!
//! A curated, pure re-export of the public surface — zero runtime cost, no new
//! types. The deep module paths remain available for anything not listed here.
//!
//! # Derive macros are NOT re-exported here
//!
//! `#[derive(Component)]` / `#[derive(Resource)]` / `#[derive(Bundle)]` /
//! `#[derive(SystemSet)]` / `#[event]` live in the `boyko-macros` crate, which
//! `boyko-ecs` depends on only as a **dev-dependency**. Re-exporting them from
//! this prelude would require promoting `boyko-macros` to a normal dependency
//! of `boyko-ecs`. Until that decision is made, import the derives directly:
//! `use boyko_macros::{Component, Resource, Bundle, SystemSet};`.

// ── Phase 18 — App + plugins ─────────────────────────────────────────────────
pub use crate::ecs::core::app::{App, AppExit, Plugin, Plugins};

// ── Phase 20 — multi-schedule + time ─────────────────────────────────────────
pub use crate::ecs::core::app::{CoreSchedule, EventUpdatePolicy};
pub use crate::ecs::core::time::{FixedTime, Time, fixed_advance};

// ── World + core entity/component model ──────────────────────────────────────
pub use crate::ecs::core::bundle::Bundle;
pub use crate::ecs::core::component::component::Component;
pub use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
pub use crate::ecs::core::entity::entity::Entity;
pub use crate::ecs::core::resources::resource::Resource;

// ── Phase 22 — dynamic tags ──────────────────────────────────────────────────
pub use crate::ecs::core::component::component_registry::TagId;
pub use crate::ecs::core::component::hooks::HooksError;

// ── Hierarchies (parent-child) ───────────────────────────────────────────────
pub use crate::ecs::core::hierarchy::{ChildOf, Children};

// ── Relations (generic one-to-many) ──────────────────────────────────────────
pub use crate::ecs::core::component::observers::traversal::Toward;
pub use crate::ecs::core::relationship::{
    Relationship, RelationshipSourceCollection, RelationshipTarget,
};

// ── Relation-aware triggers / observers (edge observers + Down broadcast) ─────
pub use crate::ecs::core::component::observers::propagate::propagate;
pub use crate::ecs::core::component::observers::traversal::PropagationMode;
pub use crate::ecs::core::component::observers::trigger::{Trigger, TriggerContext, TriggerFn};
pub use crate::ecs::core::relationship::{Exclusive, OnLink, OnUnlink};

// ── Entity cloning (Feature 3) ───────────────────────────────────────────────
pub use crate::ecs::core::clone::{EntityCloner, EntityClonerBuilder};

// ── Prefab / instantiate (std-lib S7) ────────────────────────────────────────
pub use crate::ecs::core::clone::Prefab;

// ── Scheduling ───────────────────────────────────────────────────────────────
pub use crate::ecs::core::schedule::{
    Schedule, ScheduleBuilder, SystemConfig, SystemSet, in_state, on_enter, on_exit,
    on_transition, run_once,
};

// ── Queries ──────────────────────────────────────────────────────────────────
pub use crate::ecs::core::iters::query::Query;
// task #9: optional / OR query data (`Option<&T>` is std; `AnyOf` is new). The
// change-detection data views `Ref` / `Mut` ride along (Decision 6).
pub use crate::ecs::core::iters::query::{AnyOf, IsEnabled, Mut, Ref};
// EnableTag filters: per-row gates over the bitset storage backend.
pub use crate::ecs::core::iters::query::filter_enable::{Disabled, Enabled};
// Relation-aware DSL: the `Related<R, D>` join, the relation filters, and the
// transitive/wildcard traversal iterators.
pub use crate::ecs::core::iters::query::{
    AncestorsIter, DescendantsIter, HasRelation, NoRelation, Related, RelatedTo, SourcesIter,
    TargetsIter,
};

// ── System params ────────────────────────────────────────────────────────────
pub use crate::ecs::core::system::{Commands, EventReader, EventWriter, Local, Res, ResMut};

// ── States ───────────────────────────────────────────────────────────────────
pub use crate::ecs::core::state::{NextState, State, States};

// ── Thread pool ──────────────────────────────────────────────────────────────
pub use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Errors ───────────────────────────────────────────────────────────────────
pub use crate::ecs::error::{EcsError, EcsResult};
