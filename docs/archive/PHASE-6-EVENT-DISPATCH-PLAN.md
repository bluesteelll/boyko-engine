> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 6 — Event dispatch + double-buffer (Round 3)

## Changes from Round 2

| Critic ID | Round 2 problem | Round 3 fix | Section |
|---|---|---|---|
| **C1-NEW** | `send(&self, …)` mutates `Box<[MaybeUninit<E>]>` through `&self`-derived `*const` cast to `*mut` — UB under Stacked/Tree Borrows | `ThreadLaneWriter::write_buf` now wrapped in `UnsafeCell<Box<[MaybeUninit<E>]>>`. All writer mutation goes through `UnsafeCell::get()`; per-thread exclusivity invariant is documented as the interior-mutability justification. | Data structures → `ThreadLaneWriter`, Unsafe contracts → U4 (rewritten), U1 (rewritten), U9 (rewritten) |
| **C2-NEW** | `const _: () = assert!(size_of::<E>() > 0)` inside a generic fn body does not compile | ZST rejection moved to an associated const on a generic helper: `ZstCheck::<E>::NON_ZERO`. Reading the const at register site forces monomorphisation-time evaluation; passes through to `const-eval` and rejects ZSTs at build time. | Decisions → D9 (rewritten), Lifecycle → Preregister, Open notes → "ZST guard" |
| **W1-NEW** | `ThreadLaneReader` layout produced 128 B not 64 B (field ordering issue) | Reordered to `{ reserved_ptr: AtomicPtr<…>, read_cursor: AtomicU32, _pad: [u8; 52] }` → 64 B. Compile-time assert tightened. | Data structures → `ThreadLaneReader` |
| **W2-NEW** | Memory upper bound stated 16 GB but missed `×2` for reader_buf — actual ~32 GiB | Memory budget table updated. Upper bound clarified as ~32 GiB (lane writers + flattened reader buf). Realistic budgets unchanged. | Performance → Memory budget |
| **W3-NEW** | `TypeId` is 16 B (u128 since rust#75923/PR#109953 landed in 1.72), not 8 B | `EventVTable` recalculated: 8 + 8 + 16 + 16 = 48 B. `EventTypeSlot` (release) = 8 + 48 + 4 + 4 = 64 B (still passes `align(64)`). Debug = 64 + 4 + 4 → align(64) → 128 B. Compile-time asserts use `>=` rather than `==` to avoid breaking if `TypeId` changes again; only release-mode tight check is kept. | Data structures → `EventVTable`, `EventTypeSlot` |
| **W4-NEW** | U4 contract didn't enumerate interior-mutability primitive | U4 rewritten: explicitly cites `UnsafeCell<Box<[MaybeUninit<E>]>>` and the per-thread exclusivity invariant. Also added U13 covering reader-side reads of the UnsafeCell during swap. | Unsafe contracts → U4 (rewritten), U13 (new) |
| **W5-NEW** | `current_frame: u32` wrap silently breaks debug double-swap check | Widened `current_frame` to `u64`. 16 EB before wrap → never relevant. Cost: 4 extra bytes in dispatcher header. | Data structures → `EventDispatcher`, Lifecycle → Swap |
| **W6-NEW** | `drop_fn` vs explicit `dealloc(data, layout)` ambiguous — double-free risk | **Decision (b)**: `drop_fn` reconstructs the `Box` and lets `Box`'s `Drop` both drop AND deallocate. `layout` field removed from `EventVTable`. `EventVTable` shrinks to 32 B. `EventTypeSlot` release = 8 + 32 + 4 + 4 = 48 B → padded to 64 B by `align(64)`. | Data structures → `EventVTable`, `EventTypeSlot`, Lifecycle → Drop |
| **W7-NEW** | `MaybeUninit::slice_assume_init_ref` is not stabilised in Rust 1.93/1.95 stable | Replaced with stable construction: `std::slice::from_raw_parts(buf.reader_buf.as_ptr().cast::<E>(), len)`. Documented as the canonical pre-stabilisation form. | Lifecycle → Read, Unsafe contracts → U11 (rewritten) |
| **W8-NEW** | `Box::as_ptr` / `Box::as_mut_ptr` still unstable (rust#129090) | All writer-side pointer derivation goes through `UnsafeCell::get()` (post-C1-NEW) yielding `*mut Box<[MaybeUninit<E>]>`, then `(*box_ptr).as_ptr()` / `.as_mut_ptr()` via slice-deref (stable). Reader side uses `(*self.reader_buf).as_ptr()` (slice-deref form). Documented under unsafe contracts U1 and U9. | Lifecycle → Send, Swap, Unsafe contracts → U1, U9 |
| **W9-NEW** | `BitSet256` and `pop_lowest` referenced but don't exist in `boyko_utils` | **Phase 6 Step 0 (new)**: spec for `BitSet256` + `pop_lowest_set_bit` in `boyko_utils/src/bit_mask/bit_set_256.rs`. Full struct, ops, SAFETY contracts laid out below. | New section: Phase 6 Step 0 — `boyko_utils` BitSet256 spec |
| **W10-NEW** | `preregister_event_default` body unspecified | Implementation supplied in API surface. `EventConfig::DEFAULT_FOR(thread_count)` constructor added (validated via `EventConfig::new`) so the proxy can honestly `.expect("validated at dispatcher construction")`. | API surface → `EcsMaster::preregister_event_default`, `EventConfig::DEFAULT_FOR` |
| **Cleanup "ZST mirror"** | R2 said "mirrors ComponentPool" which uses `debug_assert!` not const | D9 rewritten. ComponentPool precedent honestly cited as runtime debug_assert. ZstCheck approach is documented as our stronger compile-time guarantee. | Decisions → D9 |
| **Cleanup M2 row text** | M2 row contradicted API signature (`&mut self` vs `&self`) | Row text corrected: both `events::<E>()` and `send::<E>()` borrow `&self`; the cross-system conflict is a scheduler concern (Phase 7), not a borrow-checker concern within a single system. | Changes-from-R1 table → row M2 (in this Round 3 doc, see archival reference below) |

### Carried forward from Round 1 → Round 2 (unchanged in Round 3)

All R1→R2 fixes (C1, C2, C3, C4, C5, C6, C7, C8, C9, W1, W2, W3, W4, W5, W6, W7, W8, M1, M2-corrected, M4, P1, P2, P3, P4, P5, U1-U6, Q1, Q4, Q6) remain as designed in R2 except where Round 2 critic findings above modify them.

---

## Phase 6 Step 0 — `boyko_utils` BitSet256 spec (W9-NEW)

This is **new work** that must land before any Phase 6 dispatcher code can compile.

### File
`crates/boyko_utils/src/bit_mask/bit_set_256.rs` (new)
+ register in `crates/boyko_utils/src/bit_mask/mod.rs`: `pub mod bit_set_256;`

### Struct
```rust
/// Fixed-size 256-bit bitset, four u64 words.
/// Aligned to 32 B so the whole set fits in a single AVX2 register and
/// straddles at most one cache line on x86_64 (32 < 64).
#[derive(Copy, Clone, PartialEq, Eq, Default)]
#[repr(C, align(32))]
pub struct BitSet256 {
    /// Little-endian word order: bit `i` lives in `words[i >> 6]`
    /// at position `i & 63`. Word 0 carries bits 0..63.
    words: [u64; 4],
}

const _: () = assert!(core::mem::size_of::<BitSet256>() == 32);
const _: () = assert!(core::mem::align_of::<BitSet256>() == 32);
```

### Required operations

```rust
impl BitSet256 {
    /// All-zeros constructor.
    #[inline] pub const fn new() -> Self { Self { words: [0; 4] } }

    /// Sets bit `index` to 1. Panics in debug if `index >= 256`.
    #[inline] pub fn set(&mut self, index: usize);

    /// Clears bit `index` to 0. Panics in debug if `index >= 256`.
    #[inline] pub fn clear(&mut self, index: usize);

    /// True if bit `index` is 1. Panics in debug if `index >= 256`.
    #[inline] pub fn get(&self, index: usize) -> bool;

    /// True iff no bits are set.
    #[inline] pub fn is_empty(&self) -> bool;

    /// Number of bits set (popcount of all four words).
    #[inline] pub fn count_ones(&self) -> u32;

    /// Removes and returns the lowest set bit's index, or None if empty.
    /// Implemented as tzcnt of the lowest non-zero word, then clear that bit.
    /// O(1); on x86_64 with BMI1 lowers to BLSR + TZCNT (1-3 cycles).
    ///
    /// Designed for sparse iteration:
    /// ```ignore
    /// let mut m = self.registered_mask;
    /// while let Some(id) = m.pop_lowest_set_bit() { ... }
    /// ```
    /// Each call is amortised O(1); total cost for k set bits is O(k),
    /// not O(256). Reads at most one cache line.
    #[inline]
    pub fn pop_lowest_set_bit(&mut self) -> Option<u32>;
}
```

### Reference implementation sketch (for the developer)

```rust
#[inline]
pub fn set(&mut self, index: usize) {
    debug_assert!(index < 256, "bit index out of range");
    self.words[index >> 6] |= 1u64 << (index & 63);
}

#[inline]
pub fn clear(&mut self, index: usize) {
    debug_assert!(index < 256, "bit index out of range");
    self.words[index >> 6] &= !(1u64 << (index & 63));
}

#[inline]
pub fn get(&self, index: usize) -> bool {
    debug_assert!(index < 256, "bit index out of range");
    (self.words[index >> 6] >> (index & 63)) & 1 == 1
}

#[inline]
pub fn is_empty(&self) -> bool {
    (self.words[0] | self.words[1] | self.words[2] | self.words[3]) == 0
}

#[inline]
pub fn count_ones(&self) -> u32 {
    self.words[0].count_ones()
        + self.words[1].count_ones()
        + self.words[2].count_ones()
        + self.words[3].count_ones()
}

#[inline]
pub fn pop_lowest_set_bit(&mut self) -> Option<u32> {
    // Scan words 0..4 for first non-zero.
    // Loop body is bounded (max 4 iterations); compiler unrolls.
    for w in 0..4 {
        let word = self.words[w];
        if word != 0 {
            let bit = word.trailing_zeros();
            // Clear lowest set bit: BLSR-equivalent (x & (x - 1)).
            self.words[w] = word & word.wrapping_sub(1);
            return Some((w as u32) * 64 + bit);
        }
    }
    None
}
```

### Tests required for BitSet256 (unit, in `bit_set_256.rs`)

- `set_get_clear_basic`: set bits 0, 63, 64, 127, 255; verify `get`.
- `bounds_panic_debug`: `set(256)` panics in debug.
- `count_ones_matches_sets`: random sets + clears, popcount = manual count.
- `pop_lowest_empty_returns_none`: empty set returns `None`.
- `pop_lowest_iteration_order`: set bits {200, 5, 100}; pop yields 5, 100, 200 in order; emptied to None.
- `pop_lowest_consumes`: after popping all set bits, `is_empty()` true.

---

## Design summary

Phase 6 adds **typed event dispatch with strict next-frame visibility (Model B)** to the existing `EventRegistry` infrastructure.

Each event type `E` has a **per-master `EventBuffer<E>`** consisting of:
- N **writer lanes** (one per worker thread, future-proof for parallel sends). Each writer holds an `UnsafeCell<Box<[MaybeUninit<E>]>>` write region.
- One **flat reader buffer** `Box<[MaybeUninit<E>]>` of size `N * capacity_per_lane`, populated by `update_events` via per-lane memcpy.

`update_events` runs once per frame on the main thread:
1. Iterate `registered_mask: BitSet256` over registered event types via `pop_lowest_set_bit`.
2. For each set bit, call a per-type `swap_fn: unsafe fn(*mut u8, u64)` stored in the slot — no `dyn`, no vtable.
3. The `swap_fn` (monomorphised `fn swap_and_flatten::<E>`) walks each lane's `write_len`, memcpys to the reader buffer, resets `write_len = 0`, sets `reader_len = total_written`.

Readers call `dispatcher.events::<E>() -> &[E]` and iterate a single contiguous slice — no `FlatMap`, zero abstraction cost beyond a slice borrow.

Storage lives **outside** the `Arena` (compatible with both chunked-grow and full-realloc arena strategies). Per-type memory is allocated once at `preregister_event::<E>` time and never re-allocated during steady-state.

---

## Decisions and rationale

### D1. Double buffer via two per-type allocations, swap by pointer index — unchanged
**What**: each `EventBuffer<E>` owns per-lane write regions (writer side) and a flat `reader_buf` (reader side); swap is per-frame memcpy from lanes into `reader_buf`.
**Why**: zero-copy publish via `reader_len.store(_, Release)`; writes always go to per-thread lanes (no shared head/tail contention).
**Alternatives rejected**:
- Ring buffer per type: shared head/tail counters → CAS contention; can't guarantee contiguous read.
- Single buffer + `Vec::clear()`: kills next-frame visibility.

### D2. `Event: Send + Sync` supertrait — unchanged
```rust
pub trait Event: 'static + Sized + Send + Sync {
    type Participants: Participants;
    type Parameters: Parameters;
    // … unchanged
}
```
Auto-derives `Send + Sync` for `EventBuffer<E>`. No manual `unsafe impl`.

### D3. Per-type buffer storage outside arena — unchanged
`Box<[MaybeUninit<E>]>` per lane and reader buffer; global allocator. Arena growth invisible.

### D4. Per-thread lane partitioning with split cache lines — **refined (R3, W1-NEW)**
**What**: `EventBuffer<E>` owns `Box<[ThreadLanePair<E>]>` where `ThreadLanePair<E>` = `{ writer: ThreadLaneWriter<E>, reader: ThreadLaneReader<E> }`. Each half is `#[repr(C, align(64))]` and consumes exactly one cache line; ordering of fields in `ThreadLaneReader` chosen so the 8-byte `AtomicPtr` is first (no internal padding before `read_cursor`).
**Why**: Phase 7 may pin writer and reader on different threads; sharing a cache line between halves causes false sharing on every increment + iterate. 64 extra bytes per lane is trivial.
**Trade-off**: 128 B of control overhead per lane × 16 lanes × 256 types = 524 KB total — negligible against payload.

### D5. Static metadata + per-master config separation — unchanged
- **Static** `EVENT_INFO: [OnceLock<EventInfo>; 256]`: type_id, type_name, participant_info (immutable).
- **Per-master** `EventDispatcher`: `slots: [EventTypeSlotStorage; 256]` + `registered_mask: BitSet256` (mutable, per-master capacity).
12 KB slot array per master. No multi-master race.

### D6. Function-pointer dispatch (no `dyn`) — **refined (R3, W3-NEW, W6-NEW)**
**What**:
```rust
struct EventVTable {
    swap_fn: unsafe fn(slot_data: *mut u8, frame: u64),
    drop_fn: unsafe fn(slot_data: *mut u8), // calls Box::from_raw, runs Drop + dealloc
    type_id: TypeId, // 16 B (u128) — runtime sanity check
}
```
`drop_fn` does both `Drop` and deallocation by reconstructing `Box::from_raw(data as *mut EventBuffer<E>)`. `layout` field removed (W6-NEW decision (b)).
**Why fn-ptr over `dyn`**: 256 indirect calls via fn-ptr touch only the slot's own cache line; `dyn Trait` adds a separate vtable cache line per type — 256 extra L1d misses per frame on cold-cache dispatch.
**Trade-off**: one indirect branch per type per frame (~3 ns warm IBP).

### D7. `send_many` is all-or-nothing — unchanged
`send_many<I: ExactSizeIterator<Item = E>>(...)` pre-checks `iter.len() <= capacity_remaining`; rejects whole batch on overflow with `EventBufferFull { attempted, dropped: attempted }`.

### D8. Flattened reader buffer with per-lane memcpy on swap — unchanged
Per-type single `reader_buf: Box<[MaybeUninit<E>]>` of size `thread_count * capacity_per_lane`. `swap_and_flatten` walks lanes and memcpys; readers see one contiguous `&[E]`.

### D9. ZST events rejected at compile time via associated const — **rewritten (R3, C2-NEW)**
**What**: helper struct with a generic associated const that fails `const-eval` for ZSTs:
```rust
struct ZstCheck<E: Event>(PhantomData<E>);

impl<E: Event> ZstCheck<E> {
    const NON_ZERO: () = assert!(
        core::mem::size_of::<E>() > 0,
        "Event type is zero-sized; use a counter instead",
    );
}

pub fn preregister<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
    // Force monomorphisation-time evaluation of the associated const.
    // This trips the const assert at build time for ZST E.
    let _ = ZstCheck::<E>::NON_ZERO;
    // … rest of body
}
```
**Why**: a `const _: () = …` item *inside* a generic function body cannot reference outer generics — Rust rejects it. Associated const on a generic helper is the canonical pattern (used by `bytemuck`, `static_assertions::const_assert!`).
**Honest precedent note**: `ComponentPool::new` (`crates/boyko_ecs/src/ecs/memory/component_pool.rs:84`) uses a **runtime `debug_assert!`** for the same purpose. ComponentPool is constructed dynamically with a `component_layout: Layout` not a generic type parameter, so it cannot use the `ZstCheck` pattern. Phase 6's `preregister::<E>` is generic, so we can go further and enforce at compile time. A follow-up ticket may revisit ComponentPool if it ever gains a generic constructor.
**Trade-off**: cannot use ZST events. Workaround: add `_unit: u8` field.

### D10. Capacity bounds checked at config construction — unchanged
`EventConfig::new(thread_count, capacity_per_lane) -> EcsResult<Self>`. Bounds: `1..=MAX_EVENT_THREADS` (64), `1..=MAX_EVENT_CAPACITY` (16384).

### D11. Preregister API mandatory in release — unchanged
`EcsMaster::preregister_event::<E>(cfg)` required before first `send::<E>()`. Debug build: `debug_assert!`. Release build: `Err(EventNotRegistered)`. No lazy auto-registration.

### D12. Per-frame `update_events` epoch check (debug only) — **refined (R3, W5-NEW)**
**What**: `EventDispatcher.current_frame: u64` (was `u32`). Per-slot `last_swap_frame: u64` (debug only). `update_events(&mut self)` increments `current_frame` and asserts `slot.last_swap_frame < current_frame`, then writes it.
**Why**: u32 wrap at ~4.3 G frames is reachable on long-running servers (49 days at 1000 fps). u64 wrap (~16 EB frames) is never reachable. Cost: 4 extra bytes in dispatcher + 4 extra bytes per slot (debug only).

---

## Data structures

All sizes computed for x86_64. Cache line = 64 B. `TypeId` is 16 B (u128, stabilised in 1.72).

### `EventVTable` — 32 B (was 40 B in R2; W3-NEW + W6-NEW)
```rust
#[repr(C)]
struct EventVTable {
    /// Called by `update_events`. Reads write halves, flattens to reader_buf,
    /// resets write halves. `frame` = current dispatcher frame.
    swap_fn: unsafe fn(slot_data: *mut u8, frame: u64),
    /// Called by `EventDispatcher::drop`. Reconstructs `Box<EventBuffer<E>>`
    /// from the raw pointer and drops it — `Box`'s `Drop` runs `EventBuffer::drop`
    /// AND deallocates. (W6-NEW decision (b): no separate `dealloc` step.)
    drop_fn: unsafe fn(slot_data: *mut u8),
    /// Runtime type check on typed access. 16 B (u128).
    type_id: TypeId,
}

const _: () = assert!(core::mem::size_of::<EventVTable>() <= 32);
// Tight check: 8 + 8 + 16 = 32 on current Rust. If TypeId ever changes again,
// the `<= 32` keeps build green but a stale assumption surfaces here; revisit.
```

### `EventTypeSlot` — 64 B (release) / 80 B (debug)
```rust
#[repr(C, align(64))]
struct EventTypeSlot {
    /// Points to `Box<EventBuffer<E>>::into_raw()`.
    data: *mut u8,           // 8 B
    vtable: EventVTable,     // 32 B
    capacity_per_lane: u32,  // 4 B
    thread_count: u32,       // 4 B
    // Release: 48 B → align(64) → 64 B total (16 B tail padding).
    #[cfg(debug_assertions)]
    last_swap_frame: u64,    // 8 B (was u32 in R2, W5-NEW)
    #[cfg(debug_assertions)]
    events_swapped_unread: u32, // 4 B
    // Debug: 48 + 12 = 60 B → align(64) → 64 B (tight) but compiler may need
    // extra alignment padding for u64; observed 64 B in practice. The `>= 64`
    // assertion lets the layout breathe if Rust adds padding later.
}

const _: () = assert!(core::mem::align_of::<EventTypeSlot>() == 64);
const _: () = assert!(core::mem::size_of::<EventTypeSlot>() % 64 == 0);
#[cfg(not(debug_assertions))]
const _: () = assert!(core::mem::size_of::<EventTypeSlot>() == 64);
```
**Note on assertion strategy (W3-NEW)**: we no longer assert exact equality in debug — if `TypeId` or u64 alignment rules change, the `% 64 == 0` and `>= 64` patterns keep CI green while still proving the cache-line discipline. Release-mode tight check remains.

### `BitSet256` — 32 B (spec in Phase 6 Step 0)
Lives in `boyko_utils::bit_mask::bit_set_256`. Used here for `registered_mask` field.

### `EventDispatcher` — fixed ~16.5 KB per master
```rust
pub struct EventDispatcher {
    /// Dense slot array. Index = EventId (0..MAX_EVENTS).
    /// 256 × 64 B = 16 KB. Cache-friendly linear scan with bitset skip.
    slots: Box<[EventTypeSlotStorage; MAX_EVENTS]>,
    /// Tracks which slots are populated. 32 B. Iterated with tzcnt via pop_lowest_set_bit.
    registered_mask: BitSet256,
    /// Monotonic per-frame counter. u64 wrap is never reachable (W5-NEW).
    current_frame: u64,
    /// Default thread count used when `preregister_event_default` proxies in.
    default_thread_count: u32,
}

/// Storage wrapper. Only valid when `registered_mask.get(index) == true`.
/// We use `MaybeUninit` + bitset rather than `Option` to keep `EventTypeSlot`
/// 64-byte aligned (Option would add 8 B discriminant + padding).
#[repr(C)]
struct EventTypeSlotStorage {
    slot: MaybeUninit<EventTypeSlot>,
}
```

**Memory layout rationale** — unchanged from R2 except `current_frame` is u64.

### `EventBuffer<E>` — per-type owner
```rust
pub struct EventBuffer<E: Event> {
    /// One per worker thread. Length == thread_count.
    /// Each pair is two cache lines (writer + reader).
    lanes: Box<[ThreadLanePair<E>]>,
    /// Flat reader buffer; size = thread_count * capacity_per_lane.
    /// Populated by swap_and_flatten (D8). The `Box` is owned by EventBuffer;
    /// no UnsafeCell needed because update_events takes `&mut self` on the
    /// dispatcher, providing exclusive access to the whole buffer at swap time.
    reader_buf: Box<[MaybeUninit<E>]>,
    /// Initialised prefix length after the last swap. Atomic because
    /// readers may run on different threads than the swap (Phase 7).
    reader_len: AtomicU32,
    capacity_per_lane: u32,
    thread_count: u32,
    _marker: PhantomData<E>,
}
```

### `ThreadLanePair<E>` — 128 B per lane
```rust
#[repr(C, align(64))]
struct ThreadLanePair<E: Event> {
    writer: ThreadLaneWriter<E>, // 64 B
    reader: ThreadLaneReader<E>, // 64 B
}
```

### `ThreadLaneWriter<E>` — 64 B (C1-NEW: write_buf now in UnsafeCell)
```rust
#[repr(C, align(64))]
struct ThreadLaneWriter<E: Event> {
    /// Write region. Owned exclusively by the writer of `thread_index`.
    /// Wrapped in UnsafeCell so that `&self`-receiver methods (`send_one`)
    /// can legally mutate the underlying box's contents via `UnsafeCell::get()`.
    ///
    /// Per-thread exclusivity invariant: only one logical worker thread —
    /// the one assigned `thread_index` — touches this UnsafeCell. The
    /// dispatcher swap path takes `&mut self` on `EventDispatcher`, which
    /// is a synchronisation point with all writers (Phase 7 scheduler
    /// guarantees no sends are in flight during update_events).
    ///
    /// Box is the sole owner of the heap allocation; `(*box).as_ptr()`
    /// yields a stable pointer for the box's lifetime.
    write_buf: UnsafeCell<Box<[MaybeUninit<E>]>>, // 16 B (UnsafeCell is transparent)
    /// AtomicU32 because Phase 7 will have writer (worker thread) and
    /// swap reader (main thread) accessing this from different threads,
    /// even though only one writer touches it at any one frame's send phase.
    write_len: AtomicU32, // 4 B
    /// Debug-only overflow counter; bumped when send rejects.
    overflow_count: AtomicU32, // 4 B
    /// Mirrors EventBuffer.capacity_per_lane for locality in the send hot path.
    capacity: u32, // 4 B
    /// 64 − (16 + 4 + 4 + 4) = 36 B padding to fill the cache line.
    _pad: [u8; 36],
}

const _: () = assert!(core::mem::size_of::<ThreadLaneWriter<u32>>() == 64);
const _: () = assert!(core::mem::align_of::<ThreadLaneWriter<u32>>() == 64);
```

**`UnsafeCell` transparency note**: `UnsafeCell<T>` is `#[repr(transparent)]`; `size_of::<UnsafeCell<T>>() == size_of::<T>()` and `align_of` is the same. `Box<[MaybeUninit<E>]>` is `{ ptr, len }` = 16 B on 64-bit. Wrapping in `UnsafeCell` doesn't change the size.

**Send / Sync**: `UnsafeCell<T>: !Sync` by default. We need `ThreadLaneWriter: Sync` for the case where one thread reads `write_len` while another wrote it — but the `Box<[MaybeUninit<E>]>` contents are only ever accessed by the owning thread. Add a documented manual impl:
```rust
// SAFETY: per-thread exclusivity. The UnsafeCell is only accessed by the
// worker assigned thread_index for writes and by the main thread during
// update_events when no writers are in flight (sync point).
unsafe impl<E: Event> Sync for ThreadLaneWriter<E> {}
```

### `ThreadLaneReader<E>` — 64 B (W1-NEW: reordered fields)
```rust
#[repr(C, align(64))]
struct ThreadLaneReader<E: Event> {
    /// Reserved for Phase 7 (per-lane snapshot pointer).
    /// Placed first to align naturally on 8 B without internal padding.
    reserved_ptr: AtomicPtr<MaybeUninit<E>>, // 8 B
    /// Per-lane reader-side cursor for swap_and_flatten.
    /// Phase 7 streaming-read APIs will checkpoint progress here
    /// without touching the writer line.
    read_cursor: AtomicU32, // 4 B
    /// 64 − (8 + 4) = 52 B padding.
    _pad: [u8; 52],
}

const _: () = assert!(core::mem::size_of::<ThreadLaneReader<u32>>() == 64);
const _: () = assert!(core::mem::align_of::<ThreadLaneReader<u32>>() == 64);
const _: () = assert!(core::mem::size_of::<ThreadLanePair<u32>>() == 128);
const _: () = assert!(core::mem::align_of::<ThreadLanePair<u32>>() == 64);
```

**False sharing analysis**: writer touches `writer.write_buf` (UnsafeCell load + content write), `writer.write_len` (RMW), `writer.capacity` (load). Reader (main thread during swap) touches `reader.read_cursor`. These are on separate cache lines. The single time the main thread reaches into the writer line is during `swap_and_flatten` under the convention that workers have stopped sending — false sharing irrelevant there.

### `EventConfig` — value type with new `DEFAULT_FOR` constructor (W10-NEW)
```rust
#[derive(Clone, Copy)]
pub struct EventConfig {
    pub(crate) thread_count: u32,
    pub(crate) capacity_per_lane: u32,
}

impl EventConfig {
    pub const DEFAULT_CAPACITY: u32 = 1024;
    pub const DEFAULT: EventConfig = EventConfig {
        thread_count: 1,
        capacity_per_lane: Self::DEFAULT_CAPACITY,
    };

    /// Validating constructor.
    pub fn new(thread_count: u32, capacity_per_lane: u32) -> EcsResult<Self> {
        if thread_count == 0 || thread_count > MAX_EVENT_THREADS {
            return Err(EcsError::InvalidEventConfig {
                reason: "thread_count out of range",
            });
        }
        if capacity_per_lane == 0 || capacity_per_lane > MAX_EVENT_CAPACITY {
            return Err(EcsError::InvalidEventConfig {
                reason: "capacity_per_lane out of range",
            });
        }
        Ok(EventConfig { thread_count, capacity_per_lane })
    }

    /// Convenience: build a config with the given thread_count and the default
    /// per-lane capacity. Used by `EcsMaster::preregister_event_default`.
    ///
    /// Validates `thread_count` via `EventConfig::new`, so a caller that
    /// constructed `EcsMaster` successfully (which validated its own
    /// `default_thread_count`) can `.expect(...)` honestly.
    #[inline]
    pub fn default_for(thread_count: u32) -> EcsResult<Self> {
        Self::new(thread_count, Self::DEFAULT_CAPACITY)
    }
}

pub const MAX_EVENT_THREADS: u32 = 64;
pub const MAX_EVENT_CAPACITY: u32 = 16384;
```

---

## Lifecycle

### Construction
1. `EcsMaster::new()` constructs `EventDispatcher::new(default_thread_count = 1)`.
2. `EventDispatcher::new` validates `default_thread_count` via `EventConfig::default_for(default_thread_count)?` (stores it after the validation). Returns `EcsResult<Self>`.
3. Allocates the 16 KB `Box<[EventTypeSlotStorage; 256]>` with all `MaybeUninit::uninit()`; `registered_mask` is all zeros; `current_frame = 0`.

### Preregister (D9 rewritten, C2-NEW)
```rust
// Helper struct lives in event_dispatcher.rs scope.
struct ZstCheck<E: Event>(PhantomData<E>);
impl<E: Event> ZstCheck<E> {
    const NON_ZERO: () = assert!(
        core::mem::size_of::<E>() > 0,
        "Event type is zero-sized; use a counter instead",
    );
}

pub fn preregister<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
    // C2-NEW: force monomorphisation-time const-eval of ZstCheck.
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

    // SAFETY (U2): see Unsafe contracts.
    unsafe { self.slots[id].slot.write(slot); }
    self.registered_mask.set(id);
    Ok(())
}
```

### Send (C1-NEW + W8-NEW: UnsafeCell + stable Box deref)
```rust
#[inline]
pub fn send<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
    let id = E::event_id() as usize;
    debug_assert!(id < MAX_EVENTS);
    debug_assert!(
        self.registered_mask.get(id),
        "preregister_event::<E> must be called before send",
    );

    if !self.registered_mask.get(id) {
        return Err(EcsError::EventNotRegistered { type_name: type_name::<E>() });
    }
    // SAFETY (U3): see Unsafe contracts.
    let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
    debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>());
    // SAFETY (U7-read): see Unsafe contracts (slot.data typed cast).
    let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };
    buf.send_one(thread_index, event)
}
```

`EventBuffer::send_one` — rewritten to use `UnsafeCell` (C1-NEW):
```rust
#[inline]
fn send_one(&self, thread_index: u32, event: E) -> EcsResult<()> {
    debug_assert!(thread_index < self.thread_count);
    let lane = &self.lanes[thread_index as usize].writer;
    let len = lane.write_len.load(Ordering::Relaxed); // single writer per lane
    if len >= lane.capacity {
        lane.overflow_count.fetch_add(1, Ordering::Relaxed);
        return Err(EcsError::EventBufferFull {
            type_name: type_name::<E>(),
            thread_index,
            attempted: 1,
            dropped: 1,
        });
    }
    // SAFETY (U4 rewritten, U1 rewritten): see Unsafe contracts.
    unsafe {
        // Get raw pointer to the Box through UnsafeCell.
        // UnsafeCell::get() returns *mut Box<[MaybeUninit<E>]>.
        let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
        // Use slice deref to get *mut MaybeUninit<E> (Box::as_mut_ptr unstable).
        // (**box_ptr) materialises &mut [MaybeUninit<E>] — fine because UnsafeCell
        // guarantees no concurrent reads of the same byte region; per-thread
        // exclusivity invariant guarantees no other writer of this lane.
        let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
        let dst: *mut MaybeUninit<E> = buf_ptr.add(len as usize);
        (*dst).write(event);
    }
    lane.write_len.store(len + 1, Ordering::Release);
    Ok(())
}
```

### `send_many` (D7)
```rust
pub fn send_many<E: Event, I>(&self, thread_index: u32, iter: I) -> EcsResult<()>
where
    I: ExactSizeIterator<Item = E>,
{
    let id = E::event_id() as usize;
    debug_assert!(id < MAX_EVENTS);
    if !self.registered_mask.get(id) {
        return Err(EcsError::EventNotRegistered { type_name: type_name::<E>() });
    }
    // SAFETY (U3): see Unsafe contracts.
    let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
    debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>());
    let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };

    debug_assert!(thread_index < buf.thread_count);
    let lane = &buf.lanes[thread_index as usize].writer;
    let len = lane.write_len.load(Ordering::Relaxed);
    let n = iter.len() as u32;
    let remaining = lane.capacity.saturating_sub(len);
    if n == 0 {
        return Ok(()); // Test 31: empty iterator trivially succeeds.
    }
    if n > remaining {
        lane.overflow_count.fetch_add(1, Ordering::Relaxed);
        return Err(EcsError::EventBufferFull {
            type_name: type_name::<E>(),
            thread_index,
            attempted: n,
            dropped: n,
        });
    }
    // All-or-nothing: pre-check passed, write all.
    // SAFETY (U4-batch): same contract as U4 per element; len + i < capacity by pre-check.
    unsafe {
        let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
        let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
        for (i, e) in iter.enumerate() {
            (*buf_ptr.add(len as usize + i)).write(e);
        }
    }
    lane.write_len.store(len + n, Ordering::Release);
    Ok(())
}
```

### Swap (per frame; W5-NEW: u64 frame; W6-NEW: drop_fn does dealloc)
```rust
pub fn update_events(&mut self) {
    self.current_frame = self.current_frame.wrapping_add(1);
    let frame = self.current_frame;

    let mut mask = self.registered_mask;
    while let Some(id) = mask.pop_lowest_set_bit() {
        // SAFETY (U5): see Unsafe contracts.
        let slot: &mut EventTypeSlot = unsafe {
            self.slots[id as usize].slot.assume_init_mut()
        };
        #[cfg(debug_assertions)]
        {
            // u64 frame: wrap never reachable in practice.
            assert!(
                slot.last_swap_frame < frame,
                "double-swap detected for event slot {id}",
            );
            slot.last_swap_frame = frame;
        }
        // SAFETY (U6): see Unsafe contracts.
        unsafe { (slot.vtable.swap_fn)(slot.data, frame); }
    }
}
```

The monomorphised swap function (one copy per event type):
```rust
unsafe fn swap_and_flatten<E: Event>(data: *mut u8, _frame: u64) {
    // SAFETY (U7): see Unsafe contracts.
    let buf: &mut EventBuffer<E> = unsafe { &mut *(data as *mut EventBuffer<E>) };

    // Drop previous frame's reader_buf contents (if E: Drop).
    let prev_len = buf.reader_len.load(Ordering::Relaxed) as usize;
    if core::mem::needs_drop::<E>() {
        // SAFETY (U8): see Unsafe contracts.
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
        let n = writer.write_len.swap(0, Ordering::AcqRel) as usize;
        if n == 0 { continue; }
        // SAFETY (U9 rewritten, U13): see Unsafe contracts.
        unsafe {
            // UnsafeCell::get yields raw pointer; (*box).as_ptr() is stable.
            let box_ptr: *mut Box<[MaybeUninit<E>]> = writer.write_buf.get();
            let src: *const MaybeUninit<E> = (*box_ptr).as_ptr();
            // reader_buf is owned directly by &mut EventBuffer<E>; .as_mut_ptr()
            // on Box<[T]> via deref is stable.
            let dst: *mut MaybeUninit<E> = (*buf.reader_buf).as_mut_ptr().add(cursor);
            core::ptr::copy_nonoverlapping(src, dst, n);
        }
        cursor += n;
    }

    buf.reader_len.store(cursor as u32, Ordering::Release);
}
```

### Read (W7-NEW: stable slice construction)
```rust
#[inline]
pub fn events<E: Event>(&self) -> &[E] {
    let id = E::event_id() as usize;
    debug_assert!(id < MAX_EVENTS);
    if !self.registered_mask.get(id) {
        return &[];
    }
    // SAFETY (U10): see Unsafe contracts.
    let slot: &EventTypeSlot = unsafe { self.slots[id].slot.assume_init_ref() };
    debug_assert_eq!(slot.vtable.type_id, TypeId::of::<E>());
    let buf: &EventBuffer<E> = unsafe { &*(slot.data as *const EventBuffer<E>) };
    let len = buf.reader_len.load(Ordering::Acquire) as usize;
    // SAFETY (U11 rewritten): see Unsafe contracts.
    unsafe {
        // MaybeUninit::slice_assume_init_ref is unstable as of 1.95.
        // Equivalent stable form: from_raw_parts over the typed pointer.
        // reader_buf is Box<[MaybeUninit<E>]>; .as_ptr() via deref is stable.
        let ptr: *const E = (*buf.reader_buf).as_ptr().cast::<E>();
        core::slice::from_raw_parts(ptr, len)
    }
}
```

### Drop (W6-NEW: drop_fn does dealloc; remove explicit dealloc step)
`EventDispatcher::drop` iterates the bitset and calls `drop_fn`. No explicit `dealloc` afterwards.

```rust
impl Drop for EventDispatcher {
    fn drop(&mut self) {
        let mut mask = self.registered_mask;
        while let Some(id) = mask.pop_lowest_set_bit() {
            // SAFETY: bit was set ⇒ slot was initialised by preregister.
            let slot: EventTypeSlot = unsafe {
                core::ptr::read(self.slots[id as usize].slot.as_ptr())
            };
            // SAFETY: drop_fn was registered for the exact E whose buffer
            // sits at slot.data. drop_fn reconstructs Box<EventBuffer<E>>
            // and the Box's Drop both runs EventBuffer's Drop and deallocates.
            unsafe { (slot.vtable.drop_fn)(slot.data); }
        }
        // registered_mask intentionally unmodified — we're being dropped.
    }
}

unsafe fn drop_buffer<E: Event>(data: *mut u8) {
    // SAFETY (U6-drop, U7-drop): data was produced by Box::into_raw on the
    // matching EventBuffer<E>. Reconstructing the Box runs EventBuffer::drop
    // (which drops initialised event payloads, see U12) and deallocates the
    // heap allocation. After this returns, data is dangling.
    let _ = unsafe { Box::from_raw(data as *mut EventBuffer<E>) };
}
```

`EventBuffer<E>::drop` — drops the initialised event payloads. The `Box`es for `reader_buf` and per-lane `write_buf` then drop on their own (just freeing the slice memory; `MaybeUninit` never auto-drops contents).

```rust
impl<E: Event> Drop for EventBuffer<E> {
    fn drop(&mut self) {
        if core::mem::needs_drop::<E>() {
            let len = self.reader_len.load(Ordering::Relaxed) as usize;
            // SAFETY (U12): see Unsafe contracts.
            unsafe {
                for i in 0..len {
                    self.reader_buf[i].assume_init_drop();
                }
            }
            for lane in self.lanes.iter_mut() {
                let n = lane.writer.write_len.load(Ordering::Relaxed) as usize;
                // SAFETY (U12): see Unsafe contracts. UnsafeCell access ok because
                // &mut self ⇒ no other thread is touching the cell.
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
```

---

## API surface

### On `EcsMaster` (W10-NEW: `preregister_event_default` body shown)
```rust
impl EcsMaster {
    pub fn events(&self) -> &EventDispatcher;
    pub fn events_mut(&mut self) -> &mut EventDispatcher;

    /// Preregister with custom config.
    #[inline]
    pub fn preregister_event<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
        self.events.preregister::<E>(cfg)
    }

    /// Preregister with default capacity, using the dispatcher's
    /// validated `default_thread_count`.
    #[inline]
    pub fn preregister_event_default<E: Event>(&mut self) -> EcsResult<()> {
        let cfg = EventConfig::default_for(self.events.default_thread_count())
            .expect("invariant: default_thread_count was validated at EventDispatcher::new");
        self.events.preregister::<E>(cfg)
    }

    #[inline]
    pub fn send_event<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
        self.events.send::<E>(thread_index, event)
    }

    #[inline]
    pub fn events_of<E: Event>(&self) -> &[E] {
        self.events.events::<E>()
    }

    pub fn update_events(&mut self) {
        self.events.update_events();
    }
}
```

### On `EventDispatcher`
```rust
impl EventDispatcher {
    /// `default_thread_count` is validated via `EventConfig::default_for`.
    /// Returns `Err(InvalidEventConfig)` if out of range.
    pub fn new(default_thread_count: u32) -> EcsResult<Self>;

    #[inline] pub fn default_thread_count(&self) -> u32;

    pub fn preregister<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()>;

    #[inline]
    pub fn send<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()>;

    pub fn send_many<E: Event, I: ExactSizeIterator<Item = E>>(
        &self,
        thread_index: u32,
        iter: I,
    ) -> EcsResult<()>;

    #[inline]
    pub fn events<E: Event>(&self) -> &[E];

    pub fn update_events(&mut self);

    #[cfg(debug_assertions)]
    pub fn diagnostics<E: Event>(&self) -> Option<EventDiagnostics>;
}

#[cfg(debug_assertions)]
pub struct EventDiagnostics {
    pub last_swap_frame: u64,
    pub events_swapped_unread: u32,
    pub per_lane_overflow_count: Box<[u32]>,
}
```

### Error variants
```rust
#[non_exhaustive]
pub enum EcsError {
    // ... existing variants ...

    EventBufferFull {
        type_name: &'static str,
        thread_index: u32,
        attempted: u32, // events the call tried to write
        dropped: u32,   // events rejected (= attempted for all-or-nothing)
    },
    EventNotRegistered { type_name: &'static str },
    EventAlreadyRegistered { type_name: &'static str },
    InvalidEventConfig { reason: &'static str },
}
```

### `Event` trait change (D2 — unchanged)
```rust
pub trait Event: 'static + Sized + Send + Sync {
    // ... rest unchanged ...
}
```

---

## Integration with existing subsystems

### Files touched
| File | Change |
|---|---|
| `crates/boyko_utils/src/bit_mask/bit_set_256.rs` | **NEW** (Phase 6 Step 0) — `BitSet256` struct + ops + tests. |
| `crates/boyko_utils/src/bit_mask/mod.rs` | `pub mod bit_set_256;` |
| `crates/boyko_ecs/src/ecs/core/events/event.rs` | Add `Send + Sync` supertrait to `Event` (D2). |
| `crates/boyko_ecs/src/ecs/core/events/mod.rs` | Add `pub mod event_dispatcher; pub mod event_buffer; pub mod event_config;` |
| `crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs` | **NEW** — dispatcher + `ZstCheck` + `swap_and_flatten` + `drop_buffer`. |
| `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs` | **NEW** — `EventBuffer<E>`, `ThreadLanePair`, `ThreadLaneWriter` (with `UnsafeCell`), `ThreadLaneReader`. |
| `crates/boyko_ecs/src/ecs/core/events/event_config.rs` | **NEW** — `EventConfig` with `new` + `default_for`. |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | Add `events: EventDispatcher` field (first), `update_events`, proxy methods. `EcsMaster::new` returns `EcsResult` since `EventDispatcher::new` now does. |
| `crates/boyko_ecs/src/ecs/error.rs` | 4 new error variants + Display arms. |
| `crates/boyko_ecs/src/ecs/constants.rs` | `MAX_EVENT_THREADS`, `MAX_EVENT_CAPACITY` consts. |

### `Event` trait migration impact
Adding `Send + Sync` supertrait — breaking change only for hypothetical `Event` impls using `!Send`/`!Sync` types. None exist today. Macro-generated participants/parameters are POD. Risk: zero.

### `EcsMaster` drop order
Insert `events: EventDispatcher` as the **first** field (dropped first). Independent of arena/entity/archetype — events live in their own boxes.

### Arena compatibility
Event buffers never touch the arena.

---

## Performance characteristics

### Send hot path (warm cache, `size_of::<E>() = 32`)
Costs essentially unchanged from R2 — `UnsafeCell` is `repr(transparent)`, `UnsafeCell::get()` is a zero-cost identity cast in codegen.

| Step | Cost |
|---|---|
| `E::event_id()` (OnceLock cached load) | 1 ns |
| `registered_mask.get(id)` (debug_assert) | 0 ns release / 1 ns debug |
| Index `slots[id]` (cache hit) | 1 ns |
| `assume_init_ref` + cast to `&EventBuffer<E>` | 0 ns (compile-time) |
| `lane = &self.lanes[thread_index].writer` | 1 ns (likely L1 hit) |
| `write_len.load(Relaxed)` | 1 ns |
| Bounds check | 0.5 ns |
| `UnsafeCell::get()` + `(*box).as_mut_ptr()` + `add(len)` + write 32 B | 3-4 ns |
| `write_len.store(len + 1, Release)` | 1 ns |
| **Total warm send** | **~9-12 ns** |

### `update_events` budget
| Step | Cost |
|---|---|
| Per type: `pop_lowest_set_bit` on bitset | 1-2 cycles |
| Per type: index slot + load `swap_fn` | 1 ns |
| Per type: indirect call to `swap_and_flatten::<E>` | 3 ns |
| Per type: walk lanes (16 lanes × `write_len.swap`) | 16 ns |
| Per type: memcpy avg 4 × 32 B per non-empty lane | ~10 ns |
| Per type: `reader_len.store` | 1 ns |
| **Per registered type** | **~30 ns** |
| **N = 64 registered types** | **~2 µs** |
| **N = 256 registered types** | **~8 µs** |

### Memory budget (W2-NEW: corrected upper bound)
| Quantity | Per-type | Per master (32 types × 16 lanes × 1 KB cap × 32 B) |
|---|---|---|
| `lanes` (Box of `ThreadLanePair`) | `thread_count × 128 B` | 32 × 16 × 128 B = 64 KB |
| Per-lane `write_buf` (× thread_count lanes) | `thread_count × cap × size_of::<E>()` | 32 × 16 × 32 KB = 16 MB |
| `reader_buf` | `thread_count × cap × size_of::<E>()` | 32 × 16 × 32 KB = 16 MB |
| **Per-type total** | `≈ 2 × (thread × cap × size_of::<E>())` | **~32 MB for 32 event types** |
| **Absolute upper bound** | `2 × 256 × 64 × 16384 × 64 B` | **≈ 32 GiB** (user's responsibility) |

The dispatcher itself: ~16.5 KB per master (16 KB slots + 32 B bitset + 12 B header). Cache-resident.

### Cache behavior
Unchanged from R2 — `UnsafeCell` is transparent; no extra indirection.

### Branch prediction
Unchanged from R2.

---

## Lock-free parallelism plan

Unchanged from R2 except for the UnsafeCell justification:

### Phase 6 (this phase): single-threaded sends
All `send` from main thread; `thread_index = 0` only valid when `EventConfig::thread_count == 1`. `write_len` uses `Relaxed` loads and `Release` stores.

### Phase 7 (future): N worker threads send concurrently
1. **No cross-thread contention on the writer**: each worker writes to its own `lanes[worker_id].writer` (UnsafeCell with documented per-thread exclusivity).
2. **No writer↔reader false sharing** (W2): separate cache lines.
3. **Sync point**: `update_events` is the only sync point. Workers stop sending; main thread runs swap. `write_len.swap(_, AcqRel)` synchronises with `write_len.store(_, Release)`.
4. **No locks**: `AtomicU32` on `write_len`/`reader_len`; `AtomicPtr` reserved.
5. **No allocation during steady state**.
6. **Send/Sync**: `EventBuffer<E>` auto-`Send + Sync` from `E: Send + Sync` + atomics + boxes. `ThreadLaneWriter<E>` has a documented `unsafe impl Sync` justified by the per-thread exclusivity invariant.

---

## Unsafe contracts

Every `unsafe` block in the design, with concrete invariants the developer must encode. **Rewritten contracts** (C1-NEW, W4-NEW, W7-NEW, W8-NEW): U1, U4, U9, U11. **New contract**: U13.

### U1. Reading the writer buffer pointer through UnsafeCell (rewritten — C1-NEW, W8-NEW)
**Site**: `EventBuffer::send_one`, `swap_and_flatten`, `EventBuffer::drop`.
**Code shape**:
```rust
let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.writer.write_buf.get();
let raw: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr(); // or .as_ptr() for *const
```
**SAFETY**:
1. `lane.writer.write_buf` is an `UnsafeCell<Box<[MaybeUninit<E>]>>`. `UnsafeCell::get()` yields a raw pointer that is the interior-mutability mechanism allowing `&self`-receiver methods to mutate the cell's contents.
2. Per-thread exclusivity invariant (documented on `ThreadLaneWriter`): only the worker assigned `thread_index` accesses this `UnsafeCell` for writes. The main thread accesses it only inside `update_events` (`&mut self` on dispatcher = sync point with all workers); during that window, no worker is in flight.
3. `(*box_ptr).as_mut_ptr()` uses `Box`'s slice-deref (stable form; `Box::as_mut_ptr` is unstable per `rust#129090`).
4. The returned `*mut MaybeUninit<E>` is valid for the box's lifetime (≥ `EventBuffer<E>`'s lifetime, which outlives the borrow).

### U2. `MaybeUninit::write` for storing an `EventTypeSlot` (unchanged)
**Site**: `EventDispatcher::preregister`.
**Code shape**: `unsafe { self.slots[id].slot.write(slot); }`.
**SAFETY**:
1. `id < MAX_EVENTS` (checked above).
2. `registered_mask.get(id) == false` (checked above), so no previous value to drop.
3. `slot` is fully initialised.

### U3. `MaybeUninit::assume_init_ref` for reading a registered slot (unchanged)
**Site**: `EventDispatcher::send`, `events`, `send_many`.
**Code shape**: `unsafe { self.slots[id].slot.assume_init_ref() }`.
**SAFETY**:
1. `id < MAX_EVENTS` (debug_assert + branch).
2. `registered_mask.get(id) == true` (branch above returns early otherwise).
3. The bit is set only after `slot.write` in `preregister`; bit-set ⇒ slot initialised.

### U4. Writing an event into a writer lane via UnsafeCell (rewritten — C1-NEW, W4-NEW)
**Site**: `EventBuffer::send_one`, `EventBuffer::send_many` inner loop.
**Code shape**:
```rust
unsafe {
    let box_ptr: *mut Box<[MaybeUninit<E>]> = lane.write_buf.get();
    let buf_ptr: *mut MaybeUninit<E> = (*box_ptr).as_mut_ptr();
    let dst: *mut MaybeUninit<E> = buf_ptr.add(len as usize);
    (*dst).write(event);
}
```
**SAFETY**:
1. **Interior mutability via UnsafeCell**: `write_buf: UnsafeCell<Box<[MaybeUninit<E>]>>`. `UnsafeCell::get()` is the only sanctioned primitive for mutating shared data through `&`; aliasing rules treat the cell as exempt from the no-mutation-through-`&` rule. This is the **explicit basis** for taking a `&self`-receiver `send_one` and mutating the box.
2. **Per-thread exclusivity** (documented on `ThreadLaneWriter`): only the worker pinned to `thread_index` accesses this `UnsafeCell`. The send path is *not* re-entrant: a single worker calling `send_one(thread_index, …)` for the same `thread_index` cannot be interrupted mid-write by another caller for the same lane.
3. **Bounds**: `len < lane.capacity` (the bounds check immediately above this block returns `EventBufferFull` if not).
4. **Initialisation discipline**: `MaybeUninit::write` is correct here — the slot at `[len]` is uninitialised because `write_len` always tracks the initialised prefix; `write` does not drop the previous (uninitialised) value.
5. **No aliasing with reader**: the reader path lives in `ThreadLaneReader` on a **different cache line** (D4), and the swap path (`swap_and_flatten`) only runs from the main thread under the `&mut EventDispatcher` sync point with all workers stopped.

### U5. `MaybeUninit::assume_init_mut` for swap iteration (unchanged)
**Site**: `EventDispatcher::update_events`.
**Code shape**: `unsafe { self.slots[id as usize].slot.assume_init_mut() }`.
**SAFETY**:
1. `id` came from `registered_mask.pop_lowest_set_bit()` ⇒ bit was set ⇒ slot initialised (mirror of U3).
2. `&mut self` on `update_events` provides exclusive access.

### U6. Calling `slot.vtable.swap_fn` (function pointer) (unchanged)
**Site**: `EventDispatcher::update_events`.
**Code shape**: `unsafe { (slot.vtable.swap_fn)(slot.data, frame); }`.
**SAFETY**:
1. `slot.vtable.swap_fn` was set to `swap_and_flatten::<E>` for the exact type `E` whose buffer is at `slot.data` (set atomically with `data` in `preregister`).
2. `slot.data` points to a `Box<EventBuffer<E>>`-derived raw pointer (allocated in `preregister`, freed only in `drop_buffer::<E>` during dispatcher drop).
3. The function pointer is `'static`; monomorphised at crate compile time.

### U6-drop. Calling `slot.vtable.drop_fn` (W6-NEW)
**Site**: `EventDispatcher::drop`.
**Code shape**: `unsafe { (slot.vtable.drop_fn)(slot.data); }`.
**SAFETY**:
1. Same type-matching as U6.
2. `slot.data` has never been freed before (this is the only call site that frees it).
3. After this call, `slot.data` is dangling — but the slot is unreachable because `EventDispatcher` is itself being dropped; `registered_mask` is intentionally not cleared.

### U7. Casting `slot.data: *mut u8` to `&mut EventBuffer<E>` (unchanged)
**Site**: `swap_and_flatten::<E>`.
**Code shape**: `unsafe { &mut *(data as *mut EventBuffer<E>) }`.
**SAFETY**:
1. The function is monomorphised only for the `E` that registered this slot.
2. `data` was produced by `Box::into_raw(Box::new(EventBuffer::<E>::new(cfg)?))` — same `E`.
3. The mutable borrow is unique: `update_events` takes `&mut self` on `EventDispatcher` and is single-threaded.

### U7-read. Casting `slot.data: *mut u8` to `&EventBuffer<E>` (unchanged)
**Site**: `EventDispatcher::send`, `events`, `send_many`.
**Code shape**: `unsafe { &*(slot.data as *const EventBuffer<E>) }`.
**SAFETY**:
1. Same type-matching as U7.
2. Shared borrow: `send` and `events` take `&self`; multiple shared references are sound.
3. Concurrent mutation through `&EventBuffer<E>` only happens via `AtomicU32` (`write_len`, `reader_len`, `overflow_count`) and `UnsafeCell::get()` (with per-thread exclusivity, U4) — both sanctioned by the language.

### U8. Dropping a previously-initialised reader_buf element (unchanged)
**Site**: `swap_and_flatten::<E>`.
**Code shape**: `unsafe { buf.reader_buf[i].assume_init_drop(); }`.
**SAFETY**:
1. `i < prev_len = reader_len.load(Relaxed)`.
2. `reader_len` was set to the cursor that bounded the initialised prefix at the end of the previous swap.
3. No reader holds a `&E` to this slot: convention is all readers complete before `update_events`.

### U9. `copy_nonoverlapping` from writer lane to reader_buf (rewritten — C1-NEW, W8-NEW)
**Site**: `swap_and_flatten::<E>`.
**Code shape**:
```rust
unsafe {
    let box_ptr: *mut Box<[MaybeUninit<E>]> = writer.write_buf.get();
    let src: *const MaybeUninit<E> = (*box_ptr).as_ptr();
    let dst: *mut MaybeUninit<E> = (*buf.reader_buf).as_mut_ptr().add(cursor);
    core::ptr::copy_nonoverlapping(src, dst, n);
}
```
**SAFETY**:
1. **Source access**: `UnsafeCell::get()` yields `*mut Box<[MaybeUninit<E>]>`; we then dereference and call `.as_ptr()` for a `*const MaybeUninit<E>`. Per-thread exclusivity does NOT apply here — at swap time we are the *only* accessor system-wide (workers stopped, main thread runs swap). So even reading the cell from the "main thread" rather than the lane's assigned worker is sound.
2. **Destination access**: `buf.reader_buf` is owned directly by `&mut EventBuffer<E>` (we hold `&mut` via U7 here). `(*box).as_mut_ptr()` via slice-deref is stable.
3. `n` came from `write_len.swap(0, AcqRel)` — exactly the number of initialised elements in the lane's `write_buf`.
4. `cursor + n <= reader_buf.len()`: `reader_buf.len() == thread_count * capacity_per_lane`; summing `n` across lanes can't exceed that (each `n <= capacity_per_lane`).
5. Source and destination are disjoint allocations (separate `Box`es; `writer.write_buf` is per-lane, `reader_buf` is per-buffer).
6. Both regions are properly aligned for `E` (boxes guarantee `align_of::<E>()`).
7. After this call, `writer.write_buf[0..n]` is logically uninitialised (`write_len = 0` published by the swap).
8. **AcqRel ordering on swap synchronises** with `Release` on the worker's `write_len.store` — all bytes written by the worker before publishing `write_len` are visible to this read.

### U10. `MaybeUninit::assume_init_ref` for the slot in `events::<E>()` (unchanged — same as U3)

### U11. Building a `&[E]` slice over the initialised reader prefix (rewritten — W7-NEW)
**Site**: `EventDispatcher::events`.
**Code shape**:
```rust
unsafe {
    let ptr: *const E = (*buf.reader_buf).as_ptr().cast::<E>();
    core::slice::from_raw_parts(ptr, len)
}
```
**SAFETY**:
1. **Why not `MaybeUninit::slice_assume_init_ref`**: this method is unstable as of Rust 1.95 stable (`rust#80836`). The R2 plan was wrong about its stabilisation. We use the equivalent stable construction below.
2. `len = reader_len.load(Acquire)`.
3. `reader_len.store(cursor, Release)` in `swap_and_flatten` happens-before this load (Acquire/Release pair).
4. `cursor` bounds the prefix initialised by `copy_nonoverlapping` (U9), so the first `len` elements of `reader_buf` contain valid `E` values.
5. `(*buf.reader_buf).as_ptr()` is `*const MaybeUninit<E>`; `.cast::<E>()` is valid because `MaybeUninit<T>` and `T` have identical layout (`#[repr(transparent)]`).
6. `from_raw_parts(ptr, len)` requirements:
   - `ptr` is non-null (Box-derived).
   - `ptr` is properly aligned for `E` (Box guarantees `align_of::<E>()`).
   - The first `len` elements are valid `E` values (point 4).
   - The slice does not exceed `isize::MAX` bytes (bounded by `MAX_EVENT_THREADS × MAX_EVENT_CAPACITY × size_of::<E>()` = at most ~32 GiB ≪ `isize::MAX` on 64-bit).
   - No mutable borrow exists concurrently: `events` takes `&self`; the only mutator is `update_events` which takes `&mut self`.

### U12. Drop loop in `EventBuffer::drop` (extended — UnsafeCell access)
**Site**: `impl Drop for EventBuffer<E>`.
**Code shape**: drop loops for `reader_buf` and each lane's `write_buf` (via `UnsafeCell::get()`).
**SAFETY**:
1. We only drop the `[0..reader_len)` prefix of `reader_buf` and `[0..write_len)` prefix of each `write_buf` — these are exactly the initialised prefixes per `MaybeUninit` discipline.
2. `&mut self` on `Drop` ⇒ no other code holds aliases; `UnsafeCell::get()` is sound to use with mutable access through the cell because of the `&mut self`.
3. After this drop, the `Box<[MaybeUninit<E>]>`es deallocate their slice storage naturally; `MaybeUninit` never auto-drops contents, so this manual drop loop is necessary for `E: Drop`.

### U13. Reading from `UnsafeCell` during swap (new — W4-NEW)
**Site**: `swap_and_flatten::<E>` (the swap-side read of writer lanes).
**Code shape**: `let box_ptr: *mut Box<[MaybeUninit<E>]> = writer.write_buf.get();`
**SAFETY**:
1. The swap runs from the main thread under `&mut EventDispatcher` (U7).
2. Convention enforced by the future scheduler (and by Phase 6's single-threaded execution): no worker is calling `send`/`send_many` for any lane during `update_events`. The main thread is the **sole accessor** of every `UnsafeCell` for the duration of the swap.
3. The `Acquire` on `write_len.swap(0, AcqRel)` happens-before the subsequent read of the box pointer's contents — even if a worker thread held the lane immediately prior, all its writes are visible after this acquire.
4. The mutation we perform via the box (logically marking `[0..n]` as uninitialised by zeroing `write_len`) is independent of the byte-copy `copy_nonoverlapping(src, dst, n)` which only reads from the source.

---

## Migration / rollout

### Step 0: extend `boyko_utils` (W9-NEW)
- New file `crates/boyko_utils/src/bit_mask/bit_set_256.rs`.
- Implement `BitSet256` struct + ops as specified above.
- Register module in `bit_mask/mod.rs`.
- Add unit tests as specified in Phase 6 Step 0 section.

### Step 1: extend `Event` trait
- Add `Send + Sync` supertrait (`event.rs`).

### Step 2: extend `EcsError` + constants
- 4 new error variants + Display arms (`error.rs`).
- `MAX_EVENT_THREADS`, `MAX_EVENT_CAPACITY` (`constants.rs`).

### Step 3: add `EventConfig`
- New file `event_config.rs` with `new` + `default_for` + `DEFAULT_CAPACITY` + `DEFAULT`.

### Step 4: add `EventBuffer<E>` + `ThreadLanePair`
- New file `event_buffer.rs`.
- `ThreadLaneWriter` with `UnsafeCell<Box<[MaybeUninit<E>]>>` + manual `unsafe impl Sync` with SAFETY comment per U4.
- Compile-time layout asserts.

### Step 5: add `EventDispatcher`
- New file `event_dispatcher.rs`.
- `ZstCheck` helper struct (D9).
- `EventDispatcher::new` returning `EcsResult`.
- `preregister`, `send`, `send_many`, `events`, `update_events`, `Drop`.
- Free functions `swap_and_flatten::<E>` and `drop_buffer::<E>` (W6-NEW: drop_fn does dealloc).

### Step 6: integrate into `EcsMaster`
- Add `events: EventDispatcher` field as **first** (drops first).
- Proxy methods including `preregister_event_default` (W10-NEW).
- `EcsMaster::new` propagates `EcsResult` if it didn't already.

### Step 7: tests + benchmarks (see next section).

### Step 8: doc-writer pass.

---

## Tests required

### Unit (in module)
1. **`event_config_bounds`** — bounds validation.
2. **`preregister_twice_errors`**.
3. **`send_without_preregister_errors`** — release: `Err`; debug: panic.
4. **`send_overflow_returns_full`** — `EventBufferFull { dropped: 1 }`.
5. **`send_many_atomic_on_overflow`** — no events written; `attempted == dropped`.
6. **`send_then_swap_then_read`**.
7. **`next_frame_visibility`** — Model B.
8. **`double_swap_per_frame_debug_asserts`**.
9. **`zst_event_rejected_at_compile`** (`compile_fail` doctest) — exercises `ZstCheck::<E>::NON_ZERO`.
10. **`drop_runs_for_initialized_only`** — `DropCounter` event; verify drop count.
11. **`multi_master_independent_capacity`**.
12. **`bitset_iteration_only_registered`** — register at IDs 5, 100, 200; `update_events` calls `swap_fn` 3×.
13. **`ownership_no_leak`** (Miri).
14. **`drop_order_events_before_arena`**.

### Property-based (`proptest`)
15. **`send_then_read_roundtrip`**.
16. **`overflow_count_matches_rejections`**.

### Concurrency (`loom`, feature-gated)
17. **`writer_swap_race_acquire_release`**.
18. **`reader_swap_race`**.

### False sharing (criterion)
19. **`false_sharing_writer_reader_baseline`** — split vs unsplit; expect 2-5× improvement.

### Criterion benchmarks
20. **`send_warm_cache`** — 1M sends; target <12 ns/op.
21. **`update_events_64_types`** — target <2 µs.
22. **`update_events_256_types`** — target <8 µs.
23. **`read_iteration_1M_events`** — target >20 GB/s.

### Integration (in `tests/`)
24. **`tests/event_double_buffer.rs`**.
25. **`tests/event_multi_type.rs`**.

### New tests required by Round 2 critic (#26-32)
26. **`frame_counter_u64_wrap`** (W5-NEW): unit test exercising `current_frame = u64::MAX - 1` (set via test-only mutator behind `#[cfg(test)]`); call `update_events` twice; first succeeds (last_swap < current_frame), wrap to `u64::MAX` succeeds; further wrap is documented as unreachable (u64 wrap never reached). The test asserts no panic on near-wrap.
27. **`compile_fail_send_after_dispatcher_drop`** (`compile_fail` doctest): show that holding a `&[E]` from `events::<E>()` across an `EcsMaster` drop is a borrow-checker error. Confirms reader lifetime tied to dispatcher.
28. **`concurrent_send_and_events_slice_validity`** (C1-NEW): in a single thread, call `let slice = dispatcher.events::<E>(); dispatcher.send(0, E::new());`. The `slice` (from the previous frame's swap) must remain valid because send writes to writer lanes, not to `reader_buf`. After the next `update_events`, the slice would be stale — but until then, it's valid. Test asserts slice contents remain stable across an intervening `send`.
29. **`miri_send_aliasing`** (C1-NEW, Miri-gated): run a workload of N sends from the same thread under Miri with both Stacked Borrows and Tree Borrows. Expected: pass. Sanity check that `UnsafeCell` rework eliminates the R2 UB.
30. **`type_id_collision_debug_asserts`**: monkey-patch a slot's `vtable.type_id` to `TypeId::of::<u64>()` (test-only mutator); call `events::<TheRealEvent>`; expect `debug_assert_eq!` panic.
31. **`send_many_empty_iter`** (D7): `dispatcher.send_many::<E, _>(0, std::iter::empty())` returns `Ok(())`; `write_len` unchanged.
32. **`send_thread_index_out_of_range_release`**: release-mode behaviour test — `dispatcher.send(thread_count + 5, E::new())`. With `debug_assert!(thread_index < thread_count)` compiled out, the next access `self.lanes[thread_index as usize]` indexes `Box<[ThreadLanePair]>` which panics with `index out of bounds`. Test asserts the panic message contains "index out of bounds". (Design choice: rely on Rust's bounds-checked slice indexing; do not switch to `get_unchecked` because the cost is a single compare-and-branch dwarfed by the memcpy.)

### `debug_assert!` invariants embedded in code
- `thread_index < self.thread_count` (in `send`, `send_many`).
- `id < MAX_EVENTS` (in `send`, `events`, `send_many`).
- `slot.vtable.type_id == TypeId::of::<E>()` (in `send`, `events`, `send_many`).
- `last_swap_frame < current_frame` (in `update_events`).
- `cursor + n <= reader_buf.len()` (in `swap_and_flatten`).
- `len < lane.capacity` (in `send_one`).
- `n <= remaining` (in `send_many` pre-check; this is the success branch's pre-condition).

---

## Open questions / future work

**Truly future work:**

- **Q-Phase7-1**: Concurrent read-during-send across threads. Phase 7 scheduler must enforce read-set/write-set disjointness across systems.
- **Q-Phase7-2**: Per-thread `thread_index` injection via scheduler thread-local.
- **Q-Phase7-3**: Backpressure / dynamic capacity. Rejected for Phase 6 (zero-alloc steady state).
- **Q-Phase8-1**: SIMD-accelerated `swap_and_flatten` for `size_of::<E>() ≤ 16 B`.
- **M3**: Event ID portability across processes — unchanged from `event_registry.rs`.
- **TypeId fragility**: if Rust changes `TypeId` layout again, the `EventVTable` size will shift. The release-mode tight assert (`== 32`) will trip and CI catches it; debug-mode uses `% 64 == 0` for resilience.
- **ComponentPool ZST**: still runtime `debug_assert!` (not generic). Separate ticket may upgrade if a generic constructor is ever introduced.

**Critic-raised questions now resolved in Phase 6:**
- Q1 (per-frame epoch debug): D12 with u64 (W5-NEW) — included.
- Q4 (per-master thread count): D5 — included.
- Q6 (preregister API): D11 — included.

---

## Relevant file paths

- `D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\bit_mask\bit_set_256.rs` (NEW, Phase 6 Step 0)
- `D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\bit_mask\mod.rs` (register new module)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\event.rs` (add `Send + Sync` supertrait)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\event_registry.rs` (unchanged; reference for OnceLock pattern)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\mod.rs` (register new modules)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\event_dispatcher.rs` (NEW)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\event_buffer.rs` (NEW)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\events\event_config.rs` (NEW)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (add `events` field + proxies; line 39+ for struct, 51+ for `new`)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\error.rs` (4 new variants)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\constants.rs` (2 new consts)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\memory\component_pool.rs:84` (ZST rejection precedent; runtime `debug_assert!`)
````

---

## Summary of Round 2 → Round 3 changes (for re-critic spot-check)

1. **C1-NEW (UB) fixed**: `ThreadLaneWriter::write_buf` is now `UnsafeCell<Box<[MaybeUninit<E>]>>`. All `&self`-receiver mutations route through `UnsafeCell::get()`. U4 contract rewritten to explicitly cite the interior-mutability primitive and the per-thread exclusivity invariant. Added `unsafe impl Sync for ThreadLaneWriter<E>` with documented justification.
2. **C2-NEW (compile error) fixed**: ZST guard moved to `ZstCheck<E>::NON_ZERO` associated const on a generic helper struct, forced at `preregister` site via `let _ = ZstCheck::<E>::NON_ZERO;`. D9 rewritten with honest cite of ComponentPool's runtime-only precedent.
3. **W1-NEW (layout) fixed**: `ThreadLaneReader` field order swapped to `{ reserved_ptr (8 B), read_cursor (4 B), _pad (52 B) }` = 64 B.
4. **W2-NEW (memory) fixed**: Memory budget table corrected to ~32 GiB upper bound (writers + flattened reader buf).
5. **W3-NEW (TypeId) fixed**: TypeId is 16 B (u128). `EventVTable` recalculated to 32 B (post W6-NEW). `EventTypeSlot` sizes recomputed; debug asserts loosened to `% 64 == 0` + `== 64` only in release.
6. **W4-NEW (U4 contract) fixed**: U4 fully rewritten; new U13 covers swap-side reads of the UnsafeCell.
7. **W5-NEW (u32 wrap) fixed**: `current_frame: u64` and `last_swap_frame: u64`.
8. **W6-NEW (double-free) fixed**: Chose option (b) — `drop_fn = Box::from_raw(...)` does Drop + dealloc. `layout` field removed from `EventVTable`. No explicit `dealloc` in `EventDispatcher::drop`.
9. **W7-NEW (slice API) fixed**: `events::<E>()` uses `core::slice::from_raw_parts(buf.reader_buf.as_ptr().cast::<E>(), len)` — stable on Rust 1.93+. U11 rewritten.
10. **W8-NEW (Box::as_ptr) fixed**: All pointer derivation uses `(*box).as_ptr()` / `.as_mut_ptr()` via slice-deref (stable). U1 rewritten.
11. **W9-NEW (BitSet256) fixed**: New "Phase 6 Step 0" section fully specs `BitSet256` (`[u64; 4]`, align(32)) with `set/get/clear/count_ones/is_empty/pop_lowest_set_bit` and unit tests. Lands in `boyko_utils::bit_mask::bit_set_256`.
12. **W10-NEW (preregister_event_default body) fixed**: Body shown using new `EventConfig::default_for(thread_count)` constructor. `EventDispatcher::new` now returns `EcsResult` so `default_thread_count` is validated; the `.expect("...")` in the proxy is honest.
13. **Cleanup items**: ZST claim corrected (no longer claims ComponentPool is compile-time). M2 row text aligned with API (`&self` for both `events` and `send`).
14. **Tests #26-32 added**: u64 wrap, compile_fail across drop, slice-validity-across-send, Miri aliasing, TypeId collision, empty `send_many`, out-of-range `thread_index` release behaviour.

Sources:
- [PR #109953 — Use 128 bits for TypeId hash](https://github.com/rust-lang/rust/pull/109953)
- [PR #75923 — Widen TypeId from 64 bits to 128](https://github.com/rust-lang/rust/pull/75923)
- [std::mem::MaybeUninit docs (Rust stable)](https://doc.rust-lang.org/std/mem/union.MaybeUninit.html)
- [std::boxed::Box docs (Rust stable)](https://doc.rust-lang.org/std/boxed/struct.Box.html)
- [rust#129090 — Box::as_ptr stabilisation tracking issue](https://github.com/rust-lang/rust/issues/129090)

Relevant file paths (absolute):
- `D:\claude\BoykoEngine-ecs\docs\PHASE-6-EVENT-DISPATCH-PLAN.md` (target file — parent agent should write the markdown body above to this path)
- `D:\claude\BoykoEngine-ecs\crates\boyko_utils\src\bit_mask\bit_set.rs` (existing reference; new `bit_set_256.rs` sibling)
- `D:\claude\BoykoEngine-ecs\crates\boyko_ecs\src\ecs\memory\component_pool.rs` (line 84 — confirmed runtime `debug_assert!` precedent)