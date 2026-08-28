//! The UI clock — UI-ADVANCED rung A0 (`docs/UI-PLAN-ANIMATION-A0.md`;
//! `docs/UI-PLAN-ANIMATION-DECISIONS.md` AD1, AD9, AM6, AM7).
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
//!
//! That is the CLOCK's ordering, and the sink's is not the same shape.
//! [`ui_visual_tick`] writes [`UiVisual`] through `Mut::set_if_neq`, and a
//! `Changed<UiVisual>` reader ordered BEFORE it does not read a stale value —
//! it reads NOTHING, permanently. Any host system filtering on the sink
//! (`ui_render_discovery` is the one that matters) MUST be registered
//! `.after_set(UiAnimationSet)`. See [`ui_visual_tick`] for the mechanism and
//! the measurement.

use std::mem;

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{AnyOf, Mut, Query};
use boyko_ecs::ecs::core::schedule::system_set::SystemSet;
use boyko_ecs::ecs::core::system::{Commands, Res, ResMut};
use boyko_ecs::ecs::core::time::time::Time;
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};
use boyko_macros::Resource;

use crate::components::{
    EasingId, TweenOffset, TweenOffsetBundle, TweenOpacity, TweenOpacityBundle, TweenScale,
    TweenScaleBundle, TweenTint, TweenTintBundle, UiVisual, TWEEN_FLAG_VIRTUAL_CLOCK,
};
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

/// The rare-path refusal for a tween duration that is not finite or not
/// POSITIVELY SIGNED.
///
/// RELEASE-ACTIVE, and it REFUSES rather than panics — the
/// [`depth_clamped`](crate::layout) precedent, not
/// [`UiClock::set_max_delta`]'s: this runs on the gameplay path, and a panic in a
/// UI system under the windowed runner hangs the window instead of reporting.
///
/// # The five refused shapes, MEASURED
///
/// 2026-08-27, release, five 100 ms frames of a `0.0 → 0.25` opacity tween with
/// the guard's `return` deleted so the row is actually built:
///
/// | `duration_ms` | `inv_duration` | what the row does |
/// |---|---|---|
/// | `+inf` | `0` | `t = 0` forever — frozen at `from`, immortal |
/// | `-inf` | `-0` | `t = -0` forever — frozen at `from`, immortal |
/// | negative finite (`-100.0`) | `-10` | **diverges** −0.25 … −1.25, immortal |
/// | `-0.0` | `-inf` | sink is **`-inf`** from frame 1 on, immortal |
/// | `NaN` | `NaN` | **completes on frame 1** — see below |
///
/// Four of those five are immortal: the row never completes, never enters `done`,
/// and the reap can never reach it. The negative finite shape is the worst for
/// the render gate — it bumps `set_if_neq` on every frame, which at rung A4 is
/// the whole UI's per-slot repaint skip disarmed permanently, not "a wrong
/// picture on one node".
///
/// **`NaN` is the exception and it is recorded, not glossed.** `t` is NaN, and
/// [`advance`]'s `t < 1.0` spelling puts a NaN `t` on the COMPLETING side, so a
/// NaN-duration row completes on frame 1, assigns its endpoint and is reaped.
/// This entry point refuses it anyway — it is an authoring mistake and the
/// `debug_assert!` below names it at the site — but the refusal and `advance`'s
/// spelling OVERLAP on this member, and no gate here may claim NaN as coverage
/// earned by the guard alone.
///
/// # What this predicate does NOT close, MEASURED
///
/// It closes DEGENERATE RECIPROCALS — `duration_ms` non-finite or not positively
/// signed. It does **not** close "the immortal row class", and saying it does is
/// an over-claim this rung retracts.
///
/// `elapsed` is an `f32` accumulating `+= dt`, so absorption gives it a hard
/// ceiling: MEASURED by exact `f32` simulation 2026-08-27, `524288 s` (`2^19`,
/// **6.068 days**, 24,986,955 frames) at `dt = 1/60 s` and at 16 ms, and
/// `2097152 s` (`2^21`, **24.273 days**) at the 100 ms [`UiClock`] clamp
/// ceiling. A row completes only when `elapsed` reaches `duration_ms / 1000`,
/// so **every accepted `duration_ms` STRICTLY above that ceiling — at 60 Hz,
/// `> 5.24288e8` — yields a row that never completes, is never reaped, and
/// bumps `set_if_neq` on EVERY frame.** `1e10` ("115 days"), `1e30` and
/// `f32::MAX` are all on that side.
///
/// **The operator is `>`, not `>=`, and the boundary value itself COMPLETES.**
/// Re-measured at this site 2026-08-27 (`rustc -O`, exact `f32`), because the
/// number above was first measured elsewhere and carried here: at
/// `duration_ms = 5.24288e8` (bits `0x4dfa0000`, exactly `524288000`)
/// `inv_duration` is bits `0x36000000` — exactly `2^-19` — so at the ceiling
/// `t` is `524288.0 * 2^-19` = exactly `1.0` (bits `0x3f800000`), and
/// [`advance`]'s `t < 1.0` spelling puts exactly `1.0` on the COMPLETING side.
/// The first genuinely never-completing duration is ONE ULP ABOVE it:
/// `5.24288032e8`, bits **`0x4dfa0001`**, whose `t` at the ceiling is
/// `0.99999994`. The 100 ms clamp behaves identically — `2.097152e9` (bits
/// `0x4efa0000`) reaches exactly `1.0` at `2^21` and completes; `0x4efa0001` is
/// the first that does not.
///
/// The REFUSED `+inf` bumps zero times after the first frame; an accepted
/// `f32::MAX` bumps every frame while rendering an opacity that never leaves
/// the neighbourhood of its START endpoint — visually the same picture, and
/// strictly worse for the A4 repaint skip.
///
/// **That opacity, MEASURED at THIS site 2026-08-28.** The figure this sentence
/// used to carry (`3.673e-36`) belonged to no frame of any fixture; it is
/// corrected here rather than dropped. Read back FROM THE ENGINE through
/// `an_over_ceiling_duration_is_accepted_and_never_completes`'s own `f32::MAX`
/// arm — `start_tween_opacity(.., 0.0, 1.0, f32::MAX, LINEAR, 0)` at
/// `FRAME = 100 ms` — and, independently, by exact `f32` simulation of
/// [`advance`], the two agreeing bit-for-bit on every frame: the sink holds
/// **`2.938736e-37`** (bits `0x02c80001`) after frame 1 and **`1.469368e-36`**
/// (bits `0x03fa0001`) after frame 5.
///
/// ⚠️ **Those seven mantissa digits are also `inv_duration`'s, one decade up.**
/// `1000.0 / f32::MAX` is bits `0x047a0001` and prints `2.938736e-36` — the
/// same bit pattern the next paragraph names as the smallest duration with a
/// finite reciprocal, and the reason the digits collide at all. A rendered
/// opacity and a stored reciprocal are different quantities; tell them apart by
/// BITS, never by the printed digits.
///
/// A second, benign boundary sits far below: `1000.0 / duration_ms` overflows to
/// `+inf` only for `duration_ms` below bits **`0x047a0001`**
/// (`2.9387360564219222e-36`) — that is the SMALLEST duration with a finite
/// reciprocal. **`250.0 x f32::MIN_POSITIVE` is NOT that value**: it is bits
/// `0x047a0000` (`2.9387358770557188e-36`), one ULP BELOW, and it is the
/// LARGEST duration that still overflows — the closed form named the wrong side
/// of the boundary it defines. State it by BITS: the two floats bracketing it
/// both print `2.938736e-36` at 7 significant figures, so no decimal at that
/// width can name it. That regime is the one `+0.0` and the denormals live in,
/// and it is benign: those SNAP.
///
/// Whether to refuse an over-ceiling duration is a VALUES call and is filed for
/// the owner in `docs/OPEN-QUESTIONS.md`; it is not decided here.
///
/// **Gated by TWO tests, and neither subsumes the other.**
/// `an_over_ceiling_duration_is_accepted_and_never_completes`
/// (`tests/ui_a1_tween.rs`) shows the BEHAVIOUR this section describes actually
/// happens — the row is live and the sink bumps on every frame — at three sampled
/// durations. Three points cannot close the word *every*: MEASURED 2026-08-28, a
/// clamp restricted to `(5.0e8, 1.0e9]` preserved all three samples bit-exactly
/// and left the crate green while the class was broken, and a cap inside
/// [`advance`] (`&& *elapsed < 3_600.0`) falsified this paragraph verbatim with
/// all 1427 tests at EXIT=0. So the CLASS is closed by
/// `the_termination_condition_is_pinned_to_the_disclosure`
/// (`tests/ui_a1_source_census.rs`), which pins the normalized source of
/// [`advance`] and of the `tween_helpers!` `$start` body — any added cap, clamp,
/// sub-range rewrite or extra conjunct reds it, and its failure message says this
/// paragraph must be rewritten in the same edit. That census also reds if these
/// sentences are DELETED while the predicate stands.
///
/// # `+0.0` and the denormals are ACCEPTED
///
/// They yield `inv_duration = +inf`, so `t = inf` completes on the first frame
/// with a non-zero delta and the endpoint is ASSIGNED — a zero-duration tween
/// SNAPS to its target. That is why the predicate is spelled
/// `is_finite() && is_sign_positive()` and not `is_finite() && > 0.0`: `0.0f32 >
/// 0.0` is false, so the strict-comparison spelling turned an authored zero
/// duration into a node that never acquires a sink and never reaches its target.
/// `-0.0` must stay refused (it is `-inf`, the fourth row above), and the sign
/// bit is the only thing separating the two. Gated by
/// `a_zero_or_denormal_duration_snaps_to_the_endpoint` (`tests/ui_a1_tween.rs`).
#[cold]
#[inline(never)]
fn invalid_tween_duration(duration_ms: f32) {
    debug_assert!(
        false,
        "invariant: a tween duration is finite and POSITIVELY SIGNED (got {duration_ms} ms) \
         — deliberately NOT 'strictly positive': `+0.0` and the denormals are ACCEPTED and \
         snap to the endpoint, and only the SIGN BIT separates them from `-0.0`, which is \
         refused. The row stores 1/duration; the release build REFUSES the tween rather than \
         creating a row that can never complete"
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

/// The [`SystemSet`] rung A0's clock tick and rung A1's animation pair run in.
///
/// Three systems, in a pinned order: [`ui_clock_tick`] → [`ui_visual_tick`] →
/// [`ui_tween_reap`], the last of which is **exclusive** (`&mut EcsMaster`).
/// Exposed so a host can order its own time-varying UI systems
/// `.after_set(UiAnimationSet)` — the [`UiWidgetSet`](crate::widgets::UiWidgetSet)
/// / [`UiBindSet`](crate::interaction::UiBindSet) idiom.
///
/// **What "without that edge" costs depends on WHICH member you consume, and the
/// two are not the same.** A consumer of [`UiClock`] without the edge reads the
/// previous frame's deltas — a frame late, never a wrong number. A consumer
/// FILTERING on `Changed<UiVisual>` without the edge reads nothing at all, on
/// every frame, permanently — see [`ui_visual_tick`]'s `# Ordering`.
#[derive(Clone, Copy, Debug)]
pub struct UiAnimationSet;
impl SystemSet for UiAnimationSet {}

/// Wires rungs A0 and A1 into an [`App`]: inserts [`UiClock`] **and**
/// [`UiTweenScratch`] (both insert-if-absent), and schedules [`ui_clock_tick`] →
/// [`ui_visual_tick`] → [`ui_tween_reap`] on [`CoreSchedule::Main`] in
/// [`UiAnimationSet`], with SET edges between them because add-order is not a
/// pin. The reap is EXCLUSIVE.
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
        // Rung A1: the retained completion list. Insert-if-absent for the same
        // reason the clock is — a second `add_plugin` must not swap a live
        // buffer out from under a frame in flight.
        if !app.world_mut().contains_resource::<UiTweenScratch>() {
            app.insert_resource(UiTweenScratch::default());
        }
        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            let clock = b.add_system(ui_clock_tick).in_set(UiAnimationSet).key();
            // A1: the tick reads the clock this frame, and the reap runs
            // IMMEDIATELY after the tick (AD5) — both edges are SET edges, not
            // registration order, because add-order is not a pin.
            let tick = b.add_system(ui_visual_tick).in_set(UiAnimationSet).after(clock).key();
            b.add_system(ui_tween_reap).in_set(UiAnimationSet).after(tick);
        });
    }
}


// ─────────────────────────────────────────────────────────────────────────────
// UI-ADVANCED rung A1 — the fused tick, the deferred reap, and the authoring
// surface (`docs/UI-PLAN-ANIMATION-A1.md` A1; `docs/UI-PLAN-ANIMATION-DECISIONS.md`
// AD5, AD10, AD11, AD12, AM1, AM2).
// ─────────────────────────────────────────────────────────────────────────────

/// The retained completion list (AD5): the `(entity, channel)` pairs
/// [`ui_visual_tick`] finished this frame and [`ui_tween_reap`] removes.
///
/// A `Resource`-owned buffer reused across frames — the [`UiBarScratch`] shape,
/// so the steady animating path allocates nothing (A1 gate 6). It is FILLED and
/// DRAINED inside one frame, and A1 gate 8 leg (i) is exactly the assertion that
/// the drain happens.
///
/// # Why the generation-free key is safe
///
/// The key is a generation-free [`EntityId`], and THREE separate facts are what
/// make that safe. All three were measured 2026-08-27; none of them is "the pair
/// does not survive its frame", which is true and does not bear on this.
///
/// The pair's real lifetime is the window BETWEEN [`ui_visual_tick`] and
/// [`ui_tween_reap`]. `after(tick)` orders the reap after the tick; it forbids
/// nothing in between, and `DenseStore::remove` (`dense/dense_store.rs:299`)
/// takes a bare id with NO liveness check — MEASURED, it removes a live,
/// unrelated row and returns `true`. What closes the window is:
///
/// 1. **A despawn removes the entity's dense rows itself** (MEASURED: a channel's
///    live count went 1 → 0 across a despawn, with no reap involved), so a pair
///    naming a despawned entity finds no slot and `remove` returns `false`.
/// 2. **This kernel does not recycle [`EntityId`]** (MEASURED: spawn `[0..5]`,
///    despawn four, the next six ids are `[6..11]`; recycled = none), so there is
///    no id under which fact 1 could be undone. This is the load-bearing fact and
///    it is NOT architectural — [`Entity`] carries a `generation` and
///    `Entity::increment_generation` is documented "used to detect stale handles",
///    so the kernel is BUILT for recycling and merely does not do it yet. It is
///    therefore guarded by `entity_ids_are_not_recycled_today`
///    (`tests/miri_a1_tween.rs`), which reds the day it changes.
/// 3. Nothing in the shipped schedule occupies the window: command applies drain
///    at the schedule's apply window, so only an EXCLUSIVE system a host
///    deliberately scheduled between the two could despawn and spawn inside it.
///
/// Carrying the generation instead was priced and declined: `(Entity, ComponentId)`
/// is 12 B against this pair's 8 B (+50 % on a buffer that peaks at one entry per
/// completion), and `Query::iter_entities_mut` yields an [`EntityId`], so the tick
/// would need a per-completion generation lookup it has no world access for.
#[derive(Resource, Default)]
pub struct UiTweenScratch {
    /// Channels whose `elapsed` reached their duration this frame.
    done: Vec<(EntityId, ComponentId)>,
}

impl UiTweenScratch {
    /// How many completions are queued for the reap.
    ///
    /// Zero at every point a system outside [`UiAnimationSet`] can observe it —
    /// the tick fills it and the reap immediately after empties it. A non-zero
    /// reading from outside the set means the reap did not run or did not clear.
    #[inline]
    pub fn pending(&self) -> usize {
        self.done.len()
    }
}

/// The `on_add` hook every `Tween*` channel carries: materializes the node's
/// [`UiVisual`] sink at [`UiVisual::IDENTITY`] when it has none.
///
/// # Why a HOOK, and not the start helpers (A1's Lands list said helpers)
///
/// A1's text assigned the sink's insert to the public start helpers. MEASURED at
/// the landing: **they cannot do it.** Adding a component to an EXISTING entity
/// has exactly one route out of `boyko_ui` — `Commands` — and `Commands`
/// declares no reads, so a helper holding one cannot ask whether the sink is
/// already there. Inserting it unconditionally is not a substitute: it would
/// stomp a sink carrying a finished channel's value, which is precisely AD12's
/// "the panel that slid in jumps home". Every `EcsMaster` insert/migration
/// helper that could both read and insert is `pub(crate)`.
///
/// The hook route is the one path that has both — `DeferredEcsMaster` reads the
/// world and enqueues into the world-resident deferred queue — and it is the
/// crate's established answer to this exact shape: `UiSpriteAnim`'s
/// [`ui_sprite_anim_on_add`](crate::sprite) materializes `UiSpriteCursor` the
/// same way, for the same reason, one rung earlier. It is also strictly better
/// than the helpers would have been: a channel inserted by hand, by a bundle, or
/// at spawn gets its sink too, so "a tween that ticks into nothing" is
/// unreachable rather than merely discouraged.
///
/// `#[require(UiVisual)]` would be the obvious alternative and is NOT available
/// in the direction that matters: it would have to sit on a `Tween*`, and
/// MEASURED on this kernel the require pass resolves the required id's
/// `ComponentPool` in the target archetype — which a DENSE id owns none of — so
/// it panics at insert.
///
/// # The insert is DEFERRED, and four channels may enqueue it
///
/// The sink is present after the outermost apply, not inside the window that
/// added the channel — the same structural-not-instantaneous pairing
/// `ui_sprite_anim_on_add` documents. Adding several channels in one window
/// makes each hook read a world where the sink is still absent, so each enqueues
/// one insert of [`UiVisual::IDENTITY`]; they are byte-identical and the last
/// one wins, so the outcome is the same single identity row.
///
/// # Safety
///
/// The [`HookFn`](boyko_ecs::ecs::core::component::hooks::HookFn) contract: the
/// kernel calls this during a hook dispatch with an exclusively-borrowed live
/// world and the added entity's context. This body performs no direct storage
/// access — it reads one component through the view's own accessor and enqueues
/// one structural command into the world-resident deferred queue, which the
/// outermost drain applies strictly later.
pub(crate) unsafe fn ui_visual_sink_on_add(mut world: DeferredEcsMaster<'_>, ctx: HookContext) {
    if world.get_component::<UiVisual>(ctx.entity).is_some() {
        // A sink already carries this node's resting appearance (AM2). Replacing
        // it with the identity would undo every finished channel — AD12's
        // haunting, arriving through the back door.
        return;
    }
    world.commands().entity(ctx.entity).insert(UiVisual::IDENTITY);
}

/// One channel's per-frame advance: adds the row's own delta and returns the
/// normalized `t` while the tween is still running, or `None` at completion.
///
/// The delta is SELECTED per row (AD9 (1) / D15): bit 0 of `flags`
/// ([`TWEEN_FLAG_VIRTUAL_CLOCK`]) picks the clock's virtual delta, everything
/// else takes the real one. A select between two `f32` already in registers, not
/// a branch on a `Duration`.
///
/// The completion test is spelled `t < 1.0`, not `!(t >= 1.0)`: a NaN `t` is
/// unordered against both, and this spelling puts NaN on the COMPLETING side, so
/// a row whose `inv_duration` went NaN assigns its endpoint and is reaped instead
/// of running forever. MEASURED free — 14.002 vs 14.008 ns per 4-channel row
/// (4096 nodes, release, floor of five process runs).
///
/// It is defence in depth, not the guard: it covers NaN and NOTHING ELSE. A
/// non-positive or infinite `inv_duration` still yields a finite `t < 1.0`
/// forever. That whole class is refused at the door instead — see
/// [`invalid_tween_duration`] — because the per-row alternative MEASURED
/// **+0.536 ns/row (+3.8 %)** and would defend one `pub` field on a route where
/// `from`, `to` and `elapsed` are equally undefended.
///
/// **`t < 1.0` is the ONLY way a row stops, and that is pinned rather than
/// stated.** This function's normalized source is
/// [`ADVANCE_PIN`](../../tests/ui_a1_source_census.rs) — a second termination
/// condition here (a wall-clock cap, an `elapsed` ceiling, an early return)
/// falsifies [`invalid_tween_duration`]'s over-ceiling disclosure without moving
/// any number the door stores, so nothing that samples the door can see it.
/// MEASURED 2026-08-28: `&& *elapsed < 3_600.0` on the line below left 1427 tests
/// at EXIT=0.
///
/// Every branch of this function is enumerated with its coverage in that same
/// file's `SITES` table; the per-row clock select below is there because it had
/// ZERO executions in the allocation gate's window until the `virtual_clock`
/// cohort was added on 2026-08-28.
#[inline]
fn advance(elapsed: &mut f32, inv_duration: f32, flags: u8, dt_real: f32, dt_virtual: f32) -> Option<f32> {
    debug_assert!(*elapsed >= 0.0, "invariant: a tween's elapsed is non-negative");
    let dt = if flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0 { dt_virtual } else { dt_real };
    *elapsed += dt;
    let t = *elapsed * inv_duration;
    if t < 1.0 { Some(t) } else { None }
}

/// Applies rung A1's curve set to a normalized `t`.
///
/// A1 is LINEAR ONLY (`EasingId` exists as a field and this is the identity), so
/// A1's gates test the machinery and A2's gates test the curves — a red at A2
/// cannot be blamed on A1.
#[inline]
fn ease(t: f32, _easing: EasingId) -> f32 {
    t
}

/// Scalar interpolation. `t` is in `[0, 1)` here — the endpoint is ASSIGNED by
/// the caller at completion, never reached by interpolation, so `from + (to −
/// from) * t` can never be asked to reproduce `to` exactly.
#[inline]
fn lerp1(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Component-wise interpolation of two straight (non-premultiplied) RGBA8 words
/// (AD3: tint multiplies component-wise in straight RGBA8, before the pack's
/// premultiply).
#[inline]
fn lerp_rgba8(from: u32, to: u32, t: f32) -> u32 {
    let mut out = 0u32;
    let mut shift = 0;
    while shift < 32 {
        let a = ((from >> shift) & 0xFF) as f32;
        let b = ((to >> shift) & 0xFF) as f32;
        // `+ 0.5` then truncate: round-to-nearest without pulling in `f32::round`,
        // and the operand is non-negative by construction so the sign case cannot
        // arise.
        let v = (lerp1(a, b, t) + 0.5) as u32;
        out |= (v & 0xFF) << shift;
        shift += 8;
    }
    out
}

/// Advances every live tween channel and composes the result into each node's
/// [`UiVisual`] sink — ONE fused system over four channel columns (AD5).
///
/// # The three properties this body cannot be written without
///
/// 1. **The all-`None` `continue` (AM2).** An `AnyOf` whose arms are ALL DENSE
///    does not skip a row that has none of them: the arms' archetype predicate
///    is vacuously true for a dense id (it is in no signature), so a `UiVisual`
///    row whose channels have all been reaped IS visited and yields
///    `(None, None, None, None)`. MEASURED: 1 animating + 2 rested rows ⇒ 3
///    visits, 2 all-`None`. The `continue` fires BEFORE the sink is touched.
/// 2. **The base is `*sink`, not the identity (AD12).** The four channels own
///    DISJOINT fields, so "compose" means *overwrite the fields whose channel is
///    live and carry the rest*. From the identity, a node whose `TweenOffset`
///    finished at −400 px reads 0 on the first frame of a later `TweenTint` —
///    the finished animation silently undoes itself. MEASURED: `off[0]` is 0
///    under an identity base and −400 under `*sink`.
/// 3. **The write is `Mut::set_if_neq`, not `*sink = …` (AM1).** `&mut T` does
///    not consult ticks at all, so the sink's changed tick would never bump and
///    `ui_render_discovery`'s `Or` would never see the row: every animation
///    would render nothing while every arithmetic unit test stayed green.
///    MEASURED: `&mut` write ⇒ 0 `Changed` rows; `Mut::set_if_neq` ⇒ 1. And
///    `set_if_neq` rather than a plain deref, because a value-preserving frame
///    must not bump — a tick that bumps every frame defeats the render gate as
///    surely as one that never bumps.
///
/// # Completion is recorded here and REMOVED elsewhere
///
/// A channel whose `elapsed` reached its duration writes its endpoint EXACTLY
/// (`to`, never `to ± ULP`) and pushes its `(entity, channel)` pair onto
/// [`UiTweenScratch`]. The removal is [`ui_tween_reap`]'s, immediately after: a
/// dense remove is a structural op and this query is iterating.
///
/// # Ordering — a wrong order LOSES the repaint, it does not delay it
///
/// Any system filtering on `Changed<UiVisual>` MUST be registered
/// `.after_set(UiAnimationSet)`, as a real ordering edge; add-order is not a pin.
///
/// `Schedule::run` bumps ONE `this_run` per run and hands it to every system in
/// that run (`schedule.rs:288`, `let this_run = world.bump_change_tick();`), and
/// each system's previous `this_run` becomes its new `last_run` (`schedule.rs:342`,
/// `sys_box.system.set_change_ticks(prev_this_run, this_run)`). A `Changed` term
/// is true for a row whose changed tick lies in the HALF-OPEN window
/// `(last_run, this_run]`; the comparison itself is `Tick::is_newer_than`
/// (`change_detection/tick.rs:169-171` — `ticks_since_system > ticks_since_insert`),
/// consumed at `filter.rs:1205`, `:1225`, `:1493` and `:1503`. *(`schedule.rs:152`
/// is NOT the mechanism — it is a doc comment about the gated-system dispatch
/// stamp that merely quotes the same `(last_run, this_run]` notation.)*
/// So a write stamped in frame N carries frame N's tick, and a reader ordered
/// BEFORE the writer misses it TWICE: in frame N the write has not happened yet,
/// and in frame N+1 the window's lower bound is EXCLUSIVE and is exactly the tick
/// the write carries. The write is in neither window. It is lost permanently.
///
/// MEASURED 2026-08-27 (20 frames, one animating node, a real
/// `Or<(Changed<ComputedRect>, Changed<UiVisual>)>` over this system's
/// `Mut::set_if_neq` write): reader after the writer ⇒ a hit on all 20 frames;
/// reader before ⇒ **1** hit in 20, and that one is the out-of-schedule insert
/// stamp, not an animating frame.
///
/// This is why [`UiAnimationSet`]'s "a consumer without the edge reads the
/// previous frame's deltas" is true of [`UiClock`] and FALSE of this system's
/// sink: `Res<UiClock>` is a plain resource read with no `Changed` window, so it
/// lags; a `Changed` filter has a window, and the write falls outside both of
/// them.
//
// `clippy::type_complexity`: the `Query<(Mut<…>, AnyOf<(…)>)>` tuple IS this
// system's `SystemParam` signature, which the scheduler reads to derive access.
// A `type` alias here WOULD compile — MEASURED 2026-08-27, `type ZzTweenQuery<'w,
// 's, 'a> = Query<'w, 's, (Mut<'a, UiVisual>, AnyOf<(…)>)>` with this `#[allow]`
// removed gives `cargo clippy -p boyko-ui --lib -- -D warnings` EXIT=0, and the
// lib registers this very system below, so the `SystemParam` impl survives the
// alias (type aliases are transparent). It is declined because it would only
// hide the access set from a reader, and because the alias must spell three
// lifetimes explicitly to say what the inline signature elides. The four `AnyOf`
// arms are the four channel columns AD5 fuses into one pass; splitting them
// would be the four-system shape AD5 exists to avoid.
#[allow(clippy::type_complexity)]
pub fn ui_visual_tick(
    clock: Res<UiClock>,
    mut q: Query<(
        Mut<UiVisual>,
        AnyOf<(&mut TweenTint, &mut TweenOpacity, &mut TweenOffset, &mut TweenScale)>,
    )>,
    mut done: ResMut<UiTweenScratch>,
) {
    let dt_real = clock.dt_real();
    let dt_virtual = clock.dt_virtual();

    for (entity, (mut sink, (tint, opacity, offset, scale))) in q.iter_entities_mut() {
        if tint.is_none() && opacity.is_none() && offset.is_none() && scale.is_none() {
            // Property 1 — rested, and NOT skipped by the query. Nothing is
            // touched, so the row is silent and D6a's per-slot skip stays armed.
            continue;
        }

        // Property 2 — the composition base.
        let mut composed = *sink;

        if let Some(row) = tint {
            match advance(&mut row.elapsed, row.inv_duration, row.flags, dt_real, dt_virtual) {
                Some(t) => composed.tint_mul = lerp_rgba8(row.from, row.to, ease(t, row.easing)),
                None => {
                    composed.tint_mul = row.to;
                    done.done.push((entity, TweenTint::component_id()));
                }
            }
        }
        if let Some(row) = opacity {
            match advance(&mut row.elapsed, row.inv_duration, row.flags, dt_real, dt_virtual) {
                Some(t) => composed.opacity = lerp1(row.from, row.to, ease(t, row.easing)),
                None => {
                    composed.opacity = row.to;
                    done.done.push((entity, TweenOpacity::component_id()));
                }
            }
        }
        if let Some(row) = offset {
            match advance(&mut row.elapsed, row.inv_duration, row.flags, dt_real, dt_virtual) {
                Some(t) => {
                    let e = ease(t, row.easing);
                    composed.offset_px =
                        [lerp1(row.from[0], row.to[0], e), lerp1(row.from[1], row.to[1], e)];
                }
                None => {
                    composed.offset_px = row.to;
                    done.done.push((entity, TweenOffset::component_id()));
                }
            }
        }
        if let Some(row) = scale {
            match advance(&mut row.elapsed, row.inv_duration, row.flags, dt_real, dt_virtual) {
                Some(t) => {
                    let e = ease(t, row.easing);
                    composed.scale =
                        [lerp1(row.from[0], row.to[0], e), lerp1(row.from[1], row.to[1], e)];
                }
                None => {
                    composed.scale = row.to;
                    done.done.push((entity, TweenScale::component_id()));
                }
            }
        }

        // Property 3 — the ONE write, through the ONE verb.
        sink.set_if_neq(composed);
    }
}

/// Removes the channels [`ui_visual_tick`] finished this frame (AD5).
///
/// # Why EXCLUSIVE and not `Commands`
///
/// `Commands` would land the removal at the next apply window, which is fine for
/// a *removal* on its own — the row's last write already happened. It is not
/// fine for the contract the removal carries: `ui_transition_apply` (rung A3)
/// reads "no row present ⇒ at rest", and an exclusive system in the same set
/// gives that ordering explicitly instead of depending on where the command
/// buffer drains. It is the [`ui_bar_apply`](crate::widgets::ui_bar_apply) shape
/// with a smaller body.
///
/// # Why the removal is spelled on the dense registry
///
/// `EcsMaster::dense_remove_and_fire` — the verb `Commands::remove` reaches — is
/// `pub(crate)`, and `Commands` cannot be constructed outside the `SystemParam`
/// machinery, so an exclusive system in another crate has exactly one immediate
/// removal route: `dense_registry_mut()`, which the kernel documents as the
/// surface it exposes "to external structural callers". The difference is
/// narrow and it is stated rather than discovered: this path does NOT fire
/// `on_replace` / `on_remove` for the channel. **No `Tween*` carries either
/// hook** (their only hook is `on_add`, which this path cannot reach), and a
/// future observer on a channel's removal would have to move the reap onto
/// `Commands` and re-argue the A3 ordering above.
///
/// # The drain is the point (A1 gate 8 leg (i))
///
/// The buffer is taken, walked and PUT BACK EMPTY. An entry that survives its
/// frame is a `(EntityId, ComponentId)` for a row that is gone, and the next
/// frame replays it — removing whatever channel that id has by then, which after
/// a despawn and an id reuse is an unrelated entity's.
//
// `clippy::needless_pass_by_ref_mut`: the removal goes through `&mut self`
// engine methods clippy cannot see through. Mirrors `ui_bar_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_tween_reap(world: &mut EcsMaster) {
    let mut done = mem::take(&mut world.resource_mut::<UiTweenScratch>().done);

    for &(entity, component_id) in &done {
        if let Some(store) = world.dense_registry_mut().store_existing_mut(component_id) {
            store.remove(entity);
        }
    }

    done.clear();
    world.resource_mut::<UiTweenScratch>().done = done;
}

/// Emits one channel's public `start_` / `stop_` pair.
///
/// The four helpers differ only in their payload type and the field of
/// [`UiVisual`] they drive, so the `debug_assert!` set and the wrapper-bundle
/// spelling are written once. Their signatures take `&mut Commands` because that
/// is the ONLY route from this crate to a component on an existing entity — see
/// [`ui_visual_sink_on_add`], which is where the sink's own insert had to go as
/// a consequence.
macro_rules! tween_helpers {
    (
        $start:ident, $stop:ident, $channel:ident, $bundle:ident, $payload:ty,
        $finite:expr, $doc_what:literal
    ) => {
        #[doc = concat!("Starts (or RESTARTS) this node's ", $doc_what, " tween.")]
        ///
        /// Restarting is a RESTART, not a retarget: an entity that already
        /// carries this channel gets `from`/`to`/`duration` replaced and
        /// `elapsed` rewound to 0, at the same dense slot. That is the property
        /// rung A3's reversing transition depends on, and it is the reason this
        /// helper constructs a whole row rather than patching one.
        ///
        /// The node's [`UiVisual`] sink is materialized by the channel's
        /// `on_add` hook if it has none — see [`ui_visual_sink_on_add`] for why
        /// that is a hook and not two lines here.
        ///
        /// `duration_ms` is converted to the row's stored reciprocal once, here,
        /// so the per-frame tick is a multiply.
        ///
        /// # A degenerate `duration_ms` is REFUSED, in release too
        ///
        /// A `duration_ms` that is not finite, or whose SIGN BIT IS SET, creates
        /// NO ROW: the helper returns without inserting anything. This is the ONE
        /// check here that survives a release build, and it has to — the row
        /// stores `1000.0 / duration_ms`, and MEASURED, four of the five refused
        /// shapes produce a tween that never completes and can never be reaped
        /// (the fifth, `NaN`, is refused here but would also be survived by
        /// `advance`; the two defences overlap on it).
        ///
        /// **`+0.0` is ACCEPTED and snaps to the endpoint.** `1000.0 / +0.0` is
        /// `+inf`, so the row completes on the first frame with a non-zero delta
        /// and `to` is assigned — a zero duration means "be there now", not
        /// "do nothing". `-0.0` is REFUSED (it is `-inf`, and the sink diverges),
        /// which is why the predicate tests the sign bit rather than `> 0.0`.
        ///
        /// See `invalid_tween_duration` (private, in this module), which carries
        /// the per-shape measurement and the reason this refuses rather than
        /// panicking.
        ///
        /// # Panics
        ///
        /// In debug builds only, on a non-finite endpoint (and, through
        /// `invalid_tween_duration`'s `debug_assert!`, on the refused
        /// duration above, so an authoring mistake still names itself at the
        /// site that made it). A NaN ENDPOINT is not the release-side defence's
        /// subject — that is [`UiVisual`]'s bytewise `PartialEq` (AD11) — and
        /// every `debug_assert!` here compiles out.
        pub fn $start(
            cmds: &mut Commands,
            entity: Entity,
            from: $payload,
            to: $payload,
            duration_ms: f32,
            easing: EasingId,
            flags: u8,
        ) {
            if !(duration_ms.is_finite() && duration_ms.is_sign_positive()) {
                invalid_tween_duration(duration_ms);
                return;
            }
            let finite: fn($payload) -> bool = $finite;
            debug_assert!(
                finite(from) && finite(to),
                "invariant: a tween's endpoints are finite — a NaN endpoint reaches UiVisual and \
                 is a wrong picture on this node for as long as it stands"
            );
            let inv_duration = 1000.0 / duration_ms;
            cmds.entity(entity).insert($bundle {
                tween: $channel {
                    from,
                    to,
                    elapsed: 0.0,
                    inv_duration,
                    easing,
                    flags,
                    _pad: [0; 2],
                },
            });
        }

        #[doc = concat!("Stops this node's ", $doc_what, " tween, leaving the sink at its current value.")]
        ///
        /// Deferred (it is a `Commands` removal): the channel is gone after the
        /// next apply window, not inside the caller's system. The sink is NOT
        /// touched — its last value IS the node's resting appearance (AM2), so
        /// an author who wants the element back at rest tweens it there.
        pub fn $stop(cmds: &mut Commands, entity: Entity) {
            cmds.entity(entity).remove::<$channel>();
        }
    };
}

tween_helpers!(
    start_tween_tint,
    stop_tween_tint,
    TweenTint,
    TweenTintBundle,
    u32,
    |_v| true,
    "tint"
);
tween_helpers!(
    start_tween_opacity,
    stop_tween_opacity,
    TweenOpacity,
    TweenOpacityBundle,
    f32,
    |v: f32| v.is_finite() && (0.0..=1.0).contains(&v),
    "opacity"
);
tween_helpers!(
    start_tween_offset,
    stop_tween_offset,
    TweenOffset,
    TweenOffsetBundle,
    [f32; 2],
    |v: [f32; 2]| v[0].is_finite() && v[1].is_finite(),
    "offset"
);
tween_helpers!(
    start_tween_scale,
    stop_tween_scale,
    TweenScale,
    TweenScaleBundle,
    [f32; 2],
    |v: [f32; 2]| v[0].is_finite() && v[1].is_finite() && v[0] >= 0.0 && v[1] >= 0.0,
    "scale"
);

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
