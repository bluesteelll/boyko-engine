//! Event dispatch proxy surface on [`EcsMaster`] (mechanical split).
//!
//! Preregistration, send, read, and per-frame `update_events`. Extracted
//! verbatim from `ecs_master.rs`.

use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_config::EventConfig;
use crate::ecs::error::EcsResult;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    /// Preregisters event type `E` with a custom config.
    ///
    /// Must be called before the first `send_event::<E>` or `events_of::<E>`.
    /// All write lanes and the reader buffer are allocated here; no allocation
    /// occurs during steady-state `send_event` or `update_events`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    #[inline]
    pub fn preregister_event<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
        self.events.preregister::<E>(cfg)
    }

    /// Preregisters event type `E` with default capacity and the dispatcher's
    /// validated `default_thread_count`.
    ///
    /// Equivalent to calling [`preregister_event`] with
    /// `EventConfig::default_for(self.events.default_thread_count())`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    ///
    /// [`preregister_event`]: EcsMaster::preregister_event
    #[inline]
    pub fn preregister_event_default<E: Event>(&mut self) -> EcsResult<()> {
        let cfg = EventConfig::default_for(self.events.default_thread_count())
            .expect("invariant: default_thread_count was validated at EventDispatcher::new");
        self.events.preregister::<E>(cfg)
    }

    /// Sends a single event of type `E` to the lane for `thread_index`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::send`].
    #[inline]
    pub fn send_event<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
        self.events.send::<E>(thread_index, event)
    }

    /// Returns the slice of events of type `E` from the previous frame.
    ///
    /// Returns an empty slice if `E` was not registered or if no events were
    /// sent last frame. Slice remains valid until the next `update_events` call.
    #[inline]
    pub fn events_of<E: Event>(&self) -> &[E] {
        self.events.events::<E>()
    }

    /// Advances the frame counter and flattens write lanes into reader buffers.
    ///
    /// Must be called once per frame. After this call, `events_of::<E>()` returns
    /// the events sent during the frame that just ended.
    #[inline]
    pub fn update_events(&mut self) {
        self.events.update_events();
    }

}
