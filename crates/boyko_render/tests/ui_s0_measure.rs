//! UI-ADVANCED rung S0 — measurement §10.8 leg (a): the gather alone
//! (`docs/UI-PLAN-SPRITES.md` §5).
//!
//! Reports probes/node/frame and gather wall-clock at N ∈ {256, 2048} for
//! today's rect-only baseline — the one cost this campaign adds to every node
//! of every frame, measured SEPARATELY from pack+sort (the pack is not run).
//!
//! Leg (d) — the static frame with the D6a compare hoisted, which must read
//! ZERO probes — is specified against `host_upload_frame_from_world` and is
//! BLOCKED by the seam-callability defect recorded in `docs/OPEN-QUESTIONS.md`
//! (entry 2026-08-21). Its zero-probe property is meanwhile pinned at the CPU
//! level by G0-2's counter design, not measured here.
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

use boyko_render::{gather_ui_nodes, UiGatherScratch, UiNode};
use boyko_ui::components::{ComputedRect, StackIndex, UiBackground, UiRoot};

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

/// Builds the rect-only baseline: one `UiRoot` panel with `n - 1` children,
/// every node carrying `ComputedRect` + `UiBackground` + `StackIndex`.
fn build_world(n: usize) -> EcsMaster {
    let mut world = EcsMaster::new();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let root = {
            let mut e = cmds.spawn(ComputedRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 });
            e.insert(UiBackground { color: 0xFF20_2020, ..UiBackground::default() });
            e.insert(StackIndex(0));
            e.insert(UiRoot);
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
            e.set_parent(root);
        }
    });
    world
}

fn report(n: usize) {
    let mut world = build_world(n);
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

    println!(
        "§10.8(a) N={n}: probes/frame={probes_per_frame:.0} probes/node={probes_per_node:.2} \
         gather min/median/max = {:.1}/{:.1}/{:.1} µs over {ITERS} iters \
         (instrument: Instant/QPC, ~0.1 µs floor)",
        min as f64 / 1000.0,
        median as f64 / 1000.0,
        max as f64 / 1000.0,
    );
}

/// §10.8 leg (a): the rect-only gather baseline at N ∈ {256, 2048}.
#[test]
#[ignore = "measurement harness - run explicitly with --ignored --nocapture"]
fn measure_gather_baseline() {
    report(256);
    report(2048);
}
