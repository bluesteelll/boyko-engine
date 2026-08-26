//! The UI clock — UI-ADVANCED rung A0 (`docs/UI-PLAN-ANIMATION.md` AD1, AD9,
//! AM6, AM7).
//!
//! One resource, [`UiClock`], written exactly once per frame by
//! [`ui_clock_tick`] and read by every time-varying UI system. Three things live
//! here and they are one decision:
//!
//! 1. [`UiClock`] — two clamped `f32` deltas plus the clamp itself, so the
//!    `Duration → f32` conversion and AD1's hitch clamp happen ONCE per frame
//!    rather than once per consumer.
//! 2. [`ui_clock_tick`] — a normal `Main`-schedule system, `Res<Time>` in,
//!    `ResMut<UiClock>` out.
//! 3. [`UiAnimationPlugin`] / [`UiAnimationSet`] — the registration idiom, so a
//!    host writes one line and every UI consumer is ordered after the tick.
//!
//! # Which delta a consumer reads (AD9 (1))
//!
//! *A consumer that carries D15's per-row `flags` bit reads
//! [`dt_real`](UiClock::dt_real) unless that bit says otherwise. A consumer with
//! no `flags` bit reads [`dt_virtual`](UiClock::dt_virtual).* In v1 the only lane
//! with the bit is the tween row (rung A1), so in v1: **tweens are real by
//! default and virtual per row; everything else is virtual, full stop.**
//!
//! D15's argument for the real delta — *a pause menu that fades in on a paused
//! virtual clock never fades* — is an argument about a TWEEN WITH AN ENDPOINT, on
//! a UI shown *because* the game is paused. It is not an argument about a
//! flipbook, a fling or a dwell timer: none of those has an endpoint to be robbed
//! of, and all three are worse on the real delta — they keep running under a
//! pause menu and they ignore slow-motion. The one consumer that exists,
//! [`ui_sprite_flipbook`](crate::sprite::ui_sprite_flipbook), therefore reads
//! `dt_virtual`, which reproduces its pre-A0 arithmetic exactly.
//!
//! # Why the clamp is here and not at each consumer (AD1)
//!
//! `Time`'s own 250 ms clamp does NOT reach the real delta — `Time::real_delta`
//! is documented *"unclamped, unscaled, pause-blind"* and `Time::advance_with`
//! assigns it BEFORE taking `min(raw, max_delta)` (AM6, re-verified at the A0
//! landing). Putting the UI clamp at each consumer means the third consumer
//! forgets; putting it here means an alt-tab stall is truncated once, for
//! everybody, on both deltas.
//!
//! # The clamp value has exactly ONE definition (AD9 (3))
//!
//! [`UiClock::default`]'s `max_delta` **references**
//! [`UI_FALLBACK_MAX_DELTA`]; it does not
//! restate `0.1`. With one definition there is no second datum to diverge, so no
//! pin test is owed. Whichever rung deletes the last reader of that const moves
//! the definition onto `UiClock` and drops the const; after A0b that reader is
//! `UiClock::default()`.
//!
//! # Ordering
//!
//! [`ui_clock_tick`] runs in [`UiAnimationSet`] on [`CoreSchedule::Main`]. A
//! consumer registered without an ordering edge to it reads the PREVIOUS frame's
//! deltas — never a wrong number, but a frame late. Hosts that do not use
//! [`UiAnimationPlugin`] register the tick themselves, ahead of their consumers;
//! that is the same host responsibility the layout pair and the text measure
//! system carry.

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::schedule::system_set::SystemSet;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::time::Time;
use boyko_macros::Resource;

use crate::sprite::UI_FALLBACK_MAX_DELTA;

/// The UI's frame-delta source (AD1): both deltas, clamped, in seconds.
///
/// Written once per frame by [`ui_clock_tick`]; read by every time-varying UI
/// system through `Res<UiClock>`. This is the ONE UI frame-delta source — a
/// consumer reading `Res<Time>` and applying its own clamp is the second source
/// of truth AD1 exists to prevent. (`reload/system.rs`'s hot-reload poll
/// throttle is a wall clock consuming no frame delta and is deliberately outside
/// this rule.)
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct UiClock {
    /// Clamped real delta, seconds. The default clock of the TWEEN lane
    /// (D15, AM7) — unscaled and pause-blind by construction, which is the
    /// point.
    dt_real: f32,
    /// Clamped virtual delta, seconds — zero while `Time` is paused, AND scaled
    /// by `Time::relative_speed`. The default everywhere D15's per-row `flags`
    /// bit does not exist (AM7 / AD9).
    dt_virtual: f32,
    /// UI-local hitch clamp applied to BOTH deltas. Default 100 ms (AM6), and
    /// the default is a REFERENCE to
    /// [`UI_FALLBACK_MAX_DELTA`](crate::sprite::UI_FALLBACK_MAX_DELTA), not a
    /// second `0.1` (AD9 (3)).
    max_delta: f32,
}

impl UiClock {
    /// This frame's CLAMPED REAL delta in seconds — unscaled and pause-blind.
    ///
    /// Read this only from a lane carrying D15's per-row `flags` bit (AD9 (1)).
    /// Everything else reads [`dt_virtual`](UiClock::dt_virtual): a consumer
    /// with no endpoint keeps running under a pause menu and ignores
    /// slow-motion when it reads this field.
    #[inline]
    pub fn dt_real(&self) -> f32 {
        self.dt_real
    }

    /// This frame's CLAMPED VIRTUAL delta in seconds — zero while `Time` is
    /// paused, and scaled by `Time::relative_speed`.
    ///
    /// The default field for every consumer without D15's `flags` bit
    /// (AD9 (1)).
    #[inline]
    pub fn dt_virtual(&self) -> f32 {
        self.dt_virtual
    }

    /// The UI-local hitch clamp, in seconds, applied to BOTH deltas.
    ///
    /// A gate asserting the clamp compares against THIS, never against a `0.1`
    /// literal — the `min` is taken against this very value, so the comparison
    /// is exact by construction.
    #[inline]
    pub fn max_delta(&self) -> f32 {
        self.max_delta
    }

    /// Sets the UI-local hitch clamp, in seconds.
    ///
    /// Mirrors `Time::set_max_delta`'s validated-setter idiom: the invariant the
    /// tick relies on (`max_delta` finite and `> 0`, so `f32::min` against it
    /// can neither introduce a NaN nor freeze every consumer at zero) is checked
    /// where the value enters, not asserted where it is used.
    ///
    /// # Panics
    ///
    /// Panics if `secs` is not finite or is not strictly positive.
    #[inline]
    pub fn set_max_delta(&mut self, secs: f32) {
        if !(secs.is_finite() && secs > 0.0) {
            invalid_ui_max_delta_panic(secs);
        }
        self.max_delta = secs;
    }
}

/// The rare-path panic for an invalid [`UiClock::set_max_delta`] argument, out
/// of line so the setter stays a compare and a store.
#[cold]
#[inline(never)]
fn invalid_ui_max_delta_panic(secs: f32) -> ! {
    panic!(
        "UiClock::set_max_delta requires a finite, strictly positive number of seconds \
         (got {secs}) — a zero or NaN clamp either freezes every UI consumer at a zero \
         delta or poisons both deltas with NaN"
    );
}

impl Default for UiClock {
    /// Both deltas zero; `max_delta` is
    /// [`UI_FALLBACK_MAX_DELTA`] — the
    /// reference AD9 (3) requires, not a second `0.1`.
    #[inline]
    fn default() -> Self {
        Self {
            dt_real: 0.0,
            dt_virtual: 0.0,
            max_delta: UI_FALLBACK_MAX_DELTA,
        }
    }
}

/// Writes [`UiClock`] from [`Time`], once per frame (AD1).
///
/// Both deltas are clamped to [`UiClock::max_delta`]. `Time::real_delta` is
/// unclamped by construction (AM6), and `Time::delta_secs` carries only `Time`'s
/// own 250 ms clamp — four times the UI's — so BOTH need the `min` and AD1 says
/// so. Neither `min` can introduce a NaN: both inputs come from a `Duration` and
/// are therefore finite and non-negative, and `max_delta` is finite and positive
/// by construction ([`UiClock::default`]) and by validation
/// ([`UiClock::set_max_delta`]).
pub fn ui_clock_tick(time: Res<Time>, mut clock: ResMut<UiClock>) {
    let max = clock.max_delta;
    debug_assert!(
        max.is_finite() && max > 0.0,
        "invariant: UiClock::max_delta is finite and > 0 (setter-validated)"
    );
    clock.dt_real = time.real_delta().as_secs_f32().min(max);
    clock.dt_virtual = time.delta_secs().min(max);
}

/// The [`SystemSet`] [`ui_clock_tick`] runs in.
///
/// Exposed so a host can order its own time-varying UI systems
/// `.after_set(UiAnimationSet)` — the [`UiWidgetSet`](crate::widgets::UiWidgetSet)
/// / [`UiBindSet`](crate::interaction::UiBindSet) idiom. A consumer without that
/// edge reads the previous frame's deltas.
#[derive(Clone, Copy, Debug)]
pub struct UiAnimationSet;
impl SystemSet for UiAnimationSet {}

/// Wires the UI clock into an [`App`]: inserts [`UiClock`] and schedules
/// [`ui_clock_tick`] on [`CoreSchedule::Main`], in [`UiAnimationSet`].
///
/// # Containment
///
/// `Main` and nothing else. Registering on [`CoreSchedule::Fixed`] would create
/// the App's lazy fixed builder, which makes `App::finish` resolve the
/// process-wide `EventUpdatePolicy` to `WaitForFixed`, which holds the event swap
/// on every 0-substep frame — for INPUT, UI and COLLISION events, in an app whose
/// only change was installing the UI clock. Gated by
/// `plugin_adds_no_shared_schedule_surface` (A0 leg 5), which ACTS rather than
/// reads: `App` exposes no accessor for either the resolved policy or the
/// registered schedule set.
///
/// The insert is idempotent-by-intent: a host that configured its own
/// [`UiClock`] (a different [`max_delta`](UiClock::set_max_delta), say) before
/// adding this plugin keeps it — the [`UiSafeArea`](crate::resources::UiSafeArea)
/// precedent. Gated by `a_host_configured_clock_survives_the_plugin` (A0 leg 8),
/// which checks the survival BEHAVIOURALLY — the host's clamp is the value that
/// truncates a hitch — not merely as a field that retained a number.
///
/// # No `new()`
///
/// A unit struct is constructed by naming it, and `Default` covers the generic
/// `P::default()` call. A `pub fn new() -> Self { Self }` here would be a third
/// spelling with **zero callers**, which is what the A0 verification found on it
/// and why it was deleted rather than shipped. *(`UiBindingPlugin` and
/// `UiWidgetsPlugin` each still carry exactly that zero-caller `new()`; it is
/// pre-existing, A0 did not create it, and A0 declines to add a third copy.)*
#[derive(Default)]
pub struct UiAnimationPlugin;

impl Plugin for UiAnimationPlugin {
    fn build(&self, app: &mut App) {
        if !app.world_mut().contains_resource::<UiClock>() {
            app.insert_resource(UiClock::default());
        }
        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            b.add_system(ui_clock_tick).in_set(UiAnimationSet);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clamp is not restated here (AD9 (3)) — this asserts the REFERENCE,
    /// so a future edit that types `0.1` into `Default` instead of naming the
    /// const still has to move this line to compile a divergence in.
    #[test]
    fn default_max_delta_is_the_sprite_const_itself() {
        assert_eq!(UiClock::default().max_delta(), UI_FALLBACK_MAX_DELTA);
        assert_eq!(UiClock::default().dt_real(), 0.0);
        assert_eq!(UiClock::default().dt_virtual(), 0.0);
    }

    #[test]
    fn set_max_delta_accepts_a_positive_finite_value() {
        let mut c = UiClock::default();
        c.set_max_delta(0.25);
        assert_eq!(c.max_delta(), 0.25);
    }

    #[test]
    #[should_panic(expected = "finite, strictly positive")]
    fn set_max_delta_rejects_zero() {
        UiClock::default().set_max_delta(0.0);
    }

    #[test]
    #[should_panic(expected = "finite, strictly positive")]
    fn set_max_delta_rejects_nan() {
        UiClock::default().set_max_delta(f32::NAN);
    }

    #[test]
    #[should_panic(expected = "finite, strictly positive")]
    fn set_max_delta_rejects_negative() {
        UiClock::default().set_max_delta(-1.0);
    }
}
