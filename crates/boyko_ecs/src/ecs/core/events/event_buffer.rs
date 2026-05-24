use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_config::EventConfig;
use crate::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
use crate::ecs::core::events::parameters::parameters::Parameters;
use crate::ecs::error::{EcsError, EcsResult};

// ── Concrete stub for compile-time layout asserts ─────────────────────────────

/// Private stub event used solely for size/alignment compile-time asserts.
/// Not visible outside this module.
#[doc(hidden)]
struct LayoutAssertEvent {
    _data: u32,
}
#[derive(Copy, Clone)]
struct _NoParticipants;
impl Participants for _NoParticipants {
    fn participant_count() -> usize { 0 }
    fn participant_info() -> &'static [ParticipantInfo] { &[] }
}
#[derive(Copy, Clone)]
struct _NoParameters;
impl Parameters for _NoParameters {}
impl Event for LayoutAssertEvent {
    type Participants = _NoParticipants;
    type Parameters = _NoParameters;
    fn event_id() -> u64 { u64::MAX } // never registered in production
    fn event_name() -> &'static str { "__LayoutAssertEvent" }
    fn new(_: _NoParticipants, _: _NoParameters) -> Self { LayoutAssertEvent { _data: 0 } }
    fn participants(&self) -> &_NoParticipants { unimplemented!() }
    fn participants_mut(&mut self) -> &mut _NoParticipants { unimplemented!() }
    fn parameters(&self) -> &_NoParameters { unimplemented!() }
    fn parameters_mut(&mut self) -> &mut _NoParameters { unimplemented!() }
}

// ── Compile-time layout asserts ───────────────────────────────────────────────

const _: () = assert!(core::mem::size_of::<ThreadLaneWriter<LayoutAssertEvent>>() == 64);
const _: () = assert!(core::mem::align_of::<ThreadLaneWriter<LayoutAssertEvent>>() == 64);
const _: () = assert!(core::mem::size_of::<ThreadLaneReader<LayoutAssertEvent>>() == 64);
const _: () = assert!(core::mem::align_of::<ThreadLaneReader<LayoutAssertEvent>>() == 64);
const _: () = assert!(core::mem::size_of::<ThreadLanePair<LayoutAssertEvent>>() == 128);
const _: () = assert!(core::mem::align_of::<ThreadLanePair<LayoutAssertEvent>>() == 64);

// ── ThreadLaneWriter ─────────────────────────────────────────────────────────

/// Per-thread write lane — occupies exactly one 64-byte cache line.
///
/// Only the worker pinned to the corresponding `thread_index` writes to the
/// `write_buf` contents. The main thread accesses the lane only inside
/// `update_events`, which requires `&mut EventDispatcher` (a synchronisation
/// point that guarantees no workers are in flight).
///
/// `UnsafeCell<Box<[MaybeUninit<E>]>>` provides interior mutability so that
/// `&self`-receiver methods (`send_one`) can legally mutate the buffer; see the
/// per-thread exclusivity invariant documented on the manual `Sync` impl below.
#[repr(C, align(64))]
pub(crate) struct ThreadLaneWriter<E: Event> {
    /// Interior-mutable write buffer. Owned exclusively by the assigned worker.
    /// `UnsafeCell` is `repr(transparent)`, so `size_of` is that of
    /// `Box<[MaybeUninit<E>]>` = 16 B on 64-bit.
    pub(crate) write_buf: UnsafeCell<Box<[MaybeUninit<E>]>>,
    /// Number of initialised elements in `write_buf`. `Release`-stored on every
    /// successful write; `Acquire`-loaded by the swap path.
    pub(crate) write_len: AtomicU32,
    /// Incremented on every rejected `send` (overflow). Purely diagnostic.
    pub(crate) overflow_count: AtomicU32,
    /// Mirrors `EventBuffer::capacity_per_lane` for hot-path locality (avoids
    /// an extra pointer chase through `EventBuffer` on every `send_one` call).
    pub(crate) capacity: u32,
    /// Padding to fill the 64-byte cache line.
    /// Layout: UnsafeCell<Box<_>> = 16 B, AtomicU32 = 4 B × 2, u32 = 4 B → 28 B used;
    /// 64 - 28 = 36 B padding.
    _pad: [u8; 36],
}

// SAFETY (manual Sync for ThreadLaneWriter<E>):
// `UnsafeCell<T>` is `!Sync` by default. We restore soundness with the
// following invariants:
//   1. **Write side**: Only the single worker assigned `thread_index` ever
//      calls `UnsafeCell::get()` to obtain a mutable pointer to the buffer
//      contents during the send phase. Concurrent send calls for the *same*
//      lane from different threads are not permitted (enforced by the
//      Phase 6 single-threaded contract; Phase 7 scheduler will enforce this
//      via thread-index assignment).
//   2. **Swap side**: The main thread accesses every lane's `UnsafeCell`
//      during `update_events`, which takes `&mut EventDispatcher`. The
//      `&mut` acts as the synchronisation point: no worker is calling
//      `send` while `update_events` runs (Phase 6: single-threaded;
//      Phase 7: scheduler quiescence barrier).
//   3. `write_len` and `overflow_count` are `AtomicU32`; cross-thread
//      atomicity is not involved in the `Sync` claim — it covers the
//      *UnsafeCell contents only*.
unsafe impl<E: Event> Sync for ThreadLaneWriter<E> {}

// ── ThreadLaneReader ─────────────────────────────────────────────────────────

/// Per-thread reader metadata — occupies exactly one 64-byte cache line.
///
/// Sits on a separate cache line from `ThreadLaneWriter` to avoid false sharing
/// between the send path (writer) and the swap path (reader).
#[repr(C, align(64))]
pub(crate) struct ThreadLaneReader<E: Event> {
    /// Reserved for Phase 7 (per-lane snapshot pointer).
    /// Placed first to align naturally on 8 B without internal padding.
    pub(crate) reserved_ptr: AtomicPtr<MaybeUninit<E>>,
    /// Per-lane swap cursor; updated during `swap_and_flatten`.
    pub(crate) read_cursor: AtomicU32,
    /// Padding: 64 - (8 + 4) = 52 B.
    _pad: [u8; 52],
}

// ── ThreadLanePair ───────────────────────────────────────────────────────────

/// A pair of `{ writer, reader }` cache lines for one worker thread.
///
/// The two halves occupy adjacent 64-byte cache lines (128 B total).
/// Separating them prevents false sharing between the writer (hot) and the
/// swap reader (cold, per-frame only).
#[repr(C, align(64))]
pub(crate) struct ThreadLanePair<E: Event> {
    pub(crate) writer: ThreadLaneWriter<E>,
    pub(crate) reader: ThreadLaneReader<E>,
}

// ── EventBuffer ──────────────────────────────────────────────────────────────

/// Per-type, per-master event storage.
///
/// Owns `thread_count` writer lanes (one per worker) and a flat reader buffer
/// of size `thread_count * capacity_per_lane` that is populated by
/// `update_events` via per-lane `memcpy`.
///
/// All allocations happen once at `preregister_event` time; no allocation
/// occurs during `send` or `update_events`.
pub(crate) struct EventBuffer<E: Event> {
    /// One pair per worker thread. Length = `thread_count`.
    pub(crate) lanes: Box<[ThreadLanePair<E>]>,
    /// Flat read buffer. Size = `thread_count * capacity_per_lane`.
    /// Populated by `swap_and_flatten`; ownership is exclusively within
    /// `EventBuffer` — `update_events` takes `&mut EventDispatcher`, giving
    /// unique access.
    pub(crate) reader_buf: Box<[MaybeUninit<E>]>,
    /// Number of initialised elements in `reader_buf` after the last swap.
    /// `Release`-stored by `swap_and_flatten`; `Acquire`-loaded by `events::<E>()`.
    pub(crate) reader_len: AtomicU32,
    // Kept for diagnostics and future APIs; not read in Phase 6 hot paths.
    #[allow(dead_code)]
    pub(crate) capacity_per_lane: u32,
    pub(crate) thread_count: u32,
    _marker: core::marker::PhantomData<E>,
}

impl<E: Event> EventBuffer<E> {
    /// Allocates a new buffer for the given configuration.
    ///
    /// Allocates all lane `write_buf`s and the flat `reader_buf` upfront.
    ///
    /// # Errors
    ///
    /// Returns an error from `EventConfig` if validation fails (the config was
    /// already validated by `EventConfig::new`; this is belt-and-suspenders).
    pub(crate) fn new(cfg: EventConfig) -> EcsResult<Self> {
        let thread_count = cfg.thread_count;
        let capacity = cfg.capacity_per_lane;
        let reader_total = (thread_count as usize)
            .checked_mul(capacity as usize)
            .ok_or(EcsError::InvalidEventConfig {
                reason: "thread_count * capacity_per_lane overflows usize",
            })?;

        // Allocate the flat reader buffer (uninitialised).
        let reader_buf: Box<[MaybeUninit<E>]> =
            (0..reader_total).map(|_| MaybeUninit::uninit()).collect();

        // Allocate each lane pair.
        let lanes: Box<[ThreadLanePair<E>]> = (0..thread_count as usize)
            .map(|_| {
                let write_buf: Box<[MaybeUninit<E>]> =
                    (0..capacity as usize).map(|_| MaybeUninit::uninit()).collect();
                ThreadLanePair {
                    writer: ThreadLaneWriter {
                        write_buf: UnsafeCell::new(write_buf),
                        write_len: AtomicU32::new(0),
                        overflow_count: AtomicU32::new(0),
                        capacity,
                        _pad: [0u8; 36],
                    },
                    reader: ThreadLaneReader {
                        reserved_ptr: AtomicPtr::new(core::ptr::null_mut()),
                        read_cursor: AtomicU32::new(0),
                        _pad: [0u8; 52],
                    },
                }
            })
            .collect();

        Ok(EventBuffer {
            lanes,
            reader_buf,
            reader_len: AtomicU32::new(0),
            capacity_per_lane: capacity,
            thread_count,
            _marker: core::marker::PhantomData,
        })
    }

    /// Writes a single event to the specified thread's write lane.
    ///
    /// Returns `EventBufferFull` if the lane is at capacity (all-or-nothing
    /// for a single event — `attempted = 1, dropped = 1`).
    #[inline]
    pub(crate) fn send_one(&self, thread_index: u32, event: E) -> EcsResult<()> {
        debug_assert!(
            thread_index < self.thread_count,
            "thread_index {thread_index} >= thread_count {}",
            self.thread_count
        );
        let lane = &self.lanes[thread_index as usize].writer;
        // Single writer per lane; Relaxed load is sufficient here because
        // the owning thread is the only writer and this is a single-threaded
        // send phase in Phase 6. Phase 7 will maintain this via scheduler.
        let len = lane.write_len.load(Ordering::Relaxed);
        if len >= lane.capacity {
            lane.overflow_count.fetch_add(1, Ordering::Relaxed);
            return Err(EcsError::EventBufferFull {
                type_name: core::any::type_name::<E>(),
                thread_index,
                attempted: 1,
                dropped: 1,
            });
        }
        // SAFETY (U4 — rewritten, C1-NEW, W4-NEW):
        // 1. Interior mutability via UnsafeCell: `write_buf` is
        //    `UnsafeCell<Box<[MaybeUninit<E>]>>`. `UnsafeCell::get()` is the
        //    only sanctioned primitive for mutating shared data through `&`; the
        //    aliasing rules treat the cell as exempt from the no-mutation-through-`&`
        //    rule. This is the explicit basis for taking a `&self`-receiver
        //    `send_one` and mutating the box.
        // 2. Per-thread exclusivity: only the worker pinned to `thread_index`
        //    accesses this UnsafeCell. The send path is not re-entrant: a single
        //    worker calling `send_one(thread_index, …)` for the same `thread_index`
        //    cannot be interrupted mid-write by another caller for the same lane.
        // 3. Bounds: `len < lane.capacity` checked immediately above.
        // 4. Initialisation discipline: `MaybeUninit::write` is correct — the slot
        //    at `[len]` is uninitialised because `write_len` always tracks the
        //    initialised prefix; `write` does not drop the previous (uninitialised) value.
        // 5. No aliasing with reader: the reader path lives in `ThreadLaneReader` on
        //    a different cache line, and `swap_and_flatten` only runs from the main
        //    thread under the `&mut EventDispatcher` sync point with all workers stopped.
        unsafe {
            // U1 (rewritten, C1-NEW, W8-NEW): UnsafeCell::get() yields
            // *mut Box<[MaybeUninit<E>]>. (*box_ptr).as_mut_ptr() uses Box
            // slice-deref (stable; Box::as_mut_ptr is unstable per rust#129090).
            let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
            let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
            let dst: *mut MaybeUninit<E> = buf_ptr.add(len as usize);
            (*dst).write(event);
        }
        // Release: matches AcqRel on write_len.swap in swap_and_flatten (U9).
        lane.write_len.store(len + 1, Ordering::Release);
        Ok(())
    }

    /// Writes a batch of events to the specified thread's write lane.
    ///
    /// All-or-nothing: pre-checks capacity, then writes all events. If any
    /// overflow would occur, returns `EventBufferFull` with `attempted = dropped = n`
    /// and writes nothing.
    pub(crate) fn send_many<I>(&self, thread_index: u32, iter: I) -> EcsResult<()>
    where
        I: ExactSizeIterator<Item = E>,
    {
        debug_assert!(
            thread_index < self.thread_count,
            "thread_index {thread_index} >= thread_count {}",
            self.thread_count
        );
        let lane = &self.lanes[thread_index as usize].writer;
        let len = lane.write_len.load(Ordering::Relaxed);
        let n = iter.len() as u32;
        if n == 0 {
            return Ok(());
        }
        let remaining = lane.capacity.saturating_sub(len);
        if n > remaining {
            lane.overflow_count.fetch_add(1, Ordering::Relaxed);
            return Err(EcsError::EventBufferFull {
                type_name: core::any::type_name::<E>(),
                thread_index,
                attempted: n,
                dropped: n,
            });
        }
        // SAFETY (U4-batch — same contract as U4 per element):
        // `n <= remaining` was verified by the pre-check above, so every write
        // at offset `len + i` satisfies `len + i < capacity`. All other U4
        // invariants apply element-wise. `UnsafeCell::get()` + slice-deref is
        // stable (U1/W8-NEW). No other thread touches this lane's write_buf
        // during the send phase (per-thread exclusivity invariant).
        unsafe {
            let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
            let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
            for (i, e) in iter.enumerate() {
                (*buf_ptr.add(len as usize + i)).write(e);
            }
        }
        // Release: matches AcqRel on write_len.swap in swap_and_flatten (U9).
        lane.write_len.store(len + n, Ordering::Release);
        Ok(())
    }
}

impl<E: Event> Drop for EventBuffer<E> {
    fn drop(&mut self) {
        if core::mem::needs_drop::<E>() {
            // SAFETY (U12):
            // 1. We only drop the `[0..reader_len)` prefix of `reader_buf` and
            //    `[0..write_len)` prefix of each `write_buf` — exactly the initialised
            //    prefixes per MaybeUninit discipline (`reader_len` and `write_len`
            //    always track the number of initialised elements).
            // 2. `&mut self` on `Drop` guarantees no other code holds aliases;
            //    `UnsafeCell::get()` is sound to use with `&mut self` because we have
            //    exclusive access to the cell.
            // 3. After these drops, the `Box<[MaybeUninit<E>]>`s deallocate their
            //    slice storage naturally; `MaybeUninit` never auto-drops contents, so
            //    this manual drop loop is necessary for `E: Drop`.
            // 4. Panic-safety (W2 follow-up): each length field is swapped to 0
            //    BEFORE its drop loop runs. If `E::drop` panics on element `k`,
            //    further unwinding through this Drop sees length = 0 for the
            //    remaining lanes/reader and skips them — Rust's no-double-panic
            //    convention then aborts the process, but no slot is dropped twice.
            let len = self.reader_len.swap(0, Ordering::Relaxed) as usize;
            unsafe {
                for i in 0..len {
                    self.reader_buf[i].assume_init_drop();
                }
            }
            for lane in self.lanes.iter_mut() {
                let n = lane.writer.write_len.swap(0, Ordering::Relaxed) as usize;
                unsafe {
                    let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.writer.write_buf.get();
                    let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
                    for i in 0..n {
                        (*buf_ptr.add(i)).assume_init_drop();
                    }
                }
            }
        }
        // Box<[MaybeUninit<E>]>s drop here naturally → frees allocations.
    }
}
