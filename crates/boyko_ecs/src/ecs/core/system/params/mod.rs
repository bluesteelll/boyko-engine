//! Concrete `SystemParam` implementations.
//!
//! Hosts tuple impls (Step 6), the `Res<R>` / `ResMut<R>` newtypes
//! (Step 7), and the shared cold-path diagnostic helpers consumed by both.
//! The submodule split mirrors Bevy's `bevy_ecs::system::system_param`
//! layout.

pub(crate) mod diagnostics;
pub mod res;
pub mod resmut;
pub(crate) mod tuple_impl;

pub use res::{Res, ResState};
pub use resmut::{ResMut, ResMutState};
pub use tuple_impl::MAX_SYSTEM_PARAM_ARITY;
