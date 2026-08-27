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
// surface (`docs/UI-PLAN-ANIMATION.md` A1, AD5, AD10, AD11, AD12, AM1, AM2).
// ─────────────────────────────────────────────────────────────────────────────

/// The retained completion list (AD5): the `(entity, channel)` pairs
/// [`ui_visual_tick`] finished this frame and [`ui_tween_reap`] removes.
///
/// A `Resource`-owned buffer reused across frames — the [`UiBarScratch`] shape,
/// so the steady animating path allocates nothing (A1 gate 6). It is FILLED and
/// DRAINED inside one frame, which is also what makes the generation-free
/// [`EntityId`] key safe: a pair never survives the frame that produced it, so
/// there is no window in which a despawn could recycle the id underneath it.
/// A1 gate 8 leg (i) is exactly the assertion that the drain happens.
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
/// `t >= 1.0` is FALSE for a NaN `t`, so a row whose `inv_duration` went
/// non-finite in release keeps running and writes NaN into the sink rather than
/// completing. That is deliberate and it is bounded, not ignored: AD11's bytewise
/// equality makes the resulting `set_if_neq` idempotent, so the damage is a wrong
/// picture on one node instead of a render gate disarmed for the whole UI.
#[inline]
fn advance(elapsed: &mut f32, inv_duration: f32, flags: u8, dt_real: f32, dt_virtual: f32) -> Option<f32> {
    debug_assert!(*elapsed >= 0.0, "invariant: a tween's elapsed is non-negative");
    let dt = if flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0 { dt_virtual } else { dt_real };
    *elapsed += dt;
    let t = *elapsed * inv_duration;
    if t >= 1.0 { None } else { Some(t) }
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
        /// # Panics
        ///
        /// In debug builds only, on a non-finite or non-positive `duration_ms`
        /// or a non-finite endpoint. These are AUTHORING mistakes and the assert
        /// names them at the site that made them; they are NOT the release-side
        /// NaN defence, which is [`UiVisual`]'s bytewise `PartialEq` (AD11) —
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
            debug_assert!(
                duration_ms.is_finite() && duration_ms > 0.0,
                "invariant: a tween duration is finite and strictly positive (got {duration_ms} ms) \
                 — the row stores 1/duration, so a zero duration is a reciprocal-of-zero trap"
            );
            let finite: fn($payload) -> bool = $finite;
            debug_assert!(
                finite(from) && finite(to),
                "invariant: a tween's endpoints are finite — a NaN endpoint reaches UiVisual and \
                 is a wrong picture on this node for as long as it stands"
            );
            let inv_duration = 1000.0 / duration_ms;
            debug_assert!(
                inv_duration.is_finite() && inv_duration > 0.0,
                "invariant: inv_duration is finite and > 0"
            );
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
