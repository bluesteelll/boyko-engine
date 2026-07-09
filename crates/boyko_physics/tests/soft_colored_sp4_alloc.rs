//! Physics O11 SP4 — STEADY-STATE zero-allocation gate for the colored soft solve
//! (plan C3b: assert STEADY STATE after a warm-up window, NOT first-N-frames).
//!
//! The colored soft step's per-step scratch (the per-type colorings + the
//! self-collision pair list) is reserve-sized at first use and reused thereafter
//! (`clear()` + capacity-reuse): the common case never reallocs, but a
//! denser-than-reserved substep MAY resize-grow. So the zero-alloc claim is a
//! STEADY-STATE claim — after the first few frames warm every buffer to its working
//! capacity, a subsequent step does no heap work.
//!
//! Measured INLINE (no threadpool attached → every color falls back to the inline
//! solve), so no `pool.scope` dispatch box enters the count — this isolates the
//! SOLVER SCRATCH zero-alloc contract from the (separately-bounded, threshold-gated)
//! scope-dispatch cost. A small grid (widest color below the parallel threshold)
//! keeps the run inline even if a pool were attached.
//!
//! Gated `cfg(not(miri))`: the counting `#[global_allocator]` wrapper is a known Miri
//! harness artifact in the std shutdown path.

#![cfg(not(miri))]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{SoftBody, SoftColorScratch, physics_soft_step_colored};

/// A `w x w` grid cloth (top row pinned), structural edges + a few self-collision
/// radii so the self-collision pair list also exercises its reuse.
fn grid_cloth(w: usize) -> SoftBody {
    let mut positions = Vec::with_capacity(w * w);
    let mut inv_masses = Vec::with_capacity(w * w);
    for y in 0..w {
        for x in 0..w {
            positions.push([x as f32 * 0.1, 2.0 - y as f32 * 0.1, 0.0]);
            inv_masses.push(if y == 0 { 0.0 } else { 1.0 });
        }
    }
    let idx = |x: usize, y: usize| (y * w + x) as u32;
    let mut edges = Vec::new();
    for y in 0..w {
        for x in 0..w {
            if x + 1 < w {
                edges.push((idx(x, y), idx(x + 1, y)));
            }
            if y + 1 < w {
                edges.push((idx(x, y), idx(x, y + 1)));
            }
        }
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("grid cloth is well-formed")
}

/// Builds a colored-soft world over a 10x10 grid cloth (widest distance color below
/// `MIN_PARALLEL_SLOTS_PER_COLOR`, so every color solves inline — no scope box),
/// `soft_body_colored = colored`.
fn build_world(colored: bool) -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 2,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_body_colored: colored,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    world.insert_resource(SoftColorScratch::default());
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, grid_cloth(10))
        .expect("{SoftBody} archetype accepts a SoftBody");
    world
}

/// Warms `world` for the warm-up window, then measures the alloc delta of ONE more
/// `physics_soft_step_colored` step (the steady-state measurement, NOT first-N).
fn warmed_step_allocs(colored: bool) -> usize {
    let mut world = build_world(colored);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    for _ in 0..16 {
        world.run_system_once(&mut sys);
    }
    let before = ALLOC.count();
    world.run_system_once(&mut sys);
    ALLOC.count().wrapping_sub(before)
}

/// STEADY-STATE zero-alloc (DIFFERENTIAL): a warmed COLORED soft step allocates no
/// more than a warmed colored step with the flag OFF (which runs the serial
/// `step_body`). The shared floor is the `run_system_once` harness cost (param
/// initialize); the DIFFERENCE is the colored scratch's per-step allocation, which
/// must be ZERO — the C3b steady-state contract (the coloring + pair buffers reuse
/// capacity, and the inline path issues no scope box).
#[test]
fn colored_soft_step_adds_zero_alloc_over_serial() {
    let serial_floor = warmed_step_allocs(false);
    let colored_step = warmed_step_allocs(true);
    assert!(
        colored_step <= serial_floor,
        "SP4 C3b: a warmed colored soft step must add ZERO heap allocation over the \
         serial baseline (the coloring + pair scratch capacity is reused); \
         colored = {colored_step}, serial floor = {serial_floor}"
    );
}

// ── Counting global allocator (mirrors colored_solve_zero_alloc_o5.rs) ──────────

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

impl CountingAlloc {
    fn count(&self) -> usize {
        ALLOC_COUNT.with(|c| c.get())
    }
}

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call forwards verbatim to the platform `System` allocator with the
// same layout; the wrapper only bumps a thread-local counter (via a `try_with` that
// no-ops if TLS is mid-init, so it never re-enters the allocator). `dealloc` is an
// unchanged pass-through, so the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: forwarded verbatim to the system allocator (same layout).
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` originate from `System.alloc` above (this is the
        // process global allocator), so they satisfy `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` originate from this allocator; `new_size` forwarded
        // verbatim to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;
