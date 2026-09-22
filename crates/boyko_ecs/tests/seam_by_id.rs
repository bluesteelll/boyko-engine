//! EG2 gates — the by-id structural seam (`docs/REFLECTION-PLAN-ECS.md`, the
//! `### EG2` rung). Gates 1–9, 12–18; gate 10 is a `#[cfg(test)] mod` inside
//! `commands/migration_helpers.rs` (its subject is `pub(crate)`), gate 11 is
//! `crates/boyko_reflect/tests/seam_census.rs`, and gate 15b is a
//! `#[cfg(test)] mod` inside `ecs_master/seam_by_id.rs` for the same
//! `pub(crate)`-subject reason (the deferred-scope depth).
//!
//! # The fixture law (rung §B.0) — a single-entity fixture is a spec violation
//!
//! Every gate whose subject is a MIGRATION builds:
//!
//! * a **populated source** — ≥ 2 entities in the source archetype with the
//!   entity under test NOT in the last row, so `move_out_entity` returns
//!   `RemoveOutcome::Swapped` and the inland repoint actually executes;
//! * a **populated target** — ≥ 1 entity already living in the target
//!   archetype, so the new row index is non-zero;
//! * **bystander assertions on TWO axes** — bytes *and* row. The bytes axis
//!   alone is BLIND: with the `Swapped` repoint deleted the swapped bystander
//!   still reads back its original value (`swap_remove_unit_no_drop` copies the
//!   last row onto the removed row without scrubbing the vacated tail, and
//!   `get_component_raw` applies no `row < pool.count()` bound). The only
//!   public discriminator is `get_component_changed_tick(bystander, id)`, which
//!   flips `Some(..)` → `None` because it bounds `row` against `pool.count()`.
//! * **non-zero payload bytes in every position a fixture reads.** A fresh pool
//!   row is lazily committed OS memory, so an all-zero payload makes the
//!   "hooks see the bytes" gate pass over an implementation that fires Phase 2
//!   before writing them.
//!
//! # Hook registration order
//!
//! `ArchetypeFlags` hook bits are OR-computed ONCE in `create_by_ids` and —
//! unlike observer bits — have no retro-fit walk. Hooks are therefore
//! registered BEFORE any entity carrying the hooked id is spawned, and
//! `register_hooks_by_id`'s `Result` is `.expect`ed, never discarded: the
//! registry REFUSES with `HooksError::AlreadyArchetyped` on a wrong order, and
//! swallowing that `Err` would turn a loud refusal into gate 1 redding over a
//! correct implementation with the signature "the sibling forgot Phase 2".
//!
//! `ComponentHooks` has FIVE fields (`on_despawn` included); every literal here
//! uses `..Default::default()`. A four-field literal is an `E0063` — the exact
//! breakage that has had `tests/miri_phase22.rs` dead behind `#![cfg(miri)]`.
//!
//! # Why each test owns its types and its tag names
//!
//! `ComponentId`s, the `HOOKS` table and the `TAG_NAMES` intern are
//! process-global, and integration tests run in parallel threads in ONE
//! process.

// Test oracle model: the `Arc<Mutex<_>>` below is the cross-thread observation
// channel used to lift a `Commands::spawn` handle out of a system closure — the
// exact pattern the sibling dense / tag / retained-walk suites use. Never
// engine data itself. An integration-test target: compiled out of every
// shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle as BundleTrait;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    self, EnableTagId, ResidencyKind, StorageKind,
};
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::{ComponentHooks, HookContext};
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::{AddOutcome, RejectReason};
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::EcsMaster;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

const SEQ: Ordering = Ordering::SeqCst;

/// The victim payload. Non-zero in every byte position (§B.0).
const VICTIM: u32 = 0xA5A5_A5A5;
/// The pre-existing target occupant's payload — distinct from [`VICTIM`] so a
/// cross-row clobber is visible.
const OCCUPANT: u32 = 0x5A5A_5A5A;
/// Gate 3's SECOND write. Different from [`VICTIM`] on purpose: a repeated
/// payload makes the "nothing was written" half byte-identical for a clobbering
/// implementation, i.e. vacuous.
const SECOND: u32 = 0xDEAD_BEEF;

// ════════════════════════════════════════════════════════════════════════════
// Shared helpers
// ════════════════════════════════════════════════════════════════════════════

/// Spawns one bundle through `Commands::spawn` and returns the live handle.
fn spawn_one<B: BundleTrait + Send + Sync>(
    ecs: &mut EcsMaster,
    make: impl Fn() -> B + Send + Sync + 'static,
) -> Entity {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        probe.lock().expect("lock").push(cmds.spawn(make()).id());
    });
    sink.lock().expect("lock")[0]
}

/// Canonical-sorts an id array. `ComponentId`s are minted in process-global
/// registration order, which test parallelism makes unpredictable, so the sort
/// must happen at RUNTIME — the archetype funnels debug-assert canonical order.
fn sorted<const N: usize>(mut ids: [ComponentId; N]) -> [ComponentId; N] {
    ids.sort_unstable_by_key(|c| c.0);
    ids
}

/// Borrows a `#[repr(C)]` POD value's own bytes for the by-id attach.
///
/// # Safety
///
/// `T` must be `#[repr(C)]` POD whose byte span is a valid representation of
/// the id it is paired with (every caller here passes the matching type).
unsafe fn pod_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is `#[repr(C)]` POD (caller contract); the slice borrows `v`
    // and is consumed before `v` goes out of scope.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

/// Reads a component back as `T` through the public raw accessor.
///
/// # Safety
///
/// `component_id` must be `T`'s registered id and the entity must host it.
unsafe fn read_back<T: Copy>(ecs: &EcsMaster, e: Entity, component_id: ComponentId) -> T {
    let raw = ecs
        .get_component_raw(e, component_id)
        .expect("invariant: the entity must host the component being read back");
    // SAFETY: `raw` addresses the live, initialized `T` slot for this read.
    unsafe { *(raw as *const T) }
}

/// The ROW axis of the bystander assertion (§B.0). `get_component_changed_tick`
/// bounds `row` against `pool.count()`, so a stale inland row reads `None`.
fn row_is_inside_the_pool(ecs: &EcsMaster, e: Entity, id: ComponentId) -> bool {
    ecs.get_component_changed_tick(e, id).is_some()
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 1 — table add: bytes, ticks, hooks, hook-visible payload, bystanders
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G1Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G1Add(u32);

#[derive(Bundle)]
struct G1Src {
    p: G1Pos,
}

#[derive(Bundle)]
struct G1Tgt {
    p: G1Pos,
    a: G1Add,
}

static G1_ON_ADD: AtomicUsize = AtomicUsize::new(0);
static G1_ON_INSERT: AtomicUsize = AtomicUsize::new(0);
static G1_INSERT_SAW: AtomicU32 = AtomicU32::new(0);

unsafe fn g1_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G1_ON_ADD.fetch_add(1, SEQ);
}

/// Records the value it READS. A fire-count ledger plus a post-call read-back
/// is satisfied by an implementation that fires Phase 2 *before* committing the
/// bytes; this is the only assertion that is not.
unsafe fn g1_on_insert(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    G1_ON_INSERT.fetch_add(1, SEQ);
    if let Some(v) = w.get_component::<G1Add>(ctx.entity) {
        G1_INSERT_SAW.store(v.0, SEQ);
    }
}

#[test]
fn g1_table_add_lands_bytes_ticks_hooks_and_leaves_bystanders_intact() {
    // Hooks BEFORE any spawn carrying the id (the flags are frozen at
    // `create_by_ids`); the `Result` is expected, never discarded.
    component_registry::register_hooks_by_id(
        G1Add::component_id(),
        ComponentHooks {
            on_add: Some(g1_on_add),
            on_insert: Some(g1_on_insert),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let add_id = G1Add::component_id();

    // POPULATED TARGET: one entity already lives in {G1Pos, G1Add}.
    let occupant = spawn_one(&mut ecs, || G1Tgt {
        p: G1Pos(11),
        a: G1Add(OCCUPANT),
    });
    // POPULATED SOURCE: the victim is row 0 of three, so the source removal is
    // `Swapped` and the repoint of the last row actually runs.
    let victim = spawn_one(&mut ecs, || G1Src { p: G1Pos(1) });
    let _mid = spawn_one(&mut ecs, || G1Src { p: G1Pos(2) });
    let swapped_bystander = spawn_one(&mut ecs, || G1Src { p: G1Pos(3) });

    G1_ON_ADD.store(0, SEQ);
    G1_ON_INSERT.store(0, SEQ);
    G1_INSERT_SAW.store(0, SEQ);

    let payload = G1Add(VICTIM);
    // SAFETY: `G1Add` is `#[repr(C)]` POD and is the registered type of `add_id`.
    let bytes = unsafe { pod_bytes(&payload) };

    // (1) the return value
    assert_eq!(
        ecs.add_component_by_id(victim, add_id, bytes),
        AddOutcome::Attached,
        "a live entity + a table id + a right-sized slice must attach"
    );

    // (2) the bytes landed
    assert_eq!(
        // SAFETY: `G1Add` is the registered type for `add_id`; the entity hosts it.
        unsafe { read_back::<G1Add>(&ecs, victim, add_id) },
        G1Add(VICTIM),
        "the attached bytes must be readable back through the public accessor"
    );

    // (3) the hooks fired once each
    assert_eq!(G1_ON_ADD.load(SEQ), 1, "on_add fires exactly once");
    assert_eq!(G1_ON_INSERT.load(SEQ), 1, "on_insert fires exactly once");

    // (4) the hook SAW the payload — this is the ordering assertion
    assert_eq!(
        G1_INSERT_SAW.load(SEQ),
        VICTIM,
        "on_insert must read the WRITTEN bytes: Phase 2 fires AFTER the byte \
         write, so a hook that reads the component sees the payload, not the \
         uninitialised row"
    );

    // (5) the changed tick is the current tick
    assert_eq!(
        ecs.get_component_changed_tick(victim, add_id),
        Some(ecs.current_tick()),
        "a fresh attach stamps the changed tick at current_tick"
    );

    // (6) the added tick is the current tick. There is no public added-tick
    // READER; `Mut::is_added` on the direct-API path is exactly the predicate
    // `added == current_tick()`, and building the guard without `deref_mut`
    // does not perturb the changed tick.
    assert!(
        ecs.get_component_mut::<G1Add>(victim)
            .expect("the entity hosts G1Add")
            .is_added(),
        "a fresh attach stamps the added tick at current_tick"
    );

    // (7) the pre-existing target occupant is untouched (kills the row-0 pin)
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G1Add>(&ecs, occupant, add_id) },
        G1Add(OCCUPANT),
        "the migration must land at the target's TAIL row, not clobber row 0"
    );

    // (8) the swapped source bystander, on BOTH axes
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G1Pos>(&ecs, swapped_bystander, G1Pos::component_id()) },
        G1Pos(3),
        "BYTES: the swapped bystander keeps its value"
    );
    assert!(
        row_is_inside_the_pool(&ecs, swapped_bystander, G1Pos::component_id()),
        "ROW: the swapped bystander's inland row must be repointed INSIDE the \
         pool — the bytes axis alone is blind to a missing repoint"
    );
}

// ── Gate 1c — the size-0 leg (the shape the flipped `s1_` pass fixture runs) ──

#[test]
fn g1c_size_zero_table_add_is_first_class() {
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_tag("seam_g1_zst");
    let e = ecs.spawn_empty();

    assert_eq!(
        ecs.add_component_by_id(e, tag.component_id(), &[]),
        AddOutcome::Attached,
        "a size-0 column attaches through the SAME path with an empty slice"
    );
    assert!(
        ecs.has_tag(e, tag),
        "the size-0 column is present in the signature after the attach"
    );
    assert!(
        ecs.get_component_changed_tick(e, tag.component_id()).is_some(),
        "the size-0 column got a committed row with a tick, exactly like a data \
         column"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 2 — dense add: no migration, membership, bytes, hooks
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G2Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G2Dense(u32);

#[derive(Bundle)]
struct G2Src {
    p: G2Pos,
}

static G2_ON_ADD: AtomicUsize = AtomicUsize::new(0);
static G2_ON_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn g2_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G2_ON_ADD.fetch_add(1, SEQ);
}
unsafe fn g2_on_insert(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G2_ON_INSERT.fetch_add(1, SEQ);
}

#[test]
fn g2_dense_add_does_not_migrate_and_lands_bytes_and_hooks() {
    component_registry::register_hooks_by_id(
        G2Dense::component_id(),
        ComponentHooks {
            on_add: Some(g2_on_add),
            on_insert: Some(g2_on_insert),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let dense_id = G2Dense::component_id();
    let e = spawn_one(&mut ecs, || G2Src { p: G2Pos(1) });
    let _bystander = spawn_one(&mut ecs, || G2Src { p: G2Pos(2) });

    let before = ecs.entity_archetype_id(e).expect("live entity");
    G2_ON_ADD.store(0, SEQ);
    G2_ON_INSERT.store(0, SEQ);

    let payload = G2Dense(VICTIM);
    // SAFETY: `G2Dense` is `#[repr(C)]` POD and is the registered type of `dense_id`.
    let bytes = unsafe { pod_bytes(&payload) };

    assert_eq!(
        ecs.add_component_by_id(e, dense_id, bytes),
        AddOutcome::Attached,
        "a dense id attaches through the dense store"
    );
    assert_eq!(
        ecs.entity_archetype_id(e),
        Some(before),
        "the dense no-migration contract: the archetype is IDENTICAL before and \
         after"
    );
    assert!(
        ecs.dense_contains(e, dense_id),
        "the dense membership was recorded"
    );
    assert_eq!(
        // SAFETY: registered type for the dense id; the entity is a member.
        unsafe { read_back::<G2Dense>(&ecs, e, dense_id) },
        G2Dense(VICTIM),
        "the BYTES land in the dense column, not just the membership — \
         `dense_insert_and_fire` fires unconditionally, so the routing alone is \
         not the subject"
    );
    assert_eq!(G2_ON_ADD.load(SEQ), 1, "dense on_add fires exactly once");
    assert_eq!(G2_ON_INSERT.load(SEQ), 1, "dense on_insert fires exactly once");
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 3 — AlreadyPresent, both kinds, with a DIFFERENT second payload
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G3Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G3Add(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G3Dense(u32);

#[derive(Bundle)]
struct G3Src {
    p: G3Pos,
}

#[derive(Bundle)]
struct G3Tgt {
    p: G3Pos,
    a: G3Add,
}

#[test]
fn g3_table_second_add_is_already_present_and_writes_nothing() {
    let mut ecs = EcsMaster::new();
    let add_id = G3Add::component_id();

    let _occupant = spawn_one(&mut ecs, || G3Tgt {
        p: G3Pos(11),
        a: G3Add(OCCUPANT),
    });
    let victim = spawn_one(&mut ecs, || G3Src { p: G3Pos(1) });
    let _mid = spawn_one(&mut ecs, || G3Src { p: G3Pos(2) });
    let _tail = spawn_one(&mut ecs, || G3Src { p: G3Pos(3) });

    let first = G3Add(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    assert_eq!(
        ecs.add_component_by_id(victim, add_id, unsafe { pod_bytes(&first) }),
        AddOutcome::Attached
    );

    let second = G3Add(SECOND);
    // SAFETY: as above.
    assert_eq!(
        ecs.add_component_by_id(victim, add_id, unsafe { pod_bytes(&second) }),
        AddOutcome::AlreadyPresent,
        "an editor's Add Component must REFUSE a present data component, not \
         take `add_tag`'s in-place-replace path"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G3Add>(&ecs, victim, add_id) },
        G3Add(VICTIM),
        "AlreadyPresent means NOTHING was written — the second payload must not \
         be in the column"
    );
}

#[test]
fn g3_dense_second_add_is_already_present_and_writes_nothing() {
    let mut ecs = EcsMaster::new();
    let dense_id = G3Dense::component_id();
    let e = spawn_one(&mut ecs, || G3Src { p: G3Pos(1) });

    let first = G3Dense(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `dense_id`.
    assert_eq!(
        ecs.add_component_by_id(e, dense_id, unsafe { pod_bytes(&first) }),
        AddOutcome::Attached
    );
    assert!(ecs.dense_contains(e, dense_id), "membership recorded");

    let second = G3Dense(SECOND);
    // SAFETY: as above.
    assert_eq!(
        ecs.add_component_by_id(e, dense_id, unsafe { pod_bytes(&second) }),
        AddOutcome::AlreadyPresent,
        "the dense presence probe is the STORE, not the signature — a dense id \
         is filtered out of every archetype mask by construction"
    );
    assert_eq!(
        // SAFETY: registered type for the dense id; the entity is a member.
        unsafe { read_back::<G3Dense>(&ecs, e, dense_id) },
        G3Dense(VICTIM),
        "in release a dense clobber is exactly what `DenseStore::insert`'s \
         opening `debug_assert!` stops covering: a second slot, `e2s` remapped, \
         the old value orphaned undropped"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 4 — Rejected(WrongByteLen), both directions. Profile-independent.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G4Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G4Add(u32);

#[derive(Bundle)]
struct G4Src {
    p: G4Pos,
}

#[test]
fn g4_wrong_byte_len_is_rejected_in_both_directions() {
    let mut ecs = EcsMaster::new();
    let add_id = G4Add::component_id();
    let size = component_registry::get_layout(add_id.0)
        .expect("the derive registered the layout")
        .size;
    assert_eq!(size, 4, "precondition: the fixture component is 4 bytes");

    let e = spawn_one(&mut ecs, || G4Src { p: G4Pos(1) });
    let _bystander = spawn_one(&mut ecs, || G4Src { p: G4Pos(2) });

    let short = [0xA5u8; 3];
    assert_eq!(
        ecs.add_component_by_id(e, add_id, &short),
        AddOutcome::Rejected(RejectReason::WrongByteLen),
        "a SHORT slice must be refused before the pool memcpy — this is the \
         only check standing between a caller slice and a `copy_nonoverlapping` \
         of `layout.size` bytes, and it is a release `if`, not a debug_assert"
    );

    let long = [0xA5u8; 5];
    assert_eq!(
        ecs.add_component_by_id(e, add_id, &long),
        AddOutcome::Rejected(RejectReason::WrongByteLen),
        "an OVER-LONG slice is refused too: `!=`, not `<`"
    );

    assert!(
        ecs.get_component_raw(e, add_id).is_none(),
        "a refused call mutates NOTHING"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 5 — the refusal ORDER is the subject. Both legs pass a deliberately
//          WRONG-LENGTH slice, so a correct implementation still answers with
//          the kind / residency reason.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G5Pos(u32);

#[derive(Bundle)]
struct G5Src {
    p: G5Pos,
}

/// A type that exists ONLY to be minted through `register_new` and then
/// classified `ResidencyKind::Gpu`. It never enters an archetype, so
/// `residency_conflict_panic` is never reached from this fixture — which is
/// exactly the point: a residency refusal placed AFTER
/// `merged_archetype_id_dyn` would abort the process instead of returning.
#[repr(C)]
struct G5Gpu(u32);

#[test]
fn g5_bitset_id_is_rejected_as_not_table_or_dense_before_the_length_check() {
    let mut ecs = EcsMaster::new();
    let e = spawn_one(&mut ecs, || G5Src { p: G5Pos(1) });

    let bits = ecs.register_enable_tag("seam_g5_bits");
    assert_eq!(
        component_registry::storage_kind(bits.component_id().0),
        StorageKind::Bitset,
        "precondition: `register_enable_tag` classifies the minted id Bitset"
    );

    // A bitset id has layout size 0, so a 3-byte slice is ALSO wrong-length.
    // A correct implementation still answers `NotTableOrDense`, which is what
    // pins the kind check ahead of the length check.
    assert_eq!(
        ecs.add_component_by_id(e, bits.component_id(), &[0xA5u8; 3]),
        AddOutcome::Rejected(RejectReason::NotTableOrDense),
        "kind BEFORE length"
    );
}

#[test]
fn g5_gpu_resident_id_is_rejected_before_the_length_check_and_returns() {
    let mut ecs = EcsMaster::new();
    let e = spawn_one(&mut ecs, || G5Src { p: G5Pos(1) });

    let raw = component_registry::register_new::<G5Gpu>();
    component_registry::classify_component_residency(raw, ResidencyKind::Gpu);
    let gpu_id = ComponentId(raw);
    assert_eq!(
        component_registry::residency_class(gpu_id.0),
        ResidencyKind::Gpu,
        "precondition: the id is classified Gpu"
    );

    // Wrong-length slice again: a correct implementation answers `GpuResident`.
    // And it RETURNS — it does not abort. `residency_conflict_panic` is a real
    // `panic!` reached from `create_by_ids`, so a refusal placed after the
    // archetype resolver would turn this gate into a process-level failure.
    assert_eq!(
        ecs.add_component_by_id(e, gpu_id, &[0xA5u8; 3]),
        AddOutcome::Rejected(RejectReason::GpuResident),
        "residency BEFORE length, and BEFORE the archetype resolver"
    );
}

#[test]
fn g5_dead_entity_is_rejected_with_its_own_reason() {
    let mut ecs = EcsMaster::new();
    let e = spawn_one(&mut ecs, || G5Src { p: G5Pos(1) });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn();
    });
    assert!(!ecs.has_entity(e), "precondition: the handle is stale");

    let add_id = G4Add::component_id();
    let payload = G4Add(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    assert_eq!(
        ecs.add_component_by_id(e, add_id, unsafe { pod_bytes(&payload) }),
        AddOutcome::Rejected(RejectReason::EntityDead),
        "`RejectReason::EntityDead` has a reader only if this arm produces it — \
         the by-id twin REPORTS where `add_tag` silently no-ops"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 6 — remove_component_by_id, both kinds, Drop-carrying fixture
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G6Pos(u32);

/// A `Drop`-carrying table component: the detach path must run `drop_fn`
/// exactly once for the removed id.
#[derive(Component)]
#[repr(C)]
struct G6Drop(u32);

static G6_DROPS: AtomicUsize = AtomicUsize::new(0);
static G6_ON_REPLACE: AtomicUsize = AtomicUsize::new(0);
static G6_ON_REMOVE: AtomicUsize = AtomicUsize::new(0);
static G6_REPLACE_SAW: AtomicU32 = AtomicU32::new(0);
static G6_REMOVE_SAW: AtomicU32 = AtomicU32::new(0);

impl Drop for G6Drop {
    fn drop(&mut self) {
        G6_DROPS.fetch_add(1, SEQ);
    }
}

#[derive(Bundle)]
struct G6Tgt {
    p: G6Pos,
}

#[derive(Bundle)]
struct G6Src {
    p: G6Pos,
    d: G6Drop,
}

unsafe fn g6_on_replace(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    G6_ON_REPLACE.fetch_add(1, SEQ);
    if let Some(v) = w.get_component::<G6Drop>(ctx.entity) {
        G6_REPLACE_SAW.store(v.0, SEQ);
    }
}
unsafe fn g6_on_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    G6_ON_REMOVE.fetch_add(1, SEQ);
    if let Some(v) = w.get_component::<G6Drop>(ctx.entity) {
        G6_REMOVE_SAW.store(v.0, SEQ);
    }
}

#[test]
fn g6_table_remove_fires_against_the_dying_row_drops_once_and_spares_bystanders() {
    component_registry::register_hooks_by_id(
        G6Drop::component_id(),
        ComponentHooks {
            on_replace: Some(g6_on_replace),
            on_remove: Some(g6_on_remove),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let drop_id = G6Drop::component_id();

    // POPULATED TARGET: {G6Pos} already has a tenant before the detach lands.
    let occupant = spawn_one(&mut ecs, || G6Tgt { p: G6Pos(11) });
    // POPULATED SOURCE, victim at row 0 of three ⇒ `Swapped`.
    let victim = spawn_one(&mut ecs, || G6Src {
        p: G6Pos(1),
        d: G6Drop(VICTIM),
    });
    let _mid = spawn_one(&mut ecs, || G6Src {
        p: G6Pos(2),
        d: G6Drop(OCCUPANT),
    });
    let swapped_bystander = spawn_one(&mut ecs, || G6Src {
        p: G6Pos(3),
        d: G6Drop(SECOND),
    });

    G6_ON_REPLACE.store(0, SEQ);
    G6_ON_REMOVE.store(0, SEQ);
    G6_REPLACE_SAW.store(0, SEQ);
    G6_REMOVE_SAW.store(0, SEQ);
    // Reset immediately before the call: the spawn path's own bundle handling
    // is not this gate's subject, only the DELTA across the detach is.
    G6_DROPS.store(0, SEQ);

    assert!(
        ecs.remove_component_by_id(victim, drop_id),
        "a present table component detaches and reports true"
    );

    assert_eq!(G6_ON_REPLACE.load(SEQ), 1, "on_replace fires exactly once");
    assert_eq!(G6_ON_REMOVE.load(SEQ), 1, "on_remove fires exactly once");
    assert_eq!(
        G6_REPLACE_SAW.load(SEQ),
        VICTIM,
        "on_replace reads the DYING value (pre-migration source row)"
    );
    assert_eq!(
        G6_REMOVE_SAW.load(SEQ),
        VICTIM,
        "on_remove reads the DYING value too"
    );
    assert_eq!(
        G6_DROPS.load(SEQ),
        1,
        "the removed id is dropped EXACTLY once — not zero (leak) and not twice \
         (double free)"
    );
    assert!(
        ecs.get_component_raw(victim, drop_id).is_none(),
        "the column is gone from the entity"
    );
    assert!(
        !ecs.remove_component_by_id(victim, drop_id),
        "a second detach of an absent component is a silent `false` (W1)"
    );

    // Bystanders, on both axes.
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G6Pos>(&ecs, occupant, G6Pos::component_id()) },
        G6Pos(11),
        "BYTES: the pre-existing target occupant is untouched"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G6Pos>(&ecs, swapped_bystander, G6Pos::component_id()) },
        G6Pos(3),
        "BYTES: the swapped source bystander keeps its value"
    );
    assert!(
        row_is_inside_the_pool(&ecs, swapped_bystander, G6Pos::component_id()),
        "ROW: the swapped source bystander was repointed inside the pool"
    );
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G6Dense(u32);

static G6D_ON_REPLACE: AtomicUsize = AtomicUsize::new(0);
static G6D_ON_REMOVE: AtomicUsize = AtomicUsize::new(0);
static G6D_REMOVE_SAW: AtomicU32 = AtomicU32::new(0);

unsafe fn g6d_on_replace(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G6D_ON_REPLACE.fetch_add(1, SEQ);
}
unsafe fn g6d_on_remove(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    G6D_ON_REMOVE.fetch_add(1, SEQ);
    if let Some(v) = w.get_component::<G6Dense>(ctx.entity) {
        G6D_REMOVE_SAW.store(v.0, SEQ);
    }
}

#[test]
fn g6_dense_remove_routes_through_the_store_and_answers_false_when_absent() {
    component_registry::register_hooks_by_id(
        G6Dense::component_id(),
        ComponentHooks {
            on_replace: Some(g6d_on_replace),
            on_remove: Some(g6d_on_remove),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let dense_id = G6Dense::component_id();
    let e = spawn_one(&mut ecs, || G6Tgt { p: G6Pos(1) });
    let absent = spawn_one(&mut ecs, || G6Tgt { p: G6Pos(2) });

    // A signature-probed S2 would answer a silent `false` here — for a dense
    // component that is indistinguishable from "absent", which is the hole this
    // half exists to close.
    assert!(
        !ecs.remove_component_by_id(absent, dense_id),
        "an absent dense member answers false WITHOUT creating a store"
    );

    let payload = G6Dense(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `dense_id`.
    assert_eq!(
        ecs.add_component_by_id(e, dense_id, unsafe { pod_bytes(&payload) }),
        AddOutcome::Attached
    );
    G6D_ON_REPLACE.store(0, SEQ);
    G6D_ON_REMOVE.store(0, SEQ);
    G6D_REMOVE_SAW.store(0, SEQ);

    assert!(
        ecs.remove_component_by_id(e, dense_id),
        "a present dense member detaches and reports true"
    );
    assert!(!ecs.dense_contains(e, dense_id), "the membership is gone");
    assert_eq!(G6D_ON_REPLACE.load(SEQ), 1, "dense on_replace fires once");
    assert_eq!(G6D_ON_REMOVE.load(SEQ), 1, "dense on_remove fires once");
    assert_eq!(
        G6D_REMOVE_SAW.load(SEQ),
        VICTIM,
        "the dense fire is PRE-tombstone: the handler reads the dying value"
    );
    assert!(
        !ecs.remove_component_by_id(e, dense_id),
        "a second detach answers false"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 7 — mark_component_changed, observed the ONLY way it is expressible.
//
// The window advances ONLY under `Schedule::run`. Two measured non-routes,
// named here so the next reader does not re-derive them:
//
//   * `EcsMaster::query::<&T, Changed<T>>()` is a COMPILE ERROR
//     (`const { eval_query_no_change_detection::<D, F>() }`).
//   * `run_system` / `run_system_once` COMPILE AND RUN AND MATCH NOTHING,
//     FOREVER — `FunctionSystem::initialize` seeds `last_run == this_run`, so
//     `is_newer_than` computes `0 > ticks_since_insert`, false for every `u32`.
//
// Three frames, and the FIRST one MATCHES (it is the add's own frame). Frame 2
// (idle) is the no-match baseline; the mark is taken between frames 2 and 3
// from OUTSIDE the schedule; frame 3 matches again. A two-frame fixture fails
// its first assertion on a correct implementation.
//
// `bump_change_tick` is `pub(crate)` and `System::set_change_ticks` is
// unreachable from a test, so "with `last_run` set to the add's tick" is not
// expressible from out of crate.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G7Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G7Mark(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G7Dense(u32);

#[derive(Bundle)]
struct G7TableBundle {
    p: G7Pos,
    m: G7Mark,
}

#[derive(Bundle)]
struct G7DenseBundle {
    p: G7Pos,
    d: G7Dense,
}

static G7T_SEEN: AtomicUsize = AtomicUsize::new(0);
static G7D_SEEN: AtomicUsize = AtomicUsize::new(0);

#[test]
fn g7_mark_component_changed_table_is_observable_to_a_changed_query() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let e = spawn_one(&mut world, || G7TableBundle {
        p: G7Pos(1),
        m: G7Mark(VICTIM),
    });

    G7T_SEEN.store(0, SEQ);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G7Mark, Changed<G7Mark>>| {
        for _ in &q {
            G7T_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        G7T_SEEN.load(SEQ),
        1,
        "frame 1 is the ADD's OWN frame — it matches. Deleting this assertion \
         is how a two-frame fixture hides the baseline."
    );

    G7T_SEEN.store(0, SEQ);
    schedule.run(&mut world);
    assert_eq!(
        G7T_SEEN.load(SEQ),
        0,
        "frame 2 (idle) is the no-match BASELINE"
    );

    assert!(
        world.mark_component_changed(e, G7Mark::component_id()),
        "the mark reports that it found and stamped the row"
    );
    G7T_SEEN.store(0, SEQ);
    schedule.run(&mut world);
    assert_eq!(
        G7T_SEEN.load(SEQ),
        1,
        "frame 3 matches BECAUSE of the mark — without it frame 3 does not match"
    );
}

#[test]
fn g7_mark_component_changed_dense_is_observable_to_a_changed_query() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let e = spawn_one(&mut world, || G7DenseBundle {
        p: G7Pos(1),
        d: G7Dense(VICTIM),
    });

    // The READ twin has NO dense arm: `get_component_changed_tick` resolves
    // `pools.get_pool(id)?` and a dense id owns no per-archetype pool, so it
    // returns `None` for a dense component the entity HAS. The query is the
    // only observation channel for this half.
    assert!(
        world
            .get_component_changed_tick(e, G7Dense::component_id())
            .is_none(),
        "precondition (the §4 S3 asymmetry): the read twin is blind to dense"
    );

    G7D_SEEN.store(0, SEQ);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G7Dense, Changed<G7Dense>>| {
        for _ in &q {
            G7D_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(G7D_SEEN.load(SEQ), 1, "frame 1: the add's own frame matches");

    G7D_SEEN.store(0, SEQ);
    schedule.run(&mut world);
    assert_eq!(G7D_SEEN.load(SEQ), 0, "frame 2 (idle): the baseline");

    assert!(
        world.mark_component_changed(e, G7Dense::component_id()),
        "the dense arm found the slot and stamped it"
    );
    G7D_SEEN.store(0, SEQ);
    schedule.run(&mut world);
    assert_eq!(
        G7D_SEEN.load(SEQ),
        1,
        "frame 3 matches BECAUSE of the dense mark"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 8 — EnableTagId::try_from_component_id
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G8Table(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G8Dense(u32);

#[test]
fn g8_try_from_component_id_is_a_proof_not_a_cast() {
    let mut ecs = EcsMaster::new();
    let t = ecs.register_enable_tag("seam_g8");

    // (1) The MINT PROVENANCE, asserted independently. This is the half with
    // discriminating power: the internal round-trip test hand-builds an
    // `EnableTagId` over an id that was never classified Bitset, so it says
    // nothing about the classification.
    assert_eq!(
        component_registry::storage_kind(t.component_id().0),
        StorageKind::Bitset,
        "`register_enable_tag` classifies the minted id Bitset"
    );

    // (2) The positive arm. `EnableTagId` is `#[repr(transparent)]` over
    // `ComponentId`, so the only expressible `Some` is `Some(EnableTagId(id))`
    // — which is why (1) carries the content and this is the round trip.
    assert_eq!(
        EnableTagId::try_from_component_id(t.component_id()),
        Some(t),
        "a Bitset-classified id round-trips through the inbound half"
    );

    // (3) The negative arms — one per non-bitset storage kind.
    assert_eq!(
        EnableTagId::try_from_component_id(G8Table::component_id()),
        None,
        "a TABLE id is not an enable tag"
    );
    assert_eq!(
        EnableTagId::try_from_component_id(G8Dense::component_id()),
        None,
        "a DENSE id is not an enable tag"
    );

    // Totality: an out-of-range id answers `None` instead of tripping
    // `storage_kind`'s debug assertion.
    assert_eq!(
        EnableTagId::try_from_component_id(ComponentId(usize::MAX)),
        None,
        "the constructor is TOTAL — an out-of-range id is `None`, not a panic"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 9 — the F27 assertion: the by-NAME route mints a DIFFERENT tag.
//
// Documented kernel behaviour, pinned so the next person who reaches for the
// obvious substitute finds a test saying why it is wrong. `display_name` is a
// `boyko_reflect` fn and calling it here would be a dependency cycle;
// `get_layout(..).type_name` is the in-tree substitute.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[component(storage = "bitset")]
struct G9Flag;

#[test]
fn g9_registering_an_enable_tag_by_type_name_mints_a_different_id() {
    let mut ecs = EcsMaster::new();
    let id = G9Flag::component_id();
    assert_eq!(
        component_registry::storage_kind(id.0),
        StorageKind::Bitset,
        "precondition: the derived `storage = \"bitset\"` fixture is Bitset"
    );

    let type_name = component_registry::get_layout(id.0)
        .expect("the derive registered the layout")
        .type_name;
    let by_name = ecs.register_enable_tag(type_name);

    assert_ne!(
        by_name.component_id(),
        id,
        "the by-NAME route mints a NEW dynamic enable tag; it does NOT resolve \
         a derived component's id. A reflection-side `display_name` round trip \
         through `register_enable_tag` therefore addresses a DIFFERENT id than \
         the one it started from (F27)."
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 12 — the sibling REPRODUCES the retained-id guard.
//
// The only gate in this file whose subject is the guard. Gates 1–11 are all
// single-fixture and table-only or dense-routed, so `is_signature_id` skips
// ZERO ids in every one of them and a missing guard reds nothing. The kernel
// gate `crates/boyko_ecs/tests/retained_id_walk_pool_skip.rs` holds eight
// per-site positive tests over `add_tag` / `remove_tag` / `insert` / `remove`
// — no census, no loop over walks — so it cannot see the sibling either.
//
// Without the guard both legs panic in BOTH profiles at the copied
// `.expect("invariant: source hosts its own component id")`: an `Option`
// unwrap, not a `debug_assert!`.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G12Pos(u32);

#[derive(Component, Clone, Copy)]
#[component(storage = "dense")]
#[repr(C)]
struct G12Dense(u64);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G12Add(u32);

#[derive(Bundle)]
struct G12Mixed {
    p: G12Pos,
    d: G12Dense,
}

#[derive(Bundle)]
struct G12TableOnly {
    p: G12Pos,
}

#[derive(Bundle)]
struct G12Tgt {
    p: G12Pos,
    a: G12Add,
}

#[test]
fn g12_sibling_skips_a_retained_dense_id_for_a_table_only_entity() {
    let mut ecs = EcsMaster::new();
    let add_id = G12Add::component_id();

    // POPULATED TARGET first (a distinct archetype, {G12Pos, G12Add}).
    let occupant = spawn_one(&mut ecs, || G12Tgt {
        p: G12Pos(11),
        a: G12Add(OCCUPANT),
    });

    // Mint the SOURCE archetype from the MIXED spawn, so its retained id list
    // carries the dense id.
    let mixed = spawn_one(&mut ecs, || G12Mixed {
        p: G12Pos(1),
        d: G12Dense(1),
    });
    // A pure-table spawn with the same FILTERED mask dedups into it; it is an
    // innocent bystander of the archetype it was deduped into.
    let table_only = spawn_one(&mut ecs, || G12TableOnly { p: G12Pos(9) });
    let _tail = spawn_one(&mut ecs, || G12TableOnly { p: G12Pos(3) });

    assert_eq!(
        ecs.entity_archetype_id(mixed),
        ecs.entity_archetype_id(table_only),
        "precondition: dedup keys on the FILTERED mask, so both entities share \
         ONE archetype"
    );
    assert!(
        !ecs.dense_contains(table_only, G12Dense::component_id()),
        "precondition: the victim carries NO dense column"
    );

    let payload = G12Add(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    let bytes = unsafe { pod_bytes(&payload) };

    // UNGUARDED: panics inside `migrate_entity_attach_ids_with_bytes` — the
    // retained walk hits the dense id and `source.get_pool(dense)` is `None`.
    assert_eq!(
        ecs.add_component_by_id(table_only, add_id, bytes),
        AddOutcome::Attached,
        "the byte-carrying sibling must skip the pool-less retained id"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12Add>(&ecs, table_only, add_id) },
        G12Add(VICTIM),
        "the attached payload reads back"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12Pos>(&ecs, table_only, G12Pos::component_id()) },
        G12Pos(9),
        "the retained TABLE column keeps its value"
    );
    assert!(
        !ecs.dense_contains(table_only, G12Dense::component_id()),
        "the migration must not INVENT a dense membership"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12Add>(&ecs, occupant, add_id) },
        G12Add(OCCUPANT),
        "the pre-existing target occupant is untouched"
    );
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G12bPos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G12bAdd(u32);

/// A bitset enable tag: no pool, no dense store, filtered from every signature.
/// It cannot be a `Bundle` field (the derive suppresses that for
/// `storage = "bitset"`), so it enters a retained list only through
/// `create_archetype`.
#[derive(Component)]
#[component(storage = "bitset")]
struct G12bBit;

#[derive(Bundle)]
struct G12bSrc {
    p: G12bPos,
}

#[derive(Bundle)]
struct G12bTgt {
    p: G12bPos,
    a: G12bAdd,
}

#[test]
fn g12_sibling_skips_a_retained_bitset_id_for_a_table_only_entity() {
    let mut ecs = EcsMaster::new();
    let add_id = G12bAdd::component_id();

    let occupant = spawn_one(&mut ecs, || G12bTgt {
        p: G12bPos(11),
        a: G12bAdd(OCCUPANT),
    });

    // Mint the {G12bPos}-masked archetype so that it RETAINS a bitset id. The
    // defect class is `non-signature-storage`, not `dense` — which is why the
    // guard is the shared `is_signature_id` predicate and not a
    // `matches!(.., Dense)`.
    let retaining =
        ecs.create_archetype(&sorted([G12bPos::component_id(), G12bBit::component_id()]));

    let victim = spawn_one(&mut ecs, || G12bSrc { p: G12bPos(9) });
    let _tail = spawn_one(&mut ecs, || G12bSrc { p: G12bPos(3) });
    assert_eq!(
        ecs.entity_archetype_id(victim),
        Some(retaining),
        "precondition: the pure-table spawn dedups into the bitset-RETAINING \
         archetype"
    );

    let payload = G12bAdd(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    let bytes = unsafe { pod_bytes(&payload) };

    // UNGUARDED: panics at the identical site with a BITSET id.
    assert_eq!(
        ecs.add_component_by_id(victim, add_id, bytes),
        AddOutcome::Attached,
        "the sibling must skip the pool-less retained BITSET id too"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12bAdd>(&ecs, victim, add_id) },
        G12bAdd(VICTIM),
        "the attached payload reads back"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12bPos>(&ecs, victim, G12bPos::component_id()) },
        G12bPos(9),
        "the retained TABLE column keeps its value"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G12bAdd>(&ecs, occupant, add_id) },
        G12bAdd(OCCUPANT),
        "the pre-existing target occupant is untouched"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// GATE 12c — DEFERRED, RED BY DESIGN. A pre-existing KERNEL defect, not an EG2
// regression, and deliberately NOT fixed in this rung.
//
// `DenseStore::mark_arch_present` has EIGHT production call sites — it read "seven"
// until EG2-R round 4 recounted it; GATE 18's header below lists them and re-verifies
// the exhaustiveness claim. Every one fires only for a dense id IN the bundle / insert
// set applied, never for PRE-EXISTING dense memberships on archetype change. Candidates
// are seeded from `arch_presence`, so a dense member migrated by a TABLE attach stops matching.
//
// `dense_contains` still answers `true` — MEMBERSHIP survives; it is the query
// CANDIDATE SEED that is lost, which is why the membership assertion below is
// kept and is expected to PASS even while the query assertion fails.
//
// Reachable from `add_tag` and from the typed `Commands::insert` already; S1's
// table arm is the SAME code path (`migrate_entity_attach_ids_with_bytes` calls
// `mark_arch_present` nowhere), so the by-id seam adds routes to a defect it
// did not create.
//
// The two existing gate-12 legs are NOT modified: their subject is the
// `is_signature_id` retained-walk guard, and they pass. This is an added leg.
// ────────────────────────────────────────────────────────────────────────────

static G12C_SEEN: AtomicUsize = AtomicUsize::new(0);

#[test]
#[ignore = "deferred: DenseStore::arch_presence is not re-seeded when an entity that already \
            carries a dense component is migrated by a TABLE attach — a pre-existing kernel \
            defect (routes: add_tag, the typed Commands::insert, and now S1's table arm). \
            RED by design until that kernel change lands; see docs/OPEN-QUESTIONS.md. \
            NOTE: no `ignore_reasons_census.rs` exists on this branch, so this class prefix is \
            forward-compatible convention only — nothing mechanically checks it here."]
fn g12c_a_dense_carrying_victim_stays_visible_to_a_dense_query_after_a_table_attach() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();
    let add_id = G12Add::component_id();

    // TWO dense members, so the post-migration count discriminates "the victim
    // dropped out" (1) from "the query broke entirely" (0).
    let victim = spawn_one(&mut world, || G12Mixed {
        p: G12Pos(1),
        d: G12Dense(1),
    });
    let _sibling = spawn_one(&mut world, || G12Mixed {
        p: G12Pos(2),
        d: G12Dense(2),
    });

    G12C_SEEN.store(0, SEQ);
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G12Dense>| {
        for _ in &q {
            G12C_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        G12C_SEEN.load(SEQ),
        2,
        "baseline: both dense members are visible BEFORE any migration"
    );

    let payload = G12Add(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    let bytes = unsafe { pod_bytes(&payload) };
    assert_eq!(
        world.add_component_by_id(victim, add_id, bytes),
        AddOutcome::Attached,
        "the TABLE attach migrates the dense-carrying victim"
    );

    assert!(
        world.dense_contains(victim, G12Dense::component_id()),
        "MEMBERSHIP survives the migration — this half passes; the store still \
         answers true. Only the query candidate seed is lost."
    );

    G12C_SEEN.store(0, SEQ);
    schedule.run(&mut world);
    assert_eq!(
        G12C_SEEN.load(SEQ),
        2,
        "the migration must not drop the victim from the dense candidate seed"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 13 — `#[require]` must hold through the by-id seam exactly as it does
// through the typed one.
//
// Both legs are run in ONE test on purpose: the typed leg is the ORACLE. If the
// required-components machinery ever changes underneath, this gate reports
// "the two paths disagree" rather than baking a literal expectation that could
// go stale on the wrong side.
//
// The required-components chain is id-keyed end to end
// (`for_each_required_id_excluding` / `required_ctor_for` take `&[ComponentId]`;
// `RequiredCtor` is a capture-free `unsafe fn(*mut u8)`), and a purely by-id
// caller — `clone/materialize.rs`, Feature 2 — already drives it in production
// with no `Bundle` and no `C: Component` anywhere in the path. So the seam is
// able to serve this correctly; before FORK A it simply did not ask.
//
// Ids are derive-minted (this file's convention), so there is no fixed-id block
// to collide with gate 10's `[328, 340)`.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G13Seed(u32);

/// The REQUIRED component. Its `Default` is a sentinel, not zero: a fresh pool
/// row is lazily-committed OS memory, so a zero default would make "the ctor
/// ran" indistinguishable from "the row was never constructed" (§B.0).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G13Dep(u32);
impl Default for G13Dep {
    fn default() -> Self {
        G13Dep(0x0D0D_0D0D)
    }
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[require(G13Dep)]
#[repr(C)]
struct G13Needs(u32);

#[derive(Bundle)]
struct G13SeedBundle {
    s: G13Seed,
}

#[derive(Bundle)]
struct G13NeedsBundle {
    n: G13Needs,
}

static G13_DEP_ON_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn g13_dep_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G13_DEP_ON_ADD.fetch_add(1, SEQ);
}

#[test]
fn g13_require_holds_through_the_by_id_seam_exactly_as_through_the_typed_one() {
    // Contract order (H1): hooks BEFORE the first entity carrying the id. The
    // `Result` is `.expect`ed, never discarded (file header).
    component_registry::register_hooks_by_id(
        G13Dep::component_id(),
        ComponentHooks {
            on_add: Some(g13_dep_on_add),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let needs_id = G13Needs::component_id();
    let dep_id = G13Dep::component_id();

    let typed = spawn_one(&mut ecs, || G13SeedBundle { s: G13Seed(1) });
    let by_id = spawn_one(&mut ecs, || G13SeedBundle { s: G13Seed(2) });
    // §B.0 populated source: the victims are not in the last row, so
    // `move_out_entity` returns `Swapped` and the inland repoint executes.
    let tail = spawn_one(&mut ecs, || G13SeedBundle { s: G13Seed(3) });

    // ── Leg 1: the TYPED path, the oracle. ──────────────────────────────────
    G13_DEP_ON_ADD.store(0, SEQ);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(typed).insert(G13NeedsBundle {
            n: G13Needs(VICTIM),
        });
    });
    assert!(
        ecs.has_component(typed, needs_id),
        "oracle: the typed insert attached the requiring component"
    );
    assert!(
        ecs.has_component(typed, dep_id),
        "oracle: the typed insert expanded `#[require(G13Dep)]`"
    );
    assert_eq!(
        ecs.get_component::<G13Dep>(typed).copied(),
        Some(G13Dep::default()),
        "oracle: the required component holds its ctor's value"
    );
    assert_eq!(
        G13_DEP_ON_ADD.load(SEQ),
        1,
        "oracle: on_add fires for the CONSTRUCTED required id too"
    );

    // ── Leg 2: the BY-ID seam. Same claims, verbatim. ───────────────────────
    let payload = G13Needs(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `needs_id`.
    let bytes = unsafe { pod_bytes(&payload) };
    G13_DEP_ON_ADD.store(0, SEQ);
    assert_eq!(
        ecs.add_component_by_id(by_id, needs_id, bytes),
        AddOutcome::Attached,
        "the by-id attach lands"
    );
    assert!(
        ecs.has_component(by_id, needs_id),
        "the by-id attach attached the requiring component"
    );
    assert!(
        ecs.has_component(by_id, dep_id),
        "#[require] must hold through the by-id seam exactly as it does through \
         the typed one"
    );
    assert_eq!(
        ecs.get_component::<G13Dep>(by_id).copied(),
        Some(G13Dep::default()),
        "the by-id-constructed required component holds its ctor's value — \
         presence alone would pass over an uninitialized row"
    );
    assert_eq!(
        G13_DEP_ON_ADD.load(SEQ),
        1,
        "C1: on_add fires for the id the seam CONSTRUCTED, not only for the id \
         the caller named (the helper's Phase-2 loop iterates `added`, and the \
         constructed ids are IN `added`)"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G13Needs>(&ecs, by_id, needs_id) },
        G13Needs(VICTIM),
        "the caller's own payload is not disturbed by the expansion"
    );

    // Bystander, on both axes (§B.0).
    assert_eq!(
        // SAFETY: registered type / hosted id.
        unsafe { read_back::<G13Seed>(&ecs, tail, G13Seed::component_id()) },
        G13Seed(3),
        "BYTES: the swapped source bystander keeps its value"
    );
    assert!(
        row_is_inside_the_pool(&ecs, tail, G13Seed::component_id()),
        "ROW: the swapped source bystander was repointed inside the pool"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 14 — the bytes are MOVED into the pool, not borrowed from the caller.
//
// `add_component_by_id` memcpy's `bytes` into the row and the WORLD drops that
// row later (`drop_fn` on despawn / world drop). A caller that keeps its own
// live value and passes a byte view of it therefore DOUBLE-DROPS — and for a
// heap-owning payload that is a double free — through a `pub fn` carrying no
// `unsafe`.
//
// Every other gate in this file uses a `Copy` payload, which is exactly why
// nothing here could see this: a `Copy` type has no drop to double.
//
// **What this gate CANNOT pin, stated so nobody re-reads it as pinned.** The
// "a caller that keeps its own live value double-drops" claim is a contract on
// caller code. In-process it is a genuine double free — a process abort, not a
// test failure — and a bitwise-moved `Box` is indistinguishable from a
// bitwise-copied one, so no read-back can tell "moved" from "copied" either.
// The `# Ownership` doc block on `add_component_by_id` states the contract; the
// `ManuallyDrop` below DEMONSTRATES the correct usage. This gate pins the two
// things that ARE in reach: the engine does not drop what it was just given,
// and the world drops it exactly once at teardown — not zero (leak), not two.
//
// This is the ONLY fixture in the file with a `Drop` payload, so it is the only
// gate that reds on a DROP COUNT rather than on a byte read-back. Its red
// mutation (deleting the added column's `commit_units` in
// `migrate_entity_attach_ids_with_bytes`'s shared tail) also reds g1 and g13 —
// but on byte / presence reads, never on the ledger. That is the coverage
// nothing else here provides. **Run it in RELEASE**: in debug the lockstep
// `debug_assert!` fires first and you get a panic instead of the ledger.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G14Pos(u32);

/// A HEAP-OWNING payload — `Box`, not a bare counter — so that a double drop is
/// a double free and not merely a wrong tally.
#[derive(Component)]
#[repr(C)]
struct G14Owned {
    tag: u32,
    owned: Box<u32>,
}

static G14_DROPS: AtomicUsize = AtomicUsize::new(0);

impl Drop for G14Owned {
    fn drop(&mut self) {
        G14_DROPS.fetch_add(1, SEQ);
    }
}

#[derive(Bundle)]
struct G14Src {
    p: G14Pos,
}

#[test]
fn g14_the_world_drops_an_attached_owning_payload_exactly_once() {
    let mut ecs = EcsMaster::new();
    let owned_id = G14Owned::component_id();

    let victim = spawn_one(&mut ecs, || G14Src { p: G14Pos(1) });
    // §B.0 populated source: the victim is not in the last row.
    let tail = spawn_one(&mut ecs, || G14Src { p: G14Pos(3) });

    G14_DROPS.store(0, SEQ);
    {
        // `ManuallyDrop` is the transfer: the bytes are memcpy'd into the pool,
        // which takes ownership. Dropping this local too would count a drop the
        // engine never performed — and free the `Box` twice.
        let d = std::mem::ManuallyDrop::new(G14Owned {
            tag: VICTIM,
            owned: Box::new(0xBEEF_u32),
        });
        // SAFETY: `G14Owned` is `#[repr(C)]` and this is its registered id, so
        //   the span is a valid representation for that column; the slice
        //   borrows `d` and is consumed by the call below, before `d` goes out
        //   of scope.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&*d as *const G14Owned).cast::<u8>(),
                std::mem::size_of::<G14Owned>(),
            )
        };
        assert_eq!(
            ecs.add_component_by_id(victim, owned_id, bytes),
            AddOutcome::Attached,
            "the owning payload attaches"
        );
    }

    assert_eq!(
        G14_DROPS.load(SEQ),
        0,
        "nothing has dropped yet. A non-zero count here means the ENGINE ran a \
         destructor on the row it had just been given — the attach path took \
         `migrate_entity_insert`'s OVERLAP branch (`drop_at` before `write_at`) \
         on a column that is newly added, where nothing owns the slot yet. \
         (It cannot mean the caller's copy died: `d` is `ManuallyDrop`.)"
    );
    assert_eq!(
        ecs.get_component::<G14Owned>(victim).map(|v| v.tag),
        Some(VICTIM),
        "the moved bytes are readable through the world"
    );
    assert_eq!(
        ecs.get_component::<G14Owned>(victim).map(|v| *v.owned),
        Some(0xBEEF_u32),
        "the OWNED heap cell moved with them — this is the half a `Copy` \
         fixture cannot express"
    );
    // ROW axis for the swapped source bystander (§B.0).
    assert!(
        row_is_inside_the_pool(&ecs, tail, G14Pos::component_id()),
        "ROW: the swapped source bystander was repointed inside the pool"
    );

    drop(ecs);
    assert_eq!(
        G14_DROPS.load(SEQ),
        1,
        "exactly one drop, performed by the world at teardown — not zero (leak) \
         and not two (double free)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 15 — `remove_component_by_id`'s DENSE arm must DRAIN before returning.
//
// The table arm drains after its structural op; the dense arm went straight to
// `dense_remove_and_fire` with no drain at all. A hook that enqueues through
// `DeferredEcsMaster` therefore left its command STRANDED.
//
// **Both halves are asserted and the second is NOT redundant.** The command is
// not lost — `drain_deferred_hook_queue` no-ops only at depth >= 1, and here the
// depth is 0 with no drain call at all — so it survives in the queue and is
// applied by whichever self-draining direct API runs next. The defect is
// therefore MIS-ATTRIBUTION IN TIME, and only the second assertion can see it:
// an unrelated later call appears to create two entities. Gate 6's dense half
// asserts fire counts, membership and the pre-tombstone value, none of which
// can observe when the hook's own command lands.
//
// The bracket half of the old name was not pinned by this gate and could not
// be: deleting `DeferredScopeGuard::enter()` / `drop(scope)` from this arm
// leaves the gate GREEN — MEASURED, this session. The bracket is pinned by
// `g15b`, a `#[cfg(test)] mod` inside `ecs_master/seam_by_id.rs` (the depth it
// bumps is `pub(crate)`, so an integration test cannot read it).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G15Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G15Dense(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G15Spawned(u32);

#[derive(Bundle)]
struct G15Src {
    p: G15Pos,
}

#[derive(Bundle)]
struct G15SpawnedBundle {
    s: G15Spawned,
}

static G15_ON_REMOVE: AtomicUsize = AtomicUsize::new(0);

/// Enqueues a DEFERRED spawn from inside the dense `on_remove` fire. WHEN that
/// command lands is this gate's entire subject.
unsafe fn g15_on_remove(mut w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G15_ON_REMOVE.fetch_add(1, SEQ);
    w.commands().spawn(G15SpawnedBundle {
        s: G15Spawned(VICTIM),
    });
}

#[test]
fn g15_dense_remove_drains_its_hook_queue_before_returning() {
    component_registry::register_hooks_by_id(
        G15Dense::component_id(),
        ComponentHooks {
            on_remove: Some(g15_on_remove),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let dense_id = G15Dense::component_id();
    let e = spawn_one(&mut ecs, || G15Src { p: G15Pos(1) });

    let payload = G15Dense(VICTIM);
    assert_eq!(
        // SAFETY: registered `#[repr(C)]` POD type for `dense_id`.
        ecs.add_component_by_id(e, dense_id, unsafe { pod_bytes(&payload) }),
        AddOutcome::Attached,
        "precondition: the dense membership is established"
    );
    // Reset immediately before the call: only the DELTA across the detach is
    // this gate's subject.
    G15_ON_REMOVE.store(0, SEQ);

    let before = ecs.entity_count();
    assert!(
        ecs.remove_component_by_id(e, dense_id),
        "a present dense member detaches and reports true"
    );
    assert_eq!(
        G15_ON_REMOVE.load(SEQ),
        1,
        "the dense on_remove hook fired exactly once"
    );
    assert_eq!(
        ecs.entity_count(),
        before + 1,
        "the hook's deferred command must be APPLIED by the end of THIS call — \
         the dense arm owns a drain exactly as the table arm does"
    );

    // The mis-attribution half — load-bearing, never drop it as redundant.
    let before2 = ecs.entity_count();
    let _unrelated = ecs.spawn_empty();
    assert_eq!(
        ecs.entity_count(),
        before2 + 1,
        "an unrelated later op must not carry a STRANDED command from the \
         earlier dense remove"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 13b — the DENSE leg of gate 13 (EG2-R FORK A).
//
// A separate `#[test]` rather than a third leg inside the table gate, because it
// carries a fire-ORDER ledger and a mid-fire VISIBILITY probe the table leg has
// no use for. The rung's own gate list already uses "BOTH storage kinds" as an
// explicit discipline for gates 3 and 6; gate 13 was written table-only while
// its title claims the general property.
//
// The typed leg is again the ORACLE, and it is the oracle for FOUR properties,
// not one: state, fire order, mid-fire visibility, and (via gate 3) the
// `AlreadyPresent` refusal. The third is the one an "expand, then attach, then
// dense-insert" implementation gets wrong while passing the first two —
// MEASURED on the typed path, which writes the dense store inside
// `migrate_entity_insert`'s Phase-1 closure, BEFORE any hook fires, and fires
// the dense hooks in its POST block, AFTER the required-table ones.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G13bSeed(u32);

/// The REQUIRED component — TABLE-stored, so the dense caller id's expansion
/// forces a real archetype migration. Its `Default` is a sentinel, not zero
/// (§B.0): a fresh pool row is lazily-committed OS memory.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G13bDep(u32);
impl Default for G13bDep {
    fn default() -> Self {
        G13bDep(0x0D0D_0D0D)
    }
}

/// The caller's own id: DENSE, and carrying a TABLE `#[require]`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[require(G13bDep)]
#[repr(C)]
struct G13bDenseNeeds(u32);

#[derive(Bundle)]
struct G13bSeedBundle {
    s: G13bSeed,
}

/// A dense component suppresses the derive's single-component `Bundle` impl, so
/// the typed leg needs this explicit wrapper.
#[derive(Bundle)]
struct G13bNeedsBundle {
    n: G13bDenseNeeds,
}

/// Leg 3's source: the required component is ALREADY present, so the dense
/// caller's whole required closure collapses to nothing.
#[derive(Bundle)]
struct G13bSeedDepBundle {
    s: G13bSeed,
    d: G13bDep,
}

/// Monotonic stamp: each fire takes the next number, so ORDER is readable from
/// two `usize`s rather than from a shared log.
static G13B_SEQ: AtomicUsize = AtomicUsize::new(0);
static G13B_DEP_AT: AtomicUsize = AtomicUsize::new(0);
static G13B_DENSE_AT: AtomicUsize = AtomicUsize::new(0);
static G13B_DEP_SAW_DENSE: AtomicUsize = AtomicUsize::new(0);

unsafe fn g13b_dep_on_add(w: DeferredEcsMaster<'_>, ctx: HookContext) {
    G13B_DEP_AT.store(G13B_SEQ.fetch_add(1, SEQ) + 1, SEQ);
    // `get_component_raw` HAS a dense arm, so this read is a real probe of the
    // dense store, not a table lookup that would answer `None` regardless.
    if w.get_component::<G13bDenseNeeds>(ctx.entity).is_some() {
        G13B_DEP_SAW_DENSE.store(1, SEQ);
    }
}

unsafe fn g13b_dense_on_add(_w: DeferredEcsMaster<'_>, _ctx: HookContext) {
    G13B_DENSE_AT.store(G13B_SEQ.fetch_add(1, SEQ) + 1, SEQ);
}

/// Resets the four ledger cells. Called immediately before each leg, so only
/// the DELTA across that leg's one structural op is observed.
fn g13b_reset() {
    G13B_SEQ.store(0, SEQ);
    G13B_DEP_AT.store(0, SEQ);
    G13B_DENSE_AT.store(0, SEQ);
    G13B_DEP_SAW_DENSE.store(0, SEQ);
}

#[test]
fn g13b_require_holds_through_the_by_id_seam_for_a_dense_caller_id() {
    // Contract order (H1): hooks BEFORE the first entity carrying the id; the
    // `Result` is `.expect`ed, never discarded (file header).
    component_registry::register_hooks_by_id(
        G13bDep::component_id(),
        ComponentHooks {
            on_add: Some(g13b_dep_on_add),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");
    component_registry::register_hooks_by_id(
        G13bDenseNeeds::component_id(),
        ComponentHooks {
            on_add: Some(g13b_dense_on_add),
            ..Default::default()
        },
    )
    .expect("fresh id, never archetyped");

    let mut ecs = EcsMaster::new();
    let needs_id = G13bDenseNeeds::component_id();

    // §B.0 populated source: FOUR entities, so that after the typed leg has
    // migrated `typed` out (swapping `tail_b` into its row) the by-id victim is
    // STILL not in the last row and its own migration is `Swapped` too.
    let typed = spawn_one(&mut ecs, || G13bSeedBundle { s: G13bSeed(1) });
    let by_id = spawn_one(&mut ecs, || G13bSeedBundle { s: G13bSeed(2) });
    let tail_a = spawn_one(&mut ecs, || G13bSeedBundle { s: G13bSeed(3) });
    let tail_b = spawn_one(&mut ecs, || G13bSeedBundle { s: G13bSeed(4) });

    // ── Leg 1: the TYPED path, the ORACLE. Record all four properties. ──────
    g13b_reset();
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(typed).insert(G13bNeedsBundle {
            n: G13bDenseNeeds(VICTIM),
        });
    });
    let o_dense = ecs.dense_contains(typed, needs_id);
    let o_dep = ecs.get_component::<G13bDep>(typed).copied();
    let o_dep_at = G13B_DEP_AT.load(SEQ);
    let o_dense_at = G13B_DENSE_AT.load(SEQ);
    let o_saw = G13B_DEP_SAW_DENSE.load(SEQ);

    // The oracle must itself be non-vacuous — a run in which NEITHER hook fired
    // would make every comparison below trivially equal.
    assert!(
        o_dep_at > 0 && o_dense_at > 0,
        "oracle: both on_add hooks must fire (dep_at={o_dep_at}, dense_at={o_dense_at})"
    );
    assert!(o_dense, "oracle: the dense membership landed");
    assert_eq!(
        o_dep,
        Some(G13bDep::default()),
        "oracle: the required component's ctor ran"
    );

    // ── Leg 2: the BY-ID seam. Same four claims, verbatim. ──────────────────
    g13b_reset();
    let payload = G13bDenseNeeds(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `needs_id`.
    let bytes = unsafe { pod_bytes(&payload) };
    assert_eq!(
        ecs.add_component_by_id(by_id, needs_id, bytes),
        AddOutcome::Attached,
        "the by-id attach of a dense caller id lands"
    );

    assert_eq!(
        ecs.dense_contains(by_id, needs_id),
        o_dense,
        "STATE: the dense membership lands identically"
    );
    assert_eq!(
        ecs.get_component::<G13bDep>(by_id).copied(),
        o_dep,
        "STATE: #[require] must hold through the by-id seam for a DENSE caller \
         id exactly as it does through the typed one — the seam routed DENSE to \
         dense_insert_and_fire and returned BEFORE the expansion block, so the \
         whole required closure was silently dropped"
    );
    assert_eq!(
        G13B_DEP_AT.load(SEQ) < G13B_DENSE_AT.load(SEQ),
        o_dep_at < o_dense_at,
        "ORDER: the required TABLE component's on_add fires before the dense \
         component's on_add, as it does on the typed path (by-id dep_at={}, \
         dense_at={})",
        G13B_DEP_AT.load(SEQ),
        G13B_DENSE_AT.load(SEQ)
    );
    assert_eq!(
        G13B_DEP_SAW_DENSE.load(SEQ),
        o_saw,
        "VISIBILITY: the required component's on_add sees the dense value \
         already present. The typed path writes the dense store in Phase 1, \
         BEFORE any hook fires; an implementation that attaches the required \
         columns first and dense-inserts afterwards passes STATE and ORDER and \
         fails HERE"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id (the dense arm of the raw reader).
        unsafe { read_back::<G13bDenseNeeds>(&ecs, by_id, needs_id) },
        G13bDenseNeeds(VICTIM),
        "the caller's own payload is not disturbed by the expansion"
    );

    // ── Leg 3: the EMPTY-CLOSURE branch of the same by-id path. ────────────
    //
    // A DENSE caller whose entire required closure is ALREADY present has no
    // table work at all: no attach set, no target archetype, no migration. That
    // is a distinct branch of `add_by_id_expanding_requires` (`n == 0`), and
    // reaching `merged_archetype_id_dyn` / `migrate_entity_attach_ids_with_bytes`
    // with an empty set instead is a debug-assert away from a panic and a
    // release build away from a phantom migration. The typed twin collapses to
    // `InsertCommand::apply_replace_in_place`, whose dense branch marks the
    // SOURCE archetype present — so the fused helper on the SOURCE is the
    // parity target here, not the split.
    let pre = spawn_one(&mut ecs, || G13bSeedDepBundle {
        s: G13bSeed(5),
        d: G13bDep(0xFEED_FACE),
    });
    g13b_reset();
    let payload3 = G13bDenseNeeds(SECOND);
    // SAFETY: registered `#[repr(C)]` POD type for `needs_id`.
    let bytes3 = unsafe { pod_bytes(&payload3) };
    assert_eq!(
        ecs.add_component_by_id(pre, needs_id, bytes3),
        AddOutcome::Attached,
        "an empty required closure is still an attach"
    );
    assert!(
        ecs.dense_contains(pre, needs_id),
        "the dense membership lands with no migration at all"
    );
    assert_eq!(
        ecs.get_component::<G13bDep>(pre).copied(),
        Some(G13bDep(0xFEED_FACE)),
        "PRESENT ⇒ SKIP: the existing required value WINS and is not \
         re-constructed to its ctor's default"
    );
    assert_eq!(
        G13B_DEP_AT.load(SEQ),
        0,
        "PRESENT ⇒ SKIP fires nothing for the already-present required id"
    );
    assert!(
        G13B_DENSE_AT.load(SEQ) > 0,
        "the dense on_add still fires on the no-migration branch"
    );

    // Bystanders, on BOTH axes (§B.0). Both tails stayed in the source
    // archetype across two migrations out of it.
    for (who, want) in [(tail_a, 3u32), (tail_b, 4u32)] {
        assert_eq!(
            // SAFETY: registered type / hosted id.
            unsafe { read_back::<G13bSeed>(&ecs, who, G13bSeed::component_id()) },
            G13bSeed(want),
            "BYTES: a source bystander keeps its value across the migrations"
        );
        assert!(
            row_is_inside_the_pool(&ecs, who, G13bSeed::component_id()),
            "ROW: a swapped source bystander was repointed inside the pool"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 16 — entity-targeted `on_add` / `on_insert` observers must fire through
// the by-id seam exactly as through the typed one (EG2-R FORK B, attach half).
//
// `migrate_entity_attach_ids_with_bytes` — this rung's own `pub(crate)` D9
// sibling, with no other caller — carried NEITHER
// `world.migrate_entity_observer_bit(entity)` nor any `fire_entity_observers`,
// while its typed twin `migrate_entity_insert` carries both.
//
// **The BY-ID leg runs FIRST, and that is load-bearing.** The
// `HAS_ENTITY_OBSERVER` archetype bit is STICKY: once the typed leg has migrated
// its observed entity into the target archetype, the bit is raised there for
// good, and a by-id leg run afterwards would exercise only the
// `fire_entity_observers` half. Running by-id first leaves the destination bit
// clear, so the gate covers BOTH insertions — the observer-bit hoist and the
// fires.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G16Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G16Add(u32);

#[derive(Bundle)]
struct G16Src {
    p: G16Pos,
}

#[derive(Bundle)]
struct G16Tgt {
    p: G16Pos,
    a: G16Add,
}

#[derive(Bundle)]
struct G16AddBundle {
    a: G16Add,
}

static G16_TYPED_ADD: AtomicUsize = AtomicUsize::new(0);
static G16_TYPED_INSERT: AtomicUsize = AtomicUsize::new(0);
static G16_BYID_ADD: AtomicUsize = AtomicUsize::new(0);
static G16_BYID_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn g16_typed_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G16_TYPED_ADD.fetch_add(1, SEQ);
}
unsafe fn g16_typed_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G16_TYPED_INSERT.fetch_add(1, SEQ);
}
unsafe fn g16_byid_add(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G16_BYID_ADD.fetch_add(1, SEQ);
}
unsafe fn g16_byid_insert(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G16_BYID_INSERT.fetch_add(1, SEQ);
}

#[test]
fn g16_entity_targeted_add_observers_fire_through_the_by_id_seam_as_through_the_typed_one() {
    let mut ecs = EcsMaster::new();
    let add_id = G16Add::component_id();

    // POPULATED TARGET (§B.0): one unobserved entity already lives in
    // {G16Pos, G16Add}, so the destination archetype exists with its
    // HAS_ENTITY_OBSERVER bit CLEAR.
    let _occupant = spawn_one(&mut ecs, || G16Tgt {
        p: G16Pos(11),
        a: G16Add(OCCUPANT),
    });
    // POPULATED SOURCE: neither victim is in the last row.
    let by_id = spawn_one(&mut ecs, || G16Src { p: G16Pos(1) });
    let typed = spawn_one(&mut ecs, || G16Src { p: G16Pos(2) });
    let tail_a = spawn_one(&mut ecs, || G16Src { p: G16Pos(3) });
    let tail_b = spawn_one(&mut ecs, || G16Src { p: G16Pos(4) });

    // Entity-targeted observers on BOTH victims, same kinds, same component id.
    ecs.observe_entity(by_id, ObserverKind::Add, add_id, g16_byid_add);
    ecs.observe_entity(by_id, ObserverKind::Insert, add_id, g16_byid_insert);
    ecs.observe_entity(typed, ObserverKind::Add, add_id, g16_typed_add);
    ecs.observe_entity(typed, ObserverKind::Insert, add_id, g16_typed_insert);

    G16_BYID_ADD.store(0, SEQ);
    G16_BYID_INSERT.store(0, SEQ);
    G16_TYPED_ADD.store(0, SEQ);
    G16_TYPED_INSERT.store(0, SEQ);

    // ── Leg 1: the BY-ID seam (first, see the header). ──────────────────────
    let payload = G16Add(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `add_id`.
    let bytes = unsafe { pod_bytes(&payload) };
    assert_eq!(
        ecs.add_component_by_id(by_id, add_id, bytes),
        AddOutcome::Attached,
        "the by-id attach lands"
    );

    // ── Leg 2: the TYPED path, the ORACLE. ──────────────────────────────────
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(typed).insert(G16AddBundle { a: G16Add(VICTIM) });
    });

    // The oracle must be non-vacuous FIRST: a leg that fired nothing would make
    // the equality below trivially true.
    assert_eq!(
        G16_TYPED_ADD.load(SEQ),
        1,
        "oracle: the typed insert fires the entity-targeted on_add observer once"
    );
    assert_eq!(
        G16_TYPED_INSERT.load(SEQ),
        1,
        "oracle: the typed insert fires the entity-targeted on_insert observer once"
    );

    assert_eq!(
        G16_BYID_ADD.load(SEQ),
        G16_TYPED_ADD.load(SEQ),
        "the by-id seam must fire the entity-targeted on_add observer exactly as \
         the typed path does — `migrate_entity_attach_ids_with_bytes` carried \
         neither the `migrate_entity_observer_bit` hoist nor any \
         `fire_entity_observers` call"
    );
    assert_eq!(
        G16_BYID_INSERT.load(SEQ),
        G16_TYPED_INSERT.load(SEQ),
        "the by-id seam must fire the entity-targeted on_insert observer exactly \
         as the typed path does"
    );

    // Bystanders, on both axes (§B.0).
    for (who, want) in [(tail_a, 3u32), (tail_b, 4u32)] {
        assert_eq!(
            // SAFETY: registered type / hosted id.
            unsafe { read_back::<G16Pos>(&ecs, who, G16Pos::component_id()) },
            G16Pos(want),
            "BYTES: a source bystander keeps its value across the migrations"
        );
        assert!(
            row_is_inside_the_pool(&ecs, who, G16Pos::component_id()),
            "ROW: a swapped source bystander was repointed inside the pool"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 17 — the DEFERRED half of FORK B: the remove direction.
//
// `migrate_entity_detach_ids` contains neither `fire_entity_observers` nor
// `migrate_entity_observer_bit`, while its typed twin `migrate_entity_remove`
// carries both. Unlike the attach helper, this one is PRE-EXISTING and SHARED
// with `remove_tag` (`tag_api.rs:234`), so repairing it changes `remove_tag`
// too — a kernel change on the kernel's own schedule, outside this rung's
// approved diff.
//
// RED by design until that change lands. Same shape as g16, in the
// Replace/Remove direction.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G17Pos(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G17Gone(u32);

#[derive(Bundle)]
struct G17Src {
    p: G17Pos,
    g: G17Gone,
}

static G17_TYPED_REPLACE: AtomicUsize = AtomicUsize::new(0);
static G17_TYPED_REMOVE: AtomicUsize = AtomicUsize::new(0);
static G17_BYID_REPLACE: AtomicUsize = AtomicUsize::new(0);
static G17_BYID_REMOVE: AtomicUsize = AtomicUsize::new(0);

unsafe fn g17_typed_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G17_TYPED_REPLACE.fetch_add(1, SEQ);
}
unsafe fn g17_typed_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G17_TYPED_REMOVE.fetch_add(1, SEQ);
}
unsafe fn g17_byid_replace(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G17_BYID_REPLACE.fetch_add(1, SEQ);
}
unsafe fn g17_byid_remove(_w: DeferredEcsMaster<'_>, _c: ObserverContext) {
    G17_BYID_REMOVE.fetch_add(1, SEQ);
}

#[test]
#[ignore = "deferred: migrate_entity_detach_ids fires no entity-targeted observers and never \
            calls migrate_entity_observer_bit — a pre-existing kernel gap SHARED with \
            remove_tag (tag_api.rs:234), so repairing it changes remove_tag too and belongs \
            to its own kernel change, not to this rung. The add_tag half \
            (migrate_entity_attach_ids) has the same gap and lands with it. RED by design \
            until that change lands; see docs/OPEN-QUESTIONS.md. NOTE: no \
            `ignore_reasons_census.rs` exists on this branch, so this class prefix is \
            forward-compatible convention only — nothing mechanically checks it here."]
fn g17_entity_targeted_remove_observers_fire_through_the_by_id_seam_as_through_the_typed_one() {
    let mut ecs = EcsMaster::new();
    let gone_id = G17Gone::component_id();

    let by_id = spawn_one(&mut ecs, || G17Src {
        p: G17Pos(1),
        g: G17Gone(VICTIM),
    });
    let typed = spawn_one(&mut ecs, || G17Src {
        p: G17Pos(2),
        g: G17Gone(VICTIM),
    });
    let _tail = spawn_one(&mut ecs, || G17Src {
        p: G17Pos(3),
        g: G17Gone(VICTIM),
    });

    ecs.observe_entity(by_id, ObserverKind::Replace, gone_id, g17_byid_replace);
    ecs.observe_entity(by_id, ObserverKind::Remove, gone_id, g17_byid_remove);
    ecs.observe_entity(typed, ObserverKind::Replace, gone_id, g17_typed_replace);
    ecs.observe_entity(typed, ObserverKind::Remove, gone_id, g17_typed_remove);

    G17_BYID_REPLACE.store(0, SEQ);
    G17_BYID_REMOVE.store(0, SEQ);
    G17_TYPED_REPLACE.store(0, SEQ);
    G17_TYPED_REMOVE.store(0, SEQ);

    assert!(
        ecs.remove_component_by_id(by_id, gone_id),
        "the by-id detach reports true — and STILL returns true while firing \
         nothing, which is why a return-value gate cannot see this"
    );
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(typed).remove::<G17Gone>();
    });

    assert_eq!(
        G17_TYPED_REMOVE.load(SEQ),
        1,
        "oracle: the typed remove fires the entity-targeted on_remove observer once"
    );
    assert_eq!(
        G17_TYPED_REPLACE.load(SEQ),
        1,
        "oracle: the typed remove fires the entity-targeted on_replace observer once"
    );
    assert_eq!(
        G17_BYID_REMOVE.load(SEQ),
        G17_TYPED_REMOVE.load(SEQ),
        "the by-id detach must fire the entity-targeted on_remove observer \
         exactly as the typed path does"
    );
    assert_eq!(
        G17_BYID_REPLACE.load(SEQ),
        G17_TYPED_REPLACE.load(SEQ),
        "the by-id detach must fire the entity-targeted on_replace observer \
         exactly as the typed path does"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 18 — the dense caller's presence bit must be seeded on the **TARGET**
// archetype, not the source.
//
// `seam_by_id.rs`'s `add_by_id_expanding_requires` calls
// `self.dense_insert_only(entity, target_archetype_id, id, bytes)` BEFORE the
// migration. `dense_insert_only` forwards its archetype argument straight to
// `DenseStore::mark_arch_present` (`component_api.rs:132`), and query candidates
// for a dense id are seeded from `arch_presence` — so that one argument decides
// whether the entity is ITERABLE after the attach.
//
// # Why this needed its own gate: `dense_contains` is BLIND to it
//
// Substituting `source_archetype_id` for `target_archetype_id` is a ONE-TOKEN
// change that leaves the ENTIRE pre-existing suite green — MEASURED: 170 result
// lines, 1881 passed, 0 failed, byte-identical to the unmutated tally, all 22
// seam gates included.
//
// It survives because membership and iterability are DIFFERENT data. The dense
// store's per-entity membership map is written either way, so
// `dense_contains(entity, id)` answers `true` under the mutation — exactly the
// trap gate 12c's header already names: *"MEMBERSHIP survives; it is the query
// CANDIDATE SEED that is lost."* Gate 13b — the sibling gate that drives THIS
// path, a dense caller with a table `#[require]` — asserts state with
// `ecs.dense_contains(by_id, needs_id)` (`:1926`) and therefore cannot see it.
// That assertion is not wrong; it is simply on the other axis. This gate adds
// the missing one, and it is the ONLY assertion in the file that reads dense
// presence through a QUERY on a migrated entity.
//
// # The EIGHT `mark_arch_present` call sites — the SYMBOL, NOT the class (18c)
//
// Gate 12c's header claimed SEVEN. `grep -rn "mark_arch_present" crates/` returns
// eight production call sites across seven files, plus the definition at
// `dense/dense_store.rs:371` (not a call site) and some prose mentions:
//
//   1. `clone/materialize.rs:894`             `mark_arch_present(target_archetype_id)`
//   2. `commands/insert_command.rs:260`       `mark_arch_present(source_archetype_id_for_dense)`
//   3. `commands/migration_helpers.rs:623`    `mark_arch_present(target_archetype_id)`  ← gate 18b
//   4. `commands/spawn_at_command.rs:257`     `mark_arch_present(archetype_id)`
//   5. `commands/spawn_batch_command.rs:593`  `mark_arch_present(archetype_id)`
//   6. `ecs_master/component_api.rs:132`      `mark_arch_present(archetype_id)`
//   7. `serialize/load_writer.rs:716`         `mark_arch_present(arch)`
//   8. `serialize/load_writer.rs:855`         `mark_arch_present(arch)`
//
// The COUNT is not what gate 12c's deferral rests on — the EXHAUSTIVENESS claim
// is, so it was re-verified site by site rather than merely recounted:
//
// * 1 iterates `cloned`, the snapshot materialized onto the clone — not a
//   migration path. ⚠️ Its wrong-id mutation IS writable; gate 19 now gates it.
// * 2, 3 and 5 sit inside a `for_each_component_bytes` / `for_each_data_component_bytes`
//   closure over the BUNDLE, in the `StorageKind::Dense` arm. Bundle ids only.
// * 4 is a spawn, keyed off `dense_mask & (1 << canonical_idx)` — canonical bundle
//   slots of an entity that has no prior memberships.
// * 6 is `dense_insert_only`, which FORWARDS an archetype id it does not choose.
// * 7 and 8 iterate the members being restored into a store the caller has just
//   `debug_assert!`ed is empty (fresh-world load).
//
// So the claim HOLDS at all eight, OVER THIS SYMBOL: none of them iterates an
// entity's PRE-EXISTING dense memberships. ⚠️ EXPRESSIBILITY is a DIFFERENT axis
// and runs 5 / 3, not 1 / 7: writable at 1 (gate 19), 3 (gate 18b), 6 (THIS
// gate; its caller passes the TARGET pre-migration) and — UNGATED — 7 and 8,
// where a SIBLING's archetype is one index away (2 is equal: `insert_command.rs:113`).
//
// # Why the by-id leg runs FIRST (load-bearing, same reason as gate 16)
//
// The typed path marks the TARGET present. Once a typed insert has moved its
// entity into `{G18Seed, G18Dep}`, the bit is raised there for good and the
// by-id victim — sitting in that same archetype — becomes visible again through
// no act of its own. A by-id leg run after the typed one would measure the typed
// path's repair, not the by-id path's seed. Hence: by-id, measure, THEN typed.
//
// # Fixture (§B.0)
//
// * **populated source** — four entities in `{G18Seed}` with `by_id` at row 0,
//   so `move_out_entity` returns `Swapped` and the inland repoint executes;
// * **populated target** — `occupant` already lives in `{G18Seed, G18Dep}`, so
//   the destination archetype EXISTS with its presence bit CLEAR before the
//   attach. That is the adversarial case: a freshly minted target archetype
//   would leave "the bit was never set for this archetype" ambiguous between
//   the two arguments;
// * **a warm-up member in a THIRD archetype** — `warm` carries `G18Needs` in
//   `{G18Other, G18Dep}`, seeded through the typed path's no-migration branch
//   (its required closure is empty, so `apply_replace_in_place` marks its own
//   SOURCE archetype, which is neither the victim's source nor its target).
//   Without it the pre-attach expectation is zero, and a query pipeline that had
//   died would satisfy the baseline as convincingly as a live one.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18Seed(u32);

/// Gives the warm-up member an archetype of its own, disjoint from both the
/// victim's source `{G18Seed}` and its migration target `{G18Seed, G18Dep}`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18Other(u32);

/// The REQUIRED component — TABLE-stored, so the dense caller's expansion forces
/// a real migration and `target_archetype_id != source_archetype_id`. Without
/// that inequality the mutation under test is a no-op and the gate is vacuous.
/// Sentinel default, not zero (§B.0).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18Dep(u32);
impl Default for G18Dep {
    fn default() -> Self {
        G18Dep(0x1818_1818)
    }
}

/// The caller's own id: DENSE, carrying a TABLE `#[require]`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[require(G18Dep)]
#[repr(C)]
struct G18Needs(u32);

#[derive(Bundle)]
struct G18SeedBundle {
    s: G18Seed,
}

/// The pre-existing occupant of the migration TARGET archetype.
#[derive(Bundle)]
struct G18OccupantBundle {
    s: G18Seed,
    d: G18Dep,
}

/// The warm-up member's archetype. Already carries `G18Dep`, so its later
/// `G18Needs` insert takes the empty-required-closure branch and never migrates.
#[derive(Bundle)]
struct G18WarmBundle {
    o: G18Other,
    d: G18Dep,
}

/// A dense component suppresses the derive's single-component `Bundle` impl, so
/// the typed legs need this explicit wrapper.
#[derive(Bundle)]
struct G18NeedsBundle {
    n: G18Needs,
}

static G18_SEEN: AtomicUsize = AtomicUsize::new(0);

/// Runs the pure-dense query once and reports how many members it iterated.
fn g18_visible(schedule: &mut boyko_ecs::ecs::core::schedule::Schedule, world: &mut EcsMaster) -> usize {
    G18_SEEN.store(0, SEQ);
    schedule.run(world);
    G18_SEEN.load(SEQ)
}

#[test]
fn g18_a_dense_caller_with_require_seeds_the_target_archetypes_presence_bit() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let needs_id = G18Needs::component_id();

    let warm = spawn_one(&mut ecs, || G18WarmBundle {
        o: G18Other(1),
        d: G18Dep(0xFEED_FACE),
    });
    let occupant = spawn_one(&mut ecs, || G18OccupantBundle {
        s: G18Seed(9),
        d: G18Dep(OCCUPANT),
    });
    // Populated source, victim NOT in the last row.
    let by_id = spawn_one(&mut ecs, || G18SeedBundle { s: G18Seed(1) });
    let typed = spawn_one(&mut ecs, || G18SeedBundle { s: G18Seed(2) });
    let tail_a = spawn_one(&mut ecs, || G18SeedBundle { s: G18Seed(3) });
    let tail_b = spawn_one(&mut ecs, || G18SeedBundle { s: G18Seed(4) });

    // A PURE-DENSE query: its candidate set comes from `arch_presence` alone,
    // with no table column to fall back on. `Query<&G18Needs, With<G18Seed>>`
    // would not be this gate — a table term would change how candidates are
    // enumerated and could mask the seed entirely.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G18Needs>| {
        for _ in &q {
            G18_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // ── Warm-up: the query is LIVE, in a third archetype. ───────────────────
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(warm).insert(G18NeedsBundle {
            n: G18Needs(OCCUPANT),
        });
    });
    assert_eq!(
        g18_visible(&mut schedule, &mut ecs),
        1,
        "baseline: the pure-dense query iterates the warm-up member, so a later \
         zero means the SEED was lost and not that the query never worked"
    );

    // ── The subject: the BY-ID leg, run FIRST. ──────────────────────────────
    let src_arch = ecs.entity_archetype_id(by_id).expect("live entity");
    let payload = G18Needs(VICTIM);
    // SAFETY: registered `#[repr(C)]` POD type for `needs_id`.
    let bytes = unsafe { pod_bytes(&payload) };
    assert_eq!(
        ecs.add_component_by_id(by_id, needs_id, bytes),
        AddOutcome::Attached,
        "the by-id attach of a dense caller id with a table #[require] lands"
    );

    let tgt_arch = ecs.entity_archetype_id(by_id).expect("live entity");
    assert_ne!(
        tgt_arch, src_arch,
        "NON-VACUITY: the #[require] expansion must actually MIGRATE, otherwise \
         target and source are the same archetype and this gate tests nothing"
    );
    assert_eq!(
        Some(tgt_arch),
        ecs.entity_archetype_id(occupant),
        "the victim landed in the PRE-EXISTING target archetype, whose presence \
         bit for this dense id was clear before the attach"
    );
    assert_eq!(
        ecs.get_component::<G18Dep>(by_id).copied(),
        Some(G18Dep::default()),
        "the required component's ctor ran (gate 13b's property, re-anchored \
         here so a broken expansion reds as itself, not as a lost seed)"
    );
    assert!(
        ecs.dense_contains(by_id, needs_id),
        "MEMBERSHIP lands — and it lands under the defect too. This assertion \
         is the TRAP, kept deliberately: it is what gate 13b asserts, it is \
         `true` whichever archetype id was marked present, and it is why a \
         membership gate cannot see a wrong presence seed."
    );

    // THE DISCRIMINATOR. Seeding the SOURCE archetype instead of the TARGET
    // leaves this at 1: the victim keeps its bytes, keeps its membership, and
    // silently stops being a query candidate.
    assert_eq!(
        g18_visible(&mut schedule, &mut ecs),
        2,
        "ITERABILITY: after a #[require]-driven migration the dense caller must \
         be a candidate in the archetype it MOVED TO. `dense_insert_only` is \
         called before the migration and its archetype argument is what \
         `mark_arch_present` records, so passing `source_archetype_id` marks an \
         archetype the entity no longer occupies."
    );

    // ── The typed leg, LAST, and it is also a diagnosis. ────────────────────
    //
    // `migrate_entity_insert` marks the TARGET present. Because `typed` lands in
    // the SAME archetype as `by_id`, a victim that the defect had hidden
    // RESURFACES here through no act of its own — one unrelated typed insert
    // repairs the bit for every by-id victim in that archetype. That is why the
    // discriminator above must be read BEFORE this line, and why a gate that
    // only checked the end state would report green over the defect.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(typed).insert(G18NeedsBundle { n: G18Needs(SECOND) });
    });
    assert_eq!(
        g18_visible(&mut schedule, &mut ecs),
        3,
        "the typed path seeds the same target archetype; all three members are \
         visible"
    );
    assert_eq!(
        // SAFETY: registered type / hosted id (the dense arm of the raw reader).
        unsafe { read_back::<G18Needs>(&ecs, by_id, needs_id) },
        G18Needs(VICTIM),
        "BYTES: the caller's own payload is untouched by the expansion — the \
         seed is the only thing at issue"
    );

    // Bystanders, on BOTH axes (§B.0): two migrations out of `{G18Seed}`.
    for (who, want) in [(tail_a, 3u32), (tail_b, 4u32)] {
        assert_eq!(
            // SAFETY: registered type / hosted id.
            unsafe { read_back::<G18Seed>(&ecs, who, G18Seed::component_id()) },
            G18Seed(want),
            "BYTES: a source bystander keeps its value across the migrations"
        );
        assert!(
            row_is_inside_the_pool(&ecs, who, G18Seed::component_id()),
            "ROW: a swapped source bystander was repointed inside the pool"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 18b — the SAME defect one function over: the TYPED insert migration's
// dense presence bit must also be seeded on the **TARGET** archetype.
//
// Gate 18 above closes site 6 of the eight, `dense_insert_only`
// (`ecs_master/component_api.rs:132`), which is the by-id seam's own call.
// Site 3 — `commands/migration_helpers.rs:623`, inside
// `migrate_entity_insert`'s `for_each_component_bytes` closure — is the TYPED
// path's call, and it was ungated.
//
// # Measured, and it is the gate-18 measurement repeated verbatim
//
// Substituting `source_archetype_id` for `target_archetype_id` at that line
// compiles clean. MEASURED 2026-08-28 **on the tree as it stood BEFORE this
// gate**: `cargo test -p boyko-ecs --all-targets --no-fail-fast` at exit 0,
// 170 targets, 1882 passed, 0 failed — with gate 18 itself present and PASSING.
// The enclosing function's own doc guarantees
// `source_archetype_id != target_archetype_id`, so the swap is a real semantic
// change and not a rename of the same value.
//
// That figure is stamped rather than stated in the present tense on purpose:
// this gate is what makes it false. Under the same mutation today the run reds
// here, which is the whole point of writing it.
//
// # Why gate 18 cannot see it, and why its typed leg is not this gate
//
// Gate 18's typed leg runs LAST, by design, and by then the by-id leg has
// already raised the bit on `{G18Seed, G18Dep}`. A second `mark_arch_present`
// on an archetype that is already marked is a no-op whichever id is passed, so
// that leg reads the same 3 under the mutation as without it. Gate 18's header
// says as much about the ORDER; the consequence for the typed call site was not
// drawn.
//
// The exhaustiveness argument in gate 18's site enumeration is also not
// disturbed by this gate and does not cover it: that argument is about which
// call sites iterate an entity's PRE-EXISTING dense memberships (none do), a
// different question from whether each site passes the right archetype id.
//
// ⚠️ **This gate moves two figures gate 18's own header states.** That header
// records "170 result lines, 1881 passed … all 22 seam gates included" for ITS
// mutation, taken against a suite of 1882 tests and 22 gates. Adding this gate
// makes it 1883 tests and 23 gates, so re-running gate 18's mutation now yields
// 1882 passed over 23 gates and BOTH recorded numbers read one low.
//
// They are left as written rather than edited, because re-taking them is a full
// suite run under a DIFFERENT mutation and this rung did not perform one. An
// un-re-derived number rewritten to look current is the exact defect this
// campaign audits for; the stamp, not the digit, is what makes a past
// measurement honest. Recorded here so the next reader does not read either as
// a live figure.
//
// The same applies one layer out: `REFLECTION-PLAN-ECS.md`'s profile table
// states the clean debug total as 1882 over 170 result lines. That figure is
// ungated by construction — its subject is a `cargo test` process, which the
// measurement rung lists among the things it cannot re-derive — so nothing reds
// and the repair is a documentation edit, named rather than taken here.
//
// # What this gate does differently
//
// The victim's migration is the FIRST thing to put this dense id into the
// target archetype, so the bit is genuinely clear beforehand and the seed is the
// only thing that can raise it. There is no by-id leg at all: the subject is the
// typed `Commands::insert`, whose `#[require]` expansion forces the migration.
//
// # Fixture (§B.0)
//
// * **populated source** — three entities in `{G18bSeed}` with the victim at row
//   0, so `move_out_entity` returns `Swapped` and the inland repoint executes;
// * **populated target** — `occupant` already lives in `{G18bSeed, G18bDep}`
//   WITHOUT the dense id, so the target row is non-zero and its presence bit for
//   that id is clear;
// * **warm-up in a third archetype** — `{G18bOther, G18bDep}`, whose insert does
//   NOT migrate (the required component is already there), so the baseline comes
//   from the in-place path and a later zero means the SEED was lost rather than
//   that the query never worked;
// * **bystanders on two axes** — bytes and row.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18bSeed(u32);

/// The warm-up member's archetype, disjoint from both `{G18bSeed}` and
/// `{G18bSeed, G18bDep}`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18bOther(u32);

/// The REQUIRED component — TABLE-stored, so the typed insert of a dense id
/// forces a real migration. Sentinel default, not zero (§B.0).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18bDep(u32);
impl Default for G18bDep {
    fn default() -> Self {
        G18bDep(0x18B0_18B0)
    }
}

/// The inserted id: DENSE, carrying a TABLE `#[require]`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[require(G18bDep)]
#[repr(C)]
struct G18bNeeds(u32);

#[derive(Bundle)]
struct G18bSeedBundle {
    s: G18bSeed,
}

/// The pre-existing occupant of the migration TARGET archetype.
#[derive(Bundle)]
struct G18bOccupantBundle {
    s: G18bSeed,
    d: G18bDep,
}

/// The warm-up member's bundle. Already carries `G18bDep`, so its later
/// `G18bNeeds` insert changes no archetype and never reaches
/// `migrate_entity_insert`.
#[derive(Bundle)]
struct G18bWarmBundle {
    o: G18bOther,
    d: G18bDep,
}

/// A dense component suppresses the derive's single-component `Bundle` impl.
#[derive(Bundle)]
struct G18bNeedsBundle {
    n: G18bNeeds,
}

static G18B_SEEN: AtomicUsize = AtomicUsize::new(0);

/// Runs the pure-dense query once and reports how many members it iterated.
fn g18b_visible(
    schedule: &mut boyko_ecs::ecs::core::schedule::Schedule,
    world: &mut EcsMaster,
) -> usize {
    G18B_SEEN.store(0, SEQ);
    schedule.run(world);
    G18B_SEEN.load(SEQ)
}

#[test]
fn g18b_a_typed_dense_insert_that_migrates_seeds_the_target_archetypes_presence_bit() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let needs_id = G18bNeeds::component_id();

    let warm = spawn_one(&mut ecs, || G18bWarmBundle {
        o: G18bOther(1),
        d: G18bDep(0xFEED_FACE),
    });
    let occupant = spawn_one(&mut ecs, || G18bOccupantBundle {
        s: G18bSeed(9),
        d: G18bDep(OCCUPANT),
    });
    // Populated source, victim NOT in the last row.
    let victim = spawn_one(&mut ecs, || G18bSeedBundle { s: G18bSeed(1) });
    let tail_a = spawn_one(&mut ecs, || G18bSeedBundle { s: G18bSeed(3) });
    let tail_b = spawn_one(&mut ecs, || G18bSeedBundle { s: G18bSeed(4) });

    // A PURE-DENSE query: its candidate set comes from `arch_presence` alone,
    // with no table column to fall back on.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G18bNeeds>| {
        for _ in &q {
            G18B_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    // ── Warm-up: the query is LIVE, in a third archetype, seeded by the
    //    in-place path rather than by the migration under test. ──────────────
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(warm).insert(G18bNeedsBundle {
            n: G18bNeeds(OCCUPANT),
        });
    });
    assert_eq!(
        g18b_visible(&mut schedule, &mut ecs),
        1,
        "baseline: the pure-dense query iterates the warm-up member, so a later \
         one means the SEED was lost and not that the query never worked"
    );

    // ── The subject: a TYPED insert whose `#[require]` expansion migrates. ───
    let src_arch = ecs.entity_archetype_id(victim).expect("live entity");
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).insert(G18bNeedsBundle { n: G18bNeeds(VICTIM) });
    });
    let tgt_arch = ecs.entity_archetype_id(victim).expect("live entity");
    assert_ne!(
        tgt_arch, src_arch,
        "NON-VACUITY: the #[require] expansion must actually MIGRATE. \
         `migrate_entity_insert` is only reached when the ids differ — its own \
         doc guarantees it and it debug-asserts it — so equal ids would mean \
         this gate exercises the in-place path instead and tests nothing"
    );
    assert_eq!(
        Some(tgt_arch),
        ecs.entity_archetype_id(occupant),
        "the victim landed in the PRE-EXISTING target archetype, whose presence \
         bit for this dense id was clear before the insert — the occupant holds \
         the required component but not the dense one"
    );
    assert_eq!(
        ecs.get_component::<G18bDep>(victim).copied(),
        Some(G18bDep::default()),
        "the required component's ctor ran, so a broken expansion reds as \
         itself rather than as a lost seed"
    );
    assert_eq!(
        // SAFETY: registered `#[repr(C)]` POD type / hosted id (dense arm).
        unsafe { read_back::<G18bNeeds>(&ecs, victim, needs_id) },
        G18bNeeds(VICTIM),
        "MEMBERSHIP + BYTES land, and they land under the defect too. Kept as \
         the TRAP: `get_component_raw` resolves through the dense store and \
         `expect`s the entity hosts the id, so this reaching a value at all is \
         a membership proof — and it is a proof whichever archetype id was \
         marked present. Membership and ITERABILITY are different data, which \
         is why every assertion on this axis in the whole kernel suite was \
         green under the defect the next assertion catches"
    );
    // ⚠️ Gate 18's trap calls the dense membership predicate; this one does not,
    // and the difference is deliberate twice over.
    //
    // First, the raw read is the STRONGER proof: it panics where a boolean would
    // merely answer `false`, and it pins the payload in the same assertion.
    //
    // Second, one more call of that predicate anywhere under `crates/` is not
    // free. `REFLECTION-PLAN-ECS.md` states its call-site total as a LIVE figure
    // and the measurement rung re-derives it every run, so the first draft of
    // this gate — which did call it — took the workspace RED: the engine gate
    // reported that figure `written 54, re-measured 55`. A gate that falsifies a
    // documented count in order to assert something it already asserts is a bad
    // trade; this one does neither.
    //
    // ⚠️ And the predicate is NAMED above rather than written, for the same
    // reason: the sentence explaining why a token must not gain an occurrence
    // would otherwise BE the occurrence. The first draft of this comment was
    // exactly that — it spelled the token, and the total stayed at 55 with the
    // offending call already deleted.

    // THE DISCRIMINATOR. Seeding the SOURCE archetype instead of the TARGET
    // leaves this at 1: the victim keeps its bytes, keeps its membership, and
    // silently stops being a query candidate.
    assert_eq!(
        g18b_visible(&mut schedule, &mut ecs),
        2,
        "ITERABILITY: after a #[require]-driven TYPED migration the dense id \
         must be present on the archetype the entity MOVED TO. The bundle's \
         dense arm calls `mark_arch_present` from inside the migration, so \
         passing `source_archetype_id` marks an archetype the entity no longer \
         occupies and the entity stops being iterable through no act of its own"
    );

    // The BYTES axis is asserted ABOVE, before the discriminator, rather than
    // here: it is the same read and the same value, and running it twice with
    // only a query between would pin nothing the first one does not. What it
    // buys where it stands is that a broken payload reds as a payload failure
    // instead of arriving as a puzzling zero from the query.

    // Bystanders, on BOTH axes (§B.0): one migration out of `{G18bSeed}`.
    for (who, want) in [(tail_a, 3u32), (tail_b, 4u32)] {
        assert_eq!(
            // SAFETY: registered type / hosted id.
            unsafe { read_back::<G18bSeed>(&ecs, who, G18bSeed::component_id()) },
            G18bSeed(want),
            "BYTES: a source bystander keeps its value across the migration"
        );
        assert!(
            row_is_inside_the_pool(&ecs, who, G18bSeed::component_id()),
            "ROW: a swapped source bystander was repointed inside the pool"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 18c — the SECOND presence datum, on this rung's own by-id attach
// ════════════════════════════════════════════════════════════════════════════
//
// # The class, and what makes this enumeration complete
//
// Gates 18 and 18b guard ONE archetype-keyed candidate seed: the dense store's
// `arch_presence`. The header above gate 18b enumerates that symbol's eight
// production call sites and rules the claim to HOLD at all eight; the question
// log restated that ruling, until 2026-08-29, as an enumeration *"complete
// rather than sampled"* — a phrase that lives only there and was never written
// in this file, so a reader searching for it here finds nothing.
// **Both were complete over a SYMBOL, not over the CLASS.** There is a second
// archetype-keyed presence datum with the same failure mode, and the round that
// wrote the eight-site table did not look for it.
//
// The class is *an archetype-keyed presence bitset consumed as a query's
// candidate set*, and it is closed by a structural property rather than by a
// grep: a candidate set reaches a query ONLY through
// `QueryState::seed_from_candidates`, which is `pub(crate)` and has exactly
// FOUR call sites, all four in `iters/query/state.rs`. Two of them
// (construction and re-seed) read `EnablePresence::snapshot_present`; the other
// two read the dense terms' `arch_presence`. So the class has exactly two
// members, and a third could only appear as a fifth call site of that one
// funnel — which is a property a future reader can re-check in one grep, not a
// list they must trust.
//
// The second member is seeded by `EnablePresence::note_column_alloc`, reached
// only through `ArchetypeMaster::note_enable_column_alloc`, whose migration-side
// caller is `fire_enable_column_alloc_bookkeeping` in `migration_helpers.rs`.
// That helper is called from FIVE migration functions, each passing
// `target_archetype_id` while `source_archetype_id` is live and, in four of the
// five, a named parameter one token away:
//
//   1. `migrate_entity_insert`           4. `migrate_entity_detach_ids`
//   2. `migrate_entity_remove`           5. `migrate_entity_attach_ids_with_bytes`
//   3. `migrate_entity_attach_ids`
//
// The fifth is the by-id attach THIS rung added — so this landing extended a
// defect surface that had no gate. They are named by SYMBOL and not by line,
// twice over: a coordinate written beside a moving file has a shelf life of one
// landing, and five fresh `file.rs:N` forms in this comment would move the
// bound-citation total that another gate in this tree re-derives and pins.
//
// `note_enable_column_alloc` has ONE further caller and it is NOT a sixth
// instance: the in-place enable path in `ecs_master/enable_tag_api.rs` has a
// single archetype id in scope, so the wrong-id mutation cannot be written
// there — the same inexpressibility argument the eight-site table above makes
// for three of its eight, and a stronger statement than "it is covered".
//
// DISPOSITION: gated, here, by this test. Not deferred, not filed, not waived.
//
// And the query side is unforgiving in the same way gate 18's is: the code at
// the enable seed states that "the candidate bitset IS the membership predicate
// (nothing to trim)". There is no per-row pass to recover a wrong id. A bit on
// the wrong archetype is not a slow query, it is a silently empty one.
//
// # The measured before
//
// MEASURED 2026-08-29, one token at the by-id attach site swapped from
// `target_archetype_id` to `source_archetype_id`:
// `cargo test -p boyko-ecs --all-targets --no-fail-fast` returned **EXIT=0,
// 170 targets, 1883 passed, 0 failed** — with gates 18 and 18b passing beside
// it, because they watch the OTHER datum.
//
// ⚠️ **And `note_column_alloc`'s own "genuine first column" `debug_assert` did
// not fire either.** That is the stronger half of the finding: the assertion is
// unconditional in a debug build, so its silence proves no test in the suite
// reached the helper with a non-empty list on this path AT ALL. The site was
// UNREACHED, not merely unasserted — which is why a gate here had to construct
// the reaching case rather than pin an existing one.
//
// The reaching case is forced by where the bits come from. The migration reads
// the tags off the SOURCE row, so a non-empty list requires the source archetype
// to own an allocated column — and the target, being fresh, not to. That is what
// this gate builds, and it is why the source archetype necessarily already holds
// its own presence bit. Under the mutation the helper is therefore handed an id
// whose bit is ALREADY SET, so in a debug build the `debug_assert` fires first
// and the iterability assertion below is what pins the same defect in release.
// Both are reds; they are listed here so a future reader who sees the invariant
// panic instead of the count knows it is this gate working, not this gate broken.

/// Bitset enable tag for gate 18c — the sole term of the candidate-seeded query.
#[derive(Component)]
#[component(storage = "bitset")]
#[repr(C)]
struct G18cTag;

/// The source archetype's only table component.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18cSeed(u32);

/// The component attached BY ID, whose migration is the subject.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G18cAttach(u32);

#[derive(Bundle)]
struct G18cSeedBundle {
    s: G18cSeed,
}

static G18C_SEEN: AtomicUsize = AtomicUsize::new(0);

/// Runs the candidate-seeded query once and reports how many rows it iterated.
fn g18c_visible(
    schedule: &mut boyko_ecs::ecs::core::schedule::Schedule,
    world: &mut EcsMaster,
) -> usize {
    G18C_SEEN.store(0, SEQ);
    schedule.run(world);
    G18C_SEEN.load(SEQ)
}

#[test]
fn g18c_an_attach_by_id_seeds_the_target_archetypes_enable_presence_bit() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let attach_id = G18cAttach::component_id();

    // Populated source, victim NOT in the last row (so a lost seed cannot be
    // confused with a row-repointing bug, which the bystander block below pins).
    let warm = spawn_one(&mut ecs, || G18cSeedBundle { s: G18cSeed(1) });
    let victim = spawn_one(&mut ecs, || G18cSeedBundle { s: G18cSeed(2) });
    let tail = spawn_one(&mut ecs, || G18cSeedBundle { s: G18cSeed(3) });

    // The first toggle allocates `{G18cSeed}`'s enable column and records the
    // presence bit through the IN-PLACE path — deliberately not the migration
    // under test, so the warm-up member's visibility is independent evidence.
    ecs.enable::<G18cTag>(warm);
    ecs.enable::<G18cTag>(victim);

    // `Query<(), Enabled<T>>` is the canonical candidate-seeded shape: a sole
    // single enable term with no data component, so `IS_CANDIDATE_SEEDED` holds
    // and the enable-presence snapshot IS the membership predicate. Written
    // fully qualified because this file's `use` block sits above the highest
    // line at which the corpus cites it, and an import would renumber all of it.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(
        |q: Query<(), boyko_ecs::ecs::core::iters::query::filter_enable::Enabled<G18cTag>>| {
            for () in &q {
                G18C_SEEN.fetch_add(1, SEQ);
            }
        },
    );
    let mut schedule = builder.build(&mut ecs);

    // ── Baseline: the query is LIVE before the subject runs. ─────────────────
    assert_eq!(
        g18c_visible(&mut schedule, &mut ecs),
        2,
        "baseline: both enabled members are visible through the candidate-seeded \
         query, so a later shortfall means the SEED was lost and not that this \
         query shape never matched anything"
    );

    // ── The subject: an attach BY ID that migrates to a FRESH archetype. ─────
    let src_arch = ecs.entity_archetype_id(victim).expect("live entity");
    let payload = G18cAttach(VICTIM);
    // SAFETY: `G18cAttach` is a registered `#[repr(C)]` POD and `attach_id` is
    // its own registered id, so the borrowed span is a valid representation.
    let outcome = unsafe { ecs.add_component_by_id(victim, attach_id, pod_bytes(&payload)) };
    assert_eq!(
        outcome,
        AddOutcome::Attached,
        "NON-VACUITY: the by-id attach must actually attach. Any other outcome \
         means this gate never entered the migration it is written to guard"
    );

    let tgt_arch = ecs.entity_archetype_id(victim).expect("live entity");
    assert_ne!(
        tgt_arch, src_arch,
        "NON-VACUITY: the attach must MIGRATE. An in-place attach never reaches \
         the bookkeeping helper, so equal ids would leave this gate green over \
         nothing — the same cap-over-nothing shape the docs census gates"
    );
    assert!(
        ecs.is_enabled::<G18cTag>(victim),
        "the enable BIT must survive the migration. Asserted BEFORE the seed so \
         a lost bit reds as a lost bit rather than arriving as a puzzling zero \
         from the query — they are different data and different defects"
    );
    assert_eq!(
        // SAFETY: registered `#[repr(C)]` POD type / hosted id.
        unsafe { read_back::<G18cAttach>(&ecs, victim, attach_id) },
        G18cAttach(VICTIM),
        "BYTES land, and they land under the defect too — kept as the TRAP, \
         exactly as gate 18b keeps its raw read: membership and ITERABILITY are \
         different data, and every membership assertion in this suite was green \
         under the defect the next assertion catches"
    );

    // THE DISCRIMINATOR. Seeding the SOURCE archetype instead of the TARGET
    // leaves this at 1: the victim keeps its bytes, keeps its enable bit, keeps
    // its membership, and silently stops being a query candidate.
    assert_eq!(
        g18c_visible(&mut schedule, &mut ecs),
        2,
        "ITERABILITY: after a by-id attach the enable-presence bit must be \
         recorded on the archetype the entity MOVED TO. Passing \
         `source_archetype_id` marks an archetype the entity no longer occupies; \
         the candidate bitset IS the membership predicate for this query shape, \
         so there is no per-row trim to recover from a wrong id and the entity \
         stops being iterable through no act of its own"
    );

    // Bystanders on both axes (§B.0): one migration out of `{G18cSeed}`.
    for (who, want) in [(warm, 1u32), (tail, 3u32)] {
        assert_eq!(
            // SAFETY: registered type / hosted id.
            unsafe { read_back::<G18cSeed>(&ecs, who, G18cSeed::component_id()) },
            G18cSeed(want),
            "BYTES: a source bystander keeps its value across the migration"
        );
        assert!(
            row_is_inside_the_pool(&ecs, who, G18cSeed::component_id()),
            "ROW: a swapped source bystander was repointed inside the pool"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 19 — the CLONE materialiser seeds the CLONE's archetype, not the
//           SOURCE's. The second EXPRESSIBLE site FOUND in gate 18b's class.
// ════════════════════════════════════════════════════════════════════════════
//
// # Why this gate exists: a sentence that licensed its own absence
//
// The sweep above concluded — in its round-4 wording, superseded twice since —
// that "on SEVEN of the eight the mutation cannot be written". That sentence
// is not a description — it is a LICENCE. A claim that a defect is
// inexpressible at a site is a claim that a gate for that site is unnecessary,
// which is exactly the shape that has to be measured rather than reasoned.
//
// It was reasoned, and it is FALSE at site 1 — the clone materialiser. The
// sweep's own justification for that site reads "a clone materialiser whose
// signature carries only the target", and that is a true statement about the
// SIGNATURE and a false one about the SCOPE. `materialize_dense_memberships`
// takes `source: Entity` and `world: &mut EcsMaster` alongside
// `target_archetype_id`, and `EcsMaster::entity_archetype_id` is a public
// accessor. Hoisting one line beside the `current_tick` binding the loop
// already reads — `let src = world.entity_archetype_id(source).expect(..);` —
// makes `store.mark_arch_present(src)` compile. That is not a hypothetical
// shape: it is character-for-character the shape site 2 already ships, which
// the same sweep accepts as expressible.
//
// And the two ids can DIFFER, which is what makes the mutation a defect rather
// than a no-op. The clone's archetype is built from the cloner-FILTERED subset
// of the source's components, so a single `deny` puts source and clone in two
// different archetypes — and the fixture below does exactly that.
//
// ⚠️ Under the defect this site is SILENT in a way site 3's is not. The loop
// only runs for stores the SOURCE is a member of, so the source's archetype
// already carries the bit; marking it again changes nothing, and the clone's
// archetype is simply never marked. Nothing is corrupted, nothing panics, and
// the clone keeps its bytes and its membership. It stops being a query
// candidate, and that is the whole of the observable difference — which is why
// the discriminator below is an ITERATION COUNT and every other assertion in
// this gate is a trap that stays green under the defect.
//
// # The observation channel
//
// A PURE-DENSE `Query<&G19Dense>`, the same channel gate 18b uses and for the
// same reason: its candidate set comes from `arch_presence` alone, with no
// table column to fall back on and no per-row trim that could recover from a
// wrong id.
//
// # What this gate does NOT cover
//
// The cloner's OTHER filter mode (`only()`/`allow`), the non-cloneable skip,
// and the `strict` panic all reach the same loop and are covered elsewhere as
// behaviour; they are not re-covered here, because the seed is written once per
// materialized id and one filtered clone reaches it.

/// The table component BOTH the source and the clone carry — without it the
/// clone's archetype would be empty and the two archetypes could coincide for a
/// reason having nothing to do with the filter.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G19Seed(u32);

/// The table component the cloner DENIES. Its whole job is to make the clone's
/// archetype differ from the source's, so a seed written against the source is
/// written against an archetype the clone does not occupy.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G19Drop(u32);

/// The table component of the WARM-UP member's archetype — a third archetype,
/// so the baseline proves the query works before the subject runs.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct G19Other(u32);

/// The DENSE id under test: materialized onto the clone by the site under test,
/// and the only term of the pure-dense query.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct G19Dense(u32);

/// The clone SOURCE: two table ids and the dense membership.
#[derive(Bundle)]
struct G19SourceBundle {
    s: G19Seed,
    p: G19Drop,
    d: G19Dense,
}

/// The warm-up member, in a third archetype, seeded by the SPAWN path rather
/// than by the clone path under test.
#[derive(Bundle)]
struct G19WarmBundle {
    o: G19Other,
    d: G19Dense,
}

static G19_SEEN: AtomicUsize = AtomicUsize::new(0);

/// Runs the pure-dense query once and reports how many members it iterated.
fn g19_visible(
    schedule: &mut boyko_ecs::ecs::core::schedule::Schedule,
    world: &mut EcsMaster,
) -> usize {
    G19_SEEN.store(0, SEQ);
    schedule.run(world);
    G19_SEEN.load(SEQ)
}

#[test]
fn g19_a_filtered_clone_seeds_the_clones_archetype_presence_bit_not_the_sources() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut ecs = EcsMaster::new();
    let dense_id = G19Dense::component_id();

    // Warm-up member: a THIRD archetype, so a later count of 1 means the seed
    // was lost and not that the query never worked.
    let warm = spawn_one(&mut ecs, || G19WarmBundle {
        o: G19Other(1),
        d: G19Dense(OCCUPANT),
    });
    let source = spawn_one(&mut ecs, || G19SourceBundle {
        s: G19Seed(7),
        p: G19Drop(9),
        d: G19Dense(VICTIM),
    });

    // A PURE-DENSE query: its candidate set comes from `arch_presence` alone,
    // with no table column to fall back on.
    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&G19Dense>| {
        for _ in &q {
            G19_SEEN.fetch_add(1, SEQ);
        }
    });
    let mut schedule = builder.build(&mut ecs);

    assert_eq!(
        g19_visible(&mut schedule, &mut ecs),
        2,
        "baseline: the pure-dense query iterates the warm-up member AND the \
         clone source, both seeded by the SPAWN path. Asserted before the \
         subject runs so a later shortfall is attributable to the clone path \
         and not to a query that never matched anything"
    );

    // ── The subject: a FILTERED clone, whose target archetype differs. ───────
    let src_arch = ecs.entity_archetype_id(source).expect("live entity");
    let cloner = boyko_ecs::ecs::core::clone::EntityCloner::new()
        .deny::<G19Drop>()
        .build();
    let clone = ecs.clone_and_spawn_with(source, &cloner);
    let clone_arch = ecs.entity_archetype_id(clone).expect("live clone");

    assert_ne!(
        clone_arch, src_arch,
        "NON-VACUITY: the DENIED table id must actually move the clone into a \
         different archetype. Equal ids would make the wrong-id mutation a \
         no-op and leave this gate green over nothing — the same cap-over-\
         nothing shape gate 18b's own non-vacuity assertion guards against"
    );
    assert_eq!(
        ecs.get_component::<G19Seed>(clone).copied(),
        Some(G19Seed(7)),
        "the KEPT table id crossed the filter, so a broken filter reds as a \
         broken filter rather than arriving later as a puzzling shortfall"
    );
    assert!(
        ecs.get_component::<G19Drop>(clone).is_none(),
        "the DENIED table id did NOT cross. This is what makes the archetypes \
         differ, so it is asserted rather than assumed"
    );
    assert_eq!(
        // SAFETY: registered `#[repr(C)]` POD type / hosted id (dense arm).
        unsafe { read_back::<G19Dense>(&ecs, clone, dense_id) },
        G19Dense(VICTIM),
        "MEMBERSHIP + BYTES land, and they land under the defect too. Kept as \
         the TRAP, exactly as gates 18b and 18c keep theirs: the raw accessor \
         resolves through the dense store and `expect`s the entity hosts the \
         id, so reaching a value at all is a membership proof — and it is a \
         proof whichever archetype id was marked present"
    );

    // THE DISCRIMINATOR. Seeding the SOURCE archetype instead of the CLONE's
    // leaves this at 2: the clone keeps its bytes, keeps its membership, and
    // silently stops being a query candidate.
    assert_eq!(
        g19_visible(&mut schedule, &mut ecs),
        3,
        "ITERABILITY: the dense id materialized onto a CLONE must be present \
         on the archetype the CLONE occupies. The source's archetype already \
         carries the bit — the loop only runs for stores the source belongs to \
         — so passing the source's id marks nothing new and the clone becomes \
         invisible to the query through no act of its own. That silence is why \
         this site needed a gate and not a sentence"
    );

    // Bystanders on both axes: neither pre-existing member is disturbed.
    for (who, want) in [(warm, OCCUPANT), (source, VICTIM)] {
        assert_eq!(
            // SAFETY: registered `#[repr(C)]` POD type / hosted id (dense arm).
            unsafe { read_back::<G19Dense>(&ecs, who, dense_id) },
            G19Dense(want),
            "BYTES: a pre-existing dense member keeps its value across the \
             clone materialization, which inserts a FRESH slot rather than \
             migrating an existing one"
        );
    }
}
