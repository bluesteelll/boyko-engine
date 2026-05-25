//! System scaffolding — `Access`, `SystemMeta`, `FilteredAccessSet`,
//! `UnsafeEcsCell`, and the `SystemParam` trait + tuple impls.
//!
//! See the Phase 8a plan §3 / §4 / §7 / §9 / §13 for the design. The
//! `Res<R>` / `ResMut<R>` newtypes land in Step 7 (`params::res`); the
//! `System` trait + `FnOnceSystem` follow in Step 8.

pub mod access;
pub mod filtered_access_set;
pub(crate) mod params;
pub mod system_meta;
pub mod system_param;
pub mod unsafe_ecs_cell;

pub use access::Access;
pub use filtered_access_set::{AccessConflict, ConflictKind, FilteredAccessSet};
pub use params::{MAX_SYSTEM_PARAM_ARITY, Res, ResMut, ResMutState, ResState};
pub use system_meta::SystemMeta;
pub use system_param::SystemParam;
pub use unsafe_ecs_cell::UnsafeEcsCell;
