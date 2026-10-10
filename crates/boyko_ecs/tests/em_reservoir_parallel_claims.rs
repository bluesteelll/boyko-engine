//! EM2′ — several `Commands` systems claiming from ONE populated recycled-entity
//! stack in the same phase, through the real `Schedule` / `EntityCounter` path.
//!
//! # What the other EM2′ tests do not reach
//!
//! `em_deferred_recycle`'s T-RED-1 churns through ONE system, so every claim in
//! it comes from one thread; `entity_counter`'s 8-thread unit test races only
//! the FRESH counter (its stack is empty). Neither has two workers
//! `fetch_sub`-ing the same populated stack at once, which is the protocol's
//! whole reason to be lock-free. This file does, and pins:
//!
//! * **(u) uniqueness** — every handle the phase hands out is a distinct id.
//! * **(r) recycled exactness** — the recycled handles are EXACTLY the ids
//!   despawned before the phase, each at generation + 1.
//! * **(f) fresh exactness** — the rest are exactly the next fresh ids, at
//!   generation 0, and `next_entity_id` advanced by exactly their count.
//! * **(v) validity** — every handle is live after the apply window and reads
//!   the marker of the system that spawned it; every despawned handle is dead.
//! * **(x) the EXHAUSTED bit** — after the phase `free_top_raw` drifted below 0
//!   by at most one step per system (only the claim that runs the stack dry
//!   pays a failing `fetch_sub`); with a stack that was empty at phase start it
//!   does not drift at all (the counter presets the bit).
//! * **(c) concurrency premise** — at least two spawning systems were inside
//!   their claim loops at the same time; without it (u)/(r) would be green from
//!   a serial schedule.
//!
//! What would make each RED: (u)/(r) — a claim that reads `free[r]` instead of
//! `free[r - 1]`, or a plain load/store in place of the `fetch_sub`; (f) — a
//! fresh mint on the recycled path; (x) — dropping the `tagged.set(EXHAUSTED)`
//! in `EntityCounter::reserve_entity` (drift −512 instead of ≥ −4) or the
//! preset in `from_ptr` (drift −4 instead of 0).
//!
//! No timing is measured: every assertion is a count or an identity. The one
//! clock read is a liveness bound on the concurrency rendezvous (a system waits
//! at most 10 s for a second system to arrive), never a verdict.

// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]
// Drives a real `ThreadPool` and the full scheduler — out of Miri's reach; the
// Miri-sized twin of the claim race is `entity_reservoir`'s unit test.
#![cfg(not(miri))]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component};

/// The component every entity in this file carries: which system spawned it
/// (`SEED_MARK` for the seed rows).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Mark {
    sys: u32,
}

#[derive(Bundle)]
struct MarkBundle {
    m: Mark,
}

const SEED_MARK: u32 = u32::MAX;
/// Spawning systems in the phase.
const SYSTEMS: usize = 4;
/// Spawns per system.
const K: usize = 256;

/// Handles each system produced, packed `id << 32 | generation`, +1 so that 0
/// means "never written".
static OUT: [[AtomicU64; K]; SYSTEMS] = [const { [const { AtomicU64::new(0) }; K] }; SYSTEMS];
/// Systems that have entered their body this phase.
static ARRIVED: AtomicUsize = AtomicUsize::new(0);
/// The largest `ARRIVED` any system saw when it started claiming.
static MAX_SEEN: AtomicUsize = AtomicUsize::new(0);

fn pack(e: Entity) -> u64 {
    assert!(e.id().0 < (1 << 31), "fixture ids stay small");
    (((e.id().0 as u64) << 32) | e.generation() as u64) + 1
}

fn unpack(v: u64) -> Entity {
    let v = v - 1;
    Entity::new(EntityId((v >> 32) as usize), v as u32)
}

/// One of the `SYSTEMS` spawners. It waits (bounded) for a second spawner to
/// arrive so the claim loops overlap, then claims `K` handles.
fn spawner<const S: usize>(mut cmds: Commands) {
    let arrived = ARRIVED.fetch_add(1, Ordering::AcqRel) + 1;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = arrived;
    while seen < 2 && Instant::now() < deadline {
        std::hint::spin_loop();
        seen = ARRIVED.load(Ordering::Acquire);
    }
    MAX_SEEN.fetch_max(seen, Ordering::AcqRel);
    for slot in OUT[S].iter() {
        let e = cmds.spawn(MarkBundle { m: Mark { sys: S as u32 } }).id();
        slot.store(pack(e), Ordering::Relaxed);
    }
}

fn reset_statics() {
    for row in OUT.iter() {
        for slot in row.iter() {
            slot.store(0, Ordering::Relaxed);
        }
    }
    ARRIVED.store(0, Ordering::Relaxed);
    MAX_SEEN.store(0, Ordering::Relaxed);
}

struct Violations(Vec<String>);

impl Violations {
    fn check(&mut self, ok: bool, msg: impl FnOnce() -> String) {
        if !ok {
            self.0.push(msg());
        }
    }
}

/// Builds a world of `seed` rows, despawns `despawn` of them (scattered ids),
/// runs ONE frame of `SYSTEMS` spawners, and checks every clause.
fn scenario(name: &str, seed: usize, despawn: usize, v: &mut Violations) {
    reset_statics();
    let pool = ThreadPoolBuilder::new().num_threads(SYSTEMS).build();
    let mut app = App::with_pool(pool);
    let mut deleted: Vec<Entity> = Vec::with_capacity(despawn);
    {
        let world = app.world_mut();
        let arch = world.create_archetype(&[Mark::component_id()]);
        let mut seeded = Vec::with_capacity(seed);
        for _ in 0..seed {
            seeded.push(world.spawn_one(arch, Mark { sys: SEED_MARK }).expect("seed row"));
        }
        // Every third row first, so the recycled ids are not one contiguous run.
        let order = (0..seed).step_by(3).chain((1..seed).step_by(3)).chain((2..seed).step_by(3));
        for i in order.take(despawn) {
            assert!(world.delete_entity(seeded[i]), "fixture: seed row must despawn");
            deleted.push(seeded[i]);
        }
        assert_eq!(world.recycled_entity_count(), despawn, "fixture: stack depth");
    }
    app.add_systems(spawner::<0>);
    app.add_systems(spawner::<1>);
    app.add_systems(spawner::<2>);
    app.add_systems(spawner::<3>);
    app.finish();

    let next_before = app.world().entity_master().next_entity_id().0;
    let capacity_before = app.world().entity_master().capacity();
    app.update_with_delta(Duration::from_millis(16));
    let world = app.world();

    // Collect.
    let mut handles: Vec<(usize, Entity)> = Vec::with_capacity(SYSTEMS * K);
    let mut unwritten = 0usize;
    for (s, row) in OUT.iter().enumerate() {
        for slot in row.iter() {
            match slot.load(Ordering::Relaxed) {
                0 => unwritten += 1,
                packed => handles.push((s, unpack(packed))),
            }
        }
    }
    v.check(unwritten == 0, || format!("{name}: {unwritten} of {} spawns never ran", SYSTEMS * K));

    // (c) concurrency premise.
    let max_seen = MAX_SEEN.load(Ordering::Relaxed);
    v.check(max_seen >= 2, || {
        format!("{name}: (c) no two spawners overlapped (max arrived {max_seen}); the claim race was never run")
    });

    // (u) uniqueness.
    let mut ids: Vec<usize> = handles.iter().map(|(_, e)| e.id().0).collect();
    ids.sort_unstable();
    let dups = ids.windows(2).filter(|w| w[0] == w[1]).count();
    v.check(dups == 0, || format!("{name}: (u) {dups} duplicate id(s) handed out in one phase"));

    // (r) recycled exactness.
    let mut deleted_ids: Vec<usize> = deleted.iter().map(|e| e.id().0).collect();
    deleted_ids.sort_unstable();
    let demand = SYSTEMS * K;
    let expect_recycled = despawn.min(demand);
    let recycled: Vec<&(usize, Entity)> =
        handles.iter().filter(|(_, e)| deleted_ids.binary_search(&e.id().0).is_ok()).collect();
    v.check(recycled.len() == expect_recycled, || {
        format!("{name}: (r) {} recycled handles, expected {expect_recycled}", recycled.len())
    });
    for (_, e) in &recycled {
        let old = deleted.iter().find(|d| d.id() == e.id()).expect("invariant: filtered above");
        v.check(e.generation() == old.generation() + 1, || {
            format!("{name}: (r) recycled {e:?} after a despawn at generation {}", old.generation())
        });
    }
    if despawn > demand {
        // LIFO: the phase consumed the TOP `demand` entries, i.e. the LAST
        // `demand` despawns.
        let mut want: Vec<usize> = deleted[despawn - demand..].iter().map(|e| e.id().0).collect();
        want.sort_unstable();
        v.check(ids == want, || format!("{name}: (r) the phase did not claim exactly the top {demand} entries"));
    }

    // (f) fresh exactness.
    let expect_fresh = demand - expect_recycled;
    let mut fresh: Vec<usize> = handles
        .iter()
        .filter(|(_, e)| deleted_ids.binary_search(&e.id().0).is_err())
        .map(|(_, e)| {
            if e.generation() != 0 {
                v.0.push(format!("{name}: (f) fresh {e:?} carries a non-zero generation"));
            }
            e.id().0
        })
        .collect();
    fresh.sort_unstable();
    let want_fresh: Vec<usize> = (next_before..next_before + expect_fresh).collect();
    v.check(fresh == want_fresh, || {
        format!(
            "{name}: (f) fresh ids {:?}..{:?} ({}), expected {next_before}..{} ({expect_fresh})",
            fresh.first(),
            fresh.last(),
            fresh.len(),
            next_before + expect_fresh
        )
    });
    let next_after = world.entity_master().next_entity_id().0;
    v.check(next_after == next_before + expect_fresh, || {
        format!("{name}: (f) next_entity_id {next_before} -> {next_after}, expected +{expect_fresh}")
    });
    if expect_fresh == 0 {
        let capacity_after = world.entity_master().capacity();
        v.check(capacity_after == capacity_before, || {
            format!("{name}: (f) slot store grew {capacity_before} -> {capacity_after} with no fresh id")
        });
    }

    // (v) validity.
    for (s, e) in &handles {
        let got = world.get_component::<Mark>(*e).map(|m| m.sys);
        v.check(got == Some(*s as u32), || format!("{name}: (v) {e:?} reads {got:?}, expected Some({s})"));
    }
    for d in &deleted {
        v.check(!world.has_entity(*d), || format!("{name}: (v) despawned {d:?} is valid again"));
    }
    let left = world.recycled_entity_count();
    v.check(left == despawn - expect_recycled, || {
        format!("{name}: {left} ids left on the stack, expected {}", despawn - expect_recycled)
    });

    // (x) the EXHAUSTED bit, as an integer.
    let drift = world.entity_master().free_top_raw();
    if despawn == 0 {
        v.check(drift == 0, || {
            format!("{name}: (x) free_top drifted to {drift} over an EMPTY stack; the preset must suppress every fetch_sub")
        });
    } else if despawn >= demand {
        v.check(drift == (despawn - demand) as isize, || {
            format!("{name}: (x) free_top {drift}, expected {} (no claim ran dry)", despawn - demand)
        });
    } else {
        v.check((-(SYSTEMS as isize)..=0).contains(&drift), || {
            format!(
                "{name}: (x) free_top drifted to {drift}; at most one failing fetch_sub per system \
                 (-{SYSTEMS}..=0) is the EXHAUSTED bit's contract"
            )
        });
    }
    eprintln!(
        "{name}: stack {despawn} vs demand {demand}: recycled {}, fresh {}, drift {drift}, max overlap {max_seen}",
        recycled.len(),
        fresh.len()
    );
}

/// The three regimes, one test so the file's statics are never shared by two
/// concurrently running tests.
#[test]
fn parallel_commands_systems_claim_disjoint_ids_from_one_stack() {
    let mut v = Violations(Vec::with_capacity(16));
    // A — the stack runs dry mid-phase: 512 recycled + 512 fresh.
    scenario("A (stack < demand)", 1536, 512, &mut v);
    // B — the stack outlasts the phase: 1024 of 1500 recycled, no fresh id.
    scenario("B (stack > demand)", 1536, 1500, &mut v);
    // C — the stack is empty at phase start: 1024 fresh, zero failing claims.
    scenario("C (empty stack)", 64, 0, &mut v);
    assert!(v.0.is_empty(), "{} violation(s):\n  {}", v.0.len(), v.0.join("\n  "));
}
