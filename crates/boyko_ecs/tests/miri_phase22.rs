//! Phase 22 (Tags) — Miri (Tree Borrows) coverage for the new unsafe
//! migration surface (plan "Metrics and validation" Miri-TB list).
//!
//! Run via (NOTE the `-Zmiri-ignore-leaks` — see below):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase22
//! ```
//!
//! # Why `-Zmiri-ignore-leaks`
//!
//! The retained-columns repro spawns a data bundle through `Commands`, which
//! reaches the by-design bounded `BundleColumnCache` `Box::leak` (#53,
//! NOT-A-BUG — a deliberate borrow-decoupling leak). `-Zmiri-ignore-leaks`
//! isolates the Tree-Borrows signal, matching the sibling suites
//! (`miri_phase14b` / `miri_phase19` / `miri_pool_growth` / …).
//!
//! # What Miri proves here
//!
//! The F2 / NEW-1 / BUG-P19-TB-1 UB class (a `NonNull<EcsMaster>` /
//! raw-twin mint aliasing a live `&mut` reborrow) was historically caught
//! ONLY by Miri-TB — never by review. Phase 22's three new unsafe helpers
//! replicate the Phase-14a §3.4/§3.5 confinement; this file drives each of
//! them under `-Zmiri-tree-borrows`:
//!
//! 1. `migrate_entity_attach_ids` — INCLUDING the zero-retained shape
//!    (attach FROM the empty archetype, O3) and the retained-column memcpy +
//!    tick read/write shape (data entity).
//! 2. `migrate_entity_detach_ids` — INCLUDING detach-to-empty (O3) and the
//!    retained `MaybeUninit` slot collection + `create_entity_with_ticks`.
//! 3. `retag_in_place` — the in-place present-tag re-insert (changed-tick
//!    stamp + PRE/POST hook mints around per-iteration `&mut *archetype_ptr`
//!    reborrows).
//! 4. Hook/observer firing at the new sites (the Phase-2 `world_ptr` mint
//!    with no live archetype reborrow — SAFETY-1).
//! 5. Empty-archetype despawn (swap over ZERO pools — the D5(3) row-identity
//!    shape) and despawn of tag-carrying entities (ZST column in the drop
//!    walk).
//! 6. Drop-impl ZST pool teardown (`drop_in_place::<ZST>` at the dangling
//!    SIMD-aligned base) — raw pool AND world teardown.
//! 7. The GROW1-ZST growth path (`grow_rows_zst`): first commit + the
//!    second-commit boundary crossing through the public `add_typed` funnel.
//! 8. The deferred command paths (`AddTagCommand` / `RemoveTagCommand`
//!    through `Commands` / `EntityCommands` + the depth-gated drain).
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. Native behavioral coverage of
//! the same semantics lives in `phase22_tags.rs` / `phase22_static_tags.rs` /
//! `phase22_empty_archetype.rs` and the Phase-22 proptest suites. Entity
//! counts are kept tiny (Miri is ~100x slower); the ONE deliberate exception
//! is the GROW1-ZST second-commit test, whose 16 K iterations are leaf-level
//! ZST pool adds (no migrations, no hooks, no commands — see its doc).
//!
//! # Registry budget discipline (see `tests/phase22_tags.rs` header)
//!
//! This binary mints a handful of uniquely-named (`miri_p22_*`) dynamic tags
//! plus a few derive-minted typed components — far below the shared 512-slot
//! ComponentId budget. No mint-to-ceiling here.

#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self};
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::hooks::{ComponentHooks, HookContext};
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::memory::component_pool::ComponentPool;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

/// Runs one deferred-command system and returns the `Entity` it captured
/// (the `Arc<Mutex<_>>` capture idiom — system closures must be `Send`).
fn run_capturing(
    ecs: &mut EcsMaster,
    f: impl Fn(&mut Commands) -> Entity + Send + Sync + 'static,
) -> Entity {
    let slot: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&slot);
    ecs.run_system(move |mut cmds: Commands| {
        *probe.lock().expect("not poisoned") = Some(f(&mut cmds));
    });
    let captured = slot.lock().expect("not poisoned").take();
    captured.expect("the system ran and captured an entity")
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1 — attach FROM empty (zero retained columns) + detach TO empty (O3)
//            through the direct API: migrate_entity_attach_ids with a
//            pool-less source, migrate_entity_detach_ids with an empty kept
//            set, and the warm-path round trip over both cached archetypes.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_attach_from_empty_detach_to_empty_round_trip() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_round_trip");

    let e = world.spawn_empty();
    let empty_arch = world.get_entity_archetype_id(e).expect("live entity");
    assert!(!world.has_tag(e, tag), "fresh empty entity carries no tag");

    // Attach FROM the empty archetype: the retained-copy loop runs ZERO
    // times; `move_out_entity` walks a pool-less archetype (O3).
    world.add_tag(e, tag);
    assert!(world.has_tag(e, tag), "attach-from-empty lands the tag");
    let tagged_arch = world.get_entity_archetype_id(e).expect("live entity");
    assert_ne!(empty_arch, tagged_arch, "attach migrated out of EMPTY");
    assert_eq!(
        world.query_entities(&[tag.component_id()]),
        vec![e],
        "the tagged entity is visible to the id-keyed query"
    );

    // Detach TO the empty archetype: kept set is empty; the push routes
    // through `create_entity_with_ticks` with an empty retained slice (O3).
    world.remove_tag(e, tag);
    assert!(!world.has_tag(e, tag), "detach-to-empty clears the tag");
    assert!(world.has_entity(e), "entity survives with zero components");
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(empty_arch),
        "detach-to-empty routes back to THE empty archetype (exact-mask cache)"
    );

    // Warm round trip: both archetypes are cached now — re-run both
    // migrations over warm state.
    world.add_tag(e, tag);
    assert_eq!(world.get_entity_archetype_id(e), Some(tagged_arch));
    world.remove_tag(e, tag);
    assert_eq!(world.get_entity_archetype_id(e), Some(empty_arch));
}

// ════════════════════════════════════════════════════════════════════════════
// Target 2 — attach / detach with RETAINED columns: the Step-1 byte-slice
//            borrow + tick reads + write_at_unchecked_initialized/commit_units
//            (attach) and the MaybeUninit retained-slot collection +
//            create_entity_with_ticks (detach), each across a live data pool.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct MData {
    a: u64,
    b: u64,
}

#[derive(Bundle)]
struct MDataBundle {
    d: MData,
}

const M_DATA: MData = MData { a: 0xA5A5_5A5A_DEAD_BEEF, b: 0x0F0F_F0F0_CAFE_BABE };

#[test]
fn miri_attach_detach_with_retained_columns() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_retained");

    // Two data rows so the source pool stays non-trivial across the
    // migration (the swap path has a live neighbor).
    let e = run_capturing(&mut world, |cmds| cmds.spawn(MDataBundle { d: M_DATA }).id());
    let neighbor =
        run_capturing(&mut world, |cmds| cmds.spawn(MDataBundle { d: MData { a: 1, b: 2 } }).id());
    let data_arch = world.get_entity_archetype_id(e).expect("live entity");

    // Attach: ONE retained column rides the migration (byte slice borrowed
    // from the live source pool, consumed by the target write in the same
    // iteration — the Round-3 C-N2 shape under TB).
    world.add_tag(e, tag);
    assert!(world.has_tag(e, tag));
    assert_eq!(
        world.get_component::<MData>(e).expect("data survived the attach"),
        &M_DATA,
        "retained column memcpy is byte-exact"
    );
    assert_eq!(
        world.get_component::<MData>(neighbor).expect("neighbor untouched"),
        &MData { a: 1, b: 2 },
        "the swap fixup left the neighbor's row coherent"
    );

    // Detach: the retained set is collected into the stack MaybeUninit slot
    // array and pushed via create_entity_with_ticks; the removed ZST column
    // takes the uniform drop_at (no drop_fn for a dynamic tag).
    world.remove_tag(e, tag);
    assert!(!world.has_tag(e, tag));
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(data_arch),
        "detach returns the entity to the original data archetype"
    );
    assert_eq!(
        world.get_component::<MData>(e).expect("data survived the detach"),
        &M_DATA,
        "retained column rode back byte-exact"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 3 — retag_in_place + hook/observer mints at ALL three new fire
//            sites: the Phase-2 `NonNull<EcsMaster>` mint must alias no live
//            `&mut Archetype` reborrow (SAFETY-1) — the exact F2 class.
// ════════════════════════════════════════════════════════════════════════════

static H3_ADD: AtomicUsize = AtomicUsize::new(0);
static H3_INSERT: AtomicUsize = AtomicUsize::new(0);
static H3_REPLACE: AtomicUsize = AtomicUsize::new(0);
static H3_REMOVE: AtomicUsize = AtomicUsize::new(0);
static O3_INSERT: AtomicUsize = AtomicUsize::new(0);
static O3_REPLACE: AtomicUsize = AtomicUsize::new(0);

unsafe fn h3_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    H3_ADD.fetch_add(1, SEQ);
}
unsafe fn h3_on_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    H3_INSERT.fetch_add(1, SEQ);
}
unsafe fn h3_on_replace(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    H3_REPLACE.fetch_add(1, SEQ);
}
unsafe fn h3_on_remove(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    H3_REMOVE.fetch_add(1, SEQ);
}
unsafe fn o3_on_insert(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Insert);
    O3_INSERT.fetch_add(1, SEQ);
}
unsafe fn o3_on_replace(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    assert_eq!(ctx.kind, ObserverKind::Replace);
    O3_REPLACE.fetch_add(1, SEQ);
}

#[test]
fn miri_retag_in_place_and_fire_sites() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_retag");
    let cid = tag.component_id();

    // Contract order (H1): mint -> register hooks -> first attach.
    component_registry::register_hooks_by_id(
        cid,
        ComponentHooks {
            on_add: Some(h3_on_add),
            on_insert: Some(h3_on_insert),
            on_replace: Some(h3_on_replace),
            on_remove: Some(h3_on_remove),
        },
    )
    .expect("fresh tag, never archetyped");
    world.add_observer(ObserverKind::Insert, cid, o3_on_insert);
    world.add_observer(ObserverKind::Replace, cid, o3_on_replace);

    let e = world.spawn_empty();

    // Fresh attach (migrate_entity_attach_ids Phase 2): on_add + on_insert,
    // hook then observer.
    world.add_tag(e, tag);
    assert_eq!((H3_ADD.load(SEQ), H3_INSERT.load(SEQ)), (1, 1), "attach fires add+insert");
    assert_eq!(O3_INSERT.load(SEQ), 1, "attach fires the Insert observer");
    let tagged_arch = world.get_entity_archetype_id(e).expect("live entity");

    // Present-tag re-add (retag_in_place): on_replace PRE, on_insert POST,
    // NO on_add, NO migration — the per-iteration `&mut *archetype_ptr`
    // reborrows must be dead before each world_ptr mint.
    world.add_tag(e, tag);
    assert_eq!(
        (H3_ADD.load(SEQ), H3_REPLACE.load(SEQ), H3_INSERT.load(SEQ)),
        (1, 1, 2),
        "re-add fires replace+insert in place, never add"
    );
    assert_eq!((O3_REPLACE.load(SEQ), O3_INSERT.load(SEQ)), (1, 2));
    assert_eq!(
        world.get_entity_archetype_id(e),
        Some(tagged_arch),
        "in-place re-add must not migrate"
    );

    // Detach (migrate_entity_detach_ids Phase 2): on_replace then on_remove
    // against the dying source row (EntityInland still points at source).
    world.remove_tag(e, tag);
    assert_eq!(
        (H3_REPLACE.load(SEQ), H3_REMOVE.load(SEQ)),
        (2, 1),
        "detach fires replace then remove exactly once each"
    );
    assert_eq!(O3_REPLACE.load(SEQ), 2, "detach fires the Replace observer");
    assert!(!world.has_tag(e, tag));
    assert!(world.has_entity(e), "detach-to-empty keeps the entity alive");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 4 — empty-archetype despawn: the swap-remove Swapped/Last outcomes
//            over ZERO pools (D5(3) row identity) + despawn of tag-carrying
//            entities (a ZST column inside the despawn drop walk).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_empty_archetype_despawn_swap_and_last() {
    let mut world = EcsMaster::new();

    let e1 = world.spawn_empty();
    let e2 = world.spawn_empty();
    assert_eq!(world.entity_count(), 2);

    // Despawn the FIRST: the Swapped outcome over zero pools — e2's row
    // moves into slot 0 and its unit_index fixup must keep it addressable.
    assert!(world.delete_entity(e1), "despawn of a live empty entity succeeds");
    assert!(!world.has_entity(e1));
    assert!(world.has_entity(e2), "swapped survivor's identity intact");

    // Despawn the survivor: the Last outcome.
    assert!(world.delete_entity(e2));
    assert_eq!(world.entity_count(), 0);

    // Slot reuse after the churn: a fresh spawn is live; the stale handles
    // stay dead (generation mismatch).
    let e3 = world.spawn_empty();
    assert!(world.has_entity(e3));
    assert!(!world.has_entity(e1), "stale handle stays dead after slot reuse");
}

#[test]
fn miri_despawn_tag_carrying_entities() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_despawn_tagged");

    // Tag-only entity (ZST column only) + data+tag entity (mixed pools).
    let tag_only = world.spawn_empty();
    world.add_tag(tag_only, tag);
    let mixed = run_capturing(&mut world, |cmds| cmds.spawn(MDataBundle { d: M_DATA }).id());
    world.add_tag(mixed, tag);

    assert!(world.delete_entity(tag_only), "despawn over a pure ZST column");
    assert!(world.delete_entity(mixed), "despawn over mixed data+ZST columns");
    assert_eq!(world.entity_count(), 0);
    assert!(
        world.query_entities(&[tag.component_id()]).is_empty(),
        "no tagged row survives the despawns"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 5 — Drop-impl ZST pool teardown: drop_in_place::<ZST> at the
//            dangling SIMD-aligned base reads no bytes; swap_remove / pop /
//            pool-Drop each account exactly one drop per logical row.
// ════════════════════════════════════════════════════════════════════════════

/// Raw-pool fixture. The counter is a static (a counting FIELD would make
/// the type non-zero-sized), so this type is used by exactly ONE test.
#[derive(Component)]
struct ZstDropRaw;

static ZST_DROP_RAW_COUNT: AtomicUsize = AtomicUsize::new(0);

impl Drop for ZstDropRaw {
    fn drop(&mut self) {
        ZST_DROP_RAW_COUNT.fetch_add(1, SEQ);
    }
}

#[test]
fn miri_drop_impl_zst_pool_teardown() {
    assert_eq!(std::mem::size_of::<ZstDropRaw>(), 0, "fixture must be a ZST");
    assert!(std::mem::needs_drop::<ZstDropRaw>(), "fixture must carry drop glue");

    let mut pool = ComponentPool::new(ZstDropRaw::component_id().0, 16);
    let base = ZST_DROP_RAW_COUNT.load(SEQ);

    const M: usize = 5;
    for _ in 0..M {
        pool.add_typed(ZstDropRaw).expect("under the 16-row ceiling");
    }
    assert_eq!(ZST_DROP_RAW_COUNT.load(SEQ) - base, 0, "adds (by-move) must not drop");

    assert!(pool.swap_remove(1), "swap_remove(1) in bounds");
    assert_eq!(
        ZST_DROP_RAW_COUNT.load(SEQ) - base,
        1,
        "swap_remove drops the removed ZST exactly once"
    );

    assert!(pool.pop(), "pop while non-empty");
    assert_eq!(ZST_DROP_RAW_COUNT.load(SEQ) - base, 2, "pop drops the tail ZST exactly once");

    let live = pool.count();
    drop(pool);
    assert_eq!(
        ZST_DROP_RAW_COUNT.load(SEQ) - base,
        2 + live,
        "pool Drop drops each surviving logical row exactly once"
    );
}

/// World-teardown fixture — its own type so the two Drop-counting tests
/// never observe each other's statics under the parallel harness.
#[derive(Component)]
struct ZstDropWorld;

static ZST_DROP_WORLD_COUNT: AtomicUsize = AtomicUsize::new(0);

impl Drop for ZstDropWorld {
    fn drop(&mut self) {
        ZST_DROP_WORLD_COUNT.fetch_add(1, SEQ);
    }
}

#[test]
fn miri_world_teardown_drops_live_zst_tag_rows() {
    let base = ZST_DROP_WORLD_COUNT.load(SEQ);
    {
        let mut world = EcsMaster::new();
        let arch = world.get_or_create_archetype(&[ZstDropWorld::component_id()]);
        for _ in 0..3 {
            world.spawn_one(arch, ZstDropWorld).expect("spawn Drop-impl ZST tag");
        }
        assert_eq!(ZST_DROP_WORLD_COUNT.load(SEQ) - base, 0, "spawns must not drop");
        // <- world drops here: archetype pool teardown runs drop_in_place
        //    once per live ZST row at the dangling base.
    }
    assert_eq!(
        ZST_DROP_WORLD_COUNT.load(SEQ) - base,
        3,
        "world teardown drops each live Drop-impl ZST row exactly once"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 6 — the GROW1-ZST growth path under TB through the PUBLIC funnel.
// ════════════════════════════════════════════════════════════════════════════

/// Data-less, drop-less ZST for the grow tests.
#[derive(Component)]
#[derive(Clone, Copy)]
struct ZstGrowTag;

/// First-commit shape: committed_rows 0 -> (granule/4).min(reserve). Cheap.
#[test]
fn miri_zst_pool_first_commit_and_ceiling() {
    let mut pool = ComponentPool::new(ZstGrowTag::component_id().0, 8);
    assert_eq!(pool.component_layout().size(), 0, "fixture stride pin");
    assert_eq!(pool.committed_rows(), 0, "zero initial commit");
    let buffer_before = pool.buffer_ptr();

    for i in 0..8usize {
        assert_eq!(pool.add_typed(ZstGrowTag), Some(i), "add returns the tail index");
    }
    assert_eq!(pool.count(), 8);
    assert_eq!(pool.committed_rows(), 8, "one tick granule covers the whole 8-row reserve");
    assert!(pool.is_full(), "len == reserve_rows");

    // Ceiling: None with zero observable state change.
    let before = (pool.count(), pool.committed_rows(), pool.buffer_ptr() as usize);
    assert_eq!(pool.add_typed(ZstGrowTag), None, "reserve ceiling -> None");
    let after = (pool.count(), pool.committed_rows(), pool.buffer_ptr() as usize);
    assert_eq!(before, after, "rejected add: state EXACTLY unchanged");

    assert_eq!(pool.buffer_ptr(), buffer_before, "dangling base never moves");
    for i in 0..8 {
        assert!(pool.get_typed::<ZstGrowTag>(i).is_some(), "typed ZST read at row {i}");
    }
    assert!(pool.get_raw(8).is_none(), "out-of-bounds read is None");
}

/// THE second-commit boundary: ticks are 4 B/row, one commit granule
/// (64 KiB) covers 16,384 rows — add #16,385 drives `grow_rows_zst` to its
/// SECOND `vm.commit` pair (doubling, Z4 strict growth) through the public
/// `add_typed` funnel.
///
/// COST NOTE (tiny-count rule exception): 16,385 iterations of a LEAF ZST
/// add (len check + zero-size write + len bump — no migrations, no hooks,
/// no commands), the cheapest op per iteration in this suite. Under the
/// Miri fallback arm the reservation is one eager 256 KiB `alloc_zeroed`
/// and `commit` is bookkeeping-only, so the loop is interpreter-bound, not
/// allocation-bound. If this test ever exceeds a reasonable wall-time
/// budget, gate it with `#[cfg_attr(miri, ignore = "...")]` and keep the
/// first-commit sibling (the in-crate
/// `zst_pool_growth_two_successive_commits` unit test pins the same gate
/// natively via direct `grow_rows` calls).
#[test]
fn miri_zst_pool_grow_second_commit() {
    const GRANULE_ROWS: usize = (64 * 1024) / 4; // 16,384
    const RESERVE: usize = 20_000;

    let mut pool = ComponentPool::new(ZstGrowTag::component_id().0, RESERVE);
    let buffer_before = pool.buffer_ptr();

    // Fill exactly one granule of rows: ONE commit event at the first add.
    for _ in 0..GRANULE_ROWS {
        pool.add_typed(ZstGrowTag).expect("under the reserve ceiling");
    }
    assert_eq!(pool.count(), GRANULE_ROWS);
    assert_eq!(
        pool.committed_rows(),
        GRANULE_ROWS,
        "first commit = one tick granule = 16,384 rows"
    );

    // The boundary add: needed_t crosses the frontier -> second commit pair
    // (64 KiB -> 128 KiB), clamped to the 20,000-row reserve.
    pool.add_typed(ZstGrowTag).expect("boundary add under the ceiling");
    assert_eq!(pool.count(), GRANULE_ROWS + 1);
    assert_eq!(
        pool.committed_rows(),
        RESERVE,
        "second commit: (2G/4).min(reserve_rows) == 20,000 (Z5 clamp)"
    );

    // Base stability + a live read on each side of the granule boundary.
    assert_eq!(pool.buffer_ptr(), buffer_before, "dangling base stable across both commits");
    assert!(pool.get_typed::<ZstGrowTag>(0).is_some(), "row 0 (first granule)");
    assert!(
        pool.get_typed::<ZstGrowTag>(GRANULE_ROWS).is_some(),
        "row 16,384 (second granule)"
    );

    // swap_remove across the boundary: tick lockstep machinery runs over
    // the freshly committed second granule.
    assert!(pool.swap_remove(0), "swap_remove(0) pulls the boundary row down");
    assert_eq!(pool.count(), GRANULE_ROWS);
}

// ════════════════════════════════════════════════════════════════════════════
// Target 7 — the deferred command paths: AddTagCommand / RemoveTagCommand
//            via Commands / EntityCommands, the FIFO spawn_empty+add_tag
//            chain, the depth-gated drain (DeferredScopeGuard at depth >= 1),
//            and the dead-entity no-op at apply.
// ════════════════════════════════════════════════════════════════════════════

static DEF22_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn def22_on_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    DEF22_ADD.fetch_add(1, SEQ);
}

#[test]
fn miri_deferred_tag_command_paths() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_deferred");

    component_registry::register_hooks_by_id(
        tag.component_id(),
        ComponentHooks { on_add: Some(def22_on_add), ..Default::default() },
    )
    .expect("fresh tag, never archetyped");

    // FIFO chain in ONE system: spawn_empty then AddTagCommand — the apply
    // runs add_tag at drain depth >= 1, so its inner drain no-ops and the
    // outermost drive drains (Q-A1).
    let e = run_capturing(&mut world, move |cmds| cmds.spawn_empty().add_tag(tag).id());
    assert!(world.has_tag(e, tag), "deferred attach landed at apply");
    assert_eq!(DEF22_ADD.load(SEQ), 1, "on_add fired through the deferred route");

    // Deferred present-tag re-add: AddTagCommand delegates to retag_in_place.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).add_tag(tag);
    });
    assert!(world.has_tag(e, tag));
    assert_eq!(DEF22_ADD.load(SEQ), 1, "in-place re-add never fires on_add");

    // Deferred detach (detach-to-empty through RemoveTagCommand).
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove_tag(tag);
    });
    assert!(!world.has_tag(e, tag), "deferred detach landed at apply");
    assert!(world.has_entity(e), "entity survives detach-to-empty");

    // Despawn + add_tag on the same entity in one queue (FIFO): the
    // AddTagCommand applies against a dead entity and must no-op.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).despawn().add_tag(tag);
    });
    assert!(!world.has_entity(e), "despawn applied first (FIFO)");
    assert_eq!(DEF22_ADD.load(SEQ), 1, "the dead-entity AddTagCommand no-ops");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 8 — dead / stale handle guards on the direct API: the early-return
//            inland resolution (generation check) under TB, including a
//            recycled slot.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dead_entity_tag_ops_no_op() {
    let mut world = EcsMaster::new();
    let tag = world.register_tag("miri_p22_dead_handles");

    let e = world.spawn_empty();
    assert!(world.delete_entity(e));

    assert!(!world.has_tag(e, tag), "has_tag on a dead handle is false");
    world.add_tag(e, tag); // silent no-op, must not resurrect
    assert!(!world.has_tag(e, tag));
    world.remove_tag(e, tag); // silent no-op
    assert_eq!(world.entity_count(), 0);

    // Recycled slot: the stale generation must never touch the new occupant.
    let e2 = world.spawn_empty();
    world.add_tag(e, tag);
    assert!(!world.has_tag(e2, tag), "a stale handle must never tag the recycled entity");
    world.add_tag(e2, tag);
    assert!(world.has_tag(e2, tag), "the live handle still works after the stale probe");
}
