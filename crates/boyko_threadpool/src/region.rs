//! # The region — one pool scope, many barriered stages (SR)
//!
//! A *region* runs a whole table of stages inside ONE [`PoolInner::scope`]: the calling thread is
//! participant 0 (the orchestrator), and `participants − 1` helper tasks join it. Each schedule
//! item is either run inline on participant 0 (a one-block entry) or *published*: every
//! participant claims blocks of it, and the orchestrator waits until every block has run before it
//! publishes the next item. That replaces one `pool.scope` per stage with one scope per region —
//! the design record is `docs/physics/perf-campaign/levers/scaling/02-SR-DESIGN.md`.
//!
//! ## The protocol (the benched ω_b v2 protocol, refined)
//!
//! * **Epoch claims.** Item `i` of a region runs at epoch `g = base + 1 + i`. A block is claimed by
//!   `compare_exchange(hint, g)` on its claim word; on a failure with `w < g` (a word left at an
//!   older epoch) the claim retries ONCE with `w`, on `w ≥ g` the block is someone else's. Claim
//!   words are therefore never reset. `hint` is the entry's previous epoch in this region
//!   ([`SchedItem::prev_off`]); a wrong hint only costs that one retry.
//! * **The epoch range is reserved at entry** (the W8 fix). [`PoolInner::region`] advances the
//!   caller's epoch counter by `len + 1` BEFORE it writes anything, so every exit — END, poison, an
//!   unwind, a bound panic — leaves the counter past every epoch the region can have written, and
//!   the next region's first epoch is above every claim word ("every claim epoch ≤ base at open").
//! * **Tagged END and the open reset.** END is `END_BIT | base`. At open, participant 0 stores
//!   `publish = base` (OPEN) before any helper is spawned, so a helper of THIS region can never
//!   read the previous region's END; a helper that reads END debug-asserts the tag.
//! * **Exact completion.** Each counted block does `done.fetch_add(1, Release)` (or one batched
//!   add per participant, [`RegionPolicy::DONE_BATCHED`]); the orchestrator waits for EXACTLY
//!   `n_blocks` with `Acquire`, then resets the line to 0 before its next publish.
//! * **Poison on unwind.** Every participant holds a guard. A participant that unwinds stores its
//!   receipt, then `poison = 1`; participant 0's guard also publishes the tagged END. The
//!   orchestrator checks `poison` before every item and on every done-wait iteration, helpers
//!   before every claim. The scope's join re-raises the payload.
//! * **The open reset after a caught poisoned region.** A poisoned region can leave a done line
//!   non-zero. The next open sees `poison == 1` and clears EVERY done line of the column (not only
//!   the new table's entries: a later, larger table would otherwise inherit the stale count),
//!   then stores `poison = 0`.
//! * **Helpers spin** (the owner's value, 2026-09-29): a waiting helper PAUSEs and yields, it
//!   never sleeps or parks inside a region. The orchestrator's wait is pure PAUSE by default.
//!
//! ## Storage is the caller's (Principle 0)
//!
//! The frame — the sync line, the done lines, the receipts, the claim lines — is plain
//! [`RegionLine`]s the caller owns (the physics solver keeps them in its Resource's
//! `ScratchColumn`s). [`RegionFrame::new`] borrows them `&mut` for the region and projects them
//! as atomics; nothing is allocated here except the scope's own task cells.
//!
//! ## Every knob is a compile-time policy
//!
//! [`RegionPolicy`] carries the waiter ladders, the test-only wait bound and the three protocol
//! axes (R-a batched completion, R-b home claim lines, R-c TTAS) as associated consts, so every
//! branch on them folds after monomorphisation. The `ARMED` const parameter compiles the stall
//! census and the per-item hooks in; the disarmed [`V2Policy`] monomorph reads no clock at all.

use std::time::Instant;

use crate::sync::{AtomicU64, Ordering};
#[cfg(not(loom))]
use crate::thread_pool::{PoolInner, ThreadPool};

// =========================================================================
// Constants
// =========================================================================

/// The most blocks one published item may have, per participant (`n_blocks ≤ 8 · participants`,
/// debug-asserted at publish). One [`RegionPolicy::HOME_LINES`] claim line holds eight epochs,
/// which is where the 8 comes from.
pub const REGION_MAX_BLOCKS_PER_PARTICIPANT: u32 = 8;

/// The stall census threshold (armed regions only): a wait longer than this is counted once in its
/// participant's receipt. Independent of every ladder budget, so the census reads the same under
/// every policy.
pub const REGION_STALL_NS: u32 = 20_000;

/// The END flag of the publish word; the low bits carry the region's `base` (the tag).
const END_BIT: u64 = 1 << 63;

/// The smallest `base` a region takes (critique O1): a zero-initialised publish word or receipt
/// can then never read as this region's OPEN value or tag.
const EPOCH_FIRST: u64 = 1;

/// A PAUSE phase reads the clock once per this many iterations (when the monomorph reads a clock).
const CLOCK_EVERY: u32 = 64;

/// The sync line's words.
const S_PUBLISH: usize = 0;
const S_POISON: usize = 1;

/// A receipt line's words.
const R_TAG: usize = 0;
const R_BLOCKS: usize = 1;
const R_EXIT: usize = 2;
const R_STALLS: usize = 3;
const R_MAX_WAIT: usize = 4;
/// Words of a receipt line that carry data.
#[cfg(loom)]
const RECEIPT_WORDS: usize = 5;

// =========================================================================
// The caller-owned frame types
// =========================================================================

/// One 64-byte line of the region frame: eight plain `u64` words.
///
/// Plain data so the caller can keep frame lines in any POD column (a `ScratchColumn<T: Copy>`).
/// While a region is open the lines are accessed only as atomics, through the projection
/// [`RegionFrame::new`] takes from a `&mut` borrow of them.
///
/// Word layout by role:
/// * **sync line:** `w[0]` publish (OPEN = base, an epoch `g`, or `END_BIT | base`), `w[1]` poison;
/// * **done line:** `w[0]` the completion count of one entry (0 between items);
/// * **receipt line:** `w[0]` the region tag (`base`), `w[1]` blocks run, `w[2]` the
///   [`RegionExit`], `w[3]` stalls, `w[4]` the longest wait in ns;
/// * **claim line:** `w[0]` the epoch of the block's last claim; under
///   [`RegionPolicy::HOME_LINES`], `w[0..8]` hold one participant's home blocks of one entry.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionLine {
    w: [u64; 8],
}

const _: () = assert!(size_of::<RegionLine>() == 64 && align_of::<RegionLine>() == 64);

impl RegionLine {
    /// A line of zero words — the state of every fresh frame line.
    pub const ZERO: Self = Self { w: [0; 8] };

    /// Word `i` (`i < 8`) of the line, read as plain data (only while no region is open over it).
    #[inline]
    #[must_use]
    pub const fn word(&self, i: usize) -> u64 {
        self.w[i]
    }
}

impl Default for RegionLine {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

/// One entry of the stage table (16 B, read-only while the region is open).
///
/// The region reads only [`n_blocks`](Self::n_blocks) and [`first_claim`](Self::first_claim).
/// `kind`, `color` and `first_cut` are the caller's: its [`RegionStages`] reads them to dispatch
/// a block, the region never does.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StageEntry {
    /// The caller's stage kind (opaque to the region).
    pub kind: u8,
    _r: u8,
    /// The caller's colour or sub-index (opaque to the region).
    pub color: u16,
    /// Blocks of this entry. `1` runs inline on participant 0, is never published and has no claim
    /// word; `≥ 2` is published, at most `8 · participants`.
    pub n_blocks: u16,
    _r2: u16,
    /// Index of this entry's first claim line ([`claim_lines`] lines from here belong to it).
    pub first_claim: u32,
    /// The caller's index into its own cut column (opaque to the region).
    pub first_cut: u32,
}

const _: () = assert!(size_of::<StageEntry>() == 16);

impl StageEntry {
    /// An entry of `n_blocks` blocks whose claim lines start at `first_claim`, with the caller's
    /// opaque `kind`, `color` and `first_cut`.
    #[inline]
    #[must_use]
    pub const fn new(kind: u8, color: u16, n_blocks: u16, first_claim: u32, first_cut: u32) -> Self {
        Self { kind, _r: 0, color, n_blocks, _r2: 0, first_claim, first_cut }
    }
}

/// One schedule item (8 B): the entry it executes and the CAS hint of that execution.
///
/// Item `i` runs at epoch `base + 1 + i`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SchedItem {
    /// The entry this item executes.
    pub entry: u16,
    _r: u16,
    /// `(the previous item index of the same entry in this region) + 1`, or 0 on the entry's first
    /// execution in the region. The claim's expected value is `base + prev_off` (or 0); a wrong
    /// value costs one CAS retry, never correctness.
    pub prev_off: u32,
}

const _: () = assert!(size_of::<SchedItem>() == 8);

impl SchedItem {
    /// An item executing `entry`, whose previous execution in the same region was item
    /// `prev_item` (`None` on the entry's first execution).
    #[inline]
    #[must_use]
    pub const fn new(entry: u16, prev_item: Option<u32>) -> Self {
        let prev_off = match prev_item {
            Some(j) => j + 1,
            None => 0,
        };
        Self { entry, _r: 0, prev_off }
    }
}

/// Fills every item's [`SchedItem::prev_off`] from the order of `schedule`, in one pass.
///
/// `last` is the caller's scratch (one `u32` per entry, `last.len()` > every entry index); it is
/// zeroed here first. Panics if an item names an entry `last` cannot hold — a table-build bug.
pub fn link_hints(schedule: &mut [SchedItem], last: &mut [u32]) {
    last.fill(0);
    for (i, item) in schedule.iter_mut().enumerate() {
        let slot = last
            .get_mut(usize::from(item.entry))
            .expect("invariant: the hint scratch holds one slot per entry of the table");
        item.prev_off = *slot;
        *slot = u32::try_from(i + 1).expect("invariant: a region schedule has fewer than 2^32 items");
    }
}

/// The claim lines an entry of `n_blocks` blocks occupies under policy `P` with `participants`
/// participants: 0 for an inline entry, `n_blocks` by default (one line per block), and one home
/// line per participant under [`RegionPolicy::HOME_LINES`].
#[inline]
#[must_use]
pub const fn claim_lines<P: RegionPolicy>(n_blocks: u16, participants: u32) -> u32 {
    if n_blocks <= 1 {
        0
    } else if P::HOME_LINES {
        participants
    } else {
        n_blocks as u32
    }
}

// =========================================================================
// Policy, stages, results
// =========================================================================

/// A waiter's loop. Used only as a const of a [`RegionPolicy`], so every match on it folds away.
///
/// Every ladder is active waiting: no ladder sleeps or parks (the owner's value, 2026-09-29).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ladder {
    /// A burst of `pauses` PAUSEs, then one `yield_now`, repeated: the helpers' default (5,
    /// ruling 2026-09-30 Q6: "PAUSE, then yield_now between PAUSE bursts").
    PauseThenYield {
        /// PAUSEs per burst.
        pauses: u32,
    },
    /// PAUSE on every iteration, never yield: the orchestrator's default (v2), and the helper
    /// axis R-d′ ("PAUSE-only").
    PureSpin,
    /// PAUSE until `spin_ns` is spent (clock read every 64 PAUSEs), then alternate one
    /// `yield_now` with a 64-PAUSE burst: the axes R-d (helpers) and R-e (orchestrator).
    BudgetThenYield {
        /// The PAUSE phase's budget, in ns.
        spin_ns: u32,
    },
}

/// Every protocol and waiter knob of a region, at compile time.
///
/// Tests and benches define their own impls; the shipped one is [`V2Policy`].
pub trait RegionPolicy {
    /// How a helper waits for the next publish.
    const HELPER_WAIT: Ladder;
    /// How the orchestrator waits for an item's completion.
    const ORCH_WAIT: Ladder;
    /// A wait longer than this panics with [`RegionWaitBound`]; 0 = unbounded (shipped). Checked on
    /// every yield and every 64 PAUSEs of every ladder.
    const BOUND_NS: u64;
    /// R-a: one batched `fetch_add(k)` per participant per item instead of one per block.
    const DONE_BATCHED: bool;
    /// R-b: a participant's home blocks of an entry share one claim line.
    const HOME_LINES: bool;
    /// R-c: load the claim word first and skip it without a CAS when it already holds `≥ g`.
    const TTAS: bool;
}

/// The shipped policy: the benched v2 protocol and waits, unbounded, every axis off.
#[derive(Clone, Copy, Debug)]
pub struct V2Policy;

impl RegionPolicy for V2Policy {
    const HELPER_WAIT: Ladder = Ladder::PauseThenYield { pauses: 5 };
    const ORCH_WAIT: Ladder = Ladder::PureSpin;
    const BOUND_NS: u64 = 0;
    const DONE_BATCHED: bool = false;
    const HOME_LINES: bool = false;
    const TTAS: bool = false;
}

/// The work of a region, block by block.
///
/// `Sync` because every participant calls it through a shared reference.
pub trait RegionStages: Sync {
    /// Runs block `block` of entry `entry` on participant `participant` (0 = the orchestrator).
    ///
    /// The implementer's contract: a block writes only data owned by that `(entry, block)` for the
    /// duration of its item, and reads only data no concurrent block of the same item writes. Two
    /// items never overlap: every write of item `i` happens-before every block of item `i + 1`.
    /// A block must not open a nested pool scope (it can deadlock the region).
    fn run_block(&self, entry: u32, block: u32, participant: u32);

    /// Called on participant 0 only, and only in `ARMED = true` monomorphs, before item `item`
    /// runs (before its publish, or before its inline block).
    #[inline]
    fn item_begin(&self, _item: u32) {}

    /// Called on participant 0 only, and only in `ARMED = true` monomorphs, after every block of
    /// item `item` has completed (after its done-wait, or after its inline block).
    #[inline]
    fn item_end(&self, _item: u32) {}
}

/// How a participant left a region, as its receipt records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RegionExit {
    /// No receipt was written for this region (the tag tells which region a line belongs to).
    NotRun = 0,
    /// It read (or, as participant 0, published) the region's END.
    End = 1,
    /// It left a region another participant had poisoned.
    Poisoned = 2,
    /// It unwound out of a block or a region assertion.
    Panicked = 3,
    /// Its wait passed [`RegionPolicy::BOUND_NS`].
    Bound = 4,
}

impl RegionExit {
    /// Decodes a receipt's exit word.
    #[inline]
    #[must_use]
    pub const fn from_word(w: u64) -> Option<Self> {
        match w {
            0 => Some(Self::NotRun),
            1 => Some(Self::End),
            2 => Some(Self::Poisoned),
            3 => Some(Self::Panicked),
            4 => Some(Self::Bound),
            _ => None,
        }
    }
}

/// One participant's receipt, decoded from its receipt line after the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionReceipt {
    /// The `base` of the region that wrote it.
    pub region: u64,
    /// Blocks this participant ran (inline blocks included for participant 0).
    pub blocks: u64,
    /// How it left; `None` for an undecodable word.
    pub exit: Option<RegionExit>,
    /// Armed only: waits longer than [`REGION_STALL_NS`] (a helper's recruitment wait excluded).
    pub stalls: u64,
    /// Armed only: its longest wait, in ns.
    pub max_wait_ns: u64,
}

impl RegionReceipt {
    /// Decodes a receipt line (plain data: only while no region is open over it).
    #[inline]
    #[must_use]
    pub const fn read(line: &RegionLine) -> Self {
        Self {
            region: line.w[R_TAG],
            blocks: line.w[R_BLOCKS],
            exit: RegionExit::from_word(line.w[R_EXIT]),
            stalls: line.w[R_STALLS],
            max_wait_ns: line.w[R_MAX_WAIT],
        }
    }
}

/// A completed region's summary, reduced from the participants' receipts after the join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RegionReport {
    /// The region's `base`: item `i` ran at epoch `base + 1 + i`.
    pub base: u64,
    /// Items published to the helpers (`n_blocks ≥ 2`).
    pub published: u32,
    /// Items run inline on participant 0 (`n_blocks ≤ 1`).
    pub inline: u32,
    /// Blocks the helpers ran (participants 1..).
    pub helper_blocks: u64,
    /// Armed only: the participants' stall counts, summed.
    pub stalls: u64,
    /// Armed only: the longest wait of any participant, in ns.
    pub max_wait_ns: u64,
    /// The largest `n_blocks` of a published item (0 if none was published).
    pub max_blocks: u32,
}

/// The panic payload of a waiter that passed [`RegionPolicy::BOUND_NS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionWaitBound {
    /// The participant whose wait expired.
    pub participant: u32,
    /// The epoch it was waiting at (an item's `g`, or the publish value a helper waited to change).
    pub g: u64,
}

// =========================================================================
// The frame view the protocol runs over
// =========================================================================

/// The words and the table a region's protocol reads, behind one interface: natively a projection
/// of the caller's [`RegionLine`]s ([`FrameWords`]); under `cfg(loom)` model-owned loom atomics
/// ([`LoomRegionWords`]). The protocol core is generic over it and monomorphised — no `dyn`.
pub(crate) trait RegionWords: Sync {
    /// The publish word.
    fn publish(&self) -> &AtomicU64;
    /// The poison word.
    fn poison(&self) -> &AtomicU64;
    /// Entry `e`'s completion count.
    fn done(&self, e: usize) -> &AtomicU64;
    /// How many done lines the frame holds (the open reset clears all of them).
    fn done_lines(&self) -> usize;
    /// Word `word` of claim line `line`.
    fn claim(&self, line: usize, word: usize) -> &AtomicU64;
    /// How many claim lines the frame holds.
    fn claim_lines(&self) -> usize;
    /// How many words of each claim line the frame stores (8 natively).
    fn claim_words(&self) -> usize;
    /// Word `k` of participant `p`'s receipt line.
    fn receipt_word(&self, p: usize, k: usize) -> &AtomicU64;
    /// Stage entry `e`.
    fn entry(&self, e: usize) -> StageEntry;
    /// Schedule item `i`.
    fn item(&self, i: usize) -> SchedItem;
    /// Schedule length.
    fn len(&self) -> usize;
    /// Participants, the orchestrator included.
    fn participants(&self) -> u32;
}

/// One 64-byte line seen as atomics — the native projection of a [`RegionLine`].
#[cfg(not(loom))]
#[repr(C, align(64))]
struct AtomicLine([AtomicU64; 8]);

#[cfg(not(loom))]
const _: () = {
    assert!(size_of::<AtomicLine>() == size_of::<RegionLine>());
    assert!(align_of::<AtomicLine>() == align_of::<RegionLine>());
    assert!(size_of::<AtomicU64>() == size_of::<u64>());
    assert!(align_of::<AtomicU64>() <= align_of::<RegionLine>());
};

/// Projects exclusively borrowed plain lines as shared atomic lines for the borrow's lifetime.
#[cfg(not(loom))]
#[inline]
fn atomic_lines(lines: &mut [RegionLine]) -> &[AtomicLine] {
    // SAFETY: `AtomicLine` and `RegionLine` are both `#[repr(C, align(64))]` arrays of eight
    //   8-byte words with equal size and alignment (asserted above), and `AtomicU64` has the size
    //   and in-memory representation of `u64` with an alignment dividing 64, so every projected
    //   word is a valid, aligned `AtomicU64` (every bit pattern is valid for both types). The
    //   pointer comes from a `&mut` borrow, so it carries write provenance, and the returned
    //   slice borrows from that `&mut`: for its whole lifetime no other access to these lines
    //   exists, and every access through the slice is atomic — `AtomicU64::from_ptr`'s contract.
    //   Writes through the shared view are interior mutability (`UnsafeCell` inside the atomic),
    //   which Stacked and Tree Borrows both permit through a `&` derived from a writable pointer.
    unsafe { core::slice::from_raw_parts(lines.as_mut_ptr().cast::<AtomicLine>(), lines.len()) }
}

/// The native frame view: atomic projections of the caller's lines, plus the read-only table.
#[cfg(not(loom))]
struct FrameWords<'f> {
    sync: &'f AtomicLine,
    done: &'f [AtomicLine],
    receipts: &'f [AtomicLine],
    claims: &'f [AtomicLine],
    entries: &'f [StageEntry],
    schedule: &'f [SchedItem],
    participants: u32,
}

#[cfg(not(loom))]
impl RegionWords for FrameWords<'_> {
    #[inline]
    fn publish(&self) -> &AtomicU64 {
        &self.sync.0[S_PUBLISH]
    }
    #[inline]
    fn poison(&self) -> &AtomicU64 {
        &self.sync.0[S_POISON]
    }
    #[inline]
    fn done(&self, e: usize) -> &AtomicU64 {
        &self.done[e].0[0]
    }
    #[inline]
    fn done_lines(&self) -> usize {
        self.done.len()
    }
    #[inline]
    fn claim(&self, line: usize, word: usize) -> &AtomicU64 {
        &self.claims[line].0[word]
    }
    #[inline]
    fn claim_lines(&self) -> usize {
        self.claims.len()
    }
    #[inline]
    fn claim_words(&self) -> usize {
        8
    }
    #[inline]
    fn receipt_word(&self, p: usize, k: usize) -> &AtomicU64 {
        &self.receipts[p].0[k]
    }
    #[inline]
    fn entry(&self, e: usize) -> StageEntry {
        self.entries[e]
    }
    #[inline]
    fn item(&self, i: usize) -> SchedItem {
        self.schedule[i]
    }
    #[inline]
    fn len(&self) -> usize {
        self.schedule.len()
    }
    #[inline]
    fn participants(&self) -> u32 {
        self.participants
    }
}

/// The frame lines a region borrows, grouped. Each slice is its own storage: the done lines, the
/// receipts and the claim lines never share a column, so no done or receipt value can ever be read
/// as a claim epoch (and the borrow checker forbids overlap).
#[cfg(not(loom))]
pub struct RegionLines<'f> {
    /// The sync line (publish, poison).
    pub sync: &'f mut RegionLine,
    /// The done lines — the WHOLE column, at least one per entry: the open reset after a caught
    /// poisoned region clears every line passed here.
    pub done: &'f mut [RegionLine],
    /// The receipt lines, at least one per participant.
    pub receipts: &'f mut [RegionLine],
    /// The claim lines, at least [`claim_lines`] per published entry from its `first_claim`.
    pub claims: &'f mut [RegionLine],
}

/// One region's frame: its lines (borrowed `&mut` and seen as atomics while it runs), its table,
/// its participant count and the caller's epoch counter.
#[cfg(not(loom))]
pub struct RegionFrame<'f> {
    words: FrameWords<'f>,
    epoch: &'f mut u64,
}

#[cfg(not(loom))]
impl<'f> RegionFrame<'f> {
    /// A frame over the caller's `lines`, running `schedule` over `entries` with `participants`
    /// participants. `epoch` is the caller's region counter: it only grows, and must start at or
    /// above every epoch the claim lines hold (a fresh counter and fresh zero lines satisfy that;
    /// keep the counter and the claim column together).
    ///
    /// # Panics
    ///
    /// If `participants == 0`, if fewer receipt lines than participants or fewer done lines than
    /// entries are given, or if the table has more entries than a `u16` index can name — table-
    /// build bugs. An item or entry that indexes past its slice panics when it is run.
    #[must_use]
    pub fn new(
        lines: RegionLines<'f>,
        entries: &'f [StageEntry],
        schedule: &'f [SchedItem],
        epoch: &'f mut u64,
        participants: u32,
    ) -> Self {
        assert!(participants >= 1, "a region has at least one participant (the orchestrator)");
        assert!(
            lines.receipts.len() >= participants as usize,
            "a region frame needs one receipt line per participant: {} lines for {participants}",
            lines.receipts.len()
        );
        assert!(
            lines.done.len() >= entries.len(),
            "a region frame needs one done line per entry: {} lines for {} entries",
            lines.done.len(),
            entries.len()
        );
        assert!(
            entries.len() <= usize::from(u16::MAX) + 1,
            "a region table's entries are named by a u16"
        );
        Self {
            words: FrameWords {
                sync: &atomic_lines(core::slice::from_mut(lines.sync))[0],
                done: atomic_lines(lines.done),
                receipts: atomic_lines(lines.receipts),
                claims: atomic_lines(lines.claims),
                entries,
                schedule,
                participants,
            },
            epoch,
        }
    }
}

// =========================================================================
// The protocol core (generic over the frame view)
// =========================================================================

/// PAUSE once (`loom`: a model yield, so a spin is a scheduling point).
#[inline]
fn pause() {
    #[cfg(not(loom))]
    core::hint::spin_loop();
    #[cfg(loom)]
    loom::thread::yield_now();
}

/// Yield the thread's time slice.
#[inline]
fn yield_now() {
    crate::sync::thread::yield_now();
}

/// Whether a waiter of this ladder, bound and arming reads a clock at all. Folded per monomorph:
/// the disarmed [`V2Policy`] waits read none.
const fn needs_clock(ladder: Ladder, bound_ns: u64, armed: bool) -> bool {
    bound_ns != 0 || armed || matches!(ladder, Ladder::BudgetThenYield { .. })
}

/// The CAS hint of `item` in a region based at `base`.
#[inline]
const fn hint_of(item: SchedItem, base: u64) -> u64 {
    if item.prev_off == 0 { 0 } else { base + item.prev_off as u64 }
}

// The nested-scope guard (debug builds; `Scope::new` asserts against it).

#[cfg(all(debug_assertions, not(loom)))]
std::thread_local! {
    /// Whether this thread is running a region block right now.
    static IN_REGION_BLOCK: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether the calling thread is running a region block. Read by `Scope::new`'s debug assertion: a
/// scope (or `install`) opened inside a block can run this region's own queued helper task in its
/// join, and that helper then waits for a publish the blocked participant cannot make. Always false
/// in release builds and under loom.
#[inline]
pub(crate) fn in_region_block() -> bool {
    #[cfg(all(debug_assertions, not(loom)))]
    {
        IN_REGION_BLOCK.with(core::cell::Cell::get)
    }
    #[cfg(not(all(debug_assertions, not(loom))))]
    {
        false
    }
}

/// Marks the thread as inside a region block for one block; cleared on drop, an unwinding block
/// included (a caught panic must not leave the thread marked).
#[cfg(all(debug_assertions, not(loom)))]
struct BlockFlag;

#[cfg(all(debug_assertions, not(loom)))]
impl BlockFlag {
    #[inline]
    fn set() -> Self {
        IN_REGION_BLOCK.with(|f| f.set(true));
        Self
    }
}

#[cfg(all(debug_assertions, not(loom)))]
impl Drop for BlockFlag {
    #[inline]
    fn drop(&mut self) {
        IN_REGION_BLOCK.with(|f| f.set(false));
    }
}

/// Runs one block (debug builds: with the thread marked as inside a region block).
#[inline]
fn run_block<S: RegionStages>(stages: &S, entry: u32, block: u32, participant: u32) {
    #[cfg(all(debug_assertions, not(loom)))]
    let _flag = BlockFlag::set();
    stages.run_block(entry, block, participant);
}

/// One participant's private tallies and its unwind guard.
///
/// Armed from creation; every normal exit disarms it after storing the receipt. Dropped while
/// armed (an unwind) it stores the receipt (PANICKED, or BOUND if the wait bound fired), then
/// `poison = 1`, and — on participant 0 — publishes the tagged END, so no helper can be left
/// waiting for a publish that will never come.
pub(crate) struct Participant<'a, W: RegionWords> {
    w: &'a W,
    base: u64,
    index: u32,
    blocks: u64,
    stalls: u64,
    max_wait_ns: u64,
    bound: bool,
    armed: bool,
}

impl<'a, W: RegionWords> Participant<'a, W> {
    /// Participant `index` of the region based at `base`, armed.
    #[inline]
    pub(crate) fn new(w: &'a W, base: u64, index: u32) -> Self {
        Self { w, base, index, blocks: 0, stalls: 0, max_wait_ns: 0, bound: false, armed: true }
    }

    /// The base of this participant's region.
    #[cfg(loom)]
    #[inline]
    pub(crate) fn base(&self) -> u64 {
        self.base
    }

    /// Stores this participant's receipt. Relaxed: receipts are read only after the scope's join,
    /// which orders every participant's stores before the read.
    #[inline]
    fn store_receipt(&self, exit: RegionExit) {
        let p = self.index as usize;
        self.w.receipt_word(p, R_TAG).store(self.base, Ordering::Relaxed);
        self.w.receipt_word(p, R_BLOCKS).store(self.blocks, Ordering::Relaxed);
        self.w.receipt_word(p, R_EXIT).store(exit as u64, Ordering::Relaxed);
        self.w.receipt_word(p, R_STALLS).store(self.stalls, Ordering::Relaxed);
        self.w.receipt_word(p, R_MAX_WAIT).store(self.max_wait_ns, Ordering::Relaxed);
    }

    /// A normal exit: the receipt, then disarm.
    #[inline]
    fn exit(&mut self, exit: RegionExit) {
        self.store_receipt(exit);
        self.armed = false;
    }

    /// The unwind body: the receipt (PANICKED or BOUND), `poison = 1`, and on participant 0 the
    /// tagged END.
    #[cold]
    #[inline(never)]
    pub(crate) fn on_unwind(&mut self) {
        let exit = if self.bound { RegionExit::Bound } else { RegionExit::Panicked };
        self.store_receipt(exit);
        // Release: pairs with the orchestrator's Acquire loads of `poison` (before each item and
        // in its done-wait), so whatever this participant did is ordered before the orchestrator
        // acts on the poison.
        self.w.poison().store(1, Ordering::Release);
        if self.index == 0 {
            // Release: a helper's Acquire load of END then also sees `poison = 1` (stored just
            // above, in program order), so it records POISONED.
            self.w.publish().store(END_BIT | self.base, Ordering::Release);
        }
        self.armed = false;
    }
}

impl<W: RegionWords> Drop for Participant<'_, W> {
    fn drop(&mut self) {
        if self.armed {
            debug_assert!(
                std::thread::panicking(),
                "invariant: a region participant leaves armed only by unwinding"
            );
            self.on_unwind();
        }
    }
}

/// A waiter's loop state.
struct Waiter {
    spins: u32,
    yielding: bool,
    t0: Option<Instant>,
}

impl Waiter {
    /// A wait starting now; reads the clock only when `clock` (folded per monomorph).
    #[inline]
    fn start(clock: bool) -> Self {
        Self { spins: 0, yielding: false, t0: if clock { Some(Instant::now()) } else { None } }
    }

    #[inline]
    fn elapsed_ns(&self) -> u64 {
        self.t0.map_or(0, |t| u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX))
    }

    /// Reads the clock and fires the bound; returns the elapsed ns.
    #[inline]
    fn check<P: RegionPolicy, W: RegionWords>(&self, part: &mut Participant<'_, W>, g: u64) -> u64 {
        let el = self.elapsed_ns();
        if P::BOUND_NS != 0 && el > P::BOUND_NS {
            bound_fired(part, g);
        }
        el
    }

    /// One iteration of `ladder`.
    #[inline]
    fn step<P: RegionPolicy, W: RegionWords>(
        &mut self,
        ladder: Ladder,
        clock: bool,
        part: &mut Participant<'_, W>,
        g: u64,
    ) {
        match ladder {
            Ladder::PureSpin => {
                pause();
                self.spins = self.spins.wrapping_add(1);
                if clock && self.spins.is_multiple_of(CLOCK_EVERY) {
                    self.check::<P, W>(part, g);
                }
            }
            Ladder::PauseThenYield { pauses } => {
                if self.spins < pauses {
                    pause();
                    self.spins += 1;
                    if clock && self.spins.is_multiple_of(CLOCK_EVERY) {
                        self.check::<P, W>(part, g);
                    }
                } else {
                    yield_now();
                    self.spins = 0;
                    if clock {
                        self.check::<P, W>(part, g);
                    }
                }
            }
            Ladder::BudgetThenYield { spin_ns } => {
                if !self.yielding {
                    pause();
                    self.spins += 1;
                    if self.spins.is_multiple_of(CLOCK_EVERY) && self.check::<P, W>(part, g) >= u64::from(spin_ns) {
                        self.yielding = true;
                        self.spins = 0;
                    }
                } else if self.spins == 0 {
                    yield_now();
                    self.spins = 1;
                    self.check::<P, W>(part, g);
                } else {
                    pause();
                    self.spins = if self.spins >= CLOCK_EVERY { 0 } else { self.spins + 1 };
                }
            }
        }
    }

    /// The census at the end of a wait (armed, and not a helper's recruitment wait).
    #[inline]
    fn finish<const ARMED: bool, W: RegionWords>(&self, census: bool, part: &mut Participant<'_, W>) {
        if ARMED && census {
            let el = self.elapsed_ns();
            if el > u64::from(REGION_STALL_NS) {
                part.stalls += 1;
            }
            part.max_wait_ns = part.max_wait_ns.max(el);
        }
    }
}

/// The wait bound fired: record BOUND for the guard, then panic with the payload.
#[cold]
#[inline(never)]
fn bound_fired<W: RegionWords>(part: &mut Participant<'_, W>, g: u64) -> ! {
    part.bound = true;
    std::panic::panic_any(RegionWaitBound { participant: part.index, g })
}

/// One execution of a published entry, as a participant sweeps it.
#[derive(Clone, Copy)]
struct Exec {
    e: u32,
    entry: StageEntry,
    g: u64,
    hint: u64,
}

/// The claim word of block `b` of `entry`.
#[inline]
fn claim_word<'w, W: RegionWords, P: RegionPolicy>(
    w: &'w W,
    entry: &StageEntry,
    b: u32,
    participants: u32,
) -> &'w AtomicU64 {
    let first = entry.first_claim as usize;
    if P::HOME_LINES {
        // Block b's home participant is the largest p with p·n/P ≤ b (Box2D's
        // `GetWorkerStartIndex` partition); its home range is never empty and holds ≤ 8 blocks
        // while n ≤ 8·P.
        let n = u32::from(entry.n_blocks);
        let owner = ((b + 1) * participants - 1) / n;
        let home = owner * n / participants;
        w.claim(first + owner as usize, (b - home) as usize)
    } else {
        w.claim(first + b as usize, 0)
    }
}

/// Claims block `b` of `x` at its epoch: true when this participant won it.
#[inline]
fn claim<W: RegionWords, P: RegionPolicy>(w: &W, x: &Exec, b: u32, participants: u32) -> bool {
    let word = claim_word::<W, P>(w, &x.entry, b, participants);
    let cur = if P::TTAS {
        // Relaxed: a filter only; the CAS below decides.
        let c = word.load(Ordering::Relaxed);
        if c >= x.g {
            return false;
        }
        c
    } else {
        x.hint
    };
    // AcqRel on success (the v2 claim); Relaxed on failure, whose value only feeds the retry.
    match word.compare_exchange(cur, x.g, Ordering::AcqRel, Ordering::Relaxed) {
        Ok(_) => true,
        // An older epoch (the entry's first execution in this region, or a wrong hint): retry once.
        Err(seen) if seen < x.g => match word.compare_exchange(seen, x.g, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => true,
            Err(again) => {
                debug_assert!(
                    again >= x.g,
                    "invariant: within an execution only its own epoch is written to its words, so \
                     a claim retries at most once (word {again}, epoch {})",
                    x.g
                );
                false
            }
        },
        Err(_) => false,
    }
}

/// Claims and runs blocks of `x` from `start`, sweeping the ring once.
#[inline]
fn claim_sweep<W: RegionWords, S: RegionStages, P: RegionPolicy>(
    w: &W,
    stages: &S,
    x: &Exec,
    start: u32,
    part: &mut Participant<'_, W>,
) {
    let n = u32::from(x.entry.n_blocks);
    let participants = w.participants();
    let poison = w.poison();
    let done = w.done(x.e as usize);
    let mut b = start;
    let mut k = 0u64;
    for _ in 0..n {
        // Relaxed: a filter that ends a sweep early; the orchestrator's Acquire loads decide.
        if poison.load(Ordering::Relaxed) != 0 {
            break;
        }
        if claim::<W, P>(w, x, b, participants) {
            run_block(stages, x.e, b, part.index);
            part.blocks += 1;
            k += 1;
            if !P::DONE_BATCHED {
                // Release: this block's writes happen-before the orchestrator's Acquire load of a
                // count that includes this add (every later add is an RMW in its release sequence).
                done.fetch_add(1, Ordering::Release);
            }
        }
        b += 1;
        if b == n {
            b = 0;
        }
    }
    if P::DONE_BATCHED && k > 0 {
        // Release: as above, for all k blocks at once (R-a).
        done.fetch_add(k, Ordering::Release);
    }
}

/// The orchestrator's poisoned exit: its receipt, then the tagged END.
#[cold]
#[inline(never)]
fn poisoned_exit<W: RegionWords>(w: &W, part: &mut Participant<'_, W>) {
    part.exit(RegionExit::Poisoned);
    // Release: helpers that Acquire END see every write ordered before it (including the poison
    // store this exit observed).
    w.publish().store(END_BIT | part.base, Ordering::Release);
}

/// What the orchestrator counted while it ran the schedule.
#[derive(Clone, Copy, Default)]
pub(crate) struct OrchStats {
    pub(crate) published: u32,
    pub(crate) inline: u32,
    pub(crate) max_blocks: u32,
}

/// Reserves the region's epoch range, runs the open reset, stores OPEN; returns `base`.
///
/// Participant 0, before any helper exists. **The reservation comes first** (W8): from the moment
/// this returns — and even if a debug check below unwinds — `*epoch` is past every epoch this
/// region can write, so a region that follows a poisoned one never reuses an epoch.
pub(crate) fn open<W: RegionWords>(w: &W, epoch: &mut u64) -> u64 {
    let base = (*epoch).max(EPOCH_FIRST);
    *epoch = base
        .checked_add(w.len() as u64 + 1)
        .filter(|&next| next < END_BIT)
        .expect("invariant: region epochs stay below END_BIT (2^63 epochs)");
    // Relaxed: the previous region's scope join ordered all of its accesses before this thread.
    if w.poison().load(Ordering::Relaxed) != 0 {
        open_reset(w);
    }
    // Relaxed: OPEN is ordered before every helper's first load by the spawns' Release pushes.
    w.publish().store(base, Ordering::Relaxed);
    #[cfg(debug_assertions)]
    debug_open_checks(w, base);
    base
}

/// The open reset after a caught poisoned region: EVERY done line of the column back to 0, then
/// `poison = 0`. Claim words need no reset (epoch claims).
#[cold]
#[inline(never)]
fn open_reset<W: RegionWords>(w: &W) {
    for e in 0..w.done_lines() {
        w.done(e).store(0, Ordering::Relaxed);
    }
    w.poison().store(0, Ordering::Relaxed);
}

/// Debug: every claim epoch is at most `base`, and every done line is 0.
#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn debug_open_checks<W: RegionWords>(w: &W, base: u64) {
    for line in 0..w.claim_lines() {
        for word in 0..w.claim_words() {
            let v = w.claim(line, word).load(Ordering::Relaxed);
            assert!(
                v <= base,
                "region open: claim line {line} word {word} holds epoch {v} > base {base}; a claim \
                 column must never hold an epoch at or above the next region's (the epoch counter \
                 and the claim column belong together)"
            );
        }
    }
    for e in 0..w.done_lines() {
        let v = w.done(e).load(Ordering::Relaxed);
        assert!(v == 0, "region open: done line {e} holds {v}, not 0, outside a poisoned region");
    }
}

/// Participant 0: every schedule item, then the tagged END. Returns what it counted; on a poisoned
/// region it returns early (the scope's join then re-raises the helper's payload).
pub(crate) fn run_orchestrator<W: RegionWords, S: RegionStages, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
) -> OrchStats {
    let base = part.base;
    let participants = w.participants();
    let publish = w.publish();
    let poison = w.poison();
    let mut stats = OrchStats::default();
    for i in 0..w.len() {
        // Acquire: pairs with a guard's Release store of `poison`.
        if poison.load(Ordering::Acquire) != 0 {
            poisoned_exit(w, part);
            return stats;
        }
        let item = w.item(i);
        let e = u32::from(item.entry);
        let entry = w.entry(e as usize);
        let n = u32::from(entry.n_blocks);
        if ARMED {
            stages.item_begin(i as u32);
        }
        if n <= 1 {
            if n == 1 {
                run_block(stages, e, 0, 0);
                part.blocks += 1;
            }
            stats.inline += 1;
        } else {
            debug_assert!(
                n <= REGION_MAX_BLOCKS_PER_PARTICIPANT * participants,
                "region: entry {e} has {n} blocks, more than {REGION_MAX_BLOCKS_PER_PARTICIPANT} per \
                 participant ({participants} participants)"
            );
            let x = Exec { e, entry, g: base + 1 + i as u64, hint: hint_of(item, base) };
            // Release: every earlier item's writes (acquired by this thread's done-waits) and the
            // done reset below happen-before a helper's Acquire load of this epoch.
            publish.store(x.g, Ordering::Release);
            claim_sweep::<W, S, P>(w, stages, &x, 0, part);
            let done = w.done(e as usize);
            let want = u64::from(n);
            // Acquire: pairs with every block's Release add (one release sequence per line).
            // EXACT equality: a count that overshot must not read as complete.
            if done.load(Ordering::Acquire) != want && !wait_done::<W, P, ARMED>(w, done, want, part, x.g) {
                poisoned_exit(w, part);
                return stats;
            }
            // Relaxed: the next Release publish orders this reset before any later add.
            done.store(0, Ordering::Relaxed);
            stats.published += 1;
            stats.max_blocks = stats.max_blocks.max(n);
        }
        if ARMED {
            stages.item_end(i as u32);
        }
    }
    part.exit(RegionExit::End);
    // Release: every item's writes happen-before a helper's Acquire load of END.
    publish.store(END_BIT | base, Ordering::Release);
    stats
}

/// The orchestrator's done-wait on `P::ORCH_WAIT`: true when the count reached `want`, false on
/// poison.
fn wait_done<W: RegionWords, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    done: &AtomicU64,
    want: u64,
    part: &mut Participant<'_, W>,
    g: u64,
) -> bool {
    let clock = const { needs_clock(P::ORCH_WAIT, P::BOUND_NS, ARMED) };
    let poison = w.poison();
    let mut waiter = Waiter::start(clock);
    loop {
        // Acquire: pairs with a guard's Release store of `poison`.
        if poison.load(Ordering::Acquire) != 0 {
            waiter.finish::<ARMED, W>(true, part);
            return false;
        }
        waiter.step::<P, W>(P::ORCH_WAIT, clock, part, g);
        // Acquire: as at the call site.
        if done.load(Ordering::Acquire) == want {
            waiter.finish::<ARMED, W>(true, part);
            return true;
        }
    }
}

/// A helper's wait on `P::HELPER_WAIT` for a publish value other than `last`.
#[inline]
fn wait_publish<W: RegionWords, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    last: u64,
    part: &mut Participant<'_, W>,
    census: bool,
) -> u64 {
    let publish = w.publish();
    // Acquire: pairs with the orchestrator's Release publish (an epoch or END).
    let v = publish.load(Ordering::Acquire);
    if v != last {
        return v;
    }
    let clock = const { needs_clock(P::HELPER_WAIT, P::BOUND_NS, ARMED) };
    let mut waiter = Waiter::start(clock);
    loop {
        waiter.step::<P, W>(P::HELPER_WAIT, clock, part, last);
        // Acquire: as above.
        let v = publish.load(Ordering::Acquire);
        if v != last {
            waiter.finish::<ARMED, W>(census, part);
            return v;
        }
    }
}

/// Helper `part.index`: claims from its staggered start on every publish until END.
pub(crate) fn run_helper<W: RegionWords, S: RegionStages, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
) {
    let base = part.base;
    let h = part.index;
    let participants = w.participants();
    let mut last = base;
    // The first wait is recruitment, excluded from the stall census.
    let mut census = false;
    loop {
        let v = wait_publish::<W, P, ARMED>(w, last, part, census);
        census = true;
        if v & END_BIT != 0 {
            debug_assert_eq!(
                v & !END_BIT,
                base,
                "region: a helper read another region's END (the open reset did not store OPEN)"
            );
            // Relaxed: the END just acquired was published after the poison store that caused it
            // (by the same guard, or after the orchestrator's Acquire load of it).
            let exit = if w.poison().load(Ordering::Relaxed) != 0 {
                RegionExit::Poisoned
            } else {
                RegionExit::End
            };
            part.exit(exit);
            return;
        }
        let i = v.wrapping_sub(base + 1);
        debug_assert!(
            i < w.len() as u64,
            "region: publish value {v} is not an epoch of the region based at {base}"
        );
        let item = w.item(i as usize);
        let e = u32::from(item.entry);
        let entry = w.entry(e as usize);
        let start = h * u32::from(entry.n_blocks) / participants;
        claim_sweep::<W, S, P>(w, stages, &Exec { e, entry, g: v, hint: hint_of(item, base) }, start, part);
        last = v;
    }
}

/// Sums the receipts after the join.
#[cfg(not(loom))]
fn report<W: RegionWords>(w: &W, base: u64, stats: OrchStats) -> RegionReport {
    let mut r = RegionReport {
        base,
        published: stats.published,
        inline: stats.inline,
        max_blocks: stats.max_blocks,
        ..RegionReport::default()
    };
    for p in 0..w.participants() as usize {
        // Relaxed: the scope's join ordered every participant's receipt stores before these loads.
        let word = |k| w.receipt_word(p, k).load(Ordering::Relaxed);
        debug_assert_eq!(word(R_TAG), base, "region: participant {p}'s receipt is not this region's");
        debug_assert_eq!(word(R_EXIT), RegionExit::End as u64, "region: participant {p} did not read END");
        if p > 0 {
            r.helper_blocks += word(R_BLOCKS);
        }
        r.stalls += word(R_STALLS);
        r.max_wait_ns = r.max_wait_ns.max(word(R_MAX_WAIT));
    }
    r
}

// =========================================================================
// The entry point
// =========================================================================

#[cfg(not(loom))]
impl PoolInner {
    /// Runs one region over `frame` on this pool: the calling thread is participant 0, and
    /// `participants − 1` helper tasks are spawned into one [`scope`](Self::scope).
    ///
    /// Work-conserving: participant 0 alone can complete every item, so the region finishes even if
    /// no helper ever runs. A panic in any participant poisons the region, ends it, and is re-raised
    /// here at the join; no schedule item after it runs. The caller's epoch counter is advanced by
    /// the schedule's length + 1 before anything else happens, on every path.
    ///
    /// `ARMED` compiles in the stall census and [`RegionStages::item_begin`]/`item_end`; pick the
    /// monomorph once per region. Must be called inside an [`install`](Self::install) frame (as
    /// `scope` must), and never from inside a region block.
    pub fn region<S: RegionStages, P: RegionPolicy, const ARMED: bool>(
        &self,
        frame: RegionFrame<'_>,
        stages: &S,
    ) -> RegionReport {
        let RegionFrame { words, epoch } = frame;
        let base = open(&words, epoch);
        #[cfg(debug_assertions)]
        debug_table_checks::<P>(&words);
        let participants = words.participants;
        let stats = if participants <= 1 {
            let mut orch = Participant::new(&words, base, 0);
            run_orchestrator::<_, S, P, ARMED>(&words, stages, &mut orch)
        } else {
            let words = &words;
            self.scope(|s| {
                // The guard is armed BEFORE the first spawn and lives inside the scope closure, so
                // an unwind drops it (publishing END) before `Scope::drop` joins the helpers.
                let mut orch = Participant::new(words, base, 0);
                for h in 1..participants {
                    s.spawn(move || {
                        let mut part = Participant::new(words, base, h);
                        run_helper::<_, S, P, ARMED>(words, stages, &mut part);
                    });
                }
                run_orchestrator::<_, S, P, ARMED>(words, stages, &mut orch)
            })
        };
        report(&words, base, stats)
    }
}

#[cfg(not(loom))]
impl ThreadPool {
    /// Runs one region on this pool; see [`PoolInner::region`].
    #[inline]
    pub fn region<S: RegionStages, P: RegionPolicy, const ARMED: bool>(
        &self,
        frame: RegionFrame<'_>,
        stages: &S,
    ) -> RegionReport {
        self.inner.region::<S, P, ARMED>(frame, stages)
    }
}

/// Miri only: one region on scoped std threads instead of the pool — the pool-free twin the
/// Stacked Borrows leg drives. `crossbeam-epoch`'s deque transport is not Stacked-Borrows clean
/// (Miri reports its intrusive list on the first steal), so no pool-backed test can run under SB.
/// The protocol core, the frame projection and the guards are the production ones; only how the
/// helpers are started differs. The orchestrator's guard lives inside the scope closure exactly as
/// in [`PoolInner::region`], so an unwind publishes END before `std::thread::scope` joins.
#[cfg(miri)]
#[doc(hidden)]
pub fn region_on_threads<S: RegionStages, P: RegionPolicy, const ARMED: bool>(
    frame: RegionFrame<'_>,
    stages: &S,
) -> RegionReport {
    let RegionFrame { words, epoch } = frame;
    let base = open(&words, epoch);
    #[cfg(debug_assertions)]
    debug_table_checks::<P>(&words);
    let participants = words.participants;
    let stats = {
        let words = &words;
        std::thread::scope(|s| {
            let mut orch = Participant::new(words, base, 0);
            for h in 1..participants {
                s.spawn(move || {
                    let mut part = Participant::new(words, base, h);
                    run_helper::<_, S, P, ARMED>(words, stages, &mut part);
                });
            }
            run_orchestrator::<_, S, P, ARMED>(words, stages, &mut orch)
        })
    };
    report(&words, base, stats)
}

/// Debug: every entry's claim lines lie in the claim column and every item names an entry.
#[cfg(all(debug_assertions, not(loom)))]
#[cold]
#[inline(never)]
fn debug_table_checks<P: RegionPolicy>(w: &FrameWords<'_>) {
    for (e, entry) in w.entries.iter().enumerate() {
        let need = claim_lines::<P>(entry.n_blocks, w.participants) as usize;
        assert!(
            entry.first_claim as usize + need <= w.claims.len(),
            "region table: entry {e}'s {need} claim lines from {} pass the column's {}",
            entry.first_claim,
            w.claims.len()
        );
    }
    for (i, item) in w.schedule.iter().enumerate() {
        assert!(
            usize::from(item.entry) < w.entries.len(),
            "region table: item {i} names entry {} of {}",
            item.entry,
            w.entries.len()
        );
    }
}

// =========================================================================
// loom: the model-owned frame view
// =========================================================================

/// The loom frame view: model-owned loom atomics and a loom-tracked table, so the models drive the
/// production protocol core ([`open`], [`run_orchestrator`], [`run_helper`], the guards) over
/// atomics loom can see. `cfg(loom)` only; re-exported through `loom_exports::region`.
#[cfg(loom)]
pub struct LoomRegionWords {
    sync: [AtomicU64; 2],
    done: Box<[AtomicU64]>,
    receipts: Box<[AtomicU64]>,
    claims: Box<[AtomicU64]>,
    words_per_line: usize,
    entries: loom::cell::UnsafeCell<Vec<StageEntry>>,
    schedule: loom::cell::UnsafeCell<Vec<SchedItem>>,
    participants: u32,
}

// SAFETY: the atomics are loom atomics (Sync); the two table cells are read only through loom's
// `UnsafeCell::with` and written only through `with_mut` (`set_table`, whose caller promises no
// region is running), and loom REPORTS any write not ordered against a read instead of letting it
// race — which is the property the M-R6 model's mutation must turn red.
#[cfg(loom)]
unsafe impl Sync for LoomRegionWords {}

#[cfg(loom)]
impl LoomRegionWords {
    /// A zeroed frame of `done_lines` done lines, `claim_lines × words_per_line` claim words
    /// (`words_per_line` = 1 by default, 8 for [`RegionPolicy::HOME_LINES`]) and one receipt per
    /// participant, over a table.
    pub fn new(
        entries: Vec<StageEntry>,
        schedule: Vec<SchedItem>,
        done_lines: usize,
        claim_lines: usize,
        words_per_line: usize,
        participants: u32,
    ) -> Self {
        let zeros = |n: usize| (0..n).map(|_| AtomicU64::new(0)).collect::<Box<[AtomicU64]>>();
        Self {
            sync: [AtomicU64::new(0), AtomicU64::new(0)],
            done: zeros(done_lines),
            receipts: zeros(participants as usize * RECEIPT_WORDS),
            claims: zeros(claim_lines * words_per_line),
            words_per_line,
            entries: loom::cell::UnsafeCell::new(entries),
            schedule: loom::cell::UnsafeCell::new(schedule),
            participants,
        }
    }

    /// Replaces the table (a rebuild between regions).
    ///
    /// # Safety
    /// No region may be running over these words: no participant may read the table concurrently.
    /// Under loom a violation is reported as a causality error rather than being UB, which is what
    /// the M-R6 model's mutation relies on.
    pub unsafe fn set_table(&self, entries: Vec<StageEntry>, schedule: Vec<SchedItem>) {
        // SAFETY: the caller guarantees no concurrent table read; loom checks the claim.
        self.entries.with_mut(|p| unsafe { *p = entries });
        // SAFETY: as above.
        self.schedule.with_mut(|p| unsafe { *p = schedule });
    }

    /// Participant `p`'s receipt (Relaxed loads; read after the participants have joined).
    pub fn receipt(&self, p: u32) -> RegionReceipt {
        let word = |k: usize| self.receipts[p as usize * RECEIPT_WORDS + k].load(Ordering::Relaxed);
        RegionReceipt {
            region: word(R_TAG),
            blocks: word(R_BLOCKS),
            exit: RegionExit::from_word(word(R_EXIT)),
            stalls: word(R_STALLS),
            max_wait_ns: word(R_MAX_WAIT),
        }
    }

    /// Claim word `word` of line `line` (Relaxed load).
    pub fn claim_value(&self, line: usize, word: usize) -> u64 {
        self.claim(line, word).load(Ordering::Relaxed)
    }

    /// Entry `e`'s done count (Relaxed load).
    pub fn done_value(&self, e: usize) -> u64 {
        self.done[e].load(Ordering::Relaxed)
    }

    /// The poison word (Relaxed load).
    pub fn poison_value(&self) -> u64 {
        self.sync[S_POISON].load(Ordering::Relaxed)
    }
}

#[cfg(loom)]
impl RegionWords for LoomRegionWords {
    fn publish(&self) -> &AtomicU64 {
        &self.sync[S_PUBLISH]
    }
    fn poison(&self) -> &AtomicU64 {
        &self.sync[S_POISON]
    }
    fn done(&self, e: usize) -> &AtomicU64 {
        &self.done[e]
    }
    fn done_lines(&self) -> usize {
        self.done.len()
    }
    fn claim(&self, line: usize, word: usize) -> &AtomicU64 {
        assert!(word < self.words_per_line, "loom region frame: claim word {word} is not stored");
        &self.claims[line * self.words_per_line + word]
    }
    fn claim_lines(&self) -> usize {
        self.claims.len() / self.words_per_line
    }
    fn claim_words(&self) -> usize {
        self.words_per_line
    }
    fn receipt_word(&self, p: usize, k: usize) -> &AtomicU64 {
        &self.receipts[p * RECEIPT_WORDS + k]
    }
    fn entry(&self, e: usize) -> StageEntry {
        // SAFETY: a shared read of the table; loom reports any write concurrent with it.
        self.entries.with(|p| unsafe { (&*p)[e] })
    }
    fn item(&self, i: usize) -> SchedItem {
        // SAFETY: as in `entry`.
        self.schedule.with(|p| unsafe { (&*p)[i] })
    }
    fn len(&self) -> usize {
        // SAFETY: as in `entry`.
        self.schedule.with(|p| unsafe { (&*p).len() })
    }
    fn participants(&self) -> u32 {
        self.participants
    }
}
