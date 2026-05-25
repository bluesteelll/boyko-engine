//! System scaffolding — `Access`, `SystemMeta`, `FilteredAccessSet`, and
//! `UnsafeEcsCell`. See the Phase 8a plan §3 / §4 / §9 for the design.
//!
//! This module hosts the **non-trait** scaffolding for the `SystemParam`
//! ecosystem. The `SystemParam` trait itself, the param newtypes (`Res`,
//! `ResMut`), and the `System` trait land in later steps (6, 7, 8). Their
//! `pub mod` declarations are intentionally absent here so the parallel
//! work on those modules does not produce merge conflicts on `mod.rs`.

pub mod access;
pub mod filtered_access_set;
pub mod system_meta;
pub mod unsafe_ecs_cell;

pub use access::Access;
pub use filtered_access_set::{AccessConflict, ConflictKind, FilteredAccessSet};
pub use system_meta::SystemMeta;
pub use unsafe_ecs_cell::UnsafeEcsCell;
