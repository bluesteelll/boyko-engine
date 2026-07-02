//! Zero-per-frame-allocation gate for the world-UI systems after the P5 retained-
//! buffer fix: [`ui_world_pick_system`] (and, by the shared `UiWorldScratch`,
//! [`ui_world_project_system`] / [`ui_world_visibility_system`]) must allocate
//! NOTHING on a warmed steady frame.
//!
//! # Why this proves the fix
//!
//! Before the fix each frame built a FRESH `Vec` from `query_entities(...)` (the
//! root walk) plus a fresh `Vec<PickBound>` snapshot inside `collect_bounds` — a
//! per-frame heap allocation on an every-frame system. The fix moves those buffers
//! onto the retained [`UiWorldScratch`] resource, cleared-and-refilled through the
//! allocation-free `query_entities_buf`. A warmed frame (buffers already at
//! high-water) must therefore touch the heap zero times.
//!
//! # Method (mirrors `boyko_input/tests/zero_alloc.rs`)
//!
//! A pass-through global allocator counts `alloc`/`realloc` calls on the current
//! thread while a thread-local arm flag is set. The system is BUILT ONCE via
//! `IntoSystem::into_system` and driven with `run_cached_system` (so no per-call
//! `into_system` construction lands in the measured window), warmed to high-water,
//! then the counter is armed for a single steady `run_cached_system` and asserted
//! to be zero. A thread-local counter (not a process-global atomic) attributes
//! allocations to exactly this thread's measured window, sound under the default
//! multi-threaded test runner.

use std::cell::Cell;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem};

use boyko_input::PhysicalInput;
use boyko_math::{Affine3A, Mat4, Quat, Vec3, Vec4};
use boyko_scene::transform::GlobalTransform;
use boyko_scene::ViewUniform;

use boyko_ui::resources::UiViewport;
use boyko_ui::world::components::{UiPickShape, UiPickable};
use boyko_ui::world::{
    HoveredWorldEntity, UiWorldAnchor, UiWorldHoverState, UiWorldScratch, WorldTarget,
    ui_world_pick_system,
};

// ───────────────────────── counting allocator ─────────────────────────────

use std::alloc::{GlobalAlloc, Layout, System as SystemAlloc};

// A *thread-local* allocation counter. A process-global atomic would be corrupted
// by allocations from other test threads running concurrently (the default runner
// is multi-threaded). The counter is armed only inside the measured window via
// `allocs_during`, so it attributes allocations to exactly this thread's steady
// frame and nothing else — sound regardless of `--test-threads`.
thread_local! {
    static TL_ALLOCS: Cell<usize> = const { Cell::new(0) };
    static TL_ARMED: Cell<bool> = const { Cell::new(false) };
}

struct CountingAlloc;

#[inline]
fn tick() {
    if TL_ARMED.try_with(|a| a.get()).unwrap_or(false) {
        let _ = TL_ALLOCS.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: `CountingAlloc` forwards every call verbatim to the system allocator,
// which is a sound `GlobalAlloc`. The only added behavior is a thread-local counter
// increment on the allocating paths, which has no bearing on allocator soundness
// (it neither moves nor reinterprets the returned pointer, and the thread-local
// access itself never allocates, so there is no re-entrancy).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        tick();
        // SAFETY: forwarding an unchanged, valid `Layout`.
        unsafe { SystemAlloc.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarding the exact (ptr, layout) pair the caller obtained from
        // `alloc`, as required by `GlobalAlloc::dealloc`.
        unsafe { SystemAlloc.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        tick();
        // SAFETY: forwarding an unchanged, valid `Layout`.
        unsafe { SystemAlloc.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        tick();
        // SAFETY: forwarding the exact (ptr, layout) the caller holds plus a valid
        // `new_size`, as required by `GlobalAlloc::realloc`.
        unsafe { SystemAlloc.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Counts allocation calls made on *this thread* during `f`.
fn allocs_during(f: impl FnOnce()) -> usize {
    TL_ALLOCS.with(|c| c.set(0));
    TL_ARMED.with(|a| a.set(true));
    f();
    TL_ARMED.with(|a| a.set(false));
    TL_ALLOCS.with(|c| c.get())
}

// ───────────────────────── camera-at-origin view ──────────────────────────

const VP_W: f32 = 1600.0;
const VP_H: f32 = 900.0;
const ASPECT: f32 = VP_W / VP_H;
const FOV_Y: f32 = core::f32::consts::FRAC_PI_2;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;
const CENTER_PX: f64 = (VP_W as f64) * 0.5;
const CENTER_PY: f64 = (VP_H as f64) * 0.5;

fn origin_view() -> ViewUniform {
    ViewUniform {
        view_proj: Mat4::perspective_rh(FOV_Y, ASPECT, NEAR, FAR),
        inv_view: Mat4::IDENTITY,
        camera_pos: Vec4::new(0.0, 0.0, 0.0, 1.0),
        cam_forward: Vec4::new(0.0, 0.0, -1.0, 0.0),
        cam_right: Vec4::new(1.0, 0.0, 0.0, 0.0),
        cam_up: Vec4::new(0.0, 1.0, 0.0, 0.0),
        fov_y: FOV_Y,
        aspect: ASPECT,
        near: NEAR,
        far: FAR,
    }
}

fn spawn_pickable(world: &mut EcsMaster, pos: Vec3) -> Entity {
    let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(
        pos,
        Quat::IDENTITY,
        Vec3::ONE,
    ));
    world.run_system(move |mut cmds: Commands| {
        let mut ec = cmds.spawn(UiPickable {
            shape: UiPickShape::Sphere { radius: 0.5 },
            layers: u32::MAX,
        });
        ec.insert(gt);
        ec.id()
    })
}

fn spawn_world_pos_root(world: &mut EcsMaster, pos: [f32; 3]) {
    // `UiWorldAnchor` auto-inserts `UiWorldProjection` via `#[require(...)]`, so the
    // spawn is enough — the occlusion pass fills the retained `roots`/`bounds`
    // buffers before its per-root branch, so high-water is reached regardless.
    let anchor = UiWorldAnchor {
        target: WorldTarget::WorldPos(pos),
        depth_test: true,
        ..Default::default()
    };
    world.run_system(move |mut cmds: Commands| {
        cmds.spawn(anchor);
    });
}

/// A world seeded for the origin camera with the world-UI resources, several
/// pickables on the cursor ray, and several `WorldPos` anchor roots — a
/// representative non-trivial world so every scratch buffer grows to high-water.
fn seeded_world() -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(origin_view());
    world.insert_resource(UiViewport {
        width: VP_W,
        height: VP_H,
        scale_factor: 1.0,
        generation: 0,
    });
    let mut physical = PhysicalInput::new();
    physical.cursor_pos = [CENTER_PX, CENTER_PY];
    physical.cursor_inside = true;
    physical.window_focused = true;
    world.insert_resource(physical);
    world.insert_resource(HoveredWorldEntity::default());
    world.insert_resource(UiWorldHoverState::default());
    world.insert_resource(UiWorldScratch::default());

    for i in 0..8 {
        spawn_pickable(&mut world, Vec3::new(0.0, 0.0, -4.0 - i as f32));
    }
    for i in 0..8 {
        spawn_world_pos_root(&mut world, [0.0, 0.0, -20.0 - i as f32]);
    }
    world
}

#[test]
fn pick_system_steady_frame_allocates_zero() {
    let mut world = seeded_world();

    // Build the exclusive system ONCE (so no per-call `into_system` construction
    // lands in the measured window) and warm it to high-water: after a few frames
    // every retained `UiWorldScratch` buffer has grown and will not reallocate.
    let mut sys = IntoSystem::into_system(ui_world_pick_system);
    for _ in 0..8 {
        world.run_cached_system(&mut sys);
    }

    // The worst of a few armed steady frames must be exactly zero allocations —
    // the retained buffers are refilled in place (Principle 1/5).
    let worst = (0..4)
        .map(|_| allocs_during(|| {
            world.run_cached_system(&mut sys);
        }))
        .max()
        .unwrap_or(0);

    assert_eq!(
        worst, 0,
        "ui_world_pick_system must allocate nothing on a warmed steady frame \
         (retained UiWorldScratch buffers refilled in place); observed {worst} allocation(s)"
    );
}
