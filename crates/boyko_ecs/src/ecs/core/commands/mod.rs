//! `Commands` — deferred world mutation buffer.
//!
//! Phase 8d Step 6 (foundation) + Phase 11 (per-entity chaining +
//! despawn + migration). The Phase 11 changeset:
//!
//! * Renamed Phase 8.5's `SpawnCommand<B>` to [`SpawnAtCommand<B>`]
//!   (plan Q9). The new shape carries a pre-allocated [`Entity`] so
//!   `Commands::spawn(bundle).id()` can return synchronously.
//! * Added [`DespawnCommand`], [`InsertCommand<B>`], [`RemoveCommand<C>`]
//!   covering the per-entity command surface (plan §6).
//! * Added the [`migration_helpers`] module — shared archetype-migration
//!   scaffolding for `InsertCommand` / `RemoveCommand`.
//!
//! The Phase 8d primitives (`Command` trait, `CommandQueue`,
//! `consume_and_drop_glue`) survive unchanged.

pub mod command;
pub mod command_queue;
pub mod despawn_command;
pub mod enable_tag_commands;
pub mod insert_command;
pub mod migration_helpers;
pub mod observe_entity_command;
pub mod remove_command;
pub mod send_event_command;
pub mod spawn_at_command;
pub mod spawn_batch_command;
pub mod spawn_batch_iter;
pub mod tag_commands;

pub use command::Command;
// `CommandQueue` is promoted to `pub` because it is the `State` associated
// type of the public `Commands<'s>` SystemParam (Step 7 — would otherwise
// trip E0446 "crate-private type in public interface"). The struct's
// surface stays intentionally minimal — only `new()`, `push`, `apply` and
// the field-level `pub(crate)` accessors are reachable; the internal
// `RawCommandQueue` twin remains private to this module.
pub use command_queue::CommandQueue;
// Phase 11 command types stay `pub(crate)` — users go through the
// `Commands` / `EntityCommands` enqueue methods, which materialise the
// payloads internally.
#[allow(unused_imports)]
pub(crate) use despawn_command::DespawnCommand;
#[allow(unused_imports)]
pub(crate) use insert_command::InsertCommand;
#[allow(unused_imports)]
pub(crate) use remove_command::RemoveCommand;
// `SendEventCommand<E>` follows the same `pub(crate)` discipline — users
// go through `Commands::send_event`, which enqueues it.
#[allow(unused_imports)]
pub(crate) use send_event_command::SendEventCommand;
#[allow(unused_imports)]
pub(crate) use spawn_at_command::SpawnAtCommand;
#[allow(unused_imports)]
pub(crate) use spawn_batch_command::SpawnBatchCommand;
// Phase 22 tag commands follow the same `pub(crate)` discipline — users go
// through `EntityCommands::add_tag` / `remove_tag`.
#[allow(unused_imports)]
pub(crate) use tag_commands::{AddTagCommand, RemoveTagCommand};
// EnableTag Step 9 deferred toggle command follows the same `pub(crate)`
// discipline — users go through `EntityCommands::enable` / `disable`.
#[allow(unused_imports)]
pub(crate) use enable_tag_commands::EnableTagCommand;
// Phase 12.5 Opt-A2: `SpawnBatchIter` is the user-facing return type of
// `Commands::spawn_batch`. Promoted to `pub` so user code can name it in
// function signatures (without the bundle-iterator type leaking per W5).
pub use spawn_batch_iter::SpawnBatchIter;
