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
//! The primary dense component is a POD (`SerPod`/blit) type — the physics-body
//! case. An OWNING dense type (a `Vec` field → `SerializeViaFn`) is DECODED
//! per-member on load (the v1.1 dense ViaFn path, B9): this suite covers BOTH the
//! POB blit path and the ViaFn decode path (values + memberships round-trip, and a
//! mixed POB + ViaFn dense save reloads every store).

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
/// classifies `Serializability::SerializeViaFn`. Every field is `Wire` (`u32` +
/// `Vec<u8>`), so the derive auto-installs a `deserialize_fn` and the loader DECODES
/// it per-member (the v1.1 dense ViaFn path, B9) — its `tag` + `payload` round-trip
/// through a save→load. The POD `SBody` above covers the POB blit path.
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
// B9: an OWNING (SerializeViaFn) dense block is DECODED per-member on load — its
// values + memberships survive a save→load round-trip (not skipped).
// ════════════════════════════════════════════════════════════════════════════

/// Single owning-dense-component spawn bundle.
#[derive(Bundle)]
struct OwningOnly {
    body: OwningBody,
}

/// Collects `(tag, payload)` for every live `OwningBody`, sorted for an
/// order-stable compare (loaded entity ids are fresh, so equality is by value).
fn owning_pairs(world: &mut EcsMaster) -> Vec<(u32, Vec<u8>)> {
    let mut v: Vec<(u32, Vec<u8>)> = world
        .query::<&OwningBody, ()>()
        .iter()
        .map(|b| (b.tag, b.payload.clone()))
        .collect();
    v.sort_unstable();
    v
}

/// (a) An owning dense component with a ViaFn type survives save→load: its values
/// (tag + variable-length `Vec` payload) and the live member count are equal across
/// the round-trip. This is the headline B9 regression — a ViaFn dense store used to
/// be SKIPPED on load (silent data loss); it is now decoded per-member.
#[test]
fn owning_dense_block_decodes_on_load() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..3u32 {
            cmds.spawn(OwningOnly {
                // Distinct, variable-length payloads so a byte-move / decode bug
                // (wrong stride, torn cursor) shows up as a value mismatch.
                body: OwningBody { tag: 700 + i, payload: vec![i as u8; (i + 1) as usize] },
            });
        }
    });

    let want = owning_pairs(&mut src);
    assert_eq!(want.len(), 3, "3 owning-dense members in the source");

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = OwningBody::component_id();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // B9: the owning dense block is DECODED (not skipped) — one store, all members.
    assert_eq!(report.dense_stores_loaded, 1, "the owning dense store is decoded + loaded");
    assert_eq!(report.dense_members_loaded, 3, "all 3 owning dense members are decoded");
    assert_eq!(report.dense_stores_skipped, 0, "no owning dense store is skipped now");
    assert_eq!(report.dense_members_skipped, 0);
    assert_eq!(report.entities_loaded, 3, "the owning entities materialize");

    // Values + row count survive the round-trip bit-identically.
    let got = owning_pairs(&mut dst);
    assert_eq!(got, want, "every (tag, payload) must round-trip through the ViaFn dense decode");
}

/// (b) Mixed POB + ViaFn dense stores in ONE save: a `SBody` (POB/blit) dense store
/// and an `OwningBody` (ViaFn/decode) dense store both restore fully — the loader
/// blits one and decodes the other in the same dense region.
#[test]
fn mixed_pob_and_viafn_dense_stores_roundtrip() {
    let mut src = EcsMaster::new();
    // POB dense members (with a table Key so they can be matched back).
    src.run_system(|mut cmds: Commands| {
        for i in 0..4u64 {
            cmds.spawn(KeyBody {
                key: Key { k: 5000 + i },
                body: body(i as f32 + 3.0),
            });
        }
    });
    // ViaFn dense members (a disjoint entity set).
    src.run_system(|mut cmds: Commands| {
        for i in 0..3u32 {
            cmds.spawn(OwningOnly {
                body: OwningBody { tag: 900 + i, payload: vec![0xA0 | i as u8; (i + 2) as usize] },
            });
        }
    });

    let want_pob = key_body_pairs(&mut src);
    let want_owning = owning_pairs(&mut src);
    assert_eq!(want_pob.len(), 4);
    assert_eq!(want_owning.len(), 3);

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = OwningBody::component_id();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // BOTH dense stores load: 2 stores, 4 + 3 = 7 members, nothing skipped.
    assert_eq!(report.dense_stores_loaded, 2, "both the POB and ViaFn dense stores load");
    assert_eq!(report.dense_members_loaded, 7, "4 POB + 3 ViaFn dense members restored");
    assert_eq!(report.dense_stores_skipped, 0);
    assert_eq!(report.dense_members_skipped, 0);

    assert_eq!(key_body_pairs(&mut dst), want_pob, "POB dense values round-trip");
    assert_eq!(owning_pairs(&mut dst), want_owning, "ViaFn dense values round-trip");
}

/// (c) An owning dense round-trip reaches a byte-stable fixed point (save→load→
/// re-save is byte-identical) — the ViaFn decode reconstructs the exact compacted
/// snapshot the saver re-emits, so the wire bytes are deterministic across a reload.
#[test]
fn owning_dense_save_load_resave_is_byte_stable() {
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..4u32 {
            cmds.spawn(OwningOnly {
                body: OwningBody { tag: 42 + i, payload: vec![i as u8; (i + 1) as usize] },
            });
        }
    });

    let bytes1 = save(&src);
    let mut dst = EcsMaster::new();
    let _ = OwningBody::component_id();
    load_world(&mut dst, &bytes1, LoadEntityPolicy::Remap).expect("load");
    let bytes2 = save(&dst);

    assert_eq!(bytes1, bytes2, "owning dense round-trip must reach a byte-stable fixed point");
}
