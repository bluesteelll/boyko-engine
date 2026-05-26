//! `SendEventCommand<E>` — deferred event-dispatch command (Phase 9 EVT2).
//!
//! Wraps a `Command::apply` that forwards an event of type `E` to
//! [`EventDispatcher::send_event`] on the dispatcher's lane. Because the
//! `apply` path always runs on the dispatcher (under `&mut EcsMaster`),
//! `EventDispatcher::send_event` reads its TLS sentinel and routes the
//! write to lane `worker_count` — the reserved dispatcher lane (see plan
//! §2.8 EVT1 / §12.4).
//!
//! Failures from the inner `send_event` call (an unregistered event type
//! or a full lane) are swallowed: `Command::apply` returns `()`, and the
//! Phase 8d apply driver has no error channel. Callers wanting to surface
//! the error must use [`EcsMaster::events_mut`] directly.
//!
//! [`EventDispatcher::send_event`]: crate::ecs::core::events::event_dispatcher::EventDispatcher::send_event
//! [`EcsMaster::events_mut`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::events_mut

// Mirrors the gate on `spawn_command.rs`: the type only becomes "live" once
// `Commands::send_event` (Phase 9 Step 18) is exercised from a downstream
// crate or test; without consumers the lib build sees it as dead.
#![allow(dead_code)]

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::events::event::Event;

/// Deferred "send event `E` on the dispatcher lane" command.
///
/// Constructed via [`Commands::send_event`] (Phase 9 EVT2). The event is
/// moved into the queue's bytes by [`CommandQueue::push`]; on apply, the
/// dispatcher unpacks the event and forwards it via
/// [`EventDispatcher::send_event`].
///
/// # Send / Sync
///
/// `E: Event` already bounds `E: Send + Sync + 'static`, so the auto-trait
/// derivation is the same as for the `Bundle` case; no `unsafe impl` is
/// needed.
///
/// [`Commands::send_event`]: crate::ecs::core::system::params::commands::Commands::send_event
/// [`CommandQueue::push`]: crate::ecs::core::commands::command_queue::CommandQueue::push
/// [`EventDispatcher::send_event`]: crate::ecs::core::events::event_dispatcher::EventDispatcher::send_event
pub(crate) struct SendEventCommand<E: Event> {
    /// The event payload, moved into the queue at enqueue time.
    pub(crate) event: E,
}

impl<E: Event> Command for SendEventCommand<E> {
    /// Forwards the event to `world.events().send_event::<E>(...)`. Errors
    /// are intentionally dropped — the apply driver has no return channel
    /// for them, and the dispatcher lane is large enough for the system's
    /// per-frame burst by design (preregister with `worker_count + 1`
    /// lanes).
    #[inline]
    fn apply(self, world: &mut EcsMaster) {
        // `world.events()` returns `&EventDispatcher`; `send_event` uses
        // `&self` because the per-lane writer state is interior-mutable
        // (atomic counters + UnsafeCell on the write buffer). The TLS lane
        // routing runs inside `send_event` and lands on lane
        // `default_thread_count - 1` here (the dispatcher's reserved lane).
        let _ = world.events().send_event::<E>(self.event);
    }
}
