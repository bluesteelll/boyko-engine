//! Per-sink runtime policy: state, level floor, and target filter *(L14)*.
//!
//! Disposition X8 moved sinks from "boot-published, never mutated after boot" to a runtime object.
//! The **kind** stays boot-fixed — a console sink does not become a file sink — but three fields
//! become writable from any thread at any time:
//!
//! - **`state`**: `Off` / `Active` / `Paused`. A paused sink keeps its destination open and stops
//!   receiving; closing and reopening a file to mute it for ten seconds would rotate away the
//!   context the operator paused it to preserve.
//! - **`floor`**: the sink's own minimum level, independent of any target's ceiling. A file
//!   capturing everything and a console showing only warnings is the ordinary case, and without a
//!   per-sink floor it requires two ceilings that fight each other.
//! - **`filter`**: a 256-bit target mask, one bit per [`TargetId`](crate::TargetId) index.
//!
//! # Why plain byte stores and not a lock
//!
//! Every field is a `Relaxed` atomic written directly by the setter's caller. There is no request
//! ring here and no `OUT_LOCK`, because **none of these operations can block, allocate or
//! syscall** — that is exactly the line [`super::request`] exists to hold, and policy is on the
//! other side of it. Only *lifecycle* (open, close, retarget) is a syscall, so only lifecycle is
//! posted.
//!
//! # The one-drain staleness, stated rather than hidden
//!
//! A sink reads its policy **at the top of its current drain** and acts on that reading for the
//! whole pass. A change therefore lands within one drain, not instantly. G13 pins this as a
//! property rather than letting it be discovered: re-reading the filter per record would make the
//! boundary sharper and would also mean one record in a batch going to a destination its
//! predecessor did not, which is harder to explain to a reader of the file than "the change took
//! effect at the next drain".
//!
//! # Defaults are permissive, and that is load-bearing
//!
//! A fresh slot is `Active`, floor `Trace`, all 256 target bits set. Every rung below L14 wrote
//! records with no notion of a sink filter, and a restrictive default would have made this module's
//! arrival delete them — a diagnostics subsystem that silences the previous eleven rungs' output as
//! a side effect of gaining a policy field is the failure this whole campaign is about.

use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::TargetId;
use crate::level::Level;
use crate::target::MAX_TARGETS;

/// Number of sink slots: console, file, ECS ring, binary.
///
/// Fixed and small, because the set of destination KINDS is fixed — X8 keeps kind boot-fixed and
/// makes only policy runtime. A growable slot table would be an allocation on a path whose whole
/// contract is that it does not allocate.
pub const SINK_SLOTS: usize = 4;

/// Words in one slot's target filter: one bit per target index.
const FILTER_WORDS: usize = MAX_TARGETS / 64;

/// Slot index of the console sink.
pub const SLOT_CONSOLE: usize = 0;
/// Slot index of the file sink.
pub const SLOT_FILE: usize = 1;
/// Slot index of the ECS ring.
pub const SLOT_ECS: usize = 2;
/// Slot index of the binary sink.
pub const SLOT_BINARY: usize = 3;

/// What a sink is currently doing.
///
/// `Paused` is a third state and not `Off`, because the two differ in what happens to the
/// destination: `Off` releases it, `Paused` holds it. An operator muting a noisy capture for a
/// minute wants the file they already have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SinkState {
    /// Not receiving; the destination is released.
    Off = 0,
    /// Receiving.
    Active = 1,
    /// Not receiving; the destination is held open.
    Paused = 2,
}

impl SinkState {
    /// Reconstruct from the stored byte. Any unknown byte reads as `Off`.
    ///
    /// Unknown-reads-as-`Off` and not `Active`: a corrupted or partially-written policy byte that
    /// defaulted to receiving would send records to a destination whose state nobody established.
    #[must_use]
    const fn from_raw(b: u8) -> SinkState {
        match b {
            1 => SinkState::Active,
            2 => SinkState::Paused,
            _ => SinkState::Off,
        }
    }

    /// The state's name, for a record or a census row.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            SinkState::Off => "off",
            SinkState::Active => "active",
            SinkState::Paused => "paused",
        }
    }
}

/// One sink's runtime policy.
struct SinkSlot {
    /// A [`SinkState`] discriminant.
    state: AtomicU8,
    /// A [`Level`] discriminant: records below it are not delivered to this sink.
    floor: AtomicU8,
    /// One bit per target index; a set bit accepts.
    filter: [AtomicU64; FILTER_WORDS],
}

impl SinkSlot {
    const fn new() -> SinkSlot {
        SinkSlot {
            // `Active` and all-ones, so that arriving at L14 does not delete L0..L13's output.
            state: AtomicU8::new(SinkState::Active as u8),
            floor: AtomicU8::new(Level::Trace as u8),
            filter: [const { AtomicU64::new(u64::MAX) }; FILTER_WORDS],
        }
    }
}

/// The slot table. `.bss` — every field is zero or a `const` initialiser, so an unenabled process
/// pays no page for it beyond the reservation.
static SLOTS: [SinkSlot; SINK_SLOTS] = [const { SinkSlot::new() }; SINK_SLOTS];

/// Set a sink's state. Out-of-range slots are ignored.
///
/// Callable from any thread at any time: a byte store cannot block, allocate or syscall, which is
/// what separates policy from the lifecycle verbs in [`super::request`].
pub fn set_state(slot: usize, state: SinkState) {
    if let Some(s) = SLOTS.get(slot) {
        s.state.store(state as u8, Ordering::Relaxed);
    }
}

/// Read a sink's state. An out-of-range slot reads as [`SinkState::Off`].
#[must_use]
pub fn state(slot: usize) -> SinkState {
    SLOTS.get(slot).map_or(SinkState::Off, |s| SinkState::from_raw(s.state.load(Ordering::Relaxed)))
}

/// Set a sink's level floor. Out-of-range slots are ignored.
pub fn set_floor(slot: usize, floor: Level) {
    if let Some(s) = SLOTS.get(slot) {
        s.floor.store(floor as u8, Ordering::Relaxed);
    }
}

/// Read a sink's level floor. An out-of-range slot reads as [`Level::Off`].
#[must_use]
pub fn floor(slot: usize) -> Level {
    SLOTS.get(slot).map_or(Level::Off, |s| Level::from_raw(s.floor.load(Ordering::Relaxed)))
}

/// Admit or exclude one target on one sink.
pub fn set_target(slot: usize, target: TargetId, accept: bool) {
    let Some(s) = SLOTS.get(slot) else { return };
    let idx = target.index() as usize;
    let (word, bit) = (idx / 64, idx % 64);
    let Some(w) = s.filter.get(word) else { return };
    if accept {
        w.fetch_or(1u64 << bit, Ordering::Relaxed);
    } else {
        w.fetch_and(!(1u64 << bit), Ordering::Relaxed);
    }
}

/// Set a sink's filter to exactly one target, excluding every other.
///
/// The common console operation — "show me only the physics target" — expressed as one call, so a
/// caller does not clear 255 bits one at a time and race a concurrent reader through 255 partial
/// states, each of which is a filter somebody may drain against.
pub fn set_only_target(slot: usize, target: TargetId) {
    let Some(s) = SLOTS.get(slot) else { return };
    let idx = target.index() as usize;
    for (i, w) in s.filter.iter().enumerate() {
        let value = if i == idx / 64 { 1u64 << (idx % 64) } else { 0 };
        w.store(value, Ordering::Relaxed);
    }
}

/// Admit every target on a sink: the default, and the way back from [`set_only_target`].
pub fn set_all_targets(slot: usize) {
    let Some(s) = SLOTS.get(slot) else { return };
    for w in &s.filter {
        w.store(u64::MAX, Ordering::Relaxed);
    }
}

/// Whether this sink's filter admits this target, ignoring state and floor.
#[must_use]
pub fn filter_admits(slot: usize, target: TargetId) -> bool {
    let Some(s) = SLOTS.get(slot) else { return false };
    let idx = target.index() as usize;
    s.filter.get(idx / 64).is_some_and(|w| w.load(Ordering::Relaxed) & (1u64 << (idx % 64)) != 0)
}

/// Whether this sink would deliver a record on `target` at `level`.
///
/// All three fields at once, because they are three ways for the same record to not arrive and a
/// caller checking one of them has checked nothing.
#[must_use]
pub fn accepts(slot: usize, target: TargetId, level: Level) -> bool {
    state(slot) == SinkState::Active && level <= floor(slot) && filter_admits(slot, target)
}

/// Whether **any** `Active` sink would accept this target at `level`.
///
/// This is the census's `UNPROVEN(unsunk)` question (disposition E20). A target enabled at `Info`
/// with no sink accepting it produces silence that is indistinguishable from a clean run — the
/// vacuous gate in a new costume — so the census asks this before it reports an absence.
#[must_use]
pub fn any_sink_accepts(target: TargetId, level: Level) -> bool {
    (0..SINK_SLOTS).any(|slot| accepts(slot, target, level))
}

/// Reset every slot to its boot default: `Active`, floor `Trace`, all targets.
///
/// For tests and for a host restoring a known policy. Not a lifecycle operation — it opens and
/// closes nothing.
pub fn reset() {
    for slot in 0..SINK_SLOTS {
        set_state(slot, SinkState::Active);
        set_floor(slot, Level::Trace);
        set_all_targets(slot);
    }
}
