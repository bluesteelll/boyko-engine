//! UI-ADVANCED rung S0 — measurements §10.8 legs (a)+(d) and §10.3
//! (`docs/UI-PLAN-SPRITES.md` §5).
//!
//! Leg (a) reports probes/node/frame and gather wall-clock at N ∈ {256, 2048}
//! for today's rect-only baseline — the one cost this campaign adds to every
//! node of every frame, measured SEPARATELY from pack+sort (the pack is not
//! run).
//!
//! Legs (d) and §10.3 bracket **`UiUploadSystem::run_dispatcher`** — the
//! two-phase seam — through `EcsMaster::run_system_once`: (d) the static frame
//! under the hoisted D6a gate, which must read ZERO probes (asserted, not just
//! reported); §10.3 the repacks avoided on a static frame AND the unchanged
//! full cost of a changing frame, both reported so the module doc never claims
//! more than the mechanism delivers. **Scope note:** these brackets are
//! device-free — Phase 2's upload cost is DRAW-adjacent host+transfer on the
//! windowed device, is comparable only within one scene, and needs its GPU
//! zone id named before any number is quoted; that half is the owner-run
//! windowed leg, not this file.
//!
//! Instrument: `std::time::Instant` (QPC on Windows, ~100 ns resolution —
//! reported beside the numbers per §5's resolution rule). Run explicitly:
//!
//! ```text
//! cargo test -p boyko-render --test ui_s0_measure -- --ignored --nocapture
//! ```

// Test-harness plumbing only: `Arc<Mutex<…>>` is the established probe for
// smuggling spawned entities out of the one-shot system closure.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};
use std::time::Instant;

use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::access::Access;
use boyko_ecs::ecs::core::system::dispatcher_token::DispatcherToken;
use boyko_ecs::ecs::core::system::system::System;
use boyko_ecs::ecs::core::system::system_meta::SystemMeta;
use boyko_ecs::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_render::{
    gather_ui_nodes, ui_render_discovery, UiGatherScratch, UiNode, UiRenderGeneration,
    UiUploadSystem,
};
use boyko_threadpool::ThreadPoolBuilder;
use boyko_ui::components::{ComputedRect, StackIndex, UiBackground, UiImage, UiRoot};

const WARMUP: usize = 10;
const ITERS: usize = 100;

/// Adapter: runs WARMUP + ITERS gathers inside one dispatcher window, timing
/// each timed gather alone (no pack, no sort, no upload).
struct MeasureGather {
    scratch: UiGatherScratch,
    node_buf: Vec<UiNode>,
    samples_ns: Vec<u128>,
    probes_delta: u64,
    emitted: usize,
    meta: SystemMeta,
}

// SAFETY: EMPTY declared access; the body reads the world only through the
// token's read-only `WorldView`.
unsafe impl System for MeasureGather {
    type Out = ();
    fn name(&self) -> &'static str {
        self.meta.name()
    }
    fn access(&self) -> &Access {
        self.meta.access()
    }
    fn initialize(&mut self, _world: &mut EcsMaster) {}
    /// # Safety
    /// Never run on a worker.
    unsafe fn run_unsafe(&mut self, _cell: UnsafeEcsCell<'_>) -> Self::Out {
        unreachable!("MeasureGather runs only via run_system_once");
    }
    /// # Safety
    /// Reads the world only through the token's read-only `WorldView`.
    unsafe fn run_dispatcher(&mut self, token: DispatcherToken<'_>) -> Self::Out {
        let view = token.world();
        for _ in 0..WARMUP {
            gather_ui_nodes(&view, &mut self.scratch, &mut self.node_buf);
        }
        let probes_before = self.scratch.probes;
        for _ in 0..ITERS {
            self.node_buf.clear();
            let t0 = Instant::now();
            gather_ui_nodes(&view, &mut self.scratch, &mut self.node_buf);
            self.samples_ns.push(t0.elapsed().as_nanos());
        }
        self.probes_delta = self.scratch.probes.wrapping_sub(probes_before);
        self.emitted = self.node_buf.len();
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

/// Builds the baseline world: one `UiRoot` panel with `n - 1` children, every node
/// carrying `ComputedRect` + `UiBackground` + `StackIndex` — plus, when `with_image` is
/// set, a `UiImage` on every node (UI-ADVANCED S3, §10.8 leg (c)).
///
/// Leg (c) differs from leg (a) in EXACTLY that one component, so the probes/node and
/// gather-µs deltas between the two reports are the sprite lane's whole gather cost and
/// nothing else.
///
/// Returns the world and the root's handle (the seam leg mutates it).
fn build_world_with(n: usize, with_image: bool) -> (EcsMaster, Entity) {
    let mut world = EcsMaster::new();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 });
            e.insert(UiBackground { color: 0xFF20_2020, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiRoot);
            if with_image {
                e.insert(UiImage::default());
            }
            e.id()
        };
        *probe.lock().expect("probe") = Some(root);
    });
    let root = sink.lock().expect("probe").expect("root spawned");
    // Children in batches (one command list per run_system keeps the closure
    // simple; setup-time cost is irrelevant to the measurement).
    let children = n - 1;
    world.run_system(move |mut cmds: Commands| {
        for i in 0..children {
            let mut e = cmds.spawn(ComputedRect {
                x: (i % 40) as f32 * 20.0,
                y: (i / 40) as f32 * 20.0,
                w: 18.0,
                h: 18.0,
            });
            e.insert(UiBackground { color: 0xFF40_8040, ..UiBackground::default() });
            e.insert(StackIndex((i % 8) as u32));
            if with_image {
                e.insert(UiImage::default());
            }
            e.set_parent(root);
        }
    });
    (world, root)
}

/// The rect-only baseline (`build_world_with(n, false)`), kept as a name so the seam leg
/// below reads unchanged.
fn build_world(n: usize) -> (EcsMaster, Entity) {
    build_world_with(n, false)
}

fn report_with(n: usize, with_image: bool) {
    let (mut world, _root) = build_world_with(n, with_image);
    let mut sys = MeasureGather {
        scratch: UiGatherScratch::default(),
        node_buf: Vec::new(),
        samples_ns: Vec::new(),
        probes_delta: 0,
        emitted: 0,
        meta: SystemMeta::new("ui_s0::MeasureGather", Tick::new(1)),
    };
    world.run_system_once(&mut sys);

    assert_eq!(sys.emitted, n, "every node is visible in the baseline");
    let probes_per_frame = sys.probes_delta as f64 / ITERS as f64;
    let probes_per_node = probes_per_frame / n as f64;

    sys.samples_ns.sort_unstable();
    let median = sys.samples_ns[ITERS / 2];
    let min = sys.samples_ns[0];
    let max = sys.samples_ns[ITERS - 1];

    let leg = if with_image { "c" } else { "a" };
    println!(
        "§10.8({leg}) N={n}: probes/frame={probes_per_frame:.0} probes/node={probes_per_node:.2} \
         gather min/median/max = {:.1}/{:.1}/{:.1} µs over {ITERS} iters \
         (instrument: Instant/QPC, ~0.1 µs floor)",
        min as f64 / 1000.0,
        median as f64 / 1000.0,
        max as f64 / 1000.0,
    );
}

/// The rect-only report (leg (a)) — the name the baseline test and the seam leg use.
fn report(n: usize) {
    report_with(n, false);
}

/// §10.8 leg (a): the rect-only gather baseline at N ∈ {256, 2048}.
#[test]
#[ignore = "measurement harness - run explicitly with --ignored --nocapture"]
fn measure_gather_baseline() {
    report(256);
    report(2048);
}

/// §10.8 leg (c): the sprite/nine-slice lanes' gather cost (UI-ADVANCED S3, S4).
///
/// # What this leg measures — and the thing it does NOT (found by running it)
///
/// The first run of this leg — at S3, when the list held five pack inputs — reported
/// `probes/node = 6.00` for BOTH the imaged and the image-less world, and wall-clock
/// medians that disagreed in SIGN between N=256 and N=2048. That is not noise hiding a
/// signal; it is the correct answer to the wrong question. The gather probes EVERY pack
/// input on EVERY visited node — a probe that returns `None` is still a probe — so a
/// world where no node carries `UiImage` pays exactly the same probes as one where every
/// node does. Component PRESENCE changes what the pack emits, not what the gather reads.
///
/// So the cost §10.8(c) is actually about is the LIST getting longer, and it is a
/// comparison against the PREVIOUS rung's build, not against a component-less world of
/// the same build:
///
/// * before S3: 4 pack inputs + `Children` = **5.00 probes/node/frame**
/// * after  S3: 5 pack inputs + `Children` = **6.00 probes/node/frame** (+20 %)
/// * after  S4: 6 pack inputs + `Children` = **7.00 probes/node/frame** (+16.7 %) —
///   `UiNineSlice` joined the list
///
/// paid by every node of every changed frame whether or not it is a sprite, and whether or
/// not it is nine-sliced. The printed per-node figure below is derived from
/// `ui_pack_inputs!(count)`, so it stays true as the list grows — and it is the number this
/// paragraph must agree with, so the ladder above gains a row in the same edit that moves
/// the list. MEASURED on the S4 build (2026-08-21): `probes/node = 7.00` in both worlds at
/// both N. The two worlds are still both run, because the probe-HIT vs probe-MISS
/// difference (and the different archetype behind it) is the only part that is not
/// arithmetic — and the run says it is under this instrument's noise at both N.
///
/// Leg (b) (`UiVisual`) does not run until the animation plan lands it in
/// `ui_pack_inputs!` — the plan's §6 says so, and a leg measuring a component that does
/// not exist would be measuring nothing.
#[test]
#[ignore = "measurement harness - run explicitly with --ignored --nocapture"]
fn measure_gather_with_sprite_components() {
    const PACK_INPUTS: usize = boyko_render::ui_pack_inputs!(count);
    println!(
        "§10.8(c) the LIST cost: {PACK_INPUTS} pack inputs + Children = {} probes/node/frame, \
         paid by every node whether or not it carries a sprite (it was {} probes/node/frame \
         with one pack input fewer). Component PRESENCE does not change this number — the \
         two worlds below differ only in probe-hit vs probe-miss.",
        PACK_INPUTS + 1,
        PACK_INPUTS,
    );
    report_with(256, false);
    report_with(256, true);
    report_with(2048, false);
    report_with(2048, true);
}

/// §10.8 leg (d) + §10.3, bracketed at `run_dispatcher` (the two-phase seam):
/// the static frame under the hoisted gate (must read ZERO probes) and the
/// changing frame's unchanged full cost (gather + pack + z-sort into staging;
/// the gate cannot help it and the number says by how much it does not).
fn report_seam(n: usize) {
    let (mut world, root) = build_world(n);
    world.insert_resource(UiRenderGeneration::default());

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    let mut schedule = b.build(&mut world);

    let mut sys = UiUploadSystem::new(1.0);

    // Settle the spawn's own change, then arm the gate with one packed frame.
    for _ in 0..4 {
        schedule.run(&mut world);
    }
    world.run_system_once(&mut sys);
    // RECORDS, not nodes. The two are equal here only because `build_world` is
    // RECT-ONLY — row 1 of S-D12 (1)'s truth table, one background record per
    // node — and writing `n` on its own equated a record count with a node count.
    // A leg-(c) scene containing sprites or nine-slices reds that with nothing
    // wrong, which is the false-red shape S3 already recorded once.
    const RECORDS_PER_RECT_ONLY_NODE: usize = 1;
    assert_eq!(
        sys.staged().len(),
        n * RECORDS_PER_RECT_ONLY_NODE,
        "the settle dispatch packed the scene"
    );

    // ── leg (d): the static frame. ──
    let probes_before = sys.probes();
    let repacks_before = sys.repacks();
    let mut static_ns: Vec<u128> = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        schedule.run(&mut world); // no change ⇒ no bump
        let t0 = Instant::now();
        world.run_system_once(&mut sys);
        static_ns.push(t0.elapsed().as_nanos());
    }
    assert_eq!(
        sys.probes().wrapping_sub(probes_before),
        0,
        "leg (d): the static frame under the hoisted compare reads ZERO probes"
    );
    let static_repacks = sys.repacks().wrapping_sub(repacks_before);
    assert_eq!(static_repacks, 0, "§10.3: every static repack is avoided");
    static_ns.sort_unstable();

    // ── §10.3's other half: the changing frame's FULL cost, unchanged. ──
    // Mutate ONE existing pack input per frame (a same-archetype re-insert on
    // the root — the scene's node count never moves), bump via discovery, then
    // time the dispatch: gather + pack + z-sort of ALL n nodes into staging.
    let mut changed_ns: Vec<u128> = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let dx = i as f32 * 0.25;
        world.run_system(move |mut cmds: Commands| {
            cmds.entity(root)
                .insert(ComputedRect { x: dx, y: 0.0, w: 800.0, h: 600.0 });
        });
        schedule.run(&mut world); // the mutation is a change ⇒ one bump
        let t0 = Instant::now();
        world.run_system_once(&mut sys);
        changed_ns.push(t0.elapsed().as_nanos());
    }
    let changed_repacks = sys.repacks().wrapping_sub(repacks_before);
    assert_eq!(changed_repacks as usize, ITERS, "every changed frame repacks exactly once");
    changed_ns.sort_unstable();

    println!(
        "§10.8(d)+§10.3 N={n}: static dispatch min/median/max = {:.2}/{:.2}/{:.2} µs \
         (probes = 0, repacks avoided = {ITERS}/{ITERS}); \
         changed dispatch min/median/max = {:.1}/{:.1}/{:.1} µs — the FULL \
         gather+pack+sort cost, which the gate does not reduce \
         (instrument: Instant/QPC, ~0.1 µs floor; Phase 2 upload cost is the \
         owner-run windowed leg — DRAW-adjacent host+transfer, same-scene \
         comparisons only, GPU zone id named first)",
        static_ns[0] as f64 / 1000.0,
        static_ns[ITERS / 2] as f64 / 1000.0,
        static_ns[ITERS - 1] as f64 / 1000.0,
        changed_ns[0] as f64 / 1000.0,
        changed_ns[ITERS / 2] as f64 / 1000.0,
        changed_ns[ITERS - 1] as f64 / 1000.0,
    );
}

/// §10.8 leg (d) + §10.3 at N ∈ {256, 2048}, headless.
#[test]
#[ignore = "measurement harness - run explicitly with --ignored --nocapture"]
fn measure_seam_static_and_changed() {
    report_seam(256);
    report_seam(2048);
}
