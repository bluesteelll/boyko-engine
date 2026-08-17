//! Targets, their ids, and the one packed control byte per target.
//!
//! # Why `CONTROL` lives here and not in a `control.rs`
//!
//! The rung's file list says `src/{level,control,target,macros}.rs`; the decision that specifies
//! the array says the opposite, in as many words: *there is no `control.rs` — `CONTROL` is
//! declared in `target.rs`, beside the [`TargetId`] whose invariant makes its `get_unchecked`
//! sound.* The argued side wins. Splitting them puts an unchecked index in one file and the only
//! reason it is sound in another, which is how a later edit widens [`TargetId`]'s constructor set
//! without anyone noticing what it just made unsound.
//!
//! # The three bands
//!
//! | Band | Ids | How uniqueness is proven |
//! |---|---|---|
//! | Engine | `0..=95` | one [`targets!`] table with a strictly-increasing `const` assert — collisions **do not compile** |
//! | Downstream source | `96..=223` | `define_target!` plus a boot check naming both colliders (a later rung) |
//! | Dynamic | `224..=255` | minted at run time from a name; the mint **is** the proof |
//!
//! **Two of the three exist** — the engine table and, since L10, the dynamic band. So
//! [`TargetId`]'s constructor set is exactly two functions, and the SAFETY comment on the hot-path
//! index names both and the argument that bounds each. It describes the set that exists, never the
//! one this file will eventually have: a safety argument that names constructors which do not
//! exist yet is a safety argument nobody can check, and one that *stops* naming a constructor that
//! has since appeared is worse — it still reads like a proof.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::level::Level;

/// Hard cap on targets. The control array is indexed by a `u8`-worth of ids and is one `u16`
/// wide in [`TargetId`] only so the band constants read in decimal.
pub const MAX_TARGETS: usize = 256;

/// One past the last engine id. Engine targets are `0..ENGINE_BAND_END`.
pub const ENGINE_BAND_END: u16 = 96;

/// First id in the dynamic band, minted at run time.
pub const DYN_BAND_START: usize = 224;

/// Number of dynamic ids.
pub const DYN_BAND_LEN: usize = MAX_TARGETS - DYN_BAND_START;

const _: () = assert!((ENGINE_BAND_END as usize) < DYN_BAND_START);
const _: () = assert!(DYN_BAND_START < MAX_TARGETS);
// L10's arithmetic, pinned rather than trusted: `TargetId::new_dynamic` maps a `DYN_NAMES` slot to
// `DYN_BAND_START + slot`, and this is what makes "slot < DYN_BAND_LEN" imply "id < MAX_TARGETS" —
// the bound `runtime_ceiling`'s `get_unchecked` rests on. Widening the band by editing one constant
// and not the other would be a soundness change with no compile error behind it.
const _: () = assert!(DYN_BAND_START + DYN_BAND_LEN == MAX_TARGETS);

/// A target's index into [`CONTROL`].
///
/// **The field is private and that is the whole safety argument.** The invariant is
/// `.0 < MAX_TARGETS`, upheld by a closed constructor set, which is what makes
/// [`runtime_ceiling`]'s unchecked index sound. A `pub` field here would make an out-of-range
/// value constructible from safe code, and the hot path would then be either a panic or
/// reachable UB depending on the profile.
///
/// There is **no `INVALID` sentinel**: absence is `Option<TargetId>`. A public in-band sentinel
/// that indexes an array is the same hazard wearing a nicer coat.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct TargetId(u16);

impl TargetId {
    /// Mint an engine-band id. `const`, and the bound is a `const` assert, so an out-of-band
    /// literal in the [`targets!`] table is a **compile error** rather than a boot check that
    /// only fires when both colliders happen to be registered.
    ///
    /// Not `pub`: the engine table is the only caller, which is what keeps the constructor set
    /// closed at this rung.
    #[inline]
    #[must_use]
    pub(crate) const fn new_engine(id: u16) -> TargetId {
        assert!(id < ENGINE_BAND_END, "invariant: an engine target id is below ENGINE_BAND_END");
        TargetId(id)
    }

    /// Mint a dynamic-band id — the SECOND constructor, and the first to widen the set *(L10)*.
    ///
    /// `slot` is an index into [`DYN_NAMES`], so the bound is `slot < DYN_BAND_LEN` and the id is
    /// `DYN_BAND_START + slot`. The `debug_assert` is not the safety argument: the argument is that
    /// the only caller is [`register_dynamic_target`], which obtains `slot` from a bounded loop
    /// over `DYN_NAMES` and can therefore produce nothing else. The `const` assert below pins the
    /// arithmetic — `DYN_BAND_START + DYN_BAND_LEN == MAX_TARGETS` — so the widening cannot move
    /// the bound `runtime_ceiling` relies on.
    ///
    /// Not `pub`, for the same reason `new_engine` is not: a public constructor taking a raw index
    /// is a public way to make an out-of-range `TargetId`, which is the hazard the private field
    /// exists to prevent.
    #[inline]
    #[must_use]
    fn new_dynamic(slot: usize) -> TargetId {
        debug_assert!(slot < DYN_BAND_LEN, "invariant: a dynamic slot indexes DYN_NAMES");
        TargetId((DYN_BAND_START + slot) as u16)
    }

    /// The raw index. Useful to artifact writers and to tests; it cannot be turned back into a
    /// `TargetId`, so handing it out does not widen the constructor set.
    #[inline]
    #[must_use]
    pub const fn index(self) -> u16 {
        self.0
    }
}

/// One packed byte per target: level, sample shift and sync route in the register the gate has
/// already loaded.
///
/// ```text
/// bit  [0..2]  level           Off | Error | Warn | Info | Debug | Trace
/// bits [3..6]  sample shift k  0 = every record; else deliver 1 in 2^k
/// bit  [7]     sync route      format on the caller, write synchronously
/// ```
///
/// **Why one byte and not three arrays.** All three knobs arrive in one `Relaxed` load, one `and`
/// and one `cmp`. A parallel shift array would cost a second load and a second cache line on the
/// *enabled* path. Zero still means level `Off`, shift 0, sync off — so the `.bss`-zero state is
/// both correct and free, and this array is logging's runtime flag word.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetControl(u8);

impl TargetControl {
    /// Mask of the level field.
    pub const LEVEL_MASK: u8 = 0b0000_0111;
    /// Mask of the sample-shift field, in place.
    pub const SHIFT_MASK: u8 = 0b0111_1000;
    /// Bit offset of the sample-shift field.
    pub const SHIFT_POS: u32 = 3;
    /// The sync-route bit.
    pub const SYNC_BIT: u8 = 0b1000_0000;
    /// Largest representable sample shift. Four bits.
    pub const MAX_SHIFT: u8 = (Self::SHIFT_MASK >> Self::SHIFT_POS);

    /// Everything off. Identical to the `.bss`-zero state, which is the point.
    pub const OFF: TargetControl = TargetControl(0);

    /// Pack the three knobs.
    ///
    /// `sample_shift` is clamped to [`MAX_SHIFT`](Self::MAX_SHIFT) rather than asserted: this is
    /// reachable from a console command and a runtime control change may not become a panic.
    /// Clamping loses precision on a knob whose whole meaning is "deliver fewer"; panicking would
    /// lose the game.
    #[inline]
    #[must_use]
    pub const fn new(level: Level, sample_shift: u8, sync_route: bool) -> TargetControl {
        let k = if sample_shift > Self::MAX_SHIFT { Self::MAX_SHIFT } else { sample_shift };
        let sync = if sync_route { Self::SYNC_BIT } else { 0 };
        TargetControl((level as u8) | (k << Self::SHIFT_POS) | sync)
    }

    /// This target's runtime ceiling.
    #[inline]
    #[must_use]
    pub const fn level(self) -> Level {
        Level::from_raw(self.0 & Self::LEVEL_MASK)
    }

    /// Deliver one record in `2^k`; `0` means every record.
    #[inline]
    #[must_use]
    pub const fn sample_shift(self) -> u8 {
        (self.0 & Self::SHIFT_MASK) >> Self::SHIFT_POS
    }

    /// Format on the caller and write synchronously.
    #[inline]
    #[must_use]
    pub const fn sync_route(self) -> bool {
        (self.0 & Self::SYNC_BIT) != 0
    }

    /// The same control with a different level, siblings preserved.
    #[inline]
    #[must_use]
    pub const fn with_level(self, level: Level) -> TargetControl {
        TargetControl((self.0 & !Self::LEVEL_MASK) | (level as u8))
    }

    /// The packed byte.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Reconstruct from a packed byte. Every one of the 256 values is a valid control — the
    /// level field is three bits and every three-bit value below 6 is a level, so the only
    /// unrepresentable states are levels 6 and 7.
    ///
    /// # Panics
    ///
    /// On a raw byte whose level field is 6 or 7. Those bytes cannot come from [`new`](Self::new)
    /// or from [`CONTROL`], so reaching this means a caller invented one.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: u8) -> TargetControl {
        // Validate rather than mask: silently reinterpreting a level of 7 as `Trace` would turn a
        // caller's bug into the most verbose setting there is.
        let _ = Level::from_raw(raw & Self::LEVEL_MASK);
        TargetControl(raw)
    }
}

// The three fields tile the byte exactly. Written as asserts because an overlap would not fail to
// compile -- it would make a sample-shift change silently move the level.
const _: () = assert!(
    TargetControl::LEVEL_MASK | TargetControl::SHIFT_MASK | TargetControl::SYNC_BIT == 0xFF
);
const _: () = assert!(TargetControl::LEVEL_MASK & TargetControl::SHIFT_MASK == 0);
const _: () = assert!(TargetControl::SHIFT_MASK & TargetControl::SYNC_BIT == 0);
const _: () = assert!(TargetControl::LEVEL_MASK & TargetControl::SYNC_BIT == 0);
const _: () = assert!(TargetControl::OFF.raw() == 0);

/// The runtime flag word: one packed byte per target.
///
/// 256 B — four cache lines, and the gate touches exactly one of them. `.bss`-zero means every
/// target is `Off`, so a process that never enables logging never writes a byte here and never
/// makes a page of it resident. **Nothing in this crate touches this array at process start.**
static CONTROL: [AtomicU8; MAX_TARGETS] = [const { AtomicU8::new(0) }; MAX_TARGETS];

/// Monotone counter, incremented on every control change.
///
/// A UI **polls** this to learn it must repaint — the `O(1)` stand-in for change detection, which
/// `CONTROL` cannot have because it is not an ECS column and must be writable from any thread
/// without a lock. It carries no state of its own, so it cannot diverge from [`CONTROL`]; it can
/// only be stale, and a stale poll costs one redundant repaint.
static CONTROL_EPOCH_CTR: AtomicU32 = AtomicU32::new(0);

/// Per-target counts. One 64-byte cell per target, 16 KiB of `.bss`.
///
/// **Not a mirror of anything**: this is the only place delivered-per-target counts exist. The
/// lanes count records, the substrate's loss cells count losses per *lane*, and neither can answer
/// "did target `rhi` ever deliver anything", which is the one question a census exists to ask.
///
/// A full cache line per target because `delivered` is written by the consumer role while
/// `dropped` is written by producers, and putting them on one line would make every drop storm
/// contend with the drain that is trying to report it.
#[repr(C, align(64))]
struct TargetStatCell {
    /// Records handed to a sink for this target. Written by the consumer role.
    delivered: AtomicU64,
    /// Records the emission path refused for this target. Written by producers.
    dropped: AtomicU64,
    /// Records the sampler chose not to deliver. Reserved for L12; always 0 here.
    sampled_out: AtomicU64,
    /// Records that took the synchronous channel instead of a lane.
    sync_routed: AtomicU64,
    _pad: [u8; 32],
}

impl TargetStatCell {
    const fn new() -> TargetStatCell {
        TargetStatCell {
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            sampled_out: AtomicU64::new(0),
            sync_routed: AtomicU64::new(0),
            _pad: [0; 32],
        }
    }
}

const _: () = assert!(core::mem::size_of::<TargetStatCell>() == 64);

static TARGET_STATS: [TargetStatCell; MAX_TARGETS] =
    [const { TargetStatCell::new() }; MAX_TARGETS];

/// One target's counts: `delivered`, `dropped`, `sampled_out`, `sync_routed`.
///
/// A snapshot, not a view — the four loads are independent, so a reader can observe a delivery
/// that happened after the drop it is compared against. That is inherent to counting without a
/// lock and it is why the census reports a *status* rather than an arithmetic identity.
#[must_use]
pub fn target_stats(id: TargetId) -> (u64, u64, u64, u64) {
    let c = &TARGET_STATS[id.index() as usize];
    (
        c.delivered.load(Ordering::Relaxed),
        c.dropped.load(Ordering::Relaxed),
        c.sampled_out.load(Ordering::Relaxed),
        c.sync_routed.load(Ordering::Relaxed),
    )
}

/// Count one record handed to a sink. Called by the consumer role, once per record.
#[inline]
pub(crate) fn count_delivered(id: TargetId) {
    TARGET_STATS[id.index() as usize].delivered.fetch_add(1, Ordering::Relaxed);
}

/// Count one record the emission path refused.
///
/// On the **cold** path only — a record that was admitted never reaches this — so the RMW is paid
/// exactly when something has already gone wrong.
#[cold]
#[inline(never)]
pub(crate) fn count_dropped(id: TargetId) {
    TARGET_STATS[id.index() as usize].dropped.fetch_add(1, Ordering::Relaxed);
}

/// Count one record that took the synchronous channel instead of a lane.
#[cold]
#[inline(never)]
pub(crate) fn count_sync_routed(id: TargetId) {
    TARGET_STATS[id.index() as usize].sync_routed.fetch_add(1, Ordering::Relaxed);
}

/// Every engine target, in declaration order, as `(id, name)`.
///
/// The census's enumeration. Dynamic targets are L10's and are absent here rather than reported as
/// zero — a census row for a target that cannot exist yet is the vacuous row this vocabulary was
/// invented to prevent.
pub fn engine_targets() -> impl Iterator<Item = (TargetId, &'static str)> {
    ENGINE_TARGET_IDS
        .iter()
        .zip(ENGINE_TARGET_NAMES.iter())
        .map(|(&id, &name)| (TargetId(id), name))
}

/// A log target: a compile-time-unique id, a name, and a compile-time ceiling.
///
/// All three are `const` because the emission macro's first gate is `T::STATIC_CEILING >= LVL`,
/// and a gate only folds if its operands are constants.
pub trait LogTarget: 'static {
    /// Printed name, and the key a control spec addresses this target by.
    const NAME: &'static str;
    /// Index into [`CONTROL`].
    const ID: TargetId;
    /// The compile-time ceiling. A site above it is deleted **with its argument expressions**.
    const STATIC_CEILING: Level;
}

/// This target's runtime ceiling as a raw level, for the third gate.
///
/// One `Relaxed` load and one `and`. `Relaxed` is correct and not a shortcut: the byte is the
/// whole datum, a torn read is impossible, and there is nothing for it to be ordered against — a
/// control change that lands one record later is exactly the documented behaviour.
#[inline]
#[must_use]
pub fn runtime_ceiling(id: TargetId) -> u8 {
    // SAFETY: `TargetId`'s field is private and its constructor set is closed at TWO functions,
    //   each of which establishes `.0 < MAX_TARGETS` by its own argument:
    //     * `new_engine(id)` is `const` and carries `assert!(id < ENGINE_BAND_END)`, and
    //       `ENGINE_BAND_END < MAX_TARGETS` is a `const` assert above.
    //     * `new_dynamic(slot)` maps `slot` to `DYN_BAND_START + slot` and is called only with a
    //       `slot < DYN_BAND_LEN` (every caller indexes `DYN_NAMES`, whose length IS that
    //       constant), so `DYN_BAND_START + DYN_BAND_LEN == MAX_TARGETS` -- the third `const`
    //       assert above -- is what bounds it. That equality is pinned rather than trusted
    //       precisely because it is load-bearing HERE and nowhere near this line.
    //   There is no `INVALID` sentinel and no public constructor, so safe code cannot produce an
    //   out-of-range value. A THIRD constructor (the downstream band, L11a) MUST re-establish the
    //   bound and be named here -- this comment names two because there are two.
    //
    //   L10 widened the set, and the widening is what this comment is now for: the version that
    //   said "`new_engine` is its ONLY constructor" was true when written and would have gone on
    //   reading like a proof after it stopped being one.
    let cell = unsafe { CONTROL.get_unchecked(id.0 as usize) };
    cell.load(Ordering::Relaxed) & TargetControl::LEVEL_MASK
}

/// This target's full control byte.
#[inline]
#[must_use]
pub fn target_control(id: TargetId) -> TargetControl {
    // SAFETY: as `runtime_ceiling` -- the same closed constructor set bounds the index.
    let cell = unsafe { CONTROL.get_unchecked(id.0 as usize) };
    TargetControl(cell.load(Ordering::Relaxed))
}

/// Replace this target's control byte wholesale.
///
/// A plain store, not a CAS: the caller supplies all three fields, so there is nothing of this
/// target's to preserve, and neighbouring targets live in their own bytes. Use
/// [`set_target_level`] when only the level is meant to move.
#[inline]
pub fn set_target_control(id: TargetId, ctl: TargetControl) {
    // SAFETY: as `runtime_ceiling`.
    let cell = unsafe { CONTROL.get_unchecked(id.0 as usize) };
    cell.store(ctl.raw(), Ordering::Relaxed);
    bump_control_epoch();
}

/// Move this target's level, preserving the sample shift and the sync route.
///
/// A CAS loop, because the sibling bit-fields belong to whoever set them and a read-modify-write
/// through a plain store would lose a concurrent sampling change. This is the operation a console
/// command and a dev menu call, so "concurrent" is not hypothetical.
#[inline]
pub fn set_target_level(id: TargetId, level: Level) {
    // SAFETY: as `runtime_ceiling`.
    let cell = unsafe { CONTROL.get_unchecked(id.0 as usize) };
    let mut cur = cell.load(Ordering::Relaxed);
    loop {
        let next = TargetControl(cur).with_level(level).raw();
        match cell.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => cur = observed,
        }
    }
    bump_control_epoch();
}

/// The control-change counter. A consumer that stored a previous value and sees a different one
/// knows some control moved; it does not learn which, which is why this is a repaint trigger and
/// not a diff.
///
/// **Not a clock epoch** (`boyko_diag::clock::clock_epoch`) and **not a flush sequence**. Three
/// unrelated counters were called "epoch" across the two plans; this one is the control one and
/// its name says so.
#[inline]
#[must_use]
pub fn control_epoch() -> u32 {
    CONTROL_EPOCH_CTR.load(Ordering::Acquire)
}

/// `Release` so a poller that observes the new epoch also observes the `CONTROL` write that
/// caused it. Wrapping at 2³² is fine and is the reason the API is "different, therefore repaint"
/// rather than "greater, therefore newer".
#[inline]
fn bump_control_epoch() {
    CONTROL_EPOCH_CTR.fetch_add(1, Ordering::Release);
}

/// Declare the engine target table.
///
/// Ids must be **strictly increasing**, which is checked by a `const` assert over the whole table:
/// a duplicate or an out-of-order row does not compile. That is the replacement for a boot-time
/// collision check, which could only fire if *both* colliders happened to be registered — and
/// nothing forced registration, so an unregistered target still gated against its byte and never
/// tripped anything.
macro_rules! targets {
    ($( ($id:expr, $Ty:ident, $name:literal, $ceiling:expr) ),* $(,)?) => {
        $(
            #[doc = concat!("Log target `", $name, "`.")]
            #[derive(Clone, Copy, Debug)]
            pub struct $Ty;

            impl $crate::target::LogTarget for $Ty {
                const NAME: &'static str = $name;
                const ID: $crate::target::TargetId = $crate::target::TargetId::new_engine($id);
                const STATIC_CEILING: $crate::level::Level = $ceiling;
            }
        )*

        /// Every engine target id, in declaration order. Read by the `const` assert below and by
        /// the table's own test; not part of the public surface.
        const ENGINE_TARGET_IDS: &[u16] = &[$($id),*];

        /// Every engine target name, in declaration order. Used to prove the names are distinct —
        /// ids being unique does not make names unique, and a control spec addresses by name.
        const ENGINE_TARGET_NAMES: &[&str] = &[$($name),*];

        const _: () = assert!(
            ids_strictly_increasing(ENGINE_TARGET_IDS),
            "engine target ids must be strictly increasing: a duplicate or an out-of-order row"
        );
        const _: () = assert!(
            names_distinct(ENGINE_TARGET_NAMES),
            "engine target names must be non-empty and distinct: a control spec addresses by name"
        );
    };
}

/// `true` when every id is greater than the one before it. A `const fn` so the table's uniqueness
/// is a compile error rather than a test that has to be remembered.
const fn ids_strictly_increasing(ids: &[u16]) -> bool {
    let mut i = 1;
    while i < ids.len() {
        if ids[i - 1] >= ids[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `true` when every name is non-empty and no two are equal.
///
/// Distinct **ids** do not make names distinct, and a control spec — a console command, a config
/// file — addresses a target by name. Two rows sharing a name means one of them is unreachable
/// from every such spec, silently. Quadratic over a table of tens, evaluated once at compile time.
const fn names_distinct(names: &[&str]) -> bool {
    let mut i = 0;
    while i < names.len() {
        if names[i].is_empty() {
            return false;
        }
        let mut j = i + 1;
        while j < names.len() {
            if bytes_eq(names[i].as_bytes(), names[j].as_bytes()) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// `const`-evaluable byte-slice equality. `PartialEq` is not `const`, so this is the only way to
/// compare two `&'static str` before run time.
const fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// ─────────────────────────────── the engine table ────────────────────────────────
//
// One row per domain in the diagnostic-code block map, so a reader who knows a code's block knows
// its target without a second table to consult. Ids are dense and in block order; they are never
// renumbered, so a domain that later splits takes the next free id rather than shifting its
// neighbours.
//
// **Every ceiling is `Trace`, and that is a decision.** `STATIC_CEILING` is the COMPILE-time
// ceiling: setting `Ecs` to `Info` would mean no `debug!(Ecs, …)` can exist in ANY build, forever,
// and un-guessing it is a source edit rather than a config change. Before a single call site
// exists there is no evidence for lowering any of them, and the two axes that *should* be doing
// this work already are: the per-profile `GLOBAL_CEILING` deletes `debug!`/`trace!` from every
// shipping build, and the runtime byte turns targets off in a dev one. A per-target compile
// ceiling is for a target measured to be noisy even in dev, which is a fact this rung cannot have.
// The specification's illustrative rows show `Info`/`Info`/`Warn`; they are illustrative, and this
// deviation is recorded rather than taken silently.
targets! {
    (0,  Ecs,          "ecs",          Level::Trace),
    (1,  Log,          "log",          Level::Trace),
    (2,  Threadpool,   "threadpool",   Level::Trace),
    (3,  Memory,       "memory",       Level::Trace),
    (4,  Components,   "components",   Level::Trace),
    (5,  Query,        "query",        Level::Trace),
    (6,  ChangeDetect, "changedetect", Level::Trace),
    (7,  Events,       "events",       Level::Trace),
    (8,  Assets,       "assets",       Level::Trace),
    (9,  Serialize,    "serialize",    Level::Trace),
    (10, Input,        "input",        Level::Trace),
    (11, Scene,        "scene",        Level::Trace),
    (12, Physics,      "physics",      Level::Trace),
    (13, MathSdf,      "mathsdf",      Level::Trace),
    (14, Schedule,     "schedule",     Level::Trace),
    (15, App,          "app",          Level::Trace),
    (16, Rhi,          "rhi",          Level::Trace),
    (17, RhiVulkan,    "rhivulkan",    Level::Trace),
    (18, Render,       "render",       Level::Trace),
    (19, ShaderDsl,    "shaderdsl",    Level::Trace),
    (20, Ui,           "ui",           Level::Trace),
    (21, Fontbake,     "fontbake",     Level::Trace),
    (22, Image,        "image",        Level::Trace),
    (23, GpuColumns,   "gpucolumns",   Level::Trace),
    (24, Host,         "host",         Level::Trace),
    (25, Profiling,    "profiling",    Level::Trace),
    // L8b, and the one row in this table that is NOT an engine domain.
    //
    // `boyko_demo` is a downstream application, so its natural home is the DOWNSTREAM band
    // (`96..=223`, `define_target!`) described at the top of this file. That band does not exist
    // yet -- it is L11a's -- and the ledger pins `boyko_demo`'s one migrated site as
    // `error!(Demo, codes::E3001, ..)`, which needs a target today. It takes the next engine id
    // and MOVES when the band lands; recorded here rather than left for a later reader to
    // discover, because an engine-band row for a non-engine crate is exactly the kind of thing
    // that stops looking like a deviation once it has sat in a table for a while.
    (26, Demo,         "demo",         Level::Trace),
}

// ─────────────────────────── L10: the dynamic band ───────────────────────────

/// Longest dynamic target name, in bytes.
///
/// Not a taste decision: [`DynSlot`] is one cache line, and 64 − 8 (`hash`) − 1 (`len`) leaves
/// exactly this. A name is a category like `"mod:acme_weapons"`; 47 bytes is generous for that, and
/// the alternative — a heap string — would put an allocation on a table whose whole point is that
/// it makes none.
pub const MAX_DYN_NAME: usize = 47;

/// One interned dynamic-target name. **One cache line, `.bss`, never freed.**
///
/// # The publication order, and the contradiction in the specification it resolves
///
/// The corpus states two things about this slot that cannot both hold as written:
///
/// 1. *"`bytes`/`len` are written before `hash.store(h, Release)`; a reader that observes a
///    non-zero hash via `Acquire` observes the completed name."*
/// 2. *"A slot's hash transitions `0 -> h` exactly once, by CAS."*
///
/// If the CAS is on `hash`, a writer cannot have written the bytes first — it has not yet claimed
/// the slot, so writing them would race another claimant. The two clauses describe different
/// mechanisms and the file has to pick one.
///
/// **The claim moves to `len`.** A writer knows the name's length before it starts, so
/// `len.compare_exchange(0, n)` is a claim that carries information rather than a bare flag. Then:
/// bytes are written under that exclusive claim, and `hash.store(h, Release)` publishes. Clause 1
/// holds exactly; clause 2 holds in substance — only the claimant ever stores the hash, so the
/// transition still happens once — and the CAS that guarantees it has moved one field over. Written
/// down here rather than silently reinterpreted.
///
/// `len == 0` therefore means "free", which is why an empty name is refused: it could not claim.
#[repr(C, align(64))]
struct DynSlot {
    /// The name's hash, or `0` for "not published yet". Published `Release`, read `Acquire`.
    hash: AtomicU64,
    /// The name's length in bytes, or `0` for "unclaimed". **The claim word.**
    len: AtomicU8,
    /// The name. Valid for `len` bytes once `hash` is non-zero.
    bytes: UnsafeCell<[u8; MAX_DYN_NAME]>,
}

// SAFETY: every field is either an atomic or is protected by one.
//   * `len` is the claim: exactly one thread wins its `compare_exchange` from 0, and only that
//     thread writes `bytes`. No slot is ever released, so the claim is permanent and there is no
//     ABA to consider.
//   * `bytes` is written ONLY by the claimant, and only before its `hash.store(.., Release)`. A
//     reader that observes a non-zero `hash` with `Acquire` has therefore observed those writes.
//   * A reader that observes `hash == 0` treats the slot as absent and never reads `bytes`.
//   * Nothing is ever freed, so no reader can observe a dangling name.
unsafe impl Sync for DynSlot {}

impl DynSlot {
    const fn new() -> DynSlot {
        DynSlot {
            hash: AtomicU64::new(0),
            len: AtomicU8::new(0),
            bytes: UnsafeCell::new([0; MAX_DYN_NAME]),
        }
    }
}

const _: () = assert!(core::mem::size_of::<DynSlot>() == 64);

/// The interning table: 32 slots, 2 KiB of `.bss`.
///
/// **Not a map.** No rehash, no growth, no allocation — and the emission path never touches it:
/// a site carries a [`TargetId`], and the name is resolved by the sink. A flag-off run never reads
/// or writes a byte of this.
static DYN_NAMES: [DynSlot; DYN_BAND_LEN] = [const { DynSlot::new() }; DYN_BAND_LEN];

/// FNV-1a over the name, never zero.
///
/// Zero is the "unpublished" sentinel, so a name that hashes to it must not be storable as itself.
/// Forcing the low bit is one instruction and biases nothing that matters here — the table is
/// open-addressed over 32 slots, and the hash is a probe seed, not a distribution guarantee.
fn dyn_hash(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h | 1
}

/// The name in `slot`, or `None` if it is unpublished.
///
/// Reads `hash` with `Acquire` FIRST — that load is the whole synchronisation, and touching `len`
/// or `bytes` before it would be reading memory the writer may still be filling.
fn dyn_name_of(slot: usize) -> Option<&'static str> {
    let s = &DYN_NAMES[slot];
    if s.hash.load(Ordering::Acquire) == 0 {
        return None;
    }
    let n = (s.len.load(Ordering::Relaxed) as usize).min(MAX_DYN_NAME);
    // SAFETY: the `Acquire` above observed a non-zero hash, which the claimant stored with
    //   `Release` AFTER writing `len` and the first `n` bytes — so those writes are visible here.
    //   A slot is claimed once and never released, so no writer can be active now and the slice
    //   cannot alias one. `n` is clamped to the array's length above, and `get()` is a valid
    //   pointer to a live `.bss` array for the process lifetime.
    //
    //   `from_raw_parts` over a `*const u8` rather than `&(*get())[..n]`: the latter takes an
    //   implicit reference to the WHOLE array through the raw pointer, which asserts validity and
    //   aliasing over all 47 bytes including the ones past `n` that no writer ever touched.
    let bytes = unsafe { core::slice::from_raw_parts(s.bytes.get().cast::<u8>(), n) };
    core::str::from_utf8(bytes).ok()
}

/// Register a dynamic target by name, or `None` when the band is full.
///
/// **Cold, setup-time, and idempotent by name**: registering `"mod:acme"` twice returns the same
/// [`TargetId`], which is what lets two independently-loaded mods name one category without
/// coordinating. `initial` is applied only on the FIRST registration — a second caller does not get
/// to re-open a target the first one configured.
///
/// # `None` has THREE meanings, and the corpus documents one
///
/// There is no in-band sentinel to hand back — absence is the `Option` — but the corpus's signature
/// comment reads *"`None` => band exhausted"*, and this function returns `None` for three different
/// reasons: the band is full, the name is empty, or the name is longer than [`MAX_DYN_NAME`]. A
/// caller who read only that comment would see a rejected 60-byte name and conclude the band was
/// gone, stop registering, and lose every target after it.
///
/// So **`boyko-E0106` covers all three and carries the reason as an argument.** One code rather
/// than three, because they are one function's one failure return and a caller cannot act
/// differently on them; the reason string is what tells a reader which one happened. The registry
/// summary is read from the message this function actually prints, per the registry's own rule.
///
/// `RatePolicy::Every`, not `Once`: past exhaustion *every* later registration fails, and each
/// failure is a different mod whose logging is now silently absent. `Once` would name the first
/// victim and hide the rest — the same argument `W0901` records for its component types.
///
/// This function EMITS, which is unusual for `boyko_log`'s own internals — `W0103` writes through
/// `sync_out` because the file sink cannot log into itself. Nothing like that applies here: the
/// dynamic band sits beside the emission path, not underneath it, and the target it emits on
/// (`Log`) is a static engine row whose ceiling this call cannot have disturbed. A default run
/// still shows nothing, exactly as every other migrated site does.
///
/// # Why it waits on a claimed-but-unpublished slot
///
/// The probe walks `len` (the claim), not `hash` (the publication), because a slot that is claimed
/// and not yet published is **occupied** — and a prober that skipped it would walk past a name
/// being written and mint a SECOND id for it, breaking the one property this function sells. So on
/// meeting a claimed slot whose hash has not landed, it waits. The wait is bounded by one `memcpy`
/// of at most 47 bytes on another core; this function is `#[cold]`, runs at setup, and the
/// alternative — a lock — would be the allocation-free table's first.
#[cold]
#[inline(never)]
pub fn register_dynamic_target(name: &str, initial: TargetControl) -> Option<TargetId> {
    let n = name.len();
    if n == 0 || n > MAX_DYN_NAME {
        // A zero-length name could not claim a slot (`len == 0` IS the free marker), and an
        // over-long one does not fit the line. Both are caller errors, refused the way exhaustion
        // is — `None`, with no partial state written.
        let why = if n == 0 {
            "the name is empty"
        } else {
            "the name is longer than 47 bytes"
        };
        crate::error!(
            crate::Log,
            crate::codes::E0106.number(),
            "dynamic target {} refused: {}",
            name,
            why
        );
        return None;
    }
    let h = dyn_hash(name);
    let start = (h as usize) % DYN_BAND_LEN;

    for probe in 0..DYN_BAND_LEN {
        let slot = (start + probe) % DYN_BAND_LEN;
        let s = &DYN_NAMES[slot];

        if s.len.compare_exchange(0, n as u8, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            // SAFETY: the CAS above is the exclusive claim on this slot, and a slot is never
            //   released, so this thread is its only writer for the process lifetime. `n` was
            //   bounded by `MAX_DYN_NAME` before the claim, so the destination range is inside the
            //   array. Source and destination cannot overlap: one is the caller's `&str`, the other
            //   is this crate's `.bss`.
            //
            //   `copy_nonoverlapping` rather than `(*get())[..n].copy_from_slice(..)`, which would
            //   take an implicit reference to the whole array through the raw pointer.
            unsafe {
                core::ptr::copy_nonoverlapping(name.as_ptr(), s.bytes.get().cast::<u8>(), n);
            }
            // Publishes `len` and `bytes` to every `Acquire` reader of `hash`.
            s.hash.store(h, Ordering::Release);
            let id = TargetId::new_dynamic(slot);
            set_target_control(id, initial);
            return Some(id);
        }

        // Occupied. Wait for its name to land, then compare — see the doc above for why skipping a
        // claimed-but-unpublished slot would break idempotency.
        loop {
            match dyn_name_of(slot) {
                Some(existing) if existing == name => return Some(TargetId::new_dynamic(slot)),
                Some(_) => break,
                None => core::hint::spin_loop(),
            }
        }
    }
    // Every slot walked, none free, none holding this name.
    crate::error!(
        crate::Log,
        crate::codes::E0106.number(),
        "dynamic target {} refused: {}",
        name,
        "the 32-slot band is full"
    );
    None
}

/// The id registered for `name`, dynamic or engine, or `None`.
///
/// A linear scan rather than a probe, and the difference is deliberate: a probe would have to
/// reproduce the insert path's collision walk exactly, and two implementations of one addressing
/// scheme is one more than can be kept in agreement. `#[cold]`, over 32 slots, from settings
/// screens and console commands.
///
/// Engine names are searched too, because a console command's user does not know which band a
/// target lives in and should not have to.
#[cold]
#[inline(never)]
#[must_use]
pub fn find_target(name: &str) -> Option<TargetId> {
    for slot in 0..DYN_BAND_LEN {
        if dyn_name_of(slot) == Some(name) {
            return Some(TargetId::new_dynamic(slot));
        }
    }
    engine_targets().find(|(_, n)| *n == name).map(|(id, _)| id)
}

/// Every target that exists right now — engine rows first, then registered dynamic ones.
///
/// For a settings screen or a console `targets` command: the caller wants one list, and which band
/// an id came from is not a distinction a player has any use for. Unregistered dynamic slots are
/// **absent** rather than listed blank — a row for a target that does not exist is the vacuous row
/// this vocabulary was invented to prevent.
#[cold]
#[inline(never)]
pub fn targets() -> impl Iterator<Item = (TargetId, &'static str)> {
    engine_targets().chain(
        (0..DYN_BAND_LEN)
            .filter_map(|slot| dyn_name_of(slot).map(|n| (TargetId::new_dynamic(slot), n))),
    )
}

/// How many dynamic slots are taken — the census's question, and `boyko-E0106`'s subject.
#[must_use]
pub fn dyn_registered() -> usize {
    (0..DYN_BAND_LEN).filter(|&s| dyn_name_of(s).is_some()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids this module writes to.
    ///
    /// `cargo test` runs these concurrently **in one process**, and `CONTROL` is process-global,
    /// so two tests sharing a row would make the suite flaky by construction — a defect this
    /// campaign has already shipped once and caught by measurement rather than by reading. No two
    /// tests below write the same id, and `a_fresh_process_has_every_other_target_off` skips
    /// exactly this list.
    const MUTATED: &[u16] = &[
        1, // Log       -- set_target_level_preserves_the_sibling_fields
        5, // Query     -- control_epoch_moves_on_every_change
        7, // Events    -- writing_one_target_does_not_disturb_its_neighbours
    ];

    #[test]
    fn ids_strictly_increasing_rejects_what_it_must() {
        // The `const` assert over the real table can only be observed by failing to compile, so
        // the checker itself is exercised here on inputs the table must never contain.
        assert!(ids_strictly_increasing(&[]));
        assert!(ids_strictly_increasing(&[7]));
        assert!(ids_strictly_increasing(&[0, 1, 2]));
        assert!(!ids_strictly_increasing(&[0, 0]), "a duplicate must be rejected");
        assert!(!ids_strictly_increasing(&[1, 0]), "out of order must be rejected");
        assert!(!ids_strictly_increasing(&[0, 2, 1, 3]));
    }

    #[test]
    fn names_distinct_rejects_what_it_must() {
        // Same reasoning as above: the real table's check is a compile error, so the checker is
        // exercised here on the inputs the table must never contain.
        assert!(names_distinct(&[]));
        assert!(names_distinct(&["a"]));
        assert!(names_distinct(&["a", "b", "ab"]));
        assert!(!names_distinct(&["a", "a"]), "a duplicate name must be rejected");
        assert!(!names_distinct(&["a", ""]), "an empty name must be rejected");
        assert!(!names_distinct(&["x", "y", "x"]), "a non-adjacent duplicate must be rejected");
    }

    #[test]
    fn engine_table_is_in_band() {
        assert_eq!(ENGINE_TARGET_IDS.len(), ENGINE_TARGET_NAMES.len());
        for &id in ENGINE_TARGET_IDS {
            assert!(id < ENGINE_BAND_END, "engine id {id} is out of band");
        }
    }

    #[test]
    fn a_fresh_process_has_every_other_target_off() {
        // The `.bss`-zero property, observed rather than asserted about the linker: nothing in
        // this crate runs at process start, so every byte a test has not written is still the
        // loader's zero. The three rows this module writes are skipped BY ID -- see `MUTATED`.
        for &id in ENGINE_TARGET_IDS {
            if MUTATED.contains(&id) {
                continue;
            }
            let t = TargetId::new_engine(id);
            assert_eq!(target_control(t), TargetControl::OFF, "target id {id} was written");
            assert_eq!(runtime_ceiling(t), Level::Off as u8);
        }
    }

    #[test]
    fn packing_round_trips_all_three_fields() {
        for lvl in [Level::Off, Level::Error, Level::Warn, Level::Info, Level::Debug, Level::Trace] {
            for k in 0..=TargetControl::MAX_SHIFT {
                for sync in [false, true] {
                    let c = TargetControl::new(lvl, k, sync);
                    assert_eq!(c.level(), lvl);
                    assert_eq!(c.sample_shift(), k);
                    assert_eq!(c.sync_route(), sync);
                    assert_eq!(TargetControl::from_raw(c.raw()), c);
                }
            }
        }
    }

    #[test]
    fn shift_is_clamped_not_wrapped() {
        // A wrap would turn "sample very rarely" into "sample every record" -- the opposite of
        // what the caller asked for, and silently.
        let c = TargetControl::new(Level::Info, 200, false);
        assert_eq!(c.sample_shift(), TargetControl::MAX_SHIFT);
        assert_eq!(c.level(), Level::Info, "clamping must not disturb the level");
    }

    #[test]
    fn set_target_level_preserves_the_sibling_fields() {
        let t = <Log as LogTarget>::ID;
        set_target_control(t, TargetControl::new(Level::Warn, 5, true));
        set_target_level(t, Level::Debug);

        let c = target_control(t);
        assert_eq!(c.level(), Level::Debug);
        assert_eq!(c.sample_shift(), 5, "the CAS must preserve the sample shift");
        assert!(c.sync_route(), "the CAS must preserve the sync route");

        set_target_control(t, TargetControl::OFF);
    }

    #[test]
    fn control_epoch_moves_on_every_change() {
        let t = <Query as LogTarget>::ID;
        let before = control_epoch();
        set_target_control(t, TargetControl::new(Level::Info, 0, false));
        let after_store = control_epoch();
        set_target_level(t, Level::Warn);
        let after_cas = control_epoch();

        assert_ne!(before, after_store, "a wholesale set must move the epoch");
        assert_ne!(after_store, after_cas, "a level change must move the epoch");

        set_target_control(t, TargetControl::OFF);
    }

    #[test]
    fn writing_one_target_does_not_disturb_its_neighbours() {
        let t = <Events as LogTarget>::ID;
        let lo = TargetId::new_engine(t.index() - 1);
        let hi = TargetId::new_engine(t.index() + 1);

        set_target_control(t, TargetControl::new(Level::Trace, 3, true));
        assert_eq!(target_control(lo), TargetControl::OFF);
        assert_eq!(target_control(hi), TargetControl::OFF);

        set_target_control(t, TargetControl::OFF);
    }
}
