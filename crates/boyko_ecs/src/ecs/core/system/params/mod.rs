//! Concrete `SystemParam` implementations.
//!
//! Hosts tuple impls (Step 6), the `Res<R>` / `ResMut<R>` newtypes
//! (Step 7), and the shared cold-path diagnostic helpers consumed by both.
//! The submodule split mirrors Bevy's `bevy_ecs::system::system_param`
//! layout.

pub mod commands;
pub(crate) mod diagnostics;
pub mod entity_commands;
pub mod entity_counter;
pub mod event_reader;
pub mod event_writer;
pub mod local;
pub mod nonsend_res;
pub mod nonsend_resmut;
pub mod res;
pub mod resmut;
pub(crate) mod tuple_impl;

// `Commands` re-export is dead-code until Phase 8c Step 4 wires it into
// `FunctionSystem`; the SystemParam impl + in-file tests already exercise
// the full path. Suppress until the public consumer lands.
#[allow(unused_imports)]
pub use commands::Commands;
// Phase 11 (Wave C): `EntityCommands<'a, 's>` is the chainable per-entity
// handle returned by `Commands::spawn` / `Commands::entity`. Re-exported
// for end-user code that destructures the handle into a helper signature.
// Internal lib code does not import this name (consumers reach for the
// methods through the handle returned by `Commands::spawn`), so the
// re-export is marked `#[allow(unused_imports)]` until the first
// integration test that names the type by hand lands.
#[allow(unused_imports)]
pub use entity_commands::EntityCommands;
// Phase 11 (Round 3 C-N1): `EntityCounter` is the worker-safe projection
// of `EntityMaster::next_entity_id`. Re-exported because users may name
// the type when accepting `Commands<'_>` and reaching for the underlying
// counter through documentation; the public surface is `reserve_entity`.
// Same `#[allow(unused_imports)]` rationale as `EntityCommands`.
#[allow(unused_imports)]
pub use entity_counter::EntityCounter;
// Phase 12: `EventReader<'s, E>` / `EventWriter<'s, E>` re-exports. Same
// `#[allow(unused_imports)]` rationale as `Commands` / `EntityCommands` —
// the SystemParam impl + tests exercise the full path, but the lib build
// has no cross-module consumer until Phase 9 EVT4 wiring lands.
#[allow(unused_imports)]
pub use event_reader::{EventIter, EventReader, EventReaderState};
#[allow(unused_imports)]
pub use event_writer::{EventWriter, EventWriterState};
// Phase 13: `Local<'s, T>` per-system private-state re-export. Same
// `#[allow(unused_imports)]` rationale as `Commands` / `EventReader`.
#[allow(unused_imports)]
pub use local::Local;
// Phase 4 Seam 2: `NonSendRes` / `NonSendResMut` re-exports. Same
// `#[allow(unused_imports)]` rationale as the other params — the SystemParam
// impls + in-file tests exercise the full path; cross-module lib consumers
// land with Phase 5.
#[allow(unused_imports)]
pub use nonsend_res::{NonSendRes, NonSendResState};
#[allow(unused_imports)]
pub use nonsend_resmut::{NonSendResMut, NonSendResMutState};
pub use res::{Res, ResState};
pub use resmut::{ResMut, ResMutState};
pub use tuple_impl::MAX_SYSTEM_PARAM_ARITY;
