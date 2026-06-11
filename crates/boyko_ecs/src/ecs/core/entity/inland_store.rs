//! Address-stable growable store for [`EntityInland`] records (Phase X.G).
//!
//! Replaces the former `Vec<EntityInland>` inside `EntityMaster`. The buffer
//! is ONE contiguous virtual-address reservation ([`VmReservation`],
//! `DEFAULT_INLAND_RESERVE` = 1 GiB on 64-bit syscall arms) committed lazily
//! in geometric frontier slabs — growth is **O(1) in live entities: one
//! commit syscall, zero bytes copied, zero bytes written**. The base never
//! moves, so the deterministic realloc-memcpy spikes of the Vec doubling
//! chain (g7b sub-batches #285/#580, `PHASE-XF-RESULTS.md` §B6) are deleted,
//! and `EntityMaster`'s SEND5 "no mid-flight realloc" clause becomes
//! structural.
//!
//! # Invariant J (the zero-tail induction, plan R2-C1)
//!
//! **At every program point, every slot in `[len, committed_slots)` reads
//! all-zero.** Maintenance:
//! - [`ensure`](InlandStore::ensure) grows `len` into a region J guarantees
//!   is zero (and writes nothing);
//! - explicit writes land only at indices `< len` — structurally enforced:
//!   every write path is slice `Index`/`IndexMut`/`get_mut` through
//!   [`DerefMut`], which panic/`None` past `len`; `rewind_allocate` does NOT
//!   truncate `len` (it only rolls back the atomic id counter); `len` shrinks
//!   ONLY at [`clear`](InlandStore::clear);
//! - `clear` memsets `[0, len)` then sets `len = 0`, making `[0, committed)`
//!   uniformly zero.
//!
//! I-Z(b) — "a never-explicitly-written slot reads [`EntityInland::NULL`]" —
//! is a corollary of J because `NULL` is all-zero 16 bytes with no padding
//! (`repr(C)` 8+4+4; const-asserted in `entity_inland.rs`; value bytes pinned
//! by the U-S1 transmute test). Fresh-page zero sources per the
//! [`vm`](crate::ecs::memory::vm) module's zero-fill contract.
//!
//! # Why the recycled-slot caveat forbids partial re-zeroing
//!
//! Written-dead slots are `{null ptr, 0, generation+1}` — NOT all-zero — so
//! re-zeroing any sub-range of `[0, len)` outside `clear` would silently
//! reset generations (entity aliasing). No decommit/`MEM_RESET` of the live
//! range, ever.

use std::ops::{Deref, DerefMut};

use crate::ecs::constants::{
    COMMIT_GRANULE, DEFAULT_INLAND_RESERVE, INLAND_MAX_SLAB, INLAND_MIN_SLAB,
};
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::memory::vm::VmReservation;

/// One `EntityInland` record = 16 bytes (const-asserted in
/// `entity_inland.rs`; re-pinned here because the slot/byte conversions in
/// `grow_to` rely on it).
const SLOT_SIZE: usize = size_of::<EntityInland>();
// The exact-16 pin is a 64-bit fact (`EntityInland` carries a pointer-sized
// field); on 32-bit targets (wasm32) the slot is smaller and only the
// granule-divisibility invariant is load-bearing — the same
// `target_pointer_width = "64"` gating as the 28 pointer-width asserts from
// the original wasm port.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(SLOT_SIZE == 16, "EntityInland slot size drifted");
// Granule divisibility is load-bearing ONLY where a commit boundary exists
// (the 64-bit syscall arms: a PROT_NONE frontier must not land mid-slot). On
// 32-bit targets (wasm32) the slot is 12 B and does not divide the granule —
// harmless: the only supported 32-bit arm is the fallback (one eager
// `alloc_zeroed`, `commit` is a no-op), so no protection boundary exists and
// the frontier bookkeeping floors safely. 32-bit *syscall* targets are out of
// the engine's target scope (CLAUDE.md: x86_64).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    COMMIT_GRANULE.is_multiple_of(SLOT_SIZE),
    "granule must be a whole number of slots"
);

/// Address-stable sparse store of [`EntityInland`] records, indexed by
/// `EntityId.0`.
///
/// `#[repr(C)]` (plan R2-O1): the field order is part of the X.G asm-gate
/// displacement story — the hot pair (`vm.base`, `len`) sits inside one cache
/// line deterministically.
///
/// NOT `Send`/`Sync` (the `NonNull` inside `VmReservation`): `EntityMaster`'s
/// existing `unsafe impl Send/Sync` (SEND5) carries the exclusivity argument,
/// exactly as it did for the old `Vec<EntityInland>` (whose `*mut Archetype`
/// elements already suppressed the auto-impls).
#[repr(C)]
pub(crate) struct InlandStore {
    /// Cached base of the reservation, hot-path twin of `vm`'s base.
    /// **Dangling until the first `grow_to`** (XG-B4: the reservation
    /// syscall is deferred off `EcsMaster::new`) — sound because the hot
    /// `Deref` is `from_raw_parts(base, len)` and `len == 0` until the
    /// first grow: a dangling-but-aligned `NonNull` with length 0 is
    /// explicitly valid for `from_raw_parts`, and the `len` bounds check
    /// fails before any byte behind `base` could be touched. Write-once at
    /// materialization — every slot address is stable for the store's
    /// lifetime thereafter (XG-B6 witness).
    base: std::ptr::NonNull<EntityInland>,
    /// Live slot count — THE load-bearing scalar (research R-2): the bounds
    /// oracle for `get`/`Deref`, `EntityMaster::capacity()`, `iter_entities`,
    /// and `rewind_allocate`. Mutated only under `&mut self` (SEND5/SCH7).
    len: usize,
    /// Commit frontier in SLOTS (== committed_bytes / 16): the warm-path
    /// comparator in `ensure` needs no multiplication, and the `n * 16`
    /// overflow class is confined to the cold path's checked math.
    committed_slots: usize,
    /// Reservation size to materialize lazily (bytes, pre-granule-rounding).
    reserve_request: usize,
    /// The reservation itself; `None` until the first `grow_to`
    /// (NOT read on any hot path — `base`/`len` above are the hot pair).
    vm: Option<VmReservation>,
}

impl InlandStore {
    /// Default store: a LAZY [`DEFAULT_INLAND_RESERVE`] reservation
    /// (materialized by the first `grow_to` — XG-B4: `EcsMaster::new` pays
    /// no reservation syscall), commit 0, len 0.
    pub(crate) fn new() -> Self {
        Self {
            base: std::ptr::NonNull::dangling(),
            len: 0,
            committed_slots: 0,
            reserve_request: DEFAULT_INLAND_RESERVE,
            vm: None,
        }
    }

    /// `Vec::with_capacity` analog: precommits room for `slots` records
    /// (len stays 0), so the first `slots` entities trigger zero growth
    /// events. Materializes the reservation eagerly (the caller asked for
    /// committed capacity).
    ///
    /// Over-ceiling semantics (plan R2-W2, option b): the reservation itself
    /// is sized as `max(DEFAULT_INLAND_RESERVE, bytes(slots))` — a capacity
    /// request the address space can satisfy is NEVER refused (preserving
    /// `Vec::with_capacity`'s contract); there is no silent clamp.
    pub(crate) fn with_capacity(slots: usize) -> Self {
        let bytes = slots
            .checked_mul(SLOT_SIZE)
            .expect("InlandStore::with_capacity: slot count overflows bytes");
        let mut store = Self::new();
        store.reserve_request = DEFAULT_INLAND_RESERVE.max(bytes.max(1));
        if slots > 0 {
            store.grow_to(slots);
        }
        store
    }

    /// Test knob: a store with an explicit (small) reservation, for
    /// exhaustion-panic and Miri small-world tests. Lazy like `new`.
    #[cfg(test)]
    pub(crate) fn with_reserve_bytes(bytes: usize) -> Self {
        let mut store = Self::new();
        store.reserve_request = bytes;
        store
    }

    /// Grows the live range to AT LEAST `n` slots: commit-to-cover (cold,
    /// rare) + `len = max(len, n)`. **Writes nothing** — newly exposed slots
    /// read [`EntityInland::NULL`] by invariant J (module doc).
    ///
    /// Replaces every former `Vec::resize(n, NULL)` site; unlike `resize`
    /// this never copies the prefix and never fills the tail. The API
    /// deliberately cannot express a reallocation or a copy.
    #[inline]
    pub(crate) fn ensure(&mut self, n: usize) {
        if n <= self.len {
            return;
        }
        if n > self.committed_slots {
            self.grow_to(n);
        }
        self.len = n;
        debug_assert!(self.len <= self.committed_slots);
    }

    /// Cold frontier growth: commit enough slabs to cover `n` slots.
    ///
    /// Granule chain (plan R2-O3, verbatim): `os_len` is a granule multiple ⇒
    /// `align_up(x, G) ≤ os_len ⟺ x ≤ os_len`; `64 KiB | os_len` and
    /// `16 | 64 KiB` ⇒ `os_len / 16` is exact; the ceiling check BEFORE
    /// computing `needed` therefore closes the granule-slack trap (a request
    /// within granule slack of the ceiling cannot round past it). The
    /// post-condition `new_bytes ≥ needed` is a proof, not a check:
    /// `step ≥ needed − old_bytes` by the `max`, and the `min(os_len)` clamp
    /// cannot bite below `needed` because `needed ≤ os_len`.
    #[cold]
    #[inline(never)]
    fn grow_to(&mut self, n: usize) {
        // Lazy materialization (XG-B4): the reservation syscall is deferred
        // from construction to the first growth event — strictly off the
        // warm path (this fn is already #[cold]).
        let vm = match &self.vm {
            Some(vm) => vm,
            None => {
                let vm = VmReservation::reserve(self.reserve_request);
                self.base = vm.base().cast();
                self.vm.insert(vm)
            }
        };

        let ceiling_slots = vm.os_len() / SLOT_SIZE; // exact: 16 | granule | os_len
        assert!(
            n <= ceiling_slots,
            "InlandStore exhausted: {n} entity slots requested, reservation ceiling is \
             {ceiling_slots} (grow the reserve at construction)"
        );

        let old_bytes = self.committed_slots * SLOT_SIZE;
        let needed = checked_slab_round(n * SLOT_SIZE); // n*16 can't overflow: n ≤ ceiling ≤ os_len/16
        // Geometric doubling clamped to [MIN, MAX], request-dominant (a
        // single huge request is a single event), never past the reservation.
        let step = old_bytes
            .clamp(INLAND_MIN_SLAB, INLAND_MAX_SLAB)
            .max(needed - old_bytes);
        let new_bytes = (old_bytes + step).min(vm.os_len());
        debug_assert!(new_bytes >= needed, "grow_to post-condition (proof) violated");

        vm.commit(old_bytes, new_bytes);
        self.committed_slots = new_bytes / SLOT_SIZE;
    }

    /// Zeroes the ever-written range and resets `len` — the world-reset API.
    ///
    /// The memset is REQUIRED, not an optimization (plan D5): without it, a
    /// later `ensure` past a post-clear `len` would re-expose stale records
    /// (dangling `archetype_ptr`s, stale generations) — the
    /// entity-aliasing/use-after-free class. Zeroing `[0, len)` restores
    /// invariant J for `[0, committed)` (see the module doc's induction —
    /// bands written before an EARLIER clear are already zero by that clear).
    pub(crate) fn clear(&mut self) {
        if self.len > 0 {
            // SAFETY (S-CLEAR): `len > 0` implies the reservation is
            // materialized (`len` only grows via `ensure` → `grow_to`);
            // `[0, len * 16)` lies inside the committed RW range
            // (`len ≤ committed_slots`, debug-asserted everywhere the pair
            // changes); u8-level zeroing of plain-old-data `EntityInland`
            // records; restores invariant J(b) for the whole ever-written
            // range.
            unsafe {
                std::ptr::write_bytes(self.base.as_ptr().cast::<u8>(), 0, self.len * SLOT_SIZE);
            }
        }
        self.len = 0;
    }

    /// Commit frontier in slots (diagnostics/tests — mirror of
    /// `ComponentPool::committed_rows`).
    #[inline]
    pub(crate) fn committed_slots(&self) -> usize {
        self.committed_slots
    }
}

/// Cold-path granule rounding with overflow check (R2-N2 class). Private twin
/// of `vm::checked_align_up` specialized to the commit granule.
fn checked_slab_round(bytes: usize) -> usize {
    bytes
        .checked_add(COMMIT_GRANULE - 1)
        .expect("InlandStore: slab rounding overflow")
        & !(COMMIT_GRANULE - 1)
}

impl Deref for InlandStore {
    type Target = [EntityInland];

    /// The Phase-7 hot path flows through here: `store.get(i)` must compile
    /// to exactly `Vec::get`'s sequence — load base, load len, cmp, indexed
    /// 16-B load (asm gate XG-B1).
    #[inline]
    fn deref(&self) -> &[EntityInland] {
        // SAFETY (S-SLICE): before materialization `base` is
        // `NonNull::dangling()` AND `len == 0` — `from_raw_parts(dangling,
        // 0)` is explicitly valid. After materialization `base` is non-null,
        // 8-aligned on every arm (page-aligned on syscall arms; the fallback
        // Layout aligns to ≥ 4096 ≥ 8); its provenance spans the whole
        // reservation (single allocated object); `len * 16 ≤ committed_bytes
        // ≤ os_len ≤ isize::MAX`; and every byte of `[0, len)` is
        // initialized — explicitly written slots are class (a),
        // never-written slots read zero by invariant J, and OS-zeroed pages
        // count as initialized memory by the vm module's
        // alloc_zeroed-equivalence contract (plan R2-W1). No reference
        // escapes the call: callers' borrows die with their statement
        // (research R-2: zero interior pointers anywhere).
        unsafe { std::slice::from_raw_parts(self.base.as_ptr(), self.len) }
    }
}

impl DerefMut for InlandStore {
    #[inline]
    fn deref_mut(&mut self) -> &mut [EntityInland] {
        // SAFETY (S-SLICE): as `deref`, plus exclusivity: `&mut self`
        // guarantees no other slice over the buffer exists (the store is
        // reachable only through `EntityMaster`, whose mutation discipline is
        // SEND5/SCH7 dispatcher-exclusive).
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::constants::INLAND_MIN_SLAB;

    const G: usize = COMMIT_GRANULE;
    const MIN_SLOTS: usize = INLAND_MIN_SLAB / SLOT_SIZE; // 16,384

    fn non_null_record(generation: u32) -> EntityInland {
        EntityInland::dangling_for_test(7, generation)
    }

    /// U-S1 — the I-Z keystone: `NULL` is exactly 16 zero bytes (no padding,
    /// no non-zero niche), so kernel-zero pages ARE valid NULL records.
    #[test]
    fn null_is_all_zero_bytes() {
        // SAFETY: EntityInland is repr(C) POD of size 16 (const-asserted);
        // transmuting to a byte array reads all value bytes.
        let bytes: [u8; 16] = unsafe { std::mem::transmute(EntityInland::NULL) };
        assert_eq!(bytes, [0u8; 16], "EntityInland::NULL must be all-zero bytes");
    }

    /// U-S2 — XG-B6 witness: addresses are stable and written values survive
    /// growth across ≥ 3 slab boundaries (impossible with Vec).
    #[test]
    fn addresses_stable_across_multi_slab_growth() {
        let mut s = InlandStore::new();
        s.ensure(10);
        s[3] = non_null_record(42);
        let addr0 = &s[0] as *const EntityInland;
        let addr3 = &s[3] as *const EntityInland;

        // Grow across at least 3 slab boundaries (256K -> 512K -> 1M bytes).
        s.ensure(MIN_SLOTS + 1); // past slab 1
        s.ensure(3 * MIN_SLOTS); // past slab 2
        s.ensure(7 * MIN_SLOTS); // past slab 3
        assert!(s.committed_slots() >= 7 * MIN_SLOTS);

        assert_eq!(addr0, &s[0] as *const EntityInland, "slot 0 address moved");
        assert_eq!(addr3, &s[3] as *const EntityInland, "slot 3 address moved");
        assert_eq!(s[3].generation(), 42, "written value lost across growth");
    }

    /// U-S3 — never-written tail reads NULL after multi-slab ensure (sampled
    /// at slab boundaries ± 1).
    #[test]
    fn never_written_tail_reads_null() {
        let mut s = InlandStore::new();
        s.ensure(4 * MIN_SLOTS);
        for idx in [
            0,
            1,
            MIN_SLOTS - 1,
            MIN_SLOTS,
            MIN_SLOTS + 1,
            2 * MIN_SLOTS - 1,
            2 * MIN_SLOTS,
            4 * MIN_SLOTS - 1,
        ] {
            assert!(s[idx].is_null(), "slot {idx} must read NULL");
            assert_eq!(s[idx].generation(), 0, "slot {idx} generation must be 0");
        }
    }

    /// U-S4 (R2-C1, TWO-clear cycle) — the stale-bytes regression net: a
    /// second, smaller live range followed by clear + regrow past the
    /// ORIGINAL high-water must read NULL everywhere — including the band
    /// `[small_len, old_highwater)` that only the FIRST clear zeroed.
    #[test]
    fn two_clear_cycles_leave_no_stale_bytes() {
        let mut s = InlandStore::new();

        // Cycle 1: write across two slabs, then clear.
        let hw1 = MIN_SLOTS + 100;
        s.ensure(hw1);
        for i in 0..hw1 {
            s[i] = non_null_record(7);
        }
        s.clear();

        // Cycle 2: smaller live range, write, clear again (memsets only
        // [0, small)).
        let small = 50;
        s.ensure(small);
        for i in 0..small {
            s[i] = non_null_record(9);
        }
        s.clear();

        // Regrow past the original high-water: every slot must be NULL —
        // [0, small) by clear #2, [small, hw1) by clear #1 + invariant J,
        // [hw1, ..) never written.
        s.ensure(hw1 + 500);
        for idx in [0, small - 1, small, small + 1, hw1 - 1, hw1, hw1 + 499] {
            assert!(s[idx].is_null(), "stale bytes at slot {idx} after two clear cycles");
            assert_eq!(s[idx].generation(), 0, "stale generation at slot {idx}");
        }
    }

    /// U-S5 — grow-policy table (mirror of X.F U1): first event is
    /// request-dominant vs MIN_SLAB; then doubling; MAX clamp; ceiling clamp;
    /// granule alignment of every frontier.
    #[test]
    fn grow_policy_table() {
        let mut s = InlandStore::new();

        // First event: small request -> MIN_SLAB.
        s.ensure(10);
        assert_eq!(s.committed_slots(), MIN_SLOTS, "first event commits MIN_SLAB");

        // Doubling: next event commits ~old again (256K -> +256K = 512K).
        s.ensure(MIN_SLOTS + 1);
        assert_eq!(s.committed_slots(), 2 * MIN_SLOTS, "second event doubles");

        // Request-dominant: a huge single request is a single event.
        let huge = 40 * MIN_SLOTS;
        s.ensure(huge);
        assert!(s.committed_slots() >= huge, "huge request covered in one event");
        assert!(
            (s.committed_slots() * SLOT_SIZE).is_multiple_of(G),
            "frontier must stay granule-aligned"
        );

        // MAX clamp on the doubling term (not on the request term): grow a
        // fresh store to MAX bytes, the next doubling step is MAX, not 2*MAX.
        // Explicit reserve: the Miri/wasm fallback DEFAULT ceiling is exactly
        // MAX_SLAB worth of slots, so the default store would exhaust here.
        let mut s2 = InlandStore::with_reserve_bytes(4 * INLAND_MAX_SLAB);
        let max_slots = INLAND_MAX_SLAB / SLOT_SIZE;
        s2.ensure(max_slots); // commits exactly MAX (request-dominant first event)
        assert_eq!(s2.committed_slots(), max_slots);
        s2.ensure(max_slots + 1); // doubling term clamps to MAX
        assert_eq!(s2.committed_slots(), 2 * max_slots, "step clamped to MAX_SLAB");
    }

    /// U-S6 — exhaustion: a request past the reservation ceiling panics
    /// loudly naming the ceiling, with NO state change.
    #[test]
    fn exhaustion_panics_without_state_change() {
        let mut s = InlandStore::with_reserve_bytes(256 * 1024); // 16,384 slots
        s.ensure(100);
        let (len_before, committed_before) = (s.len(), s.committed_slots());

        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.ensure(20_000); // > 16,384 ceiling
        }));
        assert!(r.is_err(), "over-ceiling ensure must panic");
        assert_eq!(s.len(), len_before, "len changed by failed grow");
        assert_eq!(s.committed_slots(), committed_before, "frontier changed by failed grow");
    }

    /// U-S7 — `with_capacity` precommits: no growth events during the first
    /// `c` ensures.
    #[test]
    fn with_capacity_precommits() {
        let c = 3 * MIN_SLOTS;
        let mut s = InlandStore::with_capacity(c);
        assert!(s.committed_slots() >= c, "with_capacity must precommit");
        assert_eq!(s.len(), 0, "with_capacity must not extend len");
        let committed = s.committed_slots();
        s.ensure(c); // must be covered by the precommit
        assert_eq!(s.committed_slots(), committed, "ensure within precommit grew");
    }

    /// U-S8 — `clear` keeps the commit frontier, resets len.
    #[test]
    fn clear_keeps_commit() {
        let mut s = InlandStore::new();
        s.ensure(MIN_SLOTS + 5);
        let committed = s.committed_slots();
        s.clear();
        assert_eq!(s.len(), 0);
        assert_eq!(s.committed_slots(), committed, "clear must not decommit");
    }

    /// U-S9 (R2-W2) — `with_capacity` ABOVE the default ceiling succeeds via
    /// reservation sizing (option b): never refuses a satisfiable request.
    #[test]
    fn with_capacity_above_default_ceiling() {
        let default_ceiling = DEFAULT_INLAND_RESERVE / SLOT_SIZE;
        let c = default_ceiling + MIN_SLOTS;
        let mut s = InlandStore::with_capacity(c);
        assert!(s.committed_slots() >= c, "over-ceiling with_capacity must precommit c");
        s.ensure(c); // zero growth events; would panic if the reserve were clamped
    }

    /// P1 — model-based equivalence with the old `Vec<EntityInland>`
    /// semantics (resize-with-NULL): random ensure/write/clear/read sequences
    /// must agree slot-for-slot.
    #[test]
    #[cfg_attr(miri, ignore)] // 64 cases × slot-by-slot 40k-slot comparisons — hours under Miri
    fn model_equivalence_with_vec_resize_semantics() {
        use proptest::prelude::*;
        use proptest::test_runner::{Config, TestRunner};

        #[derive(Debug, Clone)]
        enum Op {
            Ensure(usize),
            Write(usize, u32),
            Clear,
        }

        let op = prop_oneof![
            (1usize..40_000).prop_map(Op::Ensure),
            ((0usize..40_000), (1u32..1000)).prop_map(|(i, g)| Op::Write(i, g)),
            Just(Op::Clear),
        ];

        let mut runner = TestRunner::new(Config { cases: 64, ..Config::default() });
        runner
            .run(&proptest::collection::vec(op, 1..40), |ops| {
                let mut store = InlandStore::with_reserve_bytes(1024 * 1024);
                let mut model: Vec<EntityInland> = Vec::new();
                for op in ops {
                    match op {
                        Op::Ensure(n) => {
                            if n > model.len() {
                                model.resize(n, EntityInland::NULL);
                            }
                            store.ensure(n);
                        }
                        Op::Write(i, g) => {
                            if i < model.len() {
                                model[i] = non_null_record(g);
                                store[i] = non_null_record(g);
                            }
                        }
                        Op::Clear => {
                            model.clear();
                            store.clear();
                        }
                    }
                    prop_assert_eq!(store.len(), model.len());
                    for i in 0..model.len() {
                        prop_assert_eq!(store[i].is_null(), model[i].is_null());
                        prop_assert_eq!(store[i].generation(), model[i].generation());
                    }
                }
                Ok(())
            })
            .expect("InlandStore must match Vec resize-with-NULL semantics");
    }
}
