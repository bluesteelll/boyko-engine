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
//! | Dynamic | `224..=255` | minted at run time from a name; the mint **is** the proof (a later rung) |
//!
//! **At this rung only the first band exists**, so [`TargetId`]'s constructor set is exactly one
//! function, and the SAFETY comment on the hot-path index says so rather than describing the set
//! it will eventually have. A safety argument that names constructors which do not exist yet is a
//! safety argument nobody can check.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

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
    // SAFETY: `TargetId`'s field is private and `TargetId::new_engine` is its ONLY constructor at
    //   this rung. That constructor is `const` and carries `assert!(id < ENGINE_BAND_END)`, and
    //   `ENGINE_BAND_END < MAX_TARGETS` is a `const` assert above, so every `TargetId` that can
    //   exist indexes inside `CONTROL`. There is no `INVALID` sentinel and no public constructor,
    //   so safe code cannot produce an out-of-range value. Widening the constructor set (the
    //   downstream and dynamic bands, later rungs) MUST re-establish the bound at each new
    //   constructor -- this comment names one because there is one.
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
