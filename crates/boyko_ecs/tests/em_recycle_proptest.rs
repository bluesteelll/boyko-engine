//! EM2′ — property test of the entity-id life cycle across BOTH routes: the
//! dispatcher's `create_entity` / `delete_entity` / rejected create / `compact`
//! and the deferred `Commands::spawn` / `Commands::despawn`, interleaved in
//! random order.
//!
//! After every operation:
//!
//! * `EntityMaster::check_invariants` — F1..F3 over the whole recycled stack
//!   (the "test / proptest surface" `entity_master.rs` provides; before this
//!   file nothing called it).
//! * **Conservation** — `live + recycled == next_entity_id`: every id ever
//!   minted is either live or on the stack. RED on a leak (a rejected create
//!   that does not restore its id, a claim that never registers) and on a
//!   double issue (two live handles for one id, or an id both live and free).
//! * **Model agreement** — every model-live handle is valid and reads its own
//!   value; every model-dead handle is invalid (generation ABA is RED here);
//!   `entity_count()` equals the model's live count.
//!
//! No timing anywhere; every assertion is a count or an identity.

// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::prelude::*;
use boyko_macros::{Bundle, Component};
use proptest::prelude::*;

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct PVal {
    v: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct PTag {
    t: u32,
}

#[derive(Bundle)]
struct PBundle {
    v: PVal,
    t: PTag,
}

#[derive(Debug, Clone)]
enum Op {
    /// Dispatcher create (`create_entity`).
    Create,
    /// Dispatcher delete of the model-live handle at `idx % live`.
    Delete(usize),
    /// Dispatcher delete of a stale (model-dead) handle — must return `false`.
    DeleteStale(usize),
    /// `create_entity` rejected by the archetype (missing `PTag`).
    Rejected,
    /// `EntityMaster::compact`.
    Compact,
    /// One system: `n` `Commands::spawn`s.
    CmdSpawn(u8),
    /// One system: `Commands::despawn` of the model-live handle at `idx % live`.
    CmdDespawn(usize),
    /// One system: spawn, then despawn the same handle before apply.
    CmdSpawnThenDespawn,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Create),
        3 => any::<usize>().prop_map(Op::Delete),
        1 => any::<usize>().prop_map(Op::DeleteStale),
        1 => Just(Op::Rejected),
        1 => Just(Op::Compact),
        3 => (1u8..=8).prop_map(Op::CmdSpawn),
        3 => any::<usize>().prop_map(Op::CmdDespawn),
        1 => Just(Op::CmdSpawnThenDespawn),
    ]
}

struct Model {
    live: Vec<(Entity, u32)>,
    dead: Vec<Entity>,
    next_value: u32,
}

fn check(world: &mut EcsMaster, m: &Model, step: usize, op: &Op) -> Result<(), TestCaseError> {
    world.entity_master_mut().check_invariants();
    let live = world.entity_count();
    let recycled = world.recycled_entity_count();
    let minted = world.entity_master().next_entity_id().0;
    prop_assert_eq!(live, m.live.len(), "step {} ({:?}): entity_count vs model", step, op);
    prop_assert_eq!(
        live + recycled,
        minted,
        "step {} ({:?}): conservation - {} live + {} recycled != {} minted",
        step,
        op,
        live,
        recycled,
        minted
    );
    for (e, v) in &m.live {
        let got = world.get_component::<PVal>(*e).map(|p| p.v);
        prop_assert_eq!(got, Some(*v), "step {} ({:?}): live {:?}", step, op, e);
    }
    for e in &m.dead {
        prop_assert!(!world.has_entity(*e), "step {} ({:?}): dead {:?} is valid again", step, op, e);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn id_life_cycle_conserves_ids_across_both_routes(ops in prop::collection::vec(op(), 1..96)) {
        let a = PVal::component_id();
        let b = PTag::component_id();
        let mut world = EcsMaster::new();
        let arch = world.create_archetype(&[a, b]);
        let mut m = Model { live: Vec::with_capacity(256), dead: Vec::with_capacity(256), next_value: 0 };
        let tag = 0u32.to_ne_bytes();

        for (step, op) in ops.iter().enumerate() {
            match *op {
                Op::Create => {
                    let v = m.next_value;
                    m.next_value += 1;
                    let bytes = v.to_ne_bytes();
                    let e = world.create_entity(arch, &[(a, &bytes), (b, &tag)]).expect("create");
                    m.live.push((e, v));
                }
                Op::Delete(i) => {
                    if !m.live.is_empty() {
                        let (e, _) = m.live.swap_remove(i % m.live.len());
                        prop_assert!(world.delete_entity(e), "step {}: delete of live {:?}", step, e);
                        m.dead.push(e);
                    }
                }
                Op::DeleteStale(i) => {
                    if !m.dead.is_empty() {
                        let e = m.dead[i % m.dead.len()];
                        prop_assert!(!world.delete_entity(e), "step {}: stale delete of {:?} succeeded", step, e);
                    }
                }
                Op::Rejected => {
                    let bytes = 0u32.to_ne_bytes();
                    let r = world.create_entity(arch, &[(a, &bytes)]);
                    prop_assert!(r.is_err(), "step {}: a create missing PTag succeeded", step);
                }
                Op::Compact => world.entity_master_mut().compact(),
                Op::CmdSpawn(n) => {
                    let first = m.next_value;
                    m.next_value += n as u32;
                    let handles: Vec<Entity> = world.run_system(move |mut cmds: Commands| {
                        (0..n as u32)
                            .map(|k| cmds.spawn(PBundle { v: PVal { v: first + k }, t: PTag { t: 0 } }).id())
                            .collect::<Vec<Entity>>()
                    });
                    for (k, e) in handles.into_iter().enumerate() {
                        m.live.push((e, first + k as u32));
                    }
                }
                Op::CmdDespawn(i) => {
                    if !m.live.is_empty() {
                        let (e, _) = m.live.swap_remove(i % m.live.len());
                        world.run_system(move |mut cmds: Commands| cmds.despawn(e));
                        m.dead.push(e);
                    }
                }
                Op::CmdSpawnThenDespawn => {
                    let e = world.run_system(|mut cmds: Commands| {
                        let e = cmds.spawn(PBundle { v: PVal { v: 0 }, t: PTag { t: 0 } }).id();
                        cmds.despawn(e);
                        e
                    });
                    m.dead.push(e);
                }
            }
            check(&mut world, &m, step, op)?;
        }
    }
}
