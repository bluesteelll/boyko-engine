//! Phase 22 Wave 2B — per-driver behavioral tests for dynamic-tag query
//! terms (`with_tag` / `without_tag`; plan D4 disposition table).
//!
//! One behavioral test per driver row: iter, iter_mut, par_iter,
//! par_iter_mut, `Query::for_each_chunk`, `QueryView::for_each_chunk`, both
//! `par_for_each_chunk`, `archetype_count`/`is_empty`, get, get_mut, single
//! (+ single_mut routing) — each on a two-archetype tagged/untagged fixture.
//! Plus: the >MAX_DYN_TAG_TERMS loud panic, with/without combination, and
//! composition with the typed `With<T>` / `Changed<T>` filters.
//!
//! # Fixture note (Wave 2 sequencing)
//!
//! Wave-mate 2A's `add_tag` had NOT landed when this file was first written,
//! so the standard fixtures build tagged entities via the **direct create
//! path** (the documented Wave-1 fallback): `get_or_create_archetype(&[
//! payload_id, tag.component_id()])` + `EcsMaster::create_entity` with a
//! zero-length byte slice for the tag column (ZST pools accept 0-byte writes
//! per Wave 0). `add_tag` HAS since landed: the cross-track seam — a tagged
//! population built through the 2A migration surface (`add_tag` on
//! plain-spawned entities) — is exercised by
//! `iter_honors_terms_on_population_built_via_add_tag`, so both fixture
//! styles are now covered.
//!
//! Lives OUT-OF-CRATE so every assertion proves public reachability of the
//! term surface (`with_tag`/`without_tag` on both `Query` and `QueryView`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::TagId;
use boyko_ecs::ecs::core::iters::query::{BatchingStrategy, Changed, With};
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::prelude::{EcsMaster, Entity, Query, ScheduleBuilder, ThreadPoolBuilder};
use boyko_macros::Component;

// ── Component fixtures ───────────────────────────────────────────────────────

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Payload {
    v: u32,
}

/// Typed marker used by the `With<Marker>` composition test (non-ZST so the
/// test stays focused on the term funnel, not ZST storage).
#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Marker {
    _x: u32,
}

// ── Spawn helpers (direct create path — see fixture note above) ─────────────

/// Spawns a `Payload(v)` entity into `arch`, providing a 0-byte column write
/// for every tag in `tags` (the archetype signature must be covered by the
/// input pairs).
fn spawn(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32, tags: &[TagId]) -> Entity {
    let bytes = v.to_ne_bytes();
    let empty: &[u8] = &[];
    let mut comps: Vec<(ComponentId, &[u8])> = Vec::with_capacity(1 + tags.len());
    comps.push((Payload::component_id(), &bytes));
    for t in tags {
        comps.push((t.component_id(), empty));
    }
    ecs.create_entity(arch, &comps)
        .expect("create_entity must succeed on the direct path")
}

// ── Standard two-archetype fixture ───────────────────────────────────────────

const TAGGED_VALUES: [u32; 3] = [10, 20, 30];
const UNTAGGED_VALUES: [u32; 2] = [1, 2];
const TAGGED_SUM: u32 = 60;
const UNTAGGED_SUM: u32 = 3;

struct Fixture {
    ecs: EcsMaster,
    tag: TagId,
    tagged_entities: Vec<Entity>,
    untagged_entities: Vec<Entity>,
}

/// Two archetypes matched by `&Payload`: one carrying the dynamic tag
/// (3 rows summing to [`TAGGED_SUM`]) and one without it (2 rows summing to
/// [`UNTAGGED_SUM`]).
fn fixture() -> Fixture {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22qt_main");
    let untagged_arch = ecs.create_archetype(&[Payload::component_id()]);
    let tagged_arch =
        ecs.get_or_create_archetype(&[Payload::component_id(), tag.component_id()]);

    let tagged_entities = TAGGED_VALUES
        .iter()
        .map(|&v| spawn(&mut ecs, tagged_arch, v, &[tag]))
        .collect();
    let untagged_entities = UNTAGGED_VALUES
        .iter()
        .map(|&v| spawn(&mut ecs, untagged_arch, v, &[]))
        .collect();

    Fixture {
        ecs,
        tag,
        tagged_entities,
        untagged_entities,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: iter (QueryIter constructor, iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn iter_honors_with_and_without_terms() {
    let mut f = fixture();
    {
        let view = f.ecs.query::<&Payload, ()>();
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(
            total,
            TAGGED_SUM + UNTAGGED_SUM,
            "no-terms baseline must cover both archetypes"
        );
    }
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(total, TAGGED_SUM, "with_tag must restrict iter to the tagged archetype");
    }
    {
        let view = f.ecs.query::<&Payload, ()>().without_tag(f.tag);
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(
            total, UNTAGGED_SUM,
            "without_tag must restrict iter to the untagged archetype"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cross-track seam: the tagged population built via the 2A migration surface
// (`add_tag`), not via direct create — the term driver must be agnostic to
// HOW the tagged archetype came to exist (attach migration vs direct create)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn iter_honors_terms_on_population_built_via_add_tag() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22qt_built_via_add_tag");
    let plain_arch = ecs.create_archetype(&[Payload::component_id()]);

    // Spawn EVERYTHING plain first, then attach the tag to the tagged subset
    // through the Wave-2A migration surface: the {Payload, tag} archetype is
    // created by `add_tag`'s attach path, never by direct create.
    let tagged: Vec<Entity> = TAGGED_VALUES
        .iter()
        .map(|&v| spawn(&mut ecs, plain_arch, v, &[]))
        .collect();
    for &e in &tagged {
        ecs.add_tag(e, tag);
    }
    for &v in &UNTAGGED_VALUES {
        spawn(&mut ecs, plain_arch, v, &[]);
    }

    // Same counts as the direct-create fixture drives through `iter`
    // (`iter_honors_with_and_without_terms`).
    {
        let view = ecs.query::<&Payload, ()>();
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(
            total,
            TAGGED_SUM + UNTAGGED_SUM,
            "no-terms baseline must cover both archetypes (add_tag-built fixture)"
        );
    }
    {
        let view = ecs.query::<&Payload, ()>().with_tag(tag);
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(
            total, TAGGED_SUM,
            "with_tag must match the add_tag-migrated population exactly as it \
             matches the direct-create fixture"
        );
    }
    {
        let view = ecs.query::<&Payload, ()>().without_tag(tag);
        let total: u32 = view.iter().map(|p| p.v).sum();
        assert_eq!(
            total, UNTAGGED_SUM,
            "without_tag must keep matching the plain archetype after the \
             tagged subset migrated out of it"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: iter_mut (QueryIterMut constructor, iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn iter_mut_honors_terms_and_mutations_stay_scoped() {
    let mut f = fixture();
    {
        let mut view = f.ecs.query::<&mut Payload, ()>().with_tag(f.tag);
        for p in view.iter_mut() {
            p.v += 100;
        }
    }
    let view = f.ecs.query::<&Payload, ()>();
    let total: u32 = view.iter().map(|p| p.v).sum();
    assert_eq!(
        total,
        TAGGED_SUM + 300 + UNTAGGED_SUM,
        "exactly the 3 tagged rows must gain +100; untagged rows untouched"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: par_iter (ParQuery distribution loop, par_iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn par_iter_honors_terms_under_active_pool() {
    let mut f = fixture();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let sum = AtomicU32::new(0);
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        pool.install(|_scope| {
            view.par_iter().for_each(|p| {
                sum.fetch_add(p.v, Ordering::Relaxed);
            });
        });
    }
    assert_eq!(
        sum.load(Ordering::Relaxed),
        TAGGED_SUM,
        "par_iter's distribution loop must hand workers only term-passing archetypes"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: par_iter_mut (ParQueryMut distribution loop, par_iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn par_iter_mut_honors_terms_under_active_pool() {
    let mut f = fixture();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    {
        let mut view = f.ecs.query::<&mut Payload, ()>().with_tag(f.tag);
        pool.install(|_scope| {
            view.par_iter_mut().for_each(|p| {
                p.v = p.v.wrapping_mul(2);
            });
        });
    }
    let view = f.ecs.query::<&Payload, ()>();
    let total: u32 = view.iter().map(|p| p.v).sum();
    assert_eq!(
        total,
        TAGGED_SUM * 2 + UNTAGGED_SUM,
        "only tagged rows must double; untagged rows untouched"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: Query::for_each_chunk (SystemParam surface → chunk_iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn query_for_each_chunk_honors_terms() {
    let mut f = fixture();
    let tag = f.tag;

    let with_sum: u32 = f.ecs.run_closure_once(move |q: Query<'_, '_, &Payload>| {
        let mut q = q.with_tag(tag);
        let mut sum = 0u32;
        q.for_each_chunk(|slice: &[Payload]| {
            for p in slice {
                sum += p.v;
            }
        });
        sum
    });
    assert_eq!(with_sum, TAGGED_SUM, "Query::for_each_chunk must honor with_tag");

    let without_sum: u32 = f.ecs.run_closure_once(move |q: Query<'_, '_, &Payload>| {
        let mut q = q.without_tag(tag);
        let mut sum = 0u32;
        q.for_each_chunk(|slice: &[Payload]| {
            for p in slice {
                sum += p.v;
            }
        });
        sum
    });
    assert_eq!(without_sum, UNTAGGED_SUM, "Query::for_each_chunk must honor without_tag");
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: QueryView::for_each_chunk (direct API → chunk_iter.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn query_view_for_each_chunk_honors_terms() {
    let mut f = fixture();
    let mut sum = 0u32;
    let mut invocations = 0usize;
    {
        let mut view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        view.for_each_chunk(|slice: &[Payload]| {
            invocations += 1;
            for p in slice {
                sum += p.v;
            }
        });
    }
    assert_eq!(
        invocations, 1,
        "exactly one closure invocation — only the tagged archetype passes the term"
    );
    assert_eq!(sum, TAGGED_SUM);
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: Query::par_for_each_chunk (SystemParam surface → par_chunk.rs).
// No pool is attached inside `run_closure_once`, so this exercises the PAR7
// fallback arm of the parallel driver (terms forwarded into the seq driver);
// the pooled dispatch-loop arm is covered by the QueryView test below.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn query_par_for_each_chunk_honors_terms() {
    let mut f = fixture();
    let tag = f.tag;
    let sum: u32 = f.ecs.run_closure_once(move |q: Query<'_, '_, &Payload>| {
        let mut q = q.with_tag(tag);
        let sum = AtomicU32::new(0);
        q.par_for_each_chunk(
            |slice: &[Payload]| {
                let local: u32 = slice.iter().map(|p| p.v).sum();
                sum.fetch_add(local, Ordering::Relaxed);
            },
            BatchingStrategy::default(),
        );
        sum.into_inner()
    });
    assert_eq!(sum, TAGGED_SUM, "Query::par_for_each_chunk must honor with_tag");
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: QueryView::par_for_each_chunk (direct API → par_chunk.rs, pooled
// dispatch loop — small archetype takes the PAR9 inline arm INSIDE the pool
// scope, which sits after the term test in the distribution loop)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn query_view_par_for_each_chunk_honors_terms_under_active_pool() {
    let mut f = fixture();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let sum = AtomicU32::new(0);
    let invocations = AtomicUsize::new(0);
    {
        let mut view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        pool.install(|_scope| {
            view.par_for_each_chunk(
                |slice: &[Payload]| {
                    invocations.fetch_add(1, Ordering::Relaxed);
                    sum.fetch_add(
                        slice.iter().map(|p| p.v).sum::<u32>(),
                        Ordering::Relaxed,
                    );
                },
                BatchingStrategy::default(),
            );
        });
    }
    assert_eq!(
        invocations.load(Ordering::Relaxed),
        1,
        "small tagged archetype → one inline invocation; untagged archetype term-rejected"
    );
    assert_eq!(sum.load(Ordering::Relaxed), TAGGED_SUM);
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: archetype_count / is_empty (query.rs + query_view.rs)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn archetype_count_and_is_empty_honor_terms_direct_api() {
    let mut f = fixture();
    let never_attached = f.ecs.register_tag("p22qt_never_attached");

    {
        let view = f.ecs.query::<&Payload, ()>();
        assert_eq!(view.archetype_count(), 2, "no-terms baseline: both archetypes");
        assert!(!view.is_empty());
    }
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        assert_eq!(view.archetype_count(), 1);
        assert!(!view.is_empty());
    }
    {
        let view = f.ecs.query::<&Payload, ()>().without_tag(f.tag);
        assert_eq!(view.archetype_count(), 1);
        assert!(!view.is_empty());
    }
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(never_attached);
        assert_eq!(view.archetype_count(), 0, "never-attached tag matches nothing");
        assert!(view.is_empty());
    }
}

#[test]
fn archetype_count_and_is_empty_honor_terms_system_param() {
    let mut f = fixture();
    let tag = f.tag;
    let never_attached = f.ecs.register_tag("p22qt_never_attached_sp");

    let (count_with, empty_with): (usize, bool) =
        f.ecs.run_closure_once(move |q: Query<'_, '_, &Payload>| {
            let q = q.with_tag(tag);
            (q.archetype_count(), q.is_empty())
        });
    assert_eq!(count_with, 1);
    assert!(!empty_with);

    let (count_never, empty_never): (usize, bool) =
        f.ecs.run_closure_once(move |q: Query<'_, '_, &Payload>| {
            let q = q.with_tag(never_attached);
            (q.archetype_count(), q.is_empty())
        });
    assert_eq!(count_never, 0);
    assert!(empty_never);
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: QueryView::get
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_honors_terms() {
    let mut f = fixture();
    let tagged_e = f.tagged_entities[0];
    let untagged_e = f.untagged_entities[0];

    {
        let view = f.ecs.query::<&Payload, ()>();
        assert!(view.get(tagged_e).is_some(), "no-terms baseline reaches both");
        assert!(view.get(untagged_e).is_some());
    }
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(f.tag);
        assert_eq!(
            view.get(tagged_e).map(|p| p.v),
            Some(TAGGED_VALUES[0]),
            "tagged entity is reachable under with_tag"
        );
        assert!(
            view.get(untagged_e).is_none(),
            "an entity in a term-rejected archetype is invisible to get"
        );
    }
    {
        let view = f.ecs.query::<&Payload, ()>().without_tag(f.tag);
        assert!(view.get(tagged_e).is_none());
        assert_eq!(view.get(untagged_e).map(|p| p.v), Some(UNTAGGED_VALUES[0]));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: QueryView::get_mut
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_mut_honors_terms() {
    let mut f = fixture();
    let tagged_e = f.tagged_entities[0];
    let untagged_e = f.untagged_entities[0];

    {
        let mut view = f.ecs.query::<&mut Payload, ()>().with_tag(f.tag);
        assert!(
            view.get_mut(untagged_e).is_none(),
            "an entity in a term-rejected archetype is invisible to get_mut"
        );
        let p = view.get_mut(tagged_e).expect("tagged entity reachable under with_tag");
        p.v += 5;
    }
    let view = f.ecs.query::<&Payload, ()>();
    assert_eq!(
        view.get(tagged_e).map(|p| p.v),
        Some(TAGGED_VALUES[0] + 5),
        "get_mut mutation must persist"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Driver: QueryView::single / single_mut (route via iter/iter_mut → inherit
// terms — D4 routing verification)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn single_and_single_mut_route_through_terms() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22qt_single");
    let untagged_arch = ecs.create_archetype(&[Payload::component_id()]);
    let tagged_arch =
        ecs.get_or_create_archetype(&[Payload::component_id(), tag.component_id()]);
    spawn(&mut ecs, tagged_arch, 42, &[tag]);
    spawn(&mut ecs, untagged_arch, 1, &[]);
    spawn(&mut ecs, untagged_arch, 2, &[]);

    {
        let view = ecs.query::<&Payload, ()>().with_tag(tag);
        assert_eq!(
            view.single().v,
            42,
            "single() must inherit terms via iter() — exactly one tagged row"
        );
    }
    {
        let mut view = ecs.query::<&mut Payload, ()>().with_tag(tag);
        view.single_mut().v += 1;
    }
    let view = ecs.query::<&Payload, ()>().with_tag(tag);
    assert_eq!(view.single().v, 43, "single_mut() must inherit terms via iter_mut()");
}

// ════════════════════════════════════════════════════════════════════════════
// >MAX_DYN_TAG_TERMS — loud release panic at term-add time
// ════════════════════════════════════════════════════════════════════════════

#[test]
#[should_panic(expected = "MAX_DYN_TAG_TERMS")]
fn more_than_eight_terms_panics_loudly() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22qt_overflow");
    let mut view = ecs.query::<&Payload, ()>();
    // Terms are not deduplicated — 9 pushes of the same tag overflow the
    // 8-slot stack storage and must panic loudly (release-active).
    for _ in 0..9 {
        view = view.with_tag(tag);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// with + without combination
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn with_and_without_terms_combine() {
    let mut ecs = EcsMaster::new();
    let tag_a = ecs.register_tag("p22qt_combo_a");
    let tag_b = ecs.register_tag("p22qt_combo_b");
    let arch_a =
        ecs.get_or_create_archetype(&[Payload::component_id(), tag_a.component_id()]);
    let arch_ab = ecs.get_or_create_archetype(&[
        Payload::component_id(),
        tag_a.component_id(),
        tag_b.component_id(),
    ]);
    let arch_plain = ecs.create_archetype(&[Payload::component_id()]);

    spawn(&mut ecs, arch_a, 100, &[tag_a]);
    spawn(&mut ecs, arch_ab, 200, &[tag_a, tag_b]);
    spawn(&mut ecs, arch_plain, 300, &[]);

    let view = ecs.query::<&Payload, ()>().with_tag(tag_a).without_tag(tag_b);
    let collected: Vec<u32> = view.iter().map(|p| p.v).collect();
    assert_eq!(
        collected,
        vec![100],
        "with_tag(a).without_tag(b) must select exactly the {{Payload, a}} archetype"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Composition with typed filters
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn terms_compose_with_typed_with_filter() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("p22qt_compose_with");
    let marker_bytes = 0u32.to_ne_bytes();

    // Three archetypes: {Payload, Marker, tag} → 1; {Payload, Marker} → 2;
    // {Payload, tag} → 4. `With<Marker>` + with_tag must select only the
    // first.
    let arch_mt = ecs.get_or_create_archetype(&[
        Payload::component_id(),
        Marker::component_id(),
        tag.component_id(),
    ]);
    let arch_m =
        ecs.create_archetype(&[Payload::component_id(), Marker::component_id()]);
    let arch_t =
        ecs.get_or_create_archetype(&[Payload::component_id(), tag.component_id()]);

    let empty: &[u8] = &[];
    let v1 = 1u32.to_ne_bytes();
    ecs.create_entity(
        arch_mt,
        &[
            (Payload::component_id(), &v1),
            (Marker::component_id(), &marker_bytes),
            (tag.component_id(), empty),
        ],
    )
    .expect("spawn into {Payload, Marker, tag}");
    let v2 = 2u32.to_ne_bytes();
    ecs.create_entity(
        arch_m,
        &[
            (Payload::component_id(), &v2),
            (Marker::component_id(), &marker_bytes),
        ],
    )
    .expect("spawn into {Payload, Marker}");
    spawn(&mut ecs, arch_t, 4, &[tag]);

    let view = ecs.query::<&Payload, With<Marker>>().with_tag(tag);
    let total: u32 = view.iter().map(|p| p.v).sum();
    assert_eq!(
        total, 1,
        "With<Marker> (typed, mask-level) and with_tag (dynamic term) must intersect"
    );
}

/// `Changed<T>` is a per-row tick filter and needs a real frame window —
/// one-shot `run_closure_once` systems snapshot `last_run == this_run`
/// (pre-existing semantics, no frame delta), so this test mirrors the
/// Phase 10 `added_filter_basic_spawn_query` Schedule pattern: frame 1's
/// window covers the pre-existing spawn ticks, frame 2 sees nothing new.
#[test]
fn terms_compose_with_changed_filter() {
    let mut f = fixture();
    let tag = f.tag;
    let pool = ThreadPoolBuilder::new().num_threads(2).build();

    let sum = Arc::new(AtomicU32::new(0));
    let sum_in_system = Arc::clone(&sum);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |q: Query<'_, '_, &Payload, Changed<Payload>>| {
        let q = q.with_tag(tag);
        for p in &q {
            sum_in_system.fetch_add(p.v, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut f.ecs);

    // Frame 1: the system's first-run window covers the fixture's spawn
    // ticks → Changed matches; the dynamic term narrows the walk to the
    // tagged archetype only.
    schedule.run(&mut f.ecs);
    assert_eq!(
        sum.load(Ordering::Relaxed),
        TAGGED_SUM,
        "Changed<Payload> (per-row tick filter) and with_tag (archetype term) must compose"
    );

    // Frame 2: nothing mutated between frames → zero matches (proves the
    // tick filter still runs per row underneath the term test).
    sum.store(0, Ordering::Relaxed);
    schedule.run(&mut f.ecs);
    assert_eq!(
        sum.load(Ordering::Relaxed),
        0,
        "frame 2: no mutations since frame 1 — Changed must yield zero rows"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// F1 review O4: every matched archetype fails the term — zero yields, clean
// termination of both pull cursors (pins the zero-row-window exhaustion path
// against future loop-shape edits in iter.rs).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn iter_terminates_when_all_archetypes_fail_the_term() {
    let mut f = fixture();
    let orphan_tag = f.ecs.register_tag("p22qt_o4_orphan");
    {
        let view = f.ecs.query::<&Payload, ()>().with_tag(orphan_tag);
        let mut yields = 0usize;
        for _ in view.iter() {
            yields += 1;
        }
        assert_eq!(yields, 0, "no archetype carries the orphan tag — iter must yield nothing");
    }
    {
        let mut view = f.ecs.query::<&mut Payload, ()>().with_tag(orphan_tag);
        let mut yields = 0usize;
        for _ in view.iter_mut() {
            yields += 1;
        }
        assert_eq!(yields, 0, "mut cursor must also yield nothing and terminate");
    }
}
