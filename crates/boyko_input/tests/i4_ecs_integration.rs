//! I4 — ECS integration gate suite (plan §7 + §14 "I4 gates").
//!
//! Covers, per the plan's I4 gate list:
//!   1. C1 resource-id distinctness (distinct per `A`; stable across calls;
//!      `InputMap<A>` likewise; no collision with a plain `#[derive(Resource)]`
//!      type's id).
//!   2. The C3 edge-correctness matrix (the load-bearing determinism gate):
//!      a single physical press is `fixed_just_pressed` for EXACTLY ONE frame
//!      across 0-substep, 1-substep, and 3-substep fixed frames — no miss, no
//!      double-count; the held `fixed_pressed` level persists; a release fires
//!      `fixed_just_released` once. Driven through the real `fixed_advance` /
//!      `FixedTime` API with a counting closure.
//!   3. `Res<ActionState<A>>` read in a system after ingest; two distinct
//!      Actionlike enums coexist in one world with no aliasing.
//!   4. Ingest correctness: `update_action_state` drains the queue → processes →
//!      freezes the snapshot once per frame; multi-event drains; held/release.
//!   5. `consume` clears frozen edges (O3); the next freeze resets it.
//!
//! Gate 6 (zero alloc on ingest) lives in its own test binary
//! (`i4_zero_alloc.rs`) because it installs a `#[global_allocator]`.

use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::register_new;
use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::core::system::Res;
use boyko_ecs::ecs::core::time::fixed_loop::fixed_advance;
use boyko_ecs::ecs::core::time::fixed_time::FixedTime;
use boyko_ecs::ecs::core::time::time::Time;
use boyko_ecs::ecs::identifiers::primitives::ResourceId;

use boyko_input::action::process::clear_consumed_fixed_edges;
use boyko_input::prelude::*;

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum GameplayAction {
    Jump,
    #[actionlike(Axis2D)]
    Move,
}

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum MenuAction {
    Confirm,
    Cancel,
}

// A third, single-variant enum so the C1 distinctness check spans three
// distinct `A` and not just a pair.
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum CameraAction {
    Look,
}

/// A plain `#[derive(Resource)]` type — its monomorphic `resource_id()` body
/// mints from the SAME global resource-id counter (`register_new`) the generic
/// `id_for` uses. Gate 1 asserts the generic mint never collides with it.
struct PlainResource {
    _v: u32,
}

// Hand-implement `Resource` exactly as the derive expands (a `static` in a
// MONOMORPHIC body is sound — the rust#22991 collapse only bites a generic
// body), so this test crate needs no proc-macro dependency for it.
impl Resource for PlainResource {
    #[inline]
    fn resource_id() -> ResourceId {
        use std::sync::OnceLock;
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId::new(register_new::<Self>()))
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────

/// Builds a world with the input resources for `GameplayAction`, a `Time` and a
/// `FixedTime` clock at engine defaults (64 Hz), and `Jump` bound to `Space`.
fn world_with_jump() -> EcsMaster {
    let mut world = EcsMaster::new();
    let map = InputMap::builder()
        .bind(GameplayAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    world.insert_resource(RawInputQueue::default());
    world.insert_resource(PhysicalInput::default());
    world.insert_resource(ActionState::<GameplayAction>::new());
    world.insert_resource(map);
    world.insert_resource(Time::default());
    world.insert_resource(FixedTime::default());
    world
}

fn press(world: &mut EcsMaster, code: KeyCode) {
    world.resource_mut::<RawInputQueue>().push_raw(RawInputEvent::Key {
        code,
        state: ButtonState::Pressed,
        repeat: false,
    });
}

fn release(world: &mut EcsMaster, code: KeyCode) {
    world.resource_mut::<RawInputQueue>().push_raw(RawInputEvent::Key {
        code,
        state: ButtonState::Released,
        repeat: false,
    });
}

/// Runs the Main-schedule input pass exactly as `InputPlugin` registers it on
/// `CoreSchedule::Main` (plan §7.3, BUG-I4-C3 fix): the sticky-edge clear runs
/// FIRST (`clear_consumed_fixed_edges`, gated on this frame's
/// `FixedTime::steps_this_frame > 0`), THEN the ingest
/// (`update_action_state.after(clear_key)`).
///
/// The clear must observe THIS frame's `steps_this_frame`, which `fixed_advance`
/// wrote when the fixed loop ran earlier in the frame. A test that drives the
/// Main pass WITHOUT a preceding fixed loop leaves `steps_this_frame` at its
/// prior value; helpers that model frame-over-frame seed it explicitly (see
/// `run_ingest_after_substeps`).
fn run_ingest(world: &mut EcsMaster) {
    world.run_system(clear_consumed_fixed_edges::<GameplayAction>);
    world.run_system(update_action_state::<GameplayAction>);
}

/// One full frame in the engine's documented order (`app.rs:664-676` +
/// `plugin.rs`): the FIXED catch-up loop runs FIRST (reading the snapshot frame
/// `F-1`'s Main froze, and writing `FixedTime::steps_this_frame`), THEN the Main
/// input pass runs the sticky-edge clear (gated on this frame's substep count)
/// and the ingest (drains + freezes the new snapshot, OR-accumulating edges).
///
/// `raw` is this frame's virtual delta; the closure observes the frozen
/// `fixed_just_pressed(Jump)` on every substep and tallies it through `tally`.
/// Returns the substep count of the fixed loop.
fn frame(world: &mut EcsMaster, raw: Duration, tally: &mut FixedTally) -> u32 {
    // Advance the variable clock (drives `fixed_advance`'s accumulator).
    world.resource_mut::<Time>().advance_with(raw);

    // ④ Fixed loop FIRST — reads the snapshot the PREVIOUS frame's Main froze,
    //    and records `steps_this_frame` (the gate the clear below reads).
    let mut edge_substeps = 0u32;
    let mut held_substeps = 0u32;
    let mut released_substeps = 0u32;
    let steps = fixed_advance(world, |w| {
        let s = w.resource::<ActionState<GameplayAction>>();
        if s.fixed_just_pressed(GameplayAction::Jump) {
            edge_substeps += 1;
        }
        if s.fixed_pressed(GameplayAction::Jump) {
            held_substeps += 1;
        }
        if s.fixed_just_released(GameplayAction::Jump) {
            released_substeps += 1;
        }
    });
    tally.record_frame(steps, edge_substeps, held_substeps, released_substeps);

    // ⑤ Main — sticky-edge clear (gated on THIS frame's substeps), then drain +
    //    process + OR-accumulate-freeze the snapshot for frame F+1.
    run_ingest(world);
    steps
}

/// Drives a real one-substep fixed loop (advancing the clock by exactly one
/// timestep so `fixed_advance` runs ≥ 1 substep and writes `steps_this_frame`),
/// then the Main input pass. Used by the single-frame Gate-4/Gate-5 tests that
/// assert frame-over-frame edge clearing under the sticky model: the sticky edge
/// only clears once a fixed batch has consumed it (`steps_this_frame > 0`), so
/// these tests must run a real consuming batch rather than relying on the ingest
/// alone to clear (the old freeze-every-frame behavior, now removed).
///
/// Returns whether the consuming substep observed `fixed_just_pressed(Jump)`.
fn run_frame_with_one_substep(world: &mut EcsMaster) -> bool {
    let ts = world.resource::<FixedTime>().timestep();
    world.resource_mut::<Time>().advance_with(ts);
    let mut saw_edge = false;
    let steps = fixed_advance(world, |w| {
        if w
            .resource::<ActionState<GameplayAction>>()
            .fixed_just_pressed(GameplayAction::Jump)
        {
            saw_edge = true;
        }
    });
    debug_assert!(steps >= 1, "a one-timestep delta must run at least one substep");
    run_ingest(world);
    saw_edge
}

/// Accumulates, across a multi-frame run, how the frozen edge was observed by
/// the fixed loop — the C3 evidence.
#[derive(Default)]
struct FixedTally {
    /// Frames in which AT LEAST ONE substep observed `fixed_just_pressed`.
    edge_active_frames: u32,
    /// Total substeps that observed `fixed_just_pressed` (across all frames).
    total_edge_substeps: u32,
    /// Total substeps that observed `fixed_pressed` (held level).
    total_held_substeps: u32,
    /// Total substeps that observed `fixed_just_released`.
    total_released_substeps: u32,
    /// Frames in which AT LEAST ONE substep observed `fixed_just_released`.
    released_active_frames: u32,
    /// Substep counts per frame, for documenting the 0/1/3 mix.
    steps_per_frame: Vec<u32>,
}

impl FixedTally {
    fn record_frame(&mut self, steps: u32, edge: u32, held: u32, released: u32) {
        self.steps_per_frame.push(steps);
        self.total_edge_substeps += edge;
        self.total_held_substeps += held;
        self.total_released_substeps += released;
        if edge > 0 {
            self.edge_active_frames += 1;
        }
        if released > 0 {
            self.released_active_frames += 1;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Gate 1 — C1 resource-id distinctness
// ──────────────────────────────────────────────────────────────────────────

/// THE C1 regression guard (plan §7.1, modeled on
/// `state_resource_ids_distinct_per_type`).
///
/// A `static ID: OnceLock<ResourceId>` inside the generic `resource_id()` body
/// would NOT be monomorphised — every `A` would collapse onto one id, aliasing
/// `ActionState<GameplayAction>` and `ActionState<MenuAction>` onto the same
/// resource slot (UB). The `TypeId`-keyed registry prevents that. This test
/// FAILS if the collapsing static is ever reintroduced.
#[test]
fn action_state_resource_ids_distinct_per_type() {
    let g = ActionState::<GameplayAction>::resource_id();
    let m = ActionState::<MenuAction>::resource_id();
    let c = ActionState::<CameraAction>::resource_id();
    assert_ne!(g, m, "ActionState<Gameplay> vs <Menu> must mint DISTINCT ids (C1)");
    assert_ne!(g, c, "ActionState<Gameplay> vs <Camera> must mint DISTINCT ids (C1)");
    assert_ne!(m, c, "ActionState<Menu> vs <Camera> must mint DISTINCT ids (C1)");
}

/// `InputMap<A>` mints distinct ids per `A`, and an `InputMap<A>` never shares a
/// slot with `ActionState<A>` (two independent resources for one enum).
#[test]
fn input_map_resource_ids_distinct_per_type_and_from_action_state() {
    assert_ne!(
        InputMap::<GameplayAction>::resource_id(),
        InputMap::<MenuAction>::resource_id(),
        "InputMap<A> vs InputMap<B> must mint DISTINCT ids"
    );
    assert_ne!(
        ActionState::<GameplayAction>::resource_id(),
        InputMap::<GameplayAction>::resource_id(),
        "ActionState<A> and InputMap<A> must occupy DISTINCT slots"
    );
}

/// `resource_id()` is stable per type across repeated calls (idempotent
/// minting); a non-cached impl would mint fresh ids and exhaust the slab.
#[test]
fn action_state_resource_id_stable_across_calls() {
    let a = ActionState::<GameplayAction>::resource_id();
    let b = ActionState::<GameplayAction>::resource_id();
    let c = ActionState::<GameplayAction>::resource_id();
    assert_eq!(a, b, "id must be stable across calls");
    assert_eq!(b, c, "id must be stable across many calls");

    let m1 = InputMap::<MenuAction>::resource_id();
    let m2 = InputMap::<MenuAction>::resource_id();
    assert_eq!(m1, m2, "InputMap id must be stable across calls too");
}

/// The generic-resource mint must not collide with a plain
/// `#[derive(Resource)]`-shaped type's id: both draw from the one global
/// `register_new` counter, so distinct types must get distinct ids regardless
/// of which minting path produced them.
#[test]
fn generic_resource_id_disjoint_from_plain_resource() {
    let plain = PlainResource::resource_id();
    let action = ActionState::<GameplayAction>::resource_id();
    let map = InputMap::<GameplayAction>::resource_id();
    assert_ne!(plain, action, "generic mint must not collide with a plain Resource id");
    assert_ne!(plain, map, "generic mint must not collide with a plain Resource id");
}

// ──────────────────────────────────────────────────────────────────────────
// Gate 3 — Res<ActionState> read in a system; two enums coexist
// ──────────────────────────────────────────────────────────────────────────

/// A real system reads `Res<ActionState<A>>` after the ingest has run — proves
/// the generic resource is insertable, reachable through the conflict-graph
/// access path, and carries the processed state.
#[test]
fn res_action_state_read_in_system() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world);

    let jumped = world.run_system(|state: Res<ActionState<GameplayAction>>| {
        state.just_pressed(GameplayAction::Jump)
    });
    assert!(jumped, "the ingest system fed a just_pressed Jump visible via Res");
}

/// `MenuAction` ingest is fully independent of `GameplayAction` ingest in one
/// world — the C1 distinct-slots property end-to-end (no cross-talk).
#[test]
fn two_action_enums_coexist_in_one_world() {
    let mut world = EcsMaster::new();
    let gameplay = InputMap::builder()
        .bind(GameplayAction::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let menu = InputMap::builder()
        .bind(MenuAction::Cancel, BindSpec::Key(KeyCode::Escape))
        .build();
    world.insert_resource(RawInputQueue::default());
    world.insert_resource(PhysicalInput::default());
    world.insert_resource(ActionState::<GameplayAction>::new());
    world.insert_resource(ActionState::<MenuAction>::new());
    world.insert_resource(gameplay);
    world.insert_resource(menu);

    // Push a Space press; run BOTH ingests over the SAME physical snapshot.
    world.resource_mut::<RawInputQueue>().push_raw(RawInputEvent::Key {
        code: KeyCode::Space,
        state: ButtonState::Pressed,
        repeat: false,
    });
    world.run_system(update_action_state::<GameplayAction>);
    // The gameplay ingest already drained the queue; the menu ingest re-runs
    // over the held physical snapshot — Escape was never pressed, so Cancel
    // stays inactive regardless.
    world.run_system(update_action_state::<MenuAction>);

    let jumped = world.run_system(|s: Res<ActionState<GameplayAction>>| {
        s.just_pressed(GameplayAction::Jump)
    });
    let cancelled = world.run_system(|s: Res<ActionState<MenuAction>>| {
        s.just_pressed(MenuAction::Cancel)
    });
    assert!(jumped, "GameplayAction::Jump fired (distinct slot)");
    assert!(!cancelled, "MenuAction::Cancel did not — its ActionState is a distinct slot");
}

// ──────────────────────────────────────────────────────────────────────────
// Gate 4 — ingest correctness (drain → process → freeze, once per frame)
// ──────────────────────────────────────────────────────────────────────────

/// Pushing a press then running Main updates `ActionState`, and the frozen
/// snapshot mirrors the Main-facing edge + level.
#[test]
fn ingest_drains_processes_and_freezes() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world);

    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(state.just_pressed(GameplayAction::Jump), "Main-facing edge set");
    assert!(state.pressed(GameplayAction::Jump), "Main-facing level held");
    assert!(state.fixed_just_pressed(GameplayAction::Jump), "frozen edge mirrors it");
    assert!(state.fixed_pressed(GameplayAction::Jump), "frozen level mirrors it");

    // The drain emptied the ring.
    assert!(world.resource::<RawInputQueue>().is_empty(), "ingest drained the queue");
}

/// After a press is frozen and a fixed batch consumes it, the NEXT frame's
/// sticky-edge clear removes the frozen rising edge while the held level
/// persists (the key is still down) — the C3 single-consume boundary under the
/// sticky model. The clearing is owned by `clear_consumed_fixed_edges` (gated on
/// the consuming batch's `steps_this_frame > 0`), NOT by the ingest's freeze.
#[test]
fn ingest_clears_edge_keeps_held_on_next_frame() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world); // frame 1: freeze the sticky edge

    // Frame 2: a real one-substep fixed loop consumes the frozen edge, then the
    // Main pass clears it (steps_this_frame > 0) and re-freezes the held level.
    let consumed = run_frame_with_one_substep(&mut world);
    assert!(consumed, "the consuming substep observed the frozen rising edge");

    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        !state.fixed_just_pressed(GameplayAction::Jump),
        "after a fixed batch consumed it, the sticky-edge clear removes the frozen rising edge"
    );
    assert!(
        state.fixed_pressed(GameplayAction::Jump),
        "the held level persists while the key stays down"
    );
}

/// The complement of the above and the heart of the BUG-I4-C3 no-miss fix: a
/// frame with NO fixed substeps (`steps_this_frame == 0`) does NOT clear the
/// frozen rising edge — it is carried forward (sticky) so a later consuming
/// batch still sees it. This is the exact behavior change that fixed F1.
#[test]
fn zero_substep_frame_does_not_clear_sticky_edge() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world); // frame 1: freeze the sticky edge

    // Frame 2: a SUB-timestep delta ⇒ 0 substeps ⇒ the Main clear is skipped.
    let ts = world.resource::<FixedTime>().timestep();
    world.resource_mut::<Time>().advance_with(ts / 4);
    let steps = fixed_advance(&mut world, |_| {});
    assert_eq!(steps, 0, "a quarter-timestep delta runs 0 substeps");
    run_ingest(&mut world); // clear gated OFF (steps_this_frame == 0)

    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        state.fixed_just_pressed(GameplayAction::Jump),
        "a 0-substep frame must NOT clear the sticky edge (no-miss carry-forward, BUG-I4-C3)"
    );
}

/// A release after a held frame fires `just_released` / `fixed_just_released`
/// exactly once and drops the held level.
#[test]
fn ingest_release_fires_falling_edge_once() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world); // frame 1: pressed
    release(&mut world, KeyCode::Space);
    run_ingest(&mut world); // frame 2: released

    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(state.just_released(GameplayAction::Jump), "Main-facing falling edge");
    assert!(state.fixed_just_released(GameplayAction::Jump), "frozen falling edge");
    assert!(!state.pressed(GameplayAction::Jump), "level dropped on release");
    assert!(!state.fixed_pressed(GameplayAction::Jump), "frozen level dropped");

    // Frame 3: a real fixed batch consumes the falling edge, then the Main clear
    // (steps_this_frame > 0) removes it — the sticky falling edge is live for
    // exactly the one batch that consumes it.
    run_frame_with_one_substep(&mut world);
    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        !state.fixed_just_released(GameplayAction::Jump),
        "the falling edge clears once a fixed batch has consumed it"
    );
}

/// Multiple raw events in one frame all drain before the snapshot freezes: a
/// same-frame down+up tap survives as BOTH a rising and falling edge (W4),
/// frozen for the fixed loop.
#[test]
fn ingest_multi_event_same_frame_tap_freezes_both_edges() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    release(&mut world, KeyCode::Space);
    run_ingest(&mut world);

    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        state.fixed_just_pressed(GameplayAction::Jump),
        "same-frame tap freezes a rising edge (W4)"
    );
    assert!(
        state.fixed_just_released(GameplayAction::Jump),
        "same-frame tap freezes a falling edge (W4)"
    );
    assert!(
        !state.fixed_pressed(GameplayAction::Jump),
        "end-of-frame level is not held after a same-frame tap"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Gate 5 — consume clears frozen edges (O3)
// ──────────────────────────────────────────────────────────────────────────

/// A `consume` from a (simulated) fixed system clears the frozen edge (in the
/// sticky set, so it never re-accumulates) and masks the Main-facing edge; the
/// next frame's freeze re-samples the held level. Because `consume` clears the
/// sticky edge directly, no consuming fixed batch is needed to drop it here.
#[test]
fn consume_masks_frozen_edge_then_next_freeze_resets() {
    let mut world = world_with_jump();
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world);

    // A fixed system consumes Jump mid-frame.
    {
        let state = world.resource_mut::<ActionState<GameplayAction>>();
        assert!(state.fixed_just_pressed(GameplayAction::Jump), "edge live before consume");
        state.consume(GameplayAction::Jump);
        assert!(
            !state.fixed_just_pressed(GameplayAction::Jump),
            "consume clears the frozen rising edge for the rest of the frame"
        );
        assert!(
            !state.just_pressed(GameplayAction::Jump),
            "consume also masks the Main-facing edge"
        );
    }

    // Next frame: the key is STILL down (no release), so a fresh press edge is
    // NOT re-derived. The ingest re-samples the held level into the snapshot and
    // the stale `consumed` mask does not leak; the sticky edge stays clear
    // because `consume` already removed it (nothing OR-accumulates it back).
    run_ingest(&mut world);
    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        state.fixed_pressed(GameplayAction::Jump),
        "next freeze restores the held level (consume did not leak)"
    );
    assert!(
        state.pressed(GameplayAction::Jump),
        "the Main-facing level is no longer masked by the previous frame's consume"
    );
}

/// A fresh press on the frame AFTER a consume produces a fresh, un-consumed
/// frozen edge — the consume is strictly per-frame.
#[test]
fn consume_does_not_leak_into_next_press() {
    let mut world = world_with_jump();
    // Frame 1: press + consume.
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world);
    world
        .resource_mut::<ActionState<GameplayAction>>()
        .consume(GameplayAction::Jump);

    // Frame 2: release (clears the level).
    release(&mut world, KeyCode::Space);
    run_ingest(&mut world);

    // Frame 3: a brand-new press — must be a fresh, un-masked edge.
    press(&mut world, KeyCode::Space);
    run_ingest(&mut world);
    let state = world.resource::<ActionState<GameplayAction>>();
    assert!(
        state.fixed_just_pressed(GameplayAction::Jump),
        "a new press after an earlier consume is a fresh frozen edge"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Gate 2 — the C3 edge-correctness matrix (THE crux)
// ──────────────────────────────────────────────────────────────────────────
//
// Frame structure replicates the engine (`app.rs:664-676`): per frame the
// FIXED loop runs first (consuming the snapshot frame F-1's Main froze), then
// Main ingests + freezes the snapshot for frame F+1. A single physical press
// must be observed by the fixed loop as `fixed_just_pressed` on every substep
// of EXACTLY ONE frame — never missed (even when that frame ran 0 substeps and
// a LATER frame first sees it), never double-counted (a 3-substep frame counts
// the press as one edge-active frame, not three).

/// Substep-count sanity: the chosen virtual deltas actually produce 0-, 1-, and
/// 3-substep fixed frames at 64 Hz (timestep = 15.625 ms). This anchors the
/// matrix below — if these deltas ever stop producing the intended mix, the
/// matrix's "across 0/1/3 substeps" claim is void.
#[test]
fn c3_substep_counts_are_0_1_and_3() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();

    // 0 substeps: a sub-timestep delta.
    let zero = sole_fixed_steps(&mut world, ts / 4);
    assert_eq!(zero, 0, "a quarter-timestep delta runs 0 substeps");

    // 1 substep: just over one timestep (the leftover 3/4 carries over).
    let one = sole_fixed_steps(&mut world, ts);
    assert_eq!(one, 1, "a one-timestep delta runs 1 substep");

    // 3 substeps: three timesteps worth (minus the carried remainder lands ≥3).
    let three = sole_fixed_steps(&mut world, ts * 3);
    assert!(three >= 3, "three timesteps run at least 3 substeps (got {three})");
}

/// Runs ONLY the fixed loop for one frame (no ingest) and returns the substep
/// count — a probe for the substep-mix sanity test.
fn sole_fixed_steps(world: &mut EcsMaster, raw: Duration) -> u32 {
    world.resource_mut::<Time>().advance_with(raw);
    fixed_advance(world, |_| {})
}

/// C3 — 1-substep frames: a single press is edge-active for EXACTLY ONE frame
/// and observed by EXACTLY ONE substep (1 substep/frame). No miss, no
/// double-count. Establishes the baseline of the matrix.
#[test]
fn c3_single_press_one_edge_frame_at_1_substep() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    // Frame 1: press. Main freezes the edge AT END of frame 1; the fixed loop
    // of frame 1 ran BEFORE the press (sees nothing).
    press(&mut world, KeyCode::Space);
    frame(&mut world, ts, &mut tally);
    // Frames 2..=5: key held, no new events. The frame-2 fixed loop consumes
    // the snapshot frozen by frame 1 (the 1-frame latency).
    for _ in 0..4 {
        frame(&mut world, ts, &mut tally);
    }

    assert!(
        tally.steps_per_frame.iter().all(|&s| s == 1),
        "every frame ran exactly 1 substep (got {:?})",
        tally.steps_per_frame
    );
    assert_eq!(
        tally.edge_active_frames, 1,
        "the press is fixed_just_pressed in EXACTLY ONE frame (no miss, no double-count)"
    );
    assert_eq!(
        tally.total_edge_substeps, 1,
        "with 1 substep/frame the edge is seen by exactly one substep"
    );
    // The held level persists: the press frame's snapshot held the level, and
    // every later frame re-freezes it while the key is down.
    assert!(
        tally.total_held_substeps >= 4,
        "fixed_pressed persists across the held frames (got {})",
        tally.total_held_substeps
    );
}

/// C3 DIAGNOSTIC — prints the per-frame trace of (substeps, edge-observed,
/// snapshot-edge-after-Main) so the 0-substep behavior is unambiguous in the
/// report. Always passes; it is evidence, not a gate.
#[test]
fn c3_diagnostic_trace_zero_substep_straddle() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();

    let deltas = [ts / 4, ts / 4, ts, ts, ts];
    let mut pressed = false;
    eprintln!("frame | delta_frac | substeps | edge_seen_by_fixed | snapshot_edge_after_main");
    for (i, d) in deltas.iter().enumerate() {
        if i == 0 {
            press(&mut world, KeyCode::Space);
            pressed = true;
        }
        world.resource_mut::<Time>().advance_with(*d);
        let mut edge_seen = 0u32;
        let steps = fixed_advance(&mut world, |w| {
            if w
                .resource::<ActionState<GameplayAction>>()
                .fixed_just_pressed(GameplayAction::Jump)
            {
                edge_seen += 1;
            }
        });
        run_ingest(&mut world);
        let snap_edge = world
            .resource::<ActionState<GameplayAction>>()
            .fixed_just_pressed(GameplayAction::Jump);
        eprintln!(
            "  {i}   |   {:.2}    |    {steps}     |        {edge_seen}          |        {snap_edge}",
            d.as_secs_f64() / ts.as_secs_f64()
        );
        let _ = pressed;
    }
}

/// C3 (no-double-count) — a 0-substep frame never DOUBLE-counts a press. The
/// press straddles two 0-substep frames; the fixed loop observes the edge AT
/// MOST once (never inflated by the straddle). Under the sticky-accumulate model
/// the COMPLEMENTARY no-MISS guarantee also holds — proven separately by
/// `c3_press_survives_zero_substep_frame` (the flipped former F1).
#[test]
fn c3_zero_substep_straddle_never_double_counts() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    press(&mut world, KeyCode::Space);
    let s1 = frame(&mut world, ts / 4, &mut tally);
    assert_eq!(s1, 0, "frame 1 ran 0 substeps");
    let s2 = frame(&mut world, ts / 4, &mut tally);
    assert_eq!(s2, 0, "frame 2 also ran 0 substeps");
    for _ in 0..3 {
        frame(&mut world, ts, &mut tally);
    }

    // The edge is observed AT MOST once and the press is edge-active in AT MOST
    // one frame — the no-double-count guarantee, which holds regardless of the
    // straddle.
    assert!(
        tally.total_edge_substeps <= 1,
        "a 0-substep straddle never DOUBLE-counts the press (got {} edge substeps)",
        tally.total_edge_substeps
    );
    assert!(
        tally.edge_active_frames <= 1,
        "the press is edge-active in AT MOST one frame (got {})",
        tally.edge_active_frames
    );
}

/// C3 NO-MISS across a 0-substep frame (the BUG-I4-C3 fix's target; formerly the
/// `#[ignore]`'d known-failure `c3_known_failure_press_lost_across_zero_substep_frame`,
/// flipped to a passing gate after the sticky-accumulate fix) — a single press
/// whose press-frame AND the frame after it both run 0 substeps must still be
/// delivered to the FIRST later fixed substep that runs: it is NOT lost.
///
/// Under the old freeze-every-frame model the 0-substep frame's Main
/// OVERWROTE the frozen `fixed_*` mirror before any substep saw it, so the press
/// vanished (`edge_active_frames == 0`). The fix makes `freeze_fixed_snapshot`
/// OR-accumulate the edges (sticky) and moves edge-clearing into
/// `clear_consumed_fixed_edges`, gated on `steps_this_frame > 0`: a 0-substep
/// frame skips the clear, so the edge is carried forward and reaches the first
/// batch that runs. Observed now: `edge_active_frames == 1`. Diagnostic evidence:
/// `c3_diagnostic_trace_zero_substep_straddle` (`--nocapture`).
#[test]
fn c3_press_survives_zero_substep_frame() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    // Press frame, then a 0-substep frame, then frames that DO run substeps.
    press(&mut world, KeyCode::Space);
    let s1 = frame(&mut world, ts / 4, &mut tally); // press-frame: 0 substeps
    assert_eq!(s1, 0, "the press frame ran 0 substeps");
    let s2 = frame(&mut world, ts / 4, &mut tally); // 0 substeps — edge carried (sticky)
    assert_eq!(s2, 0, "the frame after the press also ran 0 substeps");
    for _ in 0..3 {
        frame(&mut world, ts, &mut tally); // now run substeps — the sticky edge is seen
    }

    // The miss-free guarantee: exactly one edge-active frame even though the
    // press straddled two 0-substep frames (was 0 under the old model).
    assert_eq!(
        tally.edge_active_frames, 1,
        "MISS-FREE: a single press is fixed_just_pressed for exactly one frame even \
         when 0-substep frames straddle it (BUG-I4-C3, plan §7.3 0-substep bullet)"
    );
    // The first substep that runs after the straddle is the SOLE one to observe
    // it — carried forward, then consumed exactly once.
    assert_eq!(
        tally.total_edge_substeps, 1,
        "the carried-forward edge is consumed by exactly one substep (no miss, no double-count)"
    );
}

/// C3 — 3-substep frame: a single press observed by a frame whose fixed loop
/// runs 3 substeps is counted as EXACTLY ONE edge-active frame — every substep
/// sees the same frozen `fixed_just_pressed` (3 substep-observations of the SAME
/// frame), but it is one physical press, edge-active for one frame, NOT three
/// separate presses. The contract: read `fixed_just_pressed` and act
/// idempotently per frame.
#[test]
fn c3_press_not_double_counted_across_3_substep_frame() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    // Frame 1: press. Variable delta = 1 timestep ⇒ frame-1 fixed loop runs 1
    // substep BEFORE the press is frozen (sees nothing).
    press(&mut world, KeyCode::Space);
    frame(&mut world, ts, &mut tally);

    // Frame 2: a 3-timestep delta ⇒ the fixed loop runs 3 substeps, each
    // reading the SAME snapshot frame 1 froze (the rising edge). The key is
    // held; no new event ⇒ frame 2's Main re-derives no further rising edge.
    let s2 = frame(&mut world, ts * 3, &mut tally);
    assert!(s2 >= 3, "frame 2 ran ≥3 substeps (got {s2})");

    // Frames 3..=4: held, 1 substep each — no further edge.
    for _ in 0..2 {
        frame(&mut world, ts, &mut tally);
    }

    assert_eq!(
        tally.edge_active_frames, 1,
        "the press is edge-active for EXACTLY ONE frame even though that frame ran 3 substeps"
    );
    // The 3 substeps of frame 2 each observed the SAME frozen edge — so the
    // total substep observations equal that frame's substep count, NOT a count
    // that grows with extra presses. The crux: no SECOND edge-active frame.
    assert_eq!(
        tally.total_edge_substeps, s2,
        "all {s2} substeps of the one edge-active frame saw the same frozen edge — \
         one press, one edge-active frame, not {s2} presses"
    );
    // A system reading `fixed_just_pressed` and acting once-per-frame fires for
    // exactly one frame; a system acting per-substep-while-held would use
    // `fixed_pressed` instead (which is true on every held substep).
    assert!(
        tally.total_held_substeps >= s2,
        "fixed_pressed is true on every substep of the held frame (per-substep contract)"
    );
}

/// C3 — release across a multi-substep frame fires `fixed_just_released` for
/// exactly one edge-active frame, no matter the substep count.
#[test]
fn c3_release_one_edge_frame_across_substep_mix() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    // Frame 1: press (held).
    press(&mut world, KeyCode::Space);
    frame(&mut world, ts, &mut tally);
    // Frame 2: still held.
    frame(&mut world, ts, &mut tally);
    // Frame 3: RELEASE, with a 3-substep delta — the falling edge is frozen and
    // every substep of frame 4 reads it.
    release(&mut world, KeyCode::Space);
    frame(&mut world, ts, &mut tally);
    // Frames 4..=5: a 3-substep frame then a 1-substep frame consume the
    // released snapshot.
    frame(&mut world, ts * 3, &mut tally);
    frame(&mut world, ts, &mut tally);

    assert_eq!(
        tally.released_active_frames, 1,
        "the release is fixed_just_released for EXACTLY ONE frame (no miss, no double-count)"
    );
    assert!(
        tally.total_released_substeps >= 1,
        "at least one substep observed the frozen falling edge (got {})",
        tally.total_released_substeps
    );
}

/// C3 — the FULL 0/1/3 mix in one run: press once; the frame IMMEDIATELY after
/// the press runs ≥1 substep (so the edge IS observed — the achievable case),
/// and the remaining HELD frames mix 0-, 1-, and 3-substep counts. The single
/// press is edge-active for EXACTLY ONE frame across the entire heterogeneous
/// mix (no double-count by the 3-substep frame; the held 0-substep frames add
/// nothing).
#[test]
fn c3_single_press_one_edge_frame_across_full_0_1_3_mix() {
    let mut world = world_with_jump();
    let ts = world.resource::<FixedTime>().timestep();
    let mut tally = FixedTally::default();

    // Frame 1: press; 1-substep frame. (Its fixed loop runs over the prior empty
    // snapshot; the edge is frozen at end of frame 1.)
    press(&mut world, KeyCode::Space);
    frame(&mut world, ts, &mut tally);
    // Frame 2: 1 substep — observes the frozen edge exactly once (the achievable
    // no-miss case: the frame after the press runs ≥1 substep).
    frame(&mut world, ts, &mut tally);
    // Frames 3..: a heterogeneous sequence of HELD frames mixing 0/3/0/1/3
    // substeps — none of which must re-activate the (now consumed) edge.
    frame(&mut world, ts / 4, &mut tally); // ~0
    frame(&mut world, ts * 3, &mut tally); // ~3 (+ carried remainder)
    frame(&mut world, ts / 4, &mut tally); // ~0
    frame(&mut world, ts, &mut tally); // ~1
    frame(&mut world, ts * 3, &mut tally); // ~3

    // Establish the mix actually contained 0-, 1-, and 3-substep frames.
    let has_zero = tally.steps_per_frame.contains(&0);
    let has_one = tally.steps_per_frame.contains(&1);
    let has_three = tally.steps_per_frame.iter().any(|&s| s >= 3);
    assert!(
        has_zero && has_one && has_three,
        "the run must mix 0-, 1-, and 3-substep frames (got {:?})",
        tally.steps_per_frame
    );

    assert_eq!(
        tally.edge_active_frames, 1,
        "ONE physical press ⇒ EXACTLY ONE edge-active frame across the full 0/1/3 mix \
         (no double-count on the 3-substep frame; the held 0-substep frames add nothing)"
    );
    assert_eq!(
        tally.total_edge_substeps, 1,
        "the edge was observed by exactly one substep (the single substep of frame 2)"
    );
}
