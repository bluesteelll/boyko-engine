//! Phase X.F — arena growth integration tests (plan §Test matrix I1-I2).
//!
//! * **I1** — a DEFAULT `EcsMaster` creates ~30 single-component archetypes
//!   (~90-100 MB of component pools): past the pre-X.F 64 MB arena ceiling,
//!   where archetype creation used to panic inside
//!   `Arena::allocate_from_free_blocks`. The default arena now grows inside
//!   its multi-GB reservation. `#[cfg_attr(miri, ignore)]` (R2-W3b): the
//!   Miri fallback default reserve is 64 MiB, and growth under Miri is
//!   already covered by I2 + `tests/miri_arena_growth.rs`; I1's purpose is
//!   the real-OS past-the-old-ceiling witness.
//!
//! * **I2** — `EcsMaster::with_arena_reserve(16 MiB)` spawns across 4
//!   archetypes whose pool allocations cross >= 2 commit slabs, then full
//!   query iteration validates data integrity: values written into slabs
//!   committed at different times must read back exactly (pointer-stability
//!   witness — growth never moves previously returned blocks).
//!
//! # Component-slot ranges
//!
//! I1: 66..=95 (30 slots); I2: 43..=47 (5 slots) — both inside free runs
//! disjoint from every other reserved range in the codebase at the time of
//! writing (`MAX_COMPONENTS = 512` cap respected).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── I1 — default world past the old 64 MB ceiling ───────────────────────────

/// 12-byte payload: with the default pool sizing (`DEFAULT_CHUNKS_PER_POOL =
/// 128` x `TINY_COMPONENTS_PER_CHUNK = 2048` x 12 B) every single-component
/// archetype's pool takes a contiguous 3 MiB arena block.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fat12 {
    a: f32,
    b: f32,
    c: f32,
}

const I1_SLOT_BASE: usize = 66;
const I1_ARCHETYPES: usize = 30;

#[test]
#[cfg_attr(miri, ignore)] // R2-W3b: exceeds the 64 MiB Miri fallback default.
fn default_world_grows_past_old_64mb_ceiling() {
    // One layout registered under 30 distinct ids — each id gets its own
    // 3 MiB pool (the registry maps id -> Layout; the Rust type is shared).
    for k in 0..I1_ARCHETYPES {
        register_layout::<Fat12>(I1_SLOT_BASE + k);
    }

    let mut ecs = EcsMaster::new();
    let mut entities = Vec::with_capacity(I1_ARCHETYPES);

    for k in 0..I1_ARCHETYPES {
        let cid = ComponentId(I1_SLOT_BASE + k);
        // Pre-X.F this panics around the ~21st archetype (21 x 3 MiB > 64 MB).
        let arch = ecs.create_archetype(&[cid]);

        let value = Fat12 {
            a: k as f32,
            b: (k * 2) as f32,
            c: (k * 3) as f32,
        };
        // SAFETY: `Fat12` is `#[repr(C)]` with three f32 fields (no padding);
        // viewing it as 12 initialized bytes is valid for the lifetime of
        // `value` within this iteration.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&value as *const Fat12).cast::<u8>(),
                std::mem::size_of::<Fat12>(),
            )
        };
        let entity = ecs
            .create_entity(arch, &[(cid, bytes)])
            .expect("entity creation must succeed after arena growth");
        entities.push((entity, cid, value));
    }

    assert_eq!(ecs.entity_count(), I1_ARCHETYPES);

    // Read-back witness: every value — including those written into the
    // EARLIEST slabs — survives all later growth events bit-exactly.
    for (entity, cid, expected) in entities {
        let raw = ecs
            .get_component_raw(entity, cid)
            .expect("component must be readable after growth");
        // SAFETY: `get_component_raw` returned `Some`, so `raw` points at a
        // live, initialized `Fat12` slot (the pool stores `Fat12` per the
        // `register_layout::<Fat12>` contract); reads are in bounds.
        let actual = unsafe { *(raw as *const Fat12) };
        assert_eq!(actual, expected, "component data corrupted across growth");
    }
}

// ── I2 — small reserve, multi-slab crossing + full query validation ─────────

#[repr(C)]
#[derive(Clone, Copy)]
struct Pos {
    x: f32,
    y: f32,
    z: f32,
}

macro_rules! def_tag {
    ($name:ident, $slot:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $name(u8);
        impl Component for $name {
            fn component_id() -> ComponentId {
                ComponentId($slot)
            }
        }
    };
}

impl Component for Pos {
    fn component_id() -> ComponentId {
        ComponentId(43)
    }
}

def_tag!(Tag0, 44);
def_tag!(Tag1, 45);
def_tag!(Tag2, 46);
def_tag!(Tag3, 47);

#[derive(Bundle)]
struct PosTag0 {
    pos: Pos,
    tag: Tag0,
}
#[derive(Bundle)]
struct PosTag1 {
    pos: Pos,
    tag: Tag1,
}
#[derive(Bundle)]
struct PosTag2 {
    pos: Pos,
    tag: Tag2,
}
#[derive(Bundle)]
struct PosTag3 {
    pos: Pos,
    tag: Tag3,
}

fn register_i2() {
    register_layout::<Pos>(43);
    register_layout::<Tag0>(44);
    register_layout::<Tag1>(45);
    register_layout::<Tag2>(46);
    register_layout::<Tag3>(47);
}

const PER_ROUND: usize = 1_000;
const ROUNDS: usize = 2;
const ARCH_STRIDE: usize = 100_000; // x = (arch * ARCH_STRIDE + i) as f32

/// Spawns one round of `PER_ROUND` entities into archetype `arch` (0..4),
/// encoding `(arch, global index)` into `pos.x`.
fn spawn_round(ecs: &mut EcsMaster, arch: usize, round: usize) {
    let base = arch * ARCH_STRIDE + round * PER_ROUND;
    macro_rules! spawn {
        ($bundle:ident, $tag:ident) => {
            ecs.spawn_batch((0..PER_ROUND).map(move |i| $bundle {
                pos: Pos {
                    x: (base + i) as f32,
                    y: 0.0,
                    z: 0.0,
                },
                tag: $tag(arch as u8),
            }))
            .expect("sub-batch within MAX_BATCH_HINT")
        };
    }
    match arch {
        0 => drop(spawn!(PosTag0, Tag0)),
        1 => drop(spawn!(PosTag1, Tag1)),
        2 => drop(spawn!(PosTag2, Tag2)),
        3 => drop(spawn!(PosTag3, Tag3)),
        _ => unreachable!("4 archetypes"),
    }
}

#[test]
fn small_reserve_multi_slab_growth_query_integrity() {
    register_i2();
    let mut ecs = EcsMaster::with_arena_reserve(16 * 1024 * 1024);

    // Round 0 creates the 4 archetypes lazily (4 x ~3 MiB Pos pools + tag
    // pools ~= 13 MiB of arena demand inside the 16 MiB reserve — multiple
    // 2-3 MiB-class commit slabs). Round 1 then writes into slab-0 memory
    // AFTER the later slabs were committed — the pointer-stability witness.
    for round in 0..ROUNDS {
        for arch in 0..4 {
            spawn_round(&mut ecs, arch, round);
        }
    }
    assert_eq!(ecs.entity_count(), 4 * ROUNDS * PER_ROUND);

    // Full query iteration across all 4 archetypes: every encoded value must
    // read back exactly, each exactly once.
    let mut seen = vec![false; 4 * ROUNDS * PER_ROUND];
    {
        let view = ecs.query::<&Pos, ()>();
        let mut total = 0usize;
        for p in view.iter() {
            let encoded = p.x as usize;
            let arch = encoded / ARCH_STRIDE;
            let idx = encoded % ARCH_STRIDE;
            assert!(arch < 4, "decoded archetype out of range: x = {}", p.x);
            assert!(
                idx < ROUNDS * PER_ROUND,
                "decoded index out of range: x = {}",
                p.x
            );
            assert!(p.y == 0.0 && p.z == 0.0, "untouched fields corrupted");
            let flat = arch * ROUNDS * PER_ROUND + idx;
            assert!(!seen[flat], "duplicate row for x = {}", p.x);
            seen[flat] = true;
            total += 1;
        }
        assert_eq!(total, 4 * ROUNDS * PER_ROUND, "query must visit every row");
    }
    assert!(seen.iter().all(|&s| s), "every spawned value must be visited");

    // Per-archetype tag integrity (one representative typed pair query).
    let view = ecs.query::<(&Pos, &Tag2), ()>();
    let mut count = 0usize;
    for (p, t) in view.iter() {
        assert_eq!(t.0, 2, "tag bytes corrupted in archetype 2");
        let encoded = p.x as usize;
        assert_eq!(encoded / ARCH_STRIDE, 2, "foreign row in archetype-2 query");
        count += 1;
    }
    assert_eq!(count, ROUNDS * PER_ROUND);
}
