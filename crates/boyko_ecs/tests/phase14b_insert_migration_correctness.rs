//! Targeted correctness coverage for the `migrate_entity_insert` restructure
//! (Phase 14b fix-wave NEW-1).
//!
//! NEW-1 rewrote the insert-migration byte-copy to consume every bundle `&[u8]`
//! INSIDE `Bundle::for_each_component_bytes`'s closure (the prior shape read the
//! slices back AFTER the closure returned, a dangling-reference UAF that
//! Miri-TB aborted on). Because the rewrite touches the heavily-used
//! `cmds.entity(e).insert(New)` migration path, this file pins the three
//! semantics the restructure MUST preserve — semantics the UAF previously
//! corrupted silently:
//!
//!   (a) retained components keep their ORIGINAL change-detection ticks across
//!       the migration (so `Added<Retained>` does NOT re-fire and a stale
//!       `Changed<Retained>` does NOT re-fire after the move);
//!   (b) the newly-added bundle component gets the current tick (so
//!       `Added<New>` DOES fire the frame it is inserted);
//!   (c) bundle-wins-on-overlap: when the inserted bundle carries a component
//!       that ALSO exists in the source (alongside a genuinely new one, so the
//!       op still migrates), the migrated row holds the BUNDLE's bytes, not the
//!       retained source bytes (Q6 replace semantic), and a retained component's
//!       VALUE is copied through byte-exact.
//!
//! # Component-id strategy
//!
//! Each test owns distinct `#[derive(Component)]` types; their `ComponentId`s
//! are minted lazily from the global atomic counter, so they are disjoint from
//! one another and from every other test in the binary regardless of run order.
//!
//! # Change-detection observation model
//!
//! `Added<T>` / `Changed<T>` are evaluated against a system's `last_run` window,
//! which only advances inside `Schedule::run`. These tests therefore drive the
//! migration AND the observation through a multi-frame schedule (the same model
//! as `tests/phase10_change_detection.rs`), smuggling per-frame match counts out
//! of the `Send + Sync` system closures via module-level `static` probes guarded
//! by a process-wide mutex.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

/// Serializes tests that share module-level probe `static`s across schedule
/// frames (the `static` counters are process-wide in the test binary).
static TEST_MUTEX: Mutex<()> = Mutex::new(());

const REL: Ordering = Ordering::Relaxed;

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — tick preservation across insert-migration:
//   Added<Retained> must NOT re-fire; Added<New> MUST fire the migration frame.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MigBase {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MigAdded {
    w: u32,
}

#[derive(Bundle)]
struct MigAddedBundle {
    c: MigAdded,
}

static T1_BASE_ADDED_HITS: AtomicUsize = AtomicUsize::new(0);
static T1_DO_INSERT: AtomicBool = AtomicBool::new(false);

/// The load-bearing tick-preservation invariant the UAF could have corrupted
/// silently: across an insert-migration, a RETAINED component keeps its ORIGINAL
/// added-tick, so `Added<Retained>` does NOT re-fire after the move; the
/// newly-added bundle component is stamped with the CURRENT tick.
///
/// # Why two observation vehicles
///
/// `Added<T>` (the schedule-`Query` filter) uses an EXCLUSIVE `last_run` lower
/// bound. A component inserted via a deferred `Commands` op is stamped at the
/// apply-window of frame N with frame N's `this_run`; the next frame's reader has
/// `last_run == frameN.this_run`, so the freshly-inserted component lands exactly
/// on the exclusive boundary and `Added<MigAdded>` can never observe a
/// deferred-inserted component (this is a pre-existing property of the per-frame
/// single-tick-bump model — it affects deferred `spawn` identically and is NOT a
/// Phase 14b concern). The schedule path is therefore used ONLY for the assertion
/// it CAN make: the RETAINED `MigBase`'s added-tick survives the migration so
/// `Added<MigBase>` stays silent on/after the migration frame.
///
/// The "the new component got the current tick" half is asserted via the
/// direct-API `Mut` guard (`get_component_mut`), whose `is_added` uses an
/// INCLUSIVE lower bound and reads at `last_run == this_run == current_tick`:
/// after the migration frame, `MigAdded.is_added()` is `true` (stamped this
/// tick) while the retained `MigBase.is_added()` is `false` (stamped an earlier
/// frame) — a clean, boundary-free distinction.
#[test]
fn insert_migration_preserves_retained_added_tick_and_fires_new() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    // Spawn {MigBase} before the schedule begins (added-tick == Tick(0)).
    let base_arch = world.create_archetype(&[MigBase::component_id()]);
    let entity = world
        .spawn_one(base_arch, MigBase { v: 7 })
        .expect("spawn base");
    // Pre-register MigAdded so the migration target archetype is resolvable.
    let _ = MigAdded::component_id();

    T1_BASE_ADDED_HITS.store(0, REL);
    T1_DO_INSERT.store(false, REL);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Migrator: on the armed frame, insert MigAdded onto the entity.
    builder.add_system(move |mut cmds: Commands| {
        if T1_DO_INSERT.load(REL) {
            cmds.entity(entity).insert(MigAddedBundle { c: MigAdded { w: 99 } });
        }
    });
    // Reader: count rows whose MigBase added-tick lies in this frame's window.
    builder.add_system(|q: Query<&MigBase, Added<MigBase>>| {
        for _ in &q {
            T1_BASE_ADDED_HITS.fetch_add(1, REL);
        }
    });
    let mut schedule = builder.build(&mut world);

    // ── Frame 1: no insert. MigBase is "newly added" relative to the first
    //    window (Tick(0) in the enormous initial window) → Added<MigBase> fires.
    schedule.run(&mut world);
    assert_eq!(
        T1_BASE_ADDED_HITS.load(REL),
        1,
        "frame 1: Added<MigBase> matches the pre-existing row"
    );

    // ── Frame 2: arm the insert. The migration applies at this frame's
    //    apply-window. MigBase's added-tick is PRESERVED from frame 1 (the
    //    restructure must COPY the original tick, not re-stamp) → Added<MigBase>
    //    stays silent.
    T1_BASE_ADDED_HITS.store(0, REL);
    T1_DO_INSERT.store(true, REL);
    schedule.run(&mut world);
    T1_DO_INSERT.store(false, REL);

    assert!(
        world.has_component(entity, MigAdded::component_id()),
        "frame 2: the insert migrated the entity into {{MigBase, MigAdded}}"
    );
    assert_eq!(
        world.get_component::<MigBase>(entity).expect("MigBase retained").v,
        7,
        "frame 2: retained MigBase value copied through migration byte-exact"
    );
    assert_eq!(
        T1_BASE_ADDED_HITS.load(REL),
        0,
        "frame 2: Added<MigBase> must NOT re-fire — the retained added-tick is preserved across migration"
    );

    // ── Direct-API tick distinction (boundary-free), read AT the migration tick.
    //    No further schedule frame runs (no extra tick bump), so `current_tick`
    //    still equals frame 2's `this_run` == MigAdded's added-tick. The `Mut`
    //    guard's INCLUSIVE-bound `is_added` (built with last_run==this_run==
    //    current_tick) therefore reports the freshly-migrated MigAdded as added
    //    (stamped THIS tick), while the retained MigBase — whose added-tick is
    //    several frames in the past — is NOT added. This is the boundary-free
    //    confirmation that the new component got the current tick AND the retained
    //    tick was preserved (not re-stamped) by the migration byte-copy.
    {
        let m_new = world
            .get_component_mut::<MigAdded>(entity)
            .expect("MigAdded on migrated row");
        assert!(
            m_new.is_added(),
            "MigAdded got the current tick at migration — is_added true (the new component is stamped now)"
        );
    }
    {
        let m_base = world
            .get_component_mut::<MigBase>(entity)
            .expect("MigBase on migrated row");
        assert!(
            !m_base.is_added(),
            "retained MigBase's added-tick predates the current tick — is_added false (tick preserved, not re-stamped)"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — a stale Changed<Retained> must NOT re-fire because of the migration.
//   The migration itself must not bump the retained component's changed-tick.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct ChgBase {
    n: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct ChgAdded {
    m: u32,
}

#[derive(Bundle)]
struct ChgAddedBundle {
    c: ChgAdded,
}

static T2_BASE_CHANGED_HITS: AtomicUsize = AtomicUsize::new(0);
static T2_DO_INSERT: AtomicBool = AtomicBool::new(false);

/// The entity is spawned with `ChgBase` before the schedule. Frame 1 settles
/// the change-detection window (after frame 1, `ChgBase` is no longer "changed"
/// relative to subsequent windows). Frame 2 inserts `ChgAdded` (migration). The
/// migration copies `ChgBase`'s ORIGINAL changed-tick into the target row — it
/// must NOT bump it to the current tick — so `Changed<ChgBase>` stays silent on
/// the migration frame. (A re-stamp here is exactly the silent corruption the
/// UAF could mask.)
#[test]
fn insert_migration_does_not_refire_retained_changed() {
    let _guard = TEST_MUTEX.lock().expect("test mutex");

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    let base_arch = world.create_archetype(&[ChgBase::component_id()]);
    let entity = world.spawn_one(base_arch, ChgBase { n: 3 }).expect("spawn");
    let _ = ChgAdded::component_id();

    T2_BASE_CHANGED_HITS.store(0, REL);
    T2_DO_INSERT.store(false, REL);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |mut cmds: Commands| {
        if T2_DO_INSERT.load(REL) {
            cmds.entity(entity).insert(ChgAddedBundle { c: ChgAdded { m: 1 } });
        }
    });
    builder.add_system(|q: Query<&ChgBase, Changed<ChgBase>>| {
        for _ in &q {
            T2_BASE_CHANGED_HITS.fetch_add(1, REL);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: spawn-tick lies in the first window → Changed<ChgBase> fires.
    schedule.run(&mut world);
    assert_eq!(
        T2_BASE_CHANGED_HITS.load(REL),
        1,
        "frame 1: insert/spawn tick bumps changed; Changed<ChgBase> matches"
    );

    // Frame 2: migrate. The migration must carry ChgBase's ORIGINAL changed-tick
    // (frame-1 tick), which is now BEHIND this frame's window → Changed silent.
    T2_BASE_CHANGED_HITS.store(0, REL);
    T2_DO_INSERT.store(true, REL);
    schedule.run(&mut world);
    T2_DO_INSERT.store(false, REL);

    assert!(
        world.has_component(entity, ChgAdded::component_id()),
        "frame 2: migration happened"
    );
    assert_eq!(
        T2_BASE_CHANGED_HITS.load(REL),
        0,
        "frame 2: migration must NOT bump the retained ChgBase changed-tick — Changed<ChgBase> stays silent"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — bundle-wins-on-overlap VALUE correctness through a migration.
//   Bundle = {Overlap (already present), Extra (new)} so the op MIGRATES (target
//   != source); the migrated row must hold the BUNDLE's Overlap bytes (Q6), and
//   the genuinely-retained component's value is copied through byte-exact.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct OvlKeep {
    keep: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct OvlShared {
    shared: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct OvlExtra {
    extra: u64,
}

/// Bundle carries the ALREADY-PRESENT `OvlShared` plus the genuinely-new
/// `OvlExtra`, so the union {OvlKeep, OvlShared, OvlExtra} differs from the
/// source {OvlKeep, OvlShared} and the op takes the MIGRATION path (not the
/// in-place fast path). The closure overwrites the retained `OvlShared` slot
/// with the bundle value (bundle wins, Q6); `OvlKeep` is a pure retained copy.
#[derive(Bundle)]
struct OverlapBundle {
    s: OvlShared,
    e: OvlExtra,
}

#[test]
fn insert_migration_bundle_wins_on_overlap_value() {
    // No shared static probes here — local value assertions only; no mutex needed.
    let mut world = EcsMaster::new();

    // Source = {OvlKeep, OvlShared}. OvlShared starts at 100, OvlKeep at 7.
    let src_arch =
        world.create_archetype(&[OvlKeep::component_id(), OvlShared::component_id()]);
    let entity = world
        .spawn_two(src_arch, OvlKeep { keep: 7 }, OvlShared { shared: 100 })
        .expect("spawn source");
    let src_arch_id = world.get_entity_archetype_id(entity).expect("src arch id");

    // Insert {OvlShared = 999 (overlap), OvlExtra = 42 (new)}. T = S ∪ {OvlExtra}
    // differs from S → migration path; the overlapping OvlShared must win.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(OverlapBundle {
            s: OvlShared { shared: 999 },
            e: OvlExtra { extra: 42 },
        });
    });

    let new_arch_id = world.get_entity_archetype_id(entity).expect("new arch id");
    assert_ne!(
        src_arch_id, new_arch_id,
        "the new OvlExtra forces a true migration (not the in-place fast path)"
    );

    assert_eq!(
        world.get_component::<OvlShared>(entity).expect("OvlShared present").shared,
        999,
        "bundle wins on overlap: the migrated OvlShared holds the BUNDLE value (999), not the retained 100"
    );
    assert_eq!(
        world.get_component::<OvlKeep>(entity).expect("OvlKeep retained").keep,
        7,
        "the genuinely-retained OvlKeep value is copied through migration byte-exact"
    );
    assert_eq!(
        world.get_component::<OvlExtra>(entity).expect("OvlExtra inserted").extra,
        42,
        "the newly-added OvlExtra holds its bundle value"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — multi-retained byte fidelity: a 3-component source migrates +1; every
//   retained value survives the closure-internal byte copy (NEW-1 single-pass).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FidA {
    a: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FidB {
    b: u64,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FidC {
    c: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct FidNew {
    d: u64,
}

#[derive(Bundle)]
struct FidNewBundle {
    c: FidNew,
}

/// Three distinct retained components of mixed size + a fourth inserted one. The
/// NEW-1 single-pass copy writes each retained component's bytes from the live
/// source pool into the target row before the source row is released; this pins
/// that ALL THREE retained values survive byte-exact (a regression in the copy
/// order would scramble at least one).
#[test]
fn insert_migration_multi_retained_byte_fidelity() {
    let mut world = EcsMaster::new();

    let src_arch = world.create_archetype(&[
        FidA::component_id(),
        FidB::component_id(),
        FidC::component_id(),
    ]);
    // spawn_two only covers 2 components; build the 3-component row directly via
    // a deferred bundle spawn, then read back.
    #[derive(Bundle)]
    struct FidSrcBundle {
        a: FidA,
        b: FidB,
        c: FidC,
    }
    let _ = src_arch; // archetype pre-created so the bundle resolves to it
    let mut spawned: Option<boyko_ecs::ecs::core::entity::entity::Entity> = None;
    world.run_system(|mut cmds: Commands| {
        cmds.spawn(FidSrcBundle {
            a: FidA { a: 0xAAAA_AAAA },
            b: FidB { b: 0xBBBB_BBBB_BBBB_BBBB },
            c: FidC { c: 0xCCCC_CCCC },
        });
    });
    for e in world.iter_entities() {
        spawned = Some(e);
    }
    let entity = spawned.expect("one entity spawned");

    // Migrate +FidNew.
    let _ = FidNew::component_id();
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(FidNewBundle { c: FidNew { d: 0xDDDD_DDDD_DDDD_DDDD } });
    });

    assert_eq!(
        world.get_component::<FidA>(entity).expect("FidA").a,
        0xAAAA_AAAA,
        "retained FidA survives migration byte-exact"
    );
    assert_eq!(
        world.get_component::<FidB>(entity).expect("FidB").b,
        0xBBBB_BBBB_BBBB_BBBB,
        "retained FidB survives migration byte-exact"
    );
    assert_eq!(
        world.get_component::<FidC>(entity).expect("FidC").c,
        0xCCCC_CCCC,
        "retained FidC survives migration byte-exact"
    );
    assert_eq!(
        world.get_component::<FidNew>(entity).expect("FidNew").d,
        0xDDDD_DDDD_DDDD_DDDD,
        "newly-inserted FidNew holds its bundle value"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — direct-API Mut after migration: a write through get_component_mut on
//   a MIGRATED entity bumps the changed-tick and persists (sanity that the
//   migrated row's tick columns are wired and writable).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct PostMutBase {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct PostMutNew {
    w: u32,
}

#[derive(Bundle)]
struct PostMutNewBundle {
    c: PostMutNew,
}

#[test]
fn insert_migration_then_mut_persists_and_marks_changed() {
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[PostMutBase::component_id()]);
    let entity = world
        .spawn_one(arch, PostMutBase { v: 10 })
        .expect("spawn base");
    let _ = PostMutNew::component_id();

    world.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(PostMutNewBundle { c: PostMutNew { w: 5 } });
    });

    // Write through Mut on the migrated row.
    {
        let mut m = world
            .get_component_mut::<PostMutBase>(entity)
            .expect("PostMutBase on migrated row");
        m.v = 77;
        assert!(
            m.is_changed(),
            "a current-tick write through the migrated row's Mut reports is_changed"
        );
    }
    assert_eq!(
        world.get_component::<PostMutBase>(entity).expect("alive").v,
        77,
        "the write through the migrated row persisted"
    );
    assert_eq!(
        world.get_component::<PostMutNew>(entity).expect("new present").w,
        5,
        "the inserted component is intact after the post-migration write"
    );
}
