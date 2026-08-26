//! Sprite sheets and the flipbook — UI-ADVANCED rung S5
//! (`docs/UI-PLAN-SPRITES.md` S5, architecture D8a/D8b/D8c).
//!
//! Three things live here, and they are one mechanism:
//!
//! 1. [`UiSheetTable`] — the `Resource`-owned dense column of registered sheets,
//!    keyed by a dense [`SheetId`] (the [`FontTable`](crate::text::FontTable)
//!    handle discipline; never a `HashMap<name, sheet>`). Setup-time
//!    [`register`](UiSheetTable::register); never grows in-frame.
//! 2. [`UiSheet::frame_uv`] — the frame→UV arithmetic, the ONE spelling the
//!    render gather substitutes into the node's image inputs.
//! 3. [`ui_sprite_flipbook`] — one normal scheduled system advancing
//!    [`UiSpriteCursor`] and writing
//!    [`UiSpriteSheet::index`](crate::components::UiSpriteSheet::index).
//!
//! # The write verb is the rung's load-bearing decision
//!
//! The flipbook's per-frame write is
//! `Mut<UiSpriteSheet>::set_if_neq` — a TABLE component, through the verb that
//! stamps a change tick. Both halves are load-bearing:
//!
//! * **Table, not the dense cursor.** That tick IS the repaint signal: it is what
//!   `ui_render_discovery`'s `Query<(), Or<(Changed<C1>, …)>>` sees. A dense
//!   `Changed<C>` inside `Or<..>` was MEASURED never to fire on this kernel
//!   (`Or` overrides none of the dense hooks, so `HAS_DENSE` takes the trait
//!   default `false` and the inner term's fetch stays null) — a dense per-frame
//!   write would render a frozen first frame with no error, no panic and no
//!   failing assertion. See `docs/UI-PLAN-SPRITES.md` S-D16 (1).
//! * **`set_if_neq`, not `&mut`.** `&mut T` does not consult ticks at all, so the
//!   generation would never bump and the upload's per-slot gate would keep
//!   skipping. And `set_if_neq` rather than a plain deref so a 12 fps flipbook on
//!   a 60 Hz frame does not bump the generation on the four frames in five where
//!   the index does not move: the repaint churn is proportional to VISIBLE change,
//!   not to frame rate.
//!
//! # Ordering
//!
//! Register [`ui_sprite_flipbook`] `.before(ui_render_discovery)`. `Changed`
//! compares a row's changed tick against the READING system's `last_run`, so a
//! flipbook write that lands after discovery in frame N is seen in frame N+1 —
//! never a lost repaint, but a repaint one frame late, and a golden blessed under
//! the wrong order pins a stale picture. The order is the host's responsibility
//! (this module ships the system, not an App schedule), exactly as it is for the
//! layout pair and the text measure system.
//!
//! Since rung A0b the flipbook also needs
//! [`ui_clock_tick`](crate::animation::ui_clock_tick) ahead of it, for the same
//! reason and with the same remedy: the clock is written once per frame and read
//! here, so a tick ordered after this system hands it the PREVIOUS frame's delta.

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::iters::query::{Mut, Query};
use boyko_ecs::ecs::core::system::Res;
use boyko_macros::{Bundle, Resource};

use crate::animation::UiClock;
use crate::components::{SpriteAnimMode, UiSpriteAnim, UiSpriteCursor, UiSpriteSheet};

/// A one-field `#[derive(Bundle)]` wrapper carrying the DENSE
/// [`UiSpriteCursor`], so an insert verb can take it at all.
///
/// This is not a style choice. Dense storage (plan D0) SUPPRESSES the
/// single-component `Bundle` impl the derive normally emits — `boyko_macros`'
/// `component.rs` gates `bundle_items` on
/// `no_bundle || storage_bitset || storage_dense` — so
/// `insert(UiSpriteCursor::default())` is `error[E0277]: the trait bound
/// UiSpriteCursor: Bundle is not satisfied`. A wrapper bundle is the only
/// spelling that compiles, and it is the same `dense_d2_routing::T4DenseBundle`
/// idiom the S5 harnesses use.
#[derive(Bundle)]
struct SpriteCursorBundle {
    /// The cursor the hook materializes.
    cursor: UiSpriteCursor,
}

/// [`UiSpriteAnim`]'s `on_add` hook (UI-ADVANCED S6, `docs/UI-PLAN-SPRITES.md`
/// S-D20 (1)): materializes the node's [`UiSpriteCursor`] at its `Default`.
///
/// # Why this is a HOOK and not `#[require]` or a dispatch-side insert
///
/// [`ui_sprite_flipbook`] queries all three of the animation, the cursor and the
/// sheet, so an animation with no cursor is FROZEN — silently, with no panic and
/// no failing assertion. `#[require(UiSpriteCursor)]` is the obvious remedy and
/// MEASURED it panics on this kernel: the require pass resolves the required id's
/// `ComponentPool` in the target ARCHETYPE, and a dense id owns none. The hook
/// route reaches the one path that already learned the dense partition —
/// `InsertCommand` filters dense ids out of the table replace path — so the
/// deferred insert lands where the require expansion could not.
///
/// Inserting the cursor from the `.ui` dispatch instead would be THREE sites
/// (spawn, the reconcile's insert branch, the reconcile's remove branch) and
/// would make a `.ui` node carry a component a `ui!` node does not — a
/// `.ui` ≡ `ui!` ≡ hand-spawn divergence no comparator can see, because the
/// cursor is excluded from the authorable vocabulary by design. One landing at
/// the component is inherited by every construction site instead.
///
/// # The insert is DEFERRED, and `on_add` is not `on_insert`
///
/// The cursor is present after the outermost apply, not inside the window that
/// added the animation: the pairing is structural, not instantaneous. Nothing in
/// this campaign reads a cursor at spawn time, and it is stated because a future
/// reader must be told which.
///
/// `on_add` fires only on a NEW add, so (MEASURED) re-inserting an edited
/// `UiSpriteAnim` over an existing one — what a `.ui` hot-reload does when an
/// author changes `fps` — does NOT re-fire it, and a running cursor keeps its
/// phase. `on_insert` would reset it, and would also stomp
/// [`AnimatedSpriteBundle`](crate::bundles::AnimatedSpriteBundle)'s own cursor on
/// every re-insert rather than only on the spawn frame where the two values are
/// equal anyway.
///
/// # There is deliberately no symmetric `on_remove`
///
/// An animation REMOVED from a surviving node leaves its 8 B dense cursor row
/// behind, inert (the flipbook needs all three components) and self-healing (a
/// re-added animation gets a fresh `Default` cursor, MEASURED). The symmetric
/// `on_remove` hook that would tidy it is UNLANDABLE: `on_remove` also fires on
/// the per-component pass of a DESPAWN, where the entity is still live at hook
/// time and dead by the drain, and the enqueued removal then panics
/// `RemoveCommand::apply: stale entity`. A `w.is_alive(ctx.entity)` guard does
/// not help — MEASURED, it reads `true` at hook time. Despawn already reclaims
/// the dense row on its own, so the hook would buy nothing there and cost a hard
/// panic on every despawn of an animated node. Recorded in
/// `docs/OPEN-QUESTIONS.md`.
///
/// # Safety
///
/// The [`HookFn`](boyko_ecs::ecs::core::component::hooks::HookFn) contract: the
/// kernel calls this during a hook dispatch with an exclusively-borrowed live
/// world and the added entity's context. This body performs no direct storage
/// access — it only enqueues one structural command into the world-resident
/// deferred queue, which the outermost drain applies strictly later.
pub(crate) unsafe fn ui_sprite_anim_on_add(mut world: DeferredEcsMaster<'_>, ctx: HookContext) {
    world
        .commands()
        .entity(ctx.entity)
        .insert(SpriteCursorBundle { cursor: UiSpriteCursor::default() });
}

/// A registered sheet's dense handle — an index into [`UiSheetTable`], minted
/// only by [`UiSheetTable::register`]. The [`FontId`](crate::text::FontId)
/// shape, one subsystem over.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct SheetId(pub u16);

/// One registered sprite sheet: a bindless slot plus a UNIFORM grid over it
/// (architecture D8c — ragged/trimmed sheets and per-frame durations are
/// deferred with their shape recorded in the plan). `#[repr(C)]`, POD, 20 B.
///
/// It records no texture DIMENSIONS, because the engine records none anywhere:
/// every number here is normalized or a cell count, so the arithmetic in
/// [`frame_uv`](Self::frame_uv) needs no texel size.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiSheet {
    /// The bindless texture slot the sheet's atlas lives in — the same dense
    /// handle [`UiImage::texture`](crate::components::UiImage::texture) carries,
    /// and what the gather substitutes for it.
    pub slot: u32,
    /// Grid columns.
    pub cols: u16,
    /// Grid rows.
    pub rows: u16,
    /// How many of the `cols * rows` cells hold a frame. Trailing cells may be
    /// unused and are never sampled: an index at or above this is clamped.
    ///
    /// `0` leaves the sheet INERT — the node draws its `UiImage` unchanged. It
    /// is not an error and it does not suppress the sprite record: which records
    /// a node emits is decided by `ui_node_sub_codes` alone, and the gather is
    /// not allowed a second opinion.
    pub frame_count: u16,
    /// Explicit tail padding — SPELLED, so `inset_uv`'s 4-byte alignment is not
    /// reached through implicit padding (the `UiNineSlice::_pad` rule).
    pub _pad: [u8; 2],
    /// A per-frame inset in UV units, applied to all four sides of every frame.
    ///
    /// Its purpose is BILINEAR bleed: under `UiSamplerMode::Smooth` the hardware
    /// tap at a frame's outer edge straddles into the neighbouring frame, and a
    /// HALF-TEXEL inset (`0.5 / atlas_extent_in_texels` per axis) pulls the
    /// sampled range inside the frame's own texels. Under
    /// `UiSamplerMode::Pixel` (NEAREST) there is no tap to bleed and the field
    /// is inert; a `Pixel` sheet should use `(0.0, 0.0)`.
    ///
    /// It cannot fix a TILE SEAM, which is interior to the sub-rect rather than
    /// on its outer edge — that limitation is recorded rather than papered over.
    pub inset_uv: [f32; 2],
}

const _: () = assert!(size_of::<UiSheet>() == 20);
const _: () = assert!(align_of::<UiSheet>() == 4);

impl UiSheet {
    /// The UV sub-rect `(u0, v0, u1, v1)` of frame `index`, ROW-MAJOR
    /// (`col = index % cols`, `row = index / cols`), inset by
    /// [`inset_uv`](Self::inset_uv) on all four sides.
    ///
    /// ```text
    /// u0 = col / cols + inset.x      u1 = (col + 1) / cols - inset.x
    /// v0 = row / rows + inset.y      v1 = (row + 1) / rows - inset.y
    /// ```
    ///
    /// The caller is responsible for having clamped `index` below
    /// [`frame_count`](Self::frame_count) — the gather does, and counts it.
    /// A zero `cols`/`rows` (which [`UiSheetTable::register`] refuses to mint)
    /// would divide by zero, so both are floored to `1` here as well: this is a
    /// pure function reachable from a test with a hand-built `UiSheet`.
    #[inline]
    pub fn frame_uv(&self, index: u16) -> [f32; 4] {
        let cols = self.cols.max(1) as u32;
        let rows = self.rows.max(1) as u32;
        let i = index as u32;
        let col = (i % cols) as f32;
        let row = (i / cols) as f32;
        let cw = cols as f32;
        let rh = rows as f32;
        [
            col / cw + self.inset_uv[0],
            row / rh + self.inset_uv[1],
            (col + 1.0) / cw - self.inset_uv[0],
            (row + 1.0) / rh - self.inset_uv[1],
        ]
    }
}

/// The ECS-resident sprite-sheet table — a `Resource`-owned dense column
/// (Principle 0), the [`FontTable`](crate::text::FontTable) verb one subsystem
/// over. Registered once at setup; never grows in-frame.
#[derive(Resource, Default)]
pub struct UiSheetTable {
    /// Dense sheets, indexed by [`SheetId`]`.0`. Setup-time alloc.
    sheets: Vec<UiSheet>,
}

impl UiSheetTable {
    /// An empty table. A node whose [`UiSpriteSheet`] names an id this table has
    /// no entry for renders its `UiImage` unchanged.
    #[inline]
    pub fn new() -> Self {
        UiSheetTable { sheets: Vec::new() }
    }

    /// Registers a sheet, returning its dense [`SheetId`]. **Setup-only** — the
    /// table never grows in-frame.
    ///
    /// `cols`/`rows` of `0` are floored to `1` and `frame_count` is clamped to
    /// `cols * rows`, so a registered sheet's grid is always usable: the frame
    /// arithmetic downstream is a pure function with no error path, and this is
    /// the one gate it has.
    pub fn register(&mut self, mut sheet: UiSheet) -> SheetId {
        sheet.cols = sheet.cols.max(1);
        sheet.rows = sheet.rows.max(1);
        let cells = (sheet.cols as u32 * sheet.rows as u32).min(u16::MAX as u32) as u16;
        sheet.frame_count = sheet.frame_count.min(cells);
        let id = SheetId(self.sheets.len() as u16);
        self.sheets.push(sheet);
        id
    }

    /// Borrows the sheet for `id`, or `None` when the id was never registered.
    #[inline]
    pub fn get(&self, id: SheetId) -> Option<&UiSheet> {
        self.sheets.get(id.0 as usize)
    }

    /// The number of registered sheets.
    #[inline]
    pub fn len(&self) -> usize {
        self.sheets.len()
    }

    /// Whether no sheet is registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }
}

/// The UI's frame-delta hitch clamp, in seconds — `UI-PLAN-ANIMATION.md` AD1's
/// default, and **the crate's only definition of it** (AD9 (3)).
///
/// Without the clamp an alt-tab stall hands a UI consumer a multi-second delta
/// that skips whole cycles and jumps `Once` to its end.
///
/// # Its one reader is now [`UiClock::default`], not this module
///
/// S5 landed this const and applied it inline here, because the animation plan's
/// clock did not exist yet. Rung A0 landed
/// [`UiClock`], whose `Default` **references** this
/// const rather than restating `0.1`, and A0b moved
/// [`ui_sprite_flipbook`] onto that clock — so the clamp is applied once per
/// frame for every consumer instead of once per consumer. The name and the site
/// stayed put deliberately: no public symbol moves in the same rung that changes
/// a system's signature. Whichever rung deletes the last reader moves the
/// definition onto `UiClock` and drops this const.
pub const UI_FALLBACK_MAX_DELTA: f32 = 0.1;

/// Advances every animated sprite by one frame's delta (S5 item 5).
///
/// Reads [`UiSpriteAnim`] (author configuration), advances [`UiSpriteCursor`]
/// (the flipbook's private dense state), and writes
/// [`UiSpriteSheet::index`](UiSpriteSheet) through `Mut::set_if_neq` — see the
/// module doc for why each of those three is the component it is.
///
/// # The clock (A0b — `UI-PLAN-ANIMATION.md` AD9 (1), (2))
///
/// [`UiClock::dt_virtual`](crate::animation::UiClock::dt_virtual), which is the
/// CLAMPED VIRTUAL delta — the same arithmetic, from the same source, against
/// the same clamp value this system spelled inline before A0b, so the migration
/// is behaviour-preserving and `g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware`
/// re-runs unedited.
///
/// `dt_real` is AD9's default only for a lane carrying D15's per-row `flags`
/// bit, and the flipbook has no such bit: it has no endpoint to be robbed of by
/// a pause, so the real delta's "unclamped, unscaled, pause-blind" would make a
/// paused game keep animating and `set_relative_speed` do nothing — the two
/// defects S5 measured and AD9 ruled on. Register
/// [`ui_clock_tick`](crate::animation::ui_clock_tick) ahead of this system (or
/// add [`UiAnimationPlugin`](crate::animation::UiAnimationPlugin) and order this
/// system `.after_set(UiAnimationSet)`); a world with no
/// [`UiClock`] panics loudly at `get_param` rather
/// than animating on a stale zero.
pub fn ui_sprite_flipbook(
    clock: Res<UiClock>,
    mut sprites: Query<(&UiSpriteAnim, Mut<UiSpriteCursor>, Mut<UiSpriteSheet>)>,
) {
    let dt = clock.dt_virtual();
    // A plain `<=` rather than the NaN-safe `!(dt > 0.0)`: `dt_virtual` is derived from a
    // `Duration` and `min`-ed against a finite, positive, setter-validated clamp, so it is
    // always finite and non-negative and cannot be a NaN. (The pack's own degenerate guards
    // DO need the negated form, because their inputs are author-written floats — see
    // `ui_nine_slice_tiles_axis`.)
    if dt <= 0.0 {
        // A paused clock (or the very first frame) advances nothing — and does
        // NOT touch a cursor, so an idle frame stamps no ticks anywhere.
        return;
    }

    for (anim, mut cursor, mut sheet) in sprites.iter_mut() {
        let Some(step_secs) = frame_duration(anim) else {
            continue;
        };
        if anim.last < anim.first {
            // A degenerate range holds `first`; there is nothing to walk.
            continue;
        }
        if budget_spent(anim, cursor.loops_done) {
            continue;
        }

        // Accumulate, then take whole steps. A `while` rather than one step:
        // a slow frame must not silently drop the frames it covered, and a
        // `dt` clamped to UI_FALLBACK_MAX_DELTA bounds the loop at
        // `UI_FALLBACK_MAX_DELTA * fps` iterations.
        let mut elapsed = cursor.elapsed + dt;
        let mut index = sheet.index;
        let mut dir = cursor.dir;
        let mut loops_done = cursor.loops_done;
        while elapsed >= step_secs {
            elapsed -= step_secs;
            let (next, next_dir, completed) = advance(anim, index, dir);
            if completed {
                loops_done = loops_done.saturating_add(1);
                if budget_spent(anim, loops_done) {
                    // The budget ends ON this step, so the step is NOT taken:
                    // every mode holds the frame its last cycle ended on —
                    // `Forward`/`Once` hold `last`, `Reverse` and `PingPong`
                    // hold `first`. Taking the step would instead hold the
                    // frame the NEXT cycle would have started from, which is
                    // the wrong end of the range and is what makes `Once`
                    // useless.
                    elapsed = 0.0;
                    break;
                }
            }
            index = next;
            dir = next_dir;
        }

        // The cursor is DENSE and flipbook-private, so its change tick is read
        // by nobody — a plain deref is correct here and costs no repaint. It is
        // written unconditionally because `elapsed` always moved (`dt > 0`).
        cursor.elapsed = elapsed;
        cursor.dir = dir;
        cursor.loops_done = loops_done;
        // THE repaint signal (module doc): a table write through the
        // tick-stamping verb, and only when the frame actually moved.
        sheet.set_if_neq(UiSpriteSheet {
            sheet: sheet.sheet,
            index,
        });
    }
}

/// One frame's duration in seconds, or `None` when the animation is stopped
/// (`fps <= 0` or non-finite — an author's way to pause without removing a
/// component).
#[inline]
fn frame_duration(anim: &UiSpriteAnim) -> Option<f32> {
    if anim.fps.is_finite() && anim.fps > 0.0 {
        Some(1.0 / anim.fps)
    } else {
        None
    }
}

/// Whether [`UiSpriteAnim::repeats`]'s cycle budget is exhausted. `repeats == 0`
/// is INFINITE and never is.
#[inline]
fn budget_spent(anim: &UiSpriteAnim, loops_done: u8) -> bool {
    let budget = match anim.mode {
        // `Once` IS `Forward` with `repeats == 1`, and the component doc says so
        // — the two knobs are defined against each other rather than left to
        // collide.
        SpriteAnimMode::Once => 1,
        _ => anim.repeats,
    };
    budget != 0 && loops_done >= budget
}

/// One frame step: returns `(index, dir, completed_a_cycle)`.
///
/// The turn is taken BEFORE the step, not after: at `last` going forward the next
/// frame is `last - 1`, so `PingPong` shows each endpoint ONCE per round trip.
/// Flipping after the step instead repeats the endpoint, which is the classic
/// flipbook off-by-one an eyeball check cannot see (red mutation M5-d).
#[inline]
fn advance(anim: &UiSpriteAnim, index: u16, dir: i8) -> (u16, i8, bool) {
    let first = anim.first;
    let last = anim.last;
    if last == first {
        // A one-frame range: nothing to walk, and counting a cycle per tick
        // would burn the repeat budget at frame rate.
        return (first, dir, false);
    }
    // An index outside the range (an author retarget between ticks) re-enters at
    // the range's own end rather than walking from wherever it was.
    let index = index.clamp(first, last);

    match anim.mode {
        SpriteAnimMode::Forward | SpriteAnimMode::Once => {
            if index >= last {
                (first, dir, true)
            } else {
                (index + 1, dir, false)
            }
        }
        SpriteAnimMode::Reverse => {
            if index <= first {
                (last, dir, true)
            } else {
                (index - 1, dir, false)
            }
        }
        SpriteAnimMode::PingPong => {
            if dir >= 0 {
                if index >= last {
                    (index - 1, -1, false)
                } else {
                    (index + 1, 1, false)
                }
            } else if index <= first {
                // Back at the start end: one full round trip is a cycle.
                (index + 1, 1, true)
            } else {
                (index - 1, -1, false)
            }
        }
    }
}
