//! The per-scope bump allocator — [`ScopeBlock`] — and the pointer type its
//! allocations come back as, [`BlockPtr`].
//!
//! # This module is wired to NOTHING, and that is the shipping unit
//!
//! Stage 3a lands it alone: [`Scope`](crate::Scope) does not own a
//! `ScopeBlock` yet and `Task::new_scoped` still calls `alloc_cell` once per
//! spawn. Everything here is therefore dead code plus its pins, which is why
//! the file carries an `#![allow(dead_code)]` — deleted by the commit that
//! wires it. Landing it alone is what lets the layout pins and the property
//! tests be judged against a tree in which nothing else moved.
//!
//! # What it replaces, and what it must not become
//!
//! One `alloc`/`dealloc` pair per spawned task becomes one `alloc` per chunk
//! and one `dealloc` per chunk at the join. The allocation COUNT is what
//! changes; the peak payment moves from tasks IN FLIGHT to tasks SPAWNED,
//! because a cell is now reclaimed at `free_all` rather than when its body is
//! read out.
//!
//! Three properties carry the soundness argument, and each is a placement or a
//! type fact rather than a rule someone has to remember:
//!
//! **D1 — chunks hold payload bytes ONLY.** No `next`, no `used`, no `cap`, no
//! guard word. Capacities live in [`ScopeBlock::exps`], in the joiner's own
//! frame. So after publication there is no location in any chunk the owner
//! thread ever writes, and `Scope::drop`'s `&mut self` protector — which
//! covers the chunk TABLE, a row of raw `*mut u8` values — cannot reach the
//! chunks: copying a raw pointer out of a table does not retag its pointee.
//! One bookkeeping word inside a chunk destroys this and nothing announces it,
//! which is why it is stated first.
//!
//! **D2 — the offending expression is not constructible at the boundary.**
//! [`ScopeBlock::emplace`] allocates AND initialises, so no caller ever holds a
//! pointer to uninitialised block memory, and [`BlockPtr`]'s only exit is
//! [`erase`](BlockPtr::erase). There is no `Deref`, no `as_mut`, no accessor
//! and no `From`, so no caller can form a REFERENCE into chunk memory — and a
//! reference passed as a function argument is exactly the material Tree
//! Borrows turns into a protector. This is checked by the type system, not by
//! a grep.
//!
//! **D3 — the erased pointer is inert without a private type.** `erase`
//! returns `*const ()`; using one requires `cast::<ScopedCell<F>>()`, and
//! `ScopedCell` is private to `task.rs`. Laundering a pointer out of here buys
//! nothing unless the same commit also widens that type's visibility, which is
//! a type-system-visible edit.
//!
//! # Two absences that are decisions
//!
//! **No method takes `&mut self`.** `Scope::spawn(&self)` forces `&self`
//! through the call graph, so this is compiler-enforced rather than chosen —
//! and interior mutability through [`Cell`] is what a single-threaded owner
//! needs. `ScopeBlock` is `!Sync` by construction because `Cell` is, which is
//! the property that keeps the bump cursor off every other thread.
//!
//! **No `impl Drop`.** [`ScopeBlock::free_all`] runs at a point where nothing
//! can panic — immediately after `Scope::drop`'s join returns — so
//! leak-safety is structural. A `Drop` impl would additionally give the block
//! a destructor the joiner's `&mut self` would have to run, and would make the
//! "free every chunk exactly once" question a matter of drop order rather than
//! of one call site.

// Stage 3a ships the module before its caller (see the header). The crate gate
// is `-D warnings`, so without this allow an unwired module is unshippable on
// its own — which would force the wiring and its gate into the same commit,
// the exact coupling staging exists to avoid. DELETED in Stage 3b.
#![allow(dead_code)]

use core::alloc::Layout;
use core::cell::Cell;
use core::ptr::{self, NonNull};
use std::alloc::{alloc, dealloc, handle_alloc_error};

/// Base chunk capacity. EVERY chunk is `CHUNK0 << e` bytes for some `e: u8` —
/// a power of two `>= 4096`. The allocation receipts key on that.
const CHUNK0: usize = 4096;

/// Every chunk's alignment, and the second half of the receipts' bucket
/// discriminant `(size.is_power_of_two() && size >= CHUNK0, align == 64)`.
const CHUNK_ALIGN: usize = 64;

/// Chunk-table length. At 32 the table's total capacity is
/// `4096 * (2^32 - 1)` ~ 16 TiB and the LAST chunk alone is 8 TiB, so `alloc`
/// fails long before the table fills: exhaustion is unreachable while
/// allocation succeeds, and the reachable failure mode stays the
/// `handle_alloc_error` the cell allocator already has.
const MAX_CHUNKS: usize = 32;

// `CHUNK0 << (MAX_CHUNKS - 1)` = 2^43 must not overflow `usize`. THIS is the
// reason for the 64-bit requirement — and it is a reason only at this table
// length: at `MAX_CHUNKS = 16`, `4096 << 15` still fits in 32 bits, so the
// same line would have been an assertion with no arithmetic behind it.
const _: () = assert!(usize::BITS >= 64);

// The three chunk facts the allocation receipts are stated over, pinned to the
// values `__layout_receipt` publishes. The receipt module states them as
// independent literals precisely so a test binary is not computing its
// expectation from the code under test; these lines are what make that safe.
// Drift on either side is a build failure.
const _: () = assert!(CHUNK0 == crate::__layout_receipt::CHUNK0);
const _: () = assert!(CHUNK_ALIGN == crate::__layout_receipt::CHUNK_ALIGN);
const _: () = assert!(MAX_CHUNKS == crate::__layout_receipt::MAX_CHUNKS);

/// One scope's bump allocator: a cursor, a chunk table, and nothing else.
///
/// Field order is the layout, not the narrative. A spawn that does not grow
/// touches `cur` and `end` and NOTHING else — bytes 0..16 — because
/// [`bump`](Self::bump) never reads `n_chunks`, which only
/// [`grow`](Self::grow) and [`is_empty`](Self::is_empty) do. So the hot working
/// set is two words, not the three-field `#[repr(C)]` prefix `cur`/`end`/
/// `n_chunks` (bytes 0..20, or 0..24 with `_pad`); the 288-byte table behind
/// them is read exactly once per scope, by [`free_all`](Self::free_all). Under
/// Stage 3b's `#[repr(C, align(64))]` on `Scope` those two words share one
/// cache line with the scope's own two hot fields.
#[repr(C)]
pub(crate) struct ScopeBlock {
    /// HOT — the bump cursor. Null on a block that has never grown.
    cur: Cell<*mut u8>,
    /// HOT — one past the last byte of the chunk `cur` points into. Null
    /// exactly when `cur` is, so `end - cur` is 0 on a lazy block and the
    /// growth test needs no separate "is there a chunk" branch.
    end: Cell<*mut u8>,
    /// HOT-ish — the next free table index, and `free_all`'s bound.
    n_chunks: Cell<u32>,
    /// Explicit rather than implicit: `#[repr(C)]` inserts these four bytes
    /// either way, and a named field is what the size pin below is stated over.
    _pad: u32,
    /// COLD — chunk `i`'s base address. Read only by `free_all`.
    ///
    /// The chunk table lives HERE, in the owner's frame, and not in the chunks
    /// it describes. That is D1, and it is the module's load-bearing device.
    bases: [Cell<*mut u8>; MAX_CHUNKS],
    /// COLD — chunk `i`'s capacity as an EXPONENT: `cap(i) == CHUNK0 << exps[i]`.
    ///
    /// Exponents rather than capacities, and not to save 96 bytes: a capacity
    /// field can hold any `u32`, whereas an exponent can only name a power of
    /// two times `CHUNK0`. The receipts' chunk bucket is then exact by
    /// construction rather than by survey.
    exps: [Cell<u8>; MAX_CHUNKS],
}

// 8 + 8 + 4 + 4 + 256 + 32, with no tail padding at align 8. Drift is a build
// failure: the hot-prefix claim above and Stage 3b's `Scope` size pin are both
// stated over this number.
const _: () = assert!(size_of::<ScopeBlock>() == 312);

impl ScopeBlock {
    /// A block that owns nothing.
    ///
    /// Allocates on the first [`emplace`](Self::emplace) and never before, so a
    /// scope that spawns no task makes no allocator call at all — which is what
    /// keeps the empty-scope path (`schedule.rs`'s single-system rounds) free
    /// of a cost it did not pay before.
    pub(crate) const fn new() -> Self {
        Self {
            cur: Cell::new(ptr::null_mut()),
            end: Cell::new(ptr::null_mut()),
            n_chunks: Cell::new(0),
            _pad: 0,
            bases: [const { Cell::new(ptr::null_mut()) }; MAX_CHUNKS],
            exps: [const { Cell::new(0) }; MAX_CHUNKS],
        }
    }

    /// Whether the block has grown a chunk.
    ///
    /// Not a spawn-count: a scope that emplaced only zero-sized values is still
    /// empty, because a ZST costs no chunk byte.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.n_chunks.get() == 0
    }

    /// Allocate storage for `value` inside the block and MOVE it there.
    ///
    /// Allocating and initialising in one call is D2, not a convenience: a
    /// two-step `alloc` + `write` API would hand the caller a pointer to
    /// uninitialised block memory, and the moment such a pointer exists the
    /// expression that turns it into a reference is writable by anyone. Here it
    /// is not constructible, because the value goes in before the pointer comes
    /// out.
    ///
    /// `&self` for the reason the caller forces: `Scope::spawn` takes `&self`,
    /// so the whole path down to here is shared-borrowed and the compiler, not
    /// a convention, is what keeps it that way.
    #[inline]
    #[must_use = "the value has been MOVED into block memory; discarding the \
                  handle neither runs nor drops it, because free_all returns \
                  the bytes raw"]
    pub(crate) fn emplace<T>(&self, value: T) -> BlockPtr<T> {
        let size = size_of::<T>();
        let align = align_of::<T>();

        // A zero-sized value needs no chunk byte, and taking the general path
        // for one would hand back the null `cur` of a lazy block — the single
        // input for which the bump test succeeds without a chunk existing.
        // Folded away entirely for every real cell: `ScopedCell<F>` carries a
        // `CellHead` and is never zero-sized.
        if size == 0 {
            let slot = NonNull::<T>::dangling();
            // SAFETY: `slot` is non-null and aligned for `T`, which together
            //   are the whole validity requirement for a write of a zero-sized
            //   value: the write touches no byte, so there is no storage for it
            //   to fall outside of and none for the value to outlive.
            unsafe { ptr::write(slot.as_ptr(), value) };
            return BlockPtr(slot);
        }

        let mut slot = self.bump(size, align);
        if slot.is_null() {
            self.grow(size + align.saturating_sub(CHUNK_ALIGN));
            slot = self.bump(size, align);
        }
        // Unfireable, and it is the proof's own statement: a chunk grown for
        // `required(T)` fits `T`. `grow` makes `cap >= size + align - 64` and
        // bases are 64-aligned, so the leading pad is at most `align - 64` and
        // `pad + size <= cap`. If this ever fires, the growth rule drifted from
        // the padding rule and the `new_unchecked` below is the next casualty.
        debug_assert!(
            !slot.is_null(),
            "invariant: a chunk grown for required(T) admits T"
        );

        let typed = slot.cast::<T>();
        // SAFETY: `bump` returned a non-null address of `size` bytes inside a
        //   chunk this block owns, aligned to `align_of::<T>()`, and advanced
        //   `cur` past them — so no later `emplace` can hand the same bytes out
        //   again and nothing has read or written them since `alloc`.
        //   `ptr::write` takes `*mut T` by value, so it forms no reference; it
        //   moves `value` in without dropping the uninitialised bytes it
        //   overwrites.
        unsafe { ptr::write(typed, value) };
        // SAFETY: `typed` is `slot`, which the `debug_assert!` above documents
        //   and the growth rule guarantees to be non-null.
        BlockPtr(unsafe { NonNull::new_unchecked(typed) })
    }

    /// Free every chunk this block owns and return it to its lazy state.
    ///
    /// `&self` and no `Drop` impl, both deliberately (module header). The
    /// single call site runs this immediately after `Scope::drop`'s join
    /// returns, where nothing between the join and the call can panic — so
    /// leak-safety is structural.
    ///
    /// For the same reason NOTHING IN HERE MAY PANIC — and the construct that
    /// could is the RANGE SLICE, not the layout. `self.bases[..n]` and
    /// `self.exps[..n]` panic if `n > MAX_CHUNKS`, whereas
    /// `Layout::from_size_align_unchecked` and `dealloc` cannot panic at all;
    /// the unchecked constructor is chosen for the reason its own SAFETY
    /// comment gives, not to dodge an unwind. (In a debug build the
    /// `CHUNK0 << exp` in the loop is a second such site, footed identically.)
    ///
    /// That slice cannot fire, but it is held up by a RUNTIME invariant on a
    /// `Cell<u32>` rather than by the structure: `n_chunks`'s only other writer
    /// is [`grow`](Self::grow), which aborts at `MAX_CHUNKS` and stores
    /// `i as u32 + 1 <= 32`. Should the invariant ever break, the unwind lands
    /// BEFORE the first `dealloc`, so every one of the `n` chunks leaks and
    /// `n_chunks` is never reset — the failure is exactly the leak this
    /// paragraph exists to prevent, and it is unretryable.
    ///
    /// The reset at the end makes a second call a no-op. It is defence, not a
    /// licence: the contract below still says once.
    ///
    /// # Safety
    ///
    /// * Every value emplaced into this block must already be dead. The storage
    ///   goes back to the allocator, so a value still owned by it is
    ///   invalidated without being dropped. On the scoped-task path that is
    ///   what the join establishes: a task that RAN moved its body out of the
    ///   cell, and a task that did NOT run is still counted in `pending`, so
    ///   the join has not returned and this call has not happened.
    /// * No [`BlockPtr`] this block handed out may be dereferenced afterwards,
    ///   and no erased copy of one may either.
    pub(crate) unsafe fn free_all(&self) {
        let n = self.n_chunks.get() as usize;
        for (base, exp) in self.bases[..n].iter().zip(&self.exps[..n]) {
            let cap = CHUNK0 << u32::from(exp.get());
            // SAFETY: `base` and `exp` are the pair `grow` recorded together
            //   for one chunk index below `n_chunks`, so they describe a live
            //   allocation this block still owns and has not yet freed (the
            //   reset below is what keeps a second call from reaching it).
            //   `(cap, CHUNK_ALIGN)` is the SAME pair `Layout::from_size_align`
            //   accepted at the allocation site — a power of two `>= 4096` at
            //   alignment 64 — which is both why the unchecked construction is
            //   valid and why it is the layout `dealloc` demands.
            unsafe {
                dealloc(
                    base.get(),
                    Layout::from_size_align_unchecked(cap, CHUNK_ALIGN),
                )
            };
        }
        self.n_chunks.set(0);
        self.cur.set(ptr::null_mut());
        self.end.set(ptr::null_mut());
    }

    /// Take `size` bytes at `align` out of the current chunk, or return null if
    /// they do not fit.
    ///
    /// Null as the "does not fit" answer rather than `Option<NonNull<u8>>`: the
    /// caller's next move is a growth and a retry either way, and a null cursor
    /// is already the lazy block's own state, so one test covers both.
    ///
    /// Non-generic deliberately, and `#[inline]` anyway — the two do not
    /// contradict. `emplace` monomorphizes per body type; nothing here depends
    /// on `T` beyond two integers, so ONE out-of-line copy of this body exists
    /// rather than one per instantiation. The hint is still right, because
    /// those two integers are CONSTANTS at every call site: they arrive from
    /// `size_of` / `align_of`, so an inlined copy folds `pad` to a constant
    /// mask and collapses both comparisons against a compile-time size. That is
    /// a benefit, not a duplicate for nothing. Plain `#[inline]` is a hint and
    /// not `#[inline(always)]`, so the measured-inlining rule is not engaged.
    #[inline]
    fn bump(&self, size: usize, align: usize) -> *mut u8 {
        debug_assert!(
            align.is_power_of_two(),
            "invariant: align_of::<T>() is a power of two"
        );
        let cur = self.cur.get();
        // `end >= cur` always: `grow` sets the pair together, and the only
        // other writer is the line below, which first proves what it advances
        // by. On a lazy block both are null and this is 0.
        let remaining = self.end.get().addr() - cur.addr();
        // `(align - cur % align) % align` without a division, and without
        // overflowing at `cur == 0`.
        let pad = cur.addr().wrapping_neg() & (align - 1);
        // Split in two rather than `pad + size > remaining`, because the sum
        // can wrap where the difference cannot: `pad <= remaining` is
        // established before `remaining - pad` is formed.
        if pad > remaining || size > remaining - pad {
            return ptr::null_mut();
        }
        // SAFETY: `pad + size <= remaining`, so both offsets land inside the
        //   chunk `cur` points into, or on its one-past-the-end address, which
        //   is a valid place for a pointer to rest. That chunk is a single live
        //   allocation this block owns, so `cur`'s provenance covers the whole
        //   range being offset over.
        unsafe {
            let slot = cur.add(pad);
            self.cur.set(slot.add(size));
            slot
        }
    }

    /// Append a chunk large enough for `required` bytes and make it current.
    ///
    /// `required` is the caller's
    ///
    /// ```text
    /// required(T) = size_of::<T>() + align_of::<T>().saturating_sub(CHUNK_ALIGN)
    /// e_i         = max(i, smallest e such that (CHUNK0 << e) >= required(T))
    /// cap_i       = CHUNK0 << e_i
    /// ```
    ///
    /// and the `saturating_sub` is the whole padding argument: `bases[i]` is
    /// `CHUNK_ALIGN`-aligned and `size_of::<T>()` is a multiple of
    /// `align_of::<T>()` for every Rust type, so cells never need padding
    /// BETWEEN them and only the first cell of a chunk can need a leading pad —
    /// at most `align_of::<T>() - CHUNK_ALIGN` bytes, and zero for AVX2 (32)
    /// and AVX-512 (64), which is why this engine's ISA baseline costs nothing
    /// here.
    ///
    /// The `max(i, ..)` term is the doubling and the other is the fit; taking
    /// the larger is what keeps a single oversized cell from resetting the
    /// growth curve.
    ///
    /// `#[inline(never)]`, not `#[cold]`: growth is a normal event — once per
    /// chunk — but a rare branch off `emplace`, and out-of-line is what keeps
    /// the monomorphized fast path down to a test and a call.
    #[inline(never)]
    fn grow(&self, required: usize) {
        let i = self.n_chunks.get() as usize;
        if i == MAX_CHUNKS {
            chunk_table_exhausted();
        }
        // `ceil(log2(required)) - log2(CHUNK0)`: the smallest `e` whose chunk
        // holds `required`. `required >= 1` here because `emplace` returns
        // before growing for a zero-sized `T`, so `ilog2` is defined.
        let min_e = if required <= CHUNK0 {
            0
        } else {
            (required - 1).ilog2() + 1 - CHUNK0.ilog2()
        };
        // THE BOUND ON `e`, which everything below rests on and which is NOT a
        // module fact. Rust's `<<` checks the SHIFT AMOUNT and nothing else: it
        // discards value bits in silence, so `CHUNK0 << e` is 0 — not a panic,
        // not a saturation — for every `e` in `52..=63`, and is 2^63 at 51. The
        // `usize::BITS >= 64` assert at the top of this file bounds the `i` term
        // alone (`i < MAX_CHUNKS`, so `CHUNK0 << i <= 2^43`); nothing in this
        // module bounds `min_e`, because what bounds it is the COMPILER:
        //
        //   rustc refuses to lay out a type of size `1 << 61` or larger, and
        //   caps `#[repr(align(N))]` at `1 << 29`. So `required = size_of::<T>()
        //   + align_of::<T>().saturating_sub(64) < 2^61 + 2^29`, giving
        //   `min_e <= 50` and `cap = CHUNK0 << e <= 2^62`, which is half of
        //   `isize::MAX`.
        //
        // A `T` that could push `e` to 51 is a `T` rustc rejects at its
        // definition, so the wrap is unreachable rather than improbable.
        let e = min_e.max(i as u32);
        let cap = CHUNK0 << e;
        // ASSERTED AT THE PRODUCER, and that is the point of the line: the
        // receipts' chunk bucket is DEFINED by this predicate, so it is stated
        // where the layout is made rather than assumed where it is counted.
        debug_assert!(
            cap.is_power_of_two() && cap >= CHUNK0,
            "invariant: every chunk layout is a power of two at least CHUNK0"
        );
        // `expect`, and it is a panic by design — but this constructor is NOT
        // what would catch a wrapped `cap`, and a reader who assumes it is has
        // the wrong failure mode: `Layout::from_size_align(0, 64)` returns
        // `Ok`. A `cap` of 0 sails through here and becomes UB one line down.
        // What this call does rule out is the other error mode — a `cap` whose
        // round-up to `CHUNK_ALIGN` passes `isize::MAX`, reachable only at
        // `e == 51` — and the bound above says `e <= 50`. So the `expect` is
        // unfireable, not merely unlikely. Were it ever to fire, the panic
        // unwinds out of `Scope::spawn`, where `Scope::drop` still joins and
        // still frees.
        let layout = Layout::from_size_align(cap, CHUNK_ALIGN)
            .expect("invariant: chunk capacity is a power of two at CHUNK_ALIGN");
        // SAFETY: `alloc`'s one precondition is a layout of non-zero size, and
        //   what supplies it here is `e <= 50` from the bound above, not
        //   `cap >= CHUNK0` — that inequality is a CONSEQUENCE of the bound and
        //   not a syntactic fact, since `CHUNK0 << e` is 0 for `e >= 52`. With
        //   `e <= 50` no value bit is discarded, so `cap` is a power of two in
        //   `[2^12, 2^62]` and cannot be zero.
        let base = unsafe { alloc(layout) };
        if base.is_null() {
            handle_alloc_error(layout);
        }
        self.bases[i].set(base);
        // Lossless UNCONDITIONALLY, and a later reader should not fold this
        // into the bound above: `required` is a `usize`, so the formula cannot
        // return more than 52 on ANY input from `usize`'s width alone, and
        // `i < MAX_CHUNKS`. No compiler bound is needed for this one. Had it
        // ever truncated, `free_all` would `dealloc` the chunk under a layout
        // different from the one `alloc` produced it at.
        self.exps[i].set(e as u8);
        self.n_chunks.set(i as u32 + 1);
        self.cur.set(base);
        // SAFETY: `base` is the start of a live `cap`-byte allocation, so
        //   `base + cap` is its one-past-the-end address — in bounds for the
        //   offset, and never dereferenced.
        self.end.set(unsafe { base.add(cap) });
    }
}

/// The chunk table is full.
///
/// Unreachable while `alloc` succeeds, and kept as defence rather than deleted.
/// At [`MAX_CHUNKS`] the table's total capacity is `CHUNK0 * (2^32 - 1)` ~ 16
/// TiB and the 32nd chunk alone is 8 TiB, so the allocator refuses first and
/// `handle_alloc_error` — the failure mode the cell allocator already has — is
/// what a caller actually meets. This abort is what keeps that a proof instead
/// of an assumption: if it ever fires, the growth rule is wrong, not the
/// machine.
///
/// An abort rather than a panic because a panic here is catchable on the task
/// path, and a bump allocator that has silently lost its table must not be
/// recovered from.
///
/// `#[inline(never)]` so the symbol survives into the crash's stack: this
/// function's NAME is the whole diagnosis. The `boyko_log` E-code such a site is
/// owed is deliberately not minted here — a code is a workspace-level act with
/// four gates behind it (a registry row, a `docs/diagnostics/` page, an
/// identifier-use check and a ledger row for a code no test can name), and it
/// does not belong to a module that is wired to nothing.
#[cold]
#[inline(never)]
fn chunk_table_exhausted() -> ! {
    std::process::abort();
}

/// A pointer to a value living in a [`ScopeBlock`], whose ONLY exit is
/// [`erase`](BlockPtr::erase).
///
/// The absent API is the type. There is no `Deref`, no `as_mut`, no accessor
/// and no `From`, so a holder cannot form a REFERENCE into chunk memory — and a
/// reference passed as a function argument is what Tree Borrows installs a
/// protector over, live for the callee's whole activation. `erase` hands back a
/// `*const ()`, which is inert until someone casts it to a cell type that is
/// private to another module. Adding a single accessor here re-opens that
/// question for the whole crate, which is why the absence is documented as the
/// feature it is.
///
/// `#[repr(transparent)]`: this is exactly a pointer, and `Task`'s payload word
/// is what it becomes.
///
/// `#[must_use]` because dropping this handle drops a POINTER and the value it
/// names is then never run and never dropped — `free_all` hands the bytes back
/// raw. It is the one hazard the absent API does not already make
/// unconstructible, so the type system is made to check it here too.
#[must_use]
#[repr(transparent)]
pub(crate) struct BlockPtr<T>(NonNull<T>);

impl<T> BlockPtr<T> {
    /// Consume the pointer and forget its type.
    ///
    /// By value rather than `&self` so the typed handle cannot outlive its own
    /// erasure: after this call the only thing naming the cell is a `*const ()`
    /// and whatever private type its holder can cast it back to.
    #[inline]
    pub(crate) fn erase(self) -> *const () {
        self.0.as_ptr().cast_const().cast::<()>()
    }
}

#[cfg(test)]
mod tests {
    //! Property set R5 for the per-scope bump allocator, plus the edge cases the
    //! design names: `n == 0`, a ZST body, `align_of::<T>() > CHUNK_ALIGN`,
    //! `size_of::<T>() > CHUNK0`, and a body that panics mid-wave.
    //!
    //! These live HERE and not in `tests/`: [`ScopeBlock`] and [`BlockPtr`] are
    //! `pub(crate)`, so an integration binary cannot name either, and the
    //! properties below are about placement inside memory the crate does not
    //! export.
    //!
    //! # The instrument, and why the properties are stated over it
    //!
    //! "Every chunk layout is `align == 64 && size.is_power_of_two() &&
    //! size >= 4096`" and "`free_all` frees each recorded base exactly once" are
    //! statements about calls into the ALLOCATOR. Neither is observable from the
    //! block's own fields, and a test that read `bases` / `exps` back would be
    //! asking the code what it did. So the module installs a
    //! [`RecordingAlloc`] — a `#[global_allocator]` that delegates to `System`
    //! and, while a [`Recording`] guard is live ON THE CALLING THREAD, appends
    //! `(acquire, addr, size, align)` to a fixed thread-local log. `ScopeBlock`
    //! is `!Sync`, so every event it causes happens on the thread that asked for
    //! it, and the harness's other tests can run in parallel undisturbed.
    //!
    //! `#[cfg(test)]` bounds the blast radius exactly: this code is compiled
    //! into the lib test target and nowhere else — not into an integration test,
    //! not into a bench, not into a dependent crate.
    //!
    //! # Four things a naive test of this module gets WRONG
    //!
    //! 1. **A ZST's pointers are all the same address.** `emplace` of a
    //!    zero-sized `T` allocates no chunk and returns `NonNull::dangling()`,
    //!    deliberately (the general path would have handed back a lazy block's
    //!    null `cur`). So disjointness is stated over HALF-OPEN ranges
    //!    `[addr, addr + size)`, which are EMPTY for a ZST and therefore
    //!    disjoint from everything including each other. A property spelled
    //!    "every returned pointer is distinct" reds on exactly the input the ZST
    //!    branch exists for — see
    //!    [`zero_sized_emplaces_share_one_address_and_still_satisfy_disjointness`].
    //! 2. **`emplace` after `free_all` restarts at chunk 0**, because
    //!    `e = max(min_e, i)` reads `i` from a reset `n_chunks`. A chunk-count
    //!    model carried cumulatively across a `free_all` is wrong — see
    //!    [`emplace_after_free_all_restarts_the_growth_curve_at_chunk_zero`].
    //! 3. **`grow` abandons the tail of the chunk it leaves**, so
    //!    `floor(cap_i / stride)` counts a chunk's occupants only when the first
    //!    one sits at offset 0. That holds for `align <= CHUNK_ALIGN` and for
    //!    nothing above it, where the leading pad is `-base % align` — an
    //!    address no test can predict. [`model_chunk_caps`] therefore REFUSES an
    //!    over-aligned type, and the over-aligned case is covered by placement
    //!    rather than by counting.
    //! 4. **The cell class's bucket is a `(size, align)` PAIR.** The receipt
    //!    constants pin both halves; this module's contribution is
    //!    [`the_chunk_bucket_and_the_cell_bucket_cannot_hold_the_same_allocation`],
    //!    which is what makes a later stage's "zero allocations in the cell size
    //!    class" a statement about a non-empty bucket.

    use std::alloc::{GlobalAlloc, System};
    use std::io::Write;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::process::Command;

    use super::*;

    // =====================================================================
    // The instrument
    // =====================================================================

    /// One allocator event.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Ev {
        /// `true` for an acquisition (`alloc` / `alloc_zeroed` / the new half of
        /// a `realloc`), `false` for a release.
        acquire: bool,
        /// The address the allocator returned, or the one it was handed back.
        addr: usize,
        /// The layout's size AS REQUESTED — not as the system allocator may
        /// have rounded it. The chunk bucket is about what this crate asks for.
        size: usize,
        /// The layout's alignment, as requested.
        align: usize,
    }

    impl Ev {
        const ZERO: Self = Self {
            acquire: false,
            addr: 0,
            size: 0,
            align: 0,
        };
    }

    /// Events one recorded region can hold. The largest region in this module is
    /// a ten-chunk wave plus its frees, so this is generous; a region that
    /// overflows it bumps [`REC_DROPPED`], and [`Recording::stop`] refuses to
    /// return a truncated log — a log in which a MISSING `dealloc` and a
    /// DROPPED one look the same is a log that cannot fail.
    const REC_CAP: usize = 256;

    thread_local! {
        /// Whether the calling thread is inside a [`Recording`] region.
        static REC_ON: Cell<bool> = const { Cell::new(false) };
        /// Events written so far in the current region.
        static REC_LEN: Cell<usize> = const { Cell::new(0) };
        /// Events the current region could not store.
        static REC_DROPPED: Cell<usize> = const { Cell::new(0) };
        /// The log itself. An array of `Cell`s rather than one `Cell` of an
        /// array, because a single-element write must not copy the whole tape.
        /// Const-initialised and drop-free either way, which is what lets the
        /// allocator touch it without re-entering itself.
        static REC_LOG: [Cell<Ev>; REC_CAP] = const { [const { Cell::new(Ev::ZERO) }; REC_CAP] };
    }

    /// Append one event. Allocates nothing, by construction.
    fn record(acquire: bool, addr: usize, size: usize, align: usize) {
        if !REC_ON.try_with(Cell::get).unwrap_or(false) {
            return;
        }
        let i = REC_LEN.get();
        if i == REC_CAP {
            REC_DROPPED.set(REC_DROPPED.get() + 1);
            return;
        }
        REC_LOG.with(|log| {
            log[i].set(Ev {
                acquire,
                addr,
                size,
                align,
            });
        });
        REC_LEN.set(i + 1);
    }

    /// The lib test target's global allocator: `System`, plus a thread-local
    /// tape that is off unless a [`Recording`] guard is live.
    struct RecordingAlloc;

    // SAFETY: every method forwards its pointer and layout to `System`
    //   unchanged and returns what `System` returned, so this allocator's
    //   contract IS `System`'s. The added side effect is a store into a
    //   const-initialised, drop-free thread-local `Cell`, which performs no
    //   allocation and therefore cannot re-enter the allocator.
    unsafe impl GlobalAlloc for RecordingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: forwarded verbatim to the system allocator.
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                record(true, p.addr(), layout.size(), layout.align());
            }
            p
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: forwarded verbatim to the system allocator.
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                record(true, p.addr(), layout.size(), layout.align());
            }
            p
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: forwarded verbatim to the system allocator.
            let p = unsafe { System.realloc(ptr, layout, new_size) };
            // A failed `realloc` leaves the old block LIVE, so the release is
            // recorded only on the path that actually released it.
            if !p.is_null() {
                record(false, ptr.addr(), layout.size(), layout.align());
                record(true, p.addr(), new_size, layout.align());
            }
            p
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            record(false, ptr.addr(), layout.size(), layout.align());
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.dealloc(ptr, layout) };
        }
    }

    // NOT INSTALLED UNDER MIRI, and that is a measurement rather than a
    // precaution. Miri interprets a custom global allocator instead of
    // replacing it, so the delegate below becomes interpreted code — and std's
    // Windows `System` frees an OVER-ALIGNED block through a pointer it walks
    // backwards out of the caller's provenance (`HeapFree(heap, 0, block)`,
    // `std/src/sys/alloc/windows.rs:199`), which Tree Borrows rejects. The
    // harness's own `CachePadded` mpmc channel is such an allocation, so the
    // report lands during harness teardown with `ScopeBlock` nowhere on the
    // stack.
    //
    // MEASURED on this tree, `nightly-x86_64-pc-windows-gnu`, the repo's pinned
    // `-Zmiri-tree-borrows`:
    //   * `cargo miri test -p boyko-threadpool --lib -- task::tests` WITHOUT this
    //     module: `8 passed; 0 failed`.
    //   * with the allocator installed unconditionally: UB reported, and it
    //     survives DELETING EVERY `record()` CALL — so the cause is the
    //     delegation, not the tape.
    // Installing it unconditionally would therefore turn this crate's Miri gate
    // red on a std-internal issue, and that gate is where `task.rs`'s own test
    // module says its "still frees" claim is decided.
    #[cfg(not(miri))]
    #[global_allocator]
    static RECORDING_ALLOC: RecordingAlloc = RecordingAlloc;

    /// Arms the recorder for the calling thread; disarms it on drop, so an
    /// unwind through a recorded region cannot leave the tape running for
    /// whatever the harness schedules on this thread next.
    struct Recording;

    impl Recording {
        fn start() -> Self {
            refuse_under_miri();
            REC_LEN.set(0);
            REC_DROPPED.set(0);
            REC_ON.set(true);
            Self
        }

        /// Stop recording and return the region's events in order.
        ///
        /// Recording is disarmed BEFORE the `Vec` is built, or the collection
        /// would record itself.
        fn stop(self) -> Vec<Ev> {
            REC_ON.set(false);
            let n = REC_LEN.get();
            assert_eq!(
                REC_DROPPED.get(),
                0,
                "the recorded region overflowed REC_CAP ({REC_CAP}); every assertion over it \
                 would be reading a truncated log"
            );
            REC_LOG.with(|log| log[..n].iter().map(Cell::get).collect())
        }
    }

    /// Refuse to arm the recorder under Miri.
    ///
    /// The allocator above is not installed there, so a region would record
    /// nothing and every assertion over its log would pass VACUOUSLY. This turns
    /// that into a loud failure instead: a test that reaches it under Miri is
    /// one that is missing its `#[cfg_attr(miri, ignore = ...)]`, which is the
    /// only way this module's Miri skips can rot.
    ///
    /// A `cfg`-split pair rather than `assert!(!cfg!(miri), ..)`, because the
    /// latter is an assertion on a constant and the crate's clippy gate denies
    /// those.
    #[cfg(miri)]
    fn refuse_under_miri() {
        panic!(
            "the recording allocator is not installed under Miri; a test that needs it must              carry #[cfg_attr(miri, ignore = ...)]"
        );
    }

    /// The native arm of [`refuse_under_miri`]: nothing to refuse.
    #[cfg(not(miri))]
    fn refuse_under_miri() {}

    impl Drop for Recording {
        fn drop(&mut self) {
            REC_ON.set(false);
        }
    }

    /// The acquisitions in a region, in order.
    fn acquisitions(log: &[Ev]) -> Vec<Ev> {
        log.iter().copied().filter(|e| e.acquire).collect()
    }

    /// The releases in a region, in order.
    fn releases(log: &[Ev]) -> Vec<Ev> {
        log.iter().copied().filter(|e| !e.acquire).collect()
    }

    /// The chunk bucket, spelled the way the RECEIPT spells it — over
    /// `__layout_receipt`'s published literals rather than over this module's
    /// own constants, because the receipt's numbers are what a later stage keys
    /// on. (The two are tied by the `const _: () = assert!(..)` pins at the top
    /// of this file, so the choice cannot drift.)
    fn is_chunk_class(e: Ev) -> bool {
        e.align == crate::__layout_receipt::CHUNK_ALIGN
            && e.size.is_power_of_two()
            && e.size >= crate::__layout_receipt::CHUNK0
    }

    // =====================================================================
    // Bodies
    // =====================================================================

    /// A test body carrying a value the test can read back out of block memory.
    trait Shape: Copy {
        /// A value distinguishable by `i`.
        fn make(i: u64) -> Self;
        /// What [`Shape::make`] put in, recovered.
        fn tag(self) -> u64;
    }

    /// The degenerate body. `emplace` returns `NonNull::dangling()` for it and
    /// allocates nothing — trap 1 in the module header.
    #[derive(Clone, Copy)]
    struct Zst;

    impl Shape for Zst {
        fn make(_: u64) -> Self {
            Self
        }
        fn tag(self) -> u64 {
            0
        }
    }

    /// Alignment 1: the case where `pad` is 0 for a reason other than the chunk
    /// base's own alignment.
    #[derive(Clone, Copy)]
    struct Byte(u8);

    impl Shape for Byte {
        fn make(i: u64) -> Self {
            Self(i as u8)
        }
        fn tag(self) -> u64 {
            u64::from(self.0)
        }
    }

    /// The smallest body with a full-width tag.
    #[derive(Clone, Copy)]
    struct Word(u64);

    impl Shape for Word {
        fn make(i: u64) -> Self {
            Self(i)
        }
        fn tag(self) -> u64 {
            self.0
        }
    }

    /// The SHIPPED shape: `size_of::<ScopedCell<[u8; 72]>>()` is
    /// `SCOPED_CELL_HEADER + 72 == 88`, the stride the design's 1 / 1 / 2 / 5
    /// chunk prediction is stated at.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Cell88 {
        tag: u64,
        shared: u64,
        body: [u8; 72],
    }

    impl Shape for Cell88 {
        fn make(i: u64) -> Self {
            Self {
                tag: i,
                shared: 0,
                body: [0; 72],
            }
        }
        fn tag(self) -> u64 {
            self.tag
        }
    }

    /// Exactly `CHUNK0` bytes: the boundary of `min_e`'s `required <= CHUNK0`
    /// branch.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Page4096 {
        tag: u64,
        rest: [u8; 4088],
    }

    impl Shape for Page4096 {
        fn make(i: u64) -> Self {
            Self {
                tag: i,
                rest: [0; 4088],
            }
        }
        fn tag(self) -> u64 {
            self.tag
        }
    }

    /// `size_of::<T>() > CHUNK0` — a body no base chunk can hold.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Huge8200 {
        tag: u64,
        rest: [u8; 8192],
    }

    impl Shape for Huge8200 {
        fn make(i: u64) -> Self {
            Self {
                tag: i,
                rest: [0; 8192],
            }
        }
        fn tag(self) -> u64 {
            self.tag
        }
    }

    /// `align_of::<T>() > CHUNK_ALIGN` — the body `grow`'s
    /// `align.saturating_sub(CHUNK_ALIGN)` term exists for.
    #[repr(C, align(128))]
    #[derive(Clone, Copy)]
    struct Over128 {
        tag: u64,
        rest: [u8; 248],
    }

    impl Shape for Over128 {
        fn make(i: u64) -> Self {
            Self {
                tag: i,
                rest: [0; 248],
            }
        }
        fn tag(self) -> u64 {
            self.tag
        }
    }

    // =====================================================================
    // Claims, and the properties stated over them
    // =====================================================================

    /// One emplaced value as the properties see it: the address it came back at,
    /// and the HALF-OPEN range `[addr, addr + size)` it claims.
    #[derive(Clone, Copy, Debug)]
    struct Claim {
        addr: usize,
        size: usize,
        align: usize,
    }

    /// Emplace one `T` and describe the range it claimed.
    ///
    /// [`BlockPtr::erase`] is the handle's only exit (D2), so this is also the
    /// only way a test can learn an address at all.
    fn claim<T: Shape>(block: &ScopeBlock, i: u64) -> Claim {
        Claim {
            addr: block.emplace(T::make(i)).erase().addr(),
            size: size_of::<T>(),
            align: align_of::<T>(),
        }
    }

    /// Emplace `n` values of `T`, appending their claims to `out`.
    ///
    /// `out` must already have room: a recorded region has to contain the
    /// block's allocations and NOTHING else, and a `Vec` that grew mid-wave
    /// would put its own acquisition on the tape.
    fn wave<T: Shape>(block: &ScopeBlock, n: usize, out: &mut Vec<Claim>) {
        assert!(
            out.capacity() - out.len() >= n,
            "test setup: pre-size the claim vector so the wave itself allocates nothing"
        );
        for i in 0..n {
            out.push(claim::<T>(block, i as u64));
        }
    }

    /// Free the block's chunks.
    ///
    /// Every [`Shape`] in this module is `Copy`, so no emplaced value owns a
    /// resource or has a destructor, which discharges `free_all`'s "every value
    /// emplaced must already be dead" clause by TYPE rather than by care.
    fn free(block: &ScopeBlock) {
        // SAFETY: every body emplaced in this module is `Copy` and therefore
        //   owns nothing and drops nothing, so no value is invalidated without
        //   being dropped; and no test dereferences an erased pointer after
        //   calling this.
        unsafe { block.free_all() };
    }

    /// Two half-open ranges overlap iff each starts before the other ends.
    ///
    /// `O(n^2)` on purpose. A sort-based sweep has to decide where an EMPTY
    /// range sorts relative to a non-empty one containing its address, and this
    /// predicate simply has no such case: an empty range overlaps nothing, which
    /// is precisely why a ZST — whose pointers all share one address —
    /// satisfies the property rather than refuting it.
    fn assert_pairwise_disjoint(claims: &[Claim]) {
        for (i, a) in claims.iter().enumerate() {
            for (j, b) in claims.iter().enumerate().skip(i + 1) {
                assert!(
                    !(a.addr < b.addr + b.size && b.addr < a.addr + a.size),
                    "claims {i} {a:?} and {j} {b:?} overlap"
                );
            }
        }
    }

    /// Every claim is aligned for its type, and every non-empty one lies wholly
    /// inside one of `chunks`.
    fn assert_in_bounds_and_aligned(claims: &[Claim], chunks: &[Ev]) {
        for (i, c) in claims.iter().enumerate() {
            assert_eq!(
                c.addr % c.align,
                0,
                "claim {i} {c:?} is not aligned for its type"
            );
            if c.size == 0 {
                // A zero-sized value claims no byte and therefore lives in NO
                // chunk: `emplace` returns `NonNull::dangling()` without
                // growing. Asserting "inside a chunk" here would red on the one
                // input that branch exists for.
                assert_eq!(
                    c.addr, c.align,
                    "claim {i} {c:?}: a zero-sized value must come back at the dangling address"
                );
                continue;
            }
            let inside = chunks
                .iter()
                .any(|k| c.addr >= k.addr && c.addr + c.size <= k.addr + k.size);
            assert!(
                inside,
                "claim {i} {c:?} is not inside any chunk the block allocated: {chunks:?}"
            );
        }
    }

    // =====================================================================
    // The growth model — an INDEPENDENT re-derivation
    // =====================================================================

    /// The capacities the growth rule must produce for `n` emplacements of a
    /// body of this `size` and `align`, derived from the design text:
    ///
    /// ```text
    /// required(T) = size_of::<T>() + align_of::<T>().saturating_sub(64)
    /// e_i         = max(i, min e such that (4096 << e) >= required(T))
    /// cap_i       = 4096 << e_i
    /// ```
    ///
    /// The literals are spelled out rather than read from [`CHUNK0`] /
    /// [`CHUNK_ALIGN`]: a model that asked the code for its own constants could
    /// not disagree with it. They are tied to the published contract by
    /// [`the_model_is_written_over_the_receipts_own_numbers`].
    ///
    /// # Domain
    ///
    /// `align <= 64` only, and the restriction is load-bearing. `grow` ABANDONS
    /// whatever is left of the chunk it leaves, so a chunk holds exactly
    /// `floor(cap / size)` values — but only when the first one sits at offset
    /// 0. Chunk bases are 64-aligned, so that holds for every `align <= 64` and
    /// for no alignment above it, where the leading pad is `-base % align`, an
    /// address this model cannot know. Over-aligned bodies are covered by
    /// [`an_over_aligned_type_is_placed_aligned_and_inside_its_chunk`], which
    /// asserts placement instead of counts.
    fn model_chunk_caps(size: usize, align: usize, n: usize) -> Vec<usize> {
        assert!(
            size > 0 && align > 0 && align <= 64 && size.is_multiple_of(align),
            "model domain: a non-zero size that is a multiple of an alignment at most 64"
        );
        let mut caps: Vec<usize> = Vec::new();
        let mut placed = 0usize;
        while placed < n {
            let mut min_e = 0u32;
            while (4096usize << min_e) < size {
                min_e += 1;
            }
            let cap = 4096usize << min_e.max(caps.len() as u32);
            placed += cap / size;
            caps.push(cap);
        }
        caps
    }

    /// Run a `T` wave on a fresh block and report the chunk sizes it caused.
    fn caps_of<T: Shape>(n: usize) -> Vec<usize> {
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(n);
        let log = {
            let rec = Recording::start();
            wave::<T>(&block, n, &mut claims);
            rec.stop()
        };
        free(&block);
        acquisitions(&log).iter().map(|e| e.size).collect()
    }

    /// Every event — acquisitions AND releases — of one `T` wave plus its
    /// `free_all`.
    fn events_of<T: Shape>(n: usize) -> Vec<Ev> {
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(n);
        let rec = Recording::start();
        wave::<T>(&block, n, &mut claims);
        free(&block);
        rec.stop()
    }

    // =====================================================================
    // The instrument's own positive controls
    // =====================================================================

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn the_recorder_sees_a_known_acquisition_and_its_release() {
        // Without this, every "no allocation happened" assertion below could be
        // green from an inert recorder.
        let log = {
            let rec = Recording::start();
            let v = vec![0u64; 64];
            drop(v);
            rec.stop()
        };

        assert_eq!(
            log.len(),
            2,
            "one 512-byte acquisition and its release, and nothing else: {log:?}"
        );
        assert!(
            log[0].acquire && log[0].size == 512 && log[0].align == 8,
            "the acquisition is recorded with the layout that was requested: {:?}",
            log[0]
        );
        assert!(
            !log[1].acquire && log[1].addr == log[0].addr && log[1].size == 512,
            "the release is recorded against the same address and layout: {:?}",
            log[1]
        );
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn a_region_sees_only_the_events_of_its_own_region() {
        // The second control, and a DIFFERENT property from the first: a
        // region's log must be that region's events and nothing carried in from
        // an earlier one. Two regions on the SAME thread is the only shape that
        // states it falsifiably — a single region preceded by an allocation is
        // green whether or not the tape is reset, because the gate already
        // excluded that allocation. (MEASURED: deleting the reset left the
        // one-region form passing.)
        //
        // The gate's other job — keeping the harness's parallel threads apart —
        // is done by the tape being thread-local, and no mutation of this module
        // can break it, so it is not claimed as a tested property.
        let first = {
            let rec = Recording::start();
            let v = vec![0u64; 64];
            drop(v);
            rec.stop()
        };
        assert_eq!(first.len(), 2, "test setup: the first region recorded");

        let second = {
            let rec = Recording::start();
            rec.stop()
        };

        assert!(
            second.is_empty(),
            "a region's log is its OWN events; the previous region's must not carry in:              {second:?}"
        );
    }

    #[test]
    fn the_model_is_written_over_the_receipts_own_numbers() {
        // `model_chunk_caps` hardcodes 4096 and 64 so that it cannot agree with
        // the code by construction. This is the line that keeps those literals
        // honest.
        assert_eq!(
            4096usize,
            crate::__layout_receipt::CHUNK0,
            "the model's base capacity is the receipt's CHUNK0"
        );
        assert_eq!(
            64usize,
            crate::__layout_receipt::CHUNK_ALIGN,
            "the model's alignment is the receipt's CHUNK_ALIGN"
        );
    }

    // =====================================================================
    // R5 — placement
    // =====================================================================

    #[test]
    fn erase_returns_the_address_the_value_was_written_to() {
        let block = ScopeBlock::new();

        let p = block.emplace(Word::make(0xDEAD_BEEF)).erase();

        // SAFETY: `p` is the address `emplace` wrote a live `Word` to; the block
        //   still owns the chunk (no `free_all` yet) and nothing has handed the
        //   same bytes out twice. `ptr::read` copies the value out through a RAW
        //   pointer and forms no reference into chunk memory, which is the
        //   condition D2 exists to preserve.
        let v: Word = unsafe { ptr::read(p.cast::<Word>()) };
        assert_eq!(
            v.tag(),
            0xDEAD_BEEF,
            "emplace MOVES the value into block memory and erase names it"
        );

        free(&block);
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn every_emplaced_pointer_lies_inside_a_chunk_the_block_owns() {
        const N: usize = 400;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(N);
        let log = {
            let rec = Recording::start();
            wave::<Cell88>(&block, N, &mut claims);
            rec.stop()
        };
        let chunks = acquisitions(&log);
        free(&block);

        assert!(
            chunks.len() >= 2,
            "the wave must actually have grown more than once, else the property is trivial"
        );
        assert_in_bounds_and_aligned(&claims, &chunks);
    }

    #[test]
    fn every_emplaced_pointer_is_aligned_for_its_type() {
        // Four alignments, one of them ABOVE `CHUNK_ALIGN`.
        //
        // No recorded region: the property is about the ADDRESSES that came
        // back, and `n_chunks` is enough to witness that the mixed wave crossed
        // a chunk boundary. Reading the block's own counter for a SETUP check is
        // not "asking the code what it did" — the counter is not the property.
        // Being tape-free is what lets this one run under Miri, where the
        // allocator is not installed.
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(4 * 40);
        wave::<Byte>(&block, 40, &mut claims);
        wave::<Word>(&block, 40, &mut claims);
        wave::<Cell88>(&block, 40, &mut claims);
        wave::<Over128>(&block, 40, &mut claims);

        for (i, c) in claims.iter().enumerate() {
            assert_eq!(
                c.addr % c.align,
                0,
                "claim {i} {c:?} must be aligned for its type"
            );
        }
        assert!(
            block.n_chunks.get() >= 2,
            "the mixed wave must have grown, else alignment across a chunk boundary is untested"
        );

        free(&block);
    }

    #[test]
    fn emplaced_ranges_are_pairwise_disjoint_across_mixed_shapes() {
        // The ZST wave is IN the mix on purpose: its claims are empty ranges at
        // one shared address, which the half-open predicate accepts and a
        // "distinct pointers" predicate would reject.
        //
        // Tape-free like the alignment test above, and for the same reason: the
        // property is a relation among the returned ranges. Being inside a chunk
        // is a DIFFERENT property, and it has its own test.
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(5 * 30);
        wave::<Zst>(&block, 30, &mut claims);
        wave::<Byte>(&block, 30, &mut claims);
        wave::<Word>(&block, 30, &mut claims);
        wave::<Cell88>(&block, 30, &mut claims);
        wave::<Over128>(&block, 30, &mut claims);

        assert_pairwise_disjoint(&claims);
        assert!(
            block.n_chunks.get() >= 2,
            "the mixed wave must have grown, else disjointness across a chunk boundary is              untested"
        );

        free(&block);
    }

    #[test]
    fn a_pointer_handed_out_before_a_growth_still_reads_back_after_it() {
        const N: usize = 400;

        // Tape-free deliberately. This is the property Miri is BEST at — it
        // reads back through a raw pointer handed out several growths earlier —
        // so it must survive in a configuration where the recording allocator
        // cannot be installed. The companion claim, that a growth releases
        // nothing, is the tape's and lives in
        // `growth_appends_a_chunk_and_frees_none`.
        let block = ScopeBlock::new();
        let mut ptrs: Vec<*const ()> = Vec::with_capacity(N);
        for i in 0..N {
            ptrs.push(block.emplace(Cell88::make(i as u64)).erase());
        }
        let growths = block.n_chunks.get();
        assert!(
            growths >= 4,
            "the wave must have grown several times, else 'survives a growth' is untested"
        );

        for (i, p) in ptrs.iter().enumerate() {
            // SAFETY: `p` names a live `Cell88` this block still owns — it was
            //   emplaced above, `free_all` has not run, and every later growth
            //   only APPENDED a chunk, so the bytes this pointer names have not
            //   moved. `ptr::read` copies out through a raw pointer and forms no
            //   reference into chunk memory.
            let v: Cell88 = unsafe { ptr::read(p.cast::<Cell88>()) };
            assert_eq!(
                v.tag(),
                i as u64,
                "value {i}, handed out before {} growths, must still read back",
                growths - 1
            );
        }

        free(&block);
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn growth_appends_a_chunk_and_frees_none() {
        const N: usize = 400;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(N);
        let log = {
            let rec = Recording::start();
            wave::<Cell88>(&block, N, &mut claims);
            rec.stop()
        };
        let chunks = acquisitions(&log);

        // Asserted BEFORE the teardown: the mutation this test exists to catch is a
        // `grow` that releases the chunk it leaves, and a block in that state
        // DOUBLE-FREES here. Freeing first would crash the binary in place of naming
        // the property that broke.
        assert!(chunks.len() >= 2, "the wave must have grown");
        assert!(
            releases(&log).is_empty(),
            "growth releases nothing — there is no reallocation in this allocator: {log:?}"
        );
        for (i, a) in chunks.iter().enumerate() {
            for (j, b) in chunks.iter().enumerate().skip(i + 1) {
                assert!(
                    a.addr + a.size <= b.addr || b.addr + b.size <= a.addr,
                    "chunks {i} {a:?} and {j} {b:?} overlap"
                );
            }
        }

        free(&block);
    }

    // =====================================================================
    // R5 — the growth rule
    // =====================================================================

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn the_chunk_count_matches_the_designs_prediction_at_stride_88() {
        assert_eq!(
            size_of::<Cell88>(),
            88,
            "the stride the design's prediction is stated at"
        );

        // The design's own table, as literals: at stride 88 the chunk count for
        // n = 1 / 16 / 64 / 1024 is 1 / 1 / 2 / 5. Three independent statements
        // must agree — the literal, the model, and the block.
        for (n, predicted) in [(1usize, 1usize), (16, 1), (64, 2), (1024, 5)] {
            assert_eq!(
                model_chunk_caps(88, 8, n).len(),
                predicted,
                "the independent model must reproduce the design's prediction at n = {n}"
            );
            assert_eq!(
                caps_of::<Cell88>(n).len(),
                predicted,
                "the block must allocate the predicted number of chunks at n = {n}"
            );
        }
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn the_chunk_capacities_match_the_independent_growth_model() {
        // Capacities as a SEQUENCE, not just a count: a doubling curve that
        // started at the wrong exponent can still produce the right number of
        // chunks for a given n.
        for n in [0usize, 1, 46, 47, 139, 140, 400] {
            assert_eq!(
                caps_of::<Byte>(n),
                model_chunk_caps(1, 1, n),
                "stride 1, n = {n}"
            );
            assert_eq!(
                caps_of::<Word>(n),
                model_chunk_caps(8, 8, n),
                "stride 8, n = {n}"
            );
            assert_eq!(
                caps_of::<Cell88>(n),
                model_chunk_caps(88, 8, n),
                "stride 88, n = {n}"
            );
            assert_eq!(
                caps_of::<Page4096>(n),
                model_chunk_caps(4096, 8, n),
                "stride 4096, n = {n}"
            );
            assert_eq!(
                caps_of::<Huge8200>(n),
                model_chunk_caps(8200, 8, n),
                "stride 8200, n = {n}"
            );
        }
    }

    // =====================================================================
    // R5 — free_all
    // =====================================================================

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn free_all_frees_every_chunk_exactly_once() {
        const N: usize = 400;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(N);
        let log = {
            let rec = Recording::start();
            wave::<Cell88>(&block, N, &mut claims);
            free(&block);
            rec.stop()
        };

        let acq = acquisitions(&log);
        let rel = releases(&log);
        assert_eq!(acq.len(), 4, "stride 88 at n = 400 grows four chunks");
        assert_eq!(
            rel.len(),
            acq.len(),
            "one release per acquisition and no more: {log:?}"
        );
        for a in &acq {
            let freed: Vec<Ev> = rel.iter().copied().filter(|r| r.addr == a.addr).collect();
            assert_eq!(
                freed.len(),
                1,
                "the chunk at {:#x} must be released exactly once, saw {}",
                a.addr,
                freed.len()
            );
            assert_eq!(
                (freed[0].size, freed[0].align),
                (a.size, a.align),
                "a chunk must be released under the layout it was acquired at"
            );
        }
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn free_all_frees_every_chunk_exactly_once_after_an_unwind_mid_wave() {
        const N: usize = 400;
        const PANIC_AT: usize = 300;

        let block = ScopeBlock::new();

        // The interrupted wave is deliberately NOT inside a recorded region: a
        // caught panic runs the default hook, whose formatting allocates an
        // unbounded number of times (an environment with `RUST_BACKTRACE=1`
        // allocates hundreds), and those events would crowd out the ones the
        // frees are matched against.
        let caught = catch_unwind(AssertUnwindSafe(|| {
            for i in 0..N {
                assert!(i != PANIC_AT, "mid-wave unwind, on purpose");
                let _ = block.emplace(Cell88::make(i as u64)).erase();
            }
        }));
        assert!(caught.is_err(), "the wave must actually have unwound");

        // The left-hand side of "frees each RECORDED base exactly once" is the
        // block's own table; the right-hand side is the allocator's tape.
        let owned: Vec<(usize, usize)> = (0..block.n_chunks.get() as usize)
            .map(|i| {
                (
                    block.bases[i].get().addr(),
                    CHUNK0 << u32::from(block.exps[i].get()),
                )
            })
            .collect();
        assert_eq!(
            owned.len(),
            model_chunk_caps(88, 8, PANIC_AT).len(),
            "the unwound wave placed {PANIC_AT} values, so the model's chunk count applies"
        );

        let log = {
            let rec = Recording::start();
            free(&block);
            rec.stop()
        };

        let rel = releases(&log);
        assert_eq!(
            rel.len(),
            owned.len(),
            "free_all releases exactly the chunks the table records: {log:?}"
        );
        for (base, cap) in &owned {
            let freed: Vec<Ev> = rel.iter().copied().filter(|r| r.addr == *base).collect();
            assert_eq!(
                freed.len(),
                1,
                "the chunk at {base:#x} must be released exactly once on the unwinding path"
            );
            assert_eq!(
                (freed[0].size, freed[0].align),
                (*cap, CHUNK_ALIGN),
                "a chunk must be released under the layout it was acquired at"
            );
        }
    }

    #[test]
    fn free_all_returns_the_block_to_its_lazy_state() {
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(200);
        wave::<Cell88>(&block, 200, &mut claims);
        assert!(!block.is_empty(), "test setup: the wave grew chunks");

        free(&block);

        assert!(block.is_empty(), "the chunk count is reset");
        assert!(
            block.cur.get().is_null(),
            "the bump cursor is reset, so the next bump cannot hand out freed bytes"
        );
        assert!(
            block.end.get().is_null(),
            "the chunk end is reset, so `end - cur` is 0 on the lazy block"
        );
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn a_second_free_all_frees_nothing() {
        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(200);
        wave::<Cell88>(&block, 200, &mut claims);
        free(&block);

        let log = {
            let rec = Recording::start();
            free(&block);
            rec.stop()
        };

        assert!(
            log.is_empty(),
            "the reset at the end of free_all makes a second call a no-op: {log:?}"
        );
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn a_block_that_emplaced_nothing_never_calls_the_allocator() {
        let log = {
            let rec = Recording::start();
            let block = ScopeBlock::new();
            assert!(block.is_empty(), "a fresh block owns nothing");
            free(&block);
            rec.stop()
        };

        assert!(
            log.is_empty(),
            "a scope that spawns no task must make no allocator call at all: {log:?}"
        );
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn emplace_after_free_all_restarts_the_growth_curve_at_chunk_zero() {
        const N: usize = 200;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(2 * N);

        let first = {
            let rec = Recording::start();
            wave::<Cell88>(&block, N, &mut claims);
            rec.stop()
        };
        free(&block);

        let second = {
            let rec = Recording::start();
            wave::<Cell88>(&block, N, &mut claims);
            rec.stop()
        };

        let caps_first: Vec<usize> = acquisitions(&first).iter().map(|e| e.size).collect();
        let caps_second: Vec<usize> = acquisitions(&second).iter().map(|e| e.size).collect();

        // Asserted BEFORE the second `free_all`, and the ordering is not cosmetic: the
        // mutation this test exists to catch is a `free_all` that fails to reset
        // `n_chunks`, and a block in that state DOUBLE-FREES its first wave's chunks
        // here. Freeing first would crash the binary in place of reporting which
        // assertion failed.
        assert_eq!(
            caps_first,
            model_chunk_caps(88, 8, N),
            "the first wave follows the model"
        );
        assert_eq!(
            caps_second, caps_first,
            "free_all RESETS the doubling curve — `e = max(min_e, 0)` again, so a chunk-count \
             model carried cumulatively across a free_all would be wrong"
        );

        free(&block);
    }

    // =====================================================================
    // Edge cases the design names
    // =====================================================================

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn a_zero_sized_emplace_allocates_no_chunk() {
        let block = ScopeBlock::new();
        let log = {
            let rec = Recording::start();
            for i in 0..64u64 {
                let _ = block.emplace(Zst::make(i)).erase();
            }
            rec.stop()
        };

        assert!(
            log.is_empty(),
            "a zero-sized value costs no chunk byte: {log:?}"
        );
        assert!(
            block.is_empty(),
            "`is_empty` is a chunk count, not a spawn count"
        );

        free(&block);
    }

    #[test]
    fn zero_sized_emplaces_share_one_address_and_still_satisfy_disjointness() {
        let block = ScopeBlock::new();

        let a = block.emplace(Zst).erase().addr();
        let b = block.emplace(Zst).erase().addr();

        assert_eq!(
            a, b,
            "every ZST comes back at `NonNull::dangling()` — the branch exists BECAUSE the \
             general path would have returned a lazy block's null `cur`"
        );
        assert_eq!(
            a,
            align_of::<Zst>(),
            "the dangling address is the type's alignment, and is never null"
        );
        assert_pairwise_disjoint(&[
            Claim {
                addr: a,
                size: 0,
                align: 1,
            },
            Claim {
                addr: b,
                size: 0,
                align: 1,
            },
        ]);

        free(&block);
    }

    #[test]
    fn a_zero_sized_emplace_does_not_advance_the_bump_cursor() {
        let block = ScopeBlock::new();
        let _ = block.emplace(Word::make(0)).erase();
        let cursor_before = block.cur.get();

        let z = block.emplace(Zst).erase().addr();

        assert_eq!(
            block.cur.get(),
            cursor_before,
            "the ZST path returns before `bump`, so it consumes no chunk byte"
        );
        assert_eq!(
            z,
            align_of::<Zst>(),
            "and it still answers the dangling address, not a slot in the live chunk"
        );

        free(&block);
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn an_over_aligned_type_is_placed_aligned_and_inside_its_chunk() {
        // `align_of::<T>() == 128 > CHUNK_ALIGN`. The chunk COUNT is not
        // modelled here (see `model_chunk_caps`'s domain): the leading pad
        // depends on `base % 128`, which the system allocator chooses.
        const N: usize = 40;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(N);
        let log = {
            let rec = Recording::start();
            wave::<Over128>(&block, N, &mut claims);
            rec.stop()
        };
        let chunks = acquisitions(&log);
        free(&block);

        assert!(
            chunks.len() >= 2,
            "40 values of 256 bytes cannot fit in one 4096-byte chunk"
        );
        assert_in_bounds_and_aligned(&claims, &chunks);
        assert_pairwise_disjoint(&claims);

        // What `required = size + align.saturating_sub(CHUNK_ALIGN)` buys: a
        // chunk's leading pad is at most `align - CHUNK_ALIGN`, so a chunk grown
        // for `required` always admits the value that asked for it.
        for c in &chunks {
            let first = claims
                .iter()
                .filter(|k| k.addr >= c.addr && k.addr < c.addr + c.size)
                .map(|k| k.addr)
                .min()
                .expect("every chunk holds at least the value that grew it");
            assert!(
                first - c.addr <= 128 - CHUNK_ALIGN,
                "the leading pad of chunk {c:?} is {} bytes, above the `align - CHUNK_ALIGN` \
                 bound the growth rule is stated over",
                first - c.addr
            );
        }
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn a_type_larger_than_chunk0_gets_a_chunk_that_holds_it() {
        // `size_of::<Huge8200>() == 8200 > CHUNK0`, so `min_e` — not the
        // doubling term — is what sizes the first chunk.
        const N: usize = 3;

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(N);
        let log = {
            let rec = Recording::start();
            wave::<Huge8200>(&block, N, &mut claims);
            rec.stop()
        };
        let chunks = acquisitions(&log);
        free(&block);

        assert_eq!(
            chunks.iter().map(|e| e.size).collect::<Vec<_>>(),
            model_chunk_caps(8200, 8, N),
            "the first chunk is sized by `min_e`, not by CHUNK0"
        );
        for c in &chunks {
            assert!(
                c.size >= size_of::<Huge8200>(),
                "a chunk grown for a body larger than CHUNK0 must admit it: {c:?}"
            );
        }
        assert_in_bounds_and_aligned(&claims, &chunks);
    }

    // =====================================================================
    // R5 — the receipts' chunk bucket
    // =====================================================================

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn every_chunk_layout_satisfies_the_receipts_chunk_predicate() {
        // Stated over EVERY event the regions produced, not over the ones that
        // already match: filtering to the chunk class first would make a chunk
        // that had LEFT the bucket invisible, which is the exact shape of a
        // receipt that comes back green from an empty bucket.
        let mut log: Vec<Ev> = Vec::new();
        log.extend(events_of::<Byte>(200));
        log.extend(events_of::<Word>(200));
        log.extend(events_of::<Cell88>(400));
        log.extend(events_of::<Page4096>(20));
        log.extend(events_of::<Huge8200>(20));
        log.extend(events_of::<Over128>(200));

        assert!(
            log.len() >= 30,
            "the regions must have produced a real population of events, got {}",
            log.len()
        );
        for e in &log {
            assert!(
                is_chunk_class(*e),
                "every layout this allocator emits must satisfy `align == 64 && \
                 size.is_power_of_two() && size >= 4096`, got {e:?}"
            );
        }
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn the_chunk_bucket_and_the_cell_bucket_cannot_hold_the_same_allocation() {
        // This is what makes a later stage's "zero allocations in the cell size
        // class" non-vacuous: if a cell allocation could satisfy the chunk
        // predicate, a receipt counting one class would be counting members of
        // the other too.
        //
        // The two buckets are disjoint on ALIGNMENT ALONE, which is why the
        // alignment half of the cell pins is load-bearing: with only the sizes
        // pinned, a cell class is free to move to another alignment, and a
        // receipt over the old bucket then reports green from an EMPTY bucket
        // rather than from a clean run.
        assert_ne!(
            crate::__layout_receipt::SCOPED_CELL_ALIGN,
            crate::__layout_receipt::CHUNK_ALIGN,
            "a scoped cell and a chunk must never land in the same (size, align) bucket"
        );
        assert_ne!(
            crate::__layout_receipt::DETACHED_CELL_ALIGN,
            crate::__layout_receipt::CHUNK_ALIGN,
            "a detached cell and a chunk must never land in the same (size, align) bucket"
        );

        // And the same statement over allocations that really happened.
        let cell_log = {
            let rec = Recording::start();
            let task = crate::task::Task::new_detached(|| {});
            drop(task);
            rec.stop()
        };
        let cell_acq = acquisitions(&cell_log);
        assert_eq!(cell_acq.len(), 1, "one cell, one acquisition: {cell_log:?}");
        assert_eq!(
            cell_acq[0].align,
            crate::__layout_receipt::DETACHED_CELL_ALIGN,
            "the cell class's alignment is the one the receipt pins"
        );
        assert!(
            !is_chunk_class(cell_acq[0]),
            "a cell allocation must not satisfy the chunk predicate: {:?}",
            cell_acq[0]
        );

        let block = ScopeBlock::new();
        let mut claims = Vec::with_capacity(1);
        let chunk_log = {
            let rec = Recording::start();
            wave::<Cell88>(&block, 1, &mut claims);
            rec.stop()
        };
        free(&block);
        let chunk_acq = acquisitions(&chunk_log);
        assert_eq!(chunk_acq.len(), 1, "one chunk: {chunk_log:?}");
        assert!(
            is_chunk_class(chunk_acq[0]),
            "a chunk allocation must satisfy the chunk predicate: {:?}",
            chunk_acq[0]
        );
    }

    // =====================================================================
    // MAX_CHUNKS exhaustion
    // =====================================================================

    /// Set in the CHILD half of [`chunk_table_exhaustion_aborts_the_process`].
    const EXHAUSTION_CHILD_ENV: &str = "BOYKO_BLOCK_EXHAUSTION_CHILD";

    /// The child is launched by name, so the name is a constant.
    const EXHAUSTION_TEST_NAME: &str = "block::tests::chunk_table_exhaustion_aborts_the_process";

    #[cfg_attr(miri, ignore = "instrument: re-execs this test binary as a child process, which Miri does not support. Runs natively.")]
    #[test]
    fn chunk_table_exhaustion_aborts_the_process() {
        if std::env::var_os(EXHAUSTION_CHILD_ENV).is_some() {
            // ── CHILD ──
            //
            // The table is driven to `MAX_CHUNKS` by hand rather than by growing
            // 32 chunks, and the reason is the module's own claim: the 32nd
            // chunk alone is 8 TiB, so `alloc` fails long before the table
            // fills. Reaching this branch through growth is not possible on any
            // machine, which is exactly why the branch is DEFENCE. `grow` reads
            // `n_chunks` and nothing else before the check, so the rest of the
            // block being lazy is invisible to the path under test.
            //
            // The exhausting call is wrapped in `catch_unwind` because the property
            // is that exhaustion ABORTS, and an abort has to be distinguishable from
            // a panic: both leave the child dead and both skip `CHILD-RETURNED`, so
            // without this the test would accept the `panic!` the module
            // deliberately did NOT write (a panic here is catchable on the task
            // path, which is the whole reason the site aborts).
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let block = ScopeBlock::new();
                block.n_chunks.set(MAX_CHUNKS as u32);

                println!("CHILD-REACHED");
                std::io::stdout().flush().expect("test setup: flush");

                let _ = block.emplace(Word::make(1)).erase();
            }));
            match outcome {
                Ok(()) => println!("CHILD-RETURNED"),
                Err(_) => println!("CHILD-CAUGHT-PANIC"),
            }
            std::io::stdout().flush().expect("test setup: flush");
            return;
        }

        // ── PARENT ──
        let exe = std::env::current_exe().expect("test setup: this test binary's own path");
        let out = Command::new(exe)
            .args(["--exact", EXHAUSTION_TEST_NAME, "--nocapture"])
            .args(["--test-threads", "1"])
            .env(EXHAUSTION_CHILD_ENV, "1")
            .output()
            .expect("test setup: re-exec of this test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);

        assert!(
            stdout.contains("running 1 test"),
            "the child must have RUN the exhaustion test rather than filtering it away — \
             stdout:\n{stdout}"
        );
        assert!(
            stdout.contains("CHILD-REACHED"),
            "the child must have reached the emplace that exhausts the table — stdout:\n{stdout}"
        );
        assert!(
            !stdout.contains("CHILD-RETURNED"),
            "`chunk_table_exhausted` must not return — stdout:\n{stdout}"
        );
        assert!(
            !stdout.contains("CHILD-CAUGHT-PANIC"),
            "exhaustion must ABORT, not panic: a panic is catchable on the task path, \n             and a bump allocator that has silently lost its table must not be \n             recovered from — stdout:
{stdout}"
        );
        assert!(
            !out.status.success(),
            "the child must have died abnormally, got {:?}",
            out.status
        );
    }

    // =====================================================================
    // The randomised property
    // =====================================================================

    /// How many shapes [`emplace_kind`] can produce.
    const KIND_COUNT: usize = 6;

    /// Emplace one value of the shape `kind` names.
    ///
    /// The menu spans the axes the properties turn on: a ZST, alignments
    /// 1 / 8 / 128 (one ABOVE `CHUNK_ALIGN`), and sizes on both sides of
    /// `CHUNK0`.
    fn emplace_kind(block: &ScopeBlock, kind: usize, i: u64) -> Claim {
        match kind {
            0 => claim::<Zst>(block, i),
            1 => claim::<Byte>(block, i),
            2 => claim::<Word>(block, i),
            3 => claim::<Cell88>(block, i),
            4 => claim::<Over128>(block, i),
            5 => claim::<Huge8200>(block, i),
            _ => unreachable!("kind is taken modulo KIND_COUNT"),
        }
    }

    /// xorshift64. Fixed seed on purpose: a failure here is replayable from the
    /// seed and the case index, which a randomly seeded generator would not be.
    fn next(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    #[cfg_attr(miri, ignore = "instrument: reads the recording allocator's tape, and that allocator is `cfg(not(miri))` (see the `#[global_allocator]` in this module) because delegating to std's Windows `System` reds the crate's own Miri gate from inside `HeapFree`. `Recording::start` panics under Miri rather than returning an empty log, so this attribute cannot silently rot into a vacuous pass. Runs natively.")]
    #[test]
    fn random_mixed_waves_keep_every_claim_in_bounds_aligned_and_pairwise_disjoint() {
        const CASES: usize = 256;
        const MAX_WAVE: usize = 96;

        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut claims: Vec<Claim> = Vec::with_capacity(MAX_WAVE);
        let mut kinds: Vec<usize> = Vec::with_capacity(MAX_WAVE);
        let mut kinds_seen = [0usize; KIND_COUNT];
        let mut waves_that_grew = 0usize;
        let mut empty_waves = 0usize;

        for case in 0..CASES {
            let n = (next(&mut state) as usize) % (MAX_WAVE + 1);
            kinds.clear();
            for _ in 0..n {
                kinds.push((next(&mut state) % KIND_COUNT as u64) as usize);
            }
            claims.clear();

            let block = ScopeBlock::new();
            let log = {
                let rec = Recording::start();
                for (i, kind) in kinds.iter().enumerate() {
                    claims.push(emplace_kind(&block, *kind, i as u64));
                }
                rec.stop()
            };
            let chunks = acquisitions(&log);

            assert!(
                releases(&log).is_empty(),
                "case {case}: a wave frees nothing — chunks are appended: {log:?}"
            );
            assert_in_bounds_and_aligned(&claims, &chunks);
            assert_pairwise_disjoint(&claims);

            let teardown = {
                let rec = Recording::start();
                free(&block);
                rec.stop()
            };
            let freed = releases(&teardown);
            assert_eq!(
                freed.len(),
                chunks.len(),
                "case {case}: free_all releases exactly the chunks the wave acquired"
            );
            for c in &chunks {
                assert_eq!(
                    freed.iter().filter(|r| r.addr == c.addr).count(),
                    1,
                    "case {case}: the chunk at {:#x} must be released exactly once",
                    c.addr
                );
            }

            for kind in &kinds {
                kinds_seen[*kind] += 1;
            }
            if chunks.len() > 1 {
                waves_that_grew += 1;
            }
            if n == 0 {
                empty_waves += 1;
            }
        }

        // The GENERATOR'S OWN anti-vacuity guard. A property that ran 256 cases
        // over inputs that never included a ZST, never grew a second chunk and
        // never hit the empty wave is a property that did not test what its name
        // says.
        for (kind, seen) in kinds_seen.iter().enumerate() {
            assert!(
                *seen > 0,
                "the generator never produced shape {kind}: {kinds_seen:?}"
            );
        }
        assert!(
            waves_that_grew > 0,
            "no generated wave ever grew a second chunk, so the growth path went untested"
        );
        assert!(
            empty_waves > 0,
            "the generator never produced the n == 0 wave"
        );
    }
}
