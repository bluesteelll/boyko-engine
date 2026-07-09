//! Task #9 — behavioral integration tests for `Option<D>` / `AnyOf<(..)>`
//! query data (non-filtering optional data + the OR-combinator).
//!
//! Spec: `docs/OPTION-ANYOF-PLAN.md` (Decisions 1–8 + the "Gates" section).
//! These tests verify the public, out-of-crate reachable behavior; the
//! per-impl unit tests live alongside the impls in
//! `crates/boyko_ecs/src/ecs/core/iters/query/data.rs`.
//!
//! # Component-id reservation
//!
//! Slots 363..=378 — verified disjoint from every existing crate-wide
//! allocation at write time (used: 200-203, 260-339[gaps], 340, 360-362,
//! 380-417, 420-422, 440-446, 450-511). `MAX_COMPONENTS = 512` so every id
//! is in range. Hand-written `Component` impls (NOT the derive's auto-mint)
//! keep the ids deterministic across the shared lib-test process — the
//! auto-mint `NEXT_ID` counter starts at 0 and would otherwise race these.
//!
//! # Change-detection note (Decision 2 / behavioral test #4)
//!
//! `EcsMaster::query<D, F>()` rejects any change-detection `D`/`F` at compile
//! time (the W4 const-assert). So `Option<Ref<A>>` / `Option<Mut<A>>` and
//! `Option<&A>` combined with a `Changed<X>` FILTER are exercised through the
//! `Query<D, F>` SystemParam inside a `Schedule`, never through the direct
//! `query` API.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::core::iters::query::{Changed, Enabled, Mut, Query, Ref, Without};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_threadpool::ThreadPoolBuilder;

// ── Component fixtures (slots 363..=378) ───────────────────────────────────

macro_rules! comp {
    ($name:ident, $id:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy, PartialEq, Debug)]
        struct $name(u32);
        impl Component for $name {
            fn component_id() -> ComponentId {
                ComponentId($id)
            }
        }
    };
}

comp!(A, 363);
comp!(B, 364);
comp!(C, 365);
comp!(X, 366);

fn register_all() {
    register_layout::<A>(A::component_id().0);
    register_layout::<B>(B::component_id().0);
    register_layout::<C>(C::component_id().0);
    register_layout::<X>(X::component_id().0);
}

/// A typed enable tag for the `Enabled<TypedFlag>` filter. Its `component_id()`
/// is the bitset-classified id of an enable tag registered once via
/// `prime_typed_flag` (mirrors `enable_tag_step9.rs::TypedFlag`), so the typed
/// `Enabled<TypedFlag>` filter sees a genuine `StorageKind::Bitset`. The typed
/// `Enabled<T>` filter is what sets `F::IS_SOLE_SINGLE_ENABLE = true` — the
/// trigger for the Decision-4 candidate-seed bug (the dynamic `with_enabled`
/// term does NOT seed the candidate path).
struct TypedFlag;

static TYPED_FLAG_ID: std::sync::OnceLock<ComponentId> = std::sync::OnceLock::new();

impl Component for TypedFlag {
    fn component_id() -> ComponentId {
        *TYPED_FLAG_ID
            .get()
            .expect("call prime_typed_flag() before using Enabled<TypedFlag>")
    }
}

/// Idempotent across the shared lib-test process via the `OnceLock`.
fn prime_typed_flag(ecs: &mut EcsMaster) -> boyko_ecs::ecs::core::component::component_registry::EnableTagId {
    let tag = ecs.register_enable_tag("option_anyof_typed_flag");
    let _ = TYPED_FLAG_ID.set(tag.component_id());
    tag
}

// ── Spawn helpers ──────────────────────────────────────────────────────────

/// Spawn an entity into `arch` with the given component byte payloads.
fn spawn(ecs: &mut EcsMaster, arch: ArchetypeId, parts: &[(ComponentId, &[u8])]) -> Entity {
    ecs.create_entity(arch, parts).expect("create_entity must succeed")
}

fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is `#[repr(C)] Copy` POD; reading its bytes yields a valid
    //   byte slice scoped to the borrow of `v`.
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

// ════════════════════════════════════════════════════════════════════════
// #1 — Option Some/None per archetype: Query<(&A, Option<&B>)>
// ════════════════════════════════════════════════════════════════════════

#[test]
fn option_some_none_per_archetype() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_ab = ecs.create_archetype(&[A::component_id(), B::component_id()]);
    let arch_a = ecs.create_archetype(&[A::component_id()]);

    // {A,B} rows: a in {1,2}, b = a*100.
    for a in 1u32..=2 {
        let av = A(a);
        let bv = B(a * 100);
        spawn(&mut ecs, arch_ab, &[
            (A::component_id(), bytes_of(&av)),
            (B::component_id(), bytes_of(&bv)),
        ]);
    }
    // {A}-only rows: a in {10,11}.
    for a in 10u32..=11 {
        let av = A(a);
        spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);
    }

    // Brute-force oracle: every A-bearing entity yields (a, Some(b)) iff B is
    // also present (a in 1..=2 → Some(a*100)), else (a, None).
    let view = ecs.query::<(&A, Option<&B>), ()>();
    let mut got: Vec<(u32, Option<u32>)> = view
        .iter()
        .map(|(a, ob): (&A, Option<&B>)| (a.0, ob.map(|b| b.0)))
        .collect();
    got.sort();

    let expected: Vec<(u32, Option<u32>)> =
        vec![(1, Some(100)), (2, Some(200)), (10, None), (11, None)];
    assert_eq!(got, expected, "Option<&B> must be Some iff B present, per row");
}

// ════════════════════════════════════════════════════════════════════════
// #2 — sole Option<&A> visits ALL archetypes
// ════════════════════════════════════════════════════════════════════════

#[test]
fn option_sole_matches_all() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A::component_id()]);
    // A C-only archetype: A absent → Option<&A> must be None there.
    let arch_c = ecs.create_archetype(&[C::component_id()]);

    let av = A(7);
    spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);
    let cv = C(9);
    spawn(&mut ecs, arch_c, &[(C::component_id(), bytes_of(&cv))]);

    let view = ecs.query::<Option<&A>, ()>();
    let mut somes = 0usize;
    let mut nones = 0usize;
    for oa in view.iter() {
        match oa {
            Some(a) => {
                assert_eq!(a.0, 7, "the only A-present row carries A(7)");
                somes += 1;
            }
            None => nones += 1,
        }
    }
    assert_eq!(somes, 1, "exactly one A-present row → Some");
    assert_eq!(nones, 1, "the C-only row is visited and yields None (sole Option matches ALL archetypes)");
}

// ════════════════════════════════════════════════════════════════════════
// #3 — Option<&mut A> writes through Some via iter_mut; None archetypes skipped
// ════════════════════════════════════════════════════════════════════════

#[test]
fn option_mut_write() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A::component_id()]);
    let arch_c = ecs.create_archetype(&[C::component_id()]);

    let av = A(5);
    let e_a = spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);
    let cv = C(3);
    spawn(&mut ecs, arch_c, &[(C::component_id(), bytes_of(&cv))]);

    {
        let mut view = ecs.query::<(Option<&mut A>,), ()>();
        for (oa,) in view.iter_mut() {
            if let Some(a) = oa {
                a.0 += 1000;
            }
        }
    }

    // Write landed on the A-present row only; the C-only row was a None (skipped).
    assert_eq!(
        ecs.get_component::<A>(e_a).expect("A present").0,
        1005,
        "iter_mut wrote through Some(&mut A)"
    );
}

// ════════════════════════════════════════════════════════════════════════
// #4 — Option<Ref<A>>/Option<Mut<A>> change ticks fire ONLY for present-A
//      rows; Option<&A> + Changed<X> FILTER (F::NCD forces meta) reads OK.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn option_ref_change_detection_present_only() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch_a = world.create_archetype(&[A::component_id()]);
    let arch_c = world.create_archetype(&[C::component_id()]);

    world.spawn_one(arch_a, A(1)).expect("spawn A");
    let cv = C(2);
    world
        .create_entity(arch_c, &[(C::component_id(), bytes_of(&cv))])
        .expect("spawn C");

    // Count rows where Option<Ref<A>> is Some AND is_added() (freshly spawned).
    static SOME_ADDED: AtomicUsize = AtomicUsize::new(0);
    static NONE_ROWS: AtomicUsize = AtomicUsize::new(0);
    SOME_ADDED.store(0, Ordering::Relaxed);
    NONE_ROWS.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<Option<Ref<A>>, ()>| {
        for oa in &q {
            match oa {
                Some(r) => {
                    if r.is_added() {
                        SOME_ADDED.fetch_add(1, Ordering::Relaxed);
                    }
                }
                None => {
                    NONE_ROWS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: A-present row is freshly added → Some + is_added; C-only row → None.
    schedule.run(&mut world);
    assert_eq!(
        SOME_ADDED.load(Ordering::Relaxed),
        1,
        "frame 1: the A-present row yields Some(Ref) with is_added() true"
    );
    assert_eq!(
        NONE_ROWS.load(Ordering::Relaxed),
        1,
        "frame 1: the C-only row yields None (change ticks never touched for None rows)"
    );
}

#[test]
fn option_mut_change_detection_fires_on_present_write() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch_a = world.create_archetype(&[A::component_id()]);
    world.spawn_one(arch_a, A(0)).expect("spawn A");

    static WROTE: AtomicU32 = AtomicU32::new(0);
    static CHANGED_SEEN: AtomicU32 = AtomicU32::new(0);
    WROTE.store(0, Ordering::Relaxed);
    CHANGED_SEEN.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    // Writer through Option<Mut<A>>: bump the present row.
    builder.add_system(|mut q: Query<Option<Mut<A>>, ()>| {
        for oa in &mut q {
            // Present rows only — None archetypes are skipped. `let else`
            // keeps the per-row Some/None intent explicit (clippy would
            // suggest `.flatten()`, which would erase the skip-None semantics
            // the test is asserting).
            let Some(mut a) = oa else { continue };
            a.0 = a.0.wrapping_add(1);
            WROTE.fetch_add(1, Ordering::Relaxed);
        }
    });
    // Reader: Changed<A> must observe the write (present rows only).
    builder.add_system(|q: Query<&A, Changed<A>>| {
        for _ in &q {
            CHANGED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(WROTE.load(Ordering::Relaxed), 1, "writer mutated the one present A row via Option<Mut<A>>");
    assert_eq!(
        CHANGED_SEEN.load(Ordering::Relaxed),
        1,
        "Changed<A> observes the Option<Mut<A>> write (deref bumped the row's changed tick)"
    );
}

#[test]
fn option_ref_with_changed_filter_meta_path_reads_correctly() {
    // F::NCD = true (Changed<X> filter) forces the meta set_table path even
    // though Option<&A>'s inner D::NCD = false (Decision 2). The query must
    // still read Option<&A> correctly. The (A, X) archetype's freshly-spawned
    // rows are "Changed" on frame 1.
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch_ax = world.create_archetype(&[A::component_id(), X::component_id()]);

    let av = A(42);
    let xv = X(0);
    world
        .create_entity(arch_ax, &[
            (A::component_id(), bytes_of(&av)),
            (X::component_id(), bytes_of(&xv)),
        ])
        .expect("spawn (A,X)");

    static SUM: AtomicU32 = AtomicU32::new(0);
    static ROWS: AtomicUsize = AtomicUsize::new(0);
    SUM.store(0, Ordering::Relaxed);
    ROWS.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<Option<&A>, Changed<X>>| {
        for oa in &q {
            ROWS.fetch_add(1, Ordering::Relaxed);
            if let Some(a) = oa {
                SUM.fetch_add(a.0, Ordering::Relaxed);
            }
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(ROWS.load(Ordering::Relaxed), 1, "Changed<X> matches the freshly-spawned (A,X) row on frame 1");
    assert_eq!(
        SUM.load(Ordering::Relaxed),
        42,
        "Option<&A> reads correctly through the F::NCD meta path"
    );
}

// ════════════════════════════════════════════════════════════════════════
// #5 — AnyOf<(&A, &B)> matches A-or-B; every row >=1 Some; brute-force oracle
// ════════════════════════════════════════════════════════════════════════

#[test]
fn anyof_or_match() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A::component_id()]);
    let arch_b = ecs.create_archetype(&[B::component_id()]);
    let arch_ab = ecs.create_archetype(&[A::component_id(), B::component_id()]);
    // A C-only archetype must NOT be visited (neither A nor B).
    let arch_c = ecs.create_archetype(&[C::component_id()]);

    let av = A(1);
    spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);
    let bv = B(2);
    spawn(&mut ecs, arch_b, &[(B::component_id(), bytes_of(&bv))]);
    let av2 = A(3);
    let bv2 = B(4);
    spawn(&mut ecs, arch_ab, &[
        (A::component_id(), bytes_of(&av2)),
        (B::component_id(), bytes_of(&bv2)),
    ]);
    let cv = C(99);
    spawn(&mut ecs, arch_c, &[(C::component_id(), bytes_of(&cv))]);

    let view = ecs.query::<AnyOf<(&A, &B)>, ()>();
    let mut got: Vec<(Option<u32>, Option<u32>)> = Vec::new();
    for (oa, ob) in view.iter() {
        // Decision 3 / the >=1-member guarantee: no (None, None) row.
        assert!(
            oa.is_some() || ob.is_some(),
            "AnyOf must never yield a (None, None) row"
        );
        got.push((oa.map(|a| a.0), ob.map(|b| b.0)));
    }
    got.sort();

    // Oracle: A-only → (Some, None); B-only → (None, Some); AB → (Some, Some).
    // The C-only archetype is never visited.
    let expected = vec![
        (Some(1), None),
        (Some(3), Some(4)),
        (None, Some(2)),
    ];
    let mut expected_sorted = expected.clone();
    expected_sorted.sort();
    assert_eq!(got, expected_sorted, "AnyOf<(&A,&B)> brute-force oracle");
}

// ════════════════════════════════════════════════════════════════════════
// #6 — AnyOf<(&B,&C)> + Enabled<X> over an X-present archetype lacking both
//      B and C yields NO (None,None) row (Decision 4 regression).
// ════════════════════════════════════════════════════════════════════════

#[test]
fn anyof_enabled_no_phantom() {
    register_all();
    let mut ecs = EcsMaster::new();
    let tag = prime_typed_flag(&mut ecs);

    // An archetype with A only (NEITHER B nor C). Enable bits live in a
    // separate per-row enable column toggled via `enable_id`, NOT in the
    // archetype signature (mirrors `enable_tag_step9.rs`).
    let arch = ecs.create_archetype(&[A::component_id()]);
    let av = A(1);
    let e = spawn(&mut ecs, arch, &[(A::component_id(), bytes_of(&av))]);
    ecs.enable_id(e, tag);

    // Decision 4: typed `Enabled<TypedFlag>` sets F::IS_SOLE_SINGLE_ENABLE,
    // which (with D::HAS_DATA_COMPONENT = false for AnyOf) would seed the
    // candidate path and SKIP post_filter_matched — UNLESS AnyOf's
    // REQUIRES_POST_FILTER_TRIM = true keeps it off that path so the
    // >=1-member OR-trim runs and the B/C-less archetype is culled.
    let view = ecs.query::<AnyOf<(&B, &C)>, Enabled<TypedFlag>>();
    let mut rows = 0usize;
    for (ob, oc) in view.iter() {
        assert!(
            ob.is_some() || oc.is_some(),
            "AnyOf<(&B,&C)> + Enabled<X> must never yield (None,None)"
        );
        rows += 1;
    }
    assert_eq!(rows, 0, "an enable-present archetype lacking both B and C must yield ZERO rows (no phantom)");
}

// ════════════════════════════════════════════════════════════════════════
// #7 — (AnyOf<(&B,&C)>,) [1-tuple], Enabled<X>: NO (None,) phantom row.
//      THE M1 regression — must pass now; would have failed before the fix.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn anyof_enabled_tuple_no_phantom() {
    register_all();
    let mut ecs = EcsMaster::new();
    let tag = prime_typed_flag(&mut ecs);

    let arch = ecs.create_archetype(&[A::component_id()]);
    let av = A(1);
    let e = spawn(&mut ecs, arch, &[(A::component_id(), bytes_of(&av))]);
    ecs.enable_id(e, tag);

    // AnyOf wrapped in a 1-tuple. Before the M1 fix the tuple's
    // REQUIRES_POST_FILTER_TRIM fell back to the trait default `false`,
    // re-seeding the candidate path, skipping post_filter_matched, and yielding
    // a phantom (None,) row. The tuple's OR-fold of element
    // REQUIRES_POST_FILTER_TRIM closes that hole.
    let view = ecs.query::<(AnyOf<(&B, &C)>,), Enabled<TypedFlag>>();
    let mut rows = 0usize;
    for (anyof,) in view.iter() {
        let (ob, oc) = anyof;
        assert!(
            ob.is_some() || oc.is_some(),
            "(AnyOf<(&B,&C)>,) + Enabled<X> must never yield a (None,) phantom"
        );
        rows += 1;
    }
    assert_eq!(
        rows, 0,
        "1-tuple-wrapped AnyOf + Enabled<X> must yield ZERO rows (M1 phantom-row fix)"
    );
}

// ════════════════════════════════════════════════════════════════════════
// #8 — degenerate combos (Decision 7): Without<A> ⇒ always None;
//      Changed<A> ⇒ always Some.
// ════════════════════════════════════════════════════════════════════════

#[test]
fn option_without_a_always_none() {
    register_all();
    let mut ecs = EcsMaster::new();
    // C-only archetype — A is absent and Without<A> admits it.
    let arch_c = ecs.create_archetype(&[C::component_id()]);
    // Also an A-archetype — Without<A> must exclude it entirely.
    let arch_a = ecs.create_archetype(&[A::component_id()]);

    let cv = C(1);
    spawn(&mut ecs, arch_c, &[(C::component_id(), bytes_of(&cv))]);
    let av = A(2);
    spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);

    let view = ecs.query::<Option<&A>, Without<A>>();
    let mut rows = 0usize;
    for oa in view.iter() {
        assert!(oa.is_none(), "Option<&A> under Without<A> is always None");
        rows += 1;
    }
    assert_eq!(rows, 1, "Without<A> admits only the C-only archetype (A-archetype excluded)");
}

#[test]
fn option_changed_a_always_some() {
    register_all();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch_a = world.create_archetype(&[A::component_id()]);
    world.spawn_one(arch_a, A(5)).expect("spawn A");

    static ROWS: AtomicUsize = AtomicUsize::new(0);
    static NONES: AtomicUsize = AtomicUsize::new(0);
    ROWS.store(0, Ordering::Relaxed);
    NONES.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<Option<&A>, Changed<A>>| {
        for oa in &q {
            ROWS.fetch_add(1, Ordering::Relaxed);
            if oa.is_none() {
                NONES.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: freshly-spawned A row is "Changed"; Changed<A> trims to A-present
    // rows, so Option<&A> is always Some.
    schedule.run(&mut world);
    assert_eq!(ROWS.load(Ordering::Relaxed), 1, "Changed<A> matches the freshly-spawned A row");
    assert_eq!(NONES.load(Ordering::Relaxed), 0, "Option<&A> under Changed<A> is always Some");
}

// ════════════════════════════════════════════════════════════════════════
// #9 — aliasing B0002: AnyOf<(&mut A, &A)> and (&mut A, Option<&A>) MUST panic;
//      AnyOf<(&A, &A)> (read+read) is legal.
// ════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic(expected = "boyko-B0002")]
fn anyof_mut_a_read_a_aliasing_panics() {
    register_all();
    let mut ecs = EcsMaster::new();
    let _arch = ecs.create_archetype(&[A::component_id()]);
    // The intra-system write-vs-read conflict on A is detected during the
    // SystemParam init_access walk, before the closure body runs.
    ecs.run_closure_once(|mut q: Query<AnyOf<(&mut A, &A)>>| {
        for _ in q.iter_mut() {}
    });
}

#[test]
#[should_panic(expected = "boyko-B0002")]
fn tuple_mut_a_option_read_a_aliasing_panics() {
    register_all();
    let mut ecs = EcsMaster::new();
    let _arch = ecs.create_archetype(&[A::component_id()]);
    ecs.run_closure_once(|mut q: Query<(&mut A, Option<&A>)>| {
        for _ in q.iter_mut() {}
    });
}

#[test]
fn anyof_read_a_read_a_is_legal() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A::component_id()]);
    let av = A(7);
    spawn(&mut ecs, arch_a, &[(A::component_id(), bytes_of(&av))]);

    // read+read aliasing is legal (no conflict) — both arms see A(7).
    let view = ecs.query::<AnyOf<(&A, &A)>, ()>();
    let mut rows = 0usize;
    for (a0, a1) in view.iter() {
        assert_eq!(a0.map(|a| a.0), Some(7), "first arm reads A(7)");
        assert_eq!(a1.map(|a| a.0), Some(7), "second arm reads A(7)");
        rows += 1;
    }
    assert_eq!(rows, 1, "AnyOf<(&A,&A)> read+read visits the A row");
}

// ════════════════════════════════════════════════════════════════════════
// #10 — get on sole Query<Option<&A>>: T-less entity → Some(None);
//       present entity → Some(Some(&a)).
// ════════════════════════════════════════════════════════════════════════

#[test]
fn get_sole_option_double_wrap() {
    register_all();
    let mut ecs = EcsMaster::new();
    let arch_a = ecs.create_archetype(&[A::component_id()]);
    let arch_c = ecs.create_archetype(&[C::component_id()]);

    let e_a = ecs.spawn_one(arch_a, A(11)).expect("spawn A");
    let cv = C(0);
    let e_c = spawn(&mut ecs, arch_c, &[(C::component_id(), bytes_of(&cv))]);

    let view = ecs.query::<Option<&A>, ()>();

    // sole Option<&A> matches ALL archetypes, so get returns Some(D::Item) =
    // Some(Option<&A>) for any live entity in a matched (= any) archetype.
    let present = view.get(e_a);
    assert!(present.is_some(), "get on a matched-archetype entity returns Some(..)");
    let inner = present.expect("outer Some");
    assert_eq!(inner.map(|a| a.0), Some(11), "present entity → Some(Some(&A(11)))");

    let absent = view.get(e_c);
    assert!(absent.is_some(), "the C-only entity is still in a matched archetype (sole Option matches all)");
    assert!(
        absent.expect("outer Some").is_none(),
        "T-less entity → Some(None) (outer Some = archetype matched; inner None = A absent)"
    );
}
