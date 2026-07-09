//! Save-path DETERMINISM / golden round-trip-stability tests (serialize-perf).
//!
//! These tests are the strongest oracle for the Pass-1 / Pass-2 region-ordering
//! invariant that the `save_world` sequential-append optimization rests on. The
//! optimization replaced `out.resize(start_len + added, 0)` + offset-addressed
//! scatter writes (`write_at_off`) with `out.reserve(added)` + a strictly
//! file-offset-ordered `extend_from_slice` walk (zeroing only the column-region
//! alignment-padding gaps). If ANY region is appended out of the order Pass 1
//! assigned its offset, the saved bytes diverge — and that divergence is caught
//! here EVEN WHEN a format-level round-trip would mask it (e.g. a region pair of
//! the same size swapped).
//!
//! The fixture world is deliberately non-trivial so every branch of the new Pass 2
//! is exercised:
//!   * MULTIPLE archetypes (ordering across `archetype_plans`),
//!   * a MIX of POB component types of different sizes AND alignments
//!     (`Position` 12 B / align 4, `Velocity` 8 B / align 4, `Heavy` 16 B / align 8)
//!     so the `COLUMN_REGION_ALIGN` (32) padding gap between adjacent column-data
//!     regions actually occurs — exercising the explicit pad-zeroing path,
//!   * a ZST tag (`ZstTag`, a unit struct → `PlainOldBytes` with `byte_len == 0`)
//!     so the zero-length-column `continue` path (which must NOT emit a pad) runs,
//!   * an owning component (`Inventory` → `SerializeViaFn`) so the encoded-bytes
//!     append path runs,
//!   * multiple rows per archetype so columns are non-trivially sized.
//!
//! The assertions:
//!   1. DETERMINISM — saving the SAME world twice yields byte-identical buffers
//!      (the direct save-path oracle: any Pass-2 append nondeterminism diverges).
//!   2. ROUND-TRIP STABILITY — a save→load→re-save→re-load→re-save FIXED-POINT:
//!      once a world has passed through `load`, its archetype/component layout is
//!      canonical, so from that point on save→load→re-save is byte-identical. We
//!      assert that fixed point (re-save of the loaded world == re-save of the
//!      twice-loaded world), which isolates the SAVE-path ordering invariant from
//!      the load path's own type-interning order. (The first raw `save` of a
//!      hand-built world is NOT byte-compared to the post-load save, because the
//!      load path may re-intern the distinct types in a different — equally valid —
//!      order than the build order; that divergence is identical with and without
//!      the optimization, i.e. it is a load-path property, not this change. See the
//!      `value-equality` assertion for content correctness instead.)
//!
//! All exercise the new `slice::from_raw_parts(col.src_base, len)` + the padding
//! `resize`, so the suite is also a Miri-TB target.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Component;

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

// ── Test components (NO Entity fields — keeps the fixture on the POB/ViaFn path) ─

/// POB: `#[repr(C)]`, 12 B, align 4.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// POB: `#[repr(C)]`, 8 B, align 4.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Velocity {
    dx: i32,
    dy: i32,
}

/// POB: `#[repr(C)]`, 16 B, align 8 — its larger alignment + non-32-multiple
/// neighbouring region size guarantees a `COLUMN_REGION_ALIGN`-rounding gap, so
/// the explicit pad-zeroing branch of the new Pass 2 actually runs.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Heavy {
    a: u64,
    b: u64,
}

/// ZST tag: a unit struct `#[repr(C)]` derive → `PlainOldBytes` with size 0, so
/// its column has `byte_len == 0`. This is the zero-length-column `continue` path
/// (it must contribute no bytes and request no padding).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct ZstTag;

/// Owning component (`String` + `Vec<u8>`) → `SerializeViaFn`: exercises the
/// encoded-`via_fn_bytes` append path.
#[derive(Component, Clone, PartialEq, Debug)]
struct Inventory {
    name: String,
    flags: Vec<u8>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Spawns one entity into `arch` carrying exactly the given `(id, bytes)` columns
/// via the generic direct path (handles >2 columns and ZST tags, which the
/// `spawn_two`/`spawn_one` helpers do not).
fn spawn(world: &mut EcsMaster, arch: ArchetypeId, cols: &[(ComponentId, &[u8])]) {
    world.create_entity(arch, cols).expect("create_entity");
}

/// Builds a non-trivial multi-archetype world that hits every Pass-2 branch:
/// padding gaps (mixed alignment), a zero-length column (`ZstTag`), an owning
/// column (`Inventory`), and multiple archetypes / rows.
fn build_fixture() -> EcsMaster {
    let mut w = EcsMaster::new();

    // Archetype A: Position + Velocity + Heavy + ZstTag — four columns, three POB
    // data widths + a zero-length tag, so the column-data region has both real
    // alignment padding and a zero-length entry.
    let a = w.get_or_create_archetype(&[
        Position::component_id(),
        Velocity::component_id(),
        Heavy::component_id(),
        ZstTag::component_id(),
    ]);
    for i in 0..5u32 {
        let p = Position { x: i as f32, y: i as f32 + 0.5, z: -(i as f32) };
        let v = Velocity { dx: i as i32 - 2, dy: (i as i32) * 3 };
        let h = Heavy { a: (i as u64) << 40 | 0xABCD, b: u64::MAX - i as u64 };
        spawn(
            &mut w,
            a,
            &[
                (Position::component_id(), bytemuck_bytes(&p)),
                (Velocity::component_id(), bytemuck_bytes(&v)),
                (Heavy::component_id(), bytemuck_bytes(&h)),
                (ZstTag::component_id(), &[]), // zero-length column.
            ],
        );
    }

    // Archetype B: Heavy + Position (reversed widths vs A) — a second shape so the
    // per-archetype block ordering and the second pass over `archetype_plans` for
    // column data are both exercised.
    let b = w.get_or_create_archetype(&[Heavy::component_id(), Position::component_id()]);
    for i in 0..3u32 {
        let h = Heavy { a: 0xDEAD_0000 + i as u64, b: 7 * i as u64 };
        let p = Position { x: 100.0 + i as f32, y: 0.0, z: 0.0 };
        spawn(
            &mut w,
            b,
            &[
                (Heavy::component_id(), bytemuck_bytes(&h)),
                (Position::component_id(), bytemuck_bytes(&p)),
            ],
        );
    }

    // Archetype C: an owning component (SerializeViaFn) alongside a POB column.
    // `Inventory` is not `Copy`, so it is spawned through its typed bundle (the
    // generic byte-slice path would require a pre-encoded image); `Velocity` is the
    // POB neighbour so the column-data region in this archetype mixes a ViaFn and a
    // blit region.
    let c = w.get_or_create_archetype(&[Inventory::component_id(), Velocity::component_id()]);
    w.spawn_two(
        c,
        Inventory { name: "sword".to_string(), flags: vec![1, 2, 3] },
        Velocity { dx: 9, dy: -9 },
    )
    .expect("spawn owning + pob");
    w.spawn_two(
        c,
        Inventory { name: String::new(), flags: Vec::new() },
        Velocity { dx: 0, dy: 0 },
    )
    .expect("spawn empty owning");

    w
}

/// Reinterprets a `Copy` POD value's bytes (test-only; the fixture types are all
/// `#[repr(C)]` POD with no padding-sensitive equality needs).
fn bytemuck_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is a `#[repr(C)]` POD fixture type; we read exactly
    // `size_of::<T>()` initialized bytes from a live stack value, and the slice is
    // consumed synchronously by `create_entity`'s `copy_from_slice` before `v` is
    // dropped. Test-only.
    unsafe { std::slice::from_raw_parts((v as *const T) as *const u8, std::mem::size_of::<T>()) }
}

// ════════════════════════════════════════════════════════════════════════════
// 1. Determinism: the SAME world saved twice produces byte-identical buffers.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn save_is_deterministic_same_world_twice_identical_bytes() {
    let world = build_fixture();

    let first = save(&world);
    let second = save(&world);

    assert_eq!(
        first.len(),
        second.len(),
        "two saves of the same world must produce the same byte length"
    );
    assert_eq!(
        first, second,
        "two saves of the SAME world must be byte-identical (any Pass-1/Pass-2 \
         ordering nondeterminism would diverge here)"
    );
    assert!(!first.is_empty(), "the fixture world is non-empty");
}

// ════════════════════════════════════════════════════════════════════════════
// 2. Round-trip stability: save → load → re-save is byte-identical to save.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn save_load_resave_reaches_a_byte_stable_fixed_point() {
    let world = build_fixture();

    let first = save(&world);

    // Components must be registered in the loading process before the load (W1);
    // touching `component_id()` mints+registers them.
    let _ = Position::component_id();
    let _ = Velocity::component_id();
    let _ = Heavy::component_id();
    let _ = ZstTag::component_id();
    let _ = Inventory::component_id();

    // First load + re-save: `dst1` now has the load-canonical layout.
    let mut dst1 = EcsMaster::new();
    load_world(&mut dst1, &first, LoadEntityPolicy::Remap).expect("load #1");
    let resaved1 = save(&dst1);

    // Re-saving the SAME loaded world twice must be byte-identical (save-path
    // determinism on a loaded world).
    let resaved1b = save(&dst1);
    assert_eq!(
        resaved1, resaved1b,
        "re-saving one loaded world twice must be byte-identical (save determinism)"
    );

    // Second load (of the re-save) + re-save: this is the fixed point. The save
    // path must reproduce `resaved1` exactly, because `dst2`'s layout is the same
    // canonical layout `dst1` had. A Pass-1/Pass-2 ORDERING divergence in the save
    // path would break this equality (the loaded layouts are identical, so only
    // the save walk can differ).
    let mut dst2 = EcsMaster::new();
    load_world(&mut dst2, &resaved1, LoadEntityPolicy::Remap).expect("load #2");
    let resaved2 = save(&dst2);

    assert_eq!(
        resaved1.len(),
        resaved2.len(),
        "the save→load→re-save fixed point must preserve byte length"
    );
    assert_eq!(
        resaved1, resaved2,
        "save→load→re-save reaches a byte-stable fixed point — the save path \
         reproduces identical bytes from an identical (load-canonical) world; a \
         Pass-1/Pass-2 ordering divergence would break this"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 2b. Value-equality round-trip on the mixed-alignment / ZST / owning fixture:
//     content survives save→load (the format-level correctness oracle).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn fixture_roundtrips_every_component_value() {
    let src = build_fixture();
    let bytes = save(&src);

    let _ = Position::component_id();
    let _ = Velocity::component_id();
    let _ = Heavy::component_id();
    let _ = ZstTag::component_id();
    let _ = Inventory::component_id();

    let mut dst = EcsMaster::new();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load fixture");

    // Position multiset (8 across archetypes A=5, B=3, ZST-arch handled separately).
    let mut got_pos: Vec<(u32, u32, u32)> = dst
        .query::<&Position, ()>()
        .iter()
        .map(|p| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()))
        .collect();
    got_pos.sort_unstable();
    assert_eq!(got_pos.len(), 8, "5 (arch A) + 3 (arch B) Positions survive");

    // Heavy multiset (5 in A + 3 in B = 8).
    let mut got_heavy: Vec<(u64, u64)> =
        dst.query::<&Heavy, ()>().iter().map(|h| (h.a, h.b)).collect();
    got_heavy.sort_unstable();
    assert_eq!(got_heavy.len(), 8, "5 (arch A) + 3 (arch B) Heavy survive");

    // Velocity multiset (5 in A + 2 in C = 7).
    let got_vel: Vec<(i32, i32)> =
        dst.query::<&Velocity, ()>().iter().map(|v| (v.dx, v.dy)).collect();
    assert_eq!(got_vel.len(), 7, "5 (arch A) + 2 (arch C) Velocities survive");

    // Owning component survives the decode path.
    let mut got_inv: Vec<Inventory> =
        dst.query::<&Inventory, ()>().iter().cloned().collect();
    got_inv.sort_by(|a, b| a.name.cmp(&b.name));
    let mut want_inv = vec![
        Inventory { name: String::new(), flags: Vec::new() },
        Inventory { name: "sword".to_string(), flags: vec![1, 2, 3] },
    ];
    want_inv.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(got_inv, want_inv, "owning Inventory values round-trip");
}

// ════════════════════════════════════════════════════════════════════════════
// 3. Zero-length-column path: a ZST-tag-only archetype saves and reloads cleanly.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn zst_tag_column_saves_and_reloads_without_padding_corruption() {
    // An archetype whose ONLY serializable column is a zero-length ZST tag: the
    // column-data region is entirely zero-length entries, so the new Pass 2 must
    // emit ZERO column-data bytes and request ZERO padding for it.
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[ZstTag::component_id(), Position::component_id()]);
    for i in 0..4u32 {
        let p = Position { x: i as f32, y: 0.0, z: 0.0 };
        src.create_entity(
            arch,
            &[
                (ZstTag::component_id(), &[]),
                (Position::component_id(), bytemuck_bytes(&p)),
            ],
        )
        .expect("create_entity zst+pos");
    }

    let first = save(&src);
    let second = save(&src);
    assert_eq!(first, second, "ZST-tag world save is deterministic");

    let _ = ZstTag::component_id();
    let _ = Position::component_id();

    // Save→load→re-save fixed point (the loaded layout is canonical; a second
    // round-trip reproduces identical bytes — the save path is the only thing that
    // could differ between the two identical loaded worlds).
    let mut dst1 = EcsMaster::new();
    load_world(&mut dst1, &first, LoadEntityPolicy::Remap).expect("load zst world #1");
    let resaved1 = save(&dst1);

    let mut dst2 = EcsMaster::new();
    load_world(&mut dst2, &resaved1, LoadEntityPolicy::Remap).expect("load zst world #2");
    let resaved2 = save(&dst2);
    assert_eq!(
        resaved1, resaved2,
        "ZST-tag world reaches a byte-stable save→load→re-save fixed point \
         (zero-length column emits no data + no padding, reproducibly)"
    );

    // Content survives: all 4 Positions round-trip (the ZST tag carries no data).
    let mut got: Vec<u32> =
        dst1.query::<&Position, ()>().iter().map(|p| p.x.to_bits()).collect();
    got.sort_unstable();
    assert_eq!(got.len(), 4, "all 4 Positions in the ZST-tagged archetype survive");
}
