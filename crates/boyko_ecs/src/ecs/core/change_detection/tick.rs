//! [`Tick`] — monotonic per-frame counter for Phase 10 change detection.
//!
//! See Phase 10 plan §2.1 TICK1-TICK8, §4.2, §9.3 (first-principles
//! `MAX_CHANGE_AGE` derivation), and §13.1 (mandatory unit tests including
//! the Round 3 C-NEW-1 inclusive-upper-bound regression).
//!
//! # Wave A scope
//!
//! This module ships only the [`Tick`] newtype plus the
//! [`MAX_CHANGE_AGE`] / [`CHECK_TICK_THRESHOLD`] constants. The wraparound
//! scan (`check_ticks`) and the `Mut<T>` / `Ref<T>` consumers land in
//! Waves B-D.

/// Frame interval between `check_ticks` wraparound scans.
///
/// Mirrors Bevy's value (`bevy_ecs/src/component.rs`). The world now advances
/// ~2 [`Tick`]s per `Schedule::run` (the frame-start bump plus the Bug #56
/// apply-window bump), so `CHECK_TICK_THRESHOLD = 518_400_000` translates to
/// one scan every ~259.2 M frames ≈ **~50 days of continuous play at 60 FPS** —
/// still within the wraparound headroom proved for `MAX_CHANGE_AGE` below
/// (§9.3, §10.6).
pub const CHECK_TICK_THRESHOLD: u32 = 518_400_000;

/// Maximum age (in ticks) any stored tick may have relative to the world's
/// current tick before `check_ticks` MUST clamp it.
///
/// # First-principles derivation (Phase 10 plan §9.3, Round 2 W2)
///
/// `Tick::is_newer_than` interprets `this_run.wrapping_sub(stored)` as the
/// "real" elapsed-tick count under wraparound. The mapping is faithful iff
/// the actual elapsed count between the OLDEST live tick and the current
/// tick stays strictly below `u32::MAX`.
///
/// Between two consecutive `check_ticks` scans the world's tick advances
/// by at most `CHECK_TICK_THRESHOLD`. The clamp guarantees every stored
/// tick has age `≤ MAX_CHANGE_AGE` immediately after a scan. Just before
/// the next scan the worst-case age is `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD`,
/// which the formula keeps `< u32::MAX`:
///
/// ```text
/// MAX_CHANGE_AGE = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)
///                = 4_294_967_295 - 1_036_799_999
///                ≈ 3_258_167_296
///
/// MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD ≈ 3_776_567_296   < u32::MAX ✓
/// ```
///
/// Equivalently, `MAX_CHANGE_AGE ≈ 3/4 · u32::MAX`. Headroom for boyko's
/// per-frame bump regime is enormous: at 60 FPS the scan fires every ~100
/// days; at 144 FPS every ~42 days.
pub const MAX_CHANGE_AGE: u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1);

/// Compile-time gate for the plan §9.3 wraparound safety inequality:
/// `MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD < u32::MAX`. A future tweak
/// to either constant that violates the inequality fails the build at
/// this site rather than producing silently incorrect `is_newer_than`
/// results months down the line.
const _: () = {
    assert!(MAX_CHANGE_AGE.wrapping_add(CHECK_TICK_THRESHOLD) < u32::MAX);
    assert!(CHECK_TICK_THRESHOLD < MAX_CHANGE_AGE);
};

/// Monotonic change-detection counter.
///
/// `Tick` is a `u32` newtype with wrapping arithmetic; comparisons use
/// [`Tick::is_newer_than`] which interprets `wrapping_sub` differences via
/// the standard Bevy signed-comparison-via-wraparound technique. All stored
/// ticks must stay within [`MAX_CHANGE_AGE`] of the world's current tick;
/// the (future) `check_ticks` scan preserves the bound by clamping any
/// aged-out value to `current - MAX_CHANGE_AGE`.
///
/// # Layout
///
/// `#[repr(transparent)]` over `u32`. Size 4 B, alignment 4 B, no padding —
/// see plan §11.1.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Tick(u32);

impl Tick {
    /// Sentinel "never-written" value.
    ///
    /// Per plan §2.1 TICK8 the value `Tick::ZERO` MUST NOT appear as a
    /// meaningful comparand on the hot path: `SystemMeta::new(name,
    /// current_tick)` (Round 2 W5) initialises `last_run = current_tick -
    /// MAX_CHANGE_AGE`, guaranteeing every per-row tick stored at or after
    /// initialisation either equals `current_tick` (a real write) or stays
    /// at `Tick::ZERO` (an unused buffer slot — and slots below
    /// `pool.count()` are written before being read; see plan §2.2 STORE10).
    pub const ZERO: Tick = Tick(0);

    /// Constructs a [`Tick`] from a raw `u32` counter value.
    #[inline]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the underlying counter value.
    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Returns `true` iff `self` falls in the half-open window
    /// `(last_run, this_run]` accounting for u32 wraparound.
    ///
    /// # Boundary semantics
    ///
    /// * **Inclusive upper bound**: `self == this_run` returns `true` —
    ///   a write performed within the current frame is observable to a
    ///   reader using `this_run` as its observation horizon.
    /// * **Exclusive lower bound**: `self == last_run` returns `false` —
    ///   ticks at exactly `last_run` are considered "as of the previous
    ///   observation" and not newer.
    ///
    /// # Implementation (Round 3 C-NEW-1 corrected formula)
    ///
    /// Standard signed-comparison-via-wraparound technique, mirroring
    /// Bevy's `bevy_ecs/src/component.rs::Tick::is_newer_than`:
    ///
    /// ```text
    /// ticks_since_insert = this_run.wrapping_sub(self)
    /// ticks_since_system = this_run.wrapping_sub(last_run)
    /// ticks_since_system > ticks_since_insert
    /// ```
    ///
    /// Both subtractions have `this_run` as the minuend — the Round 2
    /// `age_self > age_this` form (both minuends = `last_run`) collapsed
    /// `self == this_run` to `false`, breaking the inclusive upper bound;
    /// see plan §0 changelog C-NEW-1.
    ///
    /// # Worked examples
    ///
    /// Under bounded `MAX_CHANGE_AGE`:
    /// * `self=10, last=2, this=10`: `since_insert=0, since_system=8 → true`.
    /// * `self=2,  last=2, this=10`: `since_insert=8, since_system=8 → false`.
    /// * `self=5,  last=2, this=10`: `since_insert=5, since_system=8 → true`.
    /// * `self=11, last=2, this=10`: `since_insert = u32::MAX → false`.
    #[inline]
    pub fn is_newer_than(self, last_run: Tick, this_run: Tick) -> bool {
        let ticks_since_insert = this_run.0.wrapping_sub(self.0);
        let ticks_since_system = this_run.0.wrapping_sub(last_run.0);
        ticks_since_system > ticks_since_insert
    }

    /// Clamps `self` to be no older than [`MAX_CHANGE_AGE`] ticks behind
    /// `current`.
    ///
    /// Returns a new [`Tick`]: if `current.wrapping_sub(self) > MAX_CHANGE_AGE`
    /// the result is `current.wrapping_sub(MAX_CHANGE_AGE)` (the oldest
    /// still-valid tick); otherwise `self` is returned unchanged.
    ///
    /// Wave B will add a `&mut self` variant (`check_tick(&mut self,
    /// current: Tick) -> bool`) for the in-place `check_ticks` scan.
    #[inline]
    pub fn check_tick(self, current: Tick) -> Tick {
        let age = current.0.wrapping_sub(self.0);
        if age > MAX_CHANGE_AGE {
            Tick(current.0.wrapping_sub(MAX_CHANGE_AGE))
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 2 O4 — `Tick::default()` returns [`Tick::ZERO`].
    #[test]
    fn tick_default_is_zero() {
        assert_eq!(Tick::default(), Tick::ZERO);
        assert_eq!(Tick::ZERO.get(), 0);
    }

    /// Basic in-range compare with no wraparound.
    #[test]
    fn tick_is_newer_than_simple_in_range() {
        // self=5 ∈ (2, 10] — true.
        assert!(Tick(5).is_newer_than(Tick(2), Tick(10)));
    }

    /// Lower bound is exclusive: `self == last_run` returns `false`.
    #[test]
    fn tick_is_newer_than_lower_bound_exclusive() {
        assert!(!Tick(2).is_newer_than(Tick(2), Tick(10)));
    }

    /// **Round 3 C-NEW-1 regression test (mandatory — see plan §13.1).**
    ///
    /// Upper bound is INCLUSIVE: `self == this_run` MUST return `true`.
    /// The Round 2 erroneous formula `age_this > age_self` returned
    /// `false` here; this test pins the corrected semantic.
    #[test]
    fn tick_is_newer_than_self_equal_this_run() {
        assert!(Tick(10).is_newer_than(Tick(2), Tick(10)));
    }

    /// Ticks strictly before `last_run` and strictly after `this_run` are
    /// both rejected.
    #[test]
    fn tick_is_newer_than_outside_range() {
        assert!(!Tick(1).is_newer_than(Tick(2), Tick(10)));
        // self > this_run is impossible in a well-formed scenario; the
        // wrapping_sub interpretation yields a huge unsigned value > the
        // window size, so the compare returns false.
        assert!(!Tick(15).is_newer_than(Tick(2), Tick(10)));
    }

    /// `check_tick` clamps a tick whose age exceeds [`MAX_CHANGE_AGE`].
    #[test]
    fn tick_check_tick_clamps_aged_out() {
        let current = Tick(MAX_CHANGE_AGE.wrapping_add(100));
        let old = Tick(50); // age = MAX_CHANGE_AGE + 50 — must clamp.
        let clamped = old.check_tick(current);
        assert_eq!(clamped, Tick(current.0.wrapping_sub(MAX_CHANGE_AGE)));
    }

    /// `check_tick` leaves recent ticks untouched.
    #[test]
    fn tick_check_tick_noop_when_in_range() {
        let current = Tick(1_000);
        let recent = Tick(500); // age = 500 ≪ MAX_CHANGE_AGE.
        assert_eq!(recent.check_tick(current), recent);
    }

    /// Wraparound correctness — `current` near `0` after wrapping, stored
    /// tick just below `u32::MAX`.
    #[test]
    fn tick_check_tick_wraparound_correctness() {
        let current = Tick(100);
        let stored = Tick(u32::MAX - 50); // wrapped 151 ticks ago.
        let age = current.0.wrapping_sub(stored.0);
        assert_eq!(age, 151);
        // age ≪ MAX_CHANGE_AGE → no clamp.
        assert_eq!(stored.check_tick(current), stored);
    }

    /// `MAX_CHANGE_AGE` matches Bevy's published formula. (The full
    /// wraparound-safety inequality is enforced at compile time by the
    /// `const _` block above; this test pins the exact decimal value.)
    #[test]
    fn max_change_age_matches_published_formula() {
        assert_eq!(MAX_CHANGE_AGE, u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1));
    }

    /// `Tick` is `repr(transparent)` over `u32` — same size, same align.
    #[test]
    fn tick_layout_matches_u32() {
        assert_eq!(core::mem::size_of::<Tick>(), core::mem::size_of::<u32>());
        assert_eq!(core::mem::align_of::<Tick>(), core::mem::align_of::<u32>());
    }
}
