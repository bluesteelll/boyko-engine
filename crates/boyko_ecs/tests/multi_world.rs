//! Phase 21 — multi-world hardening suite.
//!
//! N `EcsMaster`s / `App`s coexist in one process. Process-global state is
//! metadata-only (component/event/bundle/query-type registries, the `HOOKS`
//! table, the H1 `EVER_ARCHETYPED` bitmask, the `WorldId` counter); everything
//! world-derived (archetypes, entities, pools, caches, observers, resources,
//! events) is world-owned. This file pins that model:
//!
//! 1. Two `EcsMaster`s: independent spawn/get/despawn with the SAME bundle
//!    type in both (extends the `clear_respawn` multi-world pin); `clear()` of
//!    A leaves B intact.
//! 2. Two `App`s on SEPARATE pools: interleaved frames; change detection is
//!    per-world.
//! 3. Two `App`s on ONE shared pool (`App::with_pool`): interleaved frames;
//!    events preregistered per the H4 contract
//!    (`EventConfig::default_for(worker_count + 1)`); an event sent in A is
//!    never visible in B.
//! 4. States: the same `States` type in both worlds, independent transitions.
//! 5. H1 regression: hooks are PROCESS-GLOBAL per type, so registering hooks
//!    for a component already archetyped in ANOTHER world must panic (the
//!    pre-21 per-world scan silently skipped the hook in that other world).
//! 6. Observers are PER-WORLD: registered in A, never fires in B.
//! 7. Cross-world `Entity` handles: out-of-range foreign handle → `None`;
//!    the (id, generation)-collision aliasing case is pinned as DOCUMENTED
//!    behavior (Bevy parity — `Entity` is not world-tagged).
//! 8. H2 regression: a `Schedule` built on world A panics (`boyko-B9101`)
//!    when run on world B.
//!
//! # Miri subset
//!
//! The pool-less tests (1, 5, 6, 7, `world_ids_are_process_unique`) run under
//! Miri; everything constructing a real `ThreadPool` is `#[cfg(not(miri))]`
//! (the `app_plugin.rs` gate):
//!
//! ```powershell
//! $env:MIRIFLAGS = "-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test multi_world
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};

use static_assertions::{assert_impl_all, assert_not_impl_any};

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

// ── Compile-time Send/Sync pins (Phase 21 docs item) ────────────────────────
//
// `EcsMaster` IS `Send + Sync` (Phase 9 SEND1). `App` is NOT — not because of
// the world (the stale pre-21 `app.rs` comment claimed "inherited from
// EcsMaster") but because of the type-erased one-shot closures it stages:
// `StartupSystem = Box<dyn FnOnce(&mut EcsMaster)>` (no `+ Send`) and the
// schedules' `StateEntry::insert` closures of the same shape. The compiler is
// the oracle here: if a refactor ever makes `App` `Send`, this assert fails to
// compile and the `app.rs` threading doc must be revisited.
assert_impl_all!(EcsMaster: Send, Sync);
assert_not_impl_any!(App: Send, Sync);

// ═════════════════════════════════════════════════════════════════════════
// 0 — WorldId minting
// ═════════════════════════════════════════════════════════════════════════

/// Every constructor mints a fresh process-unique id (`new` AND
/// `with_capacity` — both funnel through `WorldId::mint`).
#[test]
fn world_ids_are_process_unique() {
    let a = EcsMaster::new();
    let b = EcsMaster::new();
    let c = EcsMaster::with_capacity(8, 8);
    assert_ne!(a.world_id(), b.world_id(), "two worlds must never share a WorldId");
    assert_ne!(a.world_id(), c.world_id(), "with_capacity mints too");
    assert_ne!(b.world_id(), c.world_id());
}

// ═════════════════════════════════════════════════════════════════════════
// 1 — two EcsMasters: independent storage, same bundle type
// ═════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
struct MwPayload {
    v: u64,
}

#[derive(Bundle)]
struct MwBundle {
    p: MwPayload,
}

/// Independent spawn/get/despawn with the SAME bundle type in both worlds;
/// `clear()` of A leaves B intact; A can respawn the type after clear (the
/// `clear_respawn` pin, extended across worlds).
#[test]
fn two_worlds_independent_spawn_get_despawn_clear() {
    let mut a = EcsMaster::new();
    let mut b = EcsMaster::new();

    let ea = a
        .spawn_batch((0..8u32).map(|i| MwBundle { p: MwPayload { v: 100 + i as u64 } }))
        .expect("spawn in A");
    let eb = b
        .spawn_batch((0..4u32).map(|i| MwBundle { p: MwPayload { v: 900 + i as u64 } }))
        .expect("spawn in B");

    let assert_b_intact = |b: &EcsMaster| {
        for (i, &e) in eb.iter().enumerate() {
            assert_eq!(
                b.get_component::<MwPayload>(e).expect("B row readable").v,
                900 + i as u64,
                "B's rows must be untouched by A's operations"
            );
        }
    };

    for (i, &e) in ea.iter().enumerate() {
        assert_eq!(a.get_component::<MwPayload>(e).expect("A row readable").v, 100 + i as u64);
    }
    assert_b_intact(&b);

    // Despawn in A — invisible to B.
    assert!(a.despawn_without_children(ea[0]), "despawn of a live A entity succeeds");
    assert!(a.get_component::<MwPayload>(ea[0]).is_none(), "despawned A row is gone");
    assert_b_intact(&b);

    // clear() of A — B fully intact.
    a.clear();
    for &e in &ea {
        assert!(
            a.get_component::<MwPayload>(e).is_none(),
            "no stale A entity survives clear()"
        );
    }
    assert_b_intact(&b);

    // A respawns the SAME bundle type post-clear (per-world caches reset).
    let fresh = a
        .spawn_batch((0..2u32).map(|i| MwBundle { p: MwPayload { v: 500 + i as u64 } }))
        .expect("post-clear respawn in A");
    for (i, &e) in fresh.iter().enumerate() {
        assert_eq!(a.get_component::<MwPayload>(e).expect("fresh A row").v, 500 + i as u64);
    }
    assert_b_intact(&b);
}

// ═════════════════════════════════════════════════════════════════════════
// 5 — H1 regression: the hooks staleness gate is process-global
// ═════════════════════════════════════════════════════════════════════════

/// Plain derive (no `#[component(...)]`) so the derive-conflict check passes;
/// the panic under test is the STALENESS one. Private to this test.
#[derive(Component)]
#[repr(C)]
struct MwHooked(u32);

/// THE H1 regression. World B places `MwHooked` into a live archetype; world
/// A — which never saw the type — then calls `register_component_hooks`.
/// Hooks are process-global per type, so committing them now would leave B's
/// pre-install archetype flags stale and the hook silently skipped in B.
/// Pre-Phase-21 the per-world scan (A has no archetypes) passed silently;
/// the process-global `EVER_ARCHETYPED` gate must panic.
#[test]
#[should_panic(expected = "already appears in a live archetype")]
fn h1_register_hooks_after_foreign_world_archetype_panics() {
    let mut b = EcsMaster::new();
    let _arch = b.create_archetype(&[MwHooked::component_id()]);

    let mut a = EcsMaster::new();
    let _builder = a.register_component_hooks::<MwHooked>();
}

// ═════════════════════════════════════════════════════════════════════════
// 6 — observers are per-world
// ═════════════════════════════════════════════════════════════════════════

static MW_OBS_ADD_FIRES: AtomicUsize = AtomicUsize::new(0);

/// `ObserverFn` is a bare fn pointer (cannot capture) — the counter is a
/// module static, private to this test's component type.
unsafe fn mw_obs_add(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    MW_OBS_ADD_FIRES.fetch_add(1, Ordering::SeqCst);
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MwObserved(u32);

#[derive(Bundle)]
struct MwObservedBundle {
    o: MwObserved,
}

/// An observer registered in world A fires for A's structural ops only —
/// the registry lives on each world's `ArchetypeMaster` (unlike hooks).
#[test]
fn observer_registered_in_world_a_does_not_fire_in_world_b() {
    let mut a = EcsMaster::new();
    let mut b = EcsMaster::new();

    let _id = a.observe_on_add::<MwObserved>(mw_obs_add);

    // Spawn in B (the canonical 14b Commands path) — must NOT fire A's observer.
    b.run_system(|mut cmds: Commands| {
        cmds.spawn(MwObservedBundle { o: MwObserved(1) });
    });
    assert_eq!(
        MW_OBS_ADD_FIRES.load(Ordering::SeqCst),
        0,
        "observers are per-world: a spawn in B must not fire A's observer"
    );

    // Spawn in A — fires exactly once.
    a.run_system(|mut cmds: Commands| {
        cmds.spawn(MwObservedBundle { o: MwObserved(2) });
    });
    assert_eq!(
        MW_OBS_ADD_FIRES.load(Ordering::SeqCst),
        1,
        "the same op in A fires A's observer exactly once"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// 7 — cross-world Entity handles
// ═════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
struct MwForeign {
    v: u32,
}

#[derive(Bundle)]
struct MwForeignBundle {
    f: MwForeign,
}

/// A foreign handle whose slot index is out of range for the queried world
/// resolves to `None` / `false` — no panic, no UB.
#[test]
fn cross_world_out_of_range_handle_is_none() {
    let mut a = EcsMaster::new();
    let b = EcsMaster::new();

    let ea = a
        .spawn_batch((0..5u32).map(|i| MwForeignBundle { f: MwForeign { v: i } }))
        .expect("spawn in A");
    let foreign = ea[4];

    assert!(!b.has_entity(foreign), "B never allocated this slot");
    assert!(
        b.get_component::<MwForeign>(foreign).is_none(),
        "an out-of-range foreign handle reads as absent, never panics"
    );
}

/// DOCUMENTED behavior pin (Bevy parity): `Entity` is NOT world-tagged. Two
/// worlds allocate from independent per-world slot/generation spaces, so the
/// first entity of each world carries the identical `(id, generation)` pair —
/// and a handle from world A used against world B silently resolves to B's
/// OWN row at that slot. This is the deliberate Bevy-parity trade (an 8-byte
/// `Entity`, no world id in the hot handle); the multi-world contract is
/// "don't cross handles between worlds", enforced by documentation, not type.
#[test]
fn cross_world_colliding_handle_aliases_local_row_documented() {
    let mut a = EcsMaster::new();
    let mut b = EcsMaster::new();

    let ea = a
        .spawn_batch(std::iter::once(MwForeignBundle { f: MwForeign { v: 111 } }))
        .expect("spawn in A")[0];
    let eb = b
        .spawn_batch(std::iter::once(MwForeignBundle { f: MwForeign { v: 222 } }))
        .expect("spawn in B")[0];

    assert_eq!(
        ea, eb,
        "first entity of each world has the identical (id, generation) — handles are world-blind"
    );
    assert_eq!(
        b.get_component::<MwForeign>(ea).expect("the colliding handle resolves in B").v,
        222,
        "a colliding foreign handle reads the LOCAL world's row (aliasing is the documented \
         Bevy-parity behavior, not a bug)"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// Pool-backed tests (Apps + schedules) — not under Miri (app_plugin.rs gate)
// ═════════════════════════════════════════════════════════════════════════

#[cfg(not(miri))]
mod with_pool {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use boyko_ecs::ecs::core::app::App;
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::events::event::Event;
    use boyko_ecs::ecs::core::events::event_config::EventConfig;
    use boyko_ecs::ecs::core::events::event_registry::register_event;
    use boyko_ecs::ecs::core::events::parameters::parameters::Parameters;
    use boyko_ecs::ecs::core::events::participants::participants::{
        ParticipantInfo, Participants,
    };
    use boyko_ecs::ecs::core::iters::query::{Changed, Query};
    use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
    use boyko_ecs::ecs::core::state::{NextState, States};
    use boyko_ecs::ecs::core::system::{EventReader, EventWriter, ResMut};
    use boyko_macros::{Bundle, Component};
    use boyko_threadpool::ThreadPoolBuilder;

    /// One 64 Hz step (the deterministic App-driver delta).
    const STEP: Duration = Duration::from_nanos(15_625_000);

    // ── 8 — H2 regression: Schedule is bound to its build world ─────────────

    /// THE H2 regression. Pre-Phase-21 a cross-world `run` was undetectable —
    /// the systems' per-world caches (event-buffer `NonNull`s, query-state
    /// generations) would silently dereference against the wrong world (a UAF
    /// surface). Now the `WorldId` gate panics loudly in release.
    #[test]
    #[should_panic(expected = "boyko-B9101")]
    fn h2_schedule_built_on_world_a_panics_on_world_b() {
        let pool = ThreadPoolBuilder::new().num_threads(1).build();
        let mut a = EcsMaster::new();
        let mut b = EcsMaster::new();

        let mut builder = ScheduleBuilder::new(pool);
        builder.add_system(|| {});
        let mut schedule = builder.build(&mut a);

        // Positive half: the bound world runs fine.
        schedule.run(&mut a);
        // Negative half: any other world panics (boyko-B9101).
        schedule.run(&mut b);
    }

    // ── 2 — two Apps, separate pools, interleaved change detection ──────────

    #[derive(Component)]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MwTracked {
        v: u32,
    }

    #[derive(Bundle)]
    struct MwTrackedBundle {
        t: MwTracked,
    }

    fn changed_counting_app(hits: Arc<AtomicU32>) -> App {
        let mut app = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
        app.add_systems(move |q: Query<&MwTracked, Changed<MwTracked>>| {
            hits.fetch_add(q.iter().count() as u32, Ordering::Relaxed);
        });
        app
    }

    /// Two Apps on SEPARATE pools, frames interleaved A/B/A/B: each world's
    /// `Changed<T>` windows track only its OWN spawns and mutations.
    #[test]
    fn two_apps_separate_pools_interleaved_change_detection() {
        let hits_a = Arc::new(AtomicU32::new(0));
        let hits_b = Arc::new(AtomicU32::new(0));
        let mut a = changed_counting_app(Arc::clone(&hits_a));
        let mut b = changed_counting_app(Arc::clone(&hits_b));
        a.finish();
        b.finish();

        let _ea = a
            .world_mut()
            .spawn_batch((0..3u32).map(|i| MwTrackedBundle { t: MwTracked { v: i } }))
            .expect("spawn in A");
        let eb = b
            .world_mut()
            .spawn_batch((0..5u32).map(|i| MwTrackedBundle { t: MwTracked { v: i } }))
            .expect("spawn in B");

        // Interleaved warm frames: each app observes its own spawns exactly once.
        for _ in 0..3 {
            a.update_with_delta(STEP);
            b.update_with_delta(STEP);
        }
        assert_eq!(hits_a.load(Ordering::Relaxed), 3, "A sees its 3 spawns exactly once");
        assert_eq!(hits_b.load(Ordering::Relaxed), 5, "B sees its 5 spawns exactly once");

        // Mutate ONE row in B only (direct API stamps the change tick).
        {
            let mut m = b
                .world_mut()
                .get_component_mut::<MwTracked>(eb[0])
                .expect("B row for mutation");
            m.v += 1;
        }
        for _ in 0..3 {
            a.update_with_delta(STEP);
            b.update_with_delta(STEP);
        }
        assert_eq!(
            hits_a.load(Ordering::Relaxed),
            3,
            "A NEVER observes B's mutation (change detection is per-world)"
        );
        assert_eq!(
            hits_b.load(Ordering::Relaxed),
            6,
            "B observes exactly its own mutation, exactly once"
        );
    }

    // ── 3 — two Apps, ONE shared pool: event isolation ───────────────────────

    // Hand-rolled `Event` impl with a fixed id (the phase12 pattern — the
    // integration-test binary owns its registry; id 130 collides with nothing
    // in this file). Empty participants/parameters.
    #[derive(Clone, Copy)]
    struct NoParticipants;
    impl Participants for NoParticipants {
        fn participant_count() -> usize {
            0
        }
        fn participant_info() -> &'static [ParticipantInfo] {
            &[]
        }
    }

    #[derive(Clone, Copy)]
    struct NoParameters;
    impl Parameters for NoParameters {}

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct MwEvent {
        value: u32,
    }

    impl Event for MwEvent {
        type Participants = NoParticipants;
        type Parameters = NoParameters;
        fn event_id() -> u64 {
            130
        }
        fn event_name() -> &'static str {
            "MwEvent"
        }
        fn new(_: NoParticipants, _: NoParameters) -> Self {
            MwEvent { value: 0 }
        }
        fn participants(&self) -> &NoParticipants {
            unimplemented!()
        }
        fn participants_mut(&mut self) -> &mut NoParticipants {
            unimplemented!()
        }
        fn parameters(&self) -> &NoParameters {
            unimplemented!()
        }
        fn parameters_mut(&mut self) -> &mut NoParameters {
            unimplemented!()
        }
    }

    /// Two Apps over ONE shared `ThreadPool` (`App::with_pool`), frames
    /// interleaved. Events are preregistered per the H4 shared-pool contract —
    /// `EventConfig::default_for(worker_count + 1)` so every worker lane plus
    /// the dispatcher lane is in range for BOTH worlds. An `MwEvent` sent in
    /// world A must never surface in world B: each world owns its own
    /// `EventDispatcher` (the type registry is global metadata; the buffers
    /// are not).
    #[test]
    fn two_apps_shared_pool_interleaved_event_isolation() {
        register_event::<MwEvent>(130);

        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let wc = pool.worker_count();

        let reads_a = Arc::new(AtomicU32::new(0));
        let reads_b = Arc::new(AtomicU32::new(0));

        let mut a = App::with_pool(Arc::clone(&pool));
        let mut b = App::with_pool(Arc::clone(&pool));

        // H4 contract: worker lanes 0..wc plus the dispatcher lane at index wc.
        a.world_mut()
            .preregister_event::<MwEvent>(EventConfig::default_for(wc + 1).expect("valid cfg"))
            .expect("preregister in A");
        b.world_mut()
            .preregister_event::<MwEvent>(EventConfig::default_for(wc + 1).expect("valid cfg"))
            .expect("preregister in B");

        // A: one send per frame + a counting reader. B: a counting reader only.
        a.add_systems(move |mut w: EventWriter<MwEvent>| {
            w.send(MwEvent { value: 7 }).expect("send in A");
        });
        let ra = Arc::clone(&reads_a);
        a.add_systems(move |mut r: EventReader<MwEvent>| {
            ra.fetch_add(r.read().count() as u32, Ordering::Relaxed);
        });
        let rb = Arc::clone(&reads_b);
        b.add_systems(move |mut r: EventReader<MwEvent>| {
            rb.fetch_add(r.read().count() as u32, Ordering::Relaxed);
        });

        // Interleaved frames on the ONE pool. Double-buffer rhythm: a send in
        // frame k becomes readable in frame k+1, each event read exactly once.
        for _ in 0..4 {
            a.update_with_delta(STEP);
            b.update_with_delta(STEP);
        }

        assert_eq!(
            reads_a.load(Ordering::Relaxed),
            3,
            "A's reader sees A's frame-1..3 sends exactly once each (frame-4's send is \
             still in the write buffer)"
        );
        assert_eq!(
            reads_b.load(Ordering::Relaxed),
            0,
            "an event sent in world A is NEVER visible in world B"
        );
    }

    // ── 4 — same States type, independent transitions ────────────────────────

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    enum MwState {
        #[default]
        Menu,
        Game,
    }
    impl States for MwState {}

    /// The SAME `States` type registered in two worlds: `State<S>` /
    /// `NextState<S>` are per-world resources (the generic-resource TypeId
    /// registry is global metadata; the values are world-owned), so a
    /// transition requested in A leaves B untouched.
    #[test]
    fn same_state_type_independent_transitions_across_worlds() {
        let mut a = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());
        let mut b = App::with_pool(ThreadPoolBuilder::new().num_threads(1).build());

        a.init_state::<MwState>();
        b.init_state::<MwState>();

        // A requests Menu → Game every frame; B never requests anything.
        a.add_systems(move |mut next: ResMut<NextState<MwState>>| {
            next.set(MwState::Game);
        });

        a.finish();
        b.finish();
        for _ in 0..2 {
            a.update_with_delta(STEP);
            b.update_with_delta(STEP);
        }

        assert_eq!(
            *a.world().state::<MwState>(),
            MwState::Game,
            "A's requested transition applied"
        );
        assert_eq!(
            *b.world().state::<MwState>(),
            MwState::Menu,
            "B keeps its own value — the same States type is per-world state"
        );
    }
}
