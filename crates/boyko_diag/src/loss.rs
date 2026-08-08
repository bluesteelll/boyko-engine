//! A3 — the **one** loss vocabulary, and the sticky-bit mechanism the mute leaf reports through.
//!
//! # The consequence of not sharing this
//!
//! The profiler reports its own drops *through* the logger, so under load — precisely when
//! profiler drops occur — the report of the loss is itself dropped and counted as a *logger*
//! loss. Two counters double-count one event and no rule says which is authoritative. Sharing
//! makes the report of a profiler drop **a counter read, not a log record**.
//!
//! The vocabulary is this crate's. A consumer's own drop taxonomy maps *onto* [`LossClass`];
//! it does not define it.
//!
//! # The cell is monotone and is NEVER cleared
//!
//! Every counter here is write-only-increasing. There is no `fold_into`, no `store(0)` and no
//! `fetch_sub`, and their absence is the whole correctness argument rather than a simplification.
//!
//! A clearing fold has a lost-update window that `fetch_sub` does **not** close. The producer's
//! increment is `load; add; store`; a consumer `fetch_sub` landing between that load and that
//! store is overwritten by the store, while the value it subtracted has already been folded into
//! the consumer's total — so one loss event is counted twice, which is the exact double-count the
//! `fetch_sub` was introduced to remove.
//!
//! The monotone form has no such window: the consumer never writes the cell at all. Each consumer
//! owns a [`LossSeen`] per `(row, class)` and folds `cur.wrapping_sub(last)` via [`delta_since`].
//! Exactness then follows from the **shape of the datum** — a counter that only ever goes up —
//! and there is no discipline left for a caller to forget. That, not performance, is the decisive
//! reason: under a `fetch_add`-everywhere design, exactness holds only while every producer at
//! every future call site remembers to write `fetch_add`, and one plain `store` silently restores
//! the double-count in the direction that hides work.
//!
//! `wrapping_sub` is exact for as long as fewer than 2^64 increments separate two folds, and the
//! counter never resets, so there is no ABA to reason about.
//!
//! # Ordering, and the one thing that must not be "optimised" later
//!
//! The counters are [`AtomicU64`] accessed [`Relaxed`](Ordering::Relaxed) on **both** sides.
//! The decision record spells them as a plain `u64`, justified as "single-writer, no lock
//! prefix" — but a plain `u64` written by one thread and read by another is a data race and
//! therefore UB in the Rust abstract machine, irrespective of what x86-64 does in hardware, and
//! Miri reports it. `AtomicU64` + `Relaxed` lowers to exactly the same `mov` pair with **no
//! `lock` prefix**, so the single-writer performance argument is preserved verbatim and the UB is
//! removed. There is no trade here; the record's spelling is strictly worse at equal cost.
//!
//! `Relaxed` is the correct strength because the cell is a pure counter and **no data is
//! published through it**. The ordering that matters rides [`raise`] / [`take_raised`].
//!
//! # Accumulate in `u64`, never saturate
//!
//! A saturating `u32` was justified by "an 8-byte RMW is more expensive"; on x86-64 `lock xadd`
//! costs the same at 4 and 8 bytes, so the justification does not survive. Dropping the
//! saturation also removes the `SATURATED(>=4294967295)` census token — a token a reader could
//! never compare against anything, because two saturated counters are equal at the token level
//! and unequal in fact.
//!
//! # Who writes what
//!
//! Cells are indexed `[row][class]`, and a row is a lane. Every producer lane writes **its own**
//! row — single-writer **by the lane topology**, not by convention and not by a lock. That is why
//! [`record_here`] derives the row from the calling thread's own lane and offers no way to name
//! someone else's: a foreign `fetch_add` interleaved with the owner's `load; add; store` is
//! exactly the lost update this module is built to make unrepresentable.
//!
//! One array serves every consumer. Each consumer keeps its own [`LossSeen`] state, so each
//! observes the same deltas; the per-subsystem [`LossTotal`] arrays they fold into are declared
//! by the subsystems, not here.
//!
//! # `.bss` residency, and no boot work
//!
//! `LOSS_ROW_COUNT * LossClass::COUNT * 64 B` = 81 x 8 x 64 = 40.5 KiB, plus 64 B for the
//! padded flag word — in every profile, because [`LANE_COUNT`](crate::lane::LANE_COUNT) has no
//! profile axis. Every static here is all-zero and therefore `.bss`; nothing is written, minted
//! or calibrated at process start, so a process that never enables diagnostics never faults in a
//! page of it and the extent costs address space rather than resident memory.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::lane::LANE_COUNT;

/// The classes of loss both subsystems count, and the only ones either may name.
///
/// A consumer's own taxonomy maps onto these; a consumer that needs a distinction this list
/// cannot express keeps that distinction in its own emitter, not by widening this enum — the
/// value of the vocabulary is that two artifacts a reader joins use the same eight words.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LossClass {
    /// A fixed-capacity producer buffer was full; the event was discarded at the producer.
    Overflow = 0,
    /// The producing thread held no lane, so the event could not be attributed to one.
    Unclaimed = 1,
    /// The event arrived after its window had closed and the consumer discarded it.
    Late = 2,
    /// A gate — an arm mask, a level, a filter — rejected the event before it was recorded.
    Refused = 3,
    /// The loss happened off-CPU (a GPU query pool, a driver ring) and the host learnt of it
    /// only afterwards, so the count is reconstructed rather than observed at the drop.
    Device = 4,
    /// The drain-side destination rejected, short-wrote or truncated the write.
    Sink = 5,
    /// Records were discarded because a destination was rotated or reopened underneath them.
    Rotation = 6,
    /// A configured per-window quota was reached and the remainder of the window was dropped.
    Budget = 7,
}

impl LossClass {
    /// How many classes exist. Cell arrays and `last_seen` arrays are sized by this.
    pub const COUNT: usize = 8;

    /// Every class, in discriminant order, so a fold can iterate without a `from_index` round
    /// trip. The const assertion below pins its length to [`COUNT`](Self::COUNT).
    pub const ALL: [LossClass; Self::COUNT] = [
        LossClass::Overflow,
        LossClass::Unclaimed,
        LossClass::Late,
        LossClass::Refused,
        LossClass::Device,
        LossClass::Sink,
        LossClass::Rotation,
        LossClass::Budget,
    ];

    /// The class's index into a per-row cell array.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The class at `index`, or `None` if `index` names no class.
    ///
    /// Returns an `Option` rather than panicking because the leaf is mute: a consumer decoding a
    /// stored index must be able to reject it without taking the process down.
    #[inline]
    pub const fn from_index(index: usize) -> Option<LossClass> {
        match index {
            0 => Some(LossClass::Overflow),
            1 => Some(LossClass::Unclaimed),
            2 => Some(LossClass::Late),
            3 => Some(LossClass::Refused),
            4 => Some(LossClass::Device),
            5 => Some(LossClass::Sink),
            6 => Some(LossClass::Rotation),
            7 => Some(LossClass::Budget),
            _ => None,
        }
    }

    /// The class's census token.
    ///
    /// A `&'static str` table rather than a `Display` impl: the mute-leaf rule forbids this crate
    /// a format string, and an emitter that wants one word per class should not have to pay
    /// `core::fmt` to get it.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            LossClass::Overflow => "Overflow",
            LossClass::Unclaimed => "Unclaimed",
            LossClass::Late => "Late",
            LossClass::Refused => "Refused",
            LossClass::Device => "Device",
            LossClass::Sink => "Sink",
            LossClass::Rotation => "Rotation",
            LossClass::Budget => "Budget",
        }
    }
}

/// How much a reported figure is worth — the **one** status vocabulary shared with the profiler,
/// so a reader who learnt the tokens in one artifact has learnt them in the other.
///
/// Four of the five tokens are flavours of "unproven", and that is deliberate: a census that
/// prints one `Unproven` for four different reasons tells a reader nothing about which figure to
/// re-measure and how.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LossStatus {
    /// The figure is a direct measurement of the quantity named. The only token that asserts
    /// anything.
    Measured,
    /// No measurement was made. The figure is not asserted and must not be compared.
    Unproven,
    /// Events were dropped on the path that produced the figure, so it is a lower bound.
    UnprovenLossy,
    /// The figure is extrapolated from a sample rather than observed over every event.
    UnprovenSampled,
    /// Events were recorded but had not reached a sink when the figure was taken, so it omits an
    /// unknown tail.
    UnprovenUnsunk,
}

impl LossStatus {
    /// Whether this status asserts the figure. Exactly one token does.
    #[inline]
    pub const fn is_measured(self) -> bool {
        matches!(self, LossStatus::Measured)
    }

    /// The status's census token. A `&'static str` table for the reason given on
    /// [`LossClass::as_str`].
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            LossStatus::Measured => "Measured",
            LossStatus::Unproven => "Unproven",
            LossStatus::UnprovenLossy => "UnprovenLossy",
            LossStatus::UnprovenSampled => "UnprovenSampled",
            LossStatus::UnprovenUnsunk => "UnprovenUnsunk",
        }
    }
}

/// One `(row, class)` counter pair, on its own cache line.
///
/// Monotone and never cleared. The 48 B of padding stops **cross-thread** false sharing between
/// rows; between the eight classes of one row it buys nothing, because one row has one writer.
/// That is a known 4x line-count cost on the fold, recorded as substrate Q3 and deliberately not
/// taken here, because taking it changes a type the decision record declares.
#[repr(C, align(64))]
pub struct LossCell {
    /// Events lost. Written only by the row's owner, via `load; add; store` `Relaxed`.
    count: AtomicU64,
    /// Payload bytes lost with them, for the classes where a byte figure is meaningful.
    bytes: AtomicU64,
    /// Padding to a full cache line. Zero, so the whole array lands in `.bss`.
    _pad: [u8; 48],
}

impl LossCell {
    /// An all-zero cell. The only way to make one, so every array of them is `.bss`.
    ///
    /// A `const fn` rather than an associated `const`, and the difference is not cosmetic: a
    /// named `const` holding an `AtomicU64` is **substituted at every use site**, so
    /// `LossCell::ZERO.add(1)` would increment a temporary and the write would vanish with no
    /// diagnostic. `clippy::declare_interior_mutable_const` exists for exactly that footgun and
    /// fires on the `const` form. A `const fn` has the same `.bss` property — the call is
    /// evaluated at compile time in a `static` initialiser — and offers no site to misuse.
    #[inline]
    #[must_use]
    pub const fn zero() -> LossCell {
        LossCell {
            count: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            _pad: [0u8; 48],
        }
    }

    /// The monotone event count. Prefer [`delta_since`] — a bare read cannot be folded exactly.
    #[inline]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// The monotone byte count. Prefer [`delta_since`].
    #[inline]
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Bump a cell **this thread owns**: a non-atomic `load; add; store` pair with no `lock`
    /// prefix, which is sound only because the lane topology gives the row exactly one writer.
    ///
    /// Deliberately `pub(crate)`. Exposing it would hand every future caller the obligation to
    /// know whether it owns the row, and one caller getting that wrong reintroduces the lost
    /// update this module exists to make unrepresentable. [`record_here`] decides for them.
    #[inline]
    pub(crate) fn bump_owned(&self, bytes: u64) {
        // `wrapping_add` rather than `+`: an overflow panic in the leaf would turn a counting
        // error into a process kill, and the fold's `wrapping_sub` is exact across the wrap.
        self.count
            .store(self.count.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
        self.bytes
            .store(self.bytes.load(Ordering::Relaxed).wrapping_add(bytes), Ordering::Relaxed);
    }

    /// Bump a cell with **more than one** writer — only the unlaned row qualifies. `fetch_add`
    /// costs a `lock xadd` and is the price of that row being shared by every thread the lane
    /// topology could not place.
    #[inline]
    pub(crate) fn bump_shared(&self, bytes: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// A consumer's per-class accumulator, folded from many rows' deltas.
///
/// Declared by each subsystem — `static TOTALS: [LossTotal; LossClass::COUNT]` — never by this
/// crate, so the bytes are attributed to the subsystem that reads them.
#[repr(C, align(64))]
pub struct LossTotal {
    /// Summed event count.
    count: AtomicU64,
    /// Summed byte count.
    bytes: AtomicU64,
    /// Padding to a full cache line; a total is written by its consumer and read by a census
    /// thread, so the sharing this prevents is real.
    _pad: [u8; 48],
}

impl LossTotal {
    /// An all-zero total, for the same `.bss` reason — and the same interior-mutability
    /// reason — as [`LossCell::zero`].
    #[inline]
    #[must_use]
    pub const fn zero() -> LossTotal {
        LossTotal {
            count: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            _pad: [0u8; 48],
        }
    }

    /// Fold one [`delta_since`] result in.
    ///
    /// `fetch_add`, not the cell's cheaper `load; add; store` — a total is written once per fold
    /// rather than once per event, so the RMW is unmeasurable here, and paying it buys the total
    /// the right to have more than one writer without a new rule anyone has to remember.
    #[inline]
    pub fn add(&self, delta: LossDelta) {
        self.count.fetch_add(delta.count, Ordering::Relaxed);
        self.bytes.fetch_add(delta.bytes, Ordering::Relaxed);
    }

    /// Total events folded so far.
    #[inline]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Total bytes folded so far.
    #[inline]
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

/// What a consumer last saw in one [`LossCell`] — the whole of the consumer-side fold state.
///
/// **Owned by the consumer, not by this crate.** A consumer declares
/// `[[LossSeen; LossClass::COUNT]; LOSS_ROW_COUNT]` and passes one entry to [`delta_since`]. That
/// is why the cell needs no clear: the state that makes the fold exact lives with the reader, and
/// two readers of the same cell therefore do not interfere.
///
/// The fields are public because there is no invariant to protect — this is the consumer's own
/// bookkeeping, and the decision record spells it as a bare array of `u64`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LossSeen {
    /// The `count` observed at the previous fold.
    pub count: u64,
    /// The `bytes` observed at the previous fold.
    pub bytes: u64,
}

impl LossSeen {
    /// The starting state: nothing seen yet, so the first fold yields everything recorded so far.
    pub const ZERO: LossSeen = LossSeen { count: 0, bytes: 0 };
}

/// What one cell accrued between two folds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LossDelta {
    /// Events lost since the previous fold.
    pub count: u64,
    /// Bytes lost with them since the previous fold.
    pub bytes: u64,
}

impl LossDelta {
    /// A delta that accrued nothing.
    pub const ZERO: LossDelta = LossDelta { count: 0, bytes: 0 };

    /// Whether nothing was lost in this window — the overwhelmingly common case, and the one an
    /// emitter should say nothing at all about.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.count == 0 && self.bytes == 0
    }
}

/// The row index reserved for producers the lane topology could not place.
///
/// `claim_lane` exhaustion is non-terminal: the caller stays
/// [`LANE_UNCLAIMED`](crate::lane::LANE_UNCLAIMED) and its losses are still counted, here, under
/// [`LossClass::Unclaimed`] or whatever class the producer names. Dropping them instead would
/// make the one condition a reader most needs to see the one condition the system cannot report.
///
/// This row is the single exception to single-writer ownership and is therefore the only one
/// bumped with `fetch_add`.
pub const ROW_UNLANED: usize = LANE_COUNT as usize;

/// How many rows a consumer's `last_seen` array must have: one per lane, plus [`ROW_UNLANED`].
pub const LOSS_ROW_COUNT: usize = ROW_UNLANED + 1;

/// Every lane's counters, plus the unlaned row. All-zero, hence `.bss`, hence untouched until a
/// loss actually occurs.
///
/// The inline `const` blocks are what make the nested repeat legal without naming an
/// interior-mutable `const`: a repeat operand must be a `const` item or a `Copy` value, and
/// `LossCell` is neither. `[const { … }; N]` supplies the const-ness at the expression, so the
/// earlier `ZERO_ROW` indirection — which existed only to give the outer repeat something to
/// copy — is unnecessary, and with it goes the `declare_interior_mutable_const` it tripped.
/// Verified against rustc 1.95.0, edition 2024.
static CELLS: [[LossCell; LossClass::COUNT]; LOSS_ROW_COUNT] =
    [const { [const { LossCell::zero() }; LossClass::COUNT] }; LOSS_ROW_COUNT];

/// The row a lane's losses land in: the lane itself when the topology placed it, otherwise
/// [`ROW_UNLANED`].
///
/// Total by construction, so no producer path has an error branch to get wrong.
#[inline]
pub const fn row_of(lane: u16) -> usize {
    let row = lane as usize;
    if row < LANE_COUNT as usize { row } else { ROW_UNLANED }
}

/// The cell a given lane's losses of `class` accumulate in.
#[inline]
pub fn cell(lane: u16, class: LossClass) -> &'static LossCell {
    &CELLS[row_of(lane)][class.index()]
}

/// The cell at an explicit row, for a consumer folding `0..LOSS_ROW_COUNT` against its own
/// `last_seen` array.
#[inline]
pub fn cell_at_row(row: usize, class: LossClass) -> &'static LossCell {
    debug_assert!(row < LOSS_ROW_COUNT, "invariant: fold row is in 0..LOSS_ROW_COUNT");
    &CELLS[row][class.index()]
}

/// Record one loss of `class`, costing `bytes`, against the **calling thread's own** lane.
///
/// This is the only way to write a cell, and it takes no lane argument on purpose. The row is
/// derived from the caller's own [`lane()`](crate::lane::lane), so the single-writer property
/// that licenses the cheap non-atomic bump holds by construction rather than by every caller
/// remembering whose row it is allowed to touch.
///
/// Not `#[cold]`. A loss is a degraded path but not necessarily a rare one — `Overflow` is rare
/// by definition, while `Late` and `Refused` can be systematically high under a misconfigured
/// window, which is precisely when mispredicting every one of them would be least welcome.
///
/// `bytes` is `0` for classes where no payload figure is meaningful.
#[inline]
pub fn record_here(class: LossClass, bytes: u64) {
    let row = row_of(crate::lane::lane());
    let target = &CELLS[row][class.index()];
    if row == ROW_UNLANED {
        target.bump_shared(bytes);
    } else {
        target.bump_owned(bytes);
    }
}

/// Fold one cell: return what it accrued since `last` saw it, and advance `last`.
///
/// The cell is **not** modified — that is the entire point. `last` is the caller's state, so two
/// consumers folding the same cell each get the full delta and neither steals from the other.
///
/// Two honest limits:
///
/// - `count` and `bytes` are read as two independent `Relaxed` loads, so a producer bumping
///   between them can put an event's count in this delta and its bytes in the next. Nothing is
///   lost or double-counted; the pair is only *eventually* consistent, and an emitter that
///   divides one by the other in a single window can be off by one event's worth.
/// - `wrapping_sub` is exact unless 2^64 events separated two folds, which no process reaches.
#[inline]
pub fn delta_since(cell: &LossCell, last: &mut LossSeen) -> LossDelta {
    let count = cell.count.load(Ordering::Relaxed);
    let bytes = cell.bytes.load(Ordering::Relaxed);
    let delta = LossDelta {
        count: count.wrapping_sub(last.count),
        bytes: bytes.wrapping_sub(last.bytes),
    };
    last.count = count;
    last.bytes = bytes;
    delta
}

/// A condition the mute leaf observed and cannot itself report.
///
/// The leaf emits no code, so it raises a sticky bit and bumps the paired counter; an emitter
/// above the leaf calls [`take_raised`] at its next fold, maps each set bit to a registry code
/// and prints it **with the counter's value**. The flag says *that* it happened; only the counter
/// says *how many times*, and an emitter that prints one without the other has thrown away the
/// magnitude.
///
/// **Q4 — the flag-to-code table — is RESOLVED**, and it lives in
/// `boyko_ecs::ecs::core::profiling::diag`, which is the only caller of [`take_raised`] in the
/// tree. Bits are assigned here as the emitter gains a row for them and **not before**: an
/// unreported flag is a condition the system observes and cannot say, which is the failure the
/// mute-leaf rule accepts once, at the leaf, and must not accumulate above it.
///
/// Five assigned, 27 unassigned.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiagFlag {
    /// The clock's epoch advanced: a forward jump was observed, so timestamps either side of it
    /// are not comparable and the window spanning it must be quarantined.
    ClockEpochBreak = 1 << 0,
    /// A timestamp was read before the clock was calibrated, so its scale is unknown.
    ClockUncalibrated = 1 << 1,
    /// `claim_lane` found every spare taken; the caller runs unlaned and its losses land in
    /// [`ROW_UNLANED`].
    LaneExhausted = 1 << 2,
    /// The engine zone registry handed out its last slot; further zones run unregistered, and
    /// their samples carry no id a reader can resolve to a name.
    ZoneRegistryExhausted = 1 << 3,
    /// The engine zone registry crossed 90 % occupancy. Nothing is lost yet — this is the warning
    /// that exists so exhaustion is not the first news of it.
    ZoneRegistryNearFull = 1 << 4,
}

impl DiagFlag {
    /// The flag's bit within the word [`take_raised`] returns.
    #[inline]
    pub const fn as_bits(self) -> u32 {
        self as u32
    }
}

/// The raised-condition word, alone on its cache line.
///
/// Deliberately **not** a field of the clock's globals: `raise` dirties this line, and a dirtied
/// line shared with the clock would invalidate, on every raise, a line that every hot reader
/// touches.
#[repr(C, align(64))]
struct FlagLine {
    /// The sticky bits.
    bits: AtomicU32,
    /// Padding to a full cache line. Zero, so this lands in `.bss` like everything else here.
    _pad: [u8; 60],
}

/// The process-wide raised set. `.bss`, so it is live from process start — a condition raised
/// before any emitter exists is still reported at the first fold, which is strictly better than
/// "boot the emitter earlier", a rule no host can be made to keep.
static DIAG_FLAGS: FlagLine = FlagLine { bits: AtomicU32::new(0), _pad: [0u8; 60] };

/// Raise `flag`. N occurrences raise one bit; the count lives in the paired [`LossCell`].
///
/// `fetch_or`, not `store`: a `store` of the bit would clobber a concurrent raiser's bit, and the
/// conditions worth raising are exactly the ones several threads hit at once.
///
/// `Release` pairs with the `Acquire` in [`take_raised`]. `Relaxed` would suffice *for the bit
/// alone*, but every raise is paired with a counter increment whose value the emitter prints, and
/// the pairing is what guarantees a consumer that observes the bit also observes the counter. The
/// cost is zero on x86-64; the pairing, not the ISA, is the reason it is written this way.
#[inline]
pub fn raise(flag: DiagFlag) {
    DIAG_FLAGS.bits.fetch_or(flag.as_bits(), Ordering::Release);
}

/// Take and clear every raised bit, returning them as a word.
///
/// `swap(0)` — **not** `load` then `store(0)` — is what makes the take exact: a raise landing
/// between a separate load and clear would be silently dropped, which is the same lost-update
/// defect class the monotone cell exists to rule out.
///
/// A condition raised after the final take — during teardown, or on a crash path after the drain
/// — is never reported. That is a real cost of the mute-leaf rule and is named here rather than
/// discovered later.
#[inline]
pub fn take_raised() -> u32 {
    DIAG_FLAGS.bits.swap(0, Ordering::Acquire)
}

// Layout facts the rest of the design leans on. Const assertions rather than tests: a build that
// violates one must not produce a binary a test could then be skipped on.
const _: () = assert!(size_of::<LossCell>() == 64);
const _: () = assert!(align_of::<LossCell>() == 64);
const _: () = assert!(size_of::<LossTotal>() == 64);
const _: () = assert!(align_of::<LossTotal>() == 64);
const _: () = assert!(size_of::<FlagLine>() == 64);
const _: () = assert!(LossClass::ALL.len() == LossClass::COUNT);
const _: () = assert!(LOSS_ROW_COUNT == LANE_COUNT as usize + 1);

#[cfg(test)]
mod tests {
    // Scoped threads throughout, never the free-standing spawn: the fixtures borrow locals
    // instead of requiring `'static`, joining cannot be forgotten — and DG9's mute-leaf scan
    // greps this directory for the free-standing spelling as bytes, with no way to evaluate a
    // `#[cfg(test)]`, so a fixture using it would red the gate on a correct implementation.
    use std::thread;

    use super::*;

    /// Bumping and folding a **local** cell keeps these assertions exact. The `static CELLS`
    /// array and `DIAG_FLAGS` are process-global, so any test asserting an exact figure over them
    /// would be racing every other test in the binary — including the sibling modules'.
    fn local_cell() -> LossCell {
        LossCell::zero()
    }

    #[test]
    fn class_indices_are_dense_and_round_trip() {
        for (i, class) in LossClass::ALL.iter().copied().enumerate() {
            assert!(class.index() == i, "ALL is out of discriminant order");
            assert!(LossClass::from_index(i) == Some(class));
            assert!(!class.as_str().is_empty());
        }
        assert!(LossClass::from_index(LossClass::COUNT).is_none());
    }

    #[test]
    fn class_tokens_are_distinct() {
        for (i, a) in LossClass::ALL.iter().copied().enumerate() {
            for b in LossClass::ALL.iter().copied().skip(i + 1) {
                assert!(a.as_str() != b.as_str(), "two classes share one census token");
            }
        }
    }

    #[test]
    fn only_measured_asserts_a_figure() {
        let all = [
            LossStatus::Measured,
            LossStatus::Unproven,
            LossStatus::UnprovenLossy,
            LossStatus::UnprovenSampled,
            LossStatus::UnprovenUnsunk,
        ];
        assert!(all.iter().filter(|s| s.is_measured()).count() == 1);
        for (i, a) in all.iter().copied().enumerate() {
            for b in all.iter().copied().skip(i + 1) {
                assert!(a.as_str() != b.as_str(), "two statuses share one census token");
            }
        }
    }

    #[test]
    fn row_of_places_lanes_and_catches_everything_else() {
        // Every lane gets its own row: that is what makes the row single-writer.
        for lane in 0..LANE_COUNT {
            assert!(row_of(lane) == lane as usize);
        }
        // And nothing else does, including the sentinel and every value above the topology.
        assert!(row_of(LANE_COUNT) == ROW_UNLANED);
        assert!(row_of(crate::lane::LANE_UNCLAIMED) == ROW_UNLANED);
        // NOT asserted, because it is unobservable: `<` versus `<=` on this bound. `ROW_UNLANED`
        // IS `LANE_COUNT`, so both spellings map `LANE_COUNT` to the same row (verified by
        // mutation: the `<=` mutant survives every test here, and is equivalent, not missed).
    }

    #[test]
    fn record_here_counts_a_laned_thread_on_its_own_row() {
        // The production write path, end to end: a thread the topology placed bumps its OWN row
        // through the cheap non-atomic branch. Lane 42 x Rotation is a coordinate no other test
        // in this crate touches, so the figure is exact rather than a lower bound.
        const LANE: u16 = 42;
        let cell = cell_at_row(LANE as usize, LossClass::Rotation);
        let mut seen = LossSeen { count: cell.count(), bytes: cell.bytes() };
        thread::scope(|scope| {
            scope.spawn(|| {
                crate::lane::set_lane(LANE);
                record_here(LossClass::Rotation, 5);
                record_here(LossClass::Rotation, 6);
            });
        });
        let d = delta_since(cell, &mut seen);
        assert!(d.count == 2 && d.bytes == 11, "record_here did not reach the lane's own row");
        // The neighbouring class must be untouched, or one class's losses would hide another's.
        assert!(cell_at_row(LANE as usize, LossClass::Sink).count() == 0);
    }

    #[test]
    fn delta_since_returns_the_increment_and_leaves_the_cell_alone() {
        let cell = local_cell();
        let mut seen = LossSeen::ZERO;

        assert!(delta_since(&cell, &mut seen) == LossDelta::ZERO);

        cell.bump_owned(10);
        cell.bump_owned(20);
        let d = delta_since(&cell, &mut seen);
        assert!(d.count == 2 && d.bytes == 30);
        // The monotone invariant: the fold did not decrease the cell.
        assert!(cell.count() == 2 && cell.bytes() == 30);

        // A second fold with no producer activity yields nothing, and still does not clear.
        assert!(delta_since(&cell, &mut seen).is_empty());
        assert!(cell.count() == 2 && cell.bytes() == 30);
    }

    #[test]
    fn two_consumers_of_one_cell_do_not_steal_from_each_other() {
        let cell = local_cell();
        let (mut a, mut b) = (LossSeen::ZERO, LossSeen::ZERO);

        cell.bump_shared(4);
        assert!(delta_since(&cell, &mut a).count == 1);
        // A clearing fold would have left nothing here; the monotone fold gives b the same event.
        assert!(delta_since(&cell, &mut b).count == 1);
    }

    #[test]
    fn delta_is_exact_across_the_counter_wrap() {
        let cell = local_cell();
        cell.bump_owned(1);
        cell.bump_owned(1);
        cell.bump_owned(1);
        let mut seen = LossSeen { count: u64::MAX, bytes: u64::MAX };
        let d = delta_since(&cell, &mut seen);
        assert!(d.count == 4 && d.bytes == 4, "wrapping_sub is not exact across the wrap");
    }

    #[test]
    fn folded_deltas_sum_to_the_injected_count_under_a_live_producer() {
        // DG5's shape: inject N from a live producer while a consumer folds repeatedly, and
        // assert the folded total is exactly N and the cell was never decreased.
        const N: u64 = 20_000;
        let cell = local_cell();
        let total = LossTotal::zero();
        let mut seen = LossSeen::ZERO;

        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..N {
                    cell.bump_owned(2);
                }
            });
            let mut low_water = 0u64;
            while total.count() < N {
                let d = delta_since(&cell, &mut seen);
                assert!(seen.count >= low_water, "the cell went backwards");
                low_water = seen.count;
                total.add(d);
            }
        });

        // Drain whatever landed after the loop's last read.
        total.add(delta_since(&cell, &mut seen));
        assert!(total.count() == N, "folded count is not exactly the injected count");
        assert!(total.bytes() == 2 * N);
        assert!(cell.count() == N, "the fold mutated the cell");
    }

    #[test]
    fn record_here_counts_an_unlaned_thread_on_the_unlaned_row() {
        // A freshly spawned thread has claimed no lane, so this lands in ROW_UNLANED. The figure
        // is a lower bound, not an equality: CELLS is process-global and any other test may be
        // recording concurrently. It still fails if `record_here` records nothing.
        let cell = cell_at_row(ROW_UNLANED, LossClass::Unclaimed);
        let mut seen = LossSeen { count: cell.count(), bytes: cell.bytes() };
        thread::scope(|scope| {
            scope.spawn(|| {
                assert!(crate::lane::lane() == crate::lane::LANE_UNCLAIMED);
                record_here(LossClass::Unclaimed, 9);
            });
        });
        let d = delta_since(cell, &mut seen);
        assert!(d.count >= 1 && d.bytes >= 9, "record_here did not reach the unlaned row");
    }

    #[test]
    fn cell_and_cell_at_row_name_the_same_storage() {
        for class in LossClass::ALL {
            assert!(core::ptr::eq(cell(7, class), cell_at_row(7, class)));
            assert!(core::ptr::eq(
                cell(crate::lane::LANE_UNCLAIMED, class),
                cell_at_row(ROW_UNLANED, class)
            ));
        }
        // Distinct classes must not alias, or one class's losses would hide another's.
        assert!(!core::ptr::eq(cell(7, LossClass::Late), cell(7, LossClass::Sink)));
    }

    #[test]
    fn diag_flag_bits_are_distinct_single_bits() {
        let all = [DiagFlag::ClockEpochBreak, DiagFlag::ClockUncalibrated, DiagFlag::LaneExhausted];
        let mut union = 0u32;
        for f in all {
            let bits = f.as_bits();
            assert!(bits.is_power_of_two(), "a DiagFlag is not a single bit");
            assert!(union & bits == 0, "two DiagFlags share a bit");
            union |= bits;
        }
    }

    #[test]
    fn concurrent_raises_all_survive_one_take() {
        // DIAG_FLAGS is process-global, so this test asserts only about the bits it raised and
        // never that the word is otherwise empty. It is the ONLY test here that calls
        // `take_raised`; a second one would steal this one's bits.
        let a = DiagFlag::ClockEpochBreak.as_bits();
        let b = DiagFlag::LaneExhausted.as_bits();
        let mine = a | b;

        std::thread::scope(|scope| {
            scope.spawn(|| raise(DiagFlag::ClockEpochBreak));
            scope.spawn(|| raise(DiagFlag::LaneExhausted));
        });

        // A `store`-based raise would have let one thread clobber the other's bit.
        let taken = take_raised();
        assert!(taken & mine == mine, "a concurrent raise was lost");

        // The take is exact: what it returned, it also cleared.
        assert!(take_raised() & mine == 0, "take_raised did not clear what it returned");
    }
}
