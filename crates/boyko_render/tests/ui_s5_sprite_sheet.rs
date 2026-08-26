//! UI-ADVANCED rung S5 — sprite sheets, the flipbook and the tiled lane, the
//! DEVICE-FREE half (`docs/UI-PLAN-SPRITES.md` gates G5-1, G5-2, G5-3, G5-4,
//! G5-6, G5-11).
//!
//! Every test drives production code: the frame arithmetic through
//! `UiSheet::frame_uv` and through **`UiUploadSystem::gather_into_staging`** (the
//! loop the scheduler runs), the flipbook through a real `Schedule`, and the tile
//! derivation through `pack_ui_nine_slice_instance`'s own `flags` word. No device,
//! no graphics type, no hand-packed `UiInstance`.
//!
//! # Two invocations, on purpose
//!
//! ```text
//! cargo test -p boyko-render --test ui_s5_sprite_sheet             # running 10 tests
//! cargo test -p boyko-render --test ui_s5_sprite_sheet --release   # running 10 tests
//! ```
//!
//! Unlike `ui_s4_nine_slice`, the two SETS here are the same ten tests: no S5
//! sentence is profile-gated, because none of them is about a `debug_assert!`.
//! Both profiles are still run, because the rung packs into a `flags` word through
//! shifts and masks and a release build is where an overflow would be silent.
//!
//! # The ORDER this file pins, and why it is a gate rather than a convention
//!
//! `ui_sprite_flipbook` is registered `.before(ui_render_discovery)` everywhere in
//! this file. `Changed<C>` compares a row's changed tick against the READING
//! system's `last_run`, so a flipbook write that lands after discovery in frame N
//! is only seen in frame N+1 — a repaint one frame late, never a lost one, and
//! invisible to every assertion that samples a settled state. G5-3's first leg is
//! what makes it visible: it asserts the index moved and the generation bumped IN
//! THE SAME FRAME.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles out of the `Send + Sync` one-shot system closure.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::filter::Changed;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_ecs::ecs::core::time::time::Time;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_render::{
    pack_ui_nine_slice_instance, ui_nine_slice_tiles, ui_nine_slice_tiles_axis,
    ui_render_discovery, PackInput, UiImageInput, UiInstance, UiNineSliceInput,
    UiRenderGeneration, UiUploadSystem, FLAG_TEXTURED, FLAG_TILED, UI_NINE_SLICE_MODE_TILE,
    UI_SLOT_MASK, UI_SLOT_SHIFT, UI_TILE_MASK, UI_TILE_MAX, UI_TILE_X_SHIFT, UI_TILE_Y_SHIFT,
};
use boyko_ui::components::{
    ComputedRect, NineSliceMode, SpriteAnimMode, StackIndex, UiBackground, UiImage, UiRoot,
    UiSpriteAnim, UiSpriteCursor, UiSpriteSheet,
};
use boyko_ui::sprite::{ui_sprite_flipbook, SheetId, UiSheet, UiSheetTable};

// ───────────────────────── the authored sheet ──────────────────────────────

/// The half-texel inset of a 16×16 atlas — `0.5 / 16`, exact in binary FP.
const HALF_TEXEL_16: f32 = 1.0 / 32.0;

/// S-D18 (4)'s flipbook source as a SHEET: 4×4 frames of 4×4 texels on a 16×16
/// atlas, so the half-texel inset is `1/32` against a frame extent of `1/4` and
/// the insetted extent is `0.1875` — NOT zero, which is what a 4×4-TEXEL reading
/// of "a 4×4 grid" would have produced (`u0 == u1`, every arm a no-op).
fn sheet_4x4() -> UiSheet {
    UiSheet {
        slot: 9,
        cols: 4,
        rows: 4,
        frame_count: 16,
        _pad: [0; 2],
        inset_uv: [HALF_TEXEL_16, HALF_TEXEL_16],
    }
}

/// The NON-SQUARE case (S-D18 (5)): `cols != rows`, so the row-major decode
/// `col = index % cols; row = index / cols` is no longer bit-identical to the
/// same expression with the two interchanged.
fn sheet_4x2() -> UiSheet {
    UiSheet {
        slot: 9,
        cols: 4,
        rows: 2,
        frame_count: 8,
        _pad: [0; 2],
        inset_uv: [HALF_TEXEL_16, HALF_TEXEL_16],
    }
}

/// G5-1's pinned frame: index 6 on both sheets. Chosen rather than 0 or 1 because
/// the transposed decode AGREES with the correct one at both of those (`0 → (0,0)`
/// and `1 → (1,0)` under either `cols`), and they first diverge at 2 — so a red
/// aimed at the transpose has to pin an index at least that far in. 6 is the
/// smallest index that also puts the frame off both the first row and the first
/// column of BOTH sheets.
const FRAME: u16 = 6;

/// The hand-computed UV of frame 6 on [`sheet_4x4`] — `col = 6 % 4 = 2`,
/// `row = 6 / 4 = 1`:
///
/// ```text
/// u0 = 2/4 + 1/32 = 0.53125     u1 = 3/4 - 1/32 = 0.71875
/// v0 = 1/4 + 1/32 = 0.28125     v1 = 2/4 - 1/32 = 0.46875
/// ```
///
/// Every term is a negative power of two, so the sum is EXACT in binary FP and
/// this is an equality rather than a tolerance. Asserted against the constant,
/// **not** against the implementation.
const FRAME_UV_4X4: [f32; 4] = [0.53125, 0.28125, 0.71875, 0.46875];

/// The same frame on [`sheet_4x2`] — `col = 6 % 4 = 2`, `row = 6 / 4 = 1` of TWO
/// rows:
///
/// ```text
/// u0 = 2/4 + 1/32 = 0.53125     u1 = 3/4 - 1/32 = 0.71875
/// v0 = 1/2 + 1/32 = 0.53125     v1 = 2/2 - 1/32 = 0.96875
/// ```
///
/// Under a transposed decode it would be `col = 6 % 2 = 0`, `row = 6 / 2 = 3`,
/// i.e. `(0.03125, 0.78125, 0.46875, 0.96875)` — three of the four numbers move.
const FRAME_UV_4X2: [f32; 4] = [0.53125, 0.53125, 0.71875, 0.96875];

/// The node every sheet test spawns, logical px == physical px at scale 1.0.
const NODE: [f32; 4] = [16.0, 16.0, 96.0, 96.0];

/// An opaque WHITE tint. `UiImage`'s authored default is alpha 0, and the pack
/// premultiplies the tint into every record — so a defaulted tint would make every
/// assertion below read a zero colour and disarm the reds that move pixels.
const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

// ───────────────────────── shared plumbing ─────────────────────────────────

/// A schedule with the flipbook ORDERED AHEAD of discovery (module doc).
fn flipbook_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let discovery = b.add_system(ui_render_discovery).key();
    b.add_system(ui_sprite_flipbook).before(discovery);
    b.build(world)
}

fn discovery_only_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    b.build(world)
}

/// Advances the clock by one frame's worth of wall time and runs the schedule.
/// `Time::advance_with` is `pub` and its own `debug_assert!` forbids only calling
/// it INSIDE a system body — a driver-shaped harness may, which is what this is.
fn tick(world: &mut EcsMaster, schedule: &mut Schedule, dt: Duration) {
    world.resource_mut::<Time>().advance_with(dt);
    schedule.run(world);
}

/// Runs discovery until the generation holds for two consecutive frames, then
/// dispatches Phase 1 once so the settled generation is PACKED.
fn settle(world: &mut EcsMaster, schedule: &mut Schedule, sys: &mut UiUploadSystem) {
    let mut settled = 0;
    for _ in 0..8 {
        let before = world.resource::<UiRenderGeneration>().generation;
        schedule.run(world);
        if world.resource::<UiRenderGeneration>().generation == before {
            settled += 1;
            if settled == 2 {
                break;
            }
        } else {
            settled = 0;
        }
    }
    assert_eq!(settled, 2, "discovery must go quiet after the spawn settles");
    world.run_system_once(sys);
}

/// Spawns one node carrying `UiImage` + `UiSpriteSheet` against a world holding
/// `sheet`, stages it through the scheduler's own pack, and returns the system.
fn stage_sheet_node(sheet: UiSheet, index: u16, image_uv: [[f32; 2]; 2]) -> UiUploadSystem {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    let mut table = UiSheetTable::new();
    let id = table.register(sheet);
    assert_eq!(id, SheetId(0), "the first registered sheet is id 0");
    world.insert_resource(table);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect {
            x: NODE[0],
            y: NODE[1],
            w: NODE[2],
            h: NODE[3],
        });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiImage {
            // Deliberately NOT the sheet's slot and NOT the frame's UV: the whole
            // claim is that the sheet OVERRIDES both, so a pack that read these
            // through cannot pass.
            texture: 3,
            uv_min: image_uv[0],
            uv_max: image_uv[1],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 0, index });
        e.insert(UiRoot);
    });

    let mut schedule = discovery_only_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);
    sys
}

/// The one TEXTURED record a sheet node stages (its background is sub 0).
fn sprite_record(sys: &UiUploadSystem) -> UiInstance {
    let staged = sys.staged();
    assert_eq!(
        staged.len(),
        2,
        "a node with UiImage and no UiNineSlice stages its background plus ONE image \
         record — `UiSpriteSheet` changes what that record SAMPLES, never how many \
         records exist (ui_node_sub_codes stays the sole authority)"
    );
    let textured: Vec<UiInstance> = staged
        .iter()
        .copied()
        .filter(|r| r.flags & FLAG_TEXTURED != 0)
        .collect();
    assert_eq!(textured.len(), 1, "exactly one textured record");
    textured[0]
}

// ───────────────────────── G5-1 ────────────────────────────────────────────

/// **G5-1** — the frame UV is the stated arithmetic, on a SQUARE and a NON-SQUARE
/// grid, and the gather substitutes it into the node's image inputs.
///
/// Four legs, and each is a different claim:
///
/// 1. the pure arithmetic against a hand-computed constant (square);
/// 2. the same against a NON-SQUARE grid, where the row-major decode stops being
///    invariant under transposing `cols` and `rows` (M5-f);
/// 3. the substitution — the staged record carries the FRAME's uv and the SHEET's
///    slot, not the `UiImage`'s;
/// 4. the tint is NOT substituted: the sheet decides what is sampled, the image
///    still decides how it is modulated.
#[test]
fn g5_1_frame_uv_is_the_stated_arithmetic_and_the_gather_substitutes_it() {
    // (1) square.
    assert_eq!(
        sheet_4x4().frame_uv(FRAME),
        FRAME_UV_4X4,
        "frame {FRAME} of a 4x4 sheet inset by a half texel is the hand-computed constant"
    );
    // (2) non-square — the leg M5-f reds and the square leg cannot.
    assert_eq!(
        sheet_4x2().frame_uv(FRAME),
        FRAME_UV_4X2,
        "frame {FRAME} of a 4x2 sheet: `col = index % cols` and `row = index / cols`, NOT \
         the same expression with cols and rows interchanged"
    );

    // (3) + (4) the substitution, through the scheduler's own pack.
    let sys = stage_sheet_node(sheet_4x4(), FRAME, [[0.1, 0.2], [0.3, 0.4]]);
    let rec = sprite_record(&sys);
    assert_eq!(
        rec.uv, FRAME_UV_4X4,
        "the staged sprite samples the FRAME's sub-rect, not the UiImage's own uv_min/uv_max"
    );
    assert_eq!(
        (rec.flags >> UI_SLOT_SHIFT) & UI_SLOT_MASK,
        sheet_4x4().slot,
        "the staged sprite samples the SHEET's bindless slot, not `UiImage::texture`"
    );
    assert_eq!(
        rec.color,
        boyko_render::premultiply_rgba8(TINT_OPAQUE_WHITE),
        "the TINT still comes from `UiImage` — the sheet substitutes what is sampled, not \
         how it is modulated"
    );
    assert_eq!(
        sys.sheet_index_clamps(),
        0,
        "an in-range index clamps nothing"
    );
}

/// **G5-1**, the inert paths: a sheet that cannot resolve leaves `UiImage`
/// UNTOUCHED, and does not change how many records the node emits.
///
/// Three ways to be inert, all of them ordinary rather than errors, and the third
/// is the one the plan's G5-6 row described as "emits no sprite record" — which it
/// cannot do: `ui_node_sub_codes` is the sole authority on a node's records (gate
/// G4-8) and the gather is not allowed a second opinion.
#[test]
fn g5_1_an_unresolvable_sheet_leaves_the_image_untouched() {
    let image_uv = [[0.1, 0.2], [0.3, 0.4]];
    let expect_image_uv = [0.1, 0.2, 0.3, 0.4];

    // (a) a registered sheet with ZERO frames.
    let empty = UiSheet {
        frame_count: 0,
        ..sheet_4x4()
    };
    let rec = sprite_record(&stage_sheet_node(empty, 0, image_uv));
    assert_eq!(rec.uv, expect_image_uv, "a zero-frame sheet is INERT");
    assert_eq!(
        (rec.flags >> UI_SLOT_SHIFT) & UI_SLOT_MASK,
        3,
        "…and the node keeps its own `UiImage::texture`"
    );

    // (b) a `sheet` id nothing registered (the table holds exactly id 0).
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    let mut table = UiSheetTable::new();
    table.register(sheet_4x4());
    world.insert_resource(table);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect { x: NODE[0], y: NODE[1], w: NODE[2], h: NODE[3] });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiImage {
            texture: 3,
            uv_min: image_uv[0],
            uv_max: image_uv[1],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 41, index: 0 });
        e.insert(UiRoot);
    });
    let mut schedule = discovery_only_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);
    assert_eq!(
        sprite_record(&sys).uv,
        expect_image_uv,
        "an unregistered sheet id is INERT"
    );

    // (c) NO `UiSheetTable` resource at all — the eight in-tree UI harnesses that
    //     build worlds by hand and insert only what they need. The panicking
    //     resource verb would take every one of them down; the gather uses the
    //     other one.
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect { x: NODE[0], y: NODE[1], w: NODE[2], h: NODE[3] });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiImage {
            texture: 3,
            uv_min: image_uv[0],
            uv_max: image_uv[1],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 0, index: 2 });
        e.insert(UiRoot);
    });
    let mut schedule = discovery_only_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);
    assert_eq!(
        sprite_record(&sys).uv,
        expect_image_uv,
        "no UiSheetTable in the world is INERT, not a panic"
    );
}

// ───────────────────────── G5-2 ────────────────────────────────────────────

/// One flipbook world: a node carrying the three S5 components, at `mode`, with
/// `repeats`, over frames `first..=last`.
fn flipbook_world(
    first: u16,
    last: u16,
    mode: SpriteAnimMode,
    repeats: u8,
    fps: f32,
) -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Time::default());
    let probe: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&probe);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect { x: NODE[0], y: NODE[1], w: NODE[2], h: NODE[3] });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiImage {
            texture: 3,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 0, index: first });
        e.insert(UiSpriteAnim {
            first,
            last,
            fps,
            mode,
            repeats,
            _pad: [0; 2],
        });
        // The cursor is spelled EXPLICITLY here, and G5-12 is why: on this kernel
        // `#[require(UiSpriteCursor)]` cannot make it structural, because the
        // require pass resolves the required id's pool in the target ARCHETYPE and
        // a dense id has none. `AnimatedSpriteBundle` is the authoring remedy;
        // these hand-built scenes insert the components one at a time on purpose,
        // so they exercise the same path a `ui!` tree does.
        e.insert(CursorBundle {
            c: UiSpriteCursor::default(),
        });
        e.insert(UiRoot);
        *sink.lock().expect("probe") = Some(e.id());
    });
    let node = probe.lock().expect("probe").expect("spawned node");
    (world, node)
}

fn index_of(world: &EcsMaster, node: Entity) -> u16 {
    world
        .get_component::<UiSpriteSheet>(node)
        .expect("the node carries UiSpriteSheet")
        .index
}

/// Ticks `n` frames at exactly one frame-duration each and collects the index
/// AFTER each tick.
fn walk(mode: SpriteAnimMode, repeats: u8, first: u16, last: u16, n: usize) -> Vec<u16> {
    const FPS: f32 = 10.0;
    let (mut world, node) = flipbook_world(first, last, mode, repeats, FPS);
    let mut schedule = flipbook_schedule(&mut world);
    let step = Duration::from_nanos((1_000_000_000.0 / FPS) as u64);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        tick(&mut world, &mut schedule, step);
        out.push(index_of(&world, node));
    }
    out
}

/// **G5-2** — the four modes are exactly right at the TURNS.
///
/// The sequences are pinned as literal arrays rather than recomputed, because the
/// defect this gates is an off-by-one AT THE TURN (M5-d) and a recomputation would
/// share the implementation's own off-by-one. Three cycles per mode, so a turn is
/// crossed more than once.
///
#[test]
fn g5_2_the_four_modes_are_right_at_the_turns() {
    // Forward over 0..=3, three full cycles: 1,2,3,0 · 1,2,3,0 · 1,2,3,0
    assert_eq!(
        walk(SpriteAnimMode::Forward, 0, 0, 3, 12),
        vec![1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0],
        "Forward wraps from `last` to `first`"
    );
    // Reverse over 0..=3 starting AT 0: the first step wraps to `last`.
    assert_eq!(
        walk(SpriteAnimMode::Reverse, 0, 0, 3, 12),
        vec![3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0],
        "Reverse wraps from `first` to `last`"
    );
    // PingPong over 0..=3: 1,2,3 then turn -> 2,1,0 then turn -> 1,2,3 …
    // Each endpoint appears ONCE per round trip. M5-d (flip after the step
    // instead of before) yields 1,2,3,3,2,1,0,0,… and reds here.
    assert_eq!(
        walk(SpriteAnimMode::PingPong, 0, 0, 3, 12),
        vec![1, 2, 3, 2, 1, 0, 1, 2, 3, 2, 1, 0],
        "PingPong turns WITHOUT repeating the endpoint"
    );
    // Once over 0..=3: one pass, then HOLD `last` forever.
    assert_eq!(
        walk(SpriteAnimMode::Once, 0, 0, 3, 8),
        vec![1, 2, 3, 3, 3, 3, 3, 3],
        "Once holds `last`, not `first` — a mode that wrapped and then froze would \
         hold the wrong end of the range and be useless"
    );
    // `Once` IS `Forward` with `repeats == 1`, and the component doc says so.
    assert_eq!(
        walk(SpriteAnimMode::Forward, 1, 0, 3, 8),
        walk(SpriteAnimMode::Once, 0, 0, 3, 8),
        "Once is exactly Forward with repeats == 1"
    );
    // A budget of two cycles: two passes, then hold.
    assert_eq!(
        walk(SpriteAnimMode::Forward, 2, 0, 3, 10),
        vec![1, 2, 3, 0, 1, 2, 3, 3, 3, 3],
        "repeats == 2 runs two cycles and then holds `last`"
    );
    // `repeats == 0` is INFINITE — the first sequence above already proves it, and
    // this states it as the rule rather than leaving it to be inferred.
    assert_eq!(
        walk(SpriteAnimMode::Forward, 0, 0, 3, 12).len(),
        12,
        "repeats == 0 is infinite"
    );
    // A degenerate one-frame range holds and never counts a cycle (a cycle per
    // tick would burn a u8 budget at frame rate).
    assert_eq!(
        walk(SpriteAnimMode::Forward, 3, 5, 5, 6),
        vec![5, 5, 5, 5, 5, 5],
        "a one-frame range holds `first`"
    );
}

/// **G5-2**, the clock: the fallback is CLAMPED, SCALED and PAUSE-AWARE.
///
/// The plan's S-D17 names two defects of `Time::real_delta()` in one sentence — an
/// alt-tab stall skipping whole cycles, and a paused game that keeps animating —
/// and `real_delta().min(UI_FALLBACK_MAX_DELTA)` fixes only the first.
/// `Time::delta_secs()` is already clamped, scaled and pause-aware, and AD1's
/// tighter clamp still applies on top of it. All three are asserted, because a
/// remedy that covers one of two named defects and is silent about the other is
/// the shape this campaign keeps finding.
#[test]
fn g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware() {
    const FPS: f32 = 10.0;
    let step = Duration::from_millis(100);

    // (a) CLAMPED: one two-second stall must not advance more than
    //     UI_FALLBACK_MAX_DELTA * fps == 1 frame.
    let (mut world, node) = flipbook_world(0, 15, SpriteAnimMode::Forward, 0, FPS);
    let mut schedule = flipbook_schedule(&mut world);
    tick(&mut world, &mut schedule, Duration::from_secs(2));
    assert_eq!(
        index_of(&world, node),
        1,
        "a two-second alt-tab stall advances ONE frame, not twenty — without the clamp \
         it would skip whole cycles and jump `Once` to its end"
    );

    // (b) PAUSE-AWARE.
    let (mut world, node) = flipbook_world(0, 15, SpriteAnimMode::Forward, 0, FPS);
    let mut schedule = flipbook_schedule(&mut world);
    world.run_system(|mut time: ResMut<Time>| time.pause());
    for _ in 0..5 {
        tick(&mut world, &mut schedule, step);
    }
    assert_eq!(
        index_of(&world, node),
        0,
        "a PAUSED clock animates nothing — `real_delta()` is documented pause-blind and \
         a `min` does not fix that"
    );

    // (c) SCALED.
    let (mut world, node) = flipbook_world(0, 15, SpriteAnimMode::Forward, 0, FPS);
    let mut schedule = flipbook_schedule(&mut world);
    world.run_system(|mut time: ResMut<Time>| time.set_relative_speed(0.5));
    for _ in 0..4 {
        tick(&mut world, &mut schedule, step);
    }
    assert_eq!(
        index_of(&world, node),
        2,
        "at half speed, four 100 ms frames advance two 10 fps frames"
    );
}

// ───────────────────────── G5-3 ────────────────────────────────────────────

/// A probe system counting rows whose `UiSpriteAnim` changed this frame.
fn count_anim_changed(q: Query<(), Changed<UiSpriteAnim>>, mut sink: ResMut<ChangedCensus>) {
    sink.anim += q.iter().count() as u32;
}

/// A probe system counting rows whose `UiSpriteSheet` changed this frame.
fn count_sheet_changed(q: Query<(), Changed<UiSpriteSheet>>, mut sink: ResMut<ChangedCensus>) {
    sink.sheet += q.iter().count() as u32;
}

/// The two probes' shared sink.
#[derive(boyko_macros::Resource, Default)]
struct ChangedCensus {
    anim: u32,
    sheet: u32,
}

/// **G5-3** — the churn split is real, and it is what D8a exists for.
///
/// Three legs:
///
/// 1. a per-frame ADVANCE fires `Changed<UiSpriteSheet>` and NOT
///    `Changed<UiSpriteAnim>` — the split (M5-a merges the two and reds this);
/// 2. at a frame rate where the index does NOT move between two ticks,
///    `Changed<UiSpriteSheet>` stays SILENT — `set_if_neq`'s whole purpose, and
///    nothing else in the ladder reads it (M5-g writes through `&mut` and reds it);
/// 3. an AUTHOR retarget fires `Changed<UiSpriteAnim>`.
///
/// Leg 1 additionally pins the ORDER: it asserts the index moved AND the
/// generation bumped in the SAME frame, which is only true with the flipbook
/// registered ahead of discovery.
#[test]
fn g5_3_the_churn_split_is_real() {
    const FPS: f32 = 10.0;
    let (mut world, node) = flipbook_world(0, 3, SpriteAnimMode::Forward, 0, FPS);
    world.insert_resource(ChangedCensus::default());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let discovery = b.add_system(ui_render_discovery).key();
    let flip = b.add_system(ui_sprite_flipbook).before(discovery).key();
    b.add_system(count_anim_changed).after(flip);
    b.add_system(count_sheet_changed).after(flip);
    let mut schedule = b.build(&mut world);

    // Settle the spawn (which is itself a change to everything).
    for _ in 0..3 {
        tick(&mut world, &mut schedule, Duration::ZERO);
    }
    world.run_system(|mut c: ResMut<ChangedCensus>| {
        c.anim = 0;
        c.sheet = 0;
    });

    // (1) a per-frame advance: the sheet ticks, the anim does not, and the
    //     generation moves in the SAME frame.
    let gen_before = world.resource::<UiRenderGeneration>().generation;
    tick(&mut world, &mut schedule, Duration::from_millis(100));
    assert_eq!(index_of(&world, node), 1, "the frame advanced");
    assert_eq!(
        world.resource::<ChangedCensus>().sheet,
        1,
        "a per-frame advance fires Changed<UiSpriteSheet> — that tick IS the repaint signal"
    );
    assert_eq!(
        world.resource::<ChangedCensus>().anim,
        0,
        "…and does NOT fire Changed<UiSpriteAnim>: the animation's CONFIGURATION is \
         untouched by its own ticking, which is the whole of D8a"
    );
    assert_eq!(
        world.resource::<UiRenderGeneration>().generation,
        gen_before + 1,
        "the generation bumped on the SAME frame as the write — the flipbook is ordered \
         ahead of discovery, so an animating sprite repaints this frame and not the next"
    );

    // (2) a tick too short to move the index must stay silent.
    world.run_system(|mut c: ResMut<ChangedCensus>| {
        c.anim = 0;
        c.sheet = 0;
    });
    let gen_before = world.resource::<UiRenderGeneration>().generation;
    tick(&mut world, &mut schedule, Duration::from_millis(16));
    assert_eq!(index_of(&world, node), 1, "16 ms at 10 fps moves no frame");
    assert_eq!(
        world.resource::<ChangedCensus>().sheet,
        0,
        "`set_if_neq`, not a plain deref: a 12 fps flipbook on a 60 Hz frame must not bump \
         the generation on the four frames in five where the index does not move"
    );
    assert_eq!(
        world.resource::<UiRenderGeneration>().generation,
        gen_before,
        "…so the generation holds and the upload's per-slot gate keeps skipping"
    );

    // (3) an AUTHOR retarget fires the config's own tick.
    world.run_system(|mut c: ResMut<ChangedCensus>| {
        c.anim = 0;
        c.sheet = 0;
    });
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(node).insert(UiSpriteAnim {
            first: 4,
            last: 7,
            fps: FPS,
            mode: SpriteAnimMode::PingPong,
            repeats: 0,
            _pad: [0; 2],
        });
    });
    tick(&mut world, &mut schedule, Duration::ZERO);
    assert_eq!(
        world.resource::<ChangedCensus>().anim,
        1,
        "an author retarget fires Changed<UiSpriteAnim> — the signal a repack of authored \
         animation state would key on"
    );
}

// ───────────────────────── G5-4 ────────────────────────────────────────────

/// A `Bundle` wrapper so `Commands::insert` can take the DENSE cursor — the
/// `dense_d2_routing::T4DenseBundle` idiom. Its existence is itself a small piece
/// of evidence for the claim below: a dense component does not go through the
/// same insert path a table component does.
#[derive(boyko_macros::Bundle)]
struct CursorBundle {
    c: UiSpriteCursor,
}

/// **G5-4** — the cursor is DENSE and does not migrate the archetype.
///
/// `dense_d2_routing`'s property, re-asserted at this consumer: inserting and
/// removing `UiSpriteCursor` leaves the entity's archetype id untouched, so a
/// flipbook node's per-frame private state costs no structural churn. M5-c makes
/// it a table component and reds this.
#[test]
fn g5_4_the_cursor_is_dense_and_does_not_migrate() {
    let mut world = EcsMaster::new();
    let probe: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&probe);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 8.0, h: 8.0 });
        *sink.lock().expect("probe") = Some(e.id());
    });
    let node = probe.lock().expect("probe").expect("spawned node");
    let before = world.entity_archetype_id(node).expect("live").get();

    world.run_system(move |mut cmds: Commands| {
        cmds.entity(node).insert(CursorBundle {
            c: UiSpriteCursor::default(),
        });
    });
    assert!(
        world.dense_contains(node, UiSpriteCursor::component_id()),
        "the cursor is stored in the DENSE column"
    );
    assert_eq!(
        world.entity_archetype_id(node).expect("live").get(),
        before,
        "inserting the cursor must NOT migrate the archetype — that is what dense storage \
         is for, and a table cursor would churn the archetype of every animated node"
    );

    world.run_system(move |mut cmds: Commands| {
        cmds.entity(node).remove::<UiSpriteCursor>();
    });
    assert_eq!(
        world.entity_archetype_id(node).expect("live").get(),
        before,
        "…and removing it must not migrate it back"
    );
}

/// **G5-4**, the sizes — MEASURED by `const _: () = assert!(size_of…)` beside each
/// struct, and re-stated here so a reader of the gate table sees the numbers the
/// plan states.
#[test]
fn g5_4_the_three_components_are_the_stated_sizes_with_spelled_padding() {
    assert_eq!(size_of::<UiSpriteSheet>(), 4);
    assert_eq!(size_of::<UiSpriteAnim>(), 12);
    assert_eq!(size_of::<UiSpriteCursor>(), 8);
    assert_eq!(size_of::<UiSheet>(), 20);
    // The cursor's `dir` DEFAULT is +1, not the derived 0: `#[require]`
    // materializes the cursor through `Default`, and a zero direction would make
    // every PingPong stand still with nothing to say so.
    assert_eq!(UiSpriteCursor::default().dir, 1);
}

// ───────────────────────── G5-12 ───────────────────────────────────────────

/// **G5-12** — the cursor pairing is structural at the AUTHORING site, because it
/// cannot be structural at the component.
///
/// The plan ruled `#[require(UiSpriteCursor)]` on `UiSpriteAnim` so that an
/// authored `flipbook:` could not silently never tick. MEASURED at the S5 build,
/// that spelling PANICS on this kernel: the require pass resolves the required
/// id's `ComponentPool` in the target ARCHETYPE, and a dense id is excluded from
/// every archetype signature and owns no per-archetype pool (dense plan D0). The
/// panic even names an expansion that never happened.
///
/// `AnimatedSpriteBundle` is the buildable remedy, and this row pins BOTH halves —
/// the bundle animates, and the hazard it protects against is real:
///
/// 1. a node spawned from the bundle ticks;
/// 2. a node hand-spawned with `UiSpriteAnim` and NO cursor does not, and does so
///    silently — no panic, no error, a frozen frame 0. That is what the bundle
///    exists to make unreachable, and stating it as an assertion is the difference
///    between a documented hazard and a claimed one.
#[test]
fn g5_12_the_bundle_carries_the_cursor_and_a_cursorless_animation_is_frozen() {
    const FPS: f32 = 10.0;
    let step = Duration::from_millis(100);

    // (1) the bundle: one spawn, and it animates.
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Time::default());
    let probe: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&probe);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(boyko_ui::bundles::AnimatedSpriteBundle {
            layout: boyko_ui::components::UiLayout::default(),
            rect: ComputedRect { x: NODE[0], y: NODE[1], w: NODE[2], h: NODE[3] },
            image: UiImage {
                texture: 3,
                uv_min: [0.0, 0.0],
                uv_max: [1.0, 1.0],
                tint: TINT_OPAQUE_WHITE,
            },
            sheet: UiSpriteSheet { sheet: 0, index: 0 },
            anim: UiSpriteAnim {
                first: 0,
                last: 3,
                fps: FPS,
                mode: SpriteAnimMode::Forward,
                repeats: 0,
                _pad: [0; 2],
            },
            cursor: UiSpriteCursor::default(),
        });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiRoot);
        *sink.lock().expect("probe") = Some(e.id());
    });
    let node = probe.lock().expect("probe").expect("spawned node");
    let mut schedule = flipbook_schedule(&mut world);
    for _ in 0..3 {
        tick(&mut world, &mut schedule, step);
    }
    assert_eq!(
        index_of(&world, node),
        3,
        "a node spawned from `AnimatedSpriteBundle` animates — the bundle IS the cursor \
         requirement, since `#[require]` cannot target a dense component on this kernel"
    );

    // (2) the hazard the bundle protects against, stated as an assertion.
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Time::default());
    let probe: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&probe);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect { x: NODE[0], y: NODE[1], w: NODE[2], h: NODE[3] });
        e.insert(UiBackground::default());
        e.insert(StackIndex(0));
        e.insert(UiImage {
            texture: 3,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 0, index: 0 });
        e.insert(UiSpriteAnim {
            first: 0,
            last: 3,
            fps: FPS,
            mode: SpriteAnimMode::Forward,
            repeats: 0,
            _pad: [0; 2],
        });
        // NO cursor.
        e.insert(UiRoot);
        *sink.lock().expect("probe") = Some(e.id());
    });
    let node = probe.lock().expect("probe").expect("spawned node");
    let mut schedule = flipbook_schedule(&mut world);
    for _ in 0..3 {
        tick(&mut world, &mut schedule, step);
    }
    assert_eq!(
        index_of(&world, node),
        0,
        "a `UiSpriteAnim` with no cursor is FROZEN — silently, with no panic and no error. \
         Spawn `AnimatedSpriteBundle` instead of the components one at a time"
    );
}

// ───────────────────────── G5-6 ────────────────────────────────────────────

/// **G5-6** — `frame_count < cols * rows` is honoured: an out-of-range index
/// clamps to the last FRAME (never a trailing cell) and is COUNTED.
///
/// The counter is the instrument, because the clamp is not otherwise observable:
/// a clamped node draws a real frame, so no picture and no UV assertion can tell
/// "the author asked for frame 13 of a 12-frame sheet" from "the author asked for
/// frame 11".
#[test]
fn g5_6_an_out_of_range_index_clamps_to_the_last_frame_and_is_counted() {
    // A 4x4 grid with only 12 frames: cells 12..15 hold nothing.
    let partial = UiSheet {
        frame_count: 12,
        inset_uv: [0.0, 0.0],
        ..sheet_4x4()
    };
    let expect_last = partial.frame_uv(11);

    let sys = stage_sheet_node(partial, 13, [[0.0, 0.0], [1.0, 1.0]]);
    assert_eq!(
        sprite_record(&sys).uv,
        expect_last,
        "index 13 of a 12-frame sheet samples frame 11 — a trailing cell of a partly \
         filled grid holds nothing, and sampling it would draw garbage silently"
    );
    assert_eq!(
        sys.sheet_index_clamps(),
        1,
        "…and the clamp is COUNTED: it is not otherwise observable, because a clamped \
         node draws a real frame"
    );

    // Exactly at the boundary: `index == frame_count` is out of range.
    let partial = UiSheet {
        frame_count: 12,
        inset_uv: [0.0, 0.0],
        ..sheet_4x4()
    };
    let sys = stage_sheet_node(partial, 12, [[0.0, 0.0], [1.0, 1.0]]);
    assert_eq!(sys.sheet_index_clamps(), 1, "index == frame_count clamps");
    assert_eq!(sprite_record(&sys).uv, expect_last);

    // The last legal index clamps NOTHING — the off-by-one in the other direction.
    let partial = UiSheet {
        frame_count: 12,
        inset_uv: [0.0, 0.0],
        ..sheet_4x4()
    };
    let sys = stage_sheet_node(partial, 11, [[0.0, 0.0], [1.0, 1.0]]);
    assert_eq!(
        sys.sheet_index_clamps(),
        0,
        "index == frame_count - 1 is IN range: a clamp here would mean the bound is off \
         by one in the direction no picture can see"
    );
}

/// **G5-6**, the mint: `register` is the one verb that creates a `SheetId`, and it
/// is what makes the frame arithmetic total.
#[test]
fn g5_6_the_mint_makes_every_registered_sheet_usable() {
    let mut table = UiSheetTable::new();
    assert!(table.is_empty());
    let a = table.register(sheet_4x4());
    let b = table.register(sheet_4x2());
    assert_eq!((a, b), (SheetId(0), SheetId(1)), "dense ids, in order");
    assert_eq!(table.len(), 2);
    assert!(table.get(SheetId(2)).is_none(), "an unminted id resolves to nothing");

    // A degenerate grid is repaired AT THE MINT, so `frame_uv` downstream is a pure
    // function with no error path.
    let mut table = UiSheetTable::new();
    let id = table.register(UiSheet {
        slot: 1,
        cols: 0,
        rows: 0,
        frame_count: 900,
        _pad: [0; 2],
        inset_uv: [0.0, 0.0],
    });
    let sheet = table.get(id).expect("registered");
    assert_eq!((sheet.cols, sheet.rows), (1, 1), "a zero axis is floored to one");
    assert_eq!(
        sheet.frame_count, 1,
        "frame_count is clamped to the number of cells the grid actually has"
    );
}

// ───────────────────────── G5-11 ───────────────────────────────────────────

/// G4-3's landed scene, as pack inputs — the scene S-D15 (3)'s derivation is
/// stated against, so the `4` the tiled golden probes is COMPUTED here rather
/// than asserted there.
fn g4_3_scene(mode: NineSliceMode) -> PackInput {
    PackInput {
        rect: NODE,
        color: 0xFF_00_80_80,
        border_color: 0,
        corner_radius: [0.0; 4],
        border_width: [0.0; 4],
        clip: None,
        text_uv: None,
        image: Some(UiImageInput {
            slot: 5,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        }),
        nine_slice: Some(UiNineSliceInput {
            border_px: [16.0, 24.0, 16.0, 24.0],
            border_uv: [1.0 / 3.0; 4],
            mode: match mode {
                NineSliceMode::Stretch => 0,
                NineSliceMode::Tile => UI_NINE_SLICE_MODE_TILE,
            },
            fill_center: true,
        }),
    }
}

/// **G5-11** — the repeat count is DERIVED, the bit layout is what the shader
/// reads, and every degenerate input is `Stretch`.
///
/// This row exists because S5's headline mechanism — an eight-input ratio with a
/// `round`, two clamps and four degenerate cases — otherwise had no device-free
/// gate at all: it was exercised only by two `BOYKO_UI_GOLDEN_REQUIRE_DEVICE`
/// goldens, which `boot_or_skip` past on a GPU-less box. It also gates the bit
/// LAYOUT (bit 5, 6..=12, 13..=19), which no picture can see.
///
/// The expected `(4, 2)` is hand-derived from S-D15 (3)'s formula on G4-3's own
/// numbers — `64 * (2/3) / ((1/3) * 32) = 4` and `48 * (2/3) / ((1/3) * 48) = 2` —
/// NOT read back out of the implementation.
#[test]
fn g5_11_the_tile_count_is_derived_and_rides_the_flags_word() {
    let tiled = g4_3_scene(NineSliceMode::Tile);
    let ns = tiled.nine_slice.expect("the scene is nine-sliced");
    assert_eq!(
        ui_nine_slice_tiles(&tiled, &ns),
        (4, 2),
        "S-D15 (3)'s ratio on G4-3's scene: the derivation COMPUTES the 4 the tiled \
         golden used to assert"
    );

    // `Stretch` derives nothing, whatever the borders say.
    let stretched = g4_3_scene(NineSliceMode::Stretch);
    let sns = stretched.nine_slice.expect("nine-sliced");
    assert_eq!(
        ui_nine_slice_tiles(&stretched, &sns),
        (1, 1),
        "Stretch is (1, 1) — the mode is what selects the mechanism"
    );

    // Per region: the X count on the centre COLUMN, the Y count on the centre ROW,
    // `1` on every other axis, and NO tile bits at all where both are 1.
    const REGION: [&str; 9] = ["TL", "T", "TR", "L", "C", "R", "BL", "B", "BR"];
    const EXPECT: [(u32, u32); 9] = [
        (1, 1), // TL
        (4, 1), // T
        (1, 1), // TR
        (1, 2), // L
        (4, 2), // C
        (1, 2), // R
        (1, 1), // BL
        (4, 1), // B
        (1, 1), // BR
    ];
    for r in 0..9u32 {
        let rec = pack_ui_nine_slice_instance(&tiled, r, 1.0).expect("both halves present");
        let stretch_rec =
            pack_ui_nine_slice_instance(&stretched, r, 1.0).expect("both halves present");
        let (ex, ey) = EXPECT[r as usize];
        let name = REGION[r as usize];

        if ex <= 1 && ey <= 1 {
            // The ABSOLUTE property FIRST, and it is here because the relative one
            // below could not fail on its own. MEASURED at the S5 build: red
            // mutation M5-j (set `FLAG_TILED` unconditionally) left every gate
            // GREEN, because `tile_flag_bits` is called on the `Stretch` path too —
            // the mutation moved BOTH sides of the comparison equally — and the
            // rendered picture is genuinely unchanged (`frac(local_uv * 1)` IS
            // `local_uv`, S-D15 (1)'s own finding one level down). A comparison
            // between two arms that share the mutated code is not an instrument.
            assert_eq!(
                rec.flags & (FLAG_TILED | (UI_TILE_MASK << UI_TILE_X_SHIFT)
                    | (UI_TILE_MASK << UI_TILE_Y_SHIFT)),
                0,
                "region {name} is 1x1, so its Tile record must carry NO tile bits at all — \
                 not the flag and not either count field. This is the ABSOLUTE form of \
                 S-D15's byte-identity claim; the comparison below cannot see a change \
                 that moves both arms"
            );
            assert_eq!(
                rec.flags, stretch_rec.flags,
                "…and it is therefore BYTE-IDENTICAL to its Stretch record, which is what \
                 keeps a Tile corner indistinguishable from a Stretch corner"
            );
            continue;
        }
        assert_ne!(rec.flags & FLAG_TILED, 0, "region {name} carries FLAG_TILED");
        assert_eq!(
            (rec.flags >> UI_TILE_X_SHIFT) & UI_TILE_MASK,
            ex,
            "region {name}: the X repeat count rides bits 6..=12"
        );
        assert_eq!(
            (rec.flags >> UI_TILE_Y_SHIFT) & UI_TILE_MASK,
            ey,
            "region {name}: the Y repeat count rides bits 13..=19"
        );
        // The SOURCE rect is untouched: the wrap is of the quad parameter, in the
        // shader, inside this same sub-rect. A pack that folded the count into `uv`
        // would sweep N whole sub-rects — under a sheet, N whole neighbouring
        // FRAMES, which is the bleed G5-8 forbids.
        assert_eq!(
            rec.uv, stretch_rec.uv,
            "region {name}: `Tile` changes NO source coordinate — folding the count into \
             `uv` is the mechanism S-D15 (2) struck"
        );
        assert_eq!(
            rec.min_px, stretch_rec.min_px,
            "region {name}: …and no destination coordinate either"
        );
    }
}

/// **G5-11**, the degenerate inputs: each yields `1` (i.e. `Stretch`), and the
/// clamp holds at the field's width.
///
/// A nine-slice with no border on an axis states no source→destination scale on
/// that axis and does not get to guess one. Neither gate nor red covered these
/// before this row existed.
#[test]
fn g5_11_every_degenerate_tile_input_is_stretch() {
    // (dest_centre, dest_border_sum, src_centre, src_border_sum)
    assert_eq!(
        ui_nine_slice_tiles_axis(64.0, 32.0, 1.0 / 3.0, 0.0),
        1,
        "a zero SOURCE border states no scale"
    );
    assert_eq!(
        ui_nine_slice_tiles_axis(96.0, 0.0, 1.0 / 3.0, 2.0 / 3.0),
        1,
        "a zero DESTINATION border states no scale (and would divide by zero)"
    );
    assert_eq!(
        ui_nine_slice_tiles_axis(64.0, 32.0, 0.0, 1.0),
        1,
        "a degenerate centre SOURCE extent — the S-D12 (2) release remedy's own output \
         when the two source insets sum to 1"
    );
    assert_eq!(
        ui_nine_slice_tiles_axis(f32::NAN, 32.0, 1.0 / 3.0, 2.0 / 3.0),
        1,
        "a non-finite ratio is 1, not a NaN cast to a garbage count"
    );
    assert_eq!(
        ui_nine_slice_tiles_axis(1.0, 1000.0, 0.5, 0.5),
        1,
        "a ratio BELOW 1 rounds to 1, never to 0 — a zero count in the field would make \
         the shader compute `frac(local_uv * 0) == 0` and sample one texel"
    );
    // The clamp: an enormous ratio saturates at the field's width rather than
    // wrapping into the neighbouring field.
    let huge = ui_nine_slice_tiles_axis(1.0e6, 1.0, 0.001, 0.999);
    assert_eq!(huge, UI_TILE_MAX, "the count clamps at the field's width");
    assert_eq!(
        huge & !UI_TILE_MASK,
        0,
        "…and therefore fits its field: a count that overflowed would land in the Y \
         count's bits, or in the bindless slot's"
    );
}
