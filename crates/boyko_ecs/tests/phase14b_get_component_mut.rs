//! Phase 14b — `EcsMaster::get_component_mut::<T>() -> Option<Mut<'_, T>>`
//! (the signature CHANGED from `Option<&mut T>` in 14b).
//!
//! Architect plan §11 (R2) cases:
//!
//! | case | what                                                               |
//! |------|--------------------------------------------------------------------|
//! | 12   | returns a `Mut`; `*m = v` bumps the changed tick; a later           |
//! |      | `Changed<T>` query in a system observes it; `is_changed()` is true  |
//! |      | after a write at the current tick (O4 semantics)                    |
//! | 13   | returns `None` for despawned / wrong-generation / missing-`T`       |
//!
//! # O4 semantics
//!
//! Outside a system there is no `last_run` frame boundary; the `Mut` is built
//! with `last_run == this_run == current_tick`, so `is_changed()` reports
//! whether the row was touched at the CURRENT tick (not a frame delta). A write
//! via `*m = v` therefore makes `is_changed()` true immediately.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct GcmHealth(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct GcmOther(u32);

// ════════════════════════════════════════════════════════════════════════════
// Case 12a — get_component_mut returns a Mut; the write is visible; is_changed()
//            is true after a write at the current tick (O4)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_component_mut_returns_mut_and_write_is_visible() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[GcmHealth::component_id()]);
    let e = ecs.spawn_one(arch, GcmHealth(100)).expect("spawn");

    {
        let mut m = ecs.get_component_mut::<GcmHealth>(e).expect("entity has GcmHealth");
        assert_eq!(m.0, 100, "Mut derefs to the stored value");
        m.0 = 250; // routes through DerefMut ⇒ bumps changed_tick to current_tick
        assert!(
            m.is_changed(),
            "is_changed() is true immediately after a write at the current tick (O4)"
        );
    }

    // The write persisted into the pool buffer.
    let v = ecs.get_component::<GcmHealth>(e).expect("entity still has GcmHealth");
    assert_eq!(v.0, 250, "the *m = v write is visible through a fresh read");
}

// ════════════════════════════════════════════════════════════════════════════
// Case 12b — a write via get_component_mut between schedule frames is observed by
//            a later Changed<T> reader system (the cross-into-system path)
// ════════════════════════════════════════════════════════════════════════════

/// A write performed INSIDE a schedule frame via an exclusive system
/// (`|world: &mut EcsMaster|`) calling `get_component_mut` stamps the row's
/// `changed_tick` at `current_tick()`. Since Bug #56 added an apply-window tick
/// bump (so deferred-command changes become observable), `current_tick()` during
/// system execution is one past the frame-start `this_run` that a `Changed<T>`
/// reader's window is keyed on — so the write is observed by the reader on the
/// FOLLOWING frame (exactly once), behaving like a deferred-command change. (For
/// SAME-frame change detection inside a system, query `Mut<T>` directly — it
/// stamps at the system's `this_run`. See the `get_component_mut` O4 doc.)
#[test]
fn changed_query_observes_get_component_mut_write_on_the_following_frame() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[GcmHealth::component_id()]);
    let e = world.spawn_one(arch, GcmHealth(1)).expect("spawn");

    static CHANGED_SEEN: AtomicU32 = AtomicU32::new(0);
    static DO_WRITE: AtomicU32 = AtomicU32::new(0);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    DO_WRITE.store(0, Ordering::Relaxed);

    let target = EntityStash::pack(e);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer = an EXCLUSIVE system that calls get_component_mut on the world.
    builder.add_system(move |w: &mut EcsMaster| {
        if DO_WRITE.load(Ordering::Relaxed) == 1 {
            let ent = EntityStash::unpack(target);
            if let Some(mut m) = w.get_component_mut::<GcmHealth>(ent) {
                m.0 = m.0.wrapping_add(1);
            }
        }
    });
    // Reader = counts rows matching Changed<GcmHealth>.
    builder.add_system(|q: Query<&GcmHealth, Changed<GcmHealth>>| {
        for _ in &q {
            CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1 — fresh insert; the insert tick lies in the first window, so
    // Changed matches the freshly-spawned row (the documented Phase 10 baseline).
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "frame 1: the freshly-inserted row matches Changed (insert tick in window)"
    );

    // Frame 2 — idle (no write). Changed must NOT match.
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 2: idle frame ⇒ Changed yields zero rows"
    );

    // Frame 3 — the exclusive writer runs get_component_mut. Since Bug #56 added
    // an apply-window tick bump, an exclusive system's `current_tick()` is one
    // past the frame-start `this_run` the reader's `Changed` window is keyed on,
    // so the write is NOT observed on the SAME frame (it behaves like a deferred
    // write — consistent with Commands-applied changes).
    DO_WRITE.store(1, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 3: get_component_mut (direct-API, stamped at the apply-window tick) \
         is NOT observed same-frame — see Bug #56 / the get_component_mut O4 doc"
    );

    // Stop writing; the frame-3 write must now surface on the NEXT frame, once.
    DO_WRITE.store(0, Ordering::Relaxed);

    // Frame 4 — the reader's window now includes the frame-3 apply-window tick;
    // the write is observed exactly once.
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "frame 4: the frame-3 get_component_mut write surfaces on the following frame"
    );

    // Frame 5 — observed exactly once: the write tick is no longer in window.
    CHANGED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 5: the write is observed exactly once (not on subsequent frames)"
    );

    // The value was actually mutated (1 -> 2).
    assert_eq!(
        world.get_component::<GcmHealth>(e).expect("alive").0,
        2,
        "get_component_mut write persisted (1 + 1 = 2)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Case 13 — get_component_mut returns None for despawned / wrong-generation /
//           missing-T
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_component_mut_returns_none_for_despawned_entity() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[GcmHealth::component_id()]);
    let e = ecs.spawn_one(arch, GcmHealth(1)).expect("spawn");

    assert!(ecs.delete_entity(e), "despawn succeeds");
    assert!(
        ecs.get_component_mut::<GcmHealth>(e).is_none(),
        "get_component_mut returns None for a despawned entity (slot is null)"
    );
}

#[test]
fn get_component_mut_returns_none_for_wrong_generation() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[GcmHealth::component_id()]);
    let e = ecs.spawn_one(arch, GcmHealth(1)).expect("spawn");

    // Forge a handle with the same id but a stale generation.
    let stale = Entity::new(e.id(), e.generation().wrapping_add(1));
    assert!(
        ecs.get_component_mut::<GcmHealth>(stale).is_none(),
        "get_component_mut returns None for a generation-mismatched handle"
    );
    // The real handle still works (the forge did not corrupt the live slot).
    assert!(
        ecs.get_component_mut::<GcmHealth>(e).is_some(),
        "the live handle still resolves"
    );
}

#[test]
fn get_component_mut_returns_none_for_missing_component_type() {
    let mut ecs = EcsMaster::new();
    // Entity has GcmHealth but NOT GcmOther.
    let arch = ecs.create_archetype(&[GcmHealth::component_id()]);
    let e = ecs.spawn_one(arch, GcmHealth(1)).expect("spawn");

    assert!(
        ecs.get_component_mut::<GcmOther>(e).is_none(),
        "get_component_mut returns None when the archetype lacks the requested component (null column)"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Helper: pack/unpack an Entity into a u64 for capture in a 'static closure.
// ────────────────────────────────────────────────────────────────────────────

struct EntityStash;
impl EntityStash {
    fn pack(e: Entity) -> u64 {
        (e.id().0 as u64) | ((e.generation() as u64) << 32)
    }
    fn unpack(p: u64) -> Entity {
        Entity::new(EntityId((p & 0xFFFF_FFFF) as usize), (p >> 32) as u32)
    }
}
