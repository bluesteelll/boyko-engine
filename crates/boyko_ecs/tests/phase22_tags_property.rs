//! Phase 22 (Tags) — property-based test (proptest), plan suite (a):
//! a random interleave of `spawn` / `despawn` / `add_tag` / `remove_tag`
//! over a small fixed slot pool, checked after EVERY operation against a
//! membership oracle (`slot -> Option<tag set>`; `None` = dead). Each op
//! randomly takes the DIRECT route (`EcsMaster::add_tag` / `remove_tag` /
//! `delete_entity` / `spawn_empty`) or the DEFERRED route (`Commands` /
//! `EntityCommands` through `run_system`'s apply window), so the
//! `AddTagCommand` / `RemoveTagCommand` delegation is interleaved with
//! direct structural ops — the Phase-14b lesson (deferred sites silently
//! diverging from the direct API) is what this property would catch.
//!
//! TAG ops are also issued against DEAD handles on purpose: the plan-D9
//! contract makes them silent no-ops on BOTH routes (a despawn may
//! legitimately race an enqueued tag op), so the oracle simply ignores
//! them. DESPAWN is the one carve-out: `DespawnCommand::apply` on a stale
//! handle `debug_assert`-panics by design (EC11 — silent no-op in release
//! only), so the generator never enqueues a DEFERRED despawn for a dead
//! slot; the DIRECT `delete_entity` on a dead handle (returns `false`)
//! stays in the mix.
//!
//! # Registry budget discipline (see `tests/phase22_tags.rs` header)
//!
//! The tag NAMES are fixed process-wide constants, minted idempotently once
//! per case (`register_tag` is name-keyed): 256 cases reuse the SAME 3
//! ComponentId slots. Minting per-case-unique names would burn
//! 256 x 3 slots and exhaust the shared 512-slot budget mid-run. Entities
//! are tag-only (`spawn_empty`), so the archetype universe is bounded by
//! 2^TAGS + 1.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component_registry::TagId;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use proptest::prelude::*;

/// Slot count: small enough for fast cases, big enough for swap churn.
const POOL: usize = 6;
/// Distinct dynamic tags (process-global names, minted once).
const TAGS: usize = 3;

const TAG_NAMES: [&str; TAGS] = [
    "phase22_prop_oracle_a",
    "phase22_prop_oracle_b",
    "phase22_prop_oracle_c",
];

/// A generated operation over slot indices `0..POOL` / tag indices `0..TAGS`.
/// `deferred` picks the `Commands` route over the direct API.
#[derive(Debug, Clone)]
enum Op {
    AddTag { slot: usize, tag: usize, deferred: bool },
    RemoveTag { slot: usize, tag: usize, deferred: bool },
    Despawn { slot: usize, deferred: bool },
    /// Re-spawns a DEAD slot (`spawn_empty`); a live slot is left untouched.
    Respawn { slot: usize, deferred: bool },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..POOL, 0..TAGS, any::<bool>())
            .prop_map(|(slot, tag, deferred)| Op::AddTag { slot, tag, deferred }),
        3 => (0..POOL, 0..TAGS, any::<bool>())
            .prop_map(|(slot, tag, deferred)| Op::RemoveTag { slot, tag, deferred }),
        1 => (0..POOL, any::<bool>()).prop_map(|(slot, deferred)| Op::Despawn { slot, deferred }),
        2 => (0..POOL, any::<bool>()).prop_map(|(slot, deferred)| Op::Respawn { slot, deferred }),
    ]
}

/// Mints (idempotently) the fixed process-global tag set.
fn tag_set(world: &mut EcsMaster) -> Vec<TagId> {
    TAG_NAMES.iter().map(|name| world.register_tag(name)).collect()
}

/// Deferred `spawn_empty` returning the captured handle.
fn deferred_spawn_empty(world: &mut EcsMaster) -> Entity {
    let slot: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&slot);
    world.run_system(move |mut cmds: Commands| {
        *probe.lock().expect("not poisoned") = Some(cmds.spawn_empty().id());
    });
    let captured = slot.lock().expect("not poisoned").take();
    captured.expect("the system ran and captured an entity")
}

/// Asserts world state == oracle: per-slot liveness, per-(slot, tag)
/// `has_tag`, and per-tag id-keyed query membership.
fn assert_oracle(
    world: &EcsMaster,
    handles: &[Entity],
    oracle: &[Option<HashSet<usize>>],
    tags: &[TagId],
    step: usize,
) {
    for (slot, &handle) in handles.iter().enumerate() {
        let live = oracle[slot].is_some();
        assert_eq!(
            world.has_entity(handle),
            live,
            "step {step}: slot {slot} liveness diverged from the oracle"
        );
        for (t, &tag) in tags.iter().enumerate() {
            let expected = oracle[slot].as_ref().is_some_and(|s| s.contains(&t));
            assert_eq!(
                world.has_tag(handle, tag),
                expected,
                "step {step}: has_tag(slot {slot}, tag {t}) diverged from the oracle"
            );
        }
    }
    // The id-keyed query view must agree with the oracle membership set.
    for (t, &tag) in tags.iter().enumerate() {
        let mut expected: Vec<usize> = handles
            .iter()
            .zip(oracle.iter())
            .filter(|(_, set)| set.as_ref().is_some_and(|s| s.contains(&t)))
            .map(|(e, _)| e.id().0)
            .collect();
        expected.sort_unstable();
        let mut got: Vec<usize> = world
            .query_entities(&[tag.component_id()])
            .into_iter()
            .map(|e| e.id().0)
            .collect();
        got.sort_unstable();
        assert_eq!(
            got, expected,
            "step {step}: query_entities for tag {t} diverged from the oracle"
        );
    }
}

proptest! {
    // Each case spins up a fresh EcsMaster and roughly half the ops go
    // through a full run_system apply window — modest case count (the
    // phase19 hierarchy property precedent).
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Random direct/deferred tag-op interleave; the world must match the
    /// HashMap membership oracle after EVERY operation.
    #[test]
    fn tag_membership_matches_oracle_after_every_op(
        ops in proptest::collection::vec(op_strategy(), 1..32)
    ) {
        let mut world = EcsMaster::new();
        let tags = tag_set(&mut world);

        // Initial population: POOL live empty entities.
        let mut handles: Vec<Entity> = (0..POOL).map(|_| world.spawn_empty()).collect();
        let mut oracle: Vec<Option<HashSet<usize>>> =
            (0..POOL).map(|_| Some(HashSet::new())).collect();
        assert_oracle(&world, &handles, &oracle, &tags, 0);

        for (step, op) in ops.into_iter().enumerate() {
            match op {
                // Issued even against dead handles: silent no-op contract
                // (plan D9) on BOTH routes — the oracle only mutates when
                // the slot is live.
                Op::AddTag { slot, tag, deferred } => {
                    let (e, t) = (handles[slot], tags[tag]);
                    if deferred {
                        world.run_system(move |mut cmds: Commands| {
                            cmds.entity(e).add_tag(t);
                        });
                    } else {
                        world.add_tag(e, t);
                    }
                    if let Some(set) = oracle[slot].as_mut() {
                        set.insert(tag);
                    }
                }
                Op::RemoveTag { slot, tag, deferred } => {
                    let (e, t) = (handles[slot], tags[tag]);
                    if deferred {
                        world.run_system(move |mut cmds: Commands| {
                            cmds.entity(e).remove_tag(t);
                        });
                    } else {
                        world.remove_tag(e, t);
                    }
                    if let Some(set) = oracle[slot].as_mut() {
                        set.remove(&tag);
                    }
                }
                Op::Despawn { slot, deferred } => {
                    let e = handles[slot];
                    if deferred {
                        // EC11: a deferred despawn of a stale handle
                        // debug_assert-panics BY DESIGN — only enqueue
                        // against live slots (see the header carve-out).
                        if oracle[slot].is_some() {
                            world.run_system(move |mut cmds: Commands| {
                                cmds.entity(e).despawn();
                            });
                        }
                    } else {
                        // Returns false for an already-dead handle — both
                        // outcomes are legal here.
                        let _ = world.delete_entity(e);
                    }
                    oracle[slot] = None;
                }
                Op::Respawn { slot, deferred } => {
                    // Only a DEAD slot respawns — a live slot is untouched
                    // (the property is about tag membership, not double
                    // bookkeeping of handles).
                    if oracle[slot].is_none() {
                        handles[slot] = if deferred {
                            deferred_spawn_empty(&mut world)
                        } else {
                            world.spawn_empty()
                        };
                        oracle[slot] = Some(HashSet::new());
                    }
                }
            }

            assert_oracle(&world, &handles, &oracle, &tags, step + 1);
        }
    }
}
