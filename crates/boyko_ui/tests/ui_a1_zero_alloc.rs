//! **A1 gate 6 — zero per-frame allocation on the steady animating path**
//! (`docs/UI-PLAN-ANIMATION-A1.md` A1 gate 6, Principle 5).
//!
//! The crate's established shape: a counting global allocator plus BASELINE
//! SUBTRACTION (`zero_alloc.rs`, `p4_bind_zero_alloc.rs`,
//! `text_emit_zero_alloc.rs`). An absolute "0" is not assertable here for the
//! same reason it is not there — `Schedule::run`'s parallel executor allocates a
//! small fixed number of bytes per frame for its own task machinery, a property
//! of the executor and not of the systems' bodies.
//!
//! So both arms run a schedule of **identical SHAPE** — two normal systems and
//! one exclusive system — over an **identical world** (the same animated nodes,
//! spawned the same way), and the delta is the A1 pair's own per-frame cost:
//!
//! * baseline = `[ui_clock_tick, noop_normal, noop_exclusive]`
//! * pair     = `[ui_clock_tick, ui_visual_tick, ui_tween_reap]`
//!
//! `ui_clock_tick` is in BOTH arms deliberately: the tick under test reads the
//! clock, so removing it from the baseline would make the baseline a different
//! schedule rather than a shapeless one.
//!
//! **That choice has a CONSEQUENCE, and it went unstated for six passes: the
//! clock tick's own cost is added to both sides and CANCELS.** No widening of this
//! fixture could make a cost inside `ui_clock_tick` visible here — not another
//! cohort, not another frame, not another repetition — because the cancellation is
//! structural, not statistical. MEASURED 2026-08-28, a
//! `black_box(Vec::<u8>::with_capacity(64))` at the end of the tick left this
//! gate, the census, `ui_a1_tween` and the whole crate green. The third member of
//! `UiAnimationSet` therefore has a gate of its OWN below —
//! [`ui_clock_tick_allocates_zero_over_a_same_shape_baseline`] — whose baseline is
//! [`noop_clock`]: the tick's exact `SystemParam` signature with an empty body, so
//! the two arms differ in the body and in nothing else.
//!
//! # The coverage claim is PINNED SOMEWHERE ELSE, and that is the point
//!
//! Which branches of `ui_visual_tick` and `ui_tween_reap` this fixture drives
//! used to be a prose table in this file's gate doc. Four consecutive adversarial
//! passes found the same defect — a live branch nothing here executed — at a new
//! site each time, and each repair closed the named site and left its siblings;
//! the 2026-08-27 widening closed "the reap loop never runs" by putting a
//! completion on EVERY armed frame, which **swapped that blind spot for its exact
//! complement** (the ordinary frame, where nothing completes at all).
//!
//! A prose table cannot converge on that, because a branch that nobody thought of
//! is absent from the table and absent from the fixture, and the two absences look
//! identical. So the table moved to `tests/ui_a1_source_census.rs`, where it is
//! **pinned data over a scan of `src/animation.rs`**: every control-flow line of
//! the two systems and of their intra-file callees is enumerated in source order,
//! each with its paths, and each path either names the cohort here that drives it
//! or states why it is not covered. A branch in neither column reds; a branch
//! ADDED to either system reds; a branch DELETED reds. The census also checks that
//! every driver named there occurs in THIS file, which is what stops the two from
//! drifting apart.
//!
//! What follows is the FIXTURE's side of that contract — five cohorts, and why
//! each exists.
//!
//! # The five cohorts
//!
//! All five live in the SAME world, which both arms build identically: a cohort on
//! one side only would recreate the "structurally different workloads" defect the
//! floor exists to avoid.
//!
//! * **steady** — all four channels, [`STEADY_MS`], real clock. Keeps all four
//!   `Some(t)` arms, `advance`'s real-clock lane and `Mut::set_if_neq`'s
//!   *value-changed* path executing on every armed frame.
//! * **completing** — all four channels at the four staggered durations, so
//!   `channels()[i]` completes on armed frame `i` **and on no other**. That
//!   one-to-one map is what gives each `None` arm and the reap's loop body a frame
//!   index of its own, so the per-index floor in [`armed_floor`] preserves them
//!   ([`armed_floor`] asserts the whole matrix, not merely its row sums). From
//!   armed frame 1 on it is also the only cohort that reaches a channel's
//!   `if let Some(row) = …` with that arm **absent** and others present.
//! * **rested** — warmed and never restarted, so it enters the window carrying a
//!   `UiVisual` and NO channel. Dense storage keeps a reaped channel out of the
//!   archetype signature, so such a node still matches
//!   `Query<(Mut<UiVisual>, AnyOf<(…)>)>` and is yielded with all four arms
//!   `None` — the path every at-rest animated node takes, i.e. the majority path
//!   in any real UI. MEASURED 2026-08-27 against the fixture that lacked it: a
//!   `black_box(Vec::<u8>::with_capacity(64))` immediately before that `continue`
//!   was **GREEN**, and a `panic!` there left this gate **passing** while hanging
//!   three tests in `ui_a1_tween`. [`rested_rows`] asserts it is really there and
//!   really all-`None`.
//! * **virtual_clock** — all four channels, [`STEADY_MS`], and
//!   [`TWEEN_FLAG_VIRTUAL_CLOCK`] set. `advance` SELECTS its delta on that bit,
//!   and before this cohort existed every fixture row passed `flags = 0`: the
//!   virtual lane was live production code with **zero** executions in the window.
//!   MEASURED 2026-08-28 by the sixth adversarial pass —
//!   `black_box(Vec::<u8>::with_capacity(64))` in that lane, GREEN 10/10.
//!   The whole fixture now runs at [`VIRTUAL_SPEED`], which is what makes this
//!   cohort's claim FALSIFIABLE: at the default `relative_speed` of `1.0` the two
//!   lanes carry the same number and taking the wrong one is unobservable. The
//!   seventh pass neutered the cohort by passing `0` for its flags — ONE argument,
//!   with the `let virtual_clock: Vec<Entity>` binding untouched — and nothing
//!   reddened. [`witness_virtual_lane_reads_the_other_delta`] is what reddens now.
//! * **tint_only** — ONE channel ([`TweenTint`]), [`STEADY_MS`], driving
//!   `0x0000_0000 → 0xFFFF_FFFF`. Two branches are its alone. It reaches the
//!   opacity / offset / scale `if let`s with those arms **absent** on every frame;
//!   and its composed `tint_mul` is a constant `0` from the second warm frame on
//!   (`lerp1(0, 255, t) + 0.5` truncates to `0` until `t >= 1/510`, i.e. until
//!   `elapsed >= 1.18 s`, and this window closes at `0.208 s`), so with nothing
//!   else on the node to move, `composed == *sink` and **`Mut::set_if_neq` takes
//!   its equality short-circuit on every armed frame**. MEASURED 2026-08-28 by
//!   the sixth pass: that path was GREEN 20/20 before this cohort, hidden only
//!   because every node also carried three moving `f32` channels.
//!
//! # Why the window has FIVE frames and the fifth is empty
//!
//! Four armed frames with a completion on each is the complement of an ordinary
//! UI frame, not a superset of it: `ui_tween_reap`'s `for` loop had **zero**
//! zero-iteration executions, so a cost on the "nothing to reap" path was
//! invisible (MEASURED 2026-08-28, GREEN 10/10). [`EMPTY_REAP_FRAME`] is the
//! fifth index; no threshold falls in it, so `done` is empty, the loop runs zero
//! times, and the ordinary frame is inside the window beside the completing ones.
//!
//! [`UiTweenScratch`]'s retained buffer is nonetheless warmed to a real
//! high-water mark BEFORE the window, by a burst of completions, so a `0` below
//! cannot be read as "the buffer was never used" — and so `Vec::push`'s
//! reallocation path deliberately does NOT fire inside the window.

// Test-harness plumbing only: a file-static `Mutex<()>` serializes the tests that
// arm the process-global allocator counter. Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]
#![cfg(not(miri))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::AnyOf;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::*;
use boyko_macros::Component;

use boyko_ui::animation::{
    start_tween_offset, start_tween_opacity, start_tween_scale, start_tween_tint, ui_clock_tick,
    ui_tween_reap, ui_visual_tick, UiClock, UiTweenScratch,
};
use boyko_ui::components::{
    EasingId, TweenOffset, TweenOpacity, TweenScale, TweenTint, UiVisual,
    TWEEN_FLAG_VIRTUAL_CLOCK,
};

// ───────────────────────── armed-window serialization ─────────────────────

static ARM_LOCK: Mutex<()> = Mutex::new(());

fn lock_arm() -> std::sync::MutexGuard<'static, ()> {
    ARM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ───────────────────────── counting allocator ─────────────────────────────

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

// SAFETY: forwards every call verbatim to the system allocator; the only added
// behavior is an atomic increment on alloc/realloc when armed.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn count_allocs(f: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    f();
    ARMED.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed)
}

// ───────────────────────── fixtures ────────────────────────────────────────

const FRAME: Duration = Duration::from_millis(16);
/// Nodes per cohort. Small enough to stay fast, plural enough that a per-row
/// allocation would be counted several times over. The fixture holds FIVE
/// cohorts — steady, completing, rested, virtual_clock and tint_only.
const NODES: usize = 32;

/// Unmeasured frames before the window: brings the schedule and every retained
/// buffer to steady state.
const WARM_FRAMES: usize = 8;
/// The MEASURED frames: four on which exactly one channel completes, plus
/// [`EMPTY_REAP_FRAME`].
const ARMED_FRAMES: usize = 5;

/// The armed frame index on which NOTHING completes — the ordinary frame of any
/// real UI, and the one `ui_tween_reap`'s zero-iteration path needs.
///
/// The four staggered durations all fall in armed frames 0..=3 (see [`TINT_MS`]
/// and its siblings), so on this index `UiTweenScratch`'s buffer is empty when
/// the reap takes it and the `for` loop runs zero times. Before this index
/// existed the fixture put a completion on EVERY armed frame by construction, so
/// that path had zero executions — MEASURED 2026-08-28,
/// `black_box(Vec::<u8>::with_capacity(64))` on it was GREEN 10/10.
///
/// The signal it buys is `+1` per frame, not `+NODES`: the reap runs once per
/// frame and that path is outside the loop. A deterministic `+1` is exactly what
/// the per-index FLOOR can see and a `.max()` could not — see [`armed_floor`].
const EMPTY_REAP_FRAME: usize = 4;

/// Independent repetitions of the WHOLE window (fresh world, fresh schedule,
/// fresh warm-up) that the per-frame-index floor in [`armed_floor`] is taken
/// over.
///
/// # Why 8, MEASURED
///
/// 2026-08-27, release, this box, 200 repetitions per arm, per frame index —
/// the probability that a frame index carries the sporadic executor `+1`:
///
/// | arm | idle | 16 CPU spinners |
/// |---|---|---|
/// | baseline | 0.335 / 0.205 / 0.200 / 0.250 | 0 / 0 / 0 / 0.025 |
/// | pair | 0.100 / 0.035 / 0.020 / 0.005 | 0.005 / 0.005 / 0 / 0.020 |
///
/// The deterministic floor was **6 on every index of every arm in all 1600
/// samples** — `min` was 6 and `max` was 6, 7 or 8. The longest run of
/// CONSECUTIVE noisy repetitions on one index was **7** (baseline, idle,
/// frame 0) and **2** on the pair side.
///
/// That table was taken over the FOUR-frame, three-cohort window. Re-measured
/// 2026-08-28 over the five-frame, five-cohort one: the floor is still
/// `[6, 6, 6, 6, 6]` on both arms, in every one of the 15 mutation sweeps below
/// and in 100 idle plus 60 saturated clean runs. Two more cohorts and a fifth
/// frame move the WORK, not the executor's per-frame allocation count, which is
/// the property the subtraction rests on.
///
/// A per-index floor is wrong only when EVERY repetition is noisy on that index.
/// At the worst measured rate that is `0.335^8 = 1.6e-4` per index — and on the
/// BASELINE side, which inflates the subtrahend and can only make the gate more
/// lenient. On the pair side, which is where a false RED would come from, the
/// worst rate gives `0.100^8 = 1e-8`. Eight repetitions cost ~18 ms idle and
/// ~170 ms saturated; the gate as a whole stays well under a second.
const REPS: usize = 8;

/// The steady, virtual_clock and tint_only cohorts' duration: nothing completes,
/// ever, inside this test.
const STEADY_MS: f32 = 600_000.0;

/// The completing cohort's four staggered durations, one completion per ARMED
/// frame — and NONE on [`EMPTY_REAP_FRAME`].
///
/// At [`FRAME`] = 16 ms, elapsed after the 8 warm frames is 0.128 s and the five
/// armed frames land on 0.144 / 0.160 / 0.176 / 0.192 / 0.208 s. Each threshold
/// below sits ~4 ms inside its frame — far outside f32 accumulation drift
/// (~1e-7) — and every one exceeds the 0.128 s the warm-up reaches, so NOTHING
/// completes before the window opens. That is what makes each of the four `None`
/// arms and the reap loop body execute INSIDE the measured window. The largest is
/// 188 ms, comfortably below the 0.192 s frame-3 mark and further still below
/// frame 4's 0.208 s, which is what leaves [`EMPTY_REAP_FRAME`] empty.
const TINT_MS: f32 = 140.0;
const OPACITY_MS: f32 = 156.0;
const OFFSET_MS: f32 = 172.0;
const SCALE_MS: f32 = 188.0;

/// `Time::relative_speed` for the whole fixture — the reason the virtual lane is
/// OBSERVABLE and not merely executed.
///
/// `advance` selects between `UiClock::dt_real` and `UiClock::dt_virtual` on a
/// per-row flag bit. At the default speed of `1.0` those two numbers are EQUAL,
/// so a fixture can drive the virtual lane and produce no evidence that it did:
/// every row advances by the same amount either way. The seventh adversarial pass
/// used exactly that — it neutered the cohort by passing `0` for its flags, one
/// argument, and nothing anywhere reddened while the census kept reporting the
/// lane covered.
///
/// At `0.5` the two deltas differ by construction (`dt_real` is raw and
/// unscaled — `Time::real_delta` — while `dt_virtual` is the scaled one), so the
/// cohort that claims to read the other lane must end the window with a
/// measurably smaller `elapsed` than a cohort that does not. See
/// [`witness_virtual_lane_reads_the_other_delta`].
///
/// It changes nothing else about the window: the completing cohort passes
/// `flags = 0` and its four staggered thresholds are `dt_real` thresholds, and the
/// warm burst's 1 ms tweens complete on the first frame either way.
const VIRTUAL_SPEED: f32 = 0.5;

#[derive(Component, Clone, Copy, Debug)]
struct Node;

/// The five cohorts' entity handles, kept so the witnesses can READ them.
///
/// [`seeded_world`] used to drop all five (`drop(rested)` was the only trace one
/// of them left). Two of the five then had no observable at all — their coverage
/// was a claim that a `let` binding existed, which is a claim about the fixture's
/// TEXT and not about its behaviour.
struct Cohorts {
    steady: Vec<Entity>,
    completing: Vec<Entity>,
    rested: Vec<Entity>,
    virtual_clock: Vec<Entity>,
    tint_only: Vec<Entity>,
}

fn noop_normal(_q: Query<&UiVisual>) {}
fn noop_exclusive(_w: &mut EcsMaster) {}

/// The clock gate's BASELINE system: `ui_clock_tick`'s `SystemParam` signature
/// with an empty body.
///
/// It exists so the clock gate's two arms differ in exactly one thing — the body
/// of `ui_clock_tick`. A baseline that simply omitted the system would be
/// measuring the scheduler's cost of running a third system, not the tick's.
fn noop_clock(_t: Res<Time>, _c: ResMut<UiClock>) {}

/// Live rows in a dense channel's store. `0` when the store was never created.
fn live(world: &EcsMaster, id: ComponentId) -> usize {
    world.dense_registry().store(id).map_or(0, |s| s.live_count())
}

/// Rows that `ui_visual_tick`'s OWN query shape yields with all four `AnyOf`
/// arms `None` — i.e. the rows that take the rested `continue`.
///
/// Spelled as the tick's query rather than as "nodes carrying a `UiVisual` and
/// no channel", because those two are the same set only while `AnyOf` keeps
/// yielding rows none of whose arms are present. That is the property the rested
/// cohort's coverage rests on, and it is a KERNEL property (`AnyOf` forwards
/// `resolve_dense` per arm and does not filter the archetype) that this file
/// does not own. If it ever changes, the rested branch silently stops executing
/// and this census reads 0 — which is the whole reason it counts through the
/// query instead of through the stores.
///
/// Runs OUTSIDE the armed window (`run_system` has machinery of its own).
fn rested_rows(world: &mut EcsMaster) -> usize {
    #[allow(clippy::type_complexity)]
    fn count(
        q: Query<(&UiVisual, AnyOf<(&TweenTint, &TweenOpacity, &TweenOffset, &TweenScale)>)>,
    ) -> usize {
        q.iter()
            .filter(|(_, (a, b, c, d))| {
                a.is_none() && b.is_none() && c.is_none() && d.is_none()
            })
            .count()
    }
    world.run_system(count)
}

/// One entity's [`UiVisual`] sink, read OUTSIDE the armed window.
///
/// Spelled through the query rather than through the store because the sink is a
/// TABLE component and the entity's archetype changes as channels are reaped —
/// the query follows it, a cached location would not.
fn visual_of(world: &mut EcsMaster, e: Entity) -> UiVisual {
    world.run_system(move |q: Query<&UiVisual>| {
        q.iter_entities()
            .find_map(|(id, v)| (id == e.id()).then_some(*v))
            .expect("invariant: every cohort node carries a UiVisual sink from its on_add hook")
    })
}

/// One entity's live `TweenTint::elapsed`, or `None` if that channel is gone.
///
/// Read through `ui_visual_tick`'s OWN query shape — a dense channel behind an
/// `AnyOf` — for the same reason [`rested_rows`] is: that is the shape whose
/// behaviour the coverage claims rest on.
#[allow(clippy::type_complexity)]
fn tint_elapsed_of(world: &mut EcsMaster, e: Entity) -> Option<f32> {
    world.run_system(
        move |q: Query<(
            &UiVisual,
            AnyOf<(&TweenTint, &TweenOpacity, &TweenOffset, &TweenScale)>,
        )>| {
            q.iter_entities()
                .find_map(|(id, (_, (t, _, _, _)))| (id == e.id()).then_some(t.map(|r| r.elapsed)))
                .flatten()
        },
    )
}

/// Live rows in each of the four channel columns, in [`channels`] order.
fn live_per_channel(world: &EcsMaster) -> [usize; 4] {
    let mut out = [0usize; 4];
    for (slot, &(_, id)) in out.iter_mut().zip(channels().iter()) {
        *slot = live(world, id);
    }
    out
}

/// The four channel columns, for the per-phase census below.
///
/// **This ORDER is load-bearing and is the same order as the four staggered
/// durations** `[TINT_MS, OPACITY_MS, OFFSET_MS, SCALE_MS]` passed to
/// [`start_all_four`] for the completing cohort. That is what makes
/// `channels()[i]` the channel that completes on armed frame `i`, which is the
/// coupling [`armed_floor`]'s census pins and [`REPS`]'s per-index floor
/// depends on. Reordering either list without the other reds the census.
fn channels() -> [(&'static str, ComponentId); 4] {
    [
        ("TweenTint", TweenTint::component_id()),
        ("TweenOpacity", TweenOpacity::component_id()),
        ("TweenOffset", TweenOffset::component_id()),
        ("TweenScale", TweenScale::component_id()),
    ]
}

/// Live rows each channel column carries once the window's cohorts are started,
/// in [`channels`] order.
///
/// Tint is carried by FOUR cohorts (steady, completing, virtual_clock,
/// tint_only); the other three by three. The asymmetry IS the tint_only cohort,
/// and it is what puts the opacity / offset / scale `if let Some(row) = …` arms
/// on their `None` path on every armed frame.
fn expected_live() -> [usize; 4] {
    [4 * NODES, 3 * NODES, 3 * NODES, 3 * NODES]
}

/// Starts all four channels on every node of `batch`, at the four given
/// durations and with the given per-row `flags`.
///
/// `flags` is the parameter the virtual_clock cohort exists for:
/// [`TWEEN_FLAG_VIRTUAL_CLOCK`] selects `advance`'s OTHER delta lane, and a
/// fixture where every row passes `0` never executes it.
fn start_all_four(world: &mut EcsMaster, batch: Vec<Entity>, ms: [f32; 4], flags: u8) {
    world.run_system(move |mut cmds: Commands| {
        for &e in &batch {
            start_tween_tint(&mut cmds, e, 0, 0xFFFF_FFFF, ms[0], EasingId::LINEAR, flags);
            start_tween_opacity(&mut cmds, e, 0.0, 1.0, ms[1], EasingId::LINEAR, flags);
            start_tween_offset(
                &mut cmds,
                e,
                [0.0, 0.0],
                [-400.0, 0.0],
                ms[2],
                EasingId::LINEAR,
                flags,
            );
            start_tween_scale(&mut cmds, e, [1.0, 1.0], [2.0, 2.0], ms[3], EasingId::LINEAR, flags);
        }
    });
}

/// Starts ONLY the tint channel on every node of `batch` — the tint_only cohort.
///
/// `0x0000_0000 → 0xFFFF_FFFF` over [`STEADY_MS`]: each byte interpolates as
/// `(lerp1(0, 255, t) + 0.5) as u32`, which is `0` for every `t < 1/510`, i.e.
/// for every `elapsed` below 1.18 s. The measured window closes at 0.208 s, so
/// the composed `tint_mul` is a constant `0` throughout it — and with no other
/// channel on the node to move a field, `composed == *sink` and
/// `Mut::set_if_neq` takes its equality short-circuit.
fn start_tint_only(world: &mut EcsMaster, batch: Vec<Entity>, ms: f32) {
    world.run_system(move |mut cmds: Commands| {
        for &e in &batch {
            start_tween_tint(&mut cmds, e, 0x0000_0000, 0xFFFF_FFFF, ms, EasingId::LINEAR, 0);
        }
    });
}

/// A world with FIVE cohorts of [`NODES`] animated nodes and every retained
/// buffer already at a real high-water mark.
///
/// The window must exercise BOTH halves of the pair, and no fixture before this
/// one did so completely. MEASURED 2026-08-27 against the ONE-cohort fixture
/// (steady durations only, tint + offset only): a `Vec::with_capacity(64)` behind
/// a `black_box` in `ui_tween_reap`'s loop body was **green**, and the same in
/// `ui_visual_tick`'s `scale` arm was **green** — the reap loop had zero
/// iterations and two of the four channel arms did not exist in the fixture at
/// all. MEASURED 2026-08-28 against the THREE-cohort fixture that replaced it:
/// four further sites were green, including the exact complement of the one the
/// widening had just closed. The gate's RESOLUTION has never been the problem;
/// its WINDOW has, four passes running. **Which branches this world drives is now
/// pinned data in `tests/ui_a1_source_census.rs`, checked against a scan of
/// `src/animation.rs`** — see the module header.
///
/// * The **warm burst** drives all four channels to completion three times, so
///   all four dense free lists AND [`UiTweenScratch`]'s `Vec` reach high water
///   BEFORE the window. Without this the offset/scale free lists grow inside the
///   armed window and the gate reds for a reason that is not the tick. It is also
///   what puts the rested cohort in its resting state and what gives the
///   tint_only cohort a sink with all four fields already at a finished value —
///   it is the `on_add` hook that gives a node its `UiVisual` at all, so a cohort
///   that never animated would carry no sink and would not be in the tick's query.
/// * **steady**, **completing**, **rested**, **virtual_clock** and **tint_only**
///   are described one by one in the module header, each with the branch that is
///   its alone and the measurement showing that branch was green without it.
fn seeded_world() -> (EcsMaster, Cohorts) {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    // The virtual lane's OBSERVABILITY, not merely its execution — see
    // `VIRTUAL_SPEED` and `witness_virtual_lane_reads_the_other_delta`.
    world.resource_mut::<Time>().set_relative_speed(VIRTUAL_SPEED);

    let steady: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });
    let completing: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });
    let rested: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });
    let virtual_clock: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });
    let tint_only: Vec<Entity> = world.run_system(|mut cmds: Commands| {
        (0..NODES).map(|_| cmds.spawn(Node).id()).collect::<Vec<_>>()
    });

    // Warm every retained buffer: SHORT tweens on ALL FOUR channels of every node
    // of all five cohorts, driven to completion. A `0` delta below is then "no
    // growth", not "never used".
    let mut everyone: Vec<Entity> = Vec::with_capacity(5 * NODES);
    everyone.extend_from_slice(&steady);
    everyone.extend_from_slice(&completing);
    everyone.extend_from_slice(&rested);
    everyone.extend_from_slice(&virtual_clock);
    everyone.extend_from_slice(&tint_only);
    for _ in 0..3 {
        start_all_four(&mut world, everyone.clone(), [1.0; 4], 0);
        world.resource_mut::<Time>().advance_with(FRAME);
        world.run_system(ui_clock_tick);
        world.run_system(ui_visual_tick);
        world.run_system(ui_tween_reap);
    }

    // The steady state under test: nothing completing, all four `Some` arms live.
    start_all_four(&mut world, steady.clone(), [STEADY_MS; 4], 0);
    // …and one completion per armed frame 0..=3, so the reap loop body and all
    // four `None` arms run INSIDE the measured window — and none on
    // `EMPTY_REAP_FRAME`, which is what leaves the reap's zero-iteration path in
    // it too.
    start_all_four(&mut world, completing.clone(), [TINT_MS, OPACITY_MS, OFFSET_MS, SCALE_MS], 0);
    // The OTHER delta lane of `advance`'s per-row select. The flags argument is
    // the whole cohort: pass 0 here and this world becomes a four-cohort world
    // with a fifth batch of ordinary rows in it. That neuter is not textually
    // visible anywhere — `witness_virtual_lane_reads_the_other_delta` is what
    // makes it fail.
    start_all_four(
        &mut world,
        virtual_clock.clone(),
        [STEADY_MS; 4],
        TWEEN_FLAG_VIRTUAL_CLOCK,
    );
    // One channel only: the three absent-arm `if let`s, and `set_if_neq`'s
    // equality short-circuit.
    start_tint_only(&mut world, tint_only.clone(), STEADY_MS);
    // `rested` is deliberately NOT restarted — that omission IS the third cohort.
    (world, Cohorts { steady, completing, rested, virtual_clock, tint_only })
}

// ───────────────────────── the executable coverage column ─────────────────
//
// `tests/ui_a1_source_census.rs` names one of these on every covered path. It
// checks two things about the name, neither of them a substring search: that the
// function EXISTS in this file's syntax tree, and that it is REACHABLE from a
// `#[test]` here by this file's own call graph. A witness that is defined and
// never called reds the census exactly as loudly as one that was deleted.
//
// The column used to be `Cover::By("let virtual_clock: Vec<Entity>")` asserting
// that string occurred somewhere in this file. MEASURED 2026-08-28 by the seventh
// adversarial pass: changing ONE ARGUMENT of that cohort's starter call
// (`TWEEN_FLAG_VIRTUAL_CLOCK` → `0`) neutered the lane completely while the `let`
// binding sat untouched — cohort armed ⇒ the gate RED under a planted
// `with_capacity(64)`, cohort neutered ⇒ GREEN 3/3, census EXIT=0 in both. A
// coverage column that survives the deletion of the coverage is not a column.

/// Which schedule an [`armed_floor`] run is measuring.
///
/// Two of the witnesses below can only run on the pair arm: the baseline runs
/// `noop_normal`/`noop_exclusive`, so no tween advances and no sink is written,
/// and asserting that a sink "did not move" there would pass for the wrong reason.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Pair,
    Baseline,
}

/// **The steady / completing / virtual_clock / tint_only cohorts are all live on
/// the channels their coverage rows claim.**
///
/// Called on BOTH sides of the warm window, so it proves two different things with
/// one assertion: before any frame, that the world was seeded as the census says;
/// after the warm-up, that the warm-up completed NOTHING, i.e. that every
/// completion counted later lands INSIDE the measured window rather than before it.
fn witness_steady_cohort_keeps_every_channel_live(world: &EcsMaster, rep: usize, when: &str) {
    for (c, (name, id)) in channels().into_iter().enumerate() {
        assert_eq!(
            live(world, id),
            expected_live()[c],
            "rep {rep}, {name}, {when}: the steady, completing and virtual_clock cohorts are live \
             on every channel and tint_only on TINT alone — and the RESTED cohort contributes none \
             of them, which is exactly what makes it rested. The tint-vs-rest asymmetry IS the \
             tint_only cohort, and it is what drives the other three channels' `if let Some(row) = \
             …` arms onto their `None` path"
        );
    }
}

/// **The rested cohort really is resting, and really is yielded by the tick's own
/// query with all four arms `None`.**
///
/// A `0` here means `ui_visual_tick`'s all-`None` `continue` executes ZERO times
/// inside the armed window and this gate cannot see a cost planted in it —
/// MEASURED 2026-08-27, the two-cohort fixture that preceded the rested cohort was
/// green under a `with_capacity(64)` there and green under a `panic!` there.
/// The second assertion is why this takes the cohort HANDLES and not just a
/// count: `rested_rows` returns `NODES` for any `NODES` all-`None` rows, whoever
/// they are. Naming the handle proves the rows taking that branch are the cohort
/// the census says takes it.
fn witness_rested_cohort_takes_the_all_none_arm(world: &mut EcsMaster, c: &Cohorts, rep: usize) {
    assert_eq!(
        rested_rows(world),
        NODES,
        "rep {rep}: the window opens with exactly the rested cohort taking ui_visual_tick's \
         all-`None` `continue`"
    );
    let rested = c.rested[0];
    assert!(
        tint_elapsed_of(world, rested).is_none(),
        "rep {rep}: the rested cohort's own node still carries a live TweenTint, so the NODES \
         all-`None` rows counted above are somebody else's and this cohort is not the one taking \
         the branch its coverage row claims"
    );
    // Reached through the query, so it also proves the row is still VISITED: a
    // node with no sink is not in the tick's query at all, and the `continue` it
    // is supposed to drive would then never be reached from here.
    let _ = visual_of(world, rested);
}

/// **The virtual_clock cohort reads `UiClock::dt_virtual` and the steady cohort
/// reads `UiClock::dt_real` — and at [`VIRTUAL_SPEED`] those are different
/// numbers, so the claim is falsifiable.**
///
/// This is the witness the substring column did not have. `advance`'s per-row
/// clock select is `if flags & TWEEN_FLAG_VIRTUAL_CLOCK != 0 { dt_virtual } else
/// { dt_real }`; at the default `relative_speed` of `1.0` both arms yield the same
/// value and a fixture that took the wrong one would be indistinguishable from one
/// that took the right one. At `0.5` the virtual cohort must end the window with
/// roughly HALF the steady cohort's `elapsed`.
///
/// Pair arm only: the baseline never runs the tick, so both would read 0.
fn witness_virtual_lane_reads_the_other_delta(world: &mut EcsMaster, c: &Cohorts, rep: usize) {
    let steady = tint_elapsed_of(world, c.steady[0])
        .expect("invariant: the steady cohort's tint runs for STEADY_MS and cannot have been reaped");
    let virt = tint_elapsed_of(world, c.virtual_clock[0]).expect(
        "invariant: the virtual_clock cohort's tint runs for STEADY_MS and cannot have been reaped",
    );
    assert!(
        steady > 0.15,
        "rep {rep}: the steady cohort's tint accrued {steady} s of REAL time over \
         WARM_FRAMES + ARMED_FRAMES frames of {FRAME:?}, and it must be ~0.208. If this is 0 the \
         tick never ran and every coverage claim in the census is about code this window does not \
         execute"
    );
    assert!(
        virt > steady * 0.4 && virt < steady * 0.6,
        "rep {rep}: the virtual_clock cohort accrued {virt} s against the steady cohort's \
         {steady} s, and at relative_speed {VIRTUAL_SPEED} it must be ~half. Equal means the \
         cohort is passing `flags = 0` and is a SECOND ordinary cohort: `advance`'s virtual delta \
         lane then has ZERO executions inside the armed window, and the census's row for it is a \
         claim about a lane nothing drives. That neuter is one argument wide and leaves the `let \
         virtual_clock: Vec<Entity>` binding — the text the old coverage column searched for — \
         completely untouched"
    );
}

/// **The tint_only cohort drives `Mut::set_if_neq`'s EQUALITY short-circuit, and
/// the steady cohort drives its writing path.**
///
/// The composed `tint_mul` of a tint_only node is a constant `0` across the whole
/// window (`lerp1(0, 255, t) + 0.5` truncates to `0` until `elapsed >= 1.18 s`,
/// and the window closes at 0.208 s), and the node carries nothing else that
/// moves — so `composed == *sink` on every armed frame and the verb takes its
/// short-circuit. The steady node carries three moving `f32` channels and takes
/// the other path.
///
/// Both halves are asserted, because "the sink did not move" passes trivially in a
/// world where nothing moves at all. Pair arm only.
fn witness_tint_only_sink_never_moves(
    world: &mut EcsMaster,
    c: &Cohorts,
    before: (UiVisual, UiVisual),
    rep: usize,
) {
    let (tint_before, steady_before) = before;
    let tint_after = visual_of(world, c.tint_only[0]);
    let steady_after = visual_of(world, c.steady[0]);
    assert!(
        tint_before == tint_after,
        "rep {rep}: the tint_only cohort's sink MOVED across the armed window ({tint_before:?} → \
         {tint_after:?}). Its whole purpose is that it does not: with a constant composed value \
         and nothing else on the node, `Mut::set_if_neq` takes its equality short-circuit on every \
         armed frame. MEASURED 2026-08-28, that path was GREEN 20/20 before this cohort existed, \
         hidden only because every other node also carried three moving f32 channels"
    );
    assert!(
        steady_before != steady_after,
        "rep {rep}: the steady cohort's sink did NOT move across the armed window \
         ({steady_before:?}). Then the assertion above passes for the wrong reason — a world where \
         nothing moves proves nothing about a verb that only writes when something does"
    );
}

/// **Exactly one channel completes per armed frame, and it is `channels()[i]`.**
///
/// That one-to-one map is what gives each of `ui_visual_tick`'s four `None` arms a
/// frame index of its own, and it is the whole reason a per-index floor can see
/// them. A duration edit that moves a completion to another frame (or out of the
/// window) deletes that arm from this gate's coverage and leaves it green — the
/// 2026-08-27 defect, twice over.
fn witness_completing_cohort_completes_one_channel_per_armed_frame(
    world: &mut EcsMaster,
    c: &Cohorts,
    drops: &[[usize; 4]; ARMED_FRAMES],
    rep: usize,
) {
    for i in 0..ARMED_FRAMES {
        if i == EMPTY_REAP_FRAME {
            continue;
        }
        let mut want = [0usize; 4];
        want[i] = NODES;
        assert_eq!(
            drops[i], want,
            "rep {rep}, armed frame {i}: exactly one thing may complete on it, and it must be {}. \
             Got {:?}, wanted {want:?}",
            channels()[i].0,
            drops[i]
        );
    }
    // The matrix is a count; these two say WHICH cohort the counts belong to.
    // `channels()[0]` is TINT and it completes on armed frame 0, so by the end of
    // the window the completing cohort's tint is gone and the steady cohort's —
    // STEADY_MS, 600 s — is not.
    assert!(
        tint_elapsed_of(world, c.completing[0]).is_none(),
        "rep {rep}: the completing cohort's own node still carries a live TweenTint at the end of \
         the window. Its TINT_MS threshold falls in armed frame 0, so the drop matrix's NODES on \
         index 0 is being contributed by rows that are not this cohort"
    );
    assert!(
        tint_elapsed_of(world, c.steady[0]).is_some(),
        "rep {rep}: the steady cohort's TweenTint was reaped inside the window. It runs for \
         STEADY_MS and must survive it — otherwise the `Some(t)` arms have no driver on the later \
         frames and the assertion above passes because EVERYTHING completed"
    );
}

/// **Nothing completes on [`EMPTY_REAP_FRAME`], so `ui_tween_reap`'s `for` loop
/// runs ZERO times on it.**
///
/// The ordinary frame of any real UI, and the exact complement of the four
/// completing ones. The 2026-08-27 widening gave every armed frame a completion,
/// which swapped one blind spot for its complement: MEASURED 2026-08-28, a
/// `with_capacity(64)` on the zero-iteration path was GREEN 10/10 before this
/// index existed.
fn witness_empty_reap_frame_completes_nothing(drops: &[[usize; 4]; ARMED_FRAMES]) {
    assert_eq!(
        drops[EMPTY_REAP_FRAME],
        [0usize; 4],
        "a completion landed on EMPTY_REAP_FRAME ({:?}), which deletes the reap's ZERO-ITERATION \
         path from this gate's coverage — the 2026-08-28 defect",
        drops[EMPTY_REAP_FRAME]
    );
}

fn build_pair(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let tick = b.add_system(ui_visual_tick).after(clock).key();
    b.add_system(ui_tween_reap).after(tick);
    b.build(world)
}

fn build_baseline(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let noop = b.add_system(noop_normal).after(clock).key();
    b.add_system(noop_exclusive).after(noop);
    b.build(world)
}

/// The clock gate's PAIR arm: `ui_clock_tick` under measurement, with the same
/// two noops the A1 gate's baseline uses so the schedule SHAPE is unchanged.
fn build_clock_pair(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(ui_clock_tick).key();
    let noop = b.add_system(noop_normal).after(clock).key();
    b.add_system(noop_exclusive).after(noop);
    b.build(world)
}

/// The clock gate's BASELINE arm: identical but for [`noop_clock`], which has
/// `ui_clock_tick`'s exact `SystemParam` signature and an empty body.
///
/// The scheduler derives access from the signature, so both arms take the same
/// `Res<Time>` + `ResMut<UiClock>` access, build the same dependency graph and run
/// the same number of systems in the same shape. The ONLY difference is the four
/// statements in the tick's body.
fn build_clock_baseline(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let clock = b.add_system(noop_clock).key();
    let noop = b.add_system(noop_normal).after(clock).key();
    b.add_system(noop_exclusive).after(noop);
    b.build(world)
}

/// The [`WARM_FRAMES`] UNMEASURED frames. Split out of the old `warmed_allocs`
/// so the test can read live counts on BOTH sides of the window and prove the
/// completions happened INSIDE it.
fn warm(world: &mut EcsMaster, sched: &mut Schedule) {
    for _ in 0..WARM_FRAMES {
        world.resource_mut::<Time>().advance_with(FRAME);
        sched.run(world);
    }
}

/// ONE repetition's [`ARMED_FRAMES`] MEASURED frames: the per-frame armed
/// `run()` counts, and the per-frame, PER-CHANNEL live-row drop.
///
/// The drop matrix is `drops[frame][channel]`, in [`channels`] order. Both
/// census reads sit OUTSIDE the armed window.
fn armed_once(
    world: &mut EcsMaster,
    sched: &mut Schedule,
) -> ([usize; ARMED_FRAMES], [[usize; 4]; ARMED_FRAMES]) {
    let mut counts = [0usize; ARMED_FRAMES];
    let mut drops = [[0usize; 4]; ARMED_FRAMES];
    for i in 0..ARMED_FRAMES {
        let before = live_per_channel(world);
        world.resource_mut::<Time>().advance_with(FRAME);
        counts[i] = count_allocs(|| sched.run(world));
        let after = live_per_channel(world);
        for (slot, (b, a)) in drops[i].iter_mut().zip(before.iter().zip(after.iter())) {
            *slot = b - a;
        }
    }
    (counts, drops)
}

/// [`REPS`] independent repetitions of the whole window, reduced to the
/// **per-frame-index FLOOR**: `floor[i] = min over repetitions of counts[r][i]`.
///
/// Returns that vector, the per-frame per-channel drop matrix, and the count of
/// all-`None` rows the window CLOSES on. The latter two are asserted IDENTICAL
/// across every repetition before they are returned; the all-`None` count is
/// arm-dependent (the baseline reaps nothing, so the completing cohort never
/// joins the rested one there) and is therefore checked by the caller, which is
/// the only place that knows which arm it asked for.
///
/// # The noise, and why a floor
///
/// MEASURED 2026-08-27, release, this box. Both arms have a DETERMINISTIC
/// per-frame cost of **6** allocations with byte-identical size histograms
/// (`[≥16 B ×1, ≥32 B ×2, ≥256 B ×2, ≥1024 B ×1]`), and the pair's two systems
/// contribute **zero** of them — `ui_visual_tick` and `ui_tween_reap` run
/// standalone over completion frames give a constant `2` and `0`, both of which
/// are `run_system`'s own machinery. On top of that floor sits a sporadic +1
/// (occasionally +2) charged ALWAYS to a THREADPOOL WORKER thread and never to
/// the thread running this test: 64 baseline frames split `total=6 ⇒ on_main=6,
/// off_main=0` (57 frames) and `total=7 ⇒ on_main=6, off_main=1` (7 frames). It
/// is `Schedule::run`'s executor, it appears with `noop_normal`/`noop_exclusive`
/// exactly as readily as with the pair (baseline-only probe, 348 frames:
/// `6×313, 7×34, 8×1`), and the fixture CANNOT quiet it — that executor is the
/// path under test.
///
/// A `.max()` on each side samples that noise independently and compares two
/// samples. MEASURED, it fails BOTH ways: clean code went RED **3 in 60**
/// isolated idle runs (always `baseline 6, pair 7`), and the gate's OWN
/// documented red — a `Vec::with_capacity(64)` in `ui_visual_tick`'s body,
/// signal = **1** — went GREEN **2 in 60** runs under CPU load, because
/// `base_max` absorbed the +1 as noise. A floor removes both, because
/// allocation noise here is strictly ADDITIVE.
///
/// # Why the floor is over REPETITIONS and not over FRAMES
///
/// **This is the whole point of this function, and getting the AXIS wrong is a
/// measured defect this gate has already shipped once.**
///
/// A `min` over the frames of ONE window collapses the window to a single
/// number. That number is blind to any cost that does not raise EVERY frame —
/// and most of this gate's signals are exactly that shape, BY CONSTRUCTION: the
/// staggered `TINT/OPACITY/OFFSET/SCALE_MS` durations exist so each channel
/// completes on a frame of its own, so each `None` arm's body executes on
/// exactly ONE armed frame; and [`EMPTY_REAP_FRAME`] exists so the reap's
/// zero-iteration path executes on exactly one OTHER one. MEASURED 2026-08-27, a
/// `black_box(Vec::<u8>::with_capacity(64))` planted in `ui_visual_tick`'s
/// `scale` `None` arm, on the four-frame window of the day:
///
/// ```text
/// counts = [6, 6, 6, 38]        drops row sums = [32, 32, 32, 32]
/// min over FRAMES = 6  ⇒  pair <= base  ⇒  GREEN, 40 runs out of 40
/// ```
///
/// The signal was 32 allocations on one frame and the frame-axis floor threw it
/// away. A per-frame-index floor keeps it: index 3 reads 38 against the
/// baseline's 6. On today's five-frame window the sharpest instance is the reap's
/// zero-iteration path, whose signal is a single allocation on index 4 alone
/// (`[6,6,6,6,7]`) — a frame-axis `min` would read 6 and a `.max()` would read it
/// as executor noise.
///
/// The two axes differ because the two quantities differ in KIND. The noise is
/// per-frame-RANDOM — it lands on an arbitrary frame of an arbitrary repetition,
/// so repeating the window and taking the minimum at a FIXED index removes it.
/// The signal is per-frame-DETERMINISTIC — channel `i`'s arm costs the same on
/// index `i` of every repetition, so that same minimum preserves it. `min` over
/// frames removes both; `min` over repetitions of the same index removes only
/// the noise.
///
/// Do NOT "fix" the noise by counting only main-thread allocations instead.
/// Normal systems run on workers, so a system-body allocation is off-main too;
/// that spelling would mask the very regression this gate exists for, and
/// nothing here has measured otherwise.
fn armed_floor(
    build: fn(&mut EcsMaster) -> Schedule,
    arm: Arm,
) -> ([usize; ARMED_FRAMES], [[usize; 4]; ARMED_FRAMES], usize) {
    let mut floor = [usize::MAX; ARMED_FRAMES];
    let mut drops = [[0usize; 4]; ARMED_FRAMES];
    let mut after = 0usize;

    for r in 0..REPS {
        let (mut world, cohorts) = seeded_world();
        let mut sched = build(&mut world);

        witness_steady_cohort_keeps_every_channel_live(&world, r, "before any frame");
        warm(&mut world, &mut sched);
        witness_steady_cohort_keeps_every_channel_live(&world, r, "after the warm-up");
        witness_rested_cohort_takes_the_all_none_arm(&mut world, &cohorts, r);

        // Read OUTSIDE the armed window, on both sides of it: the two witnesses
        // below are about what the window DID, and `run_system` has machinery of
        // its own that must not be counted.
        let sink_before = (
            visual_of(&mut world, cohorts.tint_only[0]),
            visual_of(&mut world, cohorts.steady[0]),
        );

        let (counts, d) = armed_once(&mut world, &mut sched);

        if arm == Arm::Pair {
            witness_virtual_lane_reads_the_other_delta(&mut world, &cohorts, r);
            witness_tint_only_sink_never_moves(&mut world, &cohorts, sink_before, r);
            witness_completing_cohort_completes_one_channel_per_armed_frame(
                &mut world, &cohorts, &d, r,
            );
            witness_empty_reap_frame_completes_nothing(&d);
        }

        let post = rested_rows(&mut world);
        assert!(
            r == 0 || post == after,
            "rep {r} ended the window with {post} rested rows, rep 0 with {after}"
        );
        after = post;
        assert!(
            r == 0 || d == drops,
            "rep {r} reaped a DIFFERENT schedule of completions than rep 0 ({d:?} vs {drops:?}). \
             The per-frame-index floor is only meaningful while every repetition puts the same \
             channel's `None` arm on the same frame index"
        );
        drops = d;
        for (slot, &c) in floor.iter_mut().zip(counts.iter()) {
            *slot = (*slot).min(c);
        }
    }

    (floor, drops, after)
}

// ───────────────────────── the gate ────────────────────────────────────────

/// **A1 gate 6.** The tick + reap pair allocates nothing of its own on the
/// steady animating path.
///
/// Red mutation 6: a `Vec::with_capacity(64)` behind a `black_box` at any of the
/// **fourteen** covered sites — `ui_visual_tick`'s body, any one of its four
/// `Some(t)` arms, any one of its four `None` arms, its rested all-`None` arm,
/// `advance`'s virtual-clock lane, `Mut::set_if_neq`'s equality short-circuit,
/// `ui_tween_reap`'s loop body, or the reap's zero-iteration path — reds this.
/// The table below has every one with its per-index signal. (A `Vec::new()` — the
/// mutation this rung shipped with — is capacity-0 and never calls the allocator;
/// measured green 2026-08-27.)
///
/// # What the statistic covers, and what it does not
///
/// The comparison is **per armed frame index**: `pair[i] <= base[i]` for each of
/// the [`ARMED_FRAMES`], each side a floor over [`REPS`] independent repetitions
/// of the whole window (see [`armed_floor`] for the axis and why it is that one).
///
/// A cost inside this pair is invisible to that comparison for exactly **two**
/// reasons, and they are independent. Both are stated here because the second
/// one shipped undocumented and cost this gate a whole branch.
///
/// **(1) The STATISTIC can absorb it.** The floor removes any cost that is
/// random in the frame index rather than fixed to one: a per-frame allocation
/// firing on a coin flip is indistinguishable, by construction, from the
/// executor noise the floor exists to remove. Nothing in either system has that
/// shape today, and a rung that gives one to them must re-argue this gate.
///
/// **(2) The FIXTURE can never execute it.** A cost perfectly deterministic in
/// the frame index is still invisible if the branch carrying it never runs
/// inside the armed window. That is not a resolution question and no statistic
/// fixes it — only the world does. MEASURED 2026-08-27: the two-cohort fixture
/// never ran the rested all-`None` arm, and a `panic!` planted there left this
/// gate PASSING while hanging three tests in `ui_a1_tween`.
///
/// So the coverage claim is a claim about the FIXTURE — and it is no longer a
/// table in this doc. **It is pinned data in `tests/ui_a1_source_census.rs`,
/// checked against a scan of `src/animation.rs`:** every control-flow line of
/// `ui_visual_tick`, `ui_tween_reap` and their intra-file callees, in source
/// order, each path either naming the cohort HERE that drives it or stating why
/// it is not covered. That census also asserts every driver it names occurs in
/// this file, and that the two systems call nothing this file has not
/// enumerated. A branch in neither column reds; an added or deleted branch reds.
///
/// The table lived here for four adversarial passes and each pass found a live
/// branch missing from it. A prose table cannot converge on that, because a
/// branch nobody thought of is absent from the table and absent from the fixture
/// and the two absences look identical. The census is the shape this repository
/// already uses for exactly that failure — the print census, the ignore-reasons
/// census, the refusal census, the trybuild-corpus witness.
///
/// `armed_floor` still proves the fixture end mechanically — the per-channel live
/// counts on both sides of the warm window, [`rested_rows`] on both sides of the
/// armed window, and the per-frame per-channel drop matrix in between.
///
/// What the armed window does **not** execute, and therefore what this gate does
/// not cover whatever its statistic:
///
/// * `ui_visual_sink_on_add` — the sink's `on_add` hook fires on channel
///   INSERT, and every insert in this fixture happens before the window opens.
///   Not claimed anywhere; a gate for it belongs where the insert is.
/// * `ui_clock_tick` — it is in BOTH arms by design, so it raises `base[i]` and
///   `pair[i]` equally and the subtraction cancels it. It is uncoverable HERE by
///   construction. **Fixed, not merely recorded, on 2026-08-28**: it has its own
///   single-arm gate below, [`ui_clock_tick_allocates_zero_over_a_same_shape_baseline`],
///   whose baseline is [`noop_clock`] rather than an omission — so the two arms
///   differ in exactly the four statements of the tick's body. The previous
///   spelling of this bullet stated the CHOICE ("it is in both arms by design")
///   and not its CONSEQUENCE (no widening of this fixture could ever cover it),
///   which is why the hole survived six adversarial passes with a row in the
///   documentation the whole time.
///
/// MEASURED 2026-08-28, `black_box(Vec::<u8>::with_capacity(64))` planted at one
/// site at a time, release, 10 runs each. The baseline floor is `[6, 6, 6, 6, 6]`
/// in every row; the pair floor is what the site's coverage predicts, which is
/// the point — a per-index signal that is not the predicted one means the fixture
/// is driving something other than what the census claims:
///
/// | site | pair floor | runs |
/// |---|---|---|
/// | `ui_visual_tick` body | `[166,166,166,166,166]` (5 x NODES) | RED 10/10 |
/// | rested all-`None` arm | `[38,38,38,38,70]` (NODES; 2 x NODES once the completing cohort rests) | RED 10/10 |
/// | `advance`'s virtual lane | `[134,134,134,134,134]` (4 x NODES) | RED 10/10 |
/// | `set_if_neq`'s EQUAL path | `[38,38,38,38,38]` (NODES — tint_only, nobody else) | RED 10/10 |
/// | `Some(t)`, tint | `[102,102,102,102,102]` (3 x NODES) | RED 10/10 |
/// | `Some(t)`, opacity | `[102,70,70,70,70]` | RED 10/10 |
/// | `Some(t)`, offset | `[102,102,70,70,70]` | RED 10/10 |
/// | `Some(t)`, scale | `[102,102,102,70,70]` | RED 10/10 |
/// | `None`, tint | `[38,6,6,6,6]` | RED 10/10 |
/// | `None`, opacity | `[6,38,6,6,6]` | RED 10/10 |
/// | `None`, offset | `[6,6,38,6,6]` | RED 10/10 |
/// | `None`, scale | `[6,6,6,38,6]` | RED 10/10 |
/// | `ui_tween_reap` loop body | `[38,38,38,38,6]` — **6 on the empty frame** | RED 10/10 |
/// | reap's ZERO-ITERATION path | `[6,6,6,6,7]` — `+1`, and only there | RED 10/10 |
/// | reap's missing-store `else` | `[6,6,6,6,6]` | **GREEN 10/10 — the recorded non-coverage** |
///
/// The reap's two rows are worth reading together: the loop BODY signals on
/// 0..=3 and is silent on 4, the zero-iteration path signals on 4 and is silent
/// on 0..=3. That disjointness is the proof that the fifth frame is a genuinely
/// new path and not a fifth copy of the first four.
///
/// Clean code over the same binary: 0 red in 100 idle runs and 0 red in 60 runs
/// under 64 CPU spinners on 16 cores.
///
/// # The non-vacuity census is the point
///
/// Both halves of the pair must actually EXECUTE inside the armed window, and no
/// fixture before 2026-08-28 fully did — see [`seeded_world`]. [`armed_floor`]
/// proves the pre-window and post-warm halves on EVERY repetition; the drop
/// matrix below proves the rest; and `tests/ui_a1_source_census.rs` proves the
/// SET of branches those two are being asked about is the set the source
/// actually has.
#[test]
fn the_steady_animating_path_allocates_zero_over_baseline() {
    let _arm = lock_arm();

    let (base, base_drops, base_rested_after) = armed_floor(build_baseline, Arm::Baseline);
    let (pair, pair_drops, pair_rested_after) = armed_floor(build_pair, Arm::Pair);

    // The rested cohort's coverage, closed at both ends. `armed_floor` already
    // asserted `NODES` all-`None` rows on the frame the window OPENS on, in both
    // arms — that is the branch's execution proof. These two say the population
    // did not drift underneath the window, and they say it ARM BY ARM because the
    // two arms legitimately differ: the pair reaps, so the completing cohort has
    // joined the rested one by the last frame; the baseline has no reap at all,
    // so it must not have.
    assert_eq!(
        base_rested_after, NODES,
        "the baseline arm runs no tick and no reap, so nothing may join the rested cohort inside \
         the window (got {base_rested_after}, wanted {NODES}). If this grows, the baseline is no \
         longer the shapeless schedule the subtraction assumes"
    );
    assert_eq!(
        pair_rested_after,
        2 * NODES,
        "the pair arm closes the window with the completing cohort newly rested beside the rested \
         cohort (got {pair_rested_after}, wanted {}). This is the header's claim made mechanical: \
         the completing cohort's own rest begins on the LAST measured tick's reap, which is why \
         it can never substitute for a cohort that was already resting when the window OPENED",
        2 * NODES
    );

    // The baseline runs `noop_normal`/`noop_exclusive`, so nothing it does can
    // complete a tween. If this ever reads a drop, the two arms are no longer
    // comparing the executor against the executor.
    assert_eq!(
        base_drops,
        [[0usize; 4]; ARMED_FRAMES],
        "the baseline arm reaped channel rows ({base_drops:?}): it has no tick and no reap, so \
         the subtraction is no longer executor-against-executor"
    );

    // The per-index floor's precondition, made mechanical — and made mechanical
    // at the RESOLUTION the floor now has. The previous spelling asserted only
    // the row SUMS (`NODES` rows reaped per frame), which is satisfied by any
    // assignment of channels to frames, the all-on-one-frame assignment
    // included; under it three of the four `None` arms would execute on one
    // index and this gate would silently stop covering them. The matrix below
    // pins WHICH channel completes on WHICH frame, which is what gives each arm
    // an index of its own. MEASURED 2026-08-27, identical on all 8 repetitions,
    // idle and under 16 CPU spinners.
    // The two drop-matrix witnesses ran INSIDE `armed_floor`, on every repetition
    // of the pair arm and against the world that produced the matrix — which is
    // what lets them name the cohort by handle rather than by count. What is left
    // here is the aggregate the floors were actually taken over.
    assert_eq!(
        pair_drops[EMPTY_REAP_FRAME], [0usize; 4],
        "the aggregate drop matrix disagrees with the per-repetition witness about \
         EMPTY_REAP_FRAME ({:?})",
        pair_drops[EMPTY_REAP_FRAME]
    );

    // Non-vacuity of the SUBTRACTION: the method means nothing while the baseline
    // allocates nothing, because then every delta below is trivially satisfied
    // and a schedule that never ran would pass.
    for (i, &b) in base.iter().enumerate() {
        assert!(
            b > 0,
            "armed frame {i}: the baseline schedule allocated nothing, so the subtraction has no \
             subtrahend (baseline floors {base:?})"
        );
    }
    for i in 0..ARMED_FRAMES {
        let what = if i == EMPTY_REAP_FRAME {
            "the frame on which NOTHING completes, so a signal here alone is the reap's \
             zero-iteration path — the ordinary frame of any real UI"
        } else {
            "the frame on which channels()[i] completes, so a signal here alone is that \
             channel's `None` arm or a reap keyed to it"
        };
        assert!(
            pair[i] <= base[i],
            "steady animating state, armed frame {i}: the A1 pair must allocate no more than the \
             scheduler baseline (baseline floors {base:?}, pair floors {pair:?}). Frame {i} is \
             {what}. A signal on ALL FIVE is the tick body, the rested arm, `advance`'s shared \
             lines, or an unkeyed reap"
        );
    }
}

// ───────────────────────── the clock's own gate ────────────────────────────

/// Frames of the clock gate's measured window.
///
/// `ui_clock_tick` has no per-row work and no completion schedule, so it needs
/// neither the five-frame stagger nor the five cohorts: its body is four
/// statements over two resources and it does the same thing on every frame. What
/// it does need is a window longer than one frame, so a cost that fires on some
/// frames and not others still lands on a fixed index.
const CLOCK_FRAMES: usize = 3;

/// A world with the two resources `ui_clock_tick` reads and writes, and nothing
/// else.
///
/// Deliberately EMPTY of entities: the tick touches no component column, so
/// seeding one would only add noise from `noop_normal`'s query.
fn clock_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    world.insert_resource(UiClock::default());
    world.insert_resource(UiTweenScratch::default());
    world.resource_mut::<Time>().set_relative_speed(VIRTUAL_SPEED);
    world
}

/// The per-frame-index allocation floor of one clock arm, over [`REPS`]
/// independent repetitions — the same axis and the same reasoning as
/// [`armed_floor`], over a world with nothing in it.
fn clock_floor(build: fn(&mut EcsMaster) -> Schedule) -> [usize; CLOCK_FRAMES] {
    let mut floor = [usize::MAX; CLOCK_FRAMES];
    for _ in 0..REPS {
        let mut world = clock_world();
        let mut sched = build(&mut world);
        for _ in 0..WARM_FRAMES {
            world.resource_mut::<Time>().advance_with(FRAME);
            sched.run(&mut world);
        }
        for slot in floor.iter_mut() {
            world.resource_mut::<Time>().advance_with(FRAME);
            let n = count_allocs(|| sched.run(&mut world));
            *slot = (*slot).min(n);
        }
    }
    floor
}

/// **`ui_clock_tick` allocates nothing of its own.**
///
/// # Why this test exists at all
///
/// `UiAnimationSet` has THREE members. Until 2026-08-28 the third had no
/// allocation gate anywhere in either crate, and — this is the part that matters —
/// it was **structurally impossible for the A1 gate to grow one**.
///
/// `the_steady_animating_path_allocates_zero_over_baseline` is a DIFFERENTIAL: it
/// runs `[ui_clock_tick, ui_visual_tick, ui_tween_reap]` against
/// `[ui_clock_tick, noop_normal, noop_exclusive]` and asserts `pair[i] <=
/// base[i]`. The tick is in BOTH arms, deliberately — the systems under test read
/// the clock, so a baseline without it would be a different schedule rather than a
/// shapeless one. But that choice has a consequence its own header stated only as
/// a preference: **the tick's cost is added to both sides and CANCELS**. No widening
/// of that fixture — not another cohort, not another frame, not another
/// repetition — could ever make a cost inside `ui_clock_tick` visible there.
/// MEASURED 2026-08-28: a `black_box(Vec::<u8>::with_capacity(64))` at the end of
/// `ui_clock_tick` left `ui_a1_source_census` EXIT=0, the A1 gate EXIT=0,
/// `ui_a1_tween` EXIT=0, and the whole crate at 345 passed.
///
/// So the tick gets a gate whose baseline differs from it in ONE thing: the body.
/// [`noop_clock`] carries the identical `SystemParam` signature — `Res<Time>` and
/// `ResMut<UiClock>` — so the scheduler derives the same access, builds the same
/// graph and runs the same three systems, and the subtraction is the four
/// statements of the tick against nothing.
///
/// # What it covers
///
/// Both nodes the census enumerates for this function: the `debug_assert!` and the
/// `&&` inside it (a DEBUG-build claim — neither exists in this release binary),
/// and the two `.min` clamps, the two `Time` reads and the two `UiClock` writes
/// that make up the body in every profile.
///
/// MEASURED 2026-08-28, release: clean code green in 100 idle runs and 40 runs
/// under 64 CPU spinners on 16 cores; `black_box(Vec::<u8>::with_capacity(64))`
/// planted at the end of the tick RED 10/10.
#[test]
fn ui_clock_tick_allocates_zero_over_a_same_shape_baseline() {
    let _arm = lock_arm();

    let base = clock_floor(build_clock_baseline);
    let pair = clock_floor(build_clock_pair);

    // Non-vacuity of the SUBTRACTION, exactly as the A1 gate argues it: while the
    // baseline allocates nothing, every delta below is trivially satisfied and a
    // schedule that never ran would pass.
    for (i, &b) in base.iter().enumerate() {
        assert!(
            b > 0,
            "clock frame {i}: the baseline schedule allocated nothing, so the subtraction has no \
             subtrahend (baseline floors {base:?})"
        );
    }
    for i in 0..CLOCK_FRAMES {
        assert!(
            pair[i] <= base[i],
            "clock frame {i}: ui_clock_tick must allocate no more than a same-shape schedule whose \
             third system is `noop_clock` (baseline floors {base:?}, pair floors {pair:?}). The \
             two arms differ in exactly one thing — the body of the tick — because `noop_clock` \
             has its `SystemParam` signature and an empty body. This gate exists because the A1 \
             pair's differential CANNOT see a cost here: the tick is in both of ITS arms and \
             cancels by construction, so `UiAnimationSet`'s third member had no allocation gate \
             anywhere until 2026-08-28"
        );
    }

    // The tick actually RAN — a floor over a schedule that did nothing would
    // satisfy every assertion above.
    let mut world = clock_world();
    let mut sched = build_clock_pair(&mut world);
    world.resource_mut::<Time>().advance_with(FRAME);
    sched.run(&mut world);
    let clock = world.resource::<UiClock>();
    assert!(
        clock.dt_real() > 0.0 && clock.dt_virtual() > 0.0,
        "the clock gate's pair arm left UiClock at ({}, {}), so the tick under measurement did not \
         write it and the floors above are about a schedule that does nothing",
        clock.dt_real(),
        clock.dt_virtual()
    );
    assert!(
        clock.dt_virtual() < clock.dt_real(),
        "the clock gate runs at relative_speed {VIRTUAL_SPEED}, so the tick must write a virtual \
         delta strictly below the real one (got {} vs {}). Equal means the two lanes are \
         indistinguishable here, and `advance`'s per-row select becomes unobservable everywhere \
         this fixture is the evidence",
        clock.dt_virtual(),
        clock.dt_real()
    );
}
