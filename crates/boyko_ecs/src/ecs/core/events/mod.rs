// Module name mirrors the public `Event` trait; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod event;
pub mod erased_buffer;
pub mod participants;
pub mod parameters;
pub mod event_registry;
pub mod event_config;
pub mod event_buffer;
pub mod event_dispatcher;