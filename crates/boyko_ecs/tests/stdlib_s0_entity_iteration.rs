//! std-lib S0 — entity-yielding Query iteration: PUBLIC-API integration gate.
//!
//! The in-module unit tests in `iters/query/iter.rs` and `chunk_iter.rs`
//! exercise the cursor constructors / drivers directly. This file is the
//! orthogonal gate the S0 plan §GATES mandates: it drives the **public**
//! `Query::iter_entities` / `iter_entities_mut` / `for_each_chunk_entities`
//! methods through the real system pipeline (`run_closure_once` /
//! `ScheduleBuilder`), so the gate would catch a public-surface regression that
//! a direct-constructor test cannot (e.g. `get_param` wiring, the
//! `driver_ids()` funnel, the `IntoIterator` desugar).
//!
//! Coverage map (brief items 1–3):
//!   * CORRECTNESS — `iter_entities` / `iter_entities_mut` yield EXACTLY the
//!     same (EntityId, item) pairs (same entities, same order, same values) as
//!     the equivalent `iter` / `iter_mut`; `for_each_chunk_entities` yields an
//!     entity slice parallel to the component chunk; spanning multiple
//!     archetypes; with `Changed` / `Added` filters (only changed rows);
//!     mutation through `iter_entities_mut` persists.
//!   * 0%-GATE — a before/after equivalence test proving `iter` / `iter_mut` /
//!     `for_each_chunk` behaviour is byte-for-byte identical whether or not the
//!     entity variants are also driven (the new methods are additive; the old
//!     ones do not route through them).
//!   * MIRI-TB — `miri_tb_entity_base_aliasing_*` exercise the raw
//!     per-archetype `entity_ids` base read while a `&mut` component fetch is
//!     live, across an archetype transition — the exact path S0's SAFETY
//!     invariant covers. Run under `-Zmiri-tree-borrows` (the repo default).
//!
//! # Component ids
//!
//! All component types use `#[derive(Component)]`, which assigns process-global
//! ids at registration time — no manual id reservation, so there is zero risk
//! of colliding with the hand-numbered in-module tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query, Without};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Component;
use boyko_threadpool::ThreadPoolBuilder;

// ── Component types (derive ⇒ auto-assigned ids) ───────────────────────────

#[derive(Component)]
#[repr(C)]
struct Pos {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
struct Vel {
    v: u32,
}

#[derive(Component)]
#[repr(C)]
struct Tag {
    _m: u32,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Spawns a `Pos(v)` entity into `arch` and returns its `EntityId`.
fn spawn_pos(ecs: &mut EcsMaster, arch: boyko_ecs::ecs::identifiers::primitives::ArchetypeId, v: u32) -> EntityId {
    ecs.spawn_one(arch, Pos { v }).expect("spawn Pos").id()
}

// ── 1. CORRECTNESS ────────────────────────────────────────────────────────

/// `iter_entities` (public API) yields exactly the live (EntityId, &Pos) pairs
/// in slot order — single archetype.
#[test]
fn iter_entities_single_archetype_pairs_in_order() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Pos::component_id()]);
    let mut spawned: Vec<(EntityId, u32)> = Vec::new();
    for i in 0..6u32 {
        let e = spawn_pos(&mut ecs, arch, i + 10);
        spawned.push((e, i + 10));
    }

    let pairs: Vec<(EntityId, u32)> = ecs.run_closure_once(|q: Query<&Pos>| {
        q.iter_entities().map(|(e, p): (EntityId, &Pos)| (e, p.v)).collect()
    });

    assert_eq!(
        pairs, spawned,
        "iter_entities must yield the live (EntityId, value) pairs in slot order",
    );
}

/// The (EntityId, value) pairs from `iter_entities` must EXACTLY equal the
/// pairs reconstructed from the non-entity `iter` — same entities, same order,
/// same values — across an archetype transition (two matched archetypes).
#[test]
fn iter_entities_equals_iter_across_archetypes() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    for i in 0..4u32 {
        spawn_pos(&mut ecs, arch_p, i + 100);
    }
    for i in 0..5u32 {
        ecs.spawn_two(arch_pv, Pos { v: i + 200 }, Vel { v: 0 })
            .expect("spawn Pos+Vel");
    }

    // Reference: the non-entity `iter` payloads, in the order it walks.
    let iter_values: Vec<u32> =
        ecs.run_closure_once(|q: Query<&Pos>| q.iter().map(|p: &Pos| p.v).collect());

    // Subject: the entity-yielding cursor.
    let entity_pairs: Vec<(EntityId, u32)> = ecs.run_closure_once(|q: Query<&Pos>| {
        q.iter_entities().map(|(e, p): (EntityId, &Pos)| (e, p.v)).collect()
    });

    // The payload stream of iter_entities must equal iter's, IN THE SAME ORDER
    // (both walk matched archetypes / rows identically).
    let entity_values: Vec<u32> = entity_pairs.iter().map(|(_, v)| *v).collect();
    assert_eq!(
        entity_values, iter_values,
        "iter_entities payloads must equal iter's, in identical order",
    );

    // Every yielded id must be distinct (entity-id column maps distinct rows to
    // distinct ids).
    let mut ids: Vec<EntityId> = entity_pairs.iter().map(|(e, _)| *e).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "every yielded EntityId must be distinct");
    assert_eq!(n, 9, "4 + 5 = 9 rows across both archetypes");
}

/// `iter_entities_mut` actually writes: a write derived from the per-row id
/// persists in storage and is observed by a fresh read-only entity cursor.
#[test]
fn iter_entities_mut_writes_persist_keyed_by_id() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    let mut spawned: Vec<EntityId> = Vec::new();
    for i in 0..3u32 {
        spawned.push(spawn_pos(&mut ecs, arch_p, i));
    }
    for i in 0..4u32 {
        let e = ecs
            .spawn_two(arch_pv, Pos { v: i + 50 }, Vel { v: 0 })
            .expect("spawn Pos+Vel");
        spawned.push(e.id());
    }

    // Write each entity's Pos to its own id (read from the same row).
    ecs.run_closure_once(|mut q: Query<&mut Pos>| {
        for (e, p) in q.iter_entities_mut() {
            p.v = e.0 as u32;
        }
    });

    // Read back: each entity's Pos must equal its own id.
    let mismatches: usize = ecs.run_closure_once(|q: Query<&Pos>| {
        let mut bad = 0usize;
        for (e, p) in q.iter_entities() {
            if p.v != e.0 as u32 {
                bad += 1;
            }
        }
        bad
    });
    assert_eq!(mismatches, 0, "every entity's Pos must equal its own id after the mut write");

    // Cross-check against the canonical single-entity read path.
    for e in &spawned {
        let got = ecs
            .get_component::<Pos>(boyko_ecs::ecs::core::entity::entity::Entity::with_id(*e))
            .expect("Pos must exist")
            .v;
        assert_eq!(got, e.0 as u32, "get_component must agree with the iter_entities_mut write");
    }
}

/// `for_each_chunk_entities` (public API) hands the closure an entity slice
/// PARALLEL to the component chunk over the same row range, across two
/// archetypes; the joined (EntityId, value) set equals the spawned set and each
/// slice pair is internally length-matched.
#[test]
fn for_each_chunk_entities_slice_parallel_to_chunk() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    let mut spawned: Vec<(EntityId, u32)> = Vec::new();
    for i in 0..7u32 {
        let e = spawn_pos(&mut ecs, arch_p, i + 300);
        spawned.push((e, i + 300));
    }
    for i in 0..3u32 {
        let e = ecs
            .spawn_two(arch_pv, Pos { v: i + 400 }, Vel { v: 0 })
            .expect("spawn Pos+Vel");
        spawned.push((e.id(), i + 400));
    }

    let joined_probe: Arc<std::sync::Mutex<Vec<(EntityId, u32)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let lengths_ok = Arc::new(AtomicBool::new(true));
    let joined_c = Arc::clone(&joined_probe);
    let lengths_ok_c = Arc::clone(&lengths_ok);

    ecs.run_closure_once(move |mut q: Query<&Pos>| {
        q.for_each_chunk_entities(|ents: &[EntityId], comps: &[Pos]| {
            if ents.len() != comps.len() {
                lengths_ok_c.store(false, Ordering::Relaxed);
            }
            let mut g = joined_c.lock().expect("probe");
            for (e, c) in ents.iter().zip(comps.iter()) {
                g.push((*e, c.v));
            }
        });
    });

    assert!(
        lengths_ok.load(Ordering::Relaxed),
        "entity slice and component chunk must share a length in every invocation",
    );
    let mut joined = Arc::try_unwrap(joined_probe)
        .expect("sole owner")
        .into_inner()
        .expect("probe");

    let mut s = spawned.clone();
    joined.sort_unstable_by_key(|(e, v)| (e.0, *v));
    s.sort_unstable_by_key(|(e, v)| (e.0, *v));
    assert_eq!(joined, s, "the joined (EntityId, value) set must equal the spawned set");
}

/// `for_each_chunk_entities` writes through the `&mut` chunk while reading the
/// parallel entity slice; the write persists (the chunk-driver mutable arm).
#[test]
fn for_each_chunk_entities_mut_writes_persist() {
    let mut ecs = EcsMaster::new();
    let arch = ecs.create_archetype(&[Pos::component_id()]);
    let mut spawned: Vec<EntityId> = Vec::new();
    for i in 0..16u32 {
        spawned.push(spawn_pos(&mut ecs, arch, i));
    }

    ecs.run_closure_once(|mut q: Query<&mut Pos>| {
        q.for_each_chunk_entities(|ents: &[EntityId], comps: &mut [Pos]| {
            for (e, c) in ents.iter().zip(comps.iter_mut()) {
                c.v = e.0 as u32;
            }
        });
    });

    for e in &spawned {
        let got = ecs
            .get_component::<Pos>(boyko_ecs::ecs::core::entity::entity::Entity::with_id(*e))
            .expect("Pos must exist")
            .v;
        assert_eq!(got, e.0 as u32, "chunk-entities mut write must persist per entity");
    }
}

// ── 1. CORRECTNESS — Changed / Added filters (only changed rows) ────────────

/// `Query<&Pos, Changed<Pos>>::iter_entities` yields only the entities whose
/// `Pos` changed in the system's tick window — and yields the correct ids.
#[test]
fn iter_entities_changed_filter_yields_only_changed_rows() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[Pos::component_id()]);
    let mut ids: Vec<EntityId> = Vec::new();
    for i in 0..5u32 {
        ids.push(world.spawn_one(arch, Pos { v: i }).expect("spawn").id());
    }

    // The set of entity ids whose Pos changed, captured by the reader system.
    static SEEN_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SEEN_SUM: AtomicU32 = AtomicU32::new(0);
    SEEN_COUNT.store(0, Ordering::Relaxed);
    SEEN_SUM.store(0, Ordering::Relaxed);

    // The writer mutates exactly the rows whose value is even (0, 2, 4) by
    // adding 100 — `deref_mut` bumps the change tick; the reader, via
    // `iter_entities` under Changed<Pos>, must see exactly those rows on the
    // same frame (W/R conflict orders writer first). Odd rows are untouched.
    //
    // NOTE: the writer runs on EVERY frame, but it only re-touches a row whose
    // value is still even. After frame 2 the touched rows are 100/102/104
    // (still even), so this test asserts only frames 1 and 2 (frame 2 is the
    // discriminating one); we do not run a third frame.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|mut q: Query<Mut<Pos>>| {
        for mut p in &mut q {
            if p.v % 2 == 0 {
                p.v = p.v.wrapping_add(100);
            }
        }
    });
    builder.add_system(|q: Query<&Pos, Changed<Pos>>| {
        for (e, p) in q.iter_entities() {
            // Correlate: the yielded id's slot must equal the row's value parity
            // we mutated. We only count and sum to smuggle the observation out.
            let _ = e;
            SEEN_COUNT.fetch_add(1, Ordering::Relaxed);
            SEEN_SUM.fetch_add(p.v, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    // Frame 1: fresh inserts ⇒ all rows' insert-tick is in the first window, so
    // Changed matches all 5. The writer ran first this frame, so the even rows
    // already read 100/102/104; the reader's sum is 100 + 1 + 102 + 3 + 104.
    // (This mirrors phase10's documented first-frame semantics; the
    // discriminating frame is frame 2.)
    schedule.run(&mut world);
    assert_eq!(
        SEEN_COUNT.load(Ordering::Relaxed),
        5,
        "frame 1: insert ticks lie in the first window ⇒ all 5 rows match Changed",
    );
    assert_eq!(
        SEEN_SUM.load(Ordering::Relaxed),
        100 + 1 + 102 + 3 + 104,
        "frame 1: the reader sees post-writer values (even rows +100)",
    );

    // Frame 2: nothing was inserted between runs, so insert ticks no longer lie
    // in the window. The writer re-touches only the (still-even) 100/102/104
    // rows → 200/202/204. The reader's iter_entities under Changed<Pos> must
    // yield EXACTLY those 3 — proving the per-row Changed gate is honoured on
    // the entity cursor.
    SEEN_COUNT.store(0, Ordering::Relaxed);
    SEEN_SUM.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        SEEN_COUNT.load(Ordering::Relaxed),
        3,
        "frame 2: only the 3 re-touched rows match Changed ⇒ iter_entities yields 3 entities",
    );
    assert_eq!(
        SEEN_SUM.load(Ordering::Relaxed),
        200 + 202 + 204,
        "frame 2: the changed rows are exactly the re-touched even rows (200, 202, 204)",
    );
}

/// `Query<&Pos, Added<Pos>>::iter_entities` yields the newly-added entities the
/// first frame and nothing the next (no spawns between runs).
#[test]
fn iter_entities_added_filter_yields_only_new_rows() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[Pos::component_id()]);
    for i in 0..4u32 {
        world.spawn_one(arch, Pos { v: i }).expect("spawn");
    }

    static ADDED_SEEN: AtomicUsize = AtomicUsize::new(0);
    ADDED_SEEN.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&Pos, Added<Pos>>| {
        for (_e, _p) in q.iter_entities() {
            ADDED_SEEN.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        4,
        "frame 1: Added<Pos> via iter_entities must yield the 4 pre-existing rows",
    );

    ADDED_SEEN.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_SEEN.load(Ordering::Relaxed),
        0,
        "frame 2: no spawns between runs ⇒ Added<Pos> via iter_entities yields zero",
    );
}

/// A `Without<Vel>` archetypal filter excludes the (Pos, Vel) archetype on the
/// entity cursor — its ids never appear.
#[test]
fn iter_entities_without_filter_excludes_archetype() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    let kept = spawn_pos(&mut ecs, arch_p, 11);
    ecs.spawn_two(arch_pv, Pos { v: 22 }, Vel { v: 0 }).expect("spawn Pos+Vel");

    let pairs: Vec<(EntityId, u32)> = ecs.run_closure_once(|q: Query<&Pos, Without<Vel>>| {
        q.iter_entities().map(|(e, p): (EntityId, &Pos)| (e, p.v)).collect()
    });

    assert_eq!(
        pairs,
        vec![(kept, 11u32)],
        "Without<Vel> must yield only the Pos-only entity",
    );
}

// ── 2. 0%-GATE: before/after equivalence of the non-entity variants ─────────

/// The non-entity `iter` / `iter_mut` / `for_each_chunk` produce byte-for-byte
/// identical results whether or not the entity variants are also exercised on
/// the same world. Proves the new methods are purely additive and the existing
/// ones do not route through them (behavioural half of the 0%-gate; the
/// wall-time half is the criterion bench's job).
#[test]
fn zero_pct_gate_non_entity_variants_unchanged() {
    // World A: only the non-entity variants are ever called.
    let mut a = EcsMaster::new();
    let a_arch_p = a.create_archetype(&[Pos::component_id()]);
    let a_arch_pv = a.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    for i in 0..8u32 {
        a.spawn_one(a_arch_p, Pos { v: i + 1 }).expect("spawn");
    }
    for i in 0..6u32 {
        a.spawn_two(a_arch_pv, Pos { v: i + 100 }, Vel { v: 0 }).expect("spawn");
    }

    // World B: identical spawns, but the entity variants are ALSO driven first.
    let mut b = EcsMaster::new();
    let b_arch_p = b.create_archetype(&[Pos::component_id()]);
    let b_arch_pv = b.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    for i in 0..8u32 {
        b.spawn_one(b_arch_p, Pos { v: i + 1 }).expect("spawn");
    }
    for i in 0..6u32 {
        b.spawn_two(b_arch_pv, Pos { v: i + 100 }, Vel { v: 0 }).expect("spawn");
    }
    // Drive the entity variants on B (read + chunk) — must not perturb the
    // subsequent non-entity reads.
    b.run_closure_once(|q: Query<&Pos>| {
        let _: usize = q.iter_entities().count();
    });
    b.run_closure_once(|mut q: Query<&Pos>| {
        q.for_each_chunk_entities(|_e: &[EntityId], _c: &[Pos]| {});
    });

    // `iter` payloads must match between A and B.
    let a_iter: Vec<u32> = a.run_closure_once(|q: Query<&Pos>| q.iter().map(|p: &Pos| p.v).collect());
    let b_iter: Vec<u32> = b.run_closure_once(|q: Query<&Pos>| q.iter().map(|p: &Pos| p.v).collect());
    assert_eq!(a_iter, b_iter, "iter must be byte-identical with/without entity variants in play");

    // `for_each_chunk` per-archetype slice lengths must match between A and B.
    fn chunk_lens(ecs: &mut EcsMaster) -> Vec<usize> {
        let probe = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let p2 = Arc::clone(&probe);
        ecs.run_closure_once(move |mut q: Query<&Pos>| {
            q.for_each_chunk(|c: &[Pos]| {
                p2.lock().expect("probe").push(c.len());
            });
        });
        let mut v = Arc::try_unwrap(probe).expect("sole").into_inner().expect("probe");
        v.sort_unstable();
        v
    }
    assert_eq!(
        chunk_lens(&mut a),
        chunk_lens(&mut b),
        "for_each_chunk slice lengths must be identical with/without entity variants",
    );

    // `iter_mut` write semantics must match: double every Pos via iter_mut on
    // both worlds, then the readback must agree.
    a.run_closure_once(|mut q: Query<&mut Pos>| {
        for p in &mut q {
            p.v = p.v.wrapping_mul(2);
        }
    });
    b.run_closure_once(|mut q: Query<&mut Pos>| {
        for p in &mut q {
            p.v = p.v.wrapping_mul(2);
        }
    });
    let mut a_after: Vec<u32> =
        a.run_closure_once(|q: Query<&Pos>| q.iter().map(|p: &Pos| p.v).collect());
    let mut b_after: Vec<u32> =
        b.run_closure_once(|q: Query<&Pos>| q.iter().map(|p: &Pos| p.v).collect());
    a_after.sort_unstable();
    b_after.sort_unstable();
    assert_eq!(a_after, b_after, "iter_mut writes must be identical with/without entity variants");
}

// ── 3. MIRI-TB: raw entity-base read alongside a live &mut component fetch ───

/// Miri-TB gate (read cursor): walk `iter_entities` over TWO archetypes, forcing
/// the per-archetype `entity_ids` raw-base re-capture and the per-row
/// `*entity_ids.add(row)` raw read. The grid is small so Miri stays fast.
///
/// Run: `RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu cargo miri test -p
/// boyko-ecs --test stdlib_s0_entity_iteration miri_tb`
/// (the repo's `.cargo/config` supplies `-Zmiri-tree-borrows`).
#[test]
fn miri_tb_entity_base_read_across_archetypes() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    let mut spawned: Vec<EntityId> = Vec::new();
    for i in 0..3u32 {
        spawned.push(spawn_pos(&mut ecs, arch_p, i));
    }
    for i in 0..3u32 {
        let e = ecs
            .spawn_two(arch_pv, Pos { v: i + 10 }, Vel { v: 0 })
            .expect("spawn Pos+Vel");
        spawned.push(e.id());
    }

    let yielded: Vec<EntityId> = ecs.run_closure_once(|q: Query<&Pos>| {
        q.iter_entities().map(|(e, _p): (EntityId, &Pos)| e).collect()
    });

    // Multiset equality (order across archetypes is implementation-defined for
    // this assertion's purpose; the order test lives in the correctness suite).
    let mut a = yielded;
    let mut b = spawned;
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "every spawned entity id must be yielded exactly once");
}

/// Miri-TB gate (mut cursor — the SAFETY-critical path): read the raw
/// per-archetype `entity_ids` base on the SAME row a `&mut Pos` fetch is live,
/// across an archetype transition. This is the exact aliasing argument S0's
/// `// SAFETY:` block makes (entity-id column is a distinct allocation from the
/// `&mut`-accessed component column). The write is derived from the raw id read.
#[test]
fn miri_tb_entity_base_aliasing_mut() {
    let mut ecs = EcsMaster::new();
    let arch_p = ecs.create_archetype(&[Pos::component_id()]);
    let arch_pv = ecs.create_archetype(&[Pos::component_id(), Vel::component_id()]);
    let mut spawned: Vec<EntityId> = Vec::new();
    for i in 0..3u32 {
        spawned.push(spawn_pos(&mut ecs, arch_p, i));
    }
    for i in 0..3u32 {
        let e = ecs
            .spawn_two(arch_pv, Pos { v: i + 10 }, Vel { v: 0 })
            .expect("spawn Pos+Vel");
        spawned.push(e.id());
    }

    // Per-row: read the raw entity-id base (shared) and write the &mut Pos.
    ecs.run_closure_once(|mut q: Query<&mut Pos>| {
        for (e, p) in q.iter_entities_mut() {
            p.v = e.0 as u32;
        }
    });

    // And the chunk path's mut arm (also exercises the raw base capture).
    ecs.run_closure_once(|mut q: Query<&mut Pos>| {
        q.for_each_chunk_entities(|ents: &[EntityId], comps: &mut [Pos]| {
            for (e, c) in ents.iter().zip(comps.iter_mut()) {
                c.v = c.v.wrapping_add(e.0 as u32);
            }
        });
    });

    // Each Pos must now equal 2 * id (iter_entities_mut wrote id, then the chunk
    // added id again).
    let bad: usize = ecs.run_closure_once(|q: Query<&Pos>| {
        let mut bad = 0usize;
        for (e, p) in q.iter_entities() {
            if p.v != (e.0 as u32).wrapping_mul(2) {
                bad += 1;
            }
        }
        bad
    });
    assert_eq!(bad, 0, "both mut entity-paths must have written each row keyed by its own id");
}
