//! UI-ADVANCED rung S6 — the `.ui` authoring landing for the sprite vocabulary
//! (`docs/UI-PLAN-SPRITES.md` S6, gates G6-1, G6-2, G6-3, G6-4).
//!
//! G6-5 (the two equivalence comparators) lives in `p6a_equivalence.rs`, because
//! one of the two hand lists it gates is private to that file.
//!
//! ```text
//! cargo test -p boyko-ui --test ui_s6_authoring             # running 7 tests
//! cargo test -p boyko-ui --test ui_s6_authoring --release   # running 7 tests
//! ```
//!
//! Both profiles run the SAME seven tests: nothing here is `cfg`-gated, and the
//! campaign's `running N` rule is about counts over DIFFERENT sets, so the names
//! are reported from both.
//!
//! # Every row's observable depends on what the row exists to prove
//!
//! That is the correction S-D20 (2)-(5) made to this rung's original table, and
//! it is what each test below is shaped around:
//!
//! * **G6-1** compares the serialization to the INPUT, never to itself. The
//!   existing corpus's `assert_serialize_fixed_point` compares `s1` to `s2`, and
//!   a component the serializer DROPS is dropped from both — MEASURED on
//!   `UiImage`, which parses, inserts, and is written by nothing, with the corpus
//!   green over it.
//! * **G6-2** asserts the live value MOVED and then that the component is ABSENT.
//!   "Hot reload preserves them" is exactly what happens with NO reconcile arm at
//!   all (`patch_node` preserves by omission), so the original wording named the
//!   one outcome its own mutation could not disturb.
//! * **G6-3** asserts on the LOWERING report. `parse_ui` does not know component
//!   types; `parse_and_insert` does, and it runs inside `spawn_ui_tree`.
//! * **G6-4** drives a real tick over a PARSED node — the rung's whole reason for
//!   existing, and the only gate in either rung that does.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure. Not engine code — the whole file is compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::time::Time;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_ui::components::{
    NineSliceMode, SpriteAnimMode, UiNineSlice, UiSpriteAnim, UiSpriteCursor, UiSpriteSheet,
};
use boyko_ui::reload::tree_view::UiTreeView;
use boyko_ui::sprite::ui_sprite_flipbook;
use boyko_ui::text::{parse_ui, serialize_ui, spawn_ui_tree, UiParseReport};

use p3_common::{discover_ui_roots, ReloadWorld};

/// Lowers `src` into a fresh world and returns `(world, root, LOWERING report)`.
///
/// The report is the one `spawn_ui_tree` writes — NOT `parse_ui`'s. The shared
/// `p3_common::spawn_dot_ui` asserts the PARSE report and then hands the lowering
/// a `owned.report.clone()` that is dropped, so a lowering diagnostic is
/// unobservable through it (S-D20 (3)). Every assertion in this file that talks
/// about the closed `match` reads THIS report.
fn lower(src: &str) -> (EcsMaster, Option<Entity>, UiParseReport) {
    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let ents: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let rep: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let ep = Arc::clone(&ents);
    let rp = Arc::clone(&rep);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let roots = spawn_ui_tree(&owned, &mut cmds, &mut report);
        let mut v = ep.lock().expect("probe");
        for r in roots.iter() {
            v.push(r);
        }
        *rp.lock().expect("probe") = report;
    });
    let root = ents.lock().expect("probe").first().copied();
    let report = rep.lock().expect("probe").clone();
    (world, root, report)
}

/// Serializes the whole document subtree of `world`.
fn serialize_world(world: &EcsMaster) -> String {
    let roots = discover_ui_roots(world);
    let view = UiTreeView::build(world, &roots);
    let mut out = String::new();
    serialize_ui(&view, &mut out);
    out
}

// ───────────────────────────── G6-1 ────────────────────────────────────────

/// A CANONICAL `.ui` document in `serialize_ui`'s own output form, carrying all
/// three S6 components and NO `UiImage`.
///
/// The `UiImage` exclusion is a RECORDED DEPARTURE, not a convenience. A
/// realistic sprite node must carry `UiImage` — it is the capability, and the
/// sheet only substitutes its slot and UV — and that node cannot round-trip
/// today: MEASURED, a `.ui` source spelling `UiImage { .. }` parses and inserts,
/// and `serialize_ui` emits the node's `UiLayout` line and nothing else, because
/// `write_node` reads only `LiveNode`'s fields and `UiImage` is not among them.
/// It is a CLASS, not an instance — ten of the dispatch's component arms have no
/// serializer arm — and S6 neither creates it nor fixes it (S-D20 (4), filed for
/// the owner in `docs/OPEN-QUESTIONS.md`). The three components S6 lands are
/// landed MORE completely than the one they modify.
///
/// `max_width` / `max_height` are spelled FINITE on purpose: `UiLayout::default`
/// uses `Px(f32::MAX)`, whose canonical spelling is a 39-digit literal that would
/// make this fixture unreadable without proving anything extra.
const CANONICAL: &str = "\
// boyko-engine .ui — generated; edits below the version line are canonicalized on rewrite
version=1
#sprite  UiLayout { layout_type: Column, position_type: Relative, width: Px(64), height: Px(64), min_width: Auto, min_height: Auto, max_width: Px(1000), max_height: Px(1000) }
    UiNineSlice { border_px: [8, 8, 8, 8], border_uv: [0.25, 0.25, 0.25, 0.25], mode: Tile, fill_center: true }
    UiSpriteSheet { sheet: 1, index: 2 }
    UiSpriteAnim { first: 0, last: 3, fps: 12, mode: PingPong, repeats: 4 }
";

/// **G6-1** — the round trip, for the three new components, against the INPUT.
///
/// The observable depends on the serializer arms EXISTING: drop
/// `write_ui_sprite_anim`'s emit block and this fails, where a fixed-point
/// comparison would not (the component would be missing from both sides).
#[test]
fn g6_1_the_three_components_round_trip_to_the_authored_bytes() {
    let (world, root, report) = lower(CANONICAL);
    assert!(report.is_clean(), "the canonical sprite document lowers clean: {:?}", report.errors);
    let e = root.expect("the #sprite node spawned");

    // The three landed, with the authored VALUES (a serializer that wrote
    // defaults would still round-trip if the parser also produced defaults).
    assert_eq!(
        world.get_component::<UiNineSlice>(e).map(|v| (v.border_px, v.mode, v.fill_center)),
        Some(([8.0, 8.0, 8.0, 8.0], NineSliceMode::Tile, true)),
        "UiNineSlice parsed with its authored values"
    );
    assert_eq!(
        world.get_component::<UiSpriteSheet>(e).copied(),
        Some(UiSpriteSheet { sheet: 1, index: 2 }),
        "UiSpriteSheet parsed with its authored values"
    );
    assert_eq!(
        world.get_component::<UiSpriteAnim>(e).map(|v| (v.first, v.last, v.fps, v.mode, v.repeats)),
        Some((0, 3, 12.0, SpriteAnimMode::PingPong, 4)),
        "UiSpriteAnim parsed with its authored values"
    );

    let out = serialize_world(&world);
    assert_eq!(
        out, CANONICAL,
        "the serialization must equal the AUTHORED bytes, not merely be a fixed point"
    );
}

/// **G6-1**, the negative the ruling turns on: the `on_add`-materialized cursor is
/// NOT serialized, so closing the flipbook hole costs the round trip nothing.
///
/// Stated as an assertion rather than as prose, because "it happens not to be a
/// `LiveNode` field" is exactly the kind of fact that stops being true when
/// somebody adds a field.
#[test]
fn g6_1_the_materialized_cursor_never_reaches_the_text() {
    let (world, root, _report) = lower(CANONICAL);
    let e = root.expect("the #sprite node spawned");
    assert!(
        world.has_component(e, UiSpriteCursor::component_id()),
        "the authored animation got its cursor from the on_add hook"
    );
    let out = serialize_world(&world);
    assert!(
        !out.contains("UiSpriteCursor"),
        "runtime state must never be written back into authorable text:\n{out}"
    );
    assert_eq!(out, CANONICAL, "…and its presence must not perturb the round trip either");
}

// ───────────────────────────── G6-2 ────────────────────────────────────────

/// A one-node document whose `UiNineSlice` carries `border_px` = `inset`.
fn nine_slice_doc(inset: u32) -> String {
    format!(
        "version=1\n\
         #panel  UiLayout {{ layout_type: Column, width: Px(64), height: Px(64) }}\n    \
         UiNineSlice {{ border_px: [{inset}, {inset}, {inset}, {inset}], border_uv: [0.25, 0.25, 0.25, 0.25], mode: Stretch, fill_center: true }}\n"
    )
}

/// **G6-2**, the EDIT leg — a reloaded edit must MOVE the live value.
///
/// The row this replaces claimed hot reload "PRESERVES" the components, which is
/// precisely what a component with NO reconcile arm does: `patch_node`'s own doc
/// is *"transient components + `UiSourceOrder` are preserved by omission"*, and
/// `patch_unit_struct`'s remove branch `(None, Some(_))` is reachable only for a
/// component the patcher already tracks. A gate written to observe a
/// disappearance observes nothing and passes (S-D20 (5)).
///
/// The two legs are two TESTS, not two halves of one, because a first leg that
/// fails hides the second — the same shadowing `--no-fail-fast` exists to stop
/// one level up. M6-b must be observed reddening BOTH.
#[test]
fn g6_2_a_reloaded_edit_moves_the_live_value() {
    let mut rw = ReloadWorld::new("s6_nine_edit", &nine_slice_doc(8));
    let node = rw.find_named("panel").expect("the #panel node spawned at startup");
    assert_eq!(
        rw.world().get_component::<UiNineSlice>(node).map(|v| v.border_px),
        Some([8.0; 4]),
        "the initial load carries the authored border"
    );

    rw.reload(&nine_slice_doc(12));
    let node = rw.find_named("panel").expect("the survivor kept its name");
    assert_eq!(
        rw.world().get_component::<UiNineSlice>(node).map(|v| v.border_px),
        Some([12.0; 4]),
        "an edited UiNineSlice must reach the live world — with no reconcile arm the \
         component goes STALE, not absent, and a 'preserved' assertion cannot see it"
    );
}

/// **G6-2**, the DELETE leg — a component deleted from the file must be ABSENT.
///
/// This is the leg the struck row could never have failed: nothing sweeps an
/// unlisted component off a survivor, so without the arm the component simply
/// stays.
#[test]
fn g6_2_a_reloaded_deletion_removes_the_component() {
    let mut rw = ReloadWorld::new("s6_nine_del", &nine_slice_doc(8));
    let node = rw.find_named("panel").expect("the #panel node spawned at startup");
    assert!(
        rw.world().has_component(node, UiNineSlice::component_id()),
        "the initial load carries the component"
    );

    rw.reload("version=1\n#panel  UiLayout { layout_type: Column, width: Px(64), height: Px(64) }\n");
    let node = rw.find_named("panel").expect("the survivor kept its name");
    assert!(
        !rw.world().has_component(node, UiNineSlice::component_id()),
        "a UiNineSlice deleted from the file must be REMOVED from the survivor"
    );
}

/// **G6-2**, the animation leg: an edited `UiSpriteAnim` reaches the world, and
/// the reload does NOT reset the running cursor.
///
/// MEASURED and pinned because it is the one property that distinguishes `on_add`
/// from `on_insert`: the hook does not re-fire on a re-insert, so an author
/// retuning `fps` in a live file does not restart the animation.
#[test]
fn g6_2_an_edited_animation_reloads_without_resetting_the_cursor() {
    let doc = |fps: u32| {
        format!(
            "version=1\n\
             #anim  UiLayout {{ layout_type: Column, width: Px(64), height: Px(64) }}\n    \
             UiSpriteSheet {{ sheet: 0, index: 0 }}\n    \
             UiSpriteAnim {{ first: 0, last: 3, fps: {fps}, mode: Forward, repeats: 0 }}\n"
        )
    };
    let mut rw = ReloadWorld::new("s6_anim", &doc(10));
    let node = rw.find_named("anim").expect("the #anim node spawned at startup");
    assert!(
        rw.world().has_component(node, UiSpriteCursor::component_id()),
        "the startup-spawned animation got its cursor from the hook"
    );

    // Move the cursor away from Default so a reset would be observable.
    rw.app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(node).insert(MovedCursor {
            cursor: UiSpriteCursor { elapsed: 0.03, dir: -1, loops_done: 2, _pad: [0, 0] },
        });
    });

    rw.reload(&doc(24));
    let node = rw.find_named("anim").expect("the survivor kept its name");
    assert_eq!(
        rw.world().get_component::<UiSpriteAnim>(node).map(|v| v.fps),
        Some(24.0),
        "the edited fps must reach the live world"
    );
    assert_eq!(
        rw.world().get_component::<UiSpriteCursor>(node).map(|c| (c.dir, c.loops_done)),
        Some((-1, 2)),
        "re-inserting an edited animation must NOT re-fire on_add — a running flipbook \
         keeps its phase across a hot reload"
    );
}

/// A `Bundle` wrapper so `Commands::insert` can take the DENSE cursor (dense
/// storage suppresses the single-component `Bundle` impl — the
/// `dense_d2_routing::T4DenseBundle` idiom).
#[derive(boyko_macros::Bundle)]
struct MovedCursor {
    cursor: UiSpriteCursor,
}

// ───────────────────────────── G6-3 ────────────────────────────────────────

/// **G6-3** — runtime state is not NAMEABLE from text.
///
/// The narrowed property (S-D20 (2)): a `.ui` file must not NAME a runtime-state
/// component, or give one a value. It is narrower than the sentence this rung
/// used to carry — under the `on_add` ruling a cursor DOES appear beside an
/// authored animation — and the narrowing is written down precisely because G6-3
/// cannot distinguish the wide claim from its negation.
#[test]
fn g6_3_naming_the_cursor_from_text_is_an_unknown_component() {
    let src = "\
version=1
#node  UiLayout { layout_type: Column, width: Px(8), height: Px(8) }
    UiSpriteCursor { elapsed: 5, dir: -1, loops_done: 9 }
";
    let (world, root, report) = lower(src);
    assert!(!report.is_clean(), "UiSpriteCursor must not be dispatchable");
    let hit = report
        .errors
        .iter()
        .find(|(_, _, msg)| msg.contains("unknown component") && msg.contains("UiSpriteCursor"))
        .unwrap_or_else(|| {
            panic!("an unknown-component diagnostic naming the cursor: {:?}", report.errors)
        });
    assert_eq!(hit.0, 3, "the diagnostic points at the offending LINE: {:?}", report.errors);
    // The COLUMN is `body_col` — the first byte INSIDE the component's `{`, which
    // is what `parse_and_insert` has in hand. It locates the component on the
    // line; it does not point at the offending NAME (that would need the span
    // scanner to hand the name's own column down, which no arm of the closed
    // match takes today). Pinned at the measured value rather than at the value a
    // reader would guess, and the difference is recorded in the plan.
    assert_eq!(hit.1, 20, "…and at its COLUMN: {:?}", report.errors);

    // The rejection is per-line and recoverable: the node still spawned, and it
    // carries no cursor (nothing on it added a UiSpriteAnim).
    let e = root.expect("the host node spawned despite the rejected line");
    assert!(
        !world.has_component(e, UiSpriteCursor::component_id()),
        "the rejected line inserted nothing"
    );
}

// ───────────────────────────── G6-4 ────────────────────────────────────────

/// A schedule carrying ONLY the flipbook.
///
/// `ui_render_discovery` is deliberately absent: it lives in `boyko_render`, and
/// the ORDER it pins (`flipbook.before(discovery)`) is S5's gate, not S6's. What
/// S6 owes is that a PARSED node ticks at all.
fn flipbook_only(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_sprite_flipbook);
    b.build(world)
}

/// **G6-4** — an AUTHORED animation TICKS. The rung's whole reason for existing.
///
/// Four assertions in this order, so each red names its own half:
///
/// 1. the LOWERING report is clean — red before the vocabulary lands;
/// 2. `UiSpriteAnim` and `UiSpriteSheet` are PRESENT on the spawned entity;
/// 3. `UiSpriteCursor` is PRESENT — the `on_add` hook's own observable, and the
///    assertion the whole S-D20 (1) ruling turns on. Red between the vocabulary
///    landing and the hook landing (M6-c);
/// 4. `UiSpriteSheet.index` MOVED.
///
/// No `UiSheetTable` is inserted, and the rung's own text was corrected for it:
/// `ui_sprite_flipbook` takes `Res<Time>` and a `Query` over the three
/// components and never reads the table — the table is the RENDER gather's input.
/// Inserting one here would be a dead datum dressed as a precondition.
#[test]
fn g6_4_a_dot_ui_authored_animation_ticks() {
    const FPS: f32 = 10.0;
    let src = "\
version=1
#hero  UiLayout { layout_type: Column, width: Px(64), height: Px(64) }
    UiSpriteSheet { sheet: 0, index: 0 }
    UiSpriteAnim { first: 0, last: 3, fps: 10, mode: Forward, repeats: 0 }
";
    let (mut world, root, report) = lower(src);

    // (1) the lowering is clean — the three names are in the closed match.
    assert!(
        report.is_clean(),
        "a .ui file spelling UiSpriteAnim / UiSpriteSheet must lower clean: {:?}",
        report.errors
    );
    let e = root.expect("the #hero node spawned");

    // (2) the authored components landed.
    assert!(
        world.has_component(e, UiSpriteAnim::component_id()),
        "the parsed node carries UiSpriteAnim"
    );
    assert!(
        world.has_component(e, UiSpriteSheet::component_id()),
        "the parsed node carries UiSpriteSheet"
    );

    // (3) THE RULING'S OBSERVABLE — the cursor no rung authored is present anyway.
    assert_eq!(
        world.get_component::<UiSpriteCursor>(e).copied(),
        Some(UiSpriteCursor::default()),
        "a .ui-authored UiSpriteAnim must get its UiSpriteCursor from the component's \
         on_add hook — without it the flipbook's three-component query never matches and \
         the node renders one frame forever, with no diagnostic"
    );

    // (4) and it TICKS.
    world.insert_resource(Time::default());
    let mut schedule = flipbook_only(&mut world);
    let step = Duration::from_nanos((1_000_000_000.0 / FPS) as u64);
    for _ in 0..3 {
        world.resource_mut::<Time>().advance_with(step);
        schedule.run(&mut world);
    }
    assert_eq!(
        world.get_component::<UiSpriteSheet>(e).map(|s| s.index),
        Some(3),
        "three frame-durations must walk a Forward 0..=3 animation to frame 3"
    );
}
