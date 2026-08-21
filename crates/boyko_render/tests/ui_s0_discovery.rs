//! UI-ADVANCED rung S0 — CPU gates G0-1 and G0-4 (`docs/UI-PLAN-SPRITES.md`).
//!
//! **G0-1** — the discovery filter and the gather read list are ONE spelling
//! (`ui_pack_inputs!`), and the discovery bumps [`UiRenderGeneration`] exactly
//! once per changed frame: a world with one node, each pack-input component
//! mutated in turn, the generation asserted to bump EXACTLY once per mutation
//! and ZERO times on an unrelated-component mutation. (The compile-level half
//! of G0-1 is the macro itself: deleting a component from
//! `__ui_pack_inputs_list!` changes the gather's read-tuple arity and the crate
//! stops building — red mutation M0-c.)
//!
//! **G0-4** — the gather's DFS carries the inherited clip and its pre-order is
//! paint order: a two-root tree with a clip at the middle level, asserting
//! (a) the leaf's packed clip IS the ancestor's, and (b) the gather's emission
//! order agrees with the interaction hit-test's paint order
//! (`collect_candidates`, observed through the public `ui_focus_system`: at a
//! probe point covered by several nodes, the hit-test hovers the LAST of them
//! in paint order — sampled at one point per ADJACENT pair of the expected
//! sequence, which pins the total order).
//!
//! CPU-only: no GPU, no window. The gather runs against a read-only
//! [`WorldView`] minted through `EcsMaster::run_system_once` + a tiny adapter
//! system (the sanctioned dispatcher-solo route).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles out of the `Send + Sync` one-shot system
// closure. Not engine code — compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::dispatcher_token::DispatcherToken;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_input::PhysicalInput;
use boyko_render::{gather_ui_nodes, ui_render_discovery, UiGatherScratch, UiNode, UiRenderGeneration};
use boyko_ui::components::{ComputedClip, ComputedRect, StackIndex, UiBackground, UiLayout, UiRoot};
use boyko_ui::interaction::components::Interaction;
use boyko_ui::interaction::focus::{
    ui_focus_system, UiInputFocus, UiInteractionConfig, UiInteractionScratch, UiPointerState,
};
use boyko_ui::interaction::plugin::{TAG_UI_FOCUSED, TAG_UI_HOVERED, TAG_UI_PRESSED};
use boyko_ui::resources::UiViewport;

// ───────────────────────── shared plumbing ─────────────────────────────────

/// Adapter system: mints the dispatcher token via `run_system_once` and runs
/// the canonical gather against its read-only `WorldView`. The buffers are
/// OWNED (moved in, moved back out) because `System: Send + Sync + 'static`
/// forbids a borrowing adapter.
struct GatherOnce {
    scratch: UiGatherScratch,
    node_buf: Vec<UiNode>,
    meta: SystemMeta,
}

impl GatherOnce {
    fn new(scratch: UiGatherScratch) -> Self {
        Self {
            scratch,
            node_buf: Vec::new(),
            meta: SystemMeta::new("ui_s0::GatherOnce", Tick::new(1)),
        }
    }
}

// SAFETY: EMPTY declared access; the only work happens in `run_dispatcher`,
// which touches the world exclusively through the read-only `WorldView` (the
// token's blessed `&self` projection) — no aliasing reference is minted.
unsafe impl System for GatherOnce {
    type Out = ();

    fn name(&self) -> &'static str {
        self.meta.name()
    }
    fn access(&self) -> &Access {
        self.meta.access()
    }
    fn initialize(&mut self, _world: &mut EcsMaster) {}

    /// # Safety
    /// Vacuous — never dispatched to a worker (`run_system_once` drives
    /// `run_dispatcher` directly).
    unsafe fn run_unsafe(&mut self, _cell: UnsafeEcsCell<'_>) -> Self::Out {
        unreachable!("GatherOnce runs only via run_system_once -> run_dispatcher");
    }

    /// # Safety
    /// Reads the world only through the token's read-only `WorldView`.
    unsafe fn run_dispatcher(&mut self, token: DispatcherToken<'_>) -> Self::Out {
        let view = token.world();
        gather_ui_nodes(&view, &mut self.scratch, &mut self.node_buf);
    }

    fn apply(&mut self, _world: &mut EcsMaster) {}
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.set_change_ticks(last_run, this_run);
    }
    fn check_change_tick(&mut self, current: Tick) {
        self.meta.clamp_change_ticks(current);
    }
}

/// Runs the canonical gather once, returning the emitted nodes in order.
fn gather(world: &mut EcsMaster, scratch: &mut UiGatherScratch) -> Vec<UiNode> {
    let mut sys = GatherOnce::new(std::mem::take(scratch));
    world.run_system_once(&mut sys);
    *scratch = sys.scratch;
    sys.node_buf
}

// ───────────────────────── G0-1 ────────────────────────────────────────────

fn generation(world: &EcsMaster) -> u64 {
    world.resource::<UiRenderGeneration>().generation
}

/// G0-1 (behavioural half): each pack-input mutation bumps the generation
/// EXACTLY once; an unrelated-component mutation bumps it ZERO times.
#[test]
fn g0_1_discovery_bumps_exactly_once_per_pack_input_mutation() {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());

    // One node carrying all four pack inputs PLUS the unrelated component
    // (spawned up front so the later mutations are same-archetype re-inserts,
    // never archetype moves).
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
        e.insert(UiBackground { color: 0xFF00_00FF, ..UiBackground::default() });
        e.insert(ComputedClip { x: 0.0, y: 0.0, w: 5.0, h: 5.0 });
        e.insert(StackIndex(0));
        e.insert(UiLayout::default());
        e.insert(UiRoot);
        *probe.lock().expect("probe") = Some(e.id());
    });
    let node = sink.lock().expect("probe").expect("spawned node");

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    let mut schedule: Schedule = b.build(&mut world);

    // Settle: the spawn itself is a change; run until two consecutive frames
    // hold the generation (bounded — a runaway bump is M0-d's signature).
    let mut settled = 0;
    for _ in 0..8 {
        let before = generation(&world);
        schedule.run(&mut world);
        if generation(&world) == before {
            settled += 1;
            if settled == 2 {
                break;
            }
        } else {
            settled = 0;
        }
    }
    assert_eq!(settled, 2, "discovery must go quiet after the spawn settles (M0-d red: it never does)");

    // Each pack input in turn: exactly one bump on the mutation frame, zero on
    // the frame after.
    fn mutate_pack_input(world: &mut EcsMaster, node: Entity, which: usize) {
        world.run_system(move |mut cmds: Commands| {
            match which {
                0 => {
                    cmds.entity(node).insert(ComputedRect { x: 1.0, y: 2.0, w: 20.0, h: 20.0 });
                }
                1 => {
                    cmds.entity(node).insert(UiBackground {
                        color: 0xFF00_FF00,
                        ..UiBackground::default()
                    });
                }
                2 => {
                    cmds.entity(node).insert(ComputedClip { x: 1.0, y: 1.0, w: 8.0, h: 8.0 });
                }
                _ => {
                    cmds.entity(node).insert(StackIndex(7));
                }
            };
        });
    }

    for (which, name) in ["ComputedRect", "UiBackground", "ComputedClip", "StackIndex"]
        .iter()
        .enumerate()
    {
        let g0 = generation(&world);
        mutate_pack_input(&mut world, node, which);
        schedule.run(&mut world);
        assert_eq!(
            generation(&world),
            g0 + 1,
            "a {name} mutation must bump the generation EXACTLY once on its frame"
        );
        schedule.run(&mut world);
        assert_eq!(
            generation(&world),
            g0 + 1,
            "the frame AFTER a {name} mutation must not bump again"
        );
    }

    // The unrelated component: a `UiLayout` re-insert is a change the pack does
    // not read — the generation must hold.
    let g0 = generation(&world);
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(node).insert(UiLayout::default());
    });
    schedule.run(&mut world);
    assert_eq!(
        generation(&world),
        g0,
        "an unrelated-component mutation must bump the generation ZERO times"
    );
}

// ───────────────────────── G0-4 ────────────────────────────────────────────

/// Node colors — unique per node, so the gather's emission order is readable
/// off the packed records.
const C_ROOT_A: u32 = 0xFF00_00A1;
const C_CHILD1: u32 = 0xFF00_00B2;
const C_LEAF: u32 = 0xFF00_00C3;
const C_CHILD2: u32 = 0xFF00_00D4;
const C_ROOT_B: u32 = 0xFF00_00E5;

struct Tree {
    world: EcsMaster,
    root_a: Entity,
    child1: Entity,
    leaf: Entity,
    child2: Entity,
    root_b: Entity,
}

/// The G0-4 fixture: two roots, a clip at the middle level, every node
/// interactive (so the hit-test records all of them) AND visible (so the
/// gather emits all of them).
///
/// ```text
/// rootA (0,0,100,100)                 color A1
///   child1 (10,10,50,50) clip=own     color B2   <- the middle-level clip
///     leaf (30,30,60,60)              color C3   <- INHERITS child1's clip
///   child2 (35,35,10,10)              color D4
/// rootB (35,35,4,4)                   color E5   <- second root, higher id
/// ```
///
/// Expected paint order (roots by id, DFS children in document order):
/// `rootA, child1, leaf, child2, rootB`.
fn build_tree() -> Tree {
    let mut world = EcsMaster::new();

    // Interaction resources, exactly as the plugin wires them.
    let hovered_tag = world.register_enable_tag(TAG_UI_HOVERED);
    let pressed_tag = world.register_enable_tag(TAG_UI_PRESSED);
    let focused_tag = world.register_enable_tag(TAG_UI_FOCUSED);
    world.insert_resource(UiInteractionConfig { hovered_tag, pressed_tag, focused_tag });
    world.insert_resource(UiPointerState::default());
    world.insert_resource(UiInputFocus::default());
    world.insert_resource(UiInteractionScratch::default());
    world.insert_resource(PhysicalInput::default());
    world.insert_resource(UiViewport { width: 1000.0, height: 800.0, scale_factor: 1.0, generation: 0 });

    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut v = probe.lock().expect("probe");

        let root_a = {
            let mut e = cmds.spawn(Interaction::None);
            e.insert(ComputedRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
            e.insert(UiBackground { color: C_ROOT_A, ..UiBackground::default() });
            e.insert(UiRoot);
            e.id()
        };
        v.push(root_a);
        let child1 = {
            let mut e = cmds.spawn(Interaction::None);
            e.insert(ComputedRect { x: 10.0, y: 10.0, w: 50.0, h: 50.0 });
            e.insert(UiBackground { color: C_CHILD1, ..UiBackground::default() });
            e.insert(ComputedClip { x: 10.0, y: 10.0, w: 50.0, h: 50.0 });
            e.set_parent(root_a);
            e.id()
        };
        v.push(child1);
        let leaf = {
            let mut e = cmds.spawn(Interaction::None);
            e.insert(ComputedRect { x: 30.0, y: 30.0, w: 60.0, h: 60.0 });
            e.insert(UiBackground { color: C_LEAF, ..UiBackground::default() });
            e.set_parent(child1);
            e.id()
        };
        v.push(leaf);
        let child2 = {
            let mut e = cmds.spawn(Interaction::None);
            e.insert(ComputedRect { x: 35.0, y: 35.0, w: 10.0, h: 10.0 });
            e.insert(UiBackground { color: C_CHILD2, ..UiBackground::default() });
            e.set_parent(root_a);
            e.id()
        };
        v.push(child2);
        let root_b = {
            let mut e = cmds.spawn(Interaction::None);
            e.insert(ComputedRect { x: 35.0, y: 35.0, w: 4.0, h: 4.0 });
            e.insert(UiBackground { color: C_ROOT_B, ..UiBackground::default() });
            e.insert(UiRoot);
            e.id()
        };
        v.push(root_b);
    });
    let ids = sink.lock().expect("probe").clone();
    let (root_a, child1, leaf, child2, root_b) = (ids[0], ids[1], ids[2], ids[3], ids[4]);
    assert!(
        root_a.id().0 < root_b.id().0,
        "fixture invariant: rootA's entity id sorts before rootB's (cross-root order)"
    );
    Tree { world, root_a, child1, leaf, child2, root_b }
}

/// Runs one `ui_focus_system` frame with the cursor at `(x, y)` and returns the
/// hovered entity among the fixture nodes (`None` if none hovered).
fn hovered_at(t: &mut Tree, x: f64, y: f64) -> Option<Entity> {
    t.world.resource_mut::<PhysicalInput>().cursor_pos = [x, y];
    ui_focus_system(&mut t.world);
    [t.root_a, t.child1, t.leaf, t.child2, t.root_b]
        .into_iter()
        .find(|&e| {
            matches!(
                t.world.get_component::<Interaction>(e),
                Some(Interaction::Hovered) | Some(Interaction::Pressed)
            )
        })
}

/// G0-4: the DFS carries the inherited clip, and the gather's pre-order IS the
/// hit-test's paint order.
#[test]
fn g0_4_gather_preorder_is_paint_order_and_clip_inherits() {
    let mut t = build_tree();

    // ── The gather's emission order + the packed clips. ──
    let mut scratch = UiGatherScratch::default();
    let nodes = gather(&mut t.world, &mut scratch);
    let colors: Vec<u32> = nodes.iter().map(|n| n.input.color).collect();
    assert_eq!(
        colors,
        [C_ROOT_A, C_CHILD1, C_LEAF, C_CHILD2, C_ROOT_B],
        "the gather's pre-order must be: rootA, child1, leaf, child2, rootB"
    );

    // The clip column: rootA/child2/rootB unclipped, child1 carries its own,
    // and the LEAF (which has no clip of its own) inherits child1's — the
    // inherited-clip-on-the-DFS-stack claim.
    let clip_of = |color: u32| -> Option<[f32; 4]> {
        nodes.iter().find(|n| n.input.color == color).expect("node emitted").input.clip
    };
    assert_eq!(clip_of(C_ROOT_A), None, "rootA is unclipped");
    assert_eq!(clip_of(C_CHILD1), Some([10.0, 10.0, 50.0, 50.0]), "child1 packs its own clip");
    assert_eq!(
        clip_of(C_LEAF),
        Some([10.0, 10.0, 50.0, 50.0]),
        "the leaf's packed clip IS the ancestor's (inherited through the DFS stack)"
    );
    assert_eq!(clip_of(C_CHILD2), None, "child2 is unclipped (no inheritance from a sibling)");
    assert_eq!(clip_of(C_ROOT_B), None, "rootB is unclipped");

    // The probe counter moved (5 probes per visited node: 4 pack inputs +
    // Children), and the emission order above required probes to happen.
    assert_eq!(
        scratch.probes,
        5 * 5,
        "five visited nodes at five probes each (four pack inputs + Children)"
    );

    // ── The paint-order oracle: `collect_candidates` through `ui_focus_system`.
    //    At a point covered by several nodes (equal StackIndex), the hit-test
    //    hovers the one with the GREATEST paint_seq — the LAST in paint order.
    //    One probe point per ADJACENT pair of the expected sequence pins the
    //    total order the gather claims. ──
    let root_a = t.root_a;
    let child1 = t.child1;
    let leaf = t.leaf;
    let child2 = t.child2;
    let root_b = t.root_b;

    // (15,15): rootA + child1 (leaf starts at 30) -> child1 paints after rootA.
    assert_eq!(hovered_at(&mut t, 15.0, 15.0), Some(child1), "child1 paints after rootA");
    // (32,32): rootA + child1 + leaf (child2 starts at 35; rootB too, but its
    // 4x4 rect ends at 39 — (32,32) is inside it? 35..39 excludes 32 — no)
    // -> leaf paints after child1.
    assert_eq!(hovered_at(&mut t, 32.0, 32.0), Some(leaf), "leaf paints after child1");
    // (41,41): rootA + child1 + leaf + child2 (rootB's rect ends at 39)
    // -> child2 paints after leaf.
    assert_eq!(hovered_at(&mut t, 41.0, 41.0), Some(child2), "child2 paints after leaf");
    // (36,36): ALL five -> rootB (the later root) paints last.
    assert_eq!(hovered_at(&mut t, 36.0, 36.0), Some(root_b), "rootB paints after child2");

    // Sanity: a point only rootA covers.
    assert_eq!(hovered_at(&mut t, 80.0, 5.0), Some(root_a), "rootA alone covers (80,5)");
}
