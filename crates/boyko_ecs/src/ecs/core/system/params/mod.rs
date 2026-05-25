//! Concrete `SystemParam` implementations.
//!
//! Hosts tuple impls (Step 6) and — once they land — the `Res<R>` /
//! `ResMut<R>` newtypes (Step 7). The submodule split mirrors Bevy's
//! `bevy_ecs::system::system_param` layout.

pub(crate) mod tuple_impl;

// `res.rs` and `resmut.rs` arrive in Step 7.

pub use tuple_impl::MAX_SYSTEM_PARAM_ARITY;
