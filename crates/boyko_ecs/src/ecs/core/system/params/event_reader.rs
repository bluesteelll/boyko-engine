//! `EventReader<'s, E>` — typed event-read `SystemParam` (Phase 12).
//!
//! Hot-path targets:
//! - `read()` empty case: ≤ 3 ns (two `Acquire` loads + 64-bit compare).
//! - `read()` per element: ≤ 2 ns (`get_unchecked` + cursor `+1`).
//! - `is_empty()` / `len()`: O(1) atomic loads, no iteration.
//!
//! See Phase 12 plan §2.2 (ER invariants), §5.2 (`EventReaderState<E>`),
//! §6.2 (`EventReader::read`), §6.3 (`EventIter`), §6.4 (SystemParam impl).

// `EventReader` is exposed through `boyko_ecs::ecs::core::system::params`
// but no consumer inside the lib build calls `read` / `is_empty` / `len`
// yet — Phase 12 tests (`tests/phase12_events_systemparam.rs`) exercise the
// public surface end-to-end. Mirror the suppression used by `commands.rs`.
#![allow(dead_code)]

use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_buffer::EventBuffer;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::event_not_preregistered_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

// ── EventReaderState ─────────────────────────────────────────────────────────

/// Per-system state for [`EventReader<E>`].
///
/// 24 B total. See Phase 12 plan §5.2 / §11.1. Round 2 O1 fix dropped the
/// Round 1 `_pad: u64` (no concrete consumer); reintroduce later if a
/// per-reader flag emerges.
///
/// # Fields
///
/// - `buffer_ptr` — cached `NonNull<EventBuffer<E>>` from
///   [`EventDispatcher::buffer_ptr`]. Stable for the dispatcher's lifetime
///   (EXT4 / EXT5).
/// - `last_event_count` — per-(system, E) cursor. Persists across frames;
///   never reset. Bevy-style cursor (Phase 12 Q1).
/// - `thread_count` — symmetric with [`EventWriterState`]; reserved for
///   future per-thread read facilities. Currently unused on the hot path.
///
/// [`EventDispatcher::buffer_ptr`]: crate::ecs::core::events::event_dispatcher::EventDispatcher
/// [`EventWriterState`]: super::event_writer::EventWriterState
#[repr(C)]
pub struct EventReaderState<E: Event> {
    /// Heap-stable buffer pointer (Phase 12 EXT4 / EXT5).
    buffer_ptr: NonNull<EventBuffer<E>>,
    /// Per-(system, E) cursor in send-count space. `Bevy` parity — survives
    /// across frames and is advanced by [`EventIter::drop`] on each read.
    last_event_count: u64,
    /// Cached `EventBuffer<E>::thread_count` at init. Kept symmetric with
    /// `EventWriterState<E>` for shape consistency; not read on hot path.
    thread_count: u32,
    /// Padding to 8-byte boundary.
    _pad: u32,
    /// Type binding without forcing `E: Send + Sync` onto the state.
    _marker: PhantomData<fn(&E)>,
}

// SAFETY (Phase 12 SEND-EV2): same rationale as `EventWriterState`.
//   - `NonNull<EventBuffer<E>>` is heap-stable; pointee is `Sync`.
//   - `last_event_count`, `thread_count`: primitive integers.
//   - `PhantomData<fn(&E)>` is `Send + Sync` regardless of `E`.
unsafe impl<E: Event> Send for EventReaderState<E> {}
unsafe impl<E: Event> Sync for EventReaderState<E> {}

// ── EventReader ──────────────────────────────────────────────────────────────

/// Typed event-read wrapper for one system invocation (Phase 12).
///
/// Carries a borrow of the [`EventReaderState<E>`] cached in the system's
/// state slot. The reader observes events that were sent in a previous frame
/// and made visible by the most recent `update_events` call (next-frame
/// visibility, Phase 6 Model B).
///
/// # Lifetime
///
/// `'s` is the state scope. The `'w` lifetime was dropped per Phase 12 OQ1
/// — the returned [`EventIter`] borrows through `'s` via the cursor
/// back-pointer (which itself borrows from `EventReaderState<E>`).
pub struct EventReader<'s, E: Event> {
    state: &'s mut EventReaderState<E>,
}

impl<'s, E: Event> EventReader<'s, E> {
    /// Returns an iterator over unread events.
    ///
    /// On drop, the cursor advances by the number of events yielded so far
    /// (Phase 12 ER3). Mid-iteration `break` is supported; remaining events
    /// become visible on the next `read()`.
    ///
    /// # Cost (hot path)
    ///
    /// - Empty case: ~3 ns (two `Acquire` loads on `start_event_count` /
    ///   `reader_len` + arithmetic).
    /// - Per element: ~2 ns (`get_unchecked` + cursor `+1`).
    ///
    /// `frame_event_count` is NOT loaded here (Phase 12 C2 resolution).
    /// Slice math depends only on `cursor`, `start_event_count`, `reader_len`.
    /// Readers observe only post-swap events; in-flight writes are invisible
    /// by Phase 9 ER5.
    #[inline]
    pub fn read(&mut self) -> EventIter<'_, E> {
        debug_assert!(
            boyko_threadpool::is_in_system_run(),
            "EventReader::read called outside a scheduled system body",
        );

        // SAFETY (Phase 12 ER2 / C1):
        //   - `state.buffer_ptr` is heap-stable; pointee is `Sync`.
        //   - Read-only access to `reader_buf` is sound concurrently with
        //     per-lane writes (disjoint allocations) and other readers
        //     (shared `&` is the same model as `EventDispatcher::events`).
        //   - `&mut EventDispatcher` (i.e. `update_events`) is mutually
        //     exclusive with the current `Schedule::run` window (ER5 / SCH7).
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };

        // ER2: two Acquire loads. Pair with the Release stores in
        // `swap_and_flatten` (start_event_count then reader_len). Loading
        // `start_event_count` first matches the store order.
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;

        let cursor = self.state.last_event_count;
        let (start_offset, missed) = if cursor < start_count {
            // ER7: reader missed at least one full swap. Iterate the whole
            // visible buffer; report the gap via `EventIter::missed`.
            (0u64, start_count - cursor)
        } else {
            (cursor - start_count, 0)
        };

        // Clamp to reader_len consistently with `len()` / `is_empty()` (OQ2).
        let visible_len = reader_len.saturating_sub(start_offset);

        // SAFETY (Phase 12 ER2, Phase 6 U11):
        //   - `reader_buf[..reader_len]` is initialised by `swap_and_flatten`
        //     (Phase 6 invariant).
        //   - `start_offset + visible_len <= reader_len` (clamped above).
        //   - `MaybeUninit<T>` and `T` share layout (`repr(transparent)`),
        //     so the `.cast::<E>()` is valid.
        //   - `from_raw_parts` requirements: pointer non-null + aligned
        //     (Box-allocated for `E`); slice length ≤ isize::MAX (lane
        //     capacity is u32-bounded, well below).
        let slice: &[E] = unsafe {
            let ptr: *const E = (*buf.reader_buf).as_ptr().cast::<E>();
            core::slice::from_raw_parts(ptr.add(start_offset as usize), visible_len as usize)
        };

        EventIter {
            slice,
            consumed: 0,
            cursor_state: &mut self.state.last_event_count,
            // The iterator's cursor base is `start_count + start_offset`:
            // after consuming N elements the new cursor is
            // `start_count + start_offset + N`, which is exactly the next
            // unread send-count value.
            start_count: start_count + start_offset,
            missed,
        }
    }

    /// Returns `true` if there are no unread post-swap events.
    ///
    /// Consistent with [`len()`](Self::len) — both clamp to `reader_len`
    /// (Phase 12 OQ2 resolution). Does NOT load `frame_event_count`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        debug_assert!(
            boyko_threadpool::is_in_system_run(),
            "EventReader::is_empty called outside a scheduled system body",
        );
        // SAFETY: see `read()`.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;
        let cursor = self.state.last_event_count;
        let start_offset = cursor.saturating_sub(start_count);
        start_offset >= reader_len
    }

    /// Number of unread events. Clamps to `reader_len` (post-swap only).
    #[inline]
    pub fn len(&self) -> usize {
        debug_assert!(
            boyko_threadpool::is_in_system_run(),
            "EventReader::len called outside a scheduled system body",
        );
        // SAFETY: see `read()`.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        let reader_len = buf.reader_len.load(Ordering::Acquire) as u64;
        let cursor = self.state.last_event_count;
        let start_offset = cursor.saturating_sub(start_count);
        reader_len.saturating_sub(start_offset) as usize
    }

    /// Number of events the cursor skipped (Phase 12 ER7).
    ///
    /// A non-zero return value means a previous swap discarded events the
    /// cursor had not yet visited. Use this for diagnostics — e.g. "audio
    /// dropped N cues because the system fell behind".
    #[inline]
    pub fn missed_events(&self) -> u64 {
        // SAFETY: see `read()`.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        let start_count = buf.start_event_count.load(Ordering::Acquire);
        start_count.saturating_sub(self.state.last_event_count)
    }

    /// Advances the cursor to the current `frame_event_count` without
    /// yielding any events.
    ///
    /// NOTE: this is the only `EventReader` method that consults
    /// `frame_event_count`. `clear()` is opt-in (not on the hot read path),
    /// so paying the extra `Acquire` load is acceptable.
    #[inline]
    pub fn clear(&mut self) {
        // SAFETY: see `read()`.
        let buf: &EventBuffer<E> = unsafe { self.state.buffer_ptr.as_ref() };
        self.state.last_event_count = buf.frame_event_count.load(Ordering::Acquire);
    }
}

// ── EventIter ────────────────────────────────────────────────────────────────

/// Iterator over unread events with a drop-finalised cursor checkpoint
/// (Phase 12 §6.3).
///
/// On drop, the underlying [`EventReaderState`]'s `last_event_count` is set
/// to `start_count + consumed` — supporting partial iteration via `break`
/// and panic-safety (cursor advances even when the loop body panics).
///
/// # Lifetime
///
/// `'a` is the borrow of the dispatcher's `reader_buf` slice AND the
/// `&'a mut last_event_count` back-pointer. Both derive from the same
/// `EventReader<'s, E>::read(&mut self)` borrow, so `'a` is bounded by `'s`.
pub struct EventIter<'a, E: Event> {
    /// Borrowed slice into `reader_buf[start_offset..start_offset + len]`.
    slice: &'a [E],
    /// Number of elements yielded so far.
    consumed: usize,
    /// Back-pointer to the state's cursor. Checkpointed on drop.
    cursor_state: &'a mut u64,
    /// `start_event_count + start_offset` snapshot at `read()` time.
    /// The next cursor value is `start_count + consumed`.
    start_count: u64,
    /// Number of events the cursor skipped (Phase 12 ER7). Surfaced via
    /// [`EventReader::missed_events`] but also stored here for tests /
    /// future per-iter diagnostic APIs.
    #[allow(dead_code)]
    missed: u64,
}

impl<'a, E: Event> Iterator for EventIter<'a, E> {
    type Item = &'a E;

    #[inline]
    fn next(&mut self) -> Option<&'a E> {
        if self.consumed >= self.slice.len() {
            return None;
        }
        // SAFETY (Phase 12 ER2 + Phase 6 U11):
        //   - Bounds checked immediately above.
        //   - Slice elements are initialised per `swap_and_flatten` (U11);
        //     the slice was constructed in `EventReader::read` from the
        //     initialised prefix of `reader_buf`.
        let item = unsafe { self.slice.get_unchecked(self.consumed) };
        self.consumed += 1;
        Some(item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.slice.len() - self.consumed;
        (remaining, Some(remaining))
    }
}

impl<E: Event> ExactSizeIterator for EventIter<'_, E> {}

impl<E: Event> Drop for EventIter<'_, E> {
    #[inline]
    fn drop(&mut self) {
        // Phase 12 ER3: cursor advances by `consumed`. Partial iteration,
        // panic mid-iter, and `drop()` without `next()` are all handled
        // uniformly — the missed-event gap is absorbed because
        // `start_count` was already set to `original_start + start_offset`
        // in `read()`.
        *self.cursor_state = self.start_count + self.consumed as u64;
    }
}

// ── SystemParam impl ─────────────────────────────────────────────────────────

// SAFETY (Phase 12 §2.2 ER1-ER9 + ER-NEW, §6.4):
//   - SP1: `init_access` declares NO access — Option A (events live outside
//     the conflict graph). Per-lane EVT1 discipline + the disjoint
//     reader_buf / write_buf allocations make concurrent
//     `EventReader<E>` / `EventWriter<E>` invocations sound.
//   - SP2: `get_param` simply re-binds the `&'s mut State` borrow.
//   - SP4: `init_state` reads `EventDispatcher::buffer_ptr` and one
//     `Acquire` load on `frame_event_count` — both pure with respect to
//     archetype / resource registries.
unsafe impl<E: Event> SystemParam for EventReader<'_, E> {
    type State = EventReaderState<E>;
    type Item<'w, 's> = EventReader<'s, E>;

    fn init_state(world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        let buffer_ptr = world
            .events()
            .buffer_ptr::<E>()
            .unwrap_or_else(|| event_not_preregistered_panic::<E>());
        // SAFETY (EXT5): heap-stable; field loads are sound.
        let buf = unsafe { buffer_ptr.as_ref() };
        let thread_count = buf.thread_count;
        EventReaderState {
            buffer_ptr,
            // Bevy parity: cursor starts at 0 so late-binding systems still
            // observe historical post-swap events. Users wanting
            // "skip historical" call `clear()`.
            last_event_count: 0,
            thread_count,
            _pad: 0,
            _marker: PhantomData,
        }
    }

    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // Phase 12 ER8 / Q2 Option A — no access declared.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        EventReader { state }
    }
}

// ── Compile-time size pins (Phase 12 §11.1) ──────────────────────────────────

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
    fn event_id() -> crate::ecs::core::events::event::EventId {
        u64::MAX
    }
    fn event_name() -> &'static str {
        "__EventReaderSizeAssert"
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

// `EventReaderState` holds a `cursor: usize` + a `NonNull` buffer pointer, and
// `EventReader` wraps a single pointer-width handle, so these size/align figures
// encode the 64-bit ABI. Gated to 64-bit (the engine's supported platform) — see
// CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::size_of::<EventReaderState<_SizeAssertEvent>>() == 24,
    "EventReaderState<E> must be 24 B (Phase 12 §11.1)",
);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::align_of::<EventReaderState<_SizeAssertEvent>>() == 8,
    "EventReaderState<E> must be 8-byte aligned (Phase 12 §11.1)",
);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::size_of::<EventReader<'_, _SizeAssertEvent>>() == 8,
    "EventReader<'s, E> must be 8 B (Phase 12 §11.1)",
);

// `thread_count` is held symmetric with `EventWriterState` for future
// per-thread read facilities (per-thread snapshot / read-from-own-lane).
// Until those consumers land, suppress the unused-field warning rather than
// drop the field (changing the state shape is a wider commitment).
#[allow(dead_code)]
const _: fn() = || {
    fn assert_field<E: Event>(s: &EventReaderState<E>) -> u32 {
        s.thread_count
    }
    let _ = assert_field::<_SizeAssertEvent>;
};
