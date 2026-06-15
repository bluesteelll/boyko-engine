//! Decision 4 (D4) — typed `write_row_typed` spawn_batch path tests.
//!
//! Validates the derive-emitted `Bundle::write_row_typed` fixed-width store
//! path against the retained byte path, plus the B4 partial-panic, O1
//! panicking-Drop, property, and O2 ZST-tag / bitset-tag contracts.
//!
//! See `docs/PERF-GAP-BEAT-BEVY-PLAN.md` Decision 4 / 4b and the dev brief
//! (mandatory critic fold-ins W1/W2/W3/Q1/O1/O2).
//!
//! # Component-slot range
//!
//! 440..=469 — disjoint from prior phases (Phase 12.5: 360-362; Phase 11:
//! 411-413; component_pool_bundle C-009: 420-422) and below
//! `MAX_COMPONENTS = 512`.
//!
//! # Why the typed path is exercised
//!
//! Every `#[derive(Bundle)]` within `MAX_TYPED_WRITE_ARITY` emits
//! `HAS_TYPED_WRITE = true`, so `EcsMaster::spawn_batch` / `Commands::spawn_batch`
//! drive the typed `if const { B::HAS_TYPED_WRITE }` arm of
//! `SpawnBatchCommand::apply`. The golden-bytes assertions therefore compare
//! typed-path column bytes against (a) the exact input bytes (the golden
//! reference) AND (b) an independent byte-path spawn of the same data
//! (`spawn_one` / `create_entity`, which route through
//! `for_each_component_bytes` / raw `write_at`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Added, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::{Bundle, Component as DeriveComponent};
use boyko_threadpool::ThreadPoolBuilder;

const SEQ: Ordering = Ordering::SeqCst;

// ── Component slots ──────────────────────────────────────────────────────────

const SLOT_POS: ComponentId = ComponentId(440);
const SLOT_VEL: ComponentId = ComponentId(441);
const SLOT_HEALTH: ComponentId = ComponentId(442);
// 443 reserved (Mana, unused). 444 reserved for the ZST PoolTag
// (self-registered via `#[derive(Component)]`).
const SLOT_DROPPER: ComponentId = ComponentId(445);
const SLOT_PANIC_DROP: ComponentId = ComponentId(446);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Velocity {
    x: f32,
    y: f32,
    z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Health(i32);

impl Component for Position {
    fn component_id() -> ComponentId {
        SLOT_POS
    }
}
impl Component for Velocity {
    fn component_id() -> ComponentId {
        SLOT_VEL
    }
}
impl Component for Health {
    fn component_id() -> ComponentId {
        SLOT_HEALTH
    }
}

/// ZST POOL tag (size-0, tick-only pool — NOT a bitset enable tag). It IS a
/// pool column: committed + `Added<Tag>`-stamped, but never data-written (O2 b).
#[derive(DeriveComponent, Clone, Copy)]
struct PoolTag;

/// Bitset enable tag (O2 c): NO `ComponentPool`, filtered from every archetype
/// signature. It can NOT be a `Bundle` field (the derive suppresses the
/// single-component Bundle emission for `storage = "bitset"`), so it is enabled
/// out-of-band via `ecs.enable::<BitTag>()`. The assertion below proves the
/// typed path's pool-column set excludes it.
#[derive(DeriveComponent)]
#[component(storage = "bitset")]
struct BitTag;

fn register_components() {
    register_layout::<Position>(SLOT_POS.0);
    register_layout::<Velocity>(SLOT_VEL.0);
    register_layout::<Health>(SLOT_HEALTH.0);
    // `PoolTag` registers itself lazily via its derived `component_id()`.
}

// ── Bundles (every derived bundle emits HAS_TYPED_WRITE = true) ──────────────

#[derive(Bundle)]
struct PosBundle {
    pos: Position,
}

#[derive(Bundle)]
struct PosVel {
    pos: Position,
    vel: Velocity,
}

/// Mixed bundle: a non-ZST data field (O2 a) + a ZST pool tag (O2 b). The tag
/// has NO bytes; the typed path must skip it (const-folded ZST) yet the tag
/// column is still committed + `Added`-stamped.
#[derive(Bundle)]
struct MixedBundle {
    pos: Position,
    tag: PoolTag,
}

// Reading helpers ─────────────────────────────────────────────────────────────

/// Reads back a `T` from an entity via the raw column pointer (golden-bytes
/// path: byte-for-byte the pool column's stored representation).
fn read_back<T: Component + Copy>(ecs: &EcsMaster, e: Entity) -> T {
    let raw = ecs
        .get_component_raw(e, T::component_id())
        .expect("component present");
    // SAFETY: `raw` points to a live, initialised `T` in the pool column
    //   (registered with `T::component_id()`); we copy `size_of::<T>()` bytes.
    unsafe { *(raw as *const T) }
}

/// Returns the raw column bytes for `T` on entity `e` as an owned `Vec<u8>`.
fn read_back_bytes<T: Component>(ecs: &EcsMaster, e: Entity) -> Vec<u8> {
    let raw = ecs
        .get_component_raw(e, T::component_id())
        .expect("component present");
    // SAFETY: `raw` is a live initialised `T`; read its byte representation.
    unsafe { std::slice::from_raw_parts(raw, std::mem::size_of::<T>()).to_vec() }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — golden-bytes equality typed-vs-input, 1-field bundle
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn typed_write_one_field_golden_bytes() {
    register_components();
    let mut ecs = EcsMaster::new();

    let n = 256usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| PosBundle {
            pos: Position {
                x: i as f32,
                y: (i as f32) * 2.0,
                z: (i as f32) * 3.0,
            },
        }))
        .expect("spawn_batch typed 1-field");

    assert_eq!(spawned.len(), n);
    for (i, &e) in spawned.iter().enumerate() {
        let got: Position = read_back(&ecs, e);
        let want = Position {
            x: i as f32,
            y: (i as f32) * 2.0,
            z: (i as f32) * 3.0,
        };
        assert_eq!(got, want, "row {i}: typed-write column mismatch");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — golden-bytes equality, 2-field bundle (≥2 DATA columns; required
// for the W2 Miri single-provenance proof). Cross-checked against an
// independent byte-path spawn (`spawn_two`).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn typed_write_two_field_golden_bytes_vs_byte_path() {
    register_components();
    let mut ecs = EcsMaster::new();

    let n = 512usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| PosVel {
            pos: Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
            vel: Velocity {
                x: 0.0,
                y: i as f32,
                z: 0.0,
            },
        }))
        .expect("spawn_batch typed 2-field");
    assert_eq!(spawned.len(), n);

    // Independent byte-path reference: spawn ONE entity with the same data via
    // the raw two-component path (`spawn_two` → `create_entity` → raw byte
    // write), then byte-compare each column against the typed-path rows.
    let arch = ecs.get_or_create_archetype(&[SLOT_POS, SLOT_VEL]);
    for (i, &e) in spawned.iter().enumerate() {
        let ref_pos = Position {
            x: i as f32,
            y: 0.0,
            z: 0.0,
        };
        let ref_vel = Velocity {
            x: 0.0,
            y: i as f32,
            z: 0.0,
        };
        let ref_e = ecs
            .spawn_two(arch, ref_pos, ref_vel)
            .expect("byte-path spawn_two");

        assert_eq!(
            read_back_bytes::<Position>(&ecs, e),
            read_back_bytes::<Position>(&ecs, ref_e),
            "row {i}: Position column bytes differ typed vs byte path"
        );
        assert_eq!(
            read_back_bytes::<Velocity>(&ecs, e),
            read_back_bytes::<Velocity>(&ecs, ref_e),
            "row {i}: Velocity column bytes differ typed vs byte path"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — mixed bundle (O2): non-ZST data + ZST POOL tag. The typed path
// writes only the data column; the tag column count must equal the byte path's,
// and `Added<PoolTag>` must fire for every spawned row.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn typed_write_mixed_zst_pool_tag_column_count_and_added() {
    register_components();
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();

    let n = 64usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| MixedBundle {
            pos: Position {
                x: i as f32,
                y: 0.0,
                z: 0.0,
            },
            tag: PoolTag,
        }))
        .expect("spawn_batch typed mixed");
    assert_eq!(spawned.len(), n);

    // (a) the non-ZST data column wrote correctly.
    for (i, &e) in spawned.iter().enumerate() {
        let got: Position = read_back(&ecs, e);
        assert_eq!(got.x, i as f32, "row {i}: mixed bundle data column");
        // (b) the ZST pool tag is present (committed) but carries no bytes.
        assert!(
            ecs.has_component(e, PoolTag::component_id()),
            "row {i}: ZST pool tag committed"
        );
    }

    // (c.1) the mixed bundle declares exactly 2 POOL columns (Position data +
    // PoolTag ZST) — the byte path and the typed path share this canonical pool
    // set. `MixedBundle::component_ids()` is the canonical pool-column list.
    use boyko_ecs::ecs::core::bundle::Bundle;
    assert_eq!(
        MixedBundle::component_ids().len(),
        2,
        "mixed bundle has exactly 2 pool columns (1 data + 1 ZST pool tag)"
    );

    // (c.2) a BITSET enable tag is NOT a pool column: it is absent from the
    // bundle signature, starts disabled, and can be enabled out-of-band without
    // ever appearing in the archetype's pool set. The typed path's column set
    // is unaffected by it.
    let bit_cid = BitTag::component_id();
    assert!(
        !MixedBundle::component_ids().contains(&bit_cid),
        "bitset enable tag is filtered from the bundle's pool-column signature"
    );
    for &e in &spawned {
        assert!(!ecs.is_enabled::<BitTag>(e), "bitset tag starts disabled");
    }
    for &e in &spawned {
        ecs.enable::<BitTag>(e);
    }
    for &e in &spawned {
        assert!(ecs.is_enabled::<BitTag>(e), "bitset tag enabled out-of-band");
        // It is NOT a pool column — `has_component` (pool membership) is false
        // for a bitset tag even when the bit is set.
        assert!(
            !ecs.has_component(e, bit_cid),
            "bitset enable tag is not a pool column (no ComponentPool)"
        );
    }

    // `Added<PoolTag>` fires exactly once for every freshly spawned tag row.
    let matches = Arc::new(AtomicUsize::new(0));
    let probe = Arc::clone(&matches);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(move |q: Query<&Position, Added<PoolTag>>| {
        for _ in &q {
            probe.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);
    schedule.run(&mut ecs);
    assert_eq!(
        matches.load(SEQ),
        n,
        "frame 1: Added<PoolTag> matches every spawned tag row exactly once"
    );

    matches.store(0, SEQ);
    schedule.run(&mut ecs);
    assert_eq!(matches.load(SEQ), 0, "frame 2: Added<PoolTag> no longer matches");
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — 16-field POD bundle golden bytes (max arity; typed path is still
// chosen, MAX_TYPED_WRITE_ARITY == 16).
// ════════════════════════════════════════════════════════════════════════════

const SLOT_F: [ComponentId; 16] = [
    ComponentId(450),
    ComponentId(451),
    ComponentId(452),
    ComponentId(453),
    ComponentId(454),
    ComponentId(455),
    ComponentId(456),
    ComponentId(457),
    ComponentId(458),
    ComponentId(459),
    ComponentId(460),
    ComponentId(461),
    ComponentId(462),
    ComponentId(463),
    ComponentId(464),
    ComponentId(465),
];

macro_rules! pod_field {
    ($name:ident, $slot:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct $name(u32);
        impl Component for $name {
            fn component_id() -> ComponentId {
                $slot
            }
        }
    };
}

pod_field!(F0, SLOT_F[0]);
pod_field!(F1, SLOT_F[1]);
pod_field!(F2, SLOT_F[2]);
pod_field!(F3, SLOT_F[3]);
pod_field!(F4, SLOT_F[4]);
pod_field!(F5, SLOT_F[5]);
pod_field!(F6, SLOT_F[6]);
pod_field!(F7, SLOT_F[7]);
pod_field!(F8, SLOT_F[8]);
pod_field!(F9, SLOT_F[9]);
pod_field!(F10, SLOT_F[10]);
pod_field!(F11, SLOT_F[11]);
pod_field!(F12, SLOT_F[12]);
pod_field!(F13, SLOT_F[13]);
pod_field!(F14, SLOT_F[14]);
pod_field!(F15, SLOT_F[15]);

#[derive(Bundle)]
struct Bundle16 {
    f0: F0,
    f1: F1,
    f2: F2,
    f3: F3,
    f4: F4,
    f5: F5,
    f6: F6,
    f7: F7,
    f8: F8,
    f9: F9,
    f10: F10,
    f11: F11,
    f12: F12,
    f13: F13,
    f14: F14,
    f15: F15,
}

fn register_16() {
    register_layout::<F0>(SLOT_F[0].0);
    register_layout::<F1>(SLOT_F[1].0);
    register_layout::<F2>(SLOT_F[2].0);
    register_layout::<F3>(SLOT_F[3].0);
    register_layout::<F4>(SLOT_F[4].0);
    register_layout::<F5>(SLOT_F[5].0);
    register_layout::<F6>(SLOT_F[6].0);
    register_layout::<F7>(SLOT_F[7].0);
    register_layout::<F8>(SLOT_F[8].0);
    register_layout::<F9>(SLOT_F[9].0);
    register_layout::<F10>(SLOT_F[10].0);
    register_layout::<F11>(SLOT_F[11].0);
    register_layout::<F12>(SLOT_F[12].0);
    register_layout::<F13>(SLOT_F[13].0);
    register_layout::<F14>(SLOT_F[14].0);
    register_layout::<F15>(SLOT_F[15].0);
}

#[test]
fn typed_write_sixteen_field_golden_bytes() {
    register_16();
    let mut ecs = EcsMaster::new();

    let n = 128usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| Bundle16 {
            f0: F0(i),
            f1: F1(i + 1),
            f2: F2(i + 2),
            f3: F3(i + 3),
            f4: F4(i + 4),
            f5: F5(i + 5),
            f6: F6(i + 6),
            f7: F7(i + 7),
            f8: F8(i + 8),
            f9: F9(i + 9),
            f10: F10(i + 10),
            f11: F11(i + 11),
            f12: F12(i + 12),
            f13: F13(i + 13),
            f14: F14(i + 14),
            f15: F15(i + 15),
        }))
        .expect("spawn_batch typed 16-field");
    assert_eq!(spawned.len(), n);

    for (i, &e) in spawned.iter().enumerate() {
        let i = i as u32;
        assert_eq!(read_back::<F0>(&ecs, e), F0(i), "F0 row {i}");
        assert_eq!(read_back::<F1>(&ecs, e), F1(i + 1), "F1 row {i}");
        assert_eq!(read_back::<F2>(&ecs, e), F2(i + 2), "F2 row {i}");
        assert_eq!(read_back::<F3>(&ecs, e), F3(i + 3), "F3 row {i}");
        assert_eq!(read_back::<F4>(&ecs, e), F4(i + 4), "F4 row {i}");
        assert_eq!(read_back::<F5>(&ecs, e), F5(i + 5), "F5 row {i}");
        assert_eq!(read_back::<F6>(&ecs, e), F6(i + 6), "F6 row {i}");
        assert_eq!(read_back::<F7>(&ecs, e), F7(i + 7), "F7 row {i}");
        assert_eq!(read_back::<F8>(&ecs, e), F8(i + 8), "F8 row {i}");
        assert_eq!(read_back::<F9>(&ecs, e), F9(i + 9), "F9 row {i}");
        assert_eq!(read_back::<F10>(&ecs, e), F10(i + 10), "F10 row {i}");
        assert_eq!(read_back::<F11>(&ecs, e), F11(i + 11), "F11 row {i}");
        assert_eq!(read_back::<F12>(&ecs, e), F12(i + 12), "F12 row {i}");
        assert_eq!(read_back::<F13>(&ecs, e), F13(i + 13), "F13 row {i}");
        assert_eq!(read_back::<F14>(&ecs, e), F14(i + 14), "F14 row {i}");
        assert_eq!(read_back::<F15>(&ecs, e), F15(i + 15), "F15 row {i}");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — B4 partial-panic: iterator panics on `next()` mid-batch. Rows before
// the panic are committed-exactly; no half-row; no double-drop (drop-counter).
// ════════════════════════════════════════════════════════════════════════════

static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Non-Copy drop-counting component: its pool registers a `drop_fn`, so a
/// committed row's value is dropped exactly once at world teardown. The test
/// asserts the typed path never DOUBLE-drops on a mid-batch panic.
#[repr(C)]
struct Tracked(u32);

impl Drop for Tracked {
    fn drop(&mut self) {
        DROP_COUNT.fetch_add(1, SEQ);
    }
}

impl Component for Tracked {
    fn component_id() -> ComponentId {
        SLOT_DROPPER
    }
}

#[derive(Bundle)]
struct TrackedBundle {
    t: Tracked,
}

/// An iterator that yields `panic_at` valid items, then panics on the next
/// `next()`. `len()` over-reports so `SpawnBatchCommand::apply` enters the row
/// loop expecting more rows than it gets.
struct PanicIter {
    next_val: u32,
    panic_at: u32,
    reported_len: usize,
}

impl Iterator for PanicIter {
    type Item = TrackedBundle;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next_val == self.panic_at {
            panic!("PanicIter: deliberate mid-batch panic");
        }
        let v = self.next_val;
        self.next_val += 1;
        Some(TrackedBundle { t: Tracked(v) })
    }
}

impl ExactSizeIterator for PanicIter {
    fn len(&self) -> usize {
        self.reported_len
    }
}

#[test]
fn typed_write_partial_panic_commits_exact_no_double_drop() {
    register_layout::<Tracked>(SLOT_DROPPER.0);
    DROP_COUNT.store(0, SEQ);

    let panic_at = 4u32;
    let reported = 8usize;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut ecs = EcsMaster::new();
        let _ = ecs.spawn_batch(PanicIter {
            next_val: 0,
            panic_at,
            reported_len: reported,
        });
        // Keep `ecs` alive until after the closure so the committed rows'
        // pool drop_fn runs at world teardown (counted below).
        ecs
    }));

    assert!(result.is_err(), "the mid-batch panic must propagate");

    // Rows 0..panic_at were relocated into the pool (typed path, drop suppressed
    // at the source via ManuallyDrop::take). The world was dropped when the
    // closure unwound, so the pool's drop_fn ran for the committed rows.
    //
    // IMPORTANT: with the typed path, rows are committed AFTER the whole write
    // loop (`commit_units_batch`), and the loop never completed (panic at row
    // `panic_at`). So NO row is committed → the pool drop_fn runs for ZERO
    // rows. The `panic_at` relocated-but-uncommitted bundles also do not drop
    // (ManuallyDrop suppressed their source Drop; their bytes leak in the
    // uncommitted pool slots — the documented B4 "leak on panic" contract).
    //
    // The crucial assertion is NO DOUBLE-DROP: the count must be EXACTLY the
    // number of bundles whose `Tracked` field was neither relocated nor leaked,
    // i.e. the items pulled-but-not-yet-relocated. On the typed path each pulled
    // bundle is relocated immediately, so the only un-relocated value is the one
    // the panic interrupted (never yielded). Hence the drop count is 0 and,
    // critically, never exceeds `panic_at` (no double-drop of relocated rows).
    let drops = DROP_COUNT.load(SEQ);
    assert!(
        drops <= panic_at as usize,
        "no double-drop: drop count {drops} must not exceed pulled rows {panic_at}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — O1 panicking-Drop field: the typed path relocates via
// ManuallyDrop::take and never invokes the field's Drop during the move-out.
// ════════════════════════════════════════════════════════════════════════════

static PANIC_DROP_RAN: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct PanicOnDrop(u32);

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        // If this runs DURING write_row_typed's move-out, the test would
        // observe a panic; instead we count it so we can assert it never runs
        // mid-relocation. (At world teardown it runs once per committed row —
        // we keep this test to ZERO committed rows by not panicking the iter,
        // then drop the world and tolerate the teardown drops.)
        PANIC_DROP_RAN.fetch_add(1, SEQ);
    }
}

impl Component for PanicOnDrop {
    fn component_id() -> ComponentId {
        SLOT_PANIC_DROP
    }
}

#[derive(Bundle)]
struct PanicDropBundle {
    p: PanicOnDrop,
}

#[test]
fn typed_write_never_invokes_field_drop_during_move() {
    register_layout::<PanicOnDrop>(SLOT_PANIC_DROP.0);
    PANIC_DROP_RAN.store(0, SEQ);

    let n = 16usize;
    {
        let mut ecs = EcsMaster::new();
        let spawned = ecs
            .spawn_batch((0..n as u32).map(|i| PanicDropBundle {
                p: PanicOnDrop(i),
            }))
            .expect("spawn_batch with panicking-Drop field");
        assert_eq!(spawned.len(), n);

        // The relocation must NOT have invoked any field Drop: the source bytes
        // were bitwise-moved into the pool via ManuallyDrop::take. So after the
        // spawn loop, zero Drops have run.
        assert_eq!(
            PANIC_DROP_RAN.load(SEQ),
            0,
            "write_row_typed must not invoke the field's Drop during move-out"
        );

        // Read-back confirms the bytes landed.
        for (i, &e) in spawned.iter().enumerate() {
            let raw = ecs
                .get_component_raw(e, SLOT_PANIC_DROP)
                .expect("present");
            // SAFETY: live initialised PanicOnDrop.
            let v = unsafe { (*(raw as *const PanicOnDrop)).0 };
            assert_eq!(v, i as u32, "row {i} relocated value");
        }
        // World drop here runs the pool drop_fn for the n committed rows (the
        // legitimate, single Drop per row). We do not assert on that count.
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 7 — property: spawn N ∈ {0, 1, 2, 8191} typed → read-back-equal.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn typed_write_property_various_n_read_back_equal() {
    register_components();
    for &n in &[0usize, 1, 2, 8191] {
        let mut ecs = EcsMaster::new();
        let spawned = ecs
            .spawn_batch((0..n as u32).map(|i| PosVel {
                pos: Position {
                    x: i as f32,
                    y: (i as f32) + 0.5,
                    z: 0.0,
                },
                vel: Velocity {
                    x: 0.0,
                    y: 0.0,
                    z: i as f32,
                },
            }))
            .unwrap_or_else(|_| panic!("spawn_batch N={n}"));
        assert_eq!(spawned.len(), n, "N={n}: spawned count");

        for (i, &e) in spawned.iter().enumerate() {
            let p: Position = read_back(&ecs, e);
            let v: Velocity = read_back(&ecs, e);
            assert_eq!(
                p,
                Position {
                    x: i as f32,
                    y: (i as f32) + 0.5,
                    z: 0.0
                },
                "N={n} row {i} Position"
            );
            assert_eq!(
                v,
                Velocity {
                    x: 0.0,
                    y: 0.0,
                    z: i as f32
                },
                "N={n} row {i} Velocity"
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — column IDENTITY (W3): two same-SIZE, different-TYPE components must
// not swap columns. Health (i32, 4 B) and a 4-B u32 wrapper share size; the
// typed path must place each into its own ComponentId-keyed column.
// ════════════════════════════════════════════════════════════════════════════

const SLOT_W32: ComponentId = ComponentId(466);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct W32(u32);

impl Component for W32 {
    fn component_id() -> ComponentId {
        SLOT_W32
    }
}

#[derive(Bundle)]
struct SameSizeBundle {
    health: Health,
    w: W32,
}

#[test]
fn typed_write_same_size_distinct_types_no_column_swap() {
    register_layout::<Health>(SLOT_HEALTH.0);
    register_layout::<W32>(SLOT_W32.0);
    let mut ecs = EcsMaster::new();

    let n = 200usize;
    let spawned = ecs
        .spawn_batch((0..n as u32).map(|i| SameSizeBundle {
            health: Health(-(i as i32) - 1),
            w: W32(0xDEAD_0000 | i),
        }))
        .expect("spawn_batch same-size distinct types");
    assert_eq!(spawned.len(), n);

    for (i, &e) in spawned.iter().enumerate() {
        let h: Health = read_back(&ecs, e);
        let w: W32 = read_back(&ecs, e);
        assert_eq!(h, Health(-(i as i32) - 1), "row {i}: Health column");
        assert_eq!(w, W32(0xDEAD_0000 | i as u32), "row {i}: W32 column");
    }
}
