//! I3 gate — zero per-frame heap allocation on the hot ingest path
//! (plan §1, §15: "random `RawInputEvent` streams never allocate on ingest").
//!
//! A counting global allocator wraps the system allocator and tallies
//! allocation *calls*. The test allocates and warms all state in a setup phase,
//! then snapshots the counter, runs the steady-state hot loop
//! (`PhysicalInput::begin_frame` + `RawInputQueue::push_raw`/`pop` +
//! `ActionState::begin_frame` via `process_actions`), and asserts the counter
//! did not move. The value/binding arrays are allocated once at build, the clash
//! candidate set is stack-resident, and the bitset edge math is in-place — so a
//! warmed frame must touch the heap zero times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_input::prelude::*;

// A *thread-local* allocation counter. A process-global atomic would be
// corrupted by allocations from other test threads running concurrently (the
// default test runner is multi-threaded, and every test's `InputMapBuilder`
// setup allocates). The counter is only armed inside the measured window via
// `allocs_during`, so it attributes allocations to exactly this thread's hot
// loop and nothing else — sound regardless of `--test-threads`.
thread_local! {
    static TL_ALLOCS: Cell<usize> = const { Cell::new(0) };
    static TL_ARMED: Cell<bool> = const { Cell::new(false) };
}

/// A pass-through allocator that counts `alloc`/`realloc` calls on the current
/// thread while the thread-local arm flag is set.
struct CountingAlloc;

#[inline]
fn tick() {
    // `TL_ARMED`/`TL_ALLOCS` access never allocates, so no re-entrancy.
    if TL_ARMED.try_with(|a| a.get()).unwrap_or(false) {
        let _ = TL_ALLOCS.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: `CountingAlloc` forwards every call verbatim to the `System`
// allocator, which is a sound `GlobalAlloc`. The only added behavior is a
// thread-local counter increment on the allocation paths, which has no bearing
// on allocator soundness (it neither moves nor reinterprets the returned
// pointer, and the thread-local access itself never allocates).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        tick();
        // SAFETY: forwarding an unchanged, valid `Layout` to the system alloc.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarding the exact (ptr, layout) pair the caller obtained
        // from `alloc`, as required by `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        tick();
        // SAFETY: forwarding an unchanged, valid `Layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        tick();
        // SAFETY: forwarding the exact (ptr, layout) the caller holds plus a
        // valid `new_size`, as required by `GlobalAlloc::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[derive(Actionlike, Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Jump,
    Fire,
    QuickSave,
    #[actionlike(Axis2D)]
    Move,
    #[actionlike(Axis1D)]
    Throttle,
}

/// Counts allocation calls made on *this thread* during `f`.
///
/// Arms the thread-local counter only for the duration of `f`, so allocations
/// from setup, from the test harness, and from other concurrently-running test
/// threads are not counted.
fn allocs_during(f: impl FnOnce()) -> usize {
    TL_ALLOCS.with(|c| c.set(0));
    TL_ARMED.with(|a| a.set(true));
    f();
    TL_ARMED.with(|a| a.set(false));
    TL_ALLOCS.with(|c| c.get())
}

#[test]
fn process_actions_hot_loop_is_alloc_free() {
    // --- setup (allocations allowed here) ---
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .bind(Act::Fire, BindSpec::Mouse(MouseButton::Left))
        .bind(Act::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .wasd(Act::Move)
        .bind(
            Act::Throttle,
            BindSpec::Axis1 {
                neg: InputRef::Key(KeyCode::KeyQ),
                pos: InputRef::Key(KeyCode::KeyE),
                dz: 0.1,
            },
        )
        .build();
    let mut state = ActionState::<Act>::new();
    let mut phys = PhysicalInput::new();

    // Warm: run a couple of frames so any lazy one-time init (e.g. the action
    // kind dispatch) has already touched whatever it touches.
    for _ in 0..2 {
        phys.begin_frame();
        phys.apply(&RawInputEvent::Key {
            code: KeyCode::Space,
            state: ButtonState::Pressed,
            repeat: false,
        });
        process_actions(&phys, &map, &mut state);
        let _ = state.just_pressed(Act::Jump);
    }

    // --- measured steady-state hot loop ---
    let allocs = allocs_during(|| {
        for frame in 0..1000u32 {
            phys.begin_frame();
            // A representative input mix each frame.
            phys.apply(&RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: if frame % 2 == 0 {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                repeat: false,
            });
            phys.apply(&RawInputEvent::MouseMotion { dx: 1.0, dy: -1.0 });
            phys.apply(&RawInputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            });
            process_actions(&phys, &map, &mut state);
            // Steady-state reads (the gameplay query surface).
            std::hint::black_box(state.pressed(Act::Jump));
            std::hint::black_box(state.just_pressed(Act::Fire));
            std::hint::black_box(state.axis2(Act::Move));
            std::hint::black_box(state.value(Act::Throttle));
        }
    });

    assert_eq!(
        allocs, 0,
        "the warmed process_actions hot loop allocated {allocs} times (expected 0)"
    );
}

#[test]
fn raw_queue_push_pop_loop_is_alloc_free() {
    // The ring is one allocation at build; push/pop/begin_frame must never alloc.
    let mut q = RawInputQueue::with_capacity(1024);
    // Warm.
    for _ in 0..16 {
        q.push_raw(RawInputEvent::Key {
            code: KeyCode::Space,
            state: ButtonState::Pressed,
            repeat: false,
        });
        let _ = q.pop();
    }

    let allocs = allocs_during(|| {
        for _ in 0..10_000u32 {
            q.begin_frame();
            q.push_raw(RawInputEvent::MouseMotion { dx: 1.0, dy: 1.0 });
            q.push_raw(RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: 1.0 }));
            while let Some(ev) = q.pop() {
                std::hint::black_box(ev);
            }
        }
    });

    assert_eq!(allocs, 0, "the ring push/pop loop allocated {allocs} times (expected 0)");
}

#[test]
fn counting_allocator_actually_counts() {
    // Sanity: the harness itself must see a deliberate allocation, otherwise a
    // broken counter would make every zero-alloc assertion vacuously pass.
    let allocs = allocs_during(|| {
        let v: Vec<u8> = Vec::with_capacity(4096);
        std::hint::black_box(v);
    });
    assert!(allocs >= 1, "the counting allocator failed to observe a Vec allocation");
}

#[test]
fn physical_begin_frame_is_alloc_free() {
    let mut p = PhysicalInput::new();
    p.apply(&RawInputEvent::Key {
        code: KeyCode::KeyW,
        state: ButtonState::Pressed,
        repeat: false,
    });
    let allocs = allocs_during(|| {
        for _ in 0..10_000u32 {
            p.begin_frame();
            std::hint::black_box(&p);
        }
    });
    assert_eq!(allocs, 0, "PhysicalInput::begin_frame allocated {allocs} times");
}
