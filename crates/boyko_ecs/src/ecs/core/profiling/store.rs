//! The durable store: a frame-major SoA on a process-lifetime `VmReservation`, and the `arm` /
//! `disarm` pair that is the enable path.
//!
//! # Frame-major, decided with numbers
//!
//! The columns are indexed `[frame * zone_stride + zone]`. The alternative — zone-major
//! `[zone * WINDOW + frame]` — spreads one fold across ~400 live zones × 5 columns, i.e. about
//! 2000 distinct cache lines, far past L1d. Frame-major touches `21 B × zone_stride` contiguous
//! bytes per frame and lets the window reduction, which runs `#[cold]` once, pay the strided side
//! instead. The fold is the frequent side, so the fold is the side the layout serves.
//!
//! # The reservation has NO owner, and that is the point
//!
//! [`VmReservation`](crate::ecs::memory::vm::VmReservation)'s `Drop` unmaps. Worker threads hold
//! `buf` pointers derived from this reservation, and those pointers are **never nulled** — that is
//! what lets a producer hold one without a lifetime. So an owner whose `Drop` could run would
//! dangle every one of them the instant a world was dropped in a multi-world test or at teardown.
//! An argument cannot fix that; only a location can. The reservation is created, committed,
//! published into [`VM_BASE`] and then deliberately `mem::forget`-ed, so *"never freed"* is
//! structural rather than asserted. **This is the one deliberate leak in the engine**, and it
//! leaks address space that the process was going to hold anyway.
//!
//! [`Profiler`] therefore holds a `base: NonNull<u8>` plus **byte offsets**, never `&'static mut`
//! slices. Eleven `&'static mut` fields aliasing memory the same struct owns are two mutable paths
//! to the same bytes; Tree Borrows flags exactly that, and the kernel's own `VmColumn` already
//! avoids it the same way.
//!
//! # What this rung stores, and what it deliberately does not
//!
//! Here: the five per-`(frame, zone)` columns, the per-frame records, and the frame-begin cut.
//!
//! Absent, by rung: `lifetime` / `hist_of` / `hists` (rung 12), `sys_of` / `rounds` (rung 3),
//! `legs` (rung 8), `compat` / `intervals` (rung 3, behind `profiling-analysis`). Each is absent
//! rather than reserved-and-zero, for the reason [`LogStats`](crate::ecs::core::log::LogStats)
//! ships one field instead of eleven: **a value that is structurally always zero is
//! indistinguishable from a measurement of zero**, and an offset nothing reads is an extent
//! nothing can prove wrong.
//!
//! The same rule shortens [`FrameRecord`]. The corpus pins the complete record at 88 B; every one
//! of the fields this rung omits (`run_gross`, `fixed_total`, `main_total`, `instrument_*`,
//! `gpu_total`, `fixed_steps`, `rounds`) is filled by the four `App` zones at rung 3 or by the GPU
//! channel at rung 5. The pin here is 32 B and moves as they land.

use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

use boyko_diag::lane::LANE_COUNT;
use boyko_diag::loss::{LossClass, LossSeen};
use boyko_diag::profile::REGION_CAPACITY;
use boyko_diag::profiling_abi::ENGINE_ZONE_SLOTS;
use boyko_diag::sample::{Region, Sample};
use boyko_diag::{clock, loss, profiling_abi, sample};

use crate::ecs::core::profiling::diag;
use crate::ecs::core::profiling::ecs_control::LatencyTable;
use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;
use crate::ecs::memory::vm::VmReservation;

/// Frames retained. **Odd, deliberately**: an even window makes every median the mean of the two
/// middle samples — a value no frame produced, sitting half a lattice tick off, which is exactly
/// how this project once mis-derived a 16 ns lattice.
///
/// 121 frames is ~2.02 s at 60 Hz.
pub const WINDOW: usize = 121;
const _: () = assert!(WINDOW % 2 == 1, "an even window medians a value no frame produced");

/// The scope bit `arm` sets — the engine's own zones, and the profiler's master switch.
///
/// # Rung 11 did NOT replace this, and the reason is a trap it would otherwise have walked into
///
/// The row read *"the mask exists from rung 1"*, and the plan was for the ECS projection to take
/// the mask over. It takes over **bits 8..63** only
/// ([`PROJECTED_SCOPE_BASE`](boyko_diag::profiling_abi::PROJECTED_SCOPE_BASE)), and this bit stays
/// `arm`'s.
///
/// With the whole word projectable, disabling every scope clears the mask; the fold's entry gate is
/// [`any_armed`](crate::ecs::core::profiling::any_armed), so the fold would stop running — and the
/// projection **is a step of the fold**. Re-enabling a scope would then write a bit nothing reads,
/// and the toggle a game just used would be one-way, permanently, with no diagnostic. `G12`'s
/// re-enable clause is what asserts it is not.
///
/// So `arm` / `disarm` mean what they always meant — the profiler is on or off as a whole — and a
/// scope is the finer switch inside an armed session. Every engine zone is declared on this bit, so
/// rung 11 is purely additive at every existing site: nothing the engine already measures is behind
/// a scope entity that has to exist for it to be measured.
pub const ROOT_SCOPE: u32 = 0;

/// Zones whose per-frame column row still fits L1d, from the corpus's own arithmetic: 21 B per
/// zone per frame × 1024 = 21 KiB of columns, plus the fold's ~9.6 KiB of sequential lane reads,
/// is 30.6 KiB against a 32 KiB L1d. It fits, and it is **tight** — which is why the figure is a
/// threshold that reports rather than a bound that refuses.
pub const FOLD_L1D_ZONE_LIMIT: u32 = 1024;

/// Bytes one zone occupies in one frame row, across all five columns: `8 + 4 + 4 + 4 + 1`.
pub const COLUMN_BYTES_PER_ZONE: u64 = 21;

/// Frames the interval ring retains — far fewer than [`WINDOW`], and deliberately.
///
/// An interval is a *per-occurrence* record: a frame contributes one per span, where a column cell
/// contributes one per `(zone, frame)` however many times the zone opened. Retaining 121 frames of
/// them would cost 3.8 MiB to answer a question — "did these two systems overlap?" — that is about
/// the schedule's shape and not about this frame in particular. Eight frames is enough to see the
/// shape and short enough that the ring stays a fixed 256 KiB.
#[cfg(feature = "profiling-analysis")]
pub const OVERLAP_FRAMES: usize = 8;

/// Intervals one frame's bank holds before it refuses and counts.
#[cfg(feature = "profiling-analysis")]
pub const INTERVALS_PER_FRAME: usize = 2048;

/// One retained span occurrence, **appended and never assigned**.
///
/// # The append is the whole point
///
/// An earlier design wrote one slot per `(frame, system)`, which a `Fixed`-schedule system running
/// N times per frame overwrote N−1 times — so the record of a system that ran eight times was the
/// eighth run, labelled as if it were the frame's. Appending is what makes "this system ran N times
/// and here is each one" representable at all.
///
/// # `zone`, not `sys`
///
/// The corpus names this field `sys`. It holds a **zone id**, because rung 3a put the
/// system → zone mapping in `SystemMeta.zone`, which the schedule owns — so zone → system is
/// resolved at report time by the one holder of the schedule, and the fold does not carry a side
/// table to say a second time what the schedule already says. Calling the field `sys` while it
/// holds a zone would be the kind of name this campaign exists to catch.
///
/// # `occ` costs no state
///
/// The occurrence index is `count[frame * stride + zone]` **before** the fold's increment — a value
/// the fold already has in a register. A counter beside the ring would be a second statement of it.
#[cfg(feature = "profiling-analysis")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Interval {
    /// The clock at the span's open — the same stamp the fold attributed on.
    pub begin: u64,
    /// The span's duration in ticks, clamped to `u32::MAX`. A clamped interval is already labelled
    /// `OverRange` in its cell and counted in `span_over_range`.
    pub dur: u32,
    /// The zone the span was opened on.
    pub zone: u16,
    /// Which occurrence of this zone in this frame, counting from 0.
    pub occ: u16,
}

#[cfg(feature = "profiling-analysis")]
const _: () = assert!(size_of::<Interval>() == 16);

/// A tick gap above which the fold treats the clock as having jumped rather than the frame as
/// having been slow.
///
/// Ten seconds at the pessimistic end of any plausible scale (1 GHz) — chosen far above any hitch
/// a frame can produce and far below a suspend, so the detector cannot mistake one for the other.
/// A frame that genuinely took 10 s has produced a number no window should carry anyway.
pub const MAX_PLAUSIBLE_FRAME_TICKS: u64 = 10_000_000_000;

/// The frame flag set when the clock's scale was never probed, so tick magnitudes in this window
/// are unscaled.
///
/// This is `DiagFlag::ClockUncalibrated`'s report. It is a **status on the data**, not an event,
/// which is why it has no `92xx` code — see [`diag`]'s module docs for the whole of that table.
pub const FRAME_FLAG_CLOCK_UNCALIBRATED: u8 = 1 << 0;

/// What a `(frame, zone)` cell's `label` column says about it.
///
/// Rung 2 can reach three of the corpus's five. `NOT_BRACKETED`, `TORN` and `LOST` are the GPU
/// channel's 2×2 label and arrive with it at rung 5; adding them now would put two variants in an
/// enum that no code path can produce, which reads as coverage and is not.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellLabel {
    /// No sample was folded into this cell. The zeroed state, so a recycled row starts here.
    Empty = 0,
    /// The cell holds a direct measurement.
    Measured = 1,
    /// At least one sample's value exceeded `u32::MAX` ticks, so `min`/`max` are clamped.
    /// `total` and `count` are still exact — only the extrema lost range.
    OverRange = 2,
}

/// Where a frame row is in its life.
///
/// `Partial` — the corpus's third state — exists for a frame whose GPU slot never retired. With no
/// GPU channel at this rung every folded frame is `Sealed`, so the variant would be unreachable.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameState {
    /// The live frame: open, still accumulating.
    Pending = 0,
    /// Closed and folded. Every sample stamped inside it that has reached the ring is in its row.
    Sealed = 1,
}

/// One frame's own record.
///
/// **32 B at this rung**, not the corpus's 88 — see the module docs on which fields land when.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRecord {
    /// The absolute frame number. Monotone across the whole session, unlike the row index.
    pub frame: u32,
    /// Samples this frame lost, from every class, at the moment it sealed.
    pub drops: u32,
    /// The clock at the frame's open — the same value the attribution cut reads.
    pub cpu_begin: u64,
    /// The clock at the frame's seal. Zero while `Pending`.
    pub cpu_end: u64,
    /// Samples folded into this frame's row.
    pub samples: u32,
    /// The clock epoch this frame was recorded in. Two frames with different epochs are not
    /// comparable, and a reader that ignores this is comparing a suspend.
    pub clock_epoch: u16,
    /// See [`FrameState`].
    pub state: FrameState,
    /// [`FRAME_FLAG_CLOCK_UNCALIBRATED`] and nothing else, at this rung.
    pub flags: u8,
}

const _: () = assert!(size_of::<FrameRecord>() == 32);
const _: () = assert!(align_of::<FrameRecord>() == 8);

impl FrameRecord {
    /// A zeroed record. `FrameState::Pending` and `CellLabel::Empty` are both discriminant 0, so a
    /// freshly committed page is already a valid, empty window — which is what lets `arm` skip an
    /// initialisation pass over 10 KiB it would otherwise have to write.
    const ZERO: FrameRecord = FrameRecord {
        frame: 0,
        drops: 0,
        cpu_begin: 0,
        cpu_end: 0,
        samples: 0,
        clock_epoch: 0,
        state: FrameState::Pending,
        flags: 0,
    };
}

/// The drop classes this rung's fold can actually move.
///
/// Six of the corpus's eighteen. The other twelve are the GPU channel's, the dynamic registry's
/// and the telemetry writer's, and each arrives with the rung that can make it non-zero.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DropCounters {
    /// Samples the engine regions refused for want of space.
    pub engine_overflow: u64,
    /// Samples the user regions refused for want of space. Separate from the engine's by
    /// construction, which is the entire reason a lane has two regions.
    pub user_overflow: u64,
    /// Samples produced by a thread that held no lane, read from the substrate's un-laned row.
    pub unclaimed: u64,
    /// Samples whose frame had already left the retained window.
    pub late: u64,
    /// Spans whose duration exceeded `u32::MAX` ticks, so their cell's `min`/`max` clamped.
    pub span_over_range: u64,
    /// Windows discarded because the clock's epoch broke under them.
    pub clock_epoch_breaks: u64,
    /// Spans refused by a full interval bank, so the overlap analysis did not see them.
    ///
    /// **Only a full bank counts here.** A span attributed to a frame the ring no longer covers is
    /// *not* counted: its measurement is in its column cell either way, and the ring's eight-frame
    /// horizon is a stated bound, not a loss — exactly as a stamp below the 121-frame floor is
    /// `late` rather than "dropped from the columns". Counting it would report one sample twice.
    #[cfg(feature = "profiling-analysis")]
    pub intervals_dropped: u64,
}

impl DropCounters {
    /// Every class summed — what a frame record carries and what `W9203`'s reader compares.
    #[must_use]
    pub fn total(&self) -> u64 {
        let t = self
            .engine_overflow
            .saturating_add(self.user_overflow)
            .saturating_add(self.unclaimed)
            .saturating_add(self.late)
            .saturating_add(self.span_over_range)
            .saturating_add(self.clock_epoch_breaks);
        #[cfg(feature = "profiling-analysis")]
        let t = t.saturating_add(self.intervals_dropped);
        t
    }
}

/// How a session is sized. One knob at this rung, because one knob is what the store reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ProfilerConfig {
    /// Zone slots reserved above [`ENGINE_ZONE_SLOTS`] for a game's own zones. `0` at the default:
    /// the dynamic registry that spends them is rung 10, and a budget nothing can claim is an
    /// extent nothing can prove wrong.
    pub user_zone_budget: u32,
}

/// What an [`Profiler::arm`] call did.
///
/// A value rather than a panic: a mis-sized re-arm is a host's configuration mistake, and a
/// profiler that kills a shipped title over one has become the failure it exists to report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArmOutcome {
    /// The reservation was created, committed and published by this call.
    Armed,
    /// A previous `arm` already published the geometry; this call only re-set the mask. **It
    /// allocates zero additional bytes**, which is a clause of the residency gate.
    Rearmed,
    /// The live session's geometry differs from the one asked for. `E9213`; nothing changed.
    GeometryMismatch {
        /// The stride the live session was armed with.
        live: u32,
        /// The stride this call asked for.
        asked: u32,
    },
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Process-global publication
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The reservation's base, published once at the first `arm` and never cleared.
///
/// `.bss`, so a process that never arms writes no page of it.
static VM_BASE: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

/// The reservation's granule-rounded length. What `Profiler::reserved_bytes` reports and what the
/// residency gate's second domain measures.
static VM_LEN: AtomicUsize = AtomicUsize::new(0);

/// The live session's `zone_stride`. `0` means "never armed", which is why a stride of 0 is
/// impossible by construction: [`ENGINE_ZONE_SLOTS`] is non-zero.
static ARMED_STRIDE: AtomicU32 = AtomicU32::new(0);

/// The world the profiler is bound to, or [`UNBOUND`].
static BOUND_WORLD: AtomicU64 = AtomicU64::new(UNBOUND);

/// No world has bound the profiler yet.
pub const UNBOUND: u64 = u64::MAX;

const _: () = assert!(ENGINE_ZONE_SLOTS > 0, "a zero stride would alias 'never armed'");

/// Bind the process-global profiler to `world_id`.
///
/// Returns `Err(live)` when another world already holds it. **v1 binds to exactly one world** —
/// the lane rings and the reservation are process-global while worlds are not, so two worlds
/// folding the same rings would each see half the samples and neither would say so.
///
/// Binding the *same* world twice succeeds: a plugin added twice is a host's duplicate
/// registration, not a second world.
pub fn bind_world(world_id: u64) -> Result<(), u64> {
    match BOUND_WORLD.compare_exchange(UNBOUND, world_id, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(()),
        Err(live) if live == world_id => Ok(()),
        Err(live) => Err(live),
    }
}

/// The world the profiler is bound to, or [`UNBOUND`].
#[must_use]
pub fn bound_world() -> u64 {
    BOUND_WORLD.load(Ordering::Acquire)
}

/// Release the binding. **Test-only**, and it exists for one reason: `BOUND_WORLD` is
/// process-global, so without it the first test to bind would decide the answer for every other
/// test in the binary.
#[cfg(test)]
pub(crate) fn unbind_world() {
    BOUND_WORLD.store(UNBOUND, Ordering::Release);
}

/// **ONE lock for this module's globals — all of them.**
///
/// `VM_BASE`, `VM_LEN`, `ARMED_STRIDE`, `BOUND_WORLD`, `ARM_MASK`, the lane rings and the `92xx`
/// once-latches are every one of them process-wide. Two tests touching any of them concurrently
/// fold each other's samples into each other's frames, or race a bind, and the failure reads as an
/// attribution bug rather than as a fixture collision.
///
/// One lock rather than one per static, and the difference has been paid for elsewhere in this
/// campaign: two locks over one domain is two orders in which they can be taken.
///
/// `Mutex` is on this project's disallowed list for the lock-free discipline (Principle 4). This is
/// the sanctioned exception shape — a `#[cfg(test)]` fixture, never on any engine path, carrying
/// its rationale — and it is the same one `boyko_log::drain_owner::TEST_SERIAL` uses for the same
/// reason.
#[cfg(test)]
#[allow(clippy::disallowed_types)]
pub(crate) static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the module's test lock, ignoring poisoning — a test that panicked while holding it has
/// already reported, and refusing every later test would turn one failure into a cascade.
#[cfg(test)]
#[allow(clippy::disallowed_types)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    TEST_SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Layout
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Every section is padded to a cache line. The alternative is per-type alignment and a
/// hand-checked argument that `min`'s `u32` start never lands mid-line; a line is one rule.
const SECTION_ALIGN: usize = 64;

/// Byte offsets of every section of the reservation, derived once from the stride.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Layout {
    /// The sample slab, first so it starts at the reservation's own alignment.
    slab: usize,
    total: usize,
    count: usize,
    min: usize,
    max: usize,
    label: usize,
    frames: usize,
    begin: usize,
    /// The interval ring: `OVERLAP_FRAMES` banks of `INTERVALS_PER_FRAME` slots each.
    #[cfg(feature = "profiling-analysis")]
    intervals: usize,
    /// Total bytes, before the reservation's granule rounding.
    bytes: usize,
}

const fn align_up(x: usize, a: usize) -> usize {
    (x + a - 1) & !(a - 1)
}

impl Layout {
    /// The layout for `zone_stride` zones.
    pub(crate) const fn new(zone_stride: u32) -> Layout {
        let cells = zone_stride as usize * WINDOW;
        let slab = 0usize;
        let slab_bytes =
            LANE_COUNT as usize * 2 * REGION_CAPACITY as usize * size_of::<Sample>();

        let total = align_up(slab + slab_bytes, SECTION_ALIGN);
        let count = align_up(total + cells * 8, SECTION_ALIGN);
        let min = align_up(count + cells * 4, SECTION_ALIGN);
        let max = align_up(min + cells * 4, SECTION_ALIGN);
        let label = align_up(max + cells * 4, SECTION_ALIGN);
        let frames = align_up(label + cells, SECTION_ALIGN);
        let begin = align_up(frames + WINDOW * size_of::<FrameRecord>(), SECTION_ALIGN);

        // The ring is the LAST section, so the two builds differ by a suffix and every offset
        // before it is identical — which is what keeps the feature from moving anything a
        // feature-off reader already knows the address of.
        #[cfg(feature = "profiling-analysis")]
        let intervals = align_up(begin + WINDOW * 8, SECTION_ALIGN);
        #[cfg(feature = "profiling-analysis")]
        let bytes = align_up(
            intervals + OVERLAP_FRAMES * INTERVALS_PER_FRAME * size_of::<Interval>(),
            SECTION_ALIGN,
        );
        #[cfg(not(feature = "profiling-analysis"))]
        let bytes = align_up(begin + WINDOW * 8, SECTION_ALIGN);

        Layout {
            slab,
            total,
            count,
            min,
            max,
            label,
            frames,
            begin,
            #[cfg(feature = "profiling-analysis")]
            intervals,
            bytes,
        }
    }

    /// Total bytes this geometry needs, before the reservation rounds to its commit granule.
    #[must_use]
    pub(crate) const fn bytes(&self) -> usize {
        self.bytes
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The resource
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The durable store. A `Resource`, mutated only outside the schedule.
pub struct Profiler {
    /// `None` until the first `arm`. The reservation's base, copied from [`VM_BASE`].
    base: Option<core::ptr::NonNull<u8>>,
    /// `ENGINE_ZONE_SLOTS + user_zone_budget`, fixed at arm and const for the session.
    zone_stride: u32,
    /// Section offsets for [`zone_stride`](Self::zone_stride).
    layout: Layout,
    /// The live frame's row index, in `0..WINDOW`.
    cursor: u32,
    /// The live frame's absolute number.
    frame: u32,
    /// The clock epoch the live window was opened in.
    epoch: u32,
    /// The clock at the previous fold, for the forward-jump detector. `0` before the first fold.
    last_fold: u64,
    /// The walk's hint: the absolute frame the previous sample was attributed to. A hint, never a
    /// bound — the walk moves both ways from it and re-derives the answer.
    walk_hint: u32,
    /// Whether `ARM_MASK` was set by this store.
    armed: bool,
    /// The six classes this rung can move.
    drops: DropCounters,
    /// The consumer-side half of the monotone overflow counters, one per `(lane, region)`.
    ///
    /// **This is `substrate/loss-fold`'s Q2(b) applied.** The region counter is never cleared by
    /// anybody; exactness comes from the reader keeping what it last saw. That is why this array
    /// is here, in the consumer, rather than being a clear in the producer's cell.
    overflow_seen: [[u64; 2]; LANE_COUNT as usize],
    /// The same, for the substrate's un-laned `Unclaimed` cell.
    unclaimed_seen: LossSeen,
    /// How many intervals each of the ring's banks holds.
    ///
    /// Not derivable from anything already stored: the frame record's `samples` counts every
    /// sample of every kind, while a bank holds spans only. Reset when the frame that owns the
    /// bank opens, which is what stops a bank from reporting the frame it held eight frames ago.
    #[cfg(feature = "profiling-analysis")]
    interval_len: [u32; OVERLAP_FRAMES],
}

// SAFETY (manual `Send`/`Sync` for `Profiler` -- `Resource: 'static + Send + Sync` while
//   `NonNull<u8>` is neither):
//   (a) EXCLUSIVITY. Every mutation of the columns happens in `fold`, which runs at the top of
//       `App::update_with_delta` -- outside any schedule, on the dispatcher/host thread, with zero
//       workers in flight. In-frame access is `Res<Profiler>`, shared-only, and the kernel's own
//       resource borrow rules refuse a second `&mut` while one is out. So there is never a
//       concurrent `&mut` to the same bytes.
//   (b) VALIDITY. `base` is copied from `VM_BASE`, which is published `Release` once and never
//       cleared, over a reservation that is `mem::forget`-ed at that same publication. The region
//       is never resized, never moved and never freed, so no pointer derived from it can dangle --
//       including the lane `buf` pointers, which are derived from the same base and handed to
//       every producer thread.
//   (c) NO INTERIOR ALIASING. The struct holds a base and byte offsets, never `&'static mut`
//       slices. A slice is reconstituted for the duration of one call, from disjoint ranges the
//       `Layout` computes, so two live `&mut` into one range is unrepresentable rather than
//       merely avoided.
unsafe impl Send for Profiler {}
// SAFETY: see the `Send` clause immediately above; (a) is what licenses shared access from the
//   schedule, and the columns are only ever read through `&Profiler` there.
unsafe impl Sync for Profiler {}

// WHAT REDS ON A NEW FIELD. A struct pattern that omits a field is `error[E0027]`, so adding one
// to `Profiler` breaks this, at compile time, in this file, immediately below the three SAFETY
// clauses that adding it obliges a human to re-read. `field: _` counts as mentioning the field, so
// the witness costs nothing at runtime and warns nothing. What it cannot do is assert `Send`/`Sync`
// per field — `base` is deliberately `!Send`, which is the entire reason the manual impls exist.
// The witness forces the re-read; the re-read decides whether (a), (b) and (c) still hold.
const _: () = {
    #[allow(dead_code)] // never called; it exists to be type-checked
    fn field_witness(p: &Profiler) {
        let Profiler {
            base: _,
            zone_stride: _,
            layout: _,
            cursor: _,
            frame: _,
            epoch: _,
            last_fold: _,
            walk_hint: _,
            armed: _,
            drops: _,
            overflow_seen: _,
            unclaimed_seen: _,
            #[cfg(feature = "profiling-analysis")]
                interval_len: _,
        } = p;
    }
};

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a dev-dependency of
// `boyko-ecs`, so its derives are unavailable in normal builds. Mirrors exactly what the derive
// expands to.
impl Resource for Profiler {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}

impl Default for Profiler {
    fn default() -> Profiler {
        Profiler::new()
    }
}

impl Profiler {
    /// A disarmed store. **Reserves nothing, commits nothing, calibrates nothing.**
    ///
    /// `ProfilerPlugin::build` runs before a host has read its launch flag, and a diagnostics
    /// subsystem may not make a syscall the flag has not authorised. Every one-time cost is in
    /// [`arm`](Self::arm), and `arm` **is** the enable path.
    #[must_use]
    pub fn new() -> Profiler {
        Profiler {
            base: None,
            zone_stride: 0,
            layout: Layout::new(0),
            cursor: 0,
            frame: 0,
            epoch: 0,
            last_fold: 0,
            walk_hint: 0,
            armed: false,
            drops: DropCounters::default(),
            overflow_seen: [[0; 2]; LANE_COUNT as usize],
            unclaimed_seen: LossSeen::ZERO,
            #[cfg(feature = "profiling-analysis")]
            interval_len: [0; OVERLAP_FRAMES],
        }
    }

    /// Whether this store has armed the mask.
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// The live geometry, or `0` when this store never armed.
    #[must_use]
    pub fn zone_stride(&self) -> u32 {
        self.zone_stride
    }

    /// The live frame's absolute number.
    #[must_use]
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// The drop counters, as of the last fold.
    #[must_use]
    pub fn drops(&self) -> DropCounters {
        self.drops
    }

    /// Bytes of address space the reservation holds, process-wide. `0` before any arm.
    ///
    /// Process-wide rather than per-store because the reservation is: a second `Profiler` in a
    /// second world would report the same figure, which is the honest answer and the reason the
    /// world bind exists.
    #[must_use]
    pub fn reserved_bytes() -> usize {
        VM_LEN.load(Ordering::Acquire)
    }

    /// Arm the profiler: reserve, commit, publish, calibrate, set the mask — in that order.
    ///
    /// # The publication order is the whole soundness argument
    ///
    /// **slab → every lane `buf` (`Release`) → `ARM_MASK` (`Release`), in that order, always.** An
    /// emitter's first act is an `Acquire` load of the mask, so a thread that observes a set mask
    /// happens-after every `buf` publication and cannot reach a null pointer with the mask set.
    /// Reversing the two would make `A1`'s `debug_assert!(!buf.is_null())` a real failure rather
    /// than a recorded invariant.
    ///
    /// # `debug_assert`, not a runtime check
    ///
    /// `arm` is a setup call. The assertion that it is not inside a system run reads the *calling
    /// thread's* TLS and therefore cannot observe another thread — it catches the mistake it can
    /// catch (a system arming the profiler) and says nothing about the one it cannot.
    pub fn arm(&mut self, cfg: ProfilerConfig) -> ArmOutcome {
        debug_assert!(
            !boyko_threadpool::is_in_system_run(),
            "invariant: Profiler::arm is a setup call and must not run inside a system"
        );

        let stride = ENGINE_ZONE_SLOTS as u32 + cfg.user_zone_budget;
        let live = ARMED_STRIDE.load(Ordering::Acquire);
        if live != 0 && live != stride {
            diag::report_geometry_mismatch(live, stride);
            return ArmOutcome::GeometryMismatch { live, asked: stride };
        }

        let layout = Layout::new(stride);

        // The clock's scale is probed on the enable path, never at process start.
        clock::calibrate();
        if diag::clock_code(clock::invariant_tsc()).is_some() {
            diag::report_no_invariant_tsc();
        }

        let (base, first) = Self::publish_reservation(layout);

        // ── publication order, step 1: every lane's buffer, BEFORE the mask ──
        //
        // Re-published on a second arm too, and `publish_region` refuses each one: publication is
        // once, because a replacement would strand every producer holding the old pointer. The
        // refusals are the expected answer, not an error.
        for lane in 0..LANE_COUNT {
            for (i, region) in [Region::Engine, Region::User].into_iter().enumerate() {
                let off = layout.slab
                    + (lane as usize * 2 + i) * REGION_CAPACITY as usize * size_of::<Sample>();
                // SAFETY: `off` is inside the slab section, which `Layout::new` sized at
                //   `LANE_COUNT * 2 * REGION_CAPACITY * size_of::<Sample>()` bytes starting at
                //   `layout.slab`; the largest `off` produced here is that size minus one region.
                //   The reservation is committed over `[0, layout.bytes())` and is never freed,
                //   never moved and never resized, so the `REGION_CAPACITY` slots at `off` stay
                //   writable and correctly aligned for the life of the process -- which is exactly
                //   `publish_region`'s contract.
                let ptr = unsafe { base.as_ptr().add(off).cast::<Sample>() };
                // SAFETY: as above -- `ptr` addresses `REGION_CAPACITY` writable, 8-aligned
                //   `Sample` slots (the section starts at the reservation base, which is page
                //   aligned, and every region is a whole multiple of `size_of::<Sample>()` from
                //   it) that outlive the process.
                unsafe {
                    sample::publish_region(lane, region, ptr);
                }
            }
        }

        self.base = Some(base);
        self.zone_stride = stride;
        self.layout = layout;
        self.cursor = 0;
        self.frame = 0;
        self.epoch = clock::clock_epoch();
        self.last_fold = 0;
        self.walk_hint = 0;

        // The consumer-side deltas start from what the counters read NOW, so a session does not
        // inherit the losses of the one before it. Q2(b) puts this state at the consumer precisely
        // so a new consumer can start clean without touching the producer's cell.
        for lane in 0..LANE_COUNT {
            self.overflow_seen[lane as usize][0] = sample::overflow(lane, Region::Engine);
            self.overflow_seen[lane as usize][1] = sample::overflow(lane, Region::User);
        }
        let cell = loss::cell_at_row(loss::ROW_UNLANED, LossClass::Unclaimed);
        self.unclaimed_seen = LossSeen { count: cell.count(), bytes: cell.bytes() };

        // Every bank, not just frame 0's: a re-arm must not leave seven banks holding the previous
        // session's intervals for the report to read as this session's.
        #[cfg(feature = "profiling-analysis")]
        {
            self.interval_len = [0; OVERLAP_FRAMES];
        }

        // A new session starts on a clean window. Without this, a re-arm would inherit the rows of
        // the session before it — a frame row is only recycled when the cursor reaches it, so rows
        // the old session never wrapped past would keep answering `frame_record` with figures from
        // a geometry that is no longer live. The memset is a one-time cost on the enable path,
        // which is where one-time costs belong.
        self.discard_window();

        // The first frame's row: opened here so the fold always has a live frame to attribute to.
        let now = clock::ticks();
        self.write_frame(
            0,
            FrameRecord {
                frame: 0,
                cpu_begin: now,
                clock_epoch: self.epoch as u16,
                ..FrameRecord::ZERO
            },
        );
        self.write_begin(0, now);

        // ── publication order, step 2: the mask, LAST, always ──
        profiling_abi::arm_scope(ROOT_SCOPE);
        ARMED_STRIDE.store(stride, Ordering::Release);
        self.armed = true;

        // Reported after arming, not before: the session is running either way, and a host that
        // reads the warning wants to know the geometry it actually got.
        let working_set = COLUMN_BYTES_PER_ZONE * u64::from(stride);
        if stride > FOLD_L1D_ZONE_LIMIT {
            diag::report_working_set(working_set, stride);
        }

        if first { ArmOutcome::Armed } else { ArmOutcome::Rearmed }
    }

    /// Reserve, commit and publish the backing store, or adopt the one a previous arm published.
    ///
    /// Returns the base and whether this call created it.
    fn publish_reservation(layout: Layout) -> (core::ptr::NonNull<u8>, bool) {
        let live = VM_BASE.load(Ordering::Acquire);
        if let Some(base) = core::ptr::NonNull::new(live) {
            return (base, false);
        }

        let vm = VmReservation::reserve(layout.bytes());
        vm.commit(0, vm.os_len());
        let base = vm.base();

        match VM_BASE.compare_exchange(
            core::ptr::null_mut(),
            base.as_ptr(),
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                VM_LEN.store(vm.os_len(), Ordering::Release);
                // THE ONE DELIBERATE LEAK. `VmReservation::drop` unmaps, and the lane `buf`
                // pointers derived from this base are published to every producer thread and are
                // never nulled -- so unmapping is UB for the life of the process. Leaving no owner
                // makes "never freed" structural instead of asserted.
                core::mem::forget(vm);
                (base, true)
            }
            // Another thread published first. Ours was never handed to anybody, so dropping it
            // unmaps only address space no pointer was derived from. One wasted reservation in a
            // race a single-world profiler cannot have is cheaper than a phase word and a spin.
            Err(won) => {
                drop(vm);
                (
                    core::ptr::NonNull::new(won).expect("invariant: a published base is non-null"),
                    false,
                )
            }
        }
    }

    /// Clear the mask. **Frees nothing and nulls nothing.**
    ///
    /// An emitter that passed the mask gate before this store could load a nulled `buf` after it,
    /// so nulling is not an option — which is the same reason there is no free. Disarm is a mask
    /// store and a cursor reset, and that is the whole of it.
    pub fn disarm(&mut self) {
        profiling_abi::disarm_scope(ROOT_SCOPE);
        self.armed = false;
    }

    // ── column access ───────────────────────────────────────────────────────────────────────

    /// Cells in one column: `zone_stride × WINDOW`.
    #[inline]
    #[must_use]
    pub fn cells(&self) -> usize {
        self.zone_stride as usize * WINDOW
    }

    /// The raw column pointers, derived once per fold.
    ///
    /// Returns `None` when the store never armed, which is the only state in which `base` is
    /// absent — so every caller's null check is this one.
    #[inline]
    fn columns(&self) -> Option<Columns> {
        let base = self.base?;
        // SAFETY: `Layout::new` places the five column sections inside `[0, layout.bytes())`,
        //   pairwise disjoint and each `SECTION_ALIGN`-aligned, and the reservation is committed
        //   over exactly that range and never freed. `base` came from `VM_BASE`, published after
        //   the commit with a `Release` this load's `Acquire` paired with.
        unsafe {
            Some(Columns {
                total: base.as_ptr().add(self.layout.total).cast::<u64>(),
                count: base.as_ptr().add(self.layout.count).cast::<u32>(),
                min: base.as_ptr().add(self.layout.min).cast::<u32>(),
                max: base.as_ptr().add(self.layout.max).cast::<u32>(),
                label: base.as_ptr().add(self.layout.label),
                cells: self.cells(),
            })
        }
    }

    /// One `(frame_row, zone)` cell, or `None` for an unarmed store or an out-of-range index.
    ///
    /// The read path. A `Res<Profiler>` system uses this; the fold writes through [`Columns`].
    #[must_use]
    pub fn cell(&self, row: u32, zone: u16) -> Option<Cell> {
        let cols = self.columns()?;
        let idx = self.index(row, zone)?;
        // SAFETY: `index` returned `Some`, so `idx < cells` and every column holds `cells`
        //   initialised elements -- the reservation is committed and zero-filled, and every
        //   discriminant this rung stores in `label` has a zero variant.
        unsafe {
            Some(Cell {
                total: cols.total.add(idx).read(),
                count: cols.count.add(idx).read(),
                min: cols.min.add(idx).read(),
                max: cols.max.add(idx).read(),
                label: match cols.label.add(idx).read() {
                    1 => CellLabel::Measured,
                    2 => CellLabel::OverRange,
                    _ => CellLabel::Empty,
                },
            })
        }
    }

    /// The flat column index of `(row, zone)`, or `None` when either is out of range.
    #[inline]
    #[must_use]
    fn index(&self, row: u32, zone: u16) -> Option<usize> {
        if row as usize >= WINDOW || u32::from(zone) >= self.zone_stride {
            return None;
        }
        Some(row as usize * self.zone_stride as usize + zone as usize)
    }

    /// One frame's record, or `None` for an unarmed store or an out-of-range row.
    #[must_use]
    pub fn frame_record(&self, row: u32) -> Option<FrameRecord> {
        let base = self.base?;
        if row as usize >= WINDOW {
            return None;
        }
        // SAFETY: `row < WINDOW` and the frames section holds `WINDOW` records inside the
        //   committed range; `FrameRecord` is `Copy` POD whose all-zero bit pattern is valid.
        unsafe {
            Some(
                base.as_ptr()
                    .add(self.layout.frames)
                    .cast::<FrameRecord>()
                    .add(row as usize)
                    .read(),
            )
        }
    }

    /// The row index of the live frame.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// The published lag table (D25) — profiling rung 11.
    ///
    /// A `Res<Profiler>` reader driving LOD, dynamic resolution or quality scaling is looking at a
    /// **windowed, lagged** picture, and the lag is structural: the fold folds closed frames only
    /// (A2's live-frame cut), so the freshest complete frame is the one before the live one.
    ///
    /// Published as a value rather than documented as a sentence because a controller has to
    /// *compute* with it — and because S1 forbids printing it. What the table deliberately omits,
    /// and the measurement behind each omission, is [`LatencyTable`]'s own doc.
    #[must_use]
    pub fn latency(&self) -> LatencyTable {
        LatencyTable { live_frame: self.frame, cpu_frames_behind: 1 }
    }

    /// The row an absolute frame number occupies, if it is still retained.
    #[must_use]
    pub fn row_of(&self, frame: u32) -> Option<u32> {
        if frame > self.frame || self.frame - frame >= WINDOW as u32 {
            return None;
        }
        Some(frame % WINDOW as u32)
    }

    // ── write helpers, all `pub(crate)`: only the fold writes ────────────────────────────────

    /// Store one frame record.
    pub(crate) fn write_frame(&mut self, row: u32, rec: FrameRecord) {
        let Some(base) = self.base else { return };
        debug_assert!((row as usize) < WINDOW, "invariant: frame row is in 0..WINDOW");
        // SAFETY: `row < WINDOW` (asserted) and the frames section holds `WINDOW` records inside
        //   the committed range. `&mut self` is the exclusivity the write needs; clause (a) of the
        //   `Send` impl is what makes that borrow meaningful across threads.
        unsafe {
            base.as_ptr()
                .add(self.layout.frames)
                .cast::<FrameRecord>()
                .add(row as usize)
                .write(rec);
        }
    }

    /// Store one frame's begin stamp — the attribution cut for the frames after it.
    pub(crate) fn write_begin(&mut self, row: u32, ticks: u64) {
        let Some(base) = self.base else { return };
        debug_assert!((row as usize) < WINDOW, "invariant: frame row is in 0..WINDOW");
        // SAFETY: as `write_frame`, over the `WINDOW`-element `u64` begin section.
        unsafe {
            base.as_ptr().add(self.layout.begin).cast::<u64>().add(row as usize).write(ticks);
        }
    }

    /// One frame's begin stamp.
    #[must_use]
    pub(crate) fn begin_of_row(&self, row: u32) -> u64 {
        let Some(base) = self.base else { return 0 };
        debug_assert!((row as usize) < WINDOW, "invariant: frame row is in 0..WINDOW");
        // SAFETY: as `write_begin`; the section is committed and zero-filled, and `0` is a valid
        //   `u64`.
        unsafe { base.as_ptr().add(self.layout.begin).cast::<u64>().add(row as usize).read() }
    }

    /// Zero every column cell of one frame row, so a recycled row cannot report the frame it held
    /// `WINDOW` frames ago.
    ///
    /// `write_bytes` rather than a loop: `CellLabel::Empty` and every numeric zero share the
    /// all-zero bit pattern, which is why the recycle is one memset per column instead of five
    /// typed passes.
    pub(crate) fn zero_row(&mut self, row: u32) {
        let Some(cols) = self.columns() else { return };
        let z = self.zone_stride as usize;
        let start = row as usize * z;
        debug_assert!(start + z <= cols.cells, "invariant: a frame row is inside the columns");
        // SAFETY: `start + z <= cells` (asserted), and every column holds `cells` elements in the
        //   committed range. `&mut self` gives exclusivity; the five ranges are inside five
        //   disjoint sections, so no two of these writes alias.
        unsafe {
            cols.total.add(start).write_bytes(0, z);
            cols.count.add(start).write_bytes(0, z);
            cols.min.add(start).write_bytes(0, z);
            cols.max.add(start).write_bytes(0, z);
            cols.label.add(start).write_bytes(0, z);
        }
    }

    /// Zero every column cell and every frame record — the epoch break's discard.
    pub(crate) fn discard_window(&mut self) {
        for row in 0..WINDOW as u32 {
            self.zero_row(row);
            self.write_frame(row, FrameRecord::ZERO);
            self.write_begin(row, 0);
        }
    }

    /// The raw columns, for the fold's inner loop.
    #[inline]
    pub(crate) fn columns_for_fold(&self) -> Option<Columns> {
        self.columns()
    }

    /// The interval ring's base, or `None` when the store never armed.
    #[cfg(feature = "profiling-analysis")]
    #[inline]
    pub(crate) fn interval_ring(&self) -> Option<IntervalRing> {
        let base = self.base?;
        // SAFETY: `Layout::new` places the ring inside `[0, layout.bytes())`, `SECTION_ALIGN`
        //   aligned and disjoint from every other section, and the reservation is committed over
        //   exactly that range and never freed. The base came from `VM_BASE`, published `Release`
        //   after the commit.
        unsafe {
            Some(IntervalRing { base: base.as_ptr().add(self.layout.intervals).cast::<Interval>() })
        }
    }

    /// Every interval retained for `frame`, or an empty slice when the ring no longer covers it.
    ///
    /// The horizon is [`OVERLAP_FRAMES`] frames, and a frame outside it gets `&[]` rather than a
    /// stale bank: bank `frame % OVERLAP_FRAMES` belongs to the newest frame that claimed it, and
    /// handing that back under an older frame's number is how a reader comes to compare two
    /// different frames' spans as if they were one's.
    #[cfg(feature = "profiling-analysis")]
    #[must_use]
    pub fn intervals_of_frame(&self, frame: u32) -> &[Interval] {
        let Some(ring) = self.interval_ring() else { return &[] };
        if frame > self.frame || self.frame - frame >= OVERLAP_FRAMES as u32 {
            return &[];
        }
        let bank = frame as usize % OVERLAP_FRAMES;
        let len = self.interval_len[bank] as usize;
        debug_assert!(len <= INTERVALS_PER_FRAME, "invariant: a bank never exceeds its capacity");
        // SAFETY: `bank < OVERLAP_FRAMES` and `len <= INTERVALS_PER_FRAME`, so the range lies
        //   inside the ring section; every slot below `len` was written by `append_interval` in a
        //   previous fold, and the section is committed and zero-filled besides. The returned
        //   lifetime is `&self`'s, and only `&mut self` can append, so no writer can run while
        //   this slice is alive.
        unsafe {
            core::slice::from_raw_parts(ring.base.add(bank * INTERVALS_PER_FRAME), len)
        }
    }

    /// Frames the interval ring currently covers, newest first — what the report iterates.
    #[cfg(feature = "profiling-analysis")]
    pub fn interval_frames(&self) -> impl Iterator<Item = u32> + '_ {
        let live = self.frame;
        let depth = u32::min(live + 1, OVERLAP_FRAMES as u32);
        (0..depth).map(move |back| live - back)
    }

    /// Backdate the previous fold's stamp, so the next fold's detector sees a forward jump.
    ///
    /// **Test-only, and it is what makes `G21` showable at all.** The detector's threshold is
    /// [`MAX_PLAUSIBLE_FRAME_TICKS`] — ten seconds — and a gate cannot wait ten seconds any more
    /// than it can suspend the machine. Injecting the *input* is the only way to exercise the
    /// branch; the alternative is a detector nothing ever runs, which is the vacuous-gate pattern.
    #[cfg(test)]
    pub(crate) fn backdate_last_fold(&mut self, ticks: u64) {
        self.last_fold = ticks;
    }

    /// The clock epoch the live window was opened in.
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Mutable access to the fold's own bookkeeping, so `fold.rs` does not need public setters
    /// that anything else could reach.
    pub(crate) fn fold_state(&mut self) -> FoldState<'_> {
        FoldState {
            cursor: &mut self.cursor,
            frame: &mut self.frame,
            epoch: &mut self.epoch,
            last_fold: &mut self.last_fold,
            walk_hint: &mut self.walk_hint,
            drops: &mut self.drops,
            overflow_seen: &mut self.overflow_seen,
            unclaimed_seen: &mut self.unclaimed_seen,
            #[cfg(feature = "profiling-analysis")]
            interval_len: &mut self.interval_len,
        }
    }
}

/// The interval ring's base pointer, derived once per fold.
#[cfg(feature = "profiling-analysis")]
#[derive(Clone, Copy)]
pub(crate) struct IntervalRing {
    pub(crate) base: *mut Interval,
}

/// The five column base pointers plus the count that bounds them.
#[derive(Clone, Copy)]
pub(crate) struct Columns {
    pub(crate) total: *mut u64,
    pub(crate) count: *mut u32,
    pub(crate) min: *mut u32,
    pub(crate) max: *mut u32,
    pub(crate) label: *mut u8,
    pub(crate) cells: usize,
}

/// One `(frame, zone)` cell, as a reader sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    /// `Span`: Σ durations in ticks. `Counter`: Σ increments. `Gauge`: the last level.
    pub total: u64,
    /// Samples folded into this cell.
    pub count: u32,
    /// Smallest value seen, clamped to `u32::MAX`. Meaningless when `count == 0`.
    pub min: u32,
    /// Largest value seen, clamped to `u32::MAX`.
    pub max: u32,
    /// What the figures are worth.
    pub label: CellLabel,
}

/// The fold's borrow of the store's scalars, handed out as one struct so `fold.rs` can hold them
/// alongside the raw columns without a second `&mut self`.
pub(crate) struct FoldState<'a> {
    pub(crate) cursor: &'a mut u32,
    pub(crate) frame: &'a mut u32,
    pub(crate) epoch: &'a mut u32,
    pub(crate) last_fold: &'a mut u64,
    pub(crate) walk_hint: &'a mut u32,
    pub(crate) drops: &'a mut DropCounters,
    pub(crate) overflow_seen: &'a mut [[u64; 2]; LANE_COUNT as usize],
    pub(crate) unclaimed_seen: &'a mut LossSeen,
    #[cfg(feature = "profiling-analysis")]
    pub(crate) interval_len: &'a mut [u32; OVERLAP_FRAMES],
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sections must be disjoint and inside the total, or one column would overwrite another
    /// and the corruption would look like data.
    #[test]
    fn the_layout_sections_are_disjoint_and_inside_the_total() {
        for stride in [1u32, 256, 1024, ENGINE_ZONE_SLOTS as u32] {
            let l = Layout::new(stride);
            let cells = stride as usize * WINDOW;
            let slab = LANE_COUNT as usize * 2 * REGION_CAPACITY as usize * size_of::<Sample>();
            let spans = [
                (l.slab, slab),
                (l.total, cells * 8),
                (l.count, cells * 4),
                (l.min, cells * 4),
                (l.max, cells * 4),
                (l.label, cells),
                (l.frames, WINDOW * size_of::<FrameRecord>()),
                (l.begin, WINDOW * 8),
                #[cfg(feature = "profiling-analysis")]
                (l.intervals, OVERLAP_FRAMES * INTERVALS_PER_FRAME * size_of::<Interval>()),
            ];
            for (i, (off, len)) in spans.iter().copied().enumerate() {
                assert!(off + len <= l.bytes(), "section {i} runs past the reservation");
                assert!(off.is_multiple_of(SECTION_ALIGN), "section {i} is not line aligned");
                for (j, (off2, len2)) in spans.iter().copied().enumerate().skip(i + 1) {
                    assert!(
                        off + len <= off2 || off2 + len2 <= off,
                        "sections {i} and {j} overlap at stride {stride}"
                    );
                }
            }
        }
    }

    /// The threshold `W9211` reports against is the corpus's own arithmetic, not a round number
    /// chosen here: 21 B/zone × 1024 zones is the 21 KiB column row that, plus the fold's ~9.6 KiB
    /// of lane reads, lands at 30.6 KiB against a 32 KiB L1d.
    #[test]
    fn the_l1d_zone_limit_is_the_figure_it_is_derived_from() {
        assert_eq!(COLUMN_BYTES_PER_ZONE * u64::from(FOLD_L1D_ZONE_LIMIT), 21_504);
        assert!(COLUMN_BYTES_PER_ZONE * u64::from(FOLD_L1D_ZONE_LIMIT) + 9_600 < 32 * 1024);
    }

    /// A disarmed store touches nothing: no reservation, no cells, no reads that could fault.
    #[test]
    fn a_fresh_store_is_inert() {
        let p = Profiler::new();
        assert!(!p.is_armed());
        assert_eq!(p.zone_stride(), 0);
        assert_eq!(p.cells(), 0);
        assert!(p.cell(0, 0).is_none(), "an unarmed store has no cells to hand out");
        assert!(p.frame_record(0).is_none());
        assert_eq!(p.drops(), DropCounters::default());
    }

    /// The retained window is the last `WINDOW` frames and nothing else — the bound the walk's
    /// `late` branch rests on.
    #[test]
    fn row_of_retains_exactly_the_window() {
        let mut p = Profiler::new();
        p.frame = 300;
        assert_eq!(p.row_of(300), Some(300 % WINDOW as u32));
        assert_eq!(p.row_of(300 - (WINDOW as u32 - 1)), Some((300 - (WINDOW as u32 - 1)) % WINDOW as u32));
        assert_eq!(p.row_of(300 - WINDOW as u32), None, "a frame one past the window is gone");
        assert_eq!(p.row_of(301), None, "a frame that has not happened is not retained");
    }
}
