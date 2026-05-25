//! System scaffolding — `Access`, `SystemMeta`, `FilteredAccessSet`,
//! `UnsafeEcsCell`, the `SystemParam` trait + tuple impls, and the Step 8
//! `System` trait + `FnOnceSystem` adapter.
//!
//! See the Phase 8a plan §3 / §4 / §7 / §8 / §9 / §13 for the design.

pub mod access;
pub mod filtered_access_set;
pub mod fn_once_system;
pub(crate) mod params;
#[allow(clippy::module_inception)]
pub mod system;
pub mod system_meta;
pub mod system_param;
pub mod unsafe_ecs_cell;

pub use access::Access;
pub use filtered_access_set::{AccessConflict, ConflictKind, FilteredAccessSet};
pub use fn_once_system::FnOnceSystem;
pub use params::{MAX_SYSTEM_PARAM_ARITY, Res, ResMut, ResMutState, ResState};
pub use system::System;
pub use system_meta::SystemMeta;
pub use system_param::SystemParam;
pub use unsafe_ecs_cell::UnsafeEcsCell;
