//! I4 gate 6 — zero per-frame heap allocation on the FULL ingest path: the
//! `update_action_state` body (`begin_frame`, queue drain, `process_actions`,
//! then `freeze_fixed_snapshot`), extending the I3 `zero_alloc.rs` gate to the
//! I4 additions.
//!
//! The system fn itself takes `ResMut`/`Res` params and can only run through the
//! scheduler (which allocates on first init); to isolate the per-frame ingest
//! arithmetic we invoke the EXACT operations the system body performs, over
//! preallocated resources. A thread-local counting allocator (sound under the
//! multi-threaded test runner) tallies allocation calls only inside the measured
//! window. The new I4 step under test is `freeze_fixed_snapshot`, whose
//! `copy_from_slice` writes into the already-allocated `fixed_*` arrays and whose
//! `consumed`-drain loop is over a stack-local `BitSet256` — so a warmed frame
//! must touch the heap zero times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use boyko_input::prelude::*;

thread_local! {
    static TL_ALLOCS: Cell<usize> = const { Cell::new(0) };
    static TL_ARMED: Cell<bool> = const { Cell::new(false) };
}

/// A pass-through allocator that counts `alloc`/`realloc` calls on the current
/// thread while the thread-local arm flag is set.
struct CountingAlloc;

#[inline]
fn tick() {
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
fn allocs_during(f: impl FnOnce()) -> usize {
    TL_ALLOCS.with(|c| c.set(0));
    TL_ARMED.with(|a| a.set(true));
    f();
    TL_ARMED.with(|a| a.set(false));
    TL_ALLOCS.with(|c| c.get())
}

/// Performs exactly what `update_action_state`'s body does, over borrowed
/// preallocated resources (no scheduler, no system-init allocation): reset the
/// per-frame queue/physical state, drain the ring into the physical snapshot,
/// aggregate into the action state, then freeze the fixed snapshot.
fn ingest_body(
    queue: &mut RawInputQueue,
    physical: &mut PhysicalInput,
    state: &mut ActionState<Act>,
    map: &InputMap<Act>,
) {
    queue.begin_frame();
    physical.begin_frame();
    while let Some(ev) = queue.pop() {
        physical.apply(&ev);
    }
    process_actions(physical, map, state);
    state.freeze_fixed_snapshot();
}

#[test]
fn update_action_state_body_is_alloc_free() {
    // --- setup (allocations allowed here) ---
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .bind(Act::Fire, BindSpec::Mouse(MouseButton::Left))
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
    let mut physical = PhysicalInput::new();
    let mut queue = RawInputQueue::with_capacity(1024);

    // Warm: run a couple of full ingest frames so any lazy one-time init has
    // already touched whatever it touches.
    for _ in 0..2 {
        queue.push_raw(RawInputEvent::Key {
            code: KeyCode::Space,
            state: ButtonState::Pressed,
            repeat: false,
        });
        ingest_body(&mut queue, &mut physical, &mut state, &map);
        let _ = state.fixed_just_pressed(Act::Jump);
    }

    // --- measured steady-state full-ingest loop ---
    let allocs = allocs_during(|| {
        for frame in 0..1000u32 {
            // A representative raw burst each frame (keys + mouse + motion).
            queue.push_raw(RawInputEvent::Key {
                code: KeyCode::KeyW,
                state: if frame % 2 == 0 {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                repeat: false,
            });
            queue.push_raw(RawInputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            });
            queue.push_raw(RawInputEvent::MouseMotion { dx: 1.0, dy: -1.0 });
            queue.push_raw(RawInputEvent::Wheel(ScrollDelta::Lines { x: 0.0, y: 1.0 }));

            ingest_body(&mut queue, &mut physical, &mut state, &map);

            // Fixed-view reads (the fixed-loop query surface added in I4).
            std::hint::black_box(state.fixed_pressed(Act::Jump));
            std::hint::black_box(state.fixed_just_pressed(Act::Fire));
            std::hint::black_box(state.fixed_axis2(Act::Move));
            std::hint::black_box(state.fixed_value(Act::Throttle));
        }
    });

    assert_eq!(
        allocs, 0,
        "the warmed update_action_state body (drain + process + freeze) allocated {allocs} times (expected 0)"
    );
}

#[test]
fn freeze_fixed_snapshot_is_alloc_free() {
    // Isolate the I4 freeze step: it must only copy into already-allocated
    // `fixed_*` arrays and drain a stack-local `consumed` bitset.
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .wasd(Act::Move)
        .build();
    let mut state = ActionState::<Act>::new();
    let mut physical = PhysicalInput::new();
    physical.apply(&RawInputEvent::Key {
        code: KeyCode::Space,
        state: ButtonState::Pressed,
        repeat: false,
    });
    process_actions(&physical, &map, &mut state);
    // Warm.
    for _ in 0..4 {
        state.freeze_fixed_snapshot();
    }

    let allocs = allocs_during(|| {
        for _ in 0..10_000u32 {
            state.freeze_fixed_snapshot();
            std::hint::black_box(state.fixed_pressed(Act::Jump));
        }
    });
    assert_eq!(allocs, 0, "freeze_fixed_snapshot allocated {allocs} times (expected 0)");
}

#[test]
fn freeze_with_consumed_is_alloc_free() {
    // The `consumed`-drain branch of `freeze_fixed_snapshot` (non-empty
    // `consumed`) must also be allocation-free — it drains a stack-local copy.
    let map = InputMap::builder()
        .clash(ClashStrategy::PrioritizeLongest)
        .bind(Act::Fire, BindSpec::Key(KeyCode::KeyS))
        .bind(
            Act::QuickSave,
            BindSpec::chord(&[KeyCode::ControlLeft, KeyCode::KeyS]),
        )
        .build();
    let mut state = ActionState::<Act>::new();
    let mut physical = PhysicalInput::new();
    // Hold Ctrl+S so QuickSave suppresses bare S (sets a `consumed` bit).
    physical.apply(&RawInputEvent::Key {
        code: KeyCode::ControlLeft,
        state: ButtonState::Pressed,
        repeat: false,
    });
    physical.apply(&RawInputEvent::Key {
        code: KeyCode::KeyS,
        state: ButtonState::Pressed,
        repeat: false,
    });
    process_actions(&physical, &map, &mut state);
    for _ in 0..4 {
        state.freeze_fixed_snapshot();
    }

    let allocs = allocs_during(|| {
        for _ in 0..10_000u32 {
            // Re-process to refresh the `consumed` set each iteration, then
            // freeze through the non-empty `consumed` drain branch.
            process_actions(&physical, &map, &mut state);
            state.freeze_fixed_snapshot();
            std::hint::black_box(state.fixed_pressed(Act::QuickSave));
        }
    });
    assert_eq!(
        allocs, 0,
        "freeze_fixed_snapshot with a non-empty consumed set allocated {allocs} times (expected 0)"
    );
}

#[test]
fn clear_fixed_edges_is_alloc_free() {
    // The clear half of the sticky-edge model (driven by
    // `clear_consumed_fixed_edges` on Main) must be allocation-free — it is two
    // `BitSet256` overwrites into the already-allocated snapshot, no heap touch.
    let map = InputMap::builder()
        .bind(Act::Jump, BindSpec::Key(KeyCode::Space))
        .build();
    let mut state = ActionState::<Act>::new();
    let mut physical = PhysicalInput::new();
    physical.apply(&RawInputEvent::Key {
        code: KeyCode::Space,
        state: ButtonState::Pressed,
        repeat: false,
    });
    process_actions(&physical, &map, &mut state);
    // Warm: a freeze (to set sticky edges) + a clear.
    for _ in 0..4 {
        state.freeze_fixed_snapshot();
        state.clear_fixed_edges();
    }

    let allocs = allocs_during(|| {
        for _ in 0..10_000u32 {
            // Re-accumulate an edge then clear it — the full sticky cycle the
            // Main pass performs each consuming frame.
            state.freeze_fixed_snapshot();
            state.clear_fixed_edges();
            std::hint::black_box(state.fixed_just_pressed(Act::Jump));
        }
    });
    assert_eq!(
        allocs, 0,
        "the freeze + clear sticky cycle allocated {allocs} times (expected 0)"
    );
}

#[test]
fn counting_allocator_actually_counts() {
    // Sanity: a broken counter would make every zero-alloc assertion vacuously
    // pass.
    let allocs = allocs_during(|| {
        let v: Vec<u8> = Vec::with_capacity(4096);
        std::hint::black_box(v);
    });
    assert!(allocs >= 1, "the counting allocator failed to observe a Vec allocation");
}
