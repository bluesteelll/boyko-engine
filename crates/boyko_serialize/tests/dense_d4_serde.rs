//! Dense plan D4 — dense-component serialization round-trip (Decision 7).
//!
//! Gates (the D4 serde contract):
//! * Serialize → deserialize a world with a dense component → BIT-IDENTICAL dense
//!   column data (the compacted live snapshot) and the SAME memberships, with each
//!   owning `EntityId` REMAPPED to its fresh load-time id.
//! * A round-trip preserves WHICH entities have the dense component and their
//!   values (matched by a paired unique table key, since loaded ids are fresh).
//! * Remap correctness: the dense `s2e` saved ids resolve through the SAME
//!   `LoadEntityMap` the archetype loads populate — a dense member's owning entity
//!   maps to the fresh id, and a `ChildOf`-style remap on the SAME entity stays
//!   consistent (the dense membership rides the remapped entity).
//! * Tombstone-free snapshot: a store with despawned (tombstoned) members saves
//!   only the LIVE members (the compacted snapshot), so the reload has exactly the
//!   live set.
//!
//! The dense component is a POD (`SerPod`/blit) type — the physics-body case. A
//! non-POD dense type's value-encode is a documented v1.1 follow-up (the loader
//! skips a ViaFn dense block), so this suite covers the shipped POB path.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

// ── Test components ────────────────────────────────────────────────────────────

/// 16-byte POD dense "body" payload (the physics-body shape; blit/`SerPod`).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct SBody {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A unique TABLE key so a loaded entity can be matched back to its source value
/// (loaded entity ids are FRESH — equality is by this key, not by id).
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C)]
struct Key {
    k: u64,
}

/// An OWNING dense component: `Clone` but NOT `Copy` (a `Vec` field), so it
/// classifies `Serializability::SerializeViaFn` — the v1.1 path the loader cannot
/// yet decode. v1 SKIPS it on load and records the skip in
/// `LoadReport::dense_stores_skipped`. Exists to exercise the W1 observable-skip
/// counter; the POD `SBody` above covers the shipped POB dense path.
#[derive(Component, Clone, PartialEq, Debug)]
#[component(storage = "dense")]
struct OwningBody {
    tag: u32,
    payload: Vec<u8>,
}

/// `(Key, SBody)` spawn bundle — a dense `SBody` rides a table `Key`.
#[derive(Bundle)]
struct KeyBody {
    key: Key,
    body: SBody,
}

#[inline]
fn body(seed: f32) -> SBody {
    SBody { x: seed, y: seed + 0.5, z: -seed, w: seed * 2.0 }
}

/// A `SBody`'s value decomposed to raw bits for an order-stable compare (`f32` is
/// not `Ord`).
type BodyBits = (u32, u32, u32, u32);

#[inline]
fn body_bits(b: &SBody) -> BodyBits {
    (b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), b.w.to_bits())
}

/// Saves `world` into a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Collects `(Key.k, SBody bits)` for every entity that has BOTH a `Key` and an
/// `SBody`, as a sorted multiset.
fn key_body_pairs(world: &mut EcsMaster) -> Vec<(u64, BodyBits)> {
    let mut v: Vec<(u64, BodyBits)> = world
        .query::<(&Key, &SBody), ()>()
        .iter()
        .map(|(k, b)| (k.k, body_bits(b)))
        .collect();
    v.sort_unstable();
    v
}

// ════════════════════════════════════════════════════════════════════════════
// Round-trip: dense memberships + values survive, ids remapped.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dense_roundtrip_preserves_memberships_and_values() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..5u64 {
            cmds.spawn(KeyBody {
                key: Key { k: 1000 + i },
                body: body(i as f32),
            });
        }
    });

    let want = key_body_pairs(&mut src);
    assert_eq!(want.len(), 5, "5 dense members in the source");

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    assert_eq!(report.dense_stores_loaded, 1, "exactly one dense store restored");
    assert_eq!(report.dense_members_loaded, 5, "all 5 dense memberships restored");
    assert_eq!(report.entities_loaded, 5, "5 (Key) entities materialized");

    // The (Key, SBody) multiset is bit-identical across the round-trip: which
    // entities have the dense component AND their values both survived. The dense
    // member rides the SAME entity as its `Key` (remapped to the fresh id), so the
    // join via `Query<(&Key, &SBody)>` proves the dense membership maps to the
    // correct fresh entity.
    let got = key_body_pairs(&mut dst);
    assert_eq!(got, want, "every (Key, SBody) pair must round-trip bit-identically");
}

// ════════════════════════════════════════════════════════════════════════════
// Tombstone-free snapshot: despawned members are NOT saved (compacted).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dense_save_is_compacted_snapshot_skips_tombstones() {
    let mut src = EcsMaster::new();
    let ids: Vec<_> = (0..6u64)
        .map(|i| {
            src.run_system(move |mut cmds: Commands| {
                cmds.spawn(KeyBody {
                    key: Key { k: 2000 + i },
                    body: body(i as f32 + 100.0),
                })
                .id()
            })
        })
        .collect();

    // Despawn the even-indexed entities → their dense slots are tombstoned.
    for (i, &e) in ids.iter().enumerate() {
        if i % 2 == 0 {
            src.delete_entity(e);
        }
    }

    let want = key_body_pairs(&mut src);
    assert_eq!(want.len(), 3, "3 live dense members after despawning the even set");

    let bytes = save(&src);
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // Only the 3 LIVE members were saved (the compacted, tombstone-free snapshot).
    assert_eq!(report.dense_members_loaded, 3, "only the live dense members are saved");

    let got = key_body_pairs(&mut dst);
    assert_eq!(got, want, "the reloaded live dense set matches the source live set");
}

// ════════════════════════════════════════════════════════════════════════════
// A dense-only world (no table component on the dense entity).
// ════════════════════════════════════════════════════════════════════════════

/// Single-dense-component spawn bundle (a bare dense component does not auto-impl
/// `Bundle`).
#[derive(Bundle)]
struct BodyOnly {
    body: SBody,
}

#[test]
fn dense_only_world_roundtrips() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..4u32 {
            cmds.spawn(BodyOnly { body: body(i as f32 + 50.0) });
        }
    });

    // The dense-only entities have a real (possibly empty-signature) archetype;
    // the dense store carries 4 members.
    let mut want: Vec<BodyBits> = src
        .query::<&SBody, ()>()
        .iter()
        .map(body_bits)
        .collect();
    want.sort_unstable();
    assert_eq!(want.len(), 4);

    let bytes = save(&src);
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");
    assert_eq!(report.dense_members_loaded, 4, "all 4 dense-only members restored");

    let mut got: Vec<BodyBits> = dst
        .query::<&SBody, ()>()
        .iter()
        .map(body_bits)
        .collect();
    got.sort_unstable();
    assert_eq!(got, want, "dense-only values round-trip bit-identically");
}

// ════════════════════════════════════════════════════════════════════════════
// A table-only world stays byte-identical to the pre-dense path (0%-gate).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn table_only_world_emits_no_dense_region() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Key::component_id()]);
    for i in 0..3u64 {
        src.spawn_one(arch, Key { k: 9000 + i }).expect("spawn");
    }

    let bytes = save(&src);
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // No dense store touched → the dense region is empty (the 0%-gate).
    assert_eq!(report.dense_stores_loaded, 0, "a table-only world emits no dense store");
    assert_eq!(report.dense_members_loaded, 0);
    assert_eq!(report.entities_loaded, 3);
}

// ════════════════════════════════════════════════════════════════════════════
// Save → load → re-save reaches a byte-stable fixed point (determinism).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dense_save_load_resave_is_byte_stable() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..5u64 {
            cmds.spawn(KeyBody {
                key: Key { k: 3000 + i },
                body: body(i as f32 + 7.0),
            });
        }
    });

    let bytes1 = save(&src);
    let mut dst = EcsMaster::new();
    load_world(&mut dst, &bytes1, LoadEntityPolicy::Remap).expect("load");
    let bytes2 = save(&dst);

    // The compacted snapshot is canonical insertion order, so a save → load →
    // re-save reaches a byte-stable fixed point (the dense region included).
    assert_eq!(bytes1, bytes2, "dense round-trip must reach a byte-stable fixed point");
}

// ════════════════════════════════════════════════════════════════════════════
// W1: an OWNING (SerializeViaFn) dense block is an OBSERVABLE skip on load —
// it bumps LoadReport::dense_stores_skipped / dense_members_skipped instead of
// silently dropping the membership.
// ════════════════════════════════════════════════════════════════════════════

/// Single owning-dense-component spawn bundle.
#[derive(Bundle)]
struct OwningOnly {
    body: OwningBody,
}

#[test]
fn owning_dense_block_is_observable_skip_on_load() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..3u32 {
            cmds.spawn(OwningOnly {
                body: OwningBody { tag: i, payload: vec![i as u8; (i + 1) as usize] },
            });
        }
    });

    // The owning dense store carries 3 live members in the source.
    let live = src.query::<&OwningBody, ()>().iter().count();
    assert_eq!(live, 3, "3 owning-dense members in the source");

    let bytes = save(&src);

    // Register the owning component in the destination so the loader resolves it as
    // a still-dense, owning (SerializeViaFn) type — the W1 skip path.
    let mut dst = EcsMaster::new();
    let _ = OwningBody::component_id();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // W1: the owning dense block is SKIPPED (v1 has no decode path) but OBSERVABLY —
    // the counters record it instead of a silent data loss.
    assert_eq!(report.dense_stores_skipped, 1, "the owning dense store is counted skipped");
    assert_eq!(report.dense_members_skipped, 3, "all 3 skipped owning members are counted");
    // It is NOT a loaded dense store (no POB blit happened).
    assert_eq!(report.dense_stores_loaded, 0, "no owning dense store is loaded in v1");
    assert_eq!(report.dense_members_loaded, 0);
    // The owning entities still materialized (they stay valid without the dense value).
    assert_eq!(report.entities_loaded, 3, "the owning entities still load");
}
