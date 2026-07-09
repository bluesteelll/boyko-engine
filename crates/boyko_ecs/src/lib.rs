pub mod ecs;
pub mod prelude;

pub use ecs::core::app::{App, AppExit, Plugin, Plugins};
pub use ecs::error::{EcsError, EcsResult};