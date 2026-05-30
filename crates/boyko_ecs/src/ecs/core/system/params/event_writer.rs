//! `EventWriter<'s, E>` — typed event-send `SystemParam` (Phase 12).
//!
//! Hot-path target: ≤ 5 ns per `send` (cached `NonNull<EventBuffer<E>>` +
//! cached `thread_count`, no `OnceLock` / slot-bounds work per call).
//!
//! See Phase 12 plan §2.1 (EW invariants), §5.1 (`EventWriterState<E>`),
//! §6.1 (`EventWriter::send`), §6.4 (SystemParam impl).

// `EventWriter` is exposed through `boyko_ecs::ecs::core::system::params`
// but no consumer inside the lib build calls `send` / `send_many` /
// `send_default` yet — Phase 12 tests (`tests/phase12_events_systemparam.rs`)
// exercise the public surface end-to-end. Mirror the suppression used by
// `commands.rs` until the first cross-module consumer lands.
#![allow(dead_code)]

use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::events::event::{Event, EventId};
use crate::ecs::core::events::event_buffer::EventBuffer;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::event_not_preregistered_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::error::EcsResult;

// ── EventWriterState ─────────────────────────────────────────────────────────

/// Per-system state for [`EventWriter<E>`].
///
/// 24 B total — fits in 3/8 of a cache line, typically co-located with other
/// param states in a tuple's storage. See Phase 12 plan §5.1 / §11.1.
///
/// # Field order (Phase 12 C1 + W1 resolution)
///
/// 1. `buffer_ptr` — cached `NonNull<EventBuffer<E>>` from
///    [`EventDispatcher::buffer_ptr`]. Stable for the dispatcher's lifetime
///    (EXT4 / EXT5). SB/TB-clean across `update_events` borrows because the
///    buffer lives in its own heap allocation, not inside `EventDispatcher`.
/// 2. `thread_count` — cached from `EventBuffer<E>::thread_count` at init.
///    Frozen post-preregister (EW8) so the lane router does not have to
///    chase through the slot on every call.
/// 3. `event_id` — cached `EventId`. Diagnostics-only on the hot path; kept
///    for future use (e.g. tracing the per-type lane on overflow).
///
/// [`EventDispatcher::buffer_ptr`]: crate::ecs::core::events::event_dispatcher::EventDispatcher
#[repr(C)]
pub struct EventWriterState<E: Event> {
    /// Heap-stable buffer pointer (Phase 12 EXT4 / EXT5).
    buffer_ptr: NonNull<EventBuffer<E>>,
    /// `EventBuffer<E>::thread_count` cached at init. Drives lane routing
    /// via `current_worker_id_or_dispatcher_lane(thread_count - 1)`.
    thread_count: u32,
    /// Padding to 8-byte boundary for the trailing `EventId` (`u64`).
    _pad: u32,
    /// Cached event id; not consulted on the hot path.
    event_id: EventId,
    /// Type binding without forcing `E: Send + Sync` onto the state.
    /// `fn(&E)` keeps `E` invariant but yields `Send + Sync` regardless of `E`.
    _marker: PhantomData<fn(&E)>,
}

// SAFETY (Phase 12 SEND-EV1):
//   - `NonNull<EventBuffer<E>>`: the pointee is `Sync` per Phase 6 SEND4
//     (per-lane writer exclusivity + `&mut self` swap barrier). The raw
//     pointer itself crossing thread boundaries when the Phase 9 scheduler
//     migrates the system is safe because the heap address is stable and
//     provenance is preserved (derived from `Box::into_raw` at preregister).
//   - `thread_count`, `event_id`: primitive integers; `Send + Sync` trivially.
//   - `PhantomData<fn(&E)>` is `Send + Sync` regardless of `E`.
unsafe impl<E: Event> Send for EventWriterState<E> {}
unsafe impl<E: Event> Sync for EventWriterState<E> {}

// ── EventWriter ──────────────────────────────────────────────────────────────

/// Typed event-send wrapper for one system invocation (Phase 12).
///
/// Carries a borrow of the [`EventWriterState<E>`] cached in the system's
/// state slot. Per-call cost: TLS lane resolve + per-lane buffer write + one
/// `Relaxed` `fetch_add` on the per-type counter — see plan §10.1.
///
/// # Lifetime
///
/// `'s` is the state scope (the system's stored state outlives the per-call
/// invocation).
#[repr(transparent)]
pub struct EventWriter<'s, E: Event> {
    state: &'s mut EventWriterState<E>,
}

impl<E: Event> EventWriter<'_, E> {
    /// Sends a single event to this thread's per-lane buffer (EVT1 routing).
    ///
    /// Cost (hot cache, release build): ~5 ns. See plan §10.1.
    ///
    /// # Out-of-scheduler use (Phase 12 EW-NEW)
    ///
    /// Debug builds `debug_assert!(is_in_system_run())`. Main-thread / FFI
    /// callers must use [`EcsMaster::events`]`().send_event::<E>(...)` instead;
    /// the unattached-thread fallback there routes safely.
    ///
    /// # Errors
    ///
    /// Forwards `EventBufferFull` from the per-lane buffer on overflow.
    ///
    /// [`EcsMaster::events`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::events
    #[inline]
    pub fn send(&mut self, event: E) -> EcsResult<()> {
        // Phase 12 EW-NEW / W2: catches main-thread misuse in debug builds.
        debug_assert!(
            boyko_threadpool::is_in_system_run(),
            "EventWriter::send called outside a scheduled system body — \
             use EcsMaster::events().send_event::<E>(...) for main-thread / FFI callers",
        );

        // SAFETY (Phase 12 EW2 / EXT5 / C1 resolution):
        //   - `state.buffer_ptr` is heap-stable: derived from `Box::into_raw`
        //     at preregister; its address never moves for the dispatcher's
        //     lifetime.
        //   - The pointee `EventBuffer<E>` is `Sync` per Phase 6 SEND4.
        //   - Concurrent per-lane writes from sibling worker threads target
        //     disjoint cache lines (EVT1) and disjoint heap allocations
        //     (per-lane `write_buf` Box).
        //   - `&mut EventDispatcher` (the only thing that could invalidate
        //     the borrow) is not in flight: `update_events` and `Schedule`
        //     are mutually exclusive (ER5 / SCH7).
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };

        let lane = boyko_threadpool::current_worker_id_or_dispatcher_lane(
            self.state.thread_count.saturating_sub(1),
        );

        // `send_one` bumps `frame_event_count` internally on success
        // (Phase 12 EXT6); the CachePadded counter sits on its own cache
        // line so the Relaxed fetch_add does not contaminate reader-side
        // fields (C3).
        buf.send_one(lane, event)
    }

    /// Sends a batch of events to this thread's per-lane buffer.
    ///
    /// All-or-nothing on overflow (Phase 6 parity). One `Relaxed` fetch_add
    /// on the per-type counter per batch.
    ///
    /// # Errors
    ///
    /// Forwards `EventBufferFull` from the per-lane buffer when the batch
    /// would exceed remaining capacity.
    #[inline]
    pub fn send_many<I>(&mut self, events: I) -> EcsResult<()>
    where
        I: ExactSizeIterator<Item = E>,
    {
        debug_assert!(
            boyko_threadpool::is_in_system_run(),
            "EventWriter::send_many called outside a scheduled system body",
        );
        // SAFETY: same as `send` above.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let lane = boyko_threadpool::current_worker_id_or_dispatcher_lane(
            self.state.thread_count.saturating_sub(1),
        );
        buf.send_many(lane, events)
    }

    /// Sends `E::default()` to this thread's per-lane buffer. Convenience
    /// wrapper around [`send`](Self::send).
    #[inline]
    pub fn send_default(&mut self) -> EcsResult<()>
    where
        E: Default,
    {
        self.send(E::default())
    }
}

// SAFETY (Phase 12 §2.1 EW1-EW8, §6.4):
//   - SP1: `init_access` declares NO access — Option A (events live outside
//     the conflict graph). Per-lane EVT1 discipline makes concurrent
//     `EventWriter<E>` invocations sound; cross-system aliasing is avoided
//     by the disjoint-lanes invariant.
//   - SP2: `get_param` simply re-binds the borrow of `EventWriterState<E>`;
//     the borrow checker enforces uniqueness via `&'s mut State`.
//   - SP4: `init_state` reads `R::event_id()` and `EventDispatcher::buffer_ptr`
//     — both are pure reads with respect to archetype/resource registries.
unsafe impl<E: Event> SystemParam for EventWriter<'_, E> {
    type State = EventWriterState<E>;
    type Item<'w, 's> = EventWriter<'s, E>;

    fn init_state(world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        let event_id = E::event_id();
        let buffer_ptr = world
            .events()
            .buffer_ptr::<E>()
            .unwrap_or_else(|| event_not_preregistered_panic::<E>());
        // SAFETY (EXT5): `buffer_ptr` is heap-stable; reading `thread_count`
        //   is a plain field load on `&EventBuffer<E>`. The borrow is bounded
        //   by this expression and does not escape.
        let thread_count = unsafe { buffer_ptr.as_ref() }.thread_count;
        EventWriterState {
            buffer_ptr,
            thread_count,
            _pad: 0,
            event_id,
            _marker: PhantomData,
        }
    }

    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // Phase 12 EW5 / Q2 Option A: events stay OUTSIDE the conflict graph.
        // Per-lane EVT1 discipline guarantees correctness; in-debug misuse
        // (out-of-scheduler call) is caught by `EventWriter::send`'s
        // `debug_assert!(is_in_system_run())` (EW-NEW).
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        EventWriter { state }
    }
}

// Phase 12 §11.1 — compile-time size pin. Bumps any future change to the
// state shape (currently 24 B). `LayoutAssertEvent` is internal to the
// `event_buffer` module; we use a concrete public stub here.
#[doc(hidden)]
struct _SizeAssertEvent {
    _data: u32,
}
#[derive(Copy, Clone)]
struct _SAParticipants;
impl crate::ecs::core::events::participants::participants::Participants for _SAParticipants {
    fn participant_count() -> usize {
        0
    }
    fn participant_info()
    -> &'static [crate::ecs::core::events::participants::participants::ParticipantInfo] {
        &[]
    }
}
#[derive(Copy, Clone)]
struct _SAParameters;
impl crate::ecs::core::events::parameters::parameters::Parameters for _SAParameters {}
impl crate::ecs::core::events::event::Event for _SizeAssertEvent {
    type Participants = _SAParticipants;
    type Parameters = _SAParameters;
    fn event_id() -> EventId {
        u64::MAX
    }
    fn event_name() -> &'static str {
        "__EventWriterSizeAssert"
    }
    fn new(_: _SAParticipants, _: _SAParameters) -> Self {
        _SizeAssertEvent { _data: 0 }
    }
    fn participants(&self) -> &_SAParticipants {
        unimplemented!()
    }
    fn participants_mut(&mut self) -> &mut _SAParticipants {
        unimplemented!()
    }
    fn parameters(&self) -> &_SAParameters {
        unimplemented!()
    }
    fn parameters_mut(&mut self) -> &mut _SAParameters {
        unimplemented!()
    }
}

// `EventWriterState` holds a pointer-width state (a `usize` + a `NonNull`
// buffer pointer), and `EventWriter` wraps a single pointer-width handle, so
// these size/align figures encode the 64-bit ABI. Gated to 64-bit (the engine's
// supported platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::size_of::<EventWriterState<_SizeAssertEvent>>() == 24,
    "EventWriterState<E> must be 24 B (Phase 12 §11.1)",
);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::align_of::<EventWriterState<_SizeAssertEvent>>() == 8,
    "EventWriterState<E> must be 8-byte aligned (Phase 12 §11.1)",
);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::size_of::<EventWriter<'_, _SizeAssertEvent>>() == 8,
    "EventWriter<'s, E> must be 8 B (Phase 12 §11.1)",
);

// `event_id` is read at init time and held for future diagnostic use (the
// per-type tracing hook in Phase 9 EVT4 will surface it on overflow paths).
// Until that consumer lands the field is intentionally unused in the hot
// path; suppress the unused-field warning rather than dropping the field
// from the state (changing the state shape is a wider commitment).
#[allow(dead_code)]
const _: fn() = || {
    // Touch the field at compile time so future field renames trip here.
    fn assert_field<E: Event>(s: &EventWriterState<E>) -> EventId {
        s.event_id
    }
    let _ = assert_field::<_SizeAssertEvent>;
};
