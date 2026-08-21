//! UI-ADVANCED rung S4 — nine-slice, the DEVICE-FREE half
//! (`docs/UI-PLAN-SPRITES.md` gates G4-1, G4-2, G4-5, G4-6, G4-8).
//!
//! Every test here drives **`UiUploadSystem::gather_into_staging`** — the loop
//! the scheduler actually runs — against a bare `EcsMaster` through
//! `EcsMaster::run_system_once`, and reads the result off the public
//! `sys.staged()` / `sys.staging_overflows()` surface. No device, no graphics
//! type, no hand-packed `UiInstance`.
//!
//! Two disciplines the rung's own audit ruled and this file obeys:
//!
//! * **No literal for a record count that is a FUNCTION of the sub space.**
//!   `SLICED_IMAGED` and `SLICED_IMAGED_NO_CENTRE` are expressions over
//!   `UI_NINE_SLICE_REGIONS`, so a later rung that adds a region moves them by
//!   itself. `BARE`, `IMAGED` and `SLICED_ONLY` are deliberately bare literals
//!   and the discipline does NOT reach them: they count records whose existence
//!   is a ROW of S-D12 (1)'s truth table, not a region count — a background is
//!   one record because every node has exactly one, and an unsliced image adds
//!   one more because there is exactly one whole-rect sub code. No expression
//!   over `UI_NINE_SLICE_REGIONS` / `UI_NINE_SLICE_SUB_BASE` / `UI_IMAGE_SUB`
//!   yields `1` or `2` honestly, and writing one would be arithmetic theatre
//!   over a constant that cannot move — the concession the rung already made
//!   for `ui_s0_measure`, stated here too rather than left as a claim this file
//!   does not keep. A literal is likewise right for the component COMBINATION a
//!   case constructs and for the destination geometry it authors: authored data,
//!   not arithmetic.
//! * **No `append` code and no `StackIndex` is read.** `UiUploadSystem.keys` is
//!   private and `UiInstance` carries no stack; both are observed as
//!   CONSEQUENCES in the sorted output — the stack by bracketing the sliced node
//!   between a lower- and a higher-stack plain node, the sub codes by each
//!   record's own kind and geometry.
//!
//! # Two invocations, on purpose
//!
//! ```text
//! cargo test -p boyko-render --test ui_s4_nine_slice              # running 6 tests
//! cargo test -p boyko-render --test ui_s4_nine_slice --release    # running 6 tests
//! ```
//!
//! G4-8 claims no component combination panics the decode **in either build
//! profile**, and the debug run is the strictly weaker leg — it additionally has
//! `debug_assert!` armed, so passing it says nothing about the `.expect`s the
//! release build keeps. The release leg is therefore its own invocation.
//!
//! **The two counts match but the two SETS do not**, and neither run is a
//! superset of the other — one test is profile-gated each way, because each
//! gates a sentence the other profile does not have:
//!
//! * [`g4_5_an_out_of_range_mode_discriminant_is_rejected_at_pack`] is
//!   `#[cfg(debug_assertions)]`; it gates a `debug_assert!`, which does not exist
//!   in release.
//! * [`s_d12_2_a_negative_inset_degenerates_in_release_instead_of_inverting`] is
//!   `#[cfg(not(debug_assertions))]`; it gates S-D12 (2)'s RELEASE remedy, which
//!   a debug build never reaches because the same `debug_assert!` fires first.
//!
//! So `running 6 tests` on both is the expected count and neither is a vacuous
//! filter — but running only one profile leaves one ruled sentence ungated.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles out of the `Send + Sync` one-shot system closure.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_render::{
    ui_render_discovery, UiInstance, UiRenderGeneration, UiUploadSystem, FLAG_TEXTURED,
    UI_NINE_SLICE_REGIONS, UI_STAGING_ROWS,
};
use boyko_ui::components::{
    ComputedClip, ComputedRect, StackIndex, UiBackground, UiImage, UiNineSlice, UiRoot,
};

// ───────────────────────── derived record counts ───────────────────────────

/// Records a node with NEITHER `UiImage` nor `UiNineSlice` emits: its background.
const BARE: usize = 1;
/// Records a node with `UiImage` and no `UiNineSlice` emits: background + image.
const IMAGED: usize = 2;
/// Records a node with `UiNineSlice` and NO `UiImage` emits: its background
/// only — `UiNineSlice` alone is a structural no-op (S-D12 (3)).
const SLICED_ONLY: usize = 1;
/// Records a nine-sliced IMAGED node emits with the centre on: background +
/// every region. Derived — a rung that adds a region moves it.
const SLICED_IMAGED: usize = 1 + UI_NINE_SLICE_REGIONS as usize;
/// The same with `fill_center == false`: the centre sub-quad is the one skipped.
const SLICED_IMAGED_NO_CENTRE: usize = SLICED_IMAGED - 1;

// ───────────────────────── the authored scene ──────────────────────────────

/// The sliced node's destination rect (logical px) — authored, not derived.
const RECT: [f32; 4] = [10.0, 20.0, 96.0, 96.0];
/// Its destination border, `[l, t, r, b]`. Deliberately ASYMMETRIC: `[16;4]`
/// would make `[l,t,r,b]` and `[t,l,b,r]` indistinguishable.
const BORDER_PX: [f32; 4] = [16.0, 24.0, 16.0, 24.0];
/// The image's UV sub-rect — deliberately NOT `(0,0,1,1)`, so "a fraction of the
/// CURRENT sub-rect" is falsifiable against "a fraction of the whole texture".
const UV: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
/// The source inset, `[l, t, r, b]`, as fractions of `UV`. Asymmetric on both
/// axes AND different from `BORDER_PX`'s proportions, so a pack that reused the
/// destination fractions for the source cannot pass.
const BORDER_UV: [f32; 4] = [0.25, 0.5, 0.25, 0.25];

/// The nine destination rects `(x, y, w, h)` the scene above must produce, in
/// contract order (row-major TL, T, TR, L, C, R, BL, B, BR).
///
/// AUTHORED BY HAND from `RECT` and `BORDER_PX` — columns 16 / 64 / 16, rows
/// 24 / 48 / 24 — rather than recomputed from the same formula the pack uses, so
/// the gate cannot agree with the pack by sharing its arithmetic.
const EXPECT_DST: [[f32; 4]; 9] = [
    [10.0, 20.0, 16.0, 24.0], // TL
    [26.0, 20.0, 64.0, 24.0], // T
    [90.0, 20.0, 16.0, 24.0], // TR
    [10.0, 44.0, 16.0, 48.0], // L
    [26.0, 44.0, 64.0, 48.0], // C
    [90.0, 44.0, 16.0, 48.0], // R
    [10.0, 92.0, 16.0, 24.0], // BL
    [26.0, 92.0, 64.0, 24.0], // B
    [90.0, 92.0, 16.0, 24.0], // BR
];

/// The nine SOURCE UV rects `(u0, v0, u1, v1)`, same order.
///
/// Also authored by hand: `UV` spans u 0.25..0.75 and v 0.5..1.0, so the inset
/// `[0.25, 0.5, 0.25, 0.25]` of that sub-rect puts the u cuts at 0.375 / 0.625
/// and the v cuts at 0.75 / 0.875. Every value is exactly representable in
/// binary32, so these are equalities and not tolerances.
const EXPECT_SRC: [[f32; 4]; 9] = [
    [0.25, 0.5, 0.375, 0.75],
    [0.375, 0.5, 0.625, 0.75],
    [0.625, 0.5, 0.75, 0.75],
    [0.25, 0.75, 0.375, 0.875],
    [0.375, 0.75, 0.625, 0.875],
    [0.625, 0.75, 0.75, 0.875],
    [0.25, 0.875, 0.375, 1.0],
    [0.375, 0.875, 0.625, 1.0],
    [0.625, 0.875, 0.75, 1.0],
];

/// The region names, for assertion messages that say WHICH slice moved.
const REGION: [&str; 9] = ["TL", "T", "TR", "L", "C", "R", "BL", "B", "BR"];

/// The sliced node's clip (its own rect), so inheritance is observable on every
/// slice.
const CLIP: [f32; 4] = [10.0, 20.0, 96.0, 96.0];

/// The bindless slot the sliced node's image names.
const SLOT: u32 = 7;

// ───────────────────────── shared plumbing ─────────────────────────────────

fn discovery_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    b.build(world)
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

/// The `UiImage` every sliced node in this file carries: an opaque white tint on
/// `SLOT`, at the `UV` sub-rect. Opaque because the pack premultiplies the tint
/// into every slice's colour and the default tint is alpha 0.
fn image() -> UiImage {
    UiImage {
        texture: SLOT,
        uv_min: [UV[0], UV[1]],
        uv_max: [UV[2], UV[3]],
        tint: 0xFF_FF_FF_FF,
    }
}

fn nine_slice(fill_center: bool) -> UiNineSlice {
    UiNineSlice {
        border_px: BORDER_PX,
        border_uv: BORDER_UV,
        fill_center,
        ..UiNineSlice::default()
    }
}

fn textured(rec: &UiInstance) -> bool {
    rec.flags & FLAG_TEXTURED != 0
}

/// Asserts one staged record IS the sliced node's region `r`: right destination,
/// right source, textured, carrying the node's inherited clip.
fn assert_region(rec: &UiInstance, r: usize, scale: f32) {
    let name = REGION[r];
    let d = EXPECT_DST[r];
    assert!(textured(rec), "slice {name} must be a TEXTURED record");
    assert_eq!(
        rec.min_px,
        [d[0] * scale, d[1] * scale],
        "slice {name}: destination origin"
    );
    assert_eq!(
        rec.size_px,
        [d[2] * scale, d[3] * scale],
        "slice {name}: destination extent — a corner is exactly `border_px`, never a \
         fraction of the rect"
    );
    assert_eq!(rec.uv, EXPECT_SRC[r], "slice {name}: source UV sub-rect");
    assert_eq!(
        rec.clip,
        [
            CLIP[0] * scale,
            CLIP[1] * scale,
            (CLIP[0] + CLIP[2]) * scale,
            (CLIP[1] + CLIP[3]) * scale
        ],
        "slice {name} inherits the node's own clip"
    );
    assert_eq!(
        (rec.flags >> boyko_render::UI_SLOT_SHIFT) & boyko_render::UI_SLOT_MASK,
        SLOT,
        "slice {name} samples the node's own `UiImage` slot"
    );
}

// ───────────────────────── G4-1 ────────────────────────────────────────────

/// Builds the G4-1 scene: a plain root at stack 0, the nine-sliced imaged node at
/// stack 1 carrying its own clip, a plain node at stack 2.
///
/// The BRACKETING is the point: `UiInstance` carries no `StackIndex`, so "all
/// nine inherit the parent's stack" is only observable as a consequence — a
/// slice that lost its parent's stack sorts outside the block between the two
/// plain nodes.
fn build_bracketed_world(fill_center: bool) -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.run_system(move |mut cmds: Commands| {
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 });
            e.insert(UiBackground { color: 0xFF11_2233, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiRoot);
            e.id()
        };
        // The nine-sliced node (stack 1), between the two plain ones.
        {
            let mut e = cmds.spawn(ComputedRect {
                x: RECT[0],
                y: RECT[1],
                w: RECT[2],
                h: RECT[3],
            });
            e.insert(UiBackground { color: 0xFF44_5566, ..UiBackground::default() });
            e.insert(ComputedClip { x: CLIP[0], y: CLIP[1], w: CLIP[2], h: CLIP[3] });
            e.insert(StackIndex(1));
            e.insert(image());
            e.insert(nine_slice(fill_center));
            e.set_parent(root);
        }
        // A plain node ABOVE it (stack 2) — the upper bracket.
        {
            let mut e = cmds.spawn(ComputedRect { x: 150.0, y: 150.0, w: 10.0, h: 10.0 });
            e.insert(UiBackground { color: 0xFF77_8899, ..UiBackground::default() });
            e.insert(StackIndex(2));
            e.set_parent(root);
        }
    });
    world
}

/// G4-1: the expansion is nine sub-quads **in addition to** the node's
/// background rect, consecutive and in contract order in the STAGED stream, each
/// inheriting the node's clip — and the whole block stays between the two plain
/// bracketing nodes, which is how the inherited `StackIndex` is observed.
#[test]
fn g4_1_nine_sub_quads_are_added_to_the_background_consecutive_and_inheriting() {
    let mut world = build_bracketed_world(true);
    let mut schedule = discovery_schedule(&mut world);
    // scale 2.0 so the logical→physical fold is observable on every slice.
    let mut sys = UiUploadSystem::new(2.0);
    settle(&mut world, &mut schedule, &mut sys);

    let staged = sys.staged();
    assert_eq!(
        staged.len(),
        BARE + SLICED_IMAGED + BARE,
        "root + the sliced node's block + the upper bracket"
    );

    // The lower bracket.
    assert!(!textured(&staged[0]), "index 0 is the plain root");
    assert_eq!(staged[0].size_px, [400.0, 400.0], "the root paints first (stack 0)");

    // The sliced node's block: its background FIRST (D4 paints the rect before
    // what sits on it), then the nine regions in contract order.
    assert!(
        !textured(&staged[1]),
        "the sliced node's own BACKGROUND opens its block, and it is untextured"
    );
    assert_eq!(
        staged[1].min_px,
        [RECT[0] * 2.0, RECT[1] * 2.0],
        "the block opens on the sliced node's own rect"
    );
    assert_eq!(staged[1].size_px, [RECT[2] * 2.0, RECT[3] * 2.0]);
    for r in 0..UI_NINE_SLICE_REGIONS as usize {
        assert_region(&staged[BARE + BARE + r], r, 2.0);
    }

    // The upper bracket: still LAST. A slice that lost the parent's stack would
    // have sorted past it.
    let last = staged.len() - 1;
    assert!(!textured(&staged[last]), "the stack-2 node is plain");
    assert_eq!(
        staged[last].size_px,
        [20.0, 20.0],
        "the stack-2 node still paints last — no slice escaped its parent's stack"
    );
}

/// G4-1, the `fill_center == false` row: the CENTRE is the one region skipped,
/// the other eight keep their contract order, and the count drops by exactly one.
#[test]
fn g4_1_fill_center_false_skips_exactly_the_centre() {
    let mut world = build_bracketed_world(false);
    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    let staged = sys.staged();
    assert_eq!(
        staged.len(),
        BARE + SLICED_IMAGED_NO_CENTRE + BARE,
        "one fewer record than the centre-on scene, and exactly one"
    );

    // The eight surviving regions, in contract order, skipping index 4 (C).
    let block = &staged[BARE + BARE..];
    let mut dst = 0usize;
    for r in 0..UI_NINE_SLICE_REGIONS as usize {
        if r == 4 {
            continue;
        }
        assert_region(&block[dst], r, 1.0);
        dst += 1;
    }
    assert_eq!(dst, SLICED_IMAGED_NO_CENTRE - 1, "eight regions were checked");
}

// ───────────────────────── G4-2 ────────────────────────────────────────────

/// G4-2: the STAGED order is D4's, read BY NAME across two nodes whose last
/// terms are ALTERNATIVES — and the sliced node emits **no whole-rect image
/// record**, which is the whole of S-D12 (1).
///
/// Node A (stack 0, the root) carries an image and no nine-slice: background,
/// then image. Node B (stack 1) carries both: background, then nine regions,
/// and NOTHING after them. The two blocks are at different stacks, so a record
/// that escaped its node's block would land in the other one.
#[test]
fn g4_2_staged_order_is_d4_and_a_sliced_node_emits_no_image_record() {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.run_system(move |mut cmds: Commands| {
        // Node A — background + image, NO nine-slice (subs 0 and the image sub).
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 });
            e.insert(UiBackground { color: 0xFF01_0203, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiImage {
                texture: 3,
                uv_min: [0.0, 0.0],
                uv_max: [1.0, 1.0],
                tint: 0xFF_FF_FF_FF,
            });
            e.insert(UiRoot);
            e.id()
        };
        // Node B — background + nine-slice + image (subs 0 and the nine regions).
        {
            let mut e = cmds.spawn(ComputedRect {
                x: RECT[0],
                y: RECT[1],
                w: RECT[2],
                h: RECT[3],
            });
            e.insert(UiBackground { color: 0xFF04_0506, ..UiBackground::default() });
            e.insert(ComputedClip { x: CLIP[0], y: CLIP[1], w: CLIP[2], h: CLIP[3] });
            e.insert(StackIndex(1));
            e.insert(image());
            e.insert(nine_slice(true));
            e.set_parent(root);
        }
    });

    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    let staged = sys.staged();
    assert_eq!(
        staged.len(),
        IMAGED + SLICED_IMAGED,
        "node A emits background + image; node B emits background + every region and \
         NO image record — a tenth textured record on B would show up here and only here"
    );

    // Node A's block: rect BEFORE image (D4), the image covering the whole rect.
    assert!(!textured(&staged[0]), "A[0] is A's background rect");
    assert!(textured(&staged[1]), "A[1] is A's whole-rect image — D4 paints it after");
    assert_eq!(
        staged[1].min_px, staged[0].min_px,
        "A's image is the node's own rect, verbatim"
    );
    assert_eq!(staged[1].size_px, staged[0].size_px);
    assert_eq!(staged[1].uv, [0.0, 0.0, 1.0, 1.0], "A's image takes its own UV whole");

    // Node B's block: background, then the nine regions, then NOTHING.
    assert!(!textured(&staged[IMAGED]), "B's block opens on B's untextured background");
    assert_eq!(
        staged[IMAGED].size_px,
        [RECT[2], RECT[3]],
        "B's background is B's whole rect"
    );
    for r in 0..UI_NINE_SLICE_REGIONS as usize {
        assert_region(&staged[IMAGED + BARE + r], r, 1.0);
    }
    // The absence, stated as geometry rather than as a count alone: the LAST
    // record is BR, not a whole-rect sprite.
    let last = &staged[staged.len() - 1];
    assert_eq!(
        last.size_px,
        [EXPECT_DST[8][2], EXPECT_DST[8][3]],
        "the sliced node's block ENDS on its BR slice — a whole-rect image record \
         after it is the bug S-D12 (1) forbids"
    );
    assert_ne!(
        last.size_px,
        [RECT[2], RECT[3]],
        "…and in particular the last record is not the node's whole rect"
    );
}

// ───────────────────────── G4-8 ────────────────────────────────────────────

/// G4-8: no component combination can panic the decode, and `UiNineSlice` alone
/// is a no-op. All four rows of S-D12 (1)'s truth table in ONE world; the total
/// is DERIVED, so a sub code added later moves it without an edit here.
///
/// The row that matters most is the third: a node carrying `UiNineSlice` and no
/// `UiImage`. Before S4 the decode was a binary `if` whose `else` arm ended in
/// `.expect(..)` over `pack_ui_image_instance`, which opens `let image =
/// input.image?` — so that node panicked in RELEASE as well as debug, and no
/// gate constructed it.
#[test]
fn g4_8_every_component_combination_packs_and_nine_slice_alone_is_a_no_op() {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.run_system(move |mut cmds: Commands| {
        // Row 1 — bare: neither component.
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
            e.insert(UiBackground { color: 0xFF01_0101, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiRoot);
            e.id()
        };
        // Row 2 — imaged, not sliced.
        {
            let mut e = cmds.spawn(ComputedRect { x: 1.0, y: 1.0, w: 8.0, h: 8.0 });
            e.insert(UiBackground { color: 0xFF02_0202, ..UiBackground::default() });
            e.insert(StackIndex(1));
            e.insert(image());
            e.set_parent(root);
        }
        // Row 3 — SLICED, NOT imaged. The node the pre-S4 decode panicked on.
        {
            let mut e = cmds.spawn(ComputedRect { x: 2.0, y: 2.0, w: 8.0, h: 8.0 });
            e.insert(UiBackground { color: 0xFF03_0303, ..UiBackground::default() });
            e.insert(StackIndex(2));
            e.insert(nine_slice(true));
            e.set_parent(root);
        }
        // Row 4 — sliced AND imaged.
        {
            let mut e = cmds.spawn(ComputedRect {
                x: RECT[0],
                y: RECT[1],
                w: RECT[2],
                h: RECT[3],
            });
            e.insert(UiBackground { color: 0xFF04_0404, ..UiBackground::default() });
            e.insert(StackIndex(3));
            e.insert(image());
            e.insert(nine_slice(true));
            e.set_parent(root);
        }
    });

    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    let staged = sys.staged();
    assert_eq!(
        staged.len(),
        BARE + IMAGED + SLICED_ONLY + SLICED_IMAGED,
        "the four truth-table rows, each contributing its own derived count"
    );

    // Row 3's node contributes EXACTLY ONE record — its background — and it is
    // not textured. It sits between row 2's pair and row 4's block by stack.
    let row3 = &staged[BARE + IMAGED];
    assert!(
        !textured(row3),
        "`UiNineSlice` alone emits the node's background and nothing else — never \
         nine invisible quads, and never a textured record with no texture"
    );
    assert_eq!(row3.size_px, [8.0, 8.0], "row 3's single record is its own rect");
    // …and the next record already belongs to row 4.
    assert!(
        !textured(&staged[BARE + IMAGED + SLICED_ONLY]),
        "row 4's block opens on its background"
    );
}

// ───────────────────────── G4-6 ────────────────────────────────────────────

/// G4-6: the staging box holds the stated node budget. `UI_MAX_NODES`
/// nine-sliced imaged nodes must stage the derived emission with ZERO overflow
/// clamps and no `debug_assert!` — the box production packs into is a FIXED
/// `Box<[UiInstance]>` that TRUNCATES, not a growable `Vec`, so nothing else in
/// the rung can see this.
#[test]
fn g4_6_the_staging_box_holds_the_stated_node_budget() {
    let n = boyko_render::UI_MAX_NODES;
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect {
            x: RECT[0],
            y: RECT[1],
            w: RECT[2],
            h: RECT[3],
        });
        e.insert(UiBackground { color: 0xFF20_2020, ..UiBackground::default() });
        e.insert(StackIndex(0));
        e.insert(image());
        e.insert(nine_slice(true));
        e.insert(UiRoot);
        *probe.lock().expect("probe") = Some(e.id());
    });
    let root = sink.lock().expect("probe").expect("root spawned");

    let children = n - 1;
    world.run_system(move |mut cmds: Commands| {
        for i in 0..children {
            let mut e = cmds.spawn(ComputedRect {
                x: (i % 64) as f32,
                y: (i / 64) as f32,
                w: RECT[2],
                h: RECT[3],
            });
            e.insert(UiBackground { color: 0xFF40_4040, ..UiBackground::default() });
            e.insert(StackIndex((i % 8) as u32));
            e.insert(image());
            e.insert(nine_slice(true));
            e.set_parent(root);
        }
    });

    let mut schedule = discovery_schedule(&mut world);
    let mut sys = UiUploadSystem::new(1.0);
    settle(&mut world, &mut schedule, &mut sys);

    let expected = n * SLICED_IMAGED;
    assert_eq!(
        sys.staged().len(),
        expected,
        "{n} nine-sliced imaged nodes stage {expected} records"
    );
    assert_eq!(
        sys.staging_overflows(),
        0,
        "the stated node budget must not clamp: the box holds {UI_STAGING_ROWS} rows and \
         the scene emits {expected}"
    );
    assert!(
        expected <= UI_STAGING_ROWS,
        "…and the budget is a CONSTANT relation, not a property of this run"
    );
}

// ───────────────────────── G4-5 ────────────────────────────────────────────

/// G4-5, the discriminant half: an out-of-range `mode` in the pack's RAW `u8` is
/// rejected at the pack boundary.
///
/// The AUTHORED component carries the typed `NineSliceMode`, where the type
/// system already forbids the value — an out-of-range discriminant there would
/// need a `transmute`, which is instant UB and cannot be a gate. So the raw byte
/// crosses the crate boundary and is `debug_assert!`ed here, the exact
/// `UiImageInput::slot` precedent.
///
/// The other half of G4-5 is not a runtime test at all: the one-variant `const`
/// match beside `NineSliceMode` in `boyko_ui` is `error[E0004]` the moment a
/// second variant is added.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a UI nine-slice mode is a legal NineSliceMode discriminant")]
fn g4_5_an_out_of_range_mode_discriminant_is_rejected_at_pack() {
    let input = boyko_render::PackInput {
        rect: [0.0, 0.0, 32.0, 32.0],
        color: 0xFF_00_00_FF,
        border_color: 0,
        corner_radius: [0.0; 4],
        border_width: [0.0; 4],
        clip: None,
        text_uv: None,
        image: Some(boyko_render::UiImageInput {
            slot: 1,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: 0xFF_FF_FF_FF,
        }),
        nine_slice: Some(boyko_render::UiNineSliceInput {
            border_px: [4.0; 4],
            border_uv: [0.25; 4],
            // One past the last legal discriminant.
            mode: boyko_render::UI_NINE_SLICE_MODE_COUNT,
            fill_center: true,
        }),
    };
    let _ = boyko_render::pack_ui_nine_slice_instance(&input, 0, 1.0);
}

// ───────────────────── S-D12 (2), the RELEASE half ─────────────────────────

/// S-D12 (2)'s validity domain, RELEASE behaviour: an out-of-domain inset
/// DEGENERATES, it never inverts.
///
/// # Why this test is release-only, and why it had to exist
///
/// The domain is "each side in `[0, 1)` with `l + r < 1`" for the source and
/// non-negative for the destination, `debug_assert!`ed at the pack — so in a
/// debug build this input never reaches the split at all. The ruling's whole
/// point is the OTHER build: "in release the offending axis's two sides are
/// scaled down proportionally … the centre source region degenerates to zero
/// width rather than inverting". A `debug_assert!` cannot gate that sentence;
/// only a release run can, and until this test nothing ran it.
///
/// It found the sentence FALSE for one half of the domain. The guard was
/// `sum > extent`, which a NEGATIVE side does not trip — `-0.5 + 0.25` is well
/// under `1.0`, so no shrink fired and the split ran on the raw value.
/// MEASURED before the fix, in `--release`, with exactly the input below:
/// TL's `uv` came out `[0.25, 0.5, 0.0, 0.625]` — `u1 < u0`, the
/// negative-extent UV rect the ruling exists to forbid — while the centre's
/// u-extent came out WIDER than the whole `UiImage` sub-rect it is a fraction
/// of, and TL's `size_px` was negative.
///
/// So the assertions below are properties of the RULING, not of these numbers:
/// no region inverts on either axis, no region leaves the node's rect, and no
/// region samples outside the image's own UV sub-rect. Plus the two exact
/// degeneracies the ruling names, which is what separates "clamped to zero"
/// from any other way of not inverting.
#[test]
#[cfg(not(debug_assertions))]
fn s_d12_2_a_negative_inset_degenerates_in_release_instead_of_inverting() {
    /// `l` is negative on both borders — the half of the domain `sum > extent`
    /// cannot see.
    const BAD_UV: [f32; 4] = [-0.5, 0.25, 0.25, 0.25];
    const BAD_PX: [f32; 4] = [-8.0, 24.0, 16.0, 24.0];

    let input = boyko_render::PackInput {
        rect: RECT,
        color: 0xFF_00_00_FF,
        border_color: 0,
        corner_radius: [0.0; 4],
        border_width: [0.0; 4],
        clip: None,
        text_uv: None,
        image: Some(boyko_render::UiImageInput {
            slot: SLOT,
            uv: UV,
            tint: 0xFF_FF_FF_FF,
        }),
        nine_slice: Some(boyko_render::UiNineSliceInput {
            border_px: BAD_PX,
            border_uv: BAD_UV,
            mode: 0,
            fill_center: true,
        }),
    };

    for r in 0..UI_NINE_SLICE_REGIONS {
        let rec = boyko_render::pack_ui_nine_slice_instance(&input, r, 1.0)
            .expect("the node carries both halves of the capability");
        let name = REGION[r as usize];

        assert!(
            rec.uv[2] >= rec.uv[0] && rec.uv[3] >= rec.uv[1],
            "slice {name}: the SOURCE rect must never invert in release — got {:?}",
            rec.uv
        );
        assert!(
            rec.size_px[0] >= 0.0 && rec.size_px[1] >= 0.0,
            "slice {name}: the DESTINATION extent must never go negative in release — \
             got {:?}",
            rec.size_px
        );
        assert!(
            rec.uv[0] >= UV[0] && rec.uv[2] <= UV[2] && rec.uv[1] >= UV[1] && rec.uv[3] <= UV[3],
            "slice {name}: no region may sample OUTSIDE the node's own UiImage sub-rect \
             {UV:?} — got {:?}. (A centre wider than the whole sub-rect is what an \
             unclamped negative inset produced.)",
            rec.uv
        );
        assert!(
            rec.min_px[0] >= RECT[0]
                && rec.min_px[1] >= RECT[1]
                && rec.min_px[0] + rec.size_px[0] <= RECT[0] + RECT[2]
                && rec.min_px[1] + rec.size_px[1] <= RECT[1] + RECT[3],
            "slice {name}: no region may leave the node's own rect — got origin {:?} \
             extent {:?}",
            rec.min_px,
            rec.size_px
        );
    }

    // The ruled degeneracy itself, stated exactly: the offending side collapses
    // to ZERO. TL is the corner both bad sides meet at, so its `l`-side extent is
    // the one that must be gone — on the destination and on the source alike.
    let tl = boyko_render::pack_ui_nine_slice_instance(&input, 0, 1.0).expect("TL packs");
    assert_eq!(
        tl.size_px[0], 0.0,
        "TL's destination width is the negative `border_px[0]`, degenerated to zero"
    );
    assert_eq!(
        tl.uv[2], tl.uv[0],
        "TL's source width is the negative `border_uv[0]`, degenerated to zero"
    );
}
