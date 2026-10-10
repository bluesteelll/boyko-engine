//! EM2′ — entity ids are recycled on the DEFERRED route (`Commands::spawn`),
//! and a rejected `create_entity` never revives a stale handle.
//!
//! # The defect these tests were written RED against (d11962a9)
//!
//! * **T-RED-1.** `Commands::spawn` minted every id through
//!   `EntityCounter::reserve_entity`, a bare `fetch_add` on the fresh counter
//!   that never looked at the free list (Phase 11 EM2 forbade a worker to pop
//!   it), while `DespawnCommand` pushed every despawned id onto that list. A
//!   flat population churned through `Commands` therefore grew the free list
//!   AND the `entities_inland` slot store by one entry per despawn, forever.
//! * **T-RED-2.** `EcsMaster::create_entity`'s rejection path chose how to undo
//!   `allocate_entity` by arithmetic (`id + 1 == next_entity_id`), not by what
//!   `allocate_entity` actually did. A recycled id that was also the newest
//!   minted id took the FRESH arm, rolled the counter back, and the next fresh
//!   mint re-registered that id with generation 0 — the despawned handle became
//!   valid again (generation ABA). A recycled id that was not the newest took
//!   neither arm and leaked.
//!
//! Both tests collect EVERY violated clause before failing, so one red run names
//! all of them rather than the first.
//!
//! No timing is measured anywhere in this file: every assertion is a count or an
//! identity.

// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]
// T-RED-1 builds a real `ThreadPool` and drives 260 frames — out of Miri's
// reach. Miri covers the claim protocol through `entity_reservoir`'s unit
// tests (`claim_race_*`, three threads); no loom model of `EntityReservoir`
// exists yet.
#![cfg(not(miri))]

use std::time::Duration;

use boyko_ecs::ecs::core::system::Entities;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component, Resource};

// ═══════════════════════════════════════════════════════════════════════════
// T-RED-1 fixtures — the census's S2 churn shape, scaled down
// ═══════════════════════════════════════════════════════════════════════════

/// Static payload rows, so the churn runs next to a populated archetype and a
/// `par_iter` fan-out, as in the census scene.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Payload {
    v: u32,
}

/// The churn marker: the wave (frame) the entity was spawned in.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Doomed {
    wave: u32,
}

#[derive(Bundle)]
struct DoomedBundle {
    d: Doomed,
}

/// Static rows in the payload archetype.
const ROWS: usize = 1024;
/// Entities spawned AND despawned every churn frame.
const CHURN: usize = 64;
/// Frames before the measured window opens.
const WARM: usize = 4;
/// The measured window.
const FRAMES: usize = 256;

/// Churn state plus the handles one frame produced. The vectors are sized once
/// at construction and cleared per frame.
#[derive(Resource)]
struct Churn {
    wave: u32,
    spawned: u64,
    despawned: u64,
    /// The `.id()`s `Commands::spawn` returned this frame.
    spawned_now: Vec<Entity>,
    /// The handles this frame enqueued a despawn for.
    despawned_now: Vec<Entity>,
}

impl Churn {
    fn new() -> Self {
        Self {
            wave: 0,
            spawned: 0,
            despawned: 0,
            spawned_now: Vec::with_capacity(CHURN),
            despawned_now: Vec::with_capacity(2 * CHURN),
        }
    }
}

fn pool(workers: usize) -> std::sync::Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(workers).build()
}

/// The fan-out that puts a worker phase next to the churn (census S2 shape).
fn par_fanout(q: Query<&Payload>) {
    static SUM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    q.par_iter().for_each(|p: &Payload| {
        SUM.fetch_add(p.v as u64, std::sync::atomic::Ordering::Relaxed);
    });
}

/// Despawns every `Doomed` not of the current wave and spawns `CHURN` of the
/// current wave, all through `Commands` — ONE system, so the churn is
/// stationary from the second frame on with no cross-system ordering edge.
fn churn(mut cmds: Commands, q: Query<&Doomed>, ents: Entities, mut st: ResMut<Churn>) {
    let wave = st.wave;
    st.spawned_now.clear();
    st.despawned_now.clear();
    for (id, d) in q.iter_entities() {
        if d.wave != wave
            && let Some(e) = ents.get(id)
        {
            cmds.despawn(e);
            st.despawned_now.push(e);
        }
    }
    for _ in 0..CHURN {
        let e = cmds.spawn(DoomedBundle { d: Doomed { wave } }).id();
        st.spawned_now.push(e);
    }
    st.despawned += st.despawned_now.len() as u64;
    st.spawned += CHURN as u64;
    st.wave = wave.wrapping_add(1);
}

/// Per-frame entity-store sample, taken after the frame's apply window.
#[derive(Clone, Copy)]
struct Sample {
    recycled: usize,
    capacity: usize,
    committed: usize,
    live: usize,
}

fn sample(world: &EcsMaster) -> Sample {
    Sample {
        recycled: world.recycled_entity_count(),
        capacity: world.entity_master().capacity(),
        committed: world.entity_master().committed_slots(),
        live: world.entity_count(),
    }
}

/// Records the FIRST message of each violated clause and how many times it
/// fired, so the final panic names every clause without flooding.
struct Violations {
    first: Vec<(char, String, usize)>,
}

impl Violations {
    fn new() -> Self {
        Self { first: Vec::with_capacity(16) }
    }

    fn check(&mut self, clause: char, ok: bool, msg: impl FnOnce() -> String) {
        if ok {
            return;
        }
        if let Some(slot) = self.first.iter_mut().find(|(c, _, _)| *c == clause) {
            slot.2 += 1;
        } else {
            self.first.push((clause, msg(), 1));
        }
    }

    fn finish(self, test: &str) {
        if self.first.is_empty() {
            return;
        }
        let mut report = format!("{test}: {} clause(s) violated\n", self.first.len());
        for (clause, msg, count) in &self.first {
            report.push_str(&format!("  ({clause}) x{count}: {msg}\n"));
        }
        panic!("{report}");
    }
}

fn sorted_ids(handles: &[Entity]) -> Vec<EntityId> {
    let mut ids: Vec<EntityId> = handles.iter().map(|e| e.id()).collect();
    ids.sort_unstable_by_key(|id| id.0);
    ids
}

/// T-RED-1 — a flat population churned through `Commands` keeps the free list
/// and the slot store bounded, and recycles the ids it despawns.
///
/// Clauses:
/// * (a) anti-vacuity: spawned = (WARM + FRAMES) · CHURN, despawned ≥ spawned −
///   2·CHURN, and the live population stays within ±CHURN.
/// * (b) the free list never holds more than one frame's despawns.
/// * (c) `capacity()` does not move across the window and stays ≤ live + 2·CHURN.
/// * (d) `committed_slots()` does not move across the window.
/// * (e) the ids spawned in frame k are exactly the ids despawned in frame
///   k−1's window, each carrying that handle's generation + 1, and at least one
///   handle in the window has a generation ≥ 1.
/// * (f) every `.id()` of frame k is valid after update k and reads `wave == k`;
///   every wave k−1 handle is invalid after update k.
/// * (g) a stale handle to a recycled id stays invalid after its successor
///   registered.
#[test]
fn commands_churn_recycles_ids_on_the_deferred_route() {
    let mut app = App::with_pool(pool(2));
    // Seeded one row at a time, NOT with `spawn_batch`: a batch apply grows the
    // slot store to `end + MAX_BATCH_HINT` (`SpawnBatchCommand::apply`), which
    // would put 8192 slots of headroom under clause (c)'s `live + 2·CHURN`
    // bound and make it unfalsifiable. One-at-a-time leaves `capacity()` equal
    // to the ids ever minted.
    {
        let world = app.world_mut();
        let arch = world.create_archetype(&[Payload::component_id()]);
        for v in 0..ROWS as u32 {
            world.spawn_one(arch, Payload { v }).expect("seed payload row");
        }
    }
    app.world_mut().insert_resource(Churn::new());
    app.add_systems(par_fanout);
    app.add_systems(churn);
    app.finish();

    for _ in 0..WARM {
        app.update_with_delta(Duration::from_millis(16));
    }

    let mut v = Violations::new();
    let mut samples: Vec<Sample> = Vec::with_capacity(FRAMES);
    let before = sample(app.world());
    let (mut prev_spawned, mut prev_despawned) = {
        let st = app.world().resource::<Churn>();
        (st.spawned_now.clone(), st.despawned_now.clone())
    };
    let mut max_generation = 0u32;

    for k in 0..FRAMES {
        app.update_with_delta(Duration::from_millis(16));
        let world = app.world();
        let s = sample(world);
        samples.push(s);
        let st = world.resource::<Churn>();
        let wave_k = st.wave.wrapping_sub(1);

        // (b)
        v.check('b', s.recycled <= CHURN, || {
            format!(
                "frame {k}: the free list holds {} ids after the apply window, more than one \
                 frame's {CHURN} despawns",
                s.recycled
            )
        });
        // (c)
        v.check('c', s.capacity == before.capacity && s.capacity <= s.live + 2 * CHURN, || {
            format!(
                "frame {k}: slot store capacity {} (window opened at {}, live {}); the id-slot \
                 store must not grow on a flat population",
                s.capacity, before.capacity, s.live
            )
        });
        // (d)
        v.check('d', s.committed == before.committed, || {
            format!(
                "frame {k}: committed slots {} (window opened at {})",
                s.committed, before.committed
            )
        });

        // (e) — the ids spawned in frame k are the ids despawned in window k−1.
        let spawned_ids = sorted_ids(&st.spawned_now);
        let despawned_prev_ids = sorted_ids(&prev_despawned);
        v.check('e', spawned_ids == despawned_prev_ids, || {
            format!(
                "frame {k}: the {} ids spawned are not the {} ids despawned one window earlier \
                 (first spawned {:?}, first despawned {:?}); the deferred route is not recycling",
                spawned_ids.len(),
                despawned_prev_ids.len(),
                spawned_ids.first(),
                despawned_prev_ids.first()
            )
        });
        for e in &st.spawned_now {
            max_generation = max_generation.max(e.generation());
            if let Some(old) = prev_despawned.iter().find(|p| p.id() == e.id()) {
                v.check('e', e.generation() == old.generation().wrapping_add(1), || {
                    format!(
                        "frame {k}: recycled id {:?} carries generation {} after a despawn at \
                         generation {}",
                        e.id(),
                        e.generation(),
                        old.generation()
                    )
                });
            }
        }

        // (f)
        for e in &st.spawned_now {
            let wave = world.get_component::<Doomed>(*e).map(|d| d.wave);
            v.check('f', wave == Some(wave_k), || {
                format!(
                    "frame {k}: the handle {e:?} returned by Commands::spawn reads {wave:?} after \
                     its apply, expected Some({wave_k})"
                )
            });
        }
        for e in &prev_spawned {
            v.check('f', !world.has_entity(*e), || {
                format!("frame {k}: wave {} handle {e:?} is still valid after its despawn", wave_k.wrapping_sub(1))
            });
        }
        // (g) — the predecessor of every recycled id stays dead.
        for old in &prev_despawned {
            v.check('g', !world.has_entity(*old), || {
                format!(
                    "frame {k}: the despawned handle {old:?} is valid again (its id's successor \
                     registered this frame)"
                )
            });
        }

        prev_spawned.clone_from(&st.spawned_now);
        prev_despawned.clone_from(&st.despawned_now);
    }

    // (a) — anti-vacuity.
    let st = app.world().resource::<Churn>();
    let expected_spawned = ((WARM + FRAMES) * CHURN) as u64;
    v.check('a', st.spawned == expected_spawned, || {
        format!("{} spawned, expected {expected_spawned}", st.spawned)
    });
    v.check('a', st.despawned + 2 * CHURN as u64 >= st.spawned, || {
        format!("{} despawned against {} spawned; the churn is not stationary", st.despawned, st.spawned)
    });
    for s in &samples {
        v.check('a', s.live.abs_diff(before.live) <= CHURN, || {
            format!("population drifted {} -> {}", before.live, s.live)
        });
    }
    // (e) — at least one handle crossed a generation, or (e) is green from
    // ids that were never recycled at all.
    v.check('e', max_generation >= 1, || {
        "no handle in the window carries a generation >= 1; nothing was recycled".to_string()
    });

    let max_recycled = samples.iter().map(|s| s.recycled).max().unwrap_or(0);
    let last = samples.last().copied().unwrap_or(before);
    eprintln!(
        "T-RED-1: max free list {max_recycled}, capacity {} -> {}, committed {} -> {}, live {} -> \
         {}, max generation {max_generation}",
        before.capacity, last.capacity, before.committed, last.committed, before.live, last.live
    );
    v.finish("T-RED-1");
}

// ═══════════════════════════════════════════════════════════════════════════
// T-RED-2 — the rejected create
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct RedA {
    v: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct RedB {
    v: u32,
}

/// T-RED-2 — `create_entity` rejected by the archetype returns the allocated id
/// to exactly where it came from, and never revives the despawned handle.
///
/// * Case A (the newest id): e0, e1 live, e1 despawned (so its recycled id is
///   also the newest minted id), then a create missing `RedB` is rejected.
///   (a) `next_entity_id()` is unchanged, (b) the recycled id is back on the
///   free list, (c) the next good create recycles it at generation 1, and (d)
///   the despawned e1 stays invalid.
/// * Case B (the leak arm): e0 despawned (not the newest), the same rejected
///   create — (e) the recycled id is back on the free list.
#[test]
fn rejected_create_after_despawning_the_newest_entity_does_not_revive_its_handle() {
    let a = RedA::component_id();
    let b = RedB::component_id();
    let bytes = 7u32.to_ne_bytes();
    let mut v = Violations::new();

    // ── Case A ──
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[a, b]);
    let _e0 = world.create_entity(arch, &[(a, &bytes), (b, &bytes)]).expect("e0");
    let e1 = world.create_entity(arch, &[(a, &bytes), (b, &bytes)]).expect("e1");
    let n = world.entity_master().next_entity_id().0;
    assert_eq!(e1.id().0 + 1, n, "fixture: e1 must be the newest minted id");
    assert!(world.delete_entity(e1), "fixture: e1 must despawn");

    let rejected = world.create_entity(arch, &[(a, &bytes)]);
    // Anti-vacuity: `ArchetypeRejectedEntity` (not `ArchetypeNotFound`) proves
    // the id WAS allocated before the rejection.
    assert!(
        matches!(rejected, Err(EcsError::ArchetypeRejectedEntity { .. })),
        "fixture: expected Err(ArchetypeRejectedEntity), got {rejected:?}"
    );
    let next_after = world.entity_master().next_entity_id().0;
    v.check('a', next_after == n, || {
        format!("case A: next_entity_id is {next_after} after the rejected create, expected {n}")
    });
    let recycled_after = world.recycled_entity_count();
    v.check('b', recycled_after == 1, || {
        format!("case A: {recycled_after} recycled ids after the rejected create, expected 1")
    });
    let e2 = world.create_entity(arch, &[(a, &bytes), (b, &bytes)]).expect("e2");
    let want = Entity::new(EntityId(n - 1), 1);
    v.check('c', e2 == want, || {
        format!("case A: the next create returned {e2:?}, expected {want:?}")
    });
    v.check('d', !world.has_entity(e1), || {
        format!(
            "case A: the despawned handle {e1:?} is VALID again after the rejected create — \
             generation ABA (the next create returned {e2:?})"
        )
    });

    // ── Case B ──
    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[a, b]);
    let e0 = world.create_entity(arch, &[(a, &bytes), (b, &bytes)]).expect("e0");
    let _e1 = world.create_entity(arch, &[(a, &bytes), (b, &bytes)]).expect("e1");
    assert!(world.delete_entity(e0), "fixture: e0 must despawn");
    let rejected = world.create_entity(arch, &[(a, &bytes)]);
    assert!(
        matches!(rejected, Err(EcsError::ArchetypeRejectedEntity { .. })),
        "fixture: expected Err(ArchetypeRejectedEntity), got {rejected:?}"
    );
    let recycled_after = world.recycled_entity_count();
    v.check('e', recycled_after == 1, || {
        format!(
            "case B: {recycled_after} recycled ids after the rejected create of a recycled, \
             not-newest id, expected 1 — the id leaked"
        )
    });

    v.finish("T-RED-2");
}
