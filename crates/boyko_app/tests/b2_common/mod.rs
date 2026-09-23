//! Shared ground for rung B2's headless gates (G-GRAPH, E1, G-RES, the id census): the two app
//! compositions the plan names, the device-free residents they need, and the schedule steal that
//! lets a test read a built schedule's system inventory without `App::finish()`.
//!
//! # App (a) and app (b), at today's counts
//!
//! * **(a)** `EnginePlugins` alone, plus the two world residents the windowed runner would insert
//!   (a device-inert `Assets<MeshGpu>` and an `Assets<Material>`, the recipe of
//!   `host_frame_zero_draws_every_mesh.rs`) and a startup spawn of [`MESHES`] `MeshBundle`s.
//! * **(b)** (a) plus today's UI plugin composition. The design's `UiPlugins` (ED19) does not
//!   exist until rung UI2, so (b) composes the plugins that do: `InputPlugin::<TestAction>` (the UI
//!   dispatch reads `ActionState<A>`; a game composes its own input), `UiInteractionPlugin`,
//!   `UiBindingPlugin`, `UiWidgetsPlugin`, `UiAnimationPlugin`, and `UiPlugin` loading the committed
//!   fixture [`UI_FIXTURE`] with hot-reload OFF (the watch system polls file metadata on wall-clock
//!   time, and E1 needs a deterministic frame). `ProfilingOverlayPlugin` is not composed: ED19 does
//!   not name it. G-GRAPH (b) and E1 re-derive when UI2 lands `UiPlugins` (B2 plan correction PC3).
//!
//! No UI plugin registers layout today (`ui_layout_discovery` / `ui_layout_apply` are added only
//! inside `layout.rs`'s own tests), so app (b) has no layout pass; that is today's count, not an
//! omission of this harness.
//!
//! # One app per process
//!
//! `EnginePlugins` can be built once per process (`LightingPlugin` installs process-global
//! component hooks — `ddgi_host_hook_registration.rs`), so every binary that includes this module
//! builds exactly one app.
//!
//! # The pool is W = [`THREADS`]
//!
//! Fixed, so E1's first-touch allowance and G-GRAPH's builder pool are the same in every run.

// Each including binary uses a different subset of these helpers.
#![allow(dead_code)]

pub mod census_scan;
pub mod graph_pins;

use std::sync::Arc;
use std::time::Duration;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::profiling::SYSTEM_ZONES_COMPILED;
use boyko_ecs::ecs::core::schedule::schedule::Schedule;
use boyko_ecs::ecs::core::schedule::schedule_builder::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::{FixedTime, Time};
use boyko_input::{Actionlike, InputMap, InputPlugin};
use boyko_math::Vec3;
use boyko_render::{Material, MeshBundle, MeshGpu};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::Transform;
use boyko_scene::render_caps::MeshHandle;
use boyko_ui::UiPlugin;
use boyko_ui::animation::UiAnimationPlugin;
use boyko_ui::interaction::{UiBindingPlugin, UiInteractionPlugin};
use boyko_ui::resources::UiViewport;
use boyko_ui::widgets::UiWidgetsPlugin;

/// Worker threads in every B2 app's pool.
pub const THREADS: usize = 4;
/// Meshes the startup spawns — the golden `taa_armed` scene's count, as in
/// `host_frame_zero_draws_every_mesh.rs`.
pub const MESHES: usize = 7;
/// App (b)'s committed UI document: a root panel, a text, a button and a bar (5 nodes).
pub const UI_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/b2_common/app_b.ui");
/// The frame delta every B2 frame is driven with.
pub const FRAME: Duration = Duration::from_millis(16);
/// The env knob that arms a binary's red-first canary (test-only; no engine source reads it).
pub const CANARY_ENV: &str = "BOYKO_B2_CANARY";

/// App (b)'s action enum. The UI dispatch lowers an `OnClick(i)` edge into `ActionState<A>`; the
/// fixture's button carries `OnClick(0)`.
#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TestAction {
    /// Index 0 — the fixture button's action.
    Confirm,
    /// Index 1.
    Cancel,
}

/// The armed canary, if any.
pub fn canary() -> Option<String> {
    std::env::var(CANARY_ENV).ok().filter(|v| !v.is_empty())
}

/// A fresh app on a W = [`THREADS`] pool.
pub fn new_app() -> App {
    App::with_threads(THREADS)
}

/// A device-inert `MeshGpu`: null buffers, a unit cube's index count. Sound because no device
/// exists in these processes and the gathers read only `index_count` / `index_type`.
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer {
        buffer: VkBuffer::NULL,
        offset: 0,
        size: 0,
        mapped: None,
        block: 0,
    };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 36,
        index_type: IndexType::Uint16,
        vertex_count: 8,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
        local_min: [-0.5; 3],
        local_max: [0.5; 3],
    }
}

/// The two world residents the windowed runner inserts before `finish()` and the gathers read.
fn insert_residents(app: &mut App) {
    let mut meshes = Assets::<MeshGpu>::with_reserved(1);
    let cube = meshes.add(dummy_mesh_gpu());
    assert_eq!(
        cube.index(),
        0,
        "precondition: the spawned `MeshHandle(0)` names the one mesh added"
    );
    app.world_mut().insert_non_send_resource(meshes);
    let mut materials = Assets::<Material>::with_reserved(1);
    materials.add(Material::default());
    app.insert_resource(materials);
}

/// Spawns `n` meshes from a startup system, the way a host scene does.
fn spawn_meshes(app: &mut App, n: usize) {
    app.add_startup_system(move |mut cmds: Commands| {
        for i in 0..n {
            let pose = Transform::from_translation(Vec3::new(i as f32 * 2.0, 0.0, -5.0));
            cmds.spawn(MeshBundle::new(MeshHandle(0), pose));
        }
    });
}

/// App (a): `EnginePlugins` + the residents + `meshes` meshes.
pub fn app_a(app: &mut App, meshes: usize) {
    app.add_plugins(EnginePlugins::window("b2", 64, 64));
    insert_residents(app);
    spawn_meshes(app, meshes);
}

/// App (b): app (a)'s composition plus today's UI plugins (see the module doc). `ui_path = None`
/// composes `UiPlugin` with no document (E1's `e1_no_ui` canary).
pub fn app_b(app: &mut App, ui_path: Option<&'static str>, meshes: usize) {
    app.add_plugins(EnginePlugins::window("b2", 64, 64));
    app.add_plugin(InputPlugin::<TestAction>::new(InputMap::builder().build()));
    app.add_plugin(UiInteractionPlugin::<TestAction>::new());
    app.add_plugin(UiBindingPlugin::new());
    app.add_plugin(UiWidgetsPlugin::new());
    app.add_plugin(UiAnimationPlugin);
    let ui = UiPlugin::new().with_hot_reload(false);
    app.add_plugin(match ui_path {
        Some(p) => ui.with_ui_path(p),
        None => ui,
    });
    insert_residents(app);
    // A UI resident no plugin inserts today: the host is to set it on surface create / resize
    // (`UiViewport`'s doc), and no production writer exists until rung HO3 derives it on the window
    // entity. The test inserts the window's 64 × 64 at scale 1.
    app.insert_resource(UiViewport {
        width: 64.0,
        height: 64.0,
        scale_factor: 1.0,
        generation: 0,
    });
    spawn_meshes(app, meshes);
}

/// The fixture parses clean — a UI document that silently lowered to nothing would make every
/// app-(b) reading a reading of an app without UI.
pub fn assert_fixture_parses_clean() {
    let src = std::fs::read_to_string(UI_FIXTURE)
        .unwrap_or_else(|e| panic!("B2: the UI fixture {UI_FIXTURE} is unreadable: {e}"));
    let tree = boyko_ui::parse_ui(&src);
    assert!(
        tree.report.is_clean(),
        "B2: the UI fixture does not parse clean: {:?}",
        tree.report.errors
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The schedule steal — a built schedule without `finish()`
// ═══════════════════════════════════════════════════════════════════════════

/// Main and Fixed, built exactly as `App::finish()` builds them.
pub struct Built {
    pub main: Schedule,
    pub fixed: Schedule,
}

/// Takes the staged Main and Fixed builders out of `app` as the LAST configuration step and
/// builds them on `app.world_mut()` in `finish()`'s order: `Time` / `FixedTime` inserted if
/// absent, then Main, then Fixed.
///
/// `App` hands out no built schedule (`builder` / `schedule` are private). `add_systems_cfg`
/// runs its closure immediately on the staged `&mut ScheduleBuilder`, and every plugin has
/// already built by now, so the closure sees every system; `mem::replace` swaps in an empty
/// builder the app never builds (the caller must not call `finish()` afterwards).
///
/// ⚠ `add_systems_cfg_in(Fixed, ..)` CREATES a Fixed builder when none was staged, where
/// `finish()` would build no Fixed schedule at all; a `FIXED_* = []` pin therefore cannot tell
/// "no Fixed schedule" from "an empty one". Both B2 apps stage Fixed systems (`EnginePlugins`
/// registers the GPU-transform pack in `FixedSet::Snapshot`), which [`fixed_len_is_positive`]
/// asserts, so the ambiguity does not arise today.
pub fn steal_and_build(app: &mut App) -> Built {
    let pool = Arc::clone(app.pool());
    let mut main: Option<ScheduleBuilder> = None;
    let mut fixed: Option<ScheduleBuilder> = None;
    app.add_systems_cfg(|b| {
        main = Some(std::mem::replace(
            b,
            ScheduleBuilder::new(Arc::clone(&pool)),
        ));
    });
    app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
        fixed = Some(std::mem::replace(
            b,
            ScheduleBuilder::new(Arc::clone(&pool)),
        ));
    });
    let world: &mut EcsMaster = app.world_mut();
    if !world.contains_resource::<Time>() {
        world.insert_resource(Time::default());
    }
    if !world.contains_resource::<FixedTime>() {
        world.insert_resource(FixedTime::default());
    }
    let main = main
        .expect("invariant: add_systems_cfg ran its closure")
        .build(world);
    let fixed = fixed
        .expect("invariant: add_systems_cfg_in ran its closure")
        .build(world);
    Built { main, fixed }
}

/// `Fixed` is non-empty in both apps (see [`steal_and_build`]'s ⚠).
pub fn fixed_len_is_positive(b: &Built) -> bool {
    !b.fixed.is_empty()
}

/// The first segment of this test binary's module path — the crate name every test-local type
/// (e.g. [`TestAction`]) carries inside a system's `type_name`.
fn this_crate() -> &'static str {
    module_path!()
        .split("::")
        .next()
        .expect("invariant: module_path! is non-empty")
}

/// A system name with the including binary's crate name replaced by `$test`, so one pinned name
/// holds in every binary that includes this module.
pub fn normalize(name: &str) -> String {
    name.replace(&format!("{}::", this_crate()), "$test::")
}

/// A schedule's system names as a sorted multiset `(name, count)`.
///
/// RED (panic) if the build folds system zones out: then `system_zones()` yields nothing, and an
/// empty inventory would read as an empty schedule.
// A runtime RED, not a `const` assert: the constant is a build-profile axis (`BOYKO_PROFILE`), and
// a compile-time assert would also fail the build of every binary that includes this module without
// reading names (E1, G-RES) under a profile that folds system zones out.
#[allow(clippy::assertions_on_constants)]
pub fn inventory(s: &Schedule) -> Vec<(String, u32)> {
    assert!(
        SYSTEM_ZONES_COMPILED,
        "G-GRAPH: system names are unavailable under this BOYKO_PROFILE (SYSTEM_ZONES_COMPILED is \
         false) — the gate cannot read the inventory, and it does not skip"
    );
    let names: Vec<String> = s.system_zones().map(|(n, _)| normalize(n)).collect();
    assert_eq!(
        names.len(),
        s.len(),
        "G-GRAPH: system_zones() yielded {} names for a schedule of {} systems",
        names.len(),
        s.len()
    );
    let mut sorted = names;
    sorted.sort();
    let mut out: Vec<(String, u32)> = Vec::new();
    for n in sorted {
        match out.last_mut() {
            Some((last, c)) if *last == n => *c += 1,
            _ => out.push((n, 1)),
        }
    }
    out
}

/// The post-topological order (printed, never pinned: it is a packing-dependent trajectory).
pub fn order(s: &Schedule) -> Vec<String> {
    s.system_zones().map(|(n, _)| normalize(n)).collect()
}

/// `(name, count)` pairs from a pinned const.
pub fn owned(pins: &[(&str, u32)]) -> Vec<(String, u32)> {
    pins.iter().map(|(n, c)| ((*n).to_string(), *c)).collect()
}

/// Prints a paste-ready `const` block for a measured inventory.
pub fn paste_ready(name: &str, inv: &[(String, u32)]) {
    println!("const {name}: &[(&str, u32)] = &[");
    for (n, c) in inv {
        println!("    ({n:?}, {c}),");
    }
    println!("];");
}

/// `measured == pinned`, with every difference named.
pub fn diff(label: &str, measured: &[(String, u32)], pinned: &[(String, u32)]) -> Vec<String> {
    let mut out = Vec::new();
    for (n, c) in measured {
        let p = pinned
            .iter()
            .find(|(pn, _)| pn == n)
            .map_or(0, |(_, pc)| *pc);
        if p != *c {
            out.push(format!("{label}: `{n}` measured ×{c}, pinned ×{p}"));
        }
    }
    for (n, p) in pinned {
        if !measured.iter().any(|(mn, _)| mn == n) {
            out.push(format!("{label}: `{n}` pinned ×{p}, measured ×0"));
        }
    }
    out
}

/// The multiset difference `b − a`.
pub fn minus(b: &[(String, u32)], a: &[(String, u32)]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (n, c) in b {
        let ac = a.iter().find(|(an, _)| an == n).map_or(0, |(_, x)| *x);
        if *c > ac {
            out.push((n.clone(), c - ac));
        }
    }
    out
}

/// The in-binary extraction self-test: a separate two-system builder — one concurrent system and
/// one `fn(&mut EcsMaster)` — must yield both names. Proves the inventory reads real names rather
/// than an empty or placeholder list.
pub fn extraction_self_test(pool: &Arc<boyko_threadpool::ThreadPool>) {
    fn b2_self_test_concurrent(_t: boyko_ecs::ecs::core::system::Res<Time>) {}
    fn b2_self_test_exclusive(_w: &mut EcsMaster) {}
    let mut world = EcsMaster::new();
    world.insert_resource(Time::default());
    let mut b = ScheduleBuilder::new(Arc::clone(pool));
    b.add_system(b2_self_test_concurrent);
    b.add_system(b2_self_test_exclusive);
    let s = b.build(&mut world);
    let names = inventory(&s);
    let has = |suffix: &str| names.iter().any(|(n, c)| n.ends_with(suffix) && *c == 1);
    assert!(
        s.len() == 2 && has("b2_self_test_concurrent") && has("b2_self_test_exclusive"),
        "G-GRAPH self-test: a two-system schedule yielded {names:?}"
    );
    println!("G-GRAPH self-test: a two-system schedule yields both names: {names:?}");
}

/// The red-first knob `g_graph_exclusive`: one `fn(&mut EcsMaster)` in Main, added before the
/// steal. It reds through the NAME inventory, not through an exclusivity check — a concurrent
/// canary would red identically; the discriminating control needs the kernel seam (PC2).
pub fn maybe_add_exclusive_canary(app: &mut App) {
    fn b2_canary_exclusive(_w: &mut EcsMaster) {}
    if canary().as_deref() == Some("g_graph_exclusive") {
        println!("CANARY ARMED: g_graph_exclusive — one fn(&mut EcsMaster) added to Main");
        app.add_systems_cfg(|b| {
            b.add_system(b2_canary_exclusive);
        });
    }
}
