//! Dense plan D4 — Miri-TB coverage for the dense-component SERDE round-trip.
//!
//! The reviewer flagged that the serde-remap path was NOT Miri-covered. The
//! correctness suite (`dense_d4_serde.rs`) exercises the save→load round-trip for
//! values, but on the STABLE toolchain — it never walks `load_dense_store`'s
//! `unsafe` byte-parse (the `slice_at` / `read_u64` POB header decode) nor the
//! `store.insert`-on-loaded-bytes write path (the dense column blit + per-slot
//! tick stamp on the loaded snapshot) under the UB checker.
//!
//! This file saves a dense world → loads it into a FRESH world → asserts the
//! values and memberships are restored, run single-threaded so Miri-TB can model
//! it (the load path is single-threaded by construction — no threadpool, unlike
//! the parallel change-detection suite). It is a SMALL fixture (4 dense members)
//! so Miri's interpreter stays fast.
//!
//! Run (Tree Borrows, the project oracle; `-Zmiri-ignore-leaks` for the documented
//! Commands-apply RawVec leak, `-Zmiri-disable-isolation` so the in-memory buffer
//! path is unaffected by the isolated-clock):
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation" \
//!   cargo +nightly miri test --test dense_d4_serde_miri
//! ```

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

/// 16-byte POD dense "body" payload (the physics-body shape; blit/`SerPod`). The
/// POB dense path the loader decodes via `load_dense_store`.
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
/// (loaded ids are FRESH — equality is by this key, not by id).
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C)]
struct Key {
    k: u64,
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

/// Saves `src` → loads into a fresh world → asserts values + memberships restored.
///
/// The load path is the load-bearing Miri target: `load_dense_store` parses the
/// dense store block bytes (`slice_at` / `read_u64` provenance over the borrowed
/// `&[u8]`) and `store.insert`s each loaded member, blitting the loaded bytes into
/// the dense column and stamping the per-slot ticks — all on memory it just wrote.
/// Miri-TB checks every read/write there is in-bounds and well-provenanced.
#[test]
fn miri_dense_serde_roundtrip_restores_values_and_memberships_no_ub() {
    // SMALL fixture (4 dense members) — Miri's interpreter is ~100× slower, so the
    // save/load byte-walk is kept tiny while still exercising every unsafe path
    // (header decode, multi-member blit loop, per-slot tick stamp, id remap).
    let mut src = EcsMaster::new();
    src.run_system(|mut cmds: Commands| {
        for i in 0..4u64 {
            cmds.spawn(KeyBody {
                key: Key { k: 7000 + i },
                body: body(i as f32 + 3.0),
            });
        }
    });

    let want = key_body_pairs(&mut src);
    assert_eq!(want.len(), 4, "4 dense members in the source");

    // Save → fresh-world load. The save serializes the compacted dense snapshot;
    // the load re-parses it through the unsafe POB decode + dense-store insert.
    let mut bytes = Vec::new();
    save_world(&src, &SaveOptions::default(), &mut bytes).expect("save");

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    // The dense store + every membership round-tripped through the unsafe decode.
    assert_eq!(report.dense_stores_loaded, 1, "exactly one dense store restored");
    assert_eq!(report.dense_members_loaded, 4, "all 4 dense memberships restored");
    assert_eq!(report.entities_loaded, 4, "4 (Key) entities materialized");

    // Values + memberships are bit-identical: the loaded bytes blitted into the
    // dense column read back exactly, and each dense member rides its remapped
    // owning entity (joined via the table `Key`).
    let got = key_body_pairs(&mut dst);
    assert_eq!(
        got, want,
        "every (Key, SBody) pair must round-trip bit-identically through the unsafe load path"
    );
}
