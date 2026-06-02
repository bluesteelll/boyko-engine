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

// ── World + core entity/component model ──────────────────────────────────────
pub use crate::ecs::core::bundle::Bundle;
pub use crate::ecs::core::component::component::Component;
pub use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
pub use crate::ecs::core::entity::entity::Entity;
pub use crate::ecs::core::resources::resource::Resource;

// ── Scheduling ───────────────────────────────────────────────────────────────
pub use crate::ecs::core::schedule::{
    Schedule, ScheduleBuilder, SystemConfig, SystemSet, in_state, on_enter, on_exit,
    on_transition, run_once,
};

// ── Queries ──────────────────────────────────────────────────────────────────
pub use crate::ecs::core::iters::query::Query;

// ── System params ────────────────────────────────────────────────────────────
pub use crate::ecs::core::system::{Commands, EventReader, EventWriter, Local, Res, ResMut};

// ── States ───────────────────────────────────────────────────────────────────
pub use crate::ecs::core::state::{NextState, State, States};

// ── Thread pool ──────────────────────────────────────────────────────────────
pub use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// ── Errors ───────────────────────────────────────────────────────────────────
pub use crate::ecs::error::{EcsError, EcsResult};
