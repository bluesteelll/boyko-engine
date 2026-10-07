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
//!   words are therefore never reset. `hint` is the epoch of the entry's previous execution in
//!   this region ([`SchedItem::prev_off`]), and 0 whenever `prev_off` names no earlier item, so
//!   every expected value is below `g`: a claim word only ever rises, and a wrong hint only costs
//!   that one retry (an expected value at or above `g` could match a word already claimed at `g`
//!   and win the block twice — review r3 W1).
//! * **The epoch range is reserved at entry** (the W8 fix). [`PoolInner::region`] advances the
//!   caller's epoch counter by `len + 1` BEFORE it writes anything, so every exit — END, poison, an
//!   unwind, a bound panic — leaves the counter past every epoch the region can have written, and
//!   the next region's first epoch is above every claim word ("every claim epoch ≤ base at open").
//! * **Tagged END and the open reset.** END is `END_BIT | base`. At open, participant 0 stores
//!   `publish = base` (OPEN) before any helper is spawned, so a helper of THIS region can never
//!   read the previous region's END; a helper that reads END debug-asserts the tag.
//! * **Exact completion.** Each counted block does `done.fetch_add(1, Release)` (or one batched
//!   add per participant, [`RegionPolicy::DONE_BATCHED`]); the orchestrator waits for EXACTLY
//!   `n_blocks` with `Acquire`.
//! * **The done line is reset just before the item's publish** (ruling 17 B1), not after its
//!   done-wait. The count an item waits on therefore starts at 0 whatever the line held — a
//!   poisoned region's partial count, a line the caller re-created or never cleared — so no stale
//!   count can complete an item early, and two items can never overlap, whatever state the lines
//!   are in. Between items a line holds its entry's last count.
//! * **Poison on unwind.** Every participant holds a guard. A participant that unwinds stores its
//!   receipt, then `poison = 1`; participant 0's guard also publishes the tagged END. The
//!   orchestrator checks `poison` before every item and on every done-wait iteration, helpers
//!   before every claim. The scope's join re-raises the payload.
//! * **The open after a caught poisoned region** sees `poison == 1` and stores `poison = 0`. The
//!   partial count the poisoned item left on its done line needs no reset here: the next publish of
//!   that entry resets its line first.
//! * **Helpers spin** (the owner's value, 2026-09-29): a waiting helper PAUSEs and yields, it
//!   never sleeps or parks inside a region. The orchestrator's wait is pure PAUSE by default.
//!
//! ## The advance axis (CR-F)
//!
//! Under [`Advance::Finisher`] — a disarmed region of at least two participants; an `ARMED`
//! region and a one-participant region run the orchestrator above — every participant runs one
//! loop, and each item boundary has exactly one advancer:
//!
//! * **Into a published item, or into the END:** the participant whose completion add made the
//!   item's count exact. Its add is `AcqRel`, so it acquires every earlier add of the item (one
//!   release sequence per done line), and with them every write of the item. It checks `poison`
//!   (Acquire), resets the next item's done line and publishes its epoch (Release), or stores the
//!   normal END after the last item. Ruling 17 B1 holds for whichever participant advances: the
//!   reset precedes the publish in its program order, and every add of the entry's previous
//!   execution precedes the reset by the chain of completions and publishes since.
//! * **Into an inline item:** participant 0, as under the orchestrator. Its own add completed the
//!   item, or its Acquire done-wait (which polls `poison`) read the exact count; it runs the inline
//!   items in order, then resets and publishes the next published item, or the END. Inline items
//!   never leave participant 0. The region's start is such a boundary.
//! * **Waiting** for a publish, every participant also loads `poison` (Acquire) on every
//!   iteration. After a helper's panic no END is certain (only participant 0's guard publishes
//!   one), so this poll is every survivor's exit. `poison` is word 1 of the sync line whose word
//!   0 the wait already spins on, so the poll adds no line traffic. Participant 0 waits on
//!   [`RegionPolicy::ORCH_WAIT`], the helpers on [`RegionPolicy::HELPER_WAIT`].
//! * The report's `published`, `inline` and `max_blocks` are a walk of the table under this
//!   axis, because participant 0 no longer sees every item; the identity on
//!   [`RegionReport::advances`] is then the witness that the schedule ran.
//!
//! ## Storage is the caller's (Principle 0)
//!
//! The frame — the sync line, the done lines, the receipts, the claim lines — is plain
//! [`RegionLine`]s the caller owns (the physics solver keeps them in its Resource's
//! `ScratchColumn`s). [`RegionFrame::new`] borrows them `&mut` for the region and projects them
//! as atomics; nothing is allocated here except the scope's own task cells. It is an `unsafe fn`
//! (ruling 17 A3): its `# Safety` contract is the storage precondition — the lines exclusively the
//! caller's, sized for the schedule, the epoch counter the claim column's own.
//!
//! ## Every knob is a compile-time policy
//!
//! [`RegionPolicy`] carries the waiter ladders, the test-only wait bound, the three protocol
//! axes (R-a batched completion, R-b home claim lines, R-c TTAS) and the advance axis
//! ([`RegionPolicy::ADVANCE`], CR-F: who publishes the next item) as associated consts, so every
//! branch on them folds after monomorphisation. The `ARMED` const parameter compiles the stall
//! census and the per-item hooks in; the disarmed [`V2Policy`] monomorph reads no clock at all.
//! [`WithAdvance`] replaces a policy's advance axis and copies every other const, so the claim
//! encoding of `P` and `WithAdvance<P, _>` is the same by construction.

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
/// The publishes this participant made (an epoch or the normal END; see [`RegionReport::advances`]).
const R_ADVANCES: usize = 5;
/// Words of a receipt line that carry data.
#[cfg(loom)]
const RECEIPT_WORDS: usize = 6;

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
/// * **done line:** `w[0]` the completion count of one entry (reset to 0 just before each publish
///   of the entry; between items it holds the entry's last count);
/// * **receipt line:** `w[0]` the region tag (`base`), `w[1]` blocks run, `w[2]` the
///   [`RegionExit`], `w[3]` stalls, `w[4]` the longest wait in ns, `w[5]` the publishes this
///   participant made ([`RegionReceipt::advances`]);
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
    /// execution in the region. For item `i` the claim's expected value is `base + prev_off` when
    /// `1 ≤ prev_off ≤ i` (an earlier item), and 0 otherwise. A performance hint only: every value
    /// — a wrong item, the item's own index + 1, a later item, `u32::MAX` — costs at most one CAS
    /// retry, never correctness.
    pub prev_off: u32,
}

const _: () = assert!(size_of::<SchedItem>() == 8);

impl SchedItem {
    /// An item executing `entry`, whose previous execution in the same region was item
    /// `prev_item` (`None` on the entry's first execution). A `prev_item` that is not an earlier
    /// item of the schedule is no hint (see [`prev_off`](Self::prev_off)); `u32::MAX` saturates.
    #[inline]
    #[must_use]
    pub const fn new(entry: u16, prev_item: Option<u32>) -> Self {
        let prev_off = match prev_item {
            Some(j) => j.saturating_add(1),
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
    /// How participant 0 waits: for an item's completion, and under [`Advance::Finisher`] also for
    /// the next publish.
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
    /// CR-F: who publishes the next item. Defaulted, so no policy that predates the axis changes;
    /// a wrapper that forwards a policy's consts must forward this one too, or it silently runs
    /// the orchestrator.
    const ADVANCE: Advance = Advance::Orchestrator;
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

/// Who publishes a region's next item (the advance axis, CR-F). Used only as a const of a
/// [`RegionPolicy`], so every match on it folds away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// Participant 0 waits for each published item's exact count and publishes the next item
    /// (the v2 protocol).
    Orchestrator,
    /// The participant whose completion add makes an item's count exact publishes the next
    /// published item, or the END after the last one (Jolt 5.6 `LargeIslandSplitter`, Rapier 0.36
    /// `staged_island_solver`). A boundary into an inline item stays participant 0's, and inline
    /// items run on participant 0 only. Disarmed monomorphs with at least two participants only:
    /// an `ARMED` region, and a region of one participant, run the orchestrator.
    Finisher,
}

/// `P` with its advance axis replaced: [`Advance::Finisher`] when `FIN`, `P`'s own otherwise.
///
/// Every other const is `P`'s, so the claim encoding, [`claim_lines`] and every ladder of
/// `WithAdvance<P, FIN>` are `P`'s by construction, and frames of `P` and `WithAdvance<P, _>` may
/// alternate over one claim column.
pub struct WithAdvance<P, const FIN: bool>(core::marker::PhantomData<P>);

impl<P: RegionPolicy, const FIN: bool> RegionPolicy for WithAdvance<P, FIN> {
    const HELPER_WAIT: Ladder = P::HELPER_WAIT;
    const ORCH_WAIT: Ladder = P::ORCH_WAIT;
    const BOUND_NS: u64 = P::BOUND_NS;
    const DONE_BATCHED: bool = P::DONE_BATCHED;
    const HOME_LINES: bool = P::HOME_LINES;
    const TTAS: bool = P::TTAS;
    const ADVANCE: Advance = if FIN { Advance::Finisher } else { P::ADVANCE };
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
    /// The publishes this participant made: epochs and the normal END, poisoned ENDs excluded
    /// (the identity they sum to is stated on [`RegionReport::advances`]).
    pub advances: u64,
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
            advances: line.w[R_ADVANCES],
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
    /// Every publish of the region, summed over the participants' receipts
    /// ([`RegionReceipt::advances`]): each published item's epoch once, plus the normal END once.
    ///
    /// **The identity** (CR-F, stated here and only cited elsewhere): on every region that
    /// returns, `Σ_p receipt(p).advances == advances == published + 1`, the `+ 1` being the END.
    /// A poisoned END is no schedule advance and is not counted, and a poisoned region never
    /// returns a report. Debug builds assert it on every region; the region tests and the ω_b
    /// bench assert it in release too.
    pub advances: u64,
    /// The part of [`advances`](Self::advances) the helpers (participants 1..) made: 0 whenever
    /// participant 0 advances every item ([`Advance::Orchestrator`], an `ARMED` region, one
    /// participant).
    pub helper_advances: u64,
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
    /// Word `word` of claim line `line`.
    fn claim(&self, line: usize, word: usize) -> &AtomicU64;
    /// How many claim lines the frame holds. Debug builds only: the open check is its one reader,
    /// and release (`-D warnings`, as CI builds it) rejects a trait method nothing calls.
    #[cfg(debug_assertions)]
    fn claim_lines(&self) -> usize;
    /// How many words of each claim line the frame stores (8 natively). Debug builds only, for the
    /// same reason as `claim_lines`.
    #[cfg(debug_assertions)]
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
    fn claim(&self, line: usize, word: usize) -> &AtomicU64 {
        &self.claims[line].0[word]
    }
    #[cfg(debug_assertions)]
    #[inline]
    fn claim_lines(&self) -> usize {
        self.claims.len()
    }
    #[cfg(debug_assertions)]
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
    /// The done lines, at least one per entry. Their contents need no reset between regions: each
    /// published item resets its own line just before its publish.
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
    /// participants. `epoch` is the caller's region counter.
    ///
    /// # Safety
    ///
    /// The frame's storage is the caller's own (ruling 17 A3). The caller guarantees that:
    ///
    /// * **Exclusively owned.** The sync line, the done lines, the receipt lines and the claim
    ///   lines belong to one owner — the caller whose `epoch` counter is passed here — and are given
    ///   to this frame alone for its lifetime (the `&mut` borrows in `lines` already make "nothing
    ///   else touches them meanwhile" a borrow-checker property). They are not shared with another
    ///   live frame: no frame over another owner's counter runs over any of them, now or between
    ///   this owner's regions.
    /// * **Sized for its schedule.** At least one done line per entry, one receipt line per
    ///   participant, and [`claim_lines`] claim lines for every published entry from its
    ///   `first_claim`, laid out for the policy and the participant count this frame runs with.
    ///   The claim column's layout and encoding depend on [`RegionPolicy::HOME_LINES`] alone,
    ///   never on [`RegionPolicy::ADVANCE`] (nor on the ladders, R-a or R-c), so a column laid
    ///   out for `P` serves `WithAdvance<P, _>` too, and frames of the two may alternate over it.
    /// * **The counter is the claim column's own.** `epoch` only grows, and it is at or above every
    ///   epoch the claim lines hold. A fresh counter over fresh zero lines satisfies it; keep the
    ///   counter and the claim column together.
    ///
    /// What a broken contract can cost is bounded. A breach a caller can make within the borrow
    /// rules — whatever the lines hold, a counter that is not the claim column's own, a column
    /// handed to two owners in turn, short columns — may hang the region (a claim word at or above
    /// an item's epoch is never claimed, so that item never completes) or panic (the asserts below,
    /// the debug open and table checks, an index past a slice). It cannot make two items overlap
    /// or run a block twice: each published item resets its own done line just before its publish
    /// (ruling 17 B1), so it completes only on its own blocks' adds, and a claim word only rises.
    /// That is the guarantee [`RegionStages::run_block`]'s implementers rely on. (Two frames live
    /// over the same lines at once would need two `&mut` borrows of them, which the borrow checker
    /// refuses.)
    ///
    /// # Panics
    ///
    /// If `participants == 0`, if fewer receipt lines than participants or fewer done lines than
    /// entries are given, or if the table has more entries than a `u16` index can name — table-
    /// build bugs. An item or entry that indexes past its slice panics when it is run.
    #[must_use]
    pub unsafe fn new(
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

/// The CAS hint of `item`, item `i` of a region based at `base`: the epoch of the earlier item
/// `prev_off − 1`, or 0 (no hint) when `prev_off` is 0 or names no earlier item (`prev_off > i`).
///
/// The clamp is what makes the hint performance-only for every value the safe API can build
/// (review r3 W1). With every expected value below `g`, every successful claim CAS writes `g` over
/// a smaller value, so a claim word only ever rises, and a block's word passes from below `g` to
/// `g` once: exactly one winner per execution. An expected value at `g` (the item's own epoch)
/// matched a word another participant had already claimed at `g`; one above `g` (a later item's
/// epoch) matched a word that item had claimed while a sweep of this item still ran, and lowered it.
/// Both won a block twice. One compare per item per participant, none per block.
#[inline]
const fn hint_of(item: SchedItem, base: u64, i: u64) -> u64 {
    let off = item.prev_off as u64;
    if off != 0 && off <= i { base + off } else { 0 }
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
    /// Publishes this participant made (the receipt's `R_ADVANCES` word).
    advances: u64,
    bound: bool,
    armed: bool,
}

impl<'a, W: RegionWords> Participant<'a, W> {
    /// Participant `index` of the region based at `base`, armed.
    #[inline]
    pub(crate) fn new(w: &'a W, base: u64, index: u32) -> Self {
        Self { w, base, index, blocks: 0, stalls: 0, max_wait_ns: 0, advances: 0, bound: false, armed: true }
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
        self.w.receipt_word(p, R_ADVANCES).store(self.advances, Ordering::Relaxed);
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
///
/// `FIN` (the finisher path, CR-F) changes only the completion add: `AcqRel` instead of `Release`,
/// and the sweep returns true, and stops, when this participant's add made the count exactly
/// `n_blocks` — every block is then claimed and run, so no claim is left to try. Under `FIN =
/// false` it returns false and is the v2 sweep. One body for both, so a later fix to the claim
/// loop cannot land in one copy only.
#[inline]
fn claim_sweep<W: RegionWords, S: RegionStages, P: RegionPolicy, const FIN: bool>(
    w: &W,
    stages: &S,
    x: &Exec,
    start: u32,
    part: &mut Participant<'_, W>,
) -> bool {
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
                if FIN {
                    // AcqRel: Release as below; and the add that makes the count `n` reads from
                    // the release sequence of every earlier add of this item (each an RMW on one
                    // line), so every block's writes happen-before this participant's reset and
                    // publish of the next item.
                    if done.fetch_add(1, Ordering::AcqRel) + 1 == u64::from(n) {
                        return true;
                    }
                } else {
                    // Release: this block's writes happen-before the orchestrator's Acquire load
                    // of a count that includes this add (every later add is an RMW in its release
                    // sequence).
                    done.fetch_add(1, Ordering::Release);
                }
            }
        }
        b += 1;
        if b == n {
            b = 0;
        }
    }
    if P::DONE_BATCHED && k > 0 {
        if FIN {
            // AcqRel: as for the per-block add above, for all k blocks at once (R-a).
            return done.fetch_add(k, Ordering::AcqRel) + k == u64::from(n);
        }
        // Release: as above, for all k blocks at once (R-a).
        done.fetch_add(k, Ordering::Release);
    }
    false
}

/// The finisher's sweep: [`claim_sweep`] with the `AcqRel` completion add; true when this
/// participant completed the item.
#[inline]
fn sweep_fin<W: RegionWords, S: RegionStages, P: RegionPolicy>(
    w: &W,
    stages: &S,
    x: &Exec,
    start: u32,
    part: &mut Participant<'_, W>,
) -> bool {
    claim_sweep::<W, S, P, true>(w, stages, x, start, part)
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

/// Reserves the region's epoch range, clears a caught poisoned region's poison, stores OPEN;
/// returns `base`.
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

/// The open reset after a caught poisoned region: `poison = 0`. Nothing else needs one: a done
/// line is reset by the next publish of its entry (ruling 17 B1), and claim words are epochs.
#[cold]
#[inline(never)]
fn open_reset<W: RegionWords>(w: &W) {
    w.poison().store(0, Ordering::Relaxed);
}

/// Debug: every claim epoch is at most `base`. (A done line may hold anything at open: each
/// published item resets its own line before its publish.)
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
            let x = Exec { e, entry, g: base + 1 + i as u64, hint: hint_of(item, base, i as u64) };
            let done = w.done(e as usize);
            // The reset comes BEFORE the publish (ruling 17 B1), so this item's count starts at 0
            // whatever the line holds, and only this item's adds can complete it. Every add of an
            // earlier execution of the entry precedes this store in the line's modification order:
            // in this region, this thread's done-wait read exactly `n_blocks` of them, and every
            // block was claimed once, so none is still to come; in an earlier region, its scope's
            // join ordered them before this thread. Relaxed: the Release publish below orders the
            // reset before every helper's Acquire load of `g`, hence before every add of this item.
            done.store(0, Ordering::Relaxed);
            // Release: every earlier item's writes (acquired by this thread's done-waits) and the
            // done reset above happen-before a helper's Acquire load of this epoch.
            publish.store(x.g, Ordering::Release);
            claim_sweep::<W, S, P, false>(w, stages, &x, 0, part);
            let want = u64::from(n);
            // Acquire: pairs with every block's Release add (one release sequence per line).
            // EXACT equality: a count that overshot must not read as complete.
            if done.load(Ordering::Acquire) != want && !wait_done::<W, P, ARMED>(w, done, want, part, x.g) {
                poisoned_exit(w, part);
                return stats;
            }
            stats.published += 1;
            stats.max_blocks = stats.max_blocks.max(n);
        }
        if ARMED {
            stages.item_end(i as u32);
        }
    }
    // Participant 0 made every publish of this region: each published item's epoch and the END
    // below (the identity on `RegionReport::advances`). Set once, here, so the item loop carries
    // no extra increment.
    part.advances = u64::from(stats.published) + 1;
    part.exit(RegionExit::End);
    // Relaxed: no reader depends on an edge from this store (tester r3 N4). A helper that loads
    // END runs no block and reads nothing participant 0 wrote: it checks the tag, loads `poison`
    // and stores its own receipt. In a region that ends normally, `poison`'s last store precedes
    // this region's spawns, which order it before every helper's load; a poisoning that races
    // this END is re-raised by the scope's join whatever the helper's receipt says. What the
    // caller reads afterwards (the blocks' data, the receipts) is ordered by the done counts'
    // release sequences and by that join, not by END. The POISONED ENDs (`poisoned_exit`,
    // `Participant::on_unwind`) stay Release: a helper's POISONED receipt depends on them.
    publish.store(END_BIT | base, Ordering::Relaxed);
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
    // Acquire: pairs with the orchestrator's Release publish of an epoch or of a poisoned END
    // (the normal END is Relaxed: nothing after it reads data, see `run_orchestrator`).
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
            // Relaxed: a poisoned END was published (Release) after the poison store that caused
            // it (by the same guard, or after the orchestrator's Acquire load of it). A normal END
            // has no poison store to show: barring a racing poisoning (which the scope's join
            // re-raises), `poison`'s last store precedes this helper's spawn.
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
        claim_sweep::<W, S, P, false>(w, stages, &Exec { e, entry, g: v, hint: hint_of(item, base, i) }, start, part);
        last = v;
    }
}

// -------------------------------------------------------------------------
// The finisher path (CR-F, `Advance::Finisher`)
// -------------------------------------------------------------------------

/// Whether a monomorph runs the finisher path: [`Advance::Finisher`] and disarmed (the armed hooks
/// are participant 0's, so an `ARMED` region runs the orchestrator). Folded per monomorph.
const fn fin<P: RegionPolicy, const ARMED: bool>() -> bool {
    matches!(P::ADVANCE, Advance::Finisher) && !ARMED
}

/// Participant 0's role entry: the finisher path under [`fin`], the orchestrator otherwise. The
/// choice is a const, so each monomorph holds exactly one of the two.
#[inline]
pub(crate) fn run_participant0<W: RegionWords, S: RegionStages, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
) -> OrchStats {
    if const { fin::<P, ARMED>() } {
        run_finisher::<W, S, P, true>(w, stages, part);
        table_stats(w)
    } else {
        run_orchestrator::<W, S, P, ARMED>(w, stages, part)
    }
}

/// A helper's role entry: the finisher path under [`fin`], the v2 helper otherwise (a const
/// choice, as in [`run_participant0`]).
#[inline]
pub(crate) fn run_helper_participant<W: RegionWords, S: RegionStages, P: RegionPolicy, const ARMED: bool>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
) {
    if const { fin::<P, ARMED>() } {
        run_finisher::<W, S, P, false>(w, stages, part);
    } else {
        run_helper::<W, S, P, ARMED>(w, stages, part);
    }
}

/// The schedule's counts, from the table alone: under the finisher participant 0 does not see
/// every item, so `report`'s `published`, `inline` and `max_blocks` are this walk (no allocation;
/// one entry read per item).
fn table_stats<W: RegionWords>(w: &W) -> OrchStats {
    let mut s = OrchStats::default();
    for i in 0..w.len() {
        let n = u32::from(w.entry(usize::from(w.item(i).entry)).n_blocks);
        if n <= 1 {
            s.inline += 1;
        } else {
            s.published += 1;
            s.max_blocks = s.max_blocks.max(n);
        }
    }
    s
}

/// A finisher's poisoned exit: participant 0 also publishes the tagged END (`poisoned_exit`), a
/// helper only stores its receipt (every other survivor's wait polls `poison`).
#[inline]
fn fin_poisoned<W: RegionWords, const P0: bool>(w: &W, part: &mut Participant<'_, W>) {
    if P0 {
        poisoned_exit(w, part);
    } else {
        part.exit(RegionExit::Poisoned);
    }
}

/// Every participant's loop under the finisher; `P0` is participant 0's role (const, so its
/// inline handling is not compiled into helpers).
///
/// Each item boundary has one advancer (module docs, "The advance axis"): the completer of a
/// published item publishes the next published item or the END; participant 0 owns every
/// boundary into an inline item and the region's start.
fn run_finisher<W: RegionWords, S: RegionStages, P: RegionPolicy, const P0: bool>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
) {
    let base = part.base;
    let len = w.len();
    let participants = w.participants();
    let poison = w.poison();
    let mut last = base;
    let mut own = None;
    if P0 {
        // The region's start is participant 0's boundary: leading inline items, then the first
        // publish (or the END).
        match p0_boundary::<W, S>(w, stages, part, 0) {
            Some(v) => own = Some(v),
            None => return,
        }
    }
    loop {
        // The value this participant just published itself needs no wait.
        let v = match own.take() {
            Some(v) => v,
            None => match wait_publish_fin::<W, P, P0>(w, last, part) {
                Some(v) => v,
                None => {
                    fin_poisoned::<W, P0>(w, part);
                    return;
                }
            },
        };
        if v & END_BIT != 0 {
            debug_assert_eq!(
                v & !END_BIT,
                base,
                "region: a participant read another region's END (the open reset did not store OPEN)"
            );
            // Relaxed: as in `run_helper` — a poisoned END was published (Release) after the
            // poison store that caused it; a normal END has no poison store to show.
            let exit = if poison.load(Ordering::Relaxed) != 0 {
                RegionExit::Poisoned
            } else {
                RegionExit::End
            };
            part.exit(exit);
            return;
        }
        let i = v.wrapping_sub(base + 1);
        debug_assert!(i < len as u64, "region: publish value {v} is not an epoch of the region based at {base}");
        let item = w.item(i as usize);
        let e = u32::from(item.entry);
        let entry = w.entry(e as usize);
        let n = u32::from(entry.n_blocks);
        let start = part.index * n / participants;
        let x = Exec { e, entry, g: v, hint: hint_of(item, base, i) };
        let completed = sweep_fin::<W, S, P>(w, stages, &x, start, part);
        last = v;
        let next = i as usize + 1;
        let inline_next = next < len && w.entry(usize::from(w.item(next).entry)).n_blocks <= 1;
        if completed && !inline_next {
            // Acquire: pairs with a guard's Release store of `poison` (as before each orchestrator
            // item). A poisoning that lands after this check races the publish below exactly as
            // the orchestrator's check-then-publish does; the survivors' polls end the region.
            if poison.load(Ordering::Acquire) != 0 {
                fin_poisoned::<W, P0>(w, part);
                return;
            }
            own = Some(publish_next(w, part, next));
        } else if P0 && inline_next {
            // The boundary into an inline item is participant 0's. Its own add completed item `i`
            // (AcqRel), or this Acquire load / done-wait reads the exact count: either way every
            // write of item `i` happens-before the inline block. A helper that completed `i`
            // publishes nothing here, so `publish` stays at `g_i` until participant 0 advances.
            let done = w.done(e as usize);
            if !completed && done.load(Ordering::Acquire) != u64::from(n) && !wait_done::<W, P, false>(w, done, u64::from(n), part, v) {
                poisoned_exit(w, part);
                return;
            }
            match p0_boundary::<W, S>(w, stages, part, next) {
                Some(v) => own = Some(v),
                None => return,
            }
        }
    }
}

/// Participant 0 at a boundary into item `j`: runs the inline items from `j` in order (each after
/// a `poison` check), then publishes the next published item or the END. `None` after a poisoned
/// exit. The orchestrator's inline handling, unchanged; the block runs with `part.index`, which is
/// 0 on every path that reaches here.
fn p0_boundary<W: RegionWords, S: RegionStages>(
    w: &W,
    stages: &S,
    part: &mut Participant<'_, W>,
    mut j: usize,
) -> Option<u64> {
    let poison = w.poison();
    while j < w.len() {
        let e = u32::from(w.item(j).entry);
        let n = w.entry(e as usize).n_blocks;
        if n > 1 {
            break;
        }
        // Acquire: pairs with a guard's Release store of `poison`.
        if poison.load(Ordering::Acquire) != 0 {
            poisoned_exit(w, part);
            return None;
        }
        if n == 1 {
            run_block(stages, e, 0, part.index);
            part.blocks += 1;
        }
        j += 1;
    }
    // Acquire: as above.
    if poison.load(Ordering::Acquire) != 0 {
        poisoned_exit(w, part);
        return None;
    }
    Some(publish_next(w, part, j))
}

/// Publishes item `j` (its done line reset first, ruling 17 B1), or the normal END when `j` is the
/// schedule's length; counts the advance and returns the value stored. Both finisher publishers.
#[inline]
fn publish_next<W: RegionWords>(w: &W, part: &mut Participant<'_, W>, j: usize) -> u64 {
    part.advances += 1;
    if j == w.len() {
        let v = END_BIT | part.base;
        // Relaxed: stored once, after the last item completed (by its completer, or by participant
        // 0 after a trailing inline item), and no reader of a normal END reads data: it checks the
        // tag, loads `poison` and stores its receipt (`run_orchestrator`'s END). What the caller
        // reads afterwards is ordered by the scope's join — the edge that, when a helper completes
        // the last item, replaces the orchestrator's done-acquire.
        w.publish().store(v, Ordering::Relaxed);
        return v;
    }
    let item = w.item(j);
    let e = u32::from(item.entry);
    let n = u32::from(w.entry(e as usize).n_blocks);
    let participants = w.participants();
    debug_assert!(
        n <= REGION_MAX_BLOCKS_PER_PARTICIPANT * participants,
        "region: entry {e} has {n} blocks, more than {REGION_MAX_BLOCKS_PER_PARTICIPANT} per \
         participant ({participants} participants)"
    );
    let g = part.base + 1 + j as u64;
    // The reset comes BEFORE the publish (ruling 17 B1), whoever advances. Every add of the
    // entry's previous execution in this region happens-before this store: that execution
    // completed, and the chain of completer adds (AcqRel), publishes (Release / Acquire) and
    // participant 0's done-acquires since then reaches this thread; so the reset follows them in
    // the line's modification order. A previous region's adds are ordered by its join. Relaxed:
    // the Release publish below orders the reset before every Acquire load of `g`, hence before
    // every add of this item.
    w.done(e as usize).store(0, Ordering::Relaxed);
    // Release: every write of the items before `j` (acquired by this thread's completing add,
    // done-acquire or its own blocks) and the reset above happen-before a participant's Acquire
    // load of this epoch.
    w.publish().store(g, Ordering::Release);
    g
}

/// A finisher's wait for a publish value other than `last`, on `P::ORCH_WAIT` for participant 0
/// and `P::HELPER_WAIT` for a helper; `None` when `poison` is set first. Disarmed: no census.
#[inline]
fn wait_publish_fin<W: RegionWords, P: RegionPolicy, const P0: bool>(
    w: &W,
    last: u64,
    part: &mut Participant<'_, W>,
) -> Option<u64> {
    let publish = w.publish();
    // Acquire: pairs with a completer's or participant 0's Release publish of an epoch, and with
    // a poisoned END (the normal END is Relaxed: nothing after it reads data).
    let v = publish.load(Ordering::Acquire);
    if v != last {
        return Some(v);
    }
    let ladder = const { if P0 { P::ORCH_WAIT } else { P::HELPER_WAIT } };
    let clock = const { needs_clock(if P0 { P::ORCH_WAIT } else { P::HELPER_WAIT }, P::BOUND_NS, false) };
    let poison = w.poison();
    let mut waiter = Waiter::start(clock);
    loop {
        // Acquire: pairs with a guard's Release store of `poison`. After a helper's panic nothing
        // is certain to publish an END, so this poll is every survivor's exit. `poison` is the
        // next word of the line this loop already spins on, so the load adds no line traffic.
        if poison.load(Ordering::Acquire) != 0 {
            return None;
        }
        waiter.step::<P, W>(ladder, clock, part, last);
        // Acquire: as above.
        let v = publish.load(Ordering::Acquire);
        if v != last {
            return Some(v);
        }
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
            r.helper_advances += word(R_ADVANCES);
        }
        r.advances += word(R_ADVANCES);
        r.stalls += word(R_STALLS);
        r.max_wait_ns = r.max_wait_ns.max(word(R_MAX_WAIT));
    }
    debug_assert_eq!(
        r.advances,
        u64::from(r.published) + 1,
        "region: Σ receipt advances is not published + 1 (the identity on RegionReport::advances)"
    );
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
    /// no helper ever runs (under either [`Advance`]). A panic in any participant poisons the
    /// region, ends it, and is re-raised here at the join; no item after the poisoned one is
    /// published, except in the check-then-publish window every advancer has (a `poison` check,
    /// then the publish), whose item the survivors' poison filters and polls then end. The
    /// caller's epoch counter is advanced by the schedule's length + 1 before anything else
    /// happens, on every path.
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
                        run_helper_participant::<_, S, P, ARMED>(words, stages, &mut part);
                    });
                }
                run_participant0::<_, S, P, ARMED>(words, stages, &mut orch)
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
                    run_helper_participant::<_, S, P, ARMED>(words, stages, &mut part);
                });
            }
            run_participant0::<_, S, P, ARMED>(words, stages, &mut orch)
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
/// production protocol core ([`open`], the role entries [`run_participant0`] and
/// [`run_helper_participant`], the guards) over atomics loom can see. `cfg(loom)` only;
/// re-exported through `loom_exports::region`.
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
            advances: word(R_ADVANCES),
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
    fn claim(&self, line: usize, word: usize) -> &AtomicU64 {
        assert!(word < self.words_per_line, "loom region frame: claim word {word} is not stored");
        &self.claims[line * self.words_per_line + word]
    }
    #[cfg(debug_assertions)]
    fn claim_lines(&self) -> usize {
        self.claims.len() / self.words_per_line
    }
    #[cfg(debug_assertions)]
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

// =========================================================================
// Unit tests
// =========================================================================

#[cfg(test)]
mod tests {
    //! The hint by value (tester r4 G-HINT). T8 and M-R14 catch a hint that BREAKS exactly-once;
    //! a clamp one step too tight, or a hint that is never used, is still correct and only costs a
    //! CAS retry per first-claimed block, which no count-free gate can see. These live here because
    //! `hint_of` is private to the protocol core.

    use super::{
        Advance, Ladder, RegionPolicy, SchedItem, V2Policy, WithAdvance, claim_lines, hint_of, link_hints,
    };

    /// A home-lines policy with every other axis and ladder off v2's, for (u1).
    struct HomeLines;

    impl RegionPolicy for HomeLines {
        const HELPER_WAIT: Ladder = Ladder::BudgetThenYield { spin_ns: 2_000 };
        const ORCH_WAIT: Ladder = Ladder::BudgetThenYield { spin_ns: 20_000 };
        const BOUND_NS: u64 = 7;
        const DONE_BATCHED: bool = true;
        const HOME_LINES: bool = true;
        const TTAS: bool = true;
    }

    /// `Y` is `X` on every claim and ladder const, and lays out every entry's claim lines as `X`
    /// does at every block count up to 128 and every participant count up to 16.
    fn same_claim_axes<X: RegionPolicy, Y: RegionPolicy>(what: &str) {
        for p in 1..=16u32 {
            for n in 0..=128u16 {
                assert_eq!(claim_lines::<Y>(n, p), claim_lines::<X>(n, p), "{what}: claim_lines({n}, {p})");
            }
        }
        assert_eq!(Y::HELPER_WAIT, X::HELPER_WAIT, "{what}: HELPER_WAIT");
        assert_eq!(Y::ORCH_WAIT, X::ORCH_WAIT, "{what}: ORCH_WAIT");
        assert_eq!(Y::BOUND_NS, X::BOUND_NS, "{what}: BOUND_NS");
        assert_eq!(Y::DONE_BATCHED, X::DONE_BATCHED, "{what}: DONE_BATCHED");
        assert_eq!(Y::HOME_LINES, X::HOME_LINES, "{what}: HOME_LINES");
        assert_eq!(Y::TTAS, X::TTAS, "{what}: TTAS");
    }

    /// (u1) `WithAdvance<X, FIN>` copies every claim and ladder const of `X`, so a claim column
    /// laid out for `X` is laid out for it too (`RegionFrame::new`'s Safety sentence on
    /// alternating frames rests on this).
    #[test]
    fn with_advance_copies_every_claim_and_ladder_const() {
        same_claim_axes::<V2Policy, WithAdvance<V2Policy, false>>("V2Policy, FIN = false");
        same_claim_axes::<V2Policy, WithAdvance<V2Policy, true>>("V2Policy, FIN = true");
        same_claim_axes::<HomeLines, WithAdvance<HomeLines, false>>("HomeLines, FIN = false");
        same_claim_axes::<HomeLines, WithAdvance<HomeLines, true>>("HomeLines, FIN = true");
    }

    /// (u2) The advance axis: defaulted to the orchestrator, replaced by `WithAdvance` only when
    /// `FIN`, and otherwise the wrapped policy's own.
    #[test]
    fn with_advance_sets_only_the_advance_axis() {
        assert_eq!(V2Policy::ADVANCE, Advance::Orchestrator);
        assert_eq!(<WithAdvance<V2Policy, false>>::ADVANCE, Advance::Orchestrator);
        assert_eq!(<WithAdvance<V2Policy, true>>::ADVANCE, Advance::Finisher);
        assert_eq!(<WithAdvance<WithAdvance<V2Policy, true>, false>>::ADVANCE, Advance::Finisher);
        assert_eq!(<WithAdvance<HomeLines, true>>::ADVANCE, Advance::Finisher);
    }

    /// `hint_of` is the epoch of the earlier item `prev_off − 1` when `1 ≤ prev_off ≤ i`, and 0 (no
    /// hint) otherwise; either way it is below the item's own epoch `base + 1 + i`.
    #[test]
    fn hint_of_names_only_an_earlier_item() {
        for base in [1u64, 1_000, 1 << 62] {
            for i in 0..24u64 {
                for off in (0..=60u32).chain([u32::MAX - 1, u32::MAX]) {
                    let item = SchedItem { entry: 0, _r: 0, prev_off: off };
                    let want = if off >= 1 && u64::from(off) <= i { base + u64::from(off) } else { 0 };
                    let got = hint_of(item, base, i);
                    assert_eq!(got, want, "base {base}, item {i}, prev_off {off}");
                    assert!(got < base + 1 + i, "base {base}, item {i}, prev_off {off}: hint {got} not below g");
                }
            }
        }
    }

    /// A schedule linked by `link_hints` hints every repeat execution of an entry with the epoch of
    /// that entry's previous execution (the value its claim words hold, so the first CAS hits), and
    /// writes exactly what `SchedItem::new` builds from the previous item's index.
    #[test]
    fn a_linked_schedule_hints_each_repeat_with_its_previous_execution() {
        let order: [u16; 12] = [0, 1, 0, 2, 1, 0, 0, 2, 1, 1, 0, 2];
        let mut schedule: Vec<SchedItem> = order.iter().map(|&e| SchedItem::new(e, None)).collect();
        // Not zero: `link_hints` zeroes its scratch first.
        let mut last = [u32::MAX; 3];
        link_hints(&mut schedule, &mut last);
        let base = 77u64;
        for (i, item) in schedule.iter().enumerate() {
            let prev = order[..i].iter().rposition(|&e| e == order[i]);
            let prev = prev.map(|j| u32::try_from(j).expect("test: a 12-item schedule"));
            assert_eq!(*item, SchedItem::new(order[i], prev), "item {i}: link_hints and SchedItem::new disagree");
            let want = prev.map_or(0, |j| base + 1 + u64::from(j));
            assert_eq!(hint_of(*item, base, i as u64), want, "item {i} (entry {})", order[i]);
        }
    }

    /// `SchedItem::new` stores the previous item's index + 1, and saturates at `u32::MAX` (a value
    /// no schedule reaches, so it is no hint) instead of wrapping to 0 or overflowing.
    #[test]
    fn sched_item_new_stores_the_offset_and_saturates() {
        assert_eq!(SchedItem::new(3, None).prev_off, 0);
        assert_eq!(SchedItem::new(3, Some(0)).prev_off, 1);
        assert_eq!(SchedItem::new(3, Some(41)).prev_off, 42);
        assert_eq!(SchedItem::new(3, Some(u32::MAX - 1)).prev_off, u32::MAX);
        assert_eq!(SchedItem::new(3, Some(u32::MAX)).prev_off, u32::MAX);
    }
}
