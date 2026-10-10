//! `EcsMaster::set_component_raw` must stamp the row's `changed` tick on the
//! TABLE path, exactly as it already does on the dense one.
//!
//! # The defect this file gates
//!
//! `set_component_raw` has two arms. The dense arm routes the write through
//! `DenseStore::insert_or_replace(.., current_tick)`, which stamps the slot's
//! `changed` tick. The table arm byte-copied into the column row and stamped
//! nothing — so a table component written through the raw API moved its bytes
//! and moved no tick, and every `Changed<T>` consumer stayed blind to it.
//!
//! That is a SILENT WRONG ANSWER, not an error: nothing returns `false`,
//! nothing logs, and the value read back through `get_component` is the new
//! one. Only the downstream dirty gates disagree — `boyko_scene`'s transform
//! propagation is gated on `Transform`'s per-row `changed_tick`, and the GPU
//! instance sync on `GlobalTransform`'s, so a raw write moved the data and
//! nothing on screen.
//!
//! # Why the assertions are written as a table/dense DIFFERENTIAL
//!
//! Every gate below asserts the table answer against the dense answer taken in
//! the same world, on the same frame, through the same reader system. The dense
//! half passes on the unfixed code; if a harness mistake (a wrong
//! `(last_run, this_run]` window, a frame that never ran, an entity that was
//! never spawned) made the table half unable to fire, it would take the dense
//! control down with it. A green table half over a red dense half cannot be a
//! vacuous pass.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

/// TABLE payload — the storage arm under test. Shaped like `Transform`'s hot
/// prefix (the real consumer whose dirty gate this defect broke).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct RawTable {
    x: f32,
    y: f32,
}

/// DENSE payload — the control. Its arm already stamps the tick, so it answers
/// what a correct table arm must answer.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct RawDense {
    x: f32,
    y: f32,
}

#[derive(Bundle)]
struct BothPayloads {
    t: RawTable,
    d: RawDense,
}

/// Rows the reader system saw as `Changed<RawTable>` on the last frame.
static TABLE_CHANGED_SEEN: AtomicUsize = AtomicUsize::new(0);
/// Rows the reader system saw as `Changed<RawDense>` on the last frame.
static DENSE_CHANGED_SEEN: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn table_bytes(v: &RawTable) -> &[u8] {
    // SAFETY: `RawTable` is `#[repr(C)]` POD (two `f32`s, no padding, no
    // interior references), so its own byte span is a valid representation of
    // the type the pool was registered with; the slice borrows `v` and lives no
    // longer than it.
    unsafe {
        std::slice::from_raw_parts((v as *const RawTable).cast::<u8>(), size_of::<RawTable>())
    }
}

#[inline]
fn dense_bytes(v: &RawDense) -> &[u8] {
    // SAFETY: as `table_bytes` — `RawDense` is `#[repr(C)]` POD and the slice
    // borrows `v`.
    unsafe {
        std::slice::from_raw_parts((v as *const RawDense).cast::<u8>(), size_of::<RawDense>())
    }
}

/// Spawns one entity carrying both payloads.
fn spawn_both(world: &mut EcsMaster) -> Entity {
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(BothPayloads {
            t: RawTable { x: 0.0, y: 0.0 },
            d: RawDense { x: 0.0, y: 0.0 },
        })
        .id()
    })
}

/// Builds the reader schedule: one system per storage kind, each counting the
/// rows its `Changed<_>` filter matched into the counter above.
///
/// The counters are reset at the start of every frame by the schedule's first
/// system, so a frame's count is that frame's answer and never an accumulation.
fn build_reader(world: &mut EcsMaster) -> boyko_ecs::ecs::core::schedule::Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&RawTable, Changed<RawTable>>| {
        for _ in &q {
            TABLE_CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    builder.add_system(|q: Query<&RawDense, Changed<RawDense>>| {
        for _ in &q {
            DENSE_CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    builder.build(world)
}

/// Zeroes both counters, runs one frame, and returns `(table_seen, dense_seen)`.
fn frame(
    world: &mut EcsMaster,
    schedule: &mut boyko_ecs::ecs::core::schedule::Schedule,
) -> (usize, usize) {
    TABLE_CHANGED_SEEN.store(0, Ordering::Relaxed);
    DENSE_CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(world);
    (
        TABLE_CHANGED_SEEN.load(Ordering::Relaxed),
        DENSE_CHANGED_SEEN.load(Ordering::Relaxed),
    )
}

/// THE GATE. A raw write between two frames must be observed by a
/// `Changed<T>` reader on the next frame — on BOTH storage kinds.
///
/// Hand oracle: `(1, 1)` on the frame after the write. Before the fix the table
/// half observed `0`.
///
/// Serialised with the other two tests (`--test-threads=1` is not required:
/// each test builds its own world, and the two counters are process-global but
/// reset per frame — so this test is run alone in its own `#[test]` and the
/// other tests below deliberately do not run the reader schedule).
#[test]
fn raw_write_is_seen_by_a_changed_query_on_both_storage_kinds() {
    let mut world = EcsMaster::new();
    let e = spawn_both(&mut world);
    let mut schedule = build_reader(&mut world);

    // Frame 1: the insert's own stamp lies in the first window, so both halves
    // match once. This is the floor that proves the reader systems run at all.
    let (t1, d1) = frame(&mut world, &mut schedule);
    assert_eq!(
        (t1, d1),
        (1, 1),
        "frame 1: the insert stamps both ticks, so both readers match once \
         (floor: a (0, 0) here means the schedule never ran the readers)"
    );

    // Frame 2: nobody wrote. Both halves must be silent — this is what makes
    // frame 3's `1` a signal rather than a component that is always Changed.
    let (t2, d2) = frame(&mut world, &mut schedule);
    assert_eq!(
        (t2, d2),
        (0, 0),
        "frame 2: an idle frame must match nothing, or frame 3 proves nothing"
    );

    // The write under test: the raw byte-copy entry point, between frames.
    assert!(
        world.set_component_raw(
            e,
            RawTable::component_id(),
            table_bytes(&RawTable { x: 7.0, y: 8.0 }),
        ),
        "set_component_raw must accept a live TABLE row"
    );
    assert!(
        world.set_component_raw(
            e,
            RawDense::component_id(),
            dense_bytes(&RawDense { x: 7.0, y: 8.0 }),
        ),
        "set_component_raw must accept a live DENSE member"
    );

    // Frame 3: both writes must surface. The dense half is the control.
    let (t3, d3) = frame(&mut world, &mut schedule);
    assert_eq!(
        d3, 1,
        "control: the DENSE arm stamps the slot tick via insert_or_replace, so \
         Changed<RawDense> matches the frame after the raw write. A 0 here means \
         the harness is wrong (window / frame), not that the table arm is broken"
    );
    assert_eq!(
        t3, 1,
        "THE GATE: a raw write to a TABLE component must bump the row's changed \
         tick, exactly as the dense arm does. 0 is the defect — set_component_raw's \
         table arm byte-copied the value and stamped no tick, so transform \
         propagation and the GPU instance sync (both dirty-gated on this tick) \
         never saw the write and nothing moved on screen"
    );

    // The value itself must be the written one on both arms — proving the gate
    // measures the TICK, not a write that failed to land.
    assert_eq!(
        world.get_component::<RawTable>(e).copied(),
        Some(RawTable { x: 7.0, y: 8.0 }),
        "the table write must have landed (the defect is the tick, not the bytes)"
    );
    assert_eq!(
        world.get_component::<RawDense>(e).copied(),
        Some(RawDense { x: 7.0, y: 8.0 }),
        "the dense write must have landed"
    );
}

/// The same fact one layer down, without a schedule: the entity-keyed tick
/// reader must report a NEWER tick after a raw table write than before it.
///
/// This is the diagnostic twin of the gate above — it fails on exactly the same
/// defect, but names the stored tick directly instead of a matched row, so a
/// future change to the `(last_run, this_run]` window cannot make it ambiguous.
#[test]
fn raw_table_write_advances_the_rows_changed_tick() {
    let mut world = EcsMaster::new();
    let e = spawn_both(&mut world);
    let cid = RawTable::component_id();

    let before = world
        .get_component_changed_tick(e, cid)
        .expect("invariant: a freshly spawned live entity has a TABLE row with a tick");

    // Advance the world tick past the spawn's stamp, so "unchanged" and
    // "changed" are distinguishable values rather than the same number.
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|_q: Query<&RawTable>| {});
    let mut tick_frame = builder.build(&mut world);
    tick_frame.run(&mut world);
    tick_frame.run(&mut world);

    let idle = world
        .get_component_changed_tick(e, cid)
        .expect("invariant: the row is still live after two idle frames");
    assert_eq!(
        idle, before,
        "an idle frame must not touch the row's changed tick — otherwise the \
         assertion below would pass without any write"
    );

    assert!(
        world.set_component_raw(e, cid, table_bytes(&RawTable { x: 1.0, y: 2.0 })),
        "set_component_raw must accept a live TABLE row"
    );

    let after = world
        .get_component_changed_tick(e, cid)
        .expect("invariant: the row is still live after the raw write");
    assert_ne!(
        after, before,
        "THE GATE (tick-level): set_component_raw's table arm must stamp the \
         row's changed tick with the world's current tick. An unchanged tick is \
         the defect: the bytes moved and the change-detection state did not"
    );
    assert_eq!(
        after,
        world.current_tick(),
        "the stamp must be the world's CURRENT tick — the same fact the dense \
         arm passes to DenseStore::insert_or_replace, not a second encoding"
    );
}

/// A raw write that is REFUSED must not stamp anything: a stale entity handle
/// returns `false` and leaves every live row's tick alone.
///
/// Without this, "stamp the current tick unconditionally at the top of the
/// function" would pass the gates above while marking rows nobody wrote.
#[test]
fn a_refused_raw_write_stamps_nothing() {
    let mut world = EcsMaster::new();
    let victim = spawn_both(&mut world);
    let doomed = spawn_both(&mut world);
    let cid = RawTable::component_id();

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|_q: Query<&RawTable>| {});
    let mut tick_frame = builder.build(&mut world);
    tick_frame.run(&mut world);

    assert!(
        world.delete_entity(doomed),
        "the doomed entity must actually be removed, or the refusal below is vacuous"
    );
    tick_frame.run(&mut world);

    let victim_before = world
        .get_component_changed_tick(victim, cid)
        .expect("invariant: the surviving entity keeps its row");

    assert!(
        !world.set_component_raw(doomed, cid, table_bytes(&RawTable { x: 3.0, y: 4.0 })),
        "a stale entity handle must be refused"
    );
    assert_eq!(
        world.get_component_changed_tick(victim, cid),
        Some(victim_before),
        "a refused write must not stamp any row's changed tick"
    );
}
