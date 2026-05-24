use core::any::{TypeId, type_name};
use core::mem::MaybeUninit;
use core::marker::PhantomData;

use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_buffer::EventBuffer;
use crate::ecs::core::events::event_config::EventConfig;
use crate::ecs::core::events::event_registry::MAX_EVENTS;
use crate::ecs::error::{EcsError, EcsResult};

// ── ZstCheck ─────────────────────────────────────────────────────────────────

/// Compile-time ZST guard (D9, C2-NEW).
///
/// An `assert!` inside a generic function body cannot reference outer generic
/// type parameters (Rust rejects it). An associated const on a generic helper
/// struct is the canonical pattern — the const is evaluated at
/// monomorphisation time when `ZstCheck::<E>::NON_ZERO` is read.
///
/// Forces a compile-time error if `E` is a zero-sized type (e.g. `struct Marker;`).
/// Workaround: add at least one field (e.g. `_unit: u8`).
struct ZstCheck<E: Event>(PhantomData<E>);

impl<E: Event> ZstCheck<E> {
    const NON_ZERO: () = assert!(
        core::mem::size_of::<E>() > 0,
        "Event type is zero-sized; use a counter instead (add a non-ZST field)",
    );
}

// ── EventVTable ───────────────────────────────────────────────────────────────

/// Per-type function pointers used by the dispatcher to perform type-erased
/// operations on `EventBuffer<E>` without `dyn Trait`.
///
/// Size: 8 + 8 + 16 = 32 B. Verified by the compile-time assert below.
/// `TypeId` is 16 B (u128) since Rust PR #109953 / #75923, stabilised in 1.72.
#[repr(C)]
struct EventVTable {
    /// Called by `update_events`. Reads write lanes, flattens to `reader_buf`,
    /// resets write lanes. `frame` = current dispatcher frame counter.
    swap_fn: unsafe fn(slot_data: *mut u8, frame: u64),
    /// Called by `EventDispatcher::drop`. Reconstructs `Box<EventBuffer<E>>` from
    /// the raw pointer and drops it — `Box`'s `Drop` runs `EventBuffer::drop` AND
    /// deallocates. (W6-NEW decision (b): no separate `dealloc` step.)
    drop_fn: unsafe fn(slot_data: *mut u8),
    /// Runtime type sanity check on typed access. 16 B (u128).
    type_id: TypeId,
}

const _: () = assert!(core::mem::size_of::<EventVTable>() <= 32);

// ── EventTypeSlot ─────────────────────────────────────────────────────────────

/// One 64-byte-aligned slot per registered event type.
///
/// Release: 48 B payload → `align(64)` → 64 B (16 B tail padding).
/// Debug: 48 + 8 + 4 = 60 B → `align(64)` → 64 B (4 B tail padding).
/// The `% 64 == 0` assertion accepts either size; the release tight check
/// guards the hot-path layout assumption.
#[repr(C, align(64))]
struct EventTypeSlot {
    /// Raw pointer to `Box<EventBuffer<E>>::into_raw()`. 8 B.
    data: *mut u8,
    /// Type-erased operations. 32 B.
    vtable: EventVTable,
    /// Capacity mirrored from `EventConfig` for diagnostics. 4 B.
    capacity_per_lane: u32,
    /// Thread count mirrored from `EventConfig` for diagnostics. 4 B.
    thread_count: u32,
    // Release total: 8 + 32 + 4 + 4 = 48 B → padded to 64 B by align(64).
    #[cfg(debug_assertions)]
    /// Frame of the last `update_events` call for this slot. u64 (W5-NEW).
    last_swap_frame: u64,
    #[cfg(debug_assertions)]
    /// Events visible in `reader_buf` after the last swap (unread diagnostic).
    events_swapped_unread: u32,
}

const _: () = assert!(core::mem::align_of::<EventTypeSlot>() == 64);
const _: () = assert!(core::mem::size_of::<EventTypeSlot>().is_multiple_of(64));
#[cfg(not(debug_assertions))]
const _: () = assert!(core::mem::size_of::<EventTypeSlot>() == 64);

// ── EventTypeSlotStorage ──────────────────────────────────────────────────────

/// `MaybeUninit` wrapper so the slot array can be heap-allocated without
/// requiring `EventTypeSlot: Default` and without pulling in an `Option`
/// discriminant that would break the 64-byte alignment invariant.
///
/// A slot is initialised if and only if `registered_mask.get(index) == true`.
#[repr(C)]
struct EventTypeSlotStorage {
    slot: MaybeUninit<EventTypeSlot>,
}

// ── Diagnostics (debug only) ─────────────────────────────────────────────────

/// Per-type diagnostics returned by [`EventDispatcher::diagnostics`].
#[cfg(debug_assertions)]
pub struct EventDiagnostics {
    /// The `current_frame` value at the time of the last swap for this type.
    pub last_swap_frame: u64,
    /// Number of events that were copied to `reader_buf` during the last swap
    /// (an approximation of "events sent but not yet consumed by systems").
    pub events_swapped_unread: u32,
    /// Per-lane overflow counter (events rejected due to full buffer).
    pub per_lane_overflow_count: Box<[u32]>,
}

// ── EventDispatcher ───────────────────────────────────────────────────────────

/// Per-master typed event dispatcher with double-buffer semantics (Model B).
///
/// Events sent during frame N become readable via [`events`] only after the
/// next [`update_events`] call (next-frame visibility). This prevents
/// read/write conflicts within a single system tick.
///
/// Storage is pre-allocated at [`preregister`] time; no heap allocation occurs
/// during `send` or `update_events`.
///
/// [`events`]: EventDispatcher::events
/// [`update_events`]: EventDispatcher::update_events
/// [`preregister`]: EventDispatcher::preregister
pub struct EventDispatcher {
    /// 256 slots, one per possible `EventId`. 256 × 64 B = 16 KB.
    slots: Box<[EventTypeSlotStorage; MAX_EVENTS]>,
    /// Tracks which slots are initialised. 32 B; iterated with TZCNT via
    /// `pop_lowest_set_bit` for O(k) dispatch where k = registered types.
    registered_mask: BitSet256,
    /// Monotonically increasing per-frame counter. `u64` so wrap-around
    /// (~16 EB frames) is never reachable in practice (W5-NEW).
    current_frame: u64,
    /// Validated default thread count; used by `preregister_event_default`.
    default_thread_count: u32,
}

impl EventDispatcher {
    /// Constructs a new dispatcher with `default_thread_count` worker lanes
    /// used by [`preregister_event_default`].
    ///
    /// Validates `default_thread_count` via `EventConfig::default_for`; returns
    /// `Err(InvalidEventConfig)` if out of range.
    ///
    /// [`preregister_event_default`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::preregister_event_default
    pub fn new(default_thread_count: u32) -> EcsResult<Self> {
        // Validate the default_thread_count via EventConfig so that subsequent
        // `.expect("invariant: ...")` in preregister_event_default is honest.
        EventConfig::default_for(default_thread_count)?;

        // Allocate 16 KB slot array on the heap; all slots start uninitialised.
        // Build via Vec<_> → boxed slice → fixed-size array to avoid a 16 KB
        // stack frame. The transmute is sound: [T; N] and [T] have the same
        // element layout; we verify the length via the assert below.
        let slots: Box<[EventTypeSlotStorage; MAX_EVENTS]> = {
            let v: Vec<EventTypeSlotStorage> = (0..MAX_EVENTS)
                .map(|_| EventTypeSlotStorage { slot: MaybeUninit::uninit() })
                .collect();
            let boxed_slice: Box<[EventTypeSlotStorage]> = v.into_boxed_slice();
            assert_eq!(boxed_slice.len(), MAX_EVENTS,
                "invariant: Vec length must equal MAX_EVENTS");
            // SAFETY: `boxed_slice` has exactly MAX_EVENTS elements (asserted).
            // Converting Box<[T]> to Box<[T; N]> is sound when lengths match;
            // the pointer provenance is preserved (same allocation, same pointer).
            unsafe {
                Box::from_raw(Box::into_raw(boxed_slice) as *mut [EventTypeSlotStorage; MAX_EVENTS])
            }
        };

        Ok(EventDispatcher {
            slots,
            registered_mask: BitSet256::new(),
            current_frame: 0,
            default_thread_count,
        })
    }

    /// Returns the validated default thread count.
    #[inline]
    pub fn default_thread_count(&self) -> u32 {
        self.default_thread_count
    }

    /// Registers event type `E` on this dispatcher with the given config.
    ///
    /// Allocates all write lanes and the reader buffer upfront. Must be called
    /// before the first `send::<E>` or `events::<E>` call.
    ///
    /// # Errors
    ///
    /// - `EventAlreadyRegistered` — if `E` was already registered.
    /// - `EventNotRegistered` — if `E::event_id() as usize >= MAX_EVENTS`.
    /// - `InvalidEventConfig` — if the allocation size overflows.
    /// - `EventBufferFull` — never returned here; included via `EcsResult`.
    ///
    /// # Compile-time failure
    ///
    /// Fails to compile if `E` is a zero-sized type (e.g. `struct Marker;`).
    pub fn preregister<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
        // C2-NEW: force monomorphisation-time const-eval; trips for ZST E.
        #[allow(clippy::let_unit_value)]
        let _ = ZstCheck::<E>::NON_ZERO;

        let id = E::event_id() as usize;
        if id >= MAX_EVENTS {
            return Err(EcsError::EventNotRegistered { type_name: type_name::<E>() });
        }
        if self.registered_mask.get(id) {
            return Err(EcsError::EventAlreadyRegistered { type_name: type_name::<E>() });
        }

        let buffer = Box::new(EventBuffer::<E>::new(cfg)?);
        let data: *mut u8 = Box::into_raw(buffer).cast();

        let slot = EventTypeSlot {
            data,
            vtable: EventVTable {
                swap_fn: swap_and_flatten::<E>,
                drop_fn: drop_buffer::<E>,
                type_id: TypeId::of::<E>(),
            },
            capacity_per_lane: cfg.capacity_per_lane,
            thread_count: cfg.thread_count,
            #[cfg(debug_assertions)]
            last_swap_frame: 0,
            #[cfg(debug_assertions)]
            events_swapped_unread: 0,
        };

        // SAFETY (U2):
        // 1. `id < MAX_EVENTS` (checked above).
        // 2. `registered_mask.get(id) == false` (checked above), so the slot
        //    holds no previous value that needs dropping.
        // 3. `slot` is fully initialised.
        // Note: `MaybeUninit::write` is safe in Rust 2024; the unsafe block
        // is kept for documentation clarity matching the SAFETY comment above.
        self.slots[id].slot.write(slot);
        self.registered_mask.set(id);
        Ok(())
    }

    /// Sends a single event of type `E` to the lane for `thread_index`.
    ///
    /// # Errors
    ///
    /// - `EventNotRegistered` — if `preregister::<E>` was not called.
    /// - `EventBufferFull` — if the lane is at capacity.
    #[inline]
    pub fn send<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
        let id = E::event_id() as usize;
        debug_assert!(id < MAX_EVENTS, "event id {id} >= MAX_EVENTS {MAX_EVENTS}");
        debug_assert!(
            self.registered_mask.get(id),
            "preregister_event::<E> must be called before send",
        );

        if !self.registered_mask.get(id) {
            return Err(EcsError::EventNotRegistered { type_name: type_name::<E>() });
        }
        // SAFETY (U3):
        // 1. `id < MAX_EVENTS` (debug_assert + inline release guard above).
        // 2. `registered_mask.get(id) == true` (branch above returns early otherwise).
        // 3. The bit is set only after `slot.write` in `preregister`; bit-set ⇒ initialised.
        let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
        debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>(), "type_id mismatch");
        // SAFETY (U7-read):
        // 1. Same type-matching as U7: slot was registered for this exact `E`.
        // 2. Shared borrow: `send` takes `&self`; multiple shared references are sound.
        // 3. Concurrent mutation through `&EventBuffer<E>` only happens via AtomicU32s
        //    and UnsafeCell::get() (with per-thread exclusivity, U4) — both sanctioned.
        let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };
        buf.send_one(thread_index, event)
    }

    /// Sends a batch of events of type `E` to the lane for `thread_index`.
    ///
    /// All-or-nothing: if `iter.len()` events would overflow the lane, nothing
    /// is written and `EventBufferFull { attempted: n, dropped: n }` is returned.
    ///
    /// # Errors
    ///
    /// - `EventNotRegistered` — if `preregister::<E>` was not called.
    /// - `EventBufferFull` — if the batch does not fit in the remaining capacity.
    pub fn send_many<E: Event, I>(&self, thread_index: u32, iter: I) -> EcsResult<()>
    where
        I: ExactSizeIterator<Item = E>,
    {
        let id = E::event_id() as usize;
        debug_assert!(id < MAX_EVENTS, "event id {id} >= MAX_EVENTS {MAX_EVENTS}");

        if !self.registered_mask.get(id) {
            return Err(EcsError::EventNotRegistered { type_name: type_name::<E>() });
        }
        // SAFETY (U3): same as `send`.
        let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
        debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>(), "type_id mismatch");
        // SAFETY (U7-read): same as `send`.
        let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };
        buf.send_many(thread_index, iter)
    }

    /// Returns the slice of events of type `E` that were sent during the
    /// **previous** frame (next-frame visibility, Model B).
    ///
    /// Returns an empty slice if `E` was not registered or if no events were
    /// sent last frame.
    #[inline]
    pub fn events<E: Event>(&self) -> &[E] {
        let id = E::event_id() as usize;
        debug_assert!(id < MAX_EVENTS, "event id {id} >= MAX_EVENTS {MAX_EVENTS}");
        if !self.registered_mask.get(id) {
            return &[];
        }
        // SAFETY (U10 — same as U3): registered_mask.get(id) == true ⇒ initialised.
        let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
        debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>(), "type_id mismatch");
        // SAFETY (U7-read): same as `send`.
        let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };
        // Acquire: matches Release in swap_and_flatten (U9 / U11).
        let len = buf.reader_len.load(core::sync::atomic::Ordering::Acquire) as usize;
        // SAFETY (U11 rewritten — W7-NEW):
        // 1. `MaybeUninit::slice_assume_init_ref` is unstable as of Rust 1.95 stable
        //    (rust#80836). We use the equivalent stable construction below.
        // 2. `len = reader_len.load(Acquire)`.
        // 3. `reader_len.store(cursor, Release)` in `swap_and_flatten` happens-before
        //    this load (Acquire/Release pair).
        // 4. `cursor` bounds the prefix initialised by `copy_nonoverlapping` (U9),
        //    so the first `len` elements of `reader_buf` contain valid `E` values.
        // 5. `(*buf.reader_buf).as_ptr()` is `*const MaybeUninit<E>`; `.cast::<E>()`
        //    is valid because `MaybeUninit<T>` and `T` have identical layout
        //    (`#[repr(transparent)]`).
        // 6. `from_raw_parts` requirements: ptr non-null (Box-derived), ptr aligned
        //    for `E` (Box guarantees align_of::<E>()), first `len` elements valid (4),
        //    slice ≤ isize::MAX bytes, no concurrent mutable borrow (`events` takes
        //    `&self`; only `update_events` takes `&mut self`).
        unsafe {
            let ptr: *const E = (*buf.reader_buf).as_ptr().cast::<E>();
            core::slice::from_raw_parts(ptr, len)
        }
    }

    /// Advances the frame counter and flattens each registered event type's
    /// write lanes into the contiguous reader buffer.
    ///
    /// Must be called once per frame, typically at the end of the frame (or
    /// beginning of the next). After this call, `events::<E>()` returns the
    /// events sent during the frame that just ended.
    pub fn update_events(&mut self) {
        // Wrapping add: u64 wrap (~16 EB frames) is never reachable in practice.
        self.current_frame = self.current_frame.wrapping_add(1);
        let frame = self.current_frame;

        // Iterate only the set bits (registered types). O(k) where k = registered.
        let mut mask = self.registered_mask;
        while let Some(id) = mask.pop_lowest_set_bit() {
            // SAFETY (U5):
            // 1. `id` came from `registered_mask.pop_lowest_set_bit()` ⇒ bit was
            //    set ⇒ slot is initialised (mirror of U3).
            // 2. `&mut self` on `update_events` provides exclusive access.
            let slot: &mut EventTypeSlot =
                unsafe { self.slots[id as usize].slot.assume_init_mut() };

            #[cfg(debug_assertions)]
            {
                // u64 frame: wrap never reachable in practice (W5-NEW).
                assert!(
                    slot.last_swap_frame < frame,
                    "double-swap detected for event slot {id} (frame={frame}, last={last})",
                    last = slot.last_swap_frame
                );
                slot.last_swap_frame = frame;
            }

            // SAFETY (U6):
            // 1. `slot.vtable.swap_fn` was set to `swap_and_flatten::<E>` for the
            //    exact type `E` whose buffer is at `slot.data` (set atomically with
            //    `data` in `preregister`).
            // 2. `slot.data` points to a `Box<EventBuffer<E>>`-derived raw pointer
            //    (allocated in `preregister`, freed only in `drop_buffer::<E>` during
            //    dispatcher drop).
            // 3. The function pointer is `'static`; monomorphised at compile time.
            unsafe { (slot.vtable.swap_fn)(slot.data, frame); }
        }
    }

    /// Returns per-type diagnostics (debug builds only).
    ///
    /// Returns `None` if `E` is not registered on this dispatcher.
    #[cfg(debug_assertions)]
    pub fn diagnostics<E: Event>(&self) -> Option<EventDiagnostics> {
        let id = E::event_id() as usize;
        if id >= MAX_EVENTS || !self.registered_mask.get(id) {
            return None;
        }
        // SAFETY (U3): bit is set ⇒ slot is initialised.
        let slot: &EventTypeSlot =
            unsafe { self.slots[id].slot.assume_init_ref() };
        // SAFETY (U7-read): data was stored for type E in preregister.
        let buf: &EventBuffer<E> =
            unsafe { &*(slot.data as *const EventBuffer<E>) };
        let per_lane_overflow: Box<[u32]> = buf
            .lanes
            .iter()
            .map(|lp| lp.writer.overflow_count.load(core::sync::atomic::Ordering::Relaxed))
            .collect();
        Some(EventDiagnostics {
            last_swap_frame: slot.last_swap_frame,
            events_swapped_unread: slot.events_swapped_unread,
            per_lane_overflow_count: per_lane_overflow,
        })
    }

    /// Test-only: exposes `current_frame` for near-wrap tests (#26).
    #[cfg(test)]
    pub(crate) fn set_current_frame_for_test(&mut self, frame: u64) {
        self.current_frame = frame;
    }

    /// Test-only: overwrites the `vtable.type_id` of a registered slot for
    /// type_id collision detection tests (#30).
    ///
    /// Returns `false` if `id` is not a registered slot.
    #[cfg(test)]
    pub(crate) fn set_slot_type_id_for_test(&mut self, id: usize, type_id: TypeId) -> bool {
        if id >= MAX_EVENTS || !self.registered_mask.get(id) {
            return false;
        }
        // SAFETY (U5): registered_mask.get(id) == true ⇒ slot is initialised.
        let slot = unsafe { self.slots[id].slot.assume_init_mut() };
        slot.vtable.type_id = type_id;
        true
    }
}

impl Drop for EventDispatcher {
    fn drop(&mut self) {
        let mut mask = self.registered_mask;
        while let Some(id) = mask.pop_lowest_set_bit() {
            // SAFETY: bit was set ⇒ slot was initialised by preregister.
            let slot: EventTypeSlot =
                unsafe { core::ptr::read(self.slots[id as usize].slot.as_ptr()) };
            // SAFETY (U6-drop):
            // 1. Same type-matching as U6: `drop_fn` was registered for the exact `E`
            //    whose buffer sits at `slot.data`.
            // 2. `slot.data` has never been freed before (this is the only call site).
            // 3. After this call, `slot.data` is dangling — but the slot is unreachable
            //    because `EventDispatcher` is itself being dropped.
            unsafe { (slot.vtable.drop_fn)(slot.data); }
        }
        // registered_mask intentionally unmodified — we are being dropped.
    }
}

// ── Monomorphised free functions ──────────────────────────────────────────────

/// Walks each lane's write_buf, copies contents to `reader_buf`, resets
/// write counters, then publishes the total event count to `reader_len`.
///
/// Called once per frame per registered event type from `update_events`.
///
/// # Safety
///
/// Caller (U6) guarantees:
/// - `data` was produced by `Box::into_raw(Box::new(EventBuffer::<E>::new(...)))`.
/// - No worker threads are calling `send` on any lane during this function.
/// - This function is the sole accessor of `data` at this instant (ensured by
///   `&mut EventDispatcher` on `update_events`).
unsafe fn swap_and_flatten<E: Event>(data: *mut u8, _frame: u64) {
    // SAFETY (U7):
    // 1. The function is monomorphised only for the `E` that registered this slot.
    // 2. `data` was produced by `Box::into_raw(Box::new(EventBuffer::<E>::new(...)))`.
    // 3. The mutable borrow is unique: `update_events` takes `&mut self` on
    //    `EventDispatcher` and is single-threaded.
    let buf: &mut EventBuffer<E> = unsafe { &mut *(data as *mut EventBuffer<E>) };

    use core::sync::atomic::Ordering;

    // Drop previous frame's reader_buf contents if E implements Drop.
    let prev_len = buf.reader_len.load(Ordering::Relaxed) as usize;
    if core::mem::needs_drop::<E>() {
        // SAFETY (U8):
        // 1. `i < prev_len = reader_len.load(Relaxed)`.
        // 2. `reader_len` was set to the cursor that bounded the initialised prefix
        //    at the end of the previous swap.
        // 3. No reader holds a `&E` to this slot: convention is all readers complete
        //    before `update_events`.
        unsafe {
            for i in 0..prev_len {
                buf.reader_buf[i].assume_init_drop();
            }
        }
    }

    // Walk lanes and copy.
    let mut cursor = 0usize;
    for lane in buf.lanes.iter_mut() {
        let writer = &lane.writer;
        // AcqRel: the Acquire half synchronises with the Release-store of
        // `write_len` in `send_one`/`send_many`, making all prior writes by the
        // worker visible here (U9, point 8).
        let n = writer.write_len.swap(0, Ordering::AcqRel) as usize;
        if n == 0 {
            continue;
        }
        debug_assert!(
            cursor + n <= buf.reader_buf.len(),
            "overflow: cursor {cursor} + n {n} > reader_buf.len() {}",
            buf.reader_buf.len()
        );
        // SAFETY (U9 rewritten — C1-NEW, W8-NEW):
        // 1. Source access: `UnsafeCell::get()` yields `*mut Box<[MaybeUninit<E>]>`;
        //    we call `.as_ptr()` for a `*const MaybeUninit<E>`. At swap time we are
        //    the only accessor system-wide (workers stopped, main thread runs swap),
        //    so reading the cell from the main thread is sound (U13).
        // 2. Destination access: `buf.reader_buf` is owned by `&mut EventBuffer<E>`
        //    (U7 above). `(*box).as_mut_ptr()` via slice-deref is stable.
        // 3. `n` came from `write_len.swap(0, AcqRel)` — exactly the number of
        //    initialised elements in the lane's `write_buf`.
        // 4. `cursor + n <= reader_buf.len()`: `reader_buf.len() == thread_count *
        //    capacity_per_lane`; summing `n` across lanes cannot exceed that bound
        //    (each `n <= capacity_per_lane`, debug_assert above).
        // 5. Source and destination are disjoint allocations (separate Boxes).
        // 6. Both regions are properly aligned for `E` (Boxes guarantee align_of::<E>()).
        // 7. After this call, `writer.write_buf[0..n]` is logically uninitialised
        //    (`write_len = 0` was published by the swap above).
        // 8. AcqRel ordering on swap synchronises with Release on the worker's
        //    `write_len.store` — all bytes written by the worker before publishing
        //    `write_len` are visible to this read (U13).
        unsafe {
            // U1 (rewritten, C1-NEW, W8-NEW): UnsafeCell::get() + slice-deref is stable.
            let box_ptr: *mut Box<[MaybeUninit<E>]> = writer.write_buf.get();
            let src: *const MaybeUninit<E> = (*box_ptr).as_ptr();
            let dst: *mut MaybeUninit<E> = (*buf.reader_buf).as_mut_ptr().add(cursor);
            core::ptr::copy_nonoverlapping(src, dst, n);
        }
        cursor += n;
    }

    // Release: matches Acquire-load in `events::<E>()` (U11).
    buf.reader_len.store(cursor as u32, Ordering::Release);
}

/// Reconstructs the `Box<EventBuffer<E>>` from the raw pointer and drops it,
/// running `EventBuffer::drop` and deallocating the heap allocation.
///
/// # Safety
///
/// - `data` was produced by `Box::into_raw(Box::new(EventBuffer::<E>::new(...)))`.
/// - This function is called exactly once per `data` pointer (from `EventDispatcher::drop`).
/// - After this returns, `data` is dangling.
unsafe fn drop_buffer<E: Event>(data: *mut u8) {
    // SAFETY (U6-drop, U7-drop):
    // `data` was produced by `Box::into_raw` on the matching `EventBuffer<E>`.
    // Reconstructing the `Box` runs `EventBuffer::drop` (which drops initialised
    // event payloads per U12) and deallocates the heap allocation.
    // After this returns, `data` is dangling.
    let _ = unsafe { Box::from_raw(data as *mut EventBuffer<E>) };
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::Ordering;
    use std::any::TypeId;

    use crate::ecs::core::events::event::Event;
    use crate::ecs::core::events::event_config::EventConfig;
    use crate::ecs::core::events::event_registry::register_event;
    use crate::ecs::core::events::participants::participants::{ParticipantInfo, Participants};
    use crate::ecs::core::events::parameters::parameters::Parameters;
    use crate::ecs::error::EcsError;

    use super::EventDispatcher;

    // ── Test event stubs ──────────────────────────────────────────────────────

    #[derive(Clone, Copy)]
    struct NoParticipants;
    impl Participants for NoParticipants {
        fn participant_count() -> usize { 0 }
        fn participant_info() -> &'static [ParticipantInfo] { &[] }
    }

    #[derive(Clone, Copy)]
    struct NoParameters;
    impl Parameters for NoParameters {}

    /// Test event type A — simple u32 payload. ID range 20–29 reserved.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct EvA {
        val: u32,
    }
    impl Event for EvA {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> u64 { 20 }
        fn event_name() -> &'static str { "EvA" }
        fn new(_: NoParticipants, _: NoParameters) -> Self { EvA { val: 0 } }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Drop-counting event for drop correctness test. ID = 22.
    static DROP_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

    struct DropCounter { _val: u32 }
    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl Clone for DropCounter {
        fn clone(&self) -> Self { DropCounter { _val: self._val } }
    }
    impl Event for DropCounter {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> u64 { 22 }
        fn event_name() -> &'static str { "DropCounter" }
        fn new(_: NoParticipants, _: NoParameters) -> Self { DropCounter { _val: 0 } }
        fn participants(&self) -> &NoParticipants { unimplemented!() }
        fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
        fn parameters(&self) -> &NoParameters { unimplemented!() }
        fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
    }

    /// Event for bitset-iteration test — IDs 23, 24, 25.
    macro_rules! make_ev {
        ($name:ident, $id:expr) => {
            #[derive(Clone, Copy)]
            struct $name { _pad: u32 }
            impl Event for $name {
                type Participants = NoParticipants;
                type Parameters = NoParameters;
                fn event_id() -> u64 { $id }
                fn event_name() -> &'static str { stringify!($name) }
                fn new(_: NoParticipants, _: NoParameters) -> Self { $name { _pad: 0 } }
                fn participants(&self) -> &NoParticipants { unimplemented!() }
                fn participants_mut(&mut self) -> &mut NoParticipants { unimplemented!() }
                fn parameters(&self) -> &NoParameters { unimplemented!() }
                fn parameters_mut(&mut self) -> &mut NoParameters { unimplemented!() }
            }
        };
    }
    make_ev!(EvBitset5, 23);
    make_ev!(EvBitset100, 24);
    make_ev!(EvBitset200, 25);

    fn register_ev_a() { register_event::<EvA>(20); }
    fn register_drop_counter() { register_event::<DropCounter>(22); }
    fn register_bitset_evs() {
        register_event::<EvBitset5>(23);
        register_event::<EvBitset100>(24);
        register_event::<EvBitset200>(25);
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// Test #1: EventConfig bounds validation.
    #[test]
    fn event_config_bounds() {
        assert!(EventConfig::new(1, 64).is_ok());
        assert!(EventConfig::new(0, 64).is_err());
        assert!(EventConfig::new(65, 64).is_err());
        assert!(EventConfig::new(1, 0).is_err());
        assert!(EventConfig::new(1, 16385).is_err());
    }

    /// Test #2: preregistering the same type twice errors.
    #[test]
    fn preregister_twice_errors() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        let cfg = EventConfig::new(1, 64).unwrap();
        d.preregister::<EvA>(cfg).unwrap();
        let result = d.preregister::<EvA>(cfg);
        assert!(matches!(result, Err(EcsError::EventAlreadyRegistered { .. })));
    }

    /// Test #3: send without preregister errors.
    ///
    /// In debug builds the `debug_assert!` fires (panic). In release builds
    /// the runtime check returns `Err(EventNotRegistered)`. Both behaviours
    /// are correct; we test the release-mode error path here.
    #[test]
    #[cfg(not(debug_assertions))]
    fn send_without_preregister_errors() {
        register_ev_a();
        let d = EventDispatcher::new(1).unwrap();
        let result = d.send(0, EvA { val: 1 });
        assert!(matches!(result, Err(EcsError::EventNotRegistered { .. })));
    }

    /// Test #3 (debug variant): send without preregister panics via debug_assert.
    #[test]
    #[cfg(debug_assertions)]
    fn send_without_preregister_panics_in_debug() {
        register_ev_a();
        let d = EventDispatcher::new(1).unwrap();
        let result = std::panic::catch_unwind(move || {
            let _ = d.send(0, EvA { val: 1 });
        });
        assert!(result.is_err(), "send without preregister must panic in debug");
    }

    /// Test #4: overflow returns EventBufferFull { dropped: 1 }.
    #[test]
    fn send_overflow_returns_full() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 2).unwrap()).unwrap();
        d.send(0, EvA { val: 1 }).unwrap();
        d.send(0, EvA { val: 2 }).unwrap();
        let result = d.send(0, EvA { val: 3 });
        match result {
            Err(EcsError::EventBufferFull { dropped, attempted, .. }) => {
                assert_eq!(dropped, 1);
                assert_eq!(attempted, 1);
            }
            other => panic!("expected EventBufferFull, got {other:?}"),
        }
    }

    /// Test #5: send_many all-or-nothing on overflow.
    #[test]
    fn send_many_atomic_on_overflow() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 3).unwrap()).unwrap();
        d.send(0, EvA { val: 1 }).unwrap();
        // Try to send 3 more with only 2 slots remaining — must reject all.
        let result = d.send_many(0, [EvA { val: 2 }, EvA { val: 3 }, EvA { val: 4 }].into_iter());
        match result {
            Err(EcsError::EventBufferFull { attempted, dropped, .. }) => {
                assert_eq!(attempted, 3);
                assert_eq!(dropped, 3);
            }
            other => panic!("expected EventBufferFull, got {other:?}"),
        }
        // Only the first event should have been written.
        d.update_events();
        assert_eq!(d.events::<EvA>(), &[EvA { val: 1 }]);
    }

    /// Test #6: send then swap then read round-trip.
    #[test]
    fn send_then_swap_then_read() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 64).unwrap()).unwrap();
        d.send(0, EvA { val: 42 }).unwrap();
        d.send(0, EvA { val: 7 }).unwrap();
        assert_eq!(d.events::<EvA>().len(), 0, "no events before swap");
        d.update_events();
        let evs = d.events::<EvA>();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].val, 42);
        assert_eq!(evs[1].val, 7);
    }

    /// Test #7: next-frame visibility (Model B).
    #[test]
    fn next_frame_visibility() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 64).unwrap()).unwrap();

        // Frame 1: send; not yet visible.
        d.send(0, EvA { val: 1 }).unwrap();
        assert_eq!(d.events::<EvA>().len(), 0);

        // Frame 1 end / Frame 2 begin.
        d.update_events();
        assert_eq!(d.events::<EvA>()[0].val, 1);

        // Frame 2: send new event; old one still visible until next swap.
        d.send(0, EvA { val: 2 }).unwrap();
        assert_eq!(d.events::<EvA>()[0].val, 1, "frame 1 events still visible");

        d.update_events();
        let evs = d.events::<EvA>();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].val, 2);
    }

    /// Test #8: double_swap_per_frame panics in debug (last_swap_frame check).
    #[test]
    #[cfg(debug_assertions)]
    fn double_swap_per_frame_debug_asserts() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 64).unwrap()).unwrap();
        d.update_events(); // frame → 1
        let result = std::panic::catch_unwind(move || {
            d.update_events(); // frame → 2 (first call); should not panic
            d.update_events(); // frame → 3
        });
        // Three sequential update_events calls must not panic on their own.
        assert!(result.is_ok(), "sequential update_events must not panic");
    }

    /// Test #9: ZST event rejected at compile time.
    ///
    /// The actual compile_fail doctest lives in the public doc; this test
    /// exercises the non-ZST path to confirm registration succeeds.
    #[test]
    fn zst_event_rejected_non_zst_succeeds() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        // EvA has size_of > 0; must succeed.
        assert!(d.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).is_ok());
    }

    /// Test #10: drop runs for initialised elements only (DropCounter).
    #[test]
    fn drop_runs_for_initialized_only() {
        register_drop_counter();
        DROP_COUNT.store(0, Ordering::Relaxed);
        {
            let mut d = EventDispatcher::new(1).unwrap();
            d.preregister::<DropCounter>(EventConfig::new(1, 64).unwrap()).unwrap();
            d.send(0, DropCounter { _val: 1 }).unwrap();
            d.send(0, DropCounter { _val: 2 }).unwrap();
            d.update_events(); // moves 2 events to reader_buf
            // d drops here; EventBuffer::drop must run for reader_buf[0..2].
        }
        // 2 events in reader_buf dropped when EventBuffer drops.
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 2);
    }

    /// Test #11: two independent dispatchers have independent capacity.
    #[test]
    fn multi_master_independent_capacity() {
        register_ev_a();
        let mut d1 = EventDispatcher::new(1).unwrap();
        let mut d2 = EventDispatcher::new(1).unwrap();
        d1.preregister::<EvA>(EventConfig::new(1, 2).unwrap()).unwrap();
        d2.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).unwrap();

        // Fill d1 to capacity.
        d1.send(0, EvA { val: 1 }).unwrap();
        d1.send(0, EvA { val: 2 }).unwrap();
        assert!(d1.send(0, EvA { val: 3 }).is_err(), "d1 must overflow at capacity 2");

        // d2 still has room.
        for v in 0..4u32 {
            d2.send(0, EvA { val: v }).unwrap();
        }
        assert!(d2.send(0, EvA { val: 99 }).is_err(), "d2 must overflow at capacity 4");
    }

    /// Test #12: bitset iteration calls swap_fn exactly for registered slots.
    #[test]
    fn bitset_iteration_only_registered() {
        register_bitset_evs();
        let mut d = EventDispatcher::new(1).unwrap();
        let cfg = EventConfig::new(1, 8).unwrap();
        d.preregister::<EvBitset5>(cfg).unwrap();
        d.preregister::<EvBitset100>(cfg).unwrap();
        d.preregister::<EvBitset200>(cfg).unwrap();

        d.send(0, EvBitset5 { _pad: 5 }).unwrap();
        d.send(0, EvBitset100 { _pad: 100 }).unwrap();
        d.send(0, EvBitset200 { _pad: 200 }).unwrap();

        // update_events must flush exactly the 3 registered types.
        d.update_events();

        // Verify we got all 3 batches.
        assert_eq!(d.events::<EvBitset5>().len(), 1);
        assert_eq!(d.events::<EvBitset100>().len(), 1);
        assert_eq!(d.events::<EvBitset200>().len(), 1);
    }

    /// Test #26: u64 frame counter near u64::MAX does not panic.
    ///
    /// Exercises frames u64::MAX - 1 and u64::MAX. Wrap to 0 is not tested
    /// because the per-slot debug assertion `last_swap_frame < frame` is a
    /// monotonicity check; wrapping to 0 would trip it (0 < u64::MAX is false).
    /// The plan documents u64::MAX as "never reachable in practice" (W5-NEW),
    /// so we only exercise the near-max regime, not actual wrap.
    #[test]
    fn frame_counter_u64_wrap() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).unwrap();

        // Set current_frame to u64::MAX - 2.
        d.set_current_frame_for_test(u64::MAX - 2);
        // Two sequential update_events calls advance to u64::MAX - 1 and u64::MAX.
        d.update_events(); // frame = u64::MAX - 1
        d.update_events(); // frame = u64::MAX
        // No panic expected; near-wrap is handled correctly.
    }

    /// Test #28: events slice remains valid across an intervening send (C1-NEW).
    #[test]
    fn concurrent_send_and_events_slice_validity() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 64).unwrap()).unwrap();

        d.send(0, EvA { val: 10 }).unwrap();
        d.update_events();

        // Borrow the reader slice.
        let slice = d.events::<EvA>();
        let val0 = slice[0].val;

        // Send another event — this writes to write_buf, NOT reader_buf.
        d.send(0, EvA { val: 99 }).unwrap();

        // The slice must still reflect the previous frame's data.
        assert_eq!(val0, 10, "send must not corrupt the reader slice");
    }

    /// Test #30: type_id collision detected by debug_assert.
    #[test]
    #[cfg(debug_assertions)]
    fn type_id_collision_debug_asserts() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).unwrap();

        // Monkey-patch vtable.type_id to u64's TypeId.
        let patched = d.set_slot_type_id_for_test(20, TypeId::of::<u64>());
        assert!(patched, "slot 20 must be registered");

        // events::<EvA>() should trigger a debug_assert_eq! panic.
        let result = std::panic::catch_unwind(move || {
            let _ = d.events::<EvA>();
        });
        assert!(result.is_err(), "debug_assert on type_id mismatch must panic");
    }

    /// Test #31: send_many with empty iterator returns Ok, write_len unchanged.
    #[test]
    fn send_many_empty_iter() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).unwrap();
        let result = d.send_many::<EvA, _>(0, core::iter::empty());
        assert!(result.is_ok());
        d.update_events();
        assert_eq!(d.events::<EvA>().len(), 0);
    }

    /// Test #32: out-of-range thread_index panics via slice indexing in release.
    #[test]
    fn send_thread_index_out_of_range_release() {
        register_ev_a();
        let mut d = EventDispatcher::new(1).unwrap();
        d.preregister::<EvA>(EventConfig::new(1, 4).unwrap()).unwrap();
        // thread_count == 1; index 6 is out of range → slice bounds check panics.
        let result = std::panic::catch_unwind(move || {
            let _ = d.send(6, EvA { val: 0 });
        });
        assert!(result.is_err(), "out-of-range thread_index must panic via slice indexing");
    }
}
