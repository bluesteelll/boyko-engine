//! System scaffolding — `Access`, `SystemMeta`, `FilteredAccessSet`,
//! `UnsafeEcsCell`, the `SystemParam` trait + tuple impls, the `System`
//! trait, and the `FunctionSystem` / `IntoSystem` adapters (Phase 8c).
//!
//! See the Phase 8a plan §3 / §4 / §7 / §8 / §9 / §13 and the Phase 8c+8d
//! plan §3..§9 for the design.

pub mod access;
pub mod dispatcher_token;
pub mod exclusive_function_system;
pub mod filtered_access_set;
pub mod function_system;
mod function_system_impls;
pub mod gpu_intent;
pub mod into_system;
pub(crate) mod params;
#[allow(clippy::module_inception)]
pub mod system;
pub(crate) mod system_kind;
pub mod system_meta;
pub mod system_param;
pub mod unsafe_ecs_cell;

pub use access::Access;
pub use dispatcher_token::DispatcherToken;
pub use exclusive_function_system::ExclusiveFunctionSystem;
pub use filtered_access_set::{AccessConflict, ConflictKind, FilteredAccessSet};
pub use gpu_intent::{GpuAccess, GpuAccessIntent, GpuStage, GpuTouch, MAX_GPU_TOUCHES};
pub use function_system::{FunctionSystem, SystemParamFunction};
pub use into_system::{ExclusiveSystemMarker, IntoSystem, IsFunctionSystem};
pub use params::{
    Commands, EventIter, EventReader, EventReaderState, EventWriter, EventWriterState, Local,
    MAX_SYSTEM_PARAM_ARITY, NonSendRes, NonSendResMut, NonSendResMutState, NonSendResState, Res,
    ResMut, ResMutState, ResState,
};
pub use system::System;
pub use system_meta::SystemMeta;
pub use system_param::SystemParam;
pub use unsafe_ecs_cell::UnsafeEcsCell;
