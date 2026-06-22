//! GUI P6a widget drivers — the [`Bar`] fill system + its scratch.
//!
//! A [`Bar`] track hosts a `0..1` fraction in [`UiValue`]
//! (the P4 `BindValue` sink, reused verbatim — this module invents no binding) and
//! has one [`BarFill`]-marked child whose main-axis size tracks that fraction.
//! [`ui_bar_apply`] translates `UiValue → fill Unit::Pct`; the EXISTING layout
//! solver then computes the fill's `ComputedRect` from the `Pct` against the
//! track's content box, so the bar adds no geometry math and rides the same
//! `ComputedRect` the renderer reads (and there is a SINGLE `ComputedRect` writer
//! — the layout pass — no write-write race).
//!
//! Split discovery/apply, mirroring `ui_bind_*`/`ui_layout_*`:
//!
//! * [`ui_bar_discovery`] — a 0%-gate: `any_changed_since(UiValue, …)`. A still
//!   frame finds no changed `UiValue` and leaves `dirty == false`, so apply
//!   early-returns (no allocation, no tick bump).
//! * [`ui_bar_apply`] — exclusive (it reads one entity's `UiValue`/`Children` and
//!   writes another's `UiLayout`; nested cross-entity mutation is not a
//!   conflict-free parallel `Query`). Writes the fill's `Unit::Pct` SET-IF-CHANGED
//!   on a QUANTIZED fraction (M1) so an FP-noisy bound value does not flip the
//!   `Pct` by an ULP and force a relayout every frame.
//!
//! # Scheduling (C2)
//!
//! `UiWidgetsPlugin` schedules the bar systems `.after(ui_bind_apply)` (so a
//! `BindValue`-driven `UiValue` write is visible THIS frame) and
//! `.before(ui_layout_discovery)` (so the fill's `Unit::Pct` change is seen by the
//! same-frame relayout — the `ui_text_measure_system` precedent). The plugin
//! depends on `UiBindingPlugin` for the bind systems.

use std::mem;

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::schedule::system_set::SystemSet;
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_macros::Resource;

use crate::binding::UiValue;
use crate::components::{Bar, BarFill, UiLayout};
use crate::interaction::UiBindSet;
use crate::resources::UiSafeArea;
use crate::units::{LayoutType, Unit};

/// Fraction quantization step (M1): the bar fraction is snapped to `1 / STEPS`
/// before being turned into a `Unit::Pct`, so a mathematically-equal-but-not-bit-
/// identical bound value (FP rounding of `current/max`) does NOT flip the `Pct`
/// by an ULP and bump the fill's `Changed<UiLayout>` tick every frame (defeating
/// the 0%-overhead steady state). `10_000` steps = 0.01% resolution — finer than
/// any display can show, coarse enough to absorb FP noise.
const FRACTION_STEPS: f32 = 10_000.0;

/// The bar driver's shared state: the per-frame `dirty` gate, the
/// `(last_run, this_run]` window for the per-track tick compare, and the retained
/// widget-query buffers (alloc-free reuse — the `UiBindScratch` pattern).
///
/// `Default` is fully EMPTY so it is a valid `mem::take` target (apply moves the
/// buffers onto its stack for the world-mutating loop, then moves them back with
/// capacity retained).
#[derive(Resource, Default)]
pub struct UiBarScratch {
    /// Discovery's per-frame "some bar `UiValue` changed" flag, consumed (and
    /// cleared) by apply.
    pub dirty: bool,
    /// The tick at the start of the last apply (the `last_run` of the
    /// `(last_run, this_run]` window). Updated by apply each time it runs.
    pub last_run: Tick,
    /// Retained `(Bar, UiValue)` widget-query buffer (alloc-free per-frame reuse).
    bars: Vec<Entity>,
    /// Retained archetype-id scratch backing the widget query.
    arch_ids: Vec<ArchetypeId>,
}

/// Discovery: sets `dirty` when ANY [`UiValue`] column changed in the
/// `(last_run, this_run]` window. Cheap — `any_changed_since` is bounded to the
/// hosting archetypes and short-circuits; a still frame returns `false`.
///
/// Exclusive (`&mut EcsMaster`) so it reads the world's archetype change epochs
/// and the scratch window together — mirrors `ui_bind_discovery`.
//
// `clippy::needless_pass_by_ref_mut`: writes `dirty` via `&mut self` engine
// methods clippy cannot see through. Mirrors `ui_bind_discovery`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_bar_discovery(world: &mut EcsMaster) {
    let this_run = world.current_tick();
    let last_run = world.resource::<UiBarScratch>().last_run;
    // The bar fraction lives in `UiValue` (the P4 sink). A change to it — whether
    // authored directly or pushed by `ui_bind_apply` — is the only thing that can
    // move a fill, so gate on the `UiValue` column alone.
    let changed = world.any_changed_since(&[UiValue::component_id()], last_run, this_run);
    world.resource_mut::<UiBarScratch>().dirty = changed;
}

/// Apply: for each [`Bar`] track whose [`UiValue`] changed since `last_run`, sets
/// its [`BarFill`] child's main-axis size to `Unit::Pct(quantized_fraction *
/// 100)` SET-IF-CHANGED. The layout solver then computes the fill's rect.
///
/// Exclusive — reads the track's `UiValue`/`Children`/`UiLayout`, writes the fill
/// child's `UiLayout`. Runs only when `dirty`. Allocation-free: the widget list +
/// archetype-id scratch are taken out of the retained buffers and put back.
//
// `clippy::needless_pass_by_ref_mut`: writes the fill's `UiLayout` via `&mut self`
// engine methods clippy cannot see through. Mirrors `ui_bind_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_bar_apply(world: &mut EcsMaster) {
    let (dirty, last_run, this_run) = {
        let s = world.resource::<UiBarScratch>();
        (s.dirty, s.last_run, world.current_tick())
    };
    if !dirty {
        return;
    }

    let (mut bars, mut arch_ids) = {
        let s = world.resource_mut::<UiBarScratch>();
        (mem::take(&mut s.bars), mem::take(&mut s.arch_ids))
    };
    world.query_entities_buf(
        &[Bar::component_id(), UiValue::component_id()],
        &mut bars,
        &mut arch_ids,
    );

    for &track in &bars {
        // Per-track tick gate (Decision 5 pattern): only act on a track whose
        // `UiValue` advanced this window. A still track is skipped, so a frame in
        // which only ONE of many bars moved touches only that bar's fill.
        let Some(tick) = world.get_component_changed_tick(track, UiValue::component_id()) else {
            continue;
        };
        if !tick.is_newer_than(last_run, this_run) {
            continue;
        }

        let Some(value) = world.get_component::<UiValue>(track).map(|v| v.0) else {
            continue;
        };
        let pct = quantized_pct(value);

        // Pick the main axis from the track's layout type (Row → fill width,
        // Column → fill height). For an Overlay/Grid track the convention defaults
        // to width; a debug-assert flags a non-flow track (Open Question 4).
        let track_lt = world
            .get_component::<UiLayout>(track)
            .map(|l| l.layout_type)
            .unwrap_or(LayoutType::Row);
        debug_assert!(
            matches!(track_lt, LayoutType::Row | LayoutType::Column),
            "invariant: a Bar track should be Row or Column (got {track_lt:?}); defaulting fill to width"
        );

        let Some(fill) = find_bar_fill_child(world, track) else {
            continue;
        };
        set_fill_pct_if_changed(world, fill, track_lt, pct);
    }

    let s = world.resource_mut::<UiBarScratch>();
    s.bars = bars;
    s.arch_ids = arch_ids;
    s.dirty = false;
    s.last_run = this_run;
}

/// Snaps `value` to the canonical `0..=1` range, quantizes it to
/// `FRACTION_STEPS`, and returns the resulting `Unit::Pct` (M1). A non-finite
/// value collapses to 0% (a NaN `Pct` would never compare equal and would churn
/// the tick forever).
#[inline]
fn quantized_pct(value: f32) -> Unit {
    let clamped = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
    // Round to the nearest 1/STEPS, then back to a fraction — a stable repr that
    // absorbs FP noise in the bound value while keeping the set-if-changed gate
    // effective.
    let quantized = (clamped * FRACTION_STEPS).round() / FRACTION_STEPS;
    Unit::Pct(quantized * 100.0)
}

/// Finds the [`BarFill`]-marked child of `track`. Debug-asserts a track has at
/// most one such child; in release the FIRST one wins (a deterministic choice).
fn find_bar_fill_child(world: &EcsMaster, track: Entity) -> Option<Entity> {
    let children = world.get_component::<Children>(track)?;
    let mut found: Option<Entity> = None;
    for &child in children.as_slice() {
        if world.get_component::<BarFill>(child).is_some() {
            if found.is_some() {
                debug_assert!(false, "invariant: a Bar track has more than one BarFill child");
                break;
            }
            found = Some(child);
            #[cfg(not(debug_assertions))]
            break;
        }
    }
    found
}

/// Writes the fill's main-axis `Unit` SET-IF-CHANGED: only acquires the `Mut`
/// guard (which bumps `Changed<UiLayout>`) when the `Pct` actually differs, so a
/// re-applied identical fraction bumps no tick (the 0%-overhead steady state).
fn set_fill_pct_if_changed(world: &mut EcsMaster, fill: Entity, track_lt: LayoutType, pct: Unit) {
    // Row track → fill spans the main axis = width; Column → height; otherwise
    // width by convention.
    let is_width = matches!(track_lt, LayoutType::Row) || !matches!(track_lt, LayoutType::Column);
    let current = match world.get_component::<UiLayout>(fill) {
        Some(l) => *l,
        None => return,
    };
    let cur_axis = if is_width { current.width } else { current.height };
    if cur_axis == pct {
        return; // bit-identical: suppress the write so the tick does not bump.
    }
    if let Some(mut guard) = world.get_component_mut::<UiLayout>(fill) {
        if is_width {
            guard.width = pct;
        } else {
            guard.height = pct;
        }
    }
}

// ───────────────────────── plugin + ordering set ──────────────────────────

/// The [`SystemSet`] the bar driver systems run in (GUI P6a). Exposed so the host
/// can order the layout discovery AFTER it (`ui_layout_discovery.before_set` is
/// not available; the host writes `ui_layout_discovery` `.after_set(UiWidgetSet)`,
/// or registers layout after this plugin so the fill's `Unit::Pct` change is seen
/// by the same-frame relayout — the `ui_text_measure_system` precedent).
#[derive(Clone, Copy, Debug)]
pub struct UiWidgetSet;
impl SystemSet for UiWidgetSet {}

/// Wires the GUI P6a widget driver systems into an [`App`].
///
/// Inserts [`UiBarScratch`] + the [`UiSafeArea`]
/// resource (the anchor inset; default zero) and schedules
/// `[ui_bar_discovery → ui_bar_apply]` on [`CoreSchedule::Main`], in
/// [`UiWidgetSet`], ordered `.after_set(`[`UiBindSet`]`)` (C2) so a
/// `BindValue`-driven `UiValue` write is visible to the bar THIS frame.
///
/// # Ordering contract (C2)
///
/// * `UiWidgetSet.after_set(UiBindSet)` is wired HERE — add
///   [`UiBindingPlugin`](crate::interaction::UiBindingPlugin) BEFORE this plugin
///   (the bind systems join [`UiBindSet`]).
/// * The bar systems MUST run BEFORE `ui_layout_discovery` so the fill's
///   `Unit::Pct` change triggers the same-frame relayout. Layout scheduling is the
///   HOST's responsibility (the `lib.rs` layout/measure-ordering convention), so
///   the host registers `ui_layout_discovery` `.after_set(UiWidgetSet)` (or simply
///   after this plugin). Without that edge the bar lags one frame — the same
///   contract `ui_text_measure_system` has.
#[derive(Default)]
pub struct UiWidgetsPlugin;

impl UiWidgetsPlugin {
    /// Creates the plugin.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for UiWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiBarScratch::default());
        // The anchor safe-area inset: insert a zero default so a host that never
        // sets one still has the resource (the layout apply reads it via
        // `try_resource`, but inserting it keeps the resource present + host-
        // settable). Idempotent intent: a host may overwrite it per resize.
        if !app.world_mut().contains_resource::<UiSafeArea>() {
            app.insert_resource(UiSafeArea::default());
        }

        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            let discovery = b
                .add_system(ui_bar_discovery)
                .in_set(UiWidgetSet)
                .after_set(UiBindSet)
                .key();
            b.add_system(ui_bar_apply)
                .in_set(UiWidgetSet)
                .after_set(UiBindSet)
                .after(discovery);
        });
    }
}
