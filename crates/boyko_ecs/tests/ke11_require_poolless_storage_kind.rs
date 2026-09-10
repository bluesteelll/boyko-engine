//! **KE11 — `#[require]` of an id whose storage kind owns NO per-archetype pool.**
//!
//! This file was a MEASUREMENT and is now a GATE. Ballot **AB-6** resolved as a
//! **split**, and the split is the whole point: the two poolless storage kinds
//! are not one class, and treating them as one is what let the defect read as a
//! single missing feature.
//!
//! * **DENSE** — `(b) build it`. A dense component has bytes and a constructor;
//!   it merely lives in another store. `#[require]` of it is meaningful, so the
//!   kernel now constructs it into its `DenseStore` at both sites.
//! * **BITSET / flag** — `(a) refuse it`. A flag has NO bytes. `RequiredCtor` is
//!   an `unsafe fn(*mut u8)` that writes bytes into a slot, and for a flag there
//!   is nothing to write. That is meaningless in the construct, not
//!   unimplemented — and the capability "attaching X sets flag F" already exists
//!   under its own name, `FLAGS_DIRECT` (KE10), designed exactly as *a bit, not
//!   bytes; no ctor*.
//!
//! The refusal lives in **`boyko_macros`**, as a const-assert on
//! `Component::STORAGE_IS_BITSET`, and NOT in Aether. Aether's prime directive
//! is that it emits exactly the code a disciplined engineer would hand-write; a
//! refusal in Aether alone would make Aether reject what the derive accepts,
//! which is a divergence between the two surfaces rather than one rule.
//!
//! # What this file measured before the fix (2026-08-30, rung R1) — RETAINED
//!
//! The historical record of what the fix removed. All four combinations panicked,
//! each landing where the code reading predicted; captured `--nocapture`:
//!
//! ```text
//! site1 … bitset_flag … panicked at crates\boyko_ecs\src\ecs\core\bundle\bundle_column_cache.rs:410:22
//! site1 … dense       … panicked at crates\boyko_ecs\src\ecs\core\bundle\bundle_column_cache.rs:410:22
//! site2 … bitset_flag … panicked at crates\boyko_ecs\src\ecs\core\commands\migration_helpers.rs:728:22
//! site2 … dense       … panicked at crates\boyko_ecs\src\ecs\core\commands\migration_helpers.rs:728:22
//! ```
//!
//! Both `.expect` messages blamed the archetype-expansion contract ("the
//! archetype was expanded with every required id" / "target hosts every required
//! id") for a cause that has nothing to do with expansion: the id owns no pool to
//! expand into. Both messages are gone.
//!
//! **Site 2 had never been confirmed before that run.** R0's census measured site
//! 1 for both kinds with a throwaway spawn probe and recorded site 2 as "STILL
//! unconfirmed", warning that a spawn-only suite would pin site 1 twice and
//! report site 2 as covered. That trap outlived the panic: the two sites are
//! still two different calls on two different paths, and the spawn path still
//! never reaches site 2. Site 1 is reached only from `SpawnAtCommand::apply` /
//! `SpawnBatchCommand::apply`; site 2 only by an **insert into an existing
//! entity**, which migrates through `migrate_entity_insert` and never consults
//! the `BundleColumnCache`. Every dense assertion below is therefore made
//! through BOTH drivers.
//!
//! # What changed here, and why the file is no longer a measurement
//!
//! * The four `should_panic` tests are gone. The dense pair became POSITIVE
//!   tests: the required dense component must be **present AND correctly
//!   constructed**. Presence alone is not the gate — the ctor writes
//!   [`DENSE_CTOR_SENTINEL`], a value no zeroed slot and no `#[derive(Default)]`
//!   would produce, so a test that only checked `has_component` would pass over a
//!   store entry holding garbage.
//! * The bitset pair stopped being runtime panics at all: the derive refuses them
//!   at COMPILE time, so a `#[require(KFlag771)]` fixture would not let this file
//!   compile. They moved to a trybuild compile-fail fixture. What remains here is
//!   the **kernel defence** — `Component::HAS_REQUIRES` / `register_required` are
//!   public trait items and `install_required` is `pub`, so a HAND-WRITTEN `impl
//!   Component` still reaches the kernel arm, and that route is what
//!   [`kernel_refuses_a_hand_written_require_of_a_bitset_flag_at_site_1`] and its
//!   site-2 twin pin.
//!
//! # The controls are load-bearing — and now in a SECOND direction
//!
//! `table_required_id_*_does_not_panic` were never decoration. Their original job
//! was that a `should_panic` test passes on ANY panic, including one from a
//! broken fixture. Their job now is the mirror of that: a POSITIVE test passes
//! for any reason, including a fixture that never exercised the path. A green
//! control proves the same fixture shape drives a real required-component
//! construction for a `Table` id, so a dense assertion that passes is passing on
//! the mechanism and not on a typo.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::{Component, RequiredBuilder};
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Added, Mut, Query};
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::Entity;
use boyko_macros::{Bundle, Component};
use boyko_threadpool::ThreadPoolBuilder;

/// The value the DENSE required ctor writes.
///
/// Deliberately NOT zero and NOT `u32::default()`: the ctor is
/// `ptr::write(dst, KDense770::default())`, so a hand-written `Default` carrying
/// a sentinel is what separates "the constructor ran" from "the slot happens to
/// be zeroed" and from "a `#[derive(Default)]` produced the same bytes the
/// allocator did".
const DENSE_CTOR_SENTINEL: u32 = 0x00C0_FFEE;

/// Sentinel for the TABLE control's ctor — the same discrimination on the
/// control side, so a green control means the column really was constructed.
const TABLE_CTOR_SENTINEL: u32 = 0x00_7AB1;

/// Sentinel written by the transitive chain's DENSE leaf.
const DENSE_LEAF_SENTINEL: u32 = 0x0000_1EAF;

/// Sentinel written by the transitive chain's TABLE leaf.
const TABLE_LEAF_SENTINEL: u32 = 0x0000_7AB2;

/// Sentinel written by the transitive chain's DENSE middle.
const DENSE_MID_SENTINEL: u32 = 0x0000_D1D0;

/// A value written over a constructed dense component, to prove present⇒skip
/// does NOT re-construct it on a later insert.
const MUTATED_MARKER: u32 = 0x0000_BEEF;

// ════════════════════════════════════════════════════════════════════════════
// Required targets — one per storage kind
// ════════════════════════════════════════════════════════════════════════════

/// DENSE required target: owns a global `DenseStore`, is excluded from every
/// archetype signature, and therefore owns no per-archetype `ComponentPool`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
#[repr(C)]
struct KDense770 {
    x: u32,
}

impl Default for KDense770 {
    fn default() -> Self {
        Self {
            x: DENSE_CTOR_SENTINEL,
        }
    }
}

/// BITSET required target: an enable bit. Signature-excluded like the dense
/// target, and it owns no bytes at all — which is why `#[require]` of it is
/// refused rather than implemented. Reached here only through the hand-written
/// `impl Component` that bypasses the derive's compile-time refusal.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct KFlag771;

/// TABLE required target: the control kind. Owns a per-archetype pool, so both
/// sites resolve it through the unchanged `Table` arm.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
struct KTable772 {
    x: u32,
}

impl Default for KTable772 {
    fn default() -> Self {
        Self {
            x: TABLE_CTOR_SENTINEL,
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Requiring components
// ════════════════════════════════════════════════════════════════════════════

/// Requires the DENSE target.
#[derive(Component, Default, Clone, Copy)]
#[require(KDense770)]
#[repr(C)]
struct KReqDense773 {
    v: u32,
}

/// A SECOND requirer of the same dense target, so an entity that already holds
/// `KDense770` can take another require-bearing insert. This is what exercises
/// present⇒skip against the DENSE membership oracle rather than the table
/// signature.
#[derive(Component, Default, Clone, Copy)]
#[require(KDense770)]
#[repr(C)]
struct KReqDense774 {
    v: u32,
}

/// Requires the TABLE target (the control).
#[derive(Component, Default, Clone, Copy)]
#[require(KTable772)]
#[repr(C)]
struct KReqTable775 {
    v: u32,
}

/// Payload the entity already carries, so that an `insert` is a real MIGRATION
/// (`{Anchor}` → `{Anchor, Req, …}`) and not the in-place replace fast path —
/// the in-place path never reaches site 2.
#[derive(Component, Default, Clone, Copy)]
#[repr(C)]
struct KAnchor776 {
    v: u32,
}

// ── the transitive chain ────────────────────────────────────────────────────
// `KReqChain780` (table) → `KDenseMid777` (DENSE) → { `KDenseLeaf778` (DENSE),
// `KTableLeaf779` (table) }. `build_required_plan` flattens the closure before
// either walker sees it, so all three arrive as SIBLING entries in one flat
// slice and each is screened on its own storage kind. The route must therefore
// screen per entry and must NOT recurse.

/// DENSE leaf of the transitive chain.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
#[repr(C)]
struct KDenseLeaf778 {
    x: u32,
}

impl Default for KDenseLeaf778 {
    fn default() -> Self {
        Self {
            x: DENSE_LEAF_SENTINEL,
        }
    }
}

/// TABLE leaf of the transitive chain — a dense component requiring a TABLE one.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
struct KTableLeaf779 {
    x: u32,
}

impl Default for KTableLeaf779 {
    fn default() -> Self {
        Self {
            x: TABLE_LEAF_SENTINEL,
        }
    }
}

/// DENSE middle of the chain, itself requiring one dense and one table leaf.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(storage = "dense")]
#[require(KDenseLeaf778)]
#[require(KTableLeaf779)]
#[repr(C)]
struct KDenseMid777 {
    x: u32,
}

impl Default for KDenseMid777 {
    fn default() -> Self {
        Self {
            x: DENSE_MID_SENTINEL,
        }
    }
}

/// The chain's root — a plain TABLE component whose `#[require]` reaches a dense
/// component that has requirements of its own.
#[derive(Component, Default, Clone, Copy)]
#[require(KDenseMid777)]
#[repr(C)]
struct KReqChain780 {
    v: u32,
}

// ════════════════════════════════════════════════════════════════════════════
// Bundles
// ════════════════════════════════════════════════════════════════════════════

#[derive(Bundle, Clone, Copy)]
struct BDense {
    a: KReqDense773,
}

#[derive(Bundle, Clone, Copy)]
struct BDense2 {
    a: KReqDense774,
}

#[derive(Bundle, Clone, Copy)]
struct BTable {
    a: KReqTable775,
}

#[derive(Bundle, Clone, Copy)]
struct BChain {
    a: KReqChain780,
}

// ════════════════════════════════════════════════════════════════════════════
// Drivers — one per SITE, shared by every kind
// ════════════════════════════════════════════════════════════════════════════

/// Site 1 driver: a deferred `Commands::spawn`, which routes through
/// `SpawnAtCommand::apply` → `BundleColumnCache::resolve_and_cache` →
/// `resolve_required_missing`. The `BundleColumnCache` is consulted on the FIRST
/// spawn of a given `(Bundle, world)` pair, which is what this is.
fn spawn_bundle<B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    world.run_system(move |mut cmds: Commands| cmds.spawn(bundle).id())
}

/// Site 2 driver: spawn an anchored entity, then `insert` the require-bearing
/// bundle into it. The source archetype is `{KAnchor776}` and the target is
/// `{KAnchor776, Req…}` (+ the required id when it is signature storage), so the
/// insert is a genuine migration through `migrate_entity_insert` and reaches the
/// `has_requires` constructor pass. It does NOT consult the `BundleColumnCache`,
/// so site 1 cannot shadow the measurement.
///
/// Returns the entity — the pre-fix version discarded it, which is precisely
/// what a `should_panic` test could afford and a positive one cannot.
fn insert_bundle_into_anchored<
    B: boyko_ecs::ecs::core::bundle::Bundle + Copy + Send + Sync + 'static,
>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    let arch = world.create_archetype(&[KAnchor776::component_id()]);
    let e = world
        .spawn_one(arch, KAnchor776 { v: 7 })
        .expect("spawn the anchor entity");
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(bundle);
    });
    e
}

/// Reads `entity`'s DENSE component `C` back out of its global store.
///
/// `None` when the entity is not a member — which is the "the route never
/// constructed it" failure, distinct from "it constructed the wrong bytes".
fn read_dense<C: Component + Copy>(world: &EcsMaster, entity: Entity) -> Option<C> {
    let ptr = world.dense_get_raw(entity, C::component_id())?;
    // SAFETY: `dense_get_raw` resolved the store registered for
    //   `C::component_id()` and returned that store's row pointer for a LIVE
    //   slot, so it is aligned to `C` and valid for `size_of::<C>()` bytes. The
    //   `&EcsMaster` borrow keeps the column alive and forbids a concurrent
    //   structural mutation across the read. `entity` was minted by this test
    //   moments earlier and never despawned, so it is live and its generation
    //   matches — the accessor's one documented caller obligation (it is
    //   generation-blind on its own).
    Some(unsafe { *(ptr as *const C) })
}

/// Reads `entity`'s TABLE component `C`.
fn read_table<C: Component + Copy>(world: &EcsMaster, entity: Entity) -> Option<C> {
    let ptr = world.get_component_raw(entity, C::component_id())?;
    // SAFETY: `get_component_raw` validated liveness + generation and returned
    //   the archetype row pointer for `C`'s column in this entity's archetype,
    //   so it is aligned to `C` and valid for `size_of::<C>()` bytes; the
    //   `&EcsMaster` borrow keeps the pool alive across the read.
    Some(unsafe { *(ptr as *const C) })
}

// ════════════════════════════════════════════════════════════════════════════
// Controls — the same two drivers over a TABLE required id
// ════════════════════════════════════════════════════════════════════════════

/// Control for site 1. If this reds, every dense assertion below could be
/// passing on a fixture that never drove the required-component path at all.
#[test]
fn table_required_id_on_spawn_does_not_panic() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, BTable { a: KReqTable775 { v: 1 } });
    assert!(
        world.has_component(e, KTable772::component_id()),
        "control: a TABLE required id is constructed on the spawn path"
    );
    assert_eq!(
        read_table::<KTable772>(&world, e).map(|c| c.x),
        Some(TABLE_CTOR_SENTINEL),
        "control: the TABLE required column holds the CTOR's value, not zeroes"
    );
}

/// Control for site 2 — the same shape through the migration path.
#[test]
fn table_required_id_on_insert_does_not_panic() {
    let mut world = EcsMaster::new();
    let e = insert_bundle_into_anchored(&mut world, BTable { a: KReqTable775 { v: 2 } });
    assert!(
        world.has_component(e, KTable772::component_id()),
        "control: a TABLE required id is constructed on the INSERT path"
    );
    assert_eq!(
        read_table::<KTable772>(&world, e).map(|c| c.x),
        Some(TABLE_CTOR_SENTINEL),
        "control: the TABLE required column holds the CTOR's value on the insert path"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SITE 1 — `BundleColumnCache::resolve_required_missing` → SpawnAtCommand
// ════════════════════════════════════════════════════════════════════════════

/// `#[require]` of a DENSE component, spawned. Pre-fix this panicked at
/// `bundle_column_cache.rs:410` blaming the expansion contract.
#[test]
fn site1_spawn_constructs_the_required_dense_component() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, BDense { a: KReqDense773 { v: 1 } });

    assert!(
        world.has_component(e, KDense770::component_id()),
        "site 1: the required DENSE component must be a member of its store after spawn"
    );
    assert_eq!(
        read_dense::<KDense770>(&world, e),
        Some(KDense770 {
            x: DENSE_CTOR_SENTINEL
        }),
        "site 1: the store slot must hold the CTOR's value — presence over a zeroed \
         slot is the failure this assertion exists to catch"
    );
}

/// The same claim through `SpawnBatchCommand`, whose required pass is a separate
/// loop with its own sentinel screen and its own per-row dense commit.
#[test]
fn site1_batch_spawn_constructs_the_required_dense_component_for_every_row() {
    const N: u32 = 5;
    let mut world = EcsMaster::new();
    world.run_system(|mut cmds: Commands| {
        cmds.spawn_batch((0..N).map(|i| BDense {
            a: KReqDense773 { v: i },
        }))
        .expect("spawn_batch of N require-bearing bundles");
    });

    let seen = world.run_system(|q: Query<&KDense770>| {
        let mut n = 0usize;
        for d in &q {
            assert_eq!(
                d.x, DENSE_CTOR_SENTINEL,
                "batch: every constructed row must hold the CTOR's value, not zeroes"
            );
            n += 1;
        }
        n
    });
    assert_eq!(
        seen, N as usize,
        "batch: exactly one constructed dense member per spawned row"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// SITE 2 — the `migrate_entity_insert` required-ctor pass
// ════════════════════════════════════════════════════════════════════════════

/// `#[require]` of a DENSE component, INSERTED into an existing entity. This is
/// the site the spawn probe could never reach; pre-fix it panicked at
/// `migration_helpers.rs:728`.
#[test]
fn site2_insert_constructs_the_required_dense_component() {
    let mut world = EcsMaster::new();
    let e = insert_bundle_into_anchored(&mut world, BDense { a: KReqDense773 { v: 1 } });

    assert!(
        world.has_component(e, KDense770::component_id()),
        "site 2: the required DENSE component must be a member of its store after insert"
    );
    assert_eq!(
        read_dense::<KDense770>(&world, e),
        Some(KDense770 {
            x: DENSE_CTOR_SENTINEL
        }),
        "site 2: the store slot must hold the CTOR's value"
    );
    assert!(
        world.has_component(e, KAnchor776::component_id()),
        "site 2: the migration must not have lost the anchor payload"
    );
}

/// present⇒skip at site 2, against the **dense membership oracle**.
///
/// The pre-KE11 shape asked `src.component_ids().contains(&req_id)` — the TABLE
/// signature — which is simply the wrong oracle for a non-signature id. It
/// answers "absent" for an entity that IS a member, and the re-insert that
/// follows trips `DenseStore::insert`'s debug-asserted absence precondition (and
/// in release would silently overwrite the live value with a fresh default).
///
/// The discrimination is the MUTATION: after the first insert the value is
/// changed away from the ctor sentinel, and the second require-bearing insert
/// must leave it alone. A route that re-constructs would put the sentinel back.
#[test]
fn site2_second_insert_does_not_reconstruct_an_already_present_dense_requirement() {
    let mut world = EcsMaster::new();
    let e = insert_bundle_into_anchored(&mut world, BDense { a: KReqDense773 { v: 1 } });
    assert_eq!(
        read_dense::<KDense770>(&world, e).map(|c| c.x),
        Some(DENSE_CTOR_SENTINEL),
        "precondition: the first insert constructed it"
    );

    // Move the value off the ctor's output so a re-construct is visible.
    world.run_system(|mut q: Query<Mut<KDense770>>| {
        for mut d in &mut q {
            d.x = MUTATED_MARKER;
        }
    });
    assert_eq!(
        read_dense::<KDense770>(&world, e).map(|c| c.x),
        Some(MUTATED_MARKER),
        "precondition: the mutation landed"
    );

    // A SECOND require-bearing insert. `{Anchor, ReqDense773}` →
    // `{Anchor, ReqDense773, ReqDense774}` is a genuine migration, and
    // `KReqDense774`'s closure emits `KDense770` again.
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(BDense2 {
            a: KReqDense774 { v: 2 },
        });
    });

    assert_eq!(
        read_dense::<KDense770>(&world, e).map(|c| c.x),
        Some(MUTATED_MARKER),
        "present⇒skip: an already-present dense requirement keeps its value; seeing \
         the ctor sentinel here means the pass consulted the TABLE signature instead \
         of the dense store's membership map"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The transitive closure — screened PER ENTRY, never recursed
// ════════════════════════════════════════════════════════════════════════════

/// `build_required_plan` flattens the closure before either walker sees it, so a
/// required DENSE component that itself requires one dense and one table
/// component emits all three as SIBLING entries in one flat, DFS-ordered slice.
/// Each must be screened on its OWN storage kind: the two dense entries take the
/// dense arm, the table entry takes the untouched table arm.
#[test]
fn transitive_require_closure_mixes_dense_and_table_on_the_spawn_path() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, BChain { a: KReqChain780 { v: 1 } });

    assert_eq!(
        read_dense::<KDenseMid777>(&world, e).map(|c| c.x),
        Some(KDenseMid777::default().x),
        "chain: the DENSE middle is constructed"
    );
    assert_eq!(
        read_dense::<KDenseLeaf778>(&world, e).map(|c| c.x),
        Some(DENSE_LEAF_SENTINEL),
        "chain: the DENSE leaf pulled through a DENSE parent is constructed"
    );
    assert_eq!(
        read_table::<KTableLeaf779>(&world, e).map(|c| c.x),
        Some(TABLE_LEAF_SENTINEL),
        "chain: the TABLE leaf pulled through a DENSE parent still takes the table arm"
    );
}

/// The same chain through the insert path — site 2's screen is a separate match
/// on a separate walker, so it needs its own transitive assertion.
#[test]
fn transitive_require_closure_mixes_dense_and_table_on_the_insert_path() {
    let mut world = EcsMaster::new();
    let e = insert_bundle_into_anchored(&mut world, BChain { a: KReqChain780 { v: 1 } });

    assert_eq!(
        read_dense::<KDenseMid777>(&world, e).map(|c| c.x),
        Some(KDenseMid777::default().x),
        "chain (insert): the DENSE middle is constructed"
    );
    assert_eq!(
        read_dense::<KDenseLeaf778>(&world, e).map(|c| c.x),
        Some(DENSE_LEAF_SENTINEL),
        "chain (insert): the DENSE leaf is constructed"
    );
    assert_eq!(
        read_table::<KTableLeaf779>(&world, e).map(|c| c.x),
        Some(TABLE_LEAF_SENTINEL),
        "chain (insert): the TABLE leaf still takes the table arm"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The ordering claim — the commit is IMMEDIATE, the deferral is the queue's
// ════════════════════════════════════════════════════════════════════════════

/// `DenseStore::insert_with_ctor` writes the bytes, sets the live bit and stamps
/// BOTH ticks synchronously under `&mut self` — nothing is queued. The only
/// deferral in play is the command queue itself, which the required dense
/// component shares verbatim with the rest of the bundle.
///
/// So the observable contract is: after the queue drains, in the SAME frame, a
/// query sees the constructed component, and it is `Added` on exactly the frame
/// its requirer is — not one frame later.
#[test]
fn required_dense_is_added_on_the_same_frame_as_its_requirer() {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut world = EcsMaster::new();

    world.run_system(|mut cmds: Commands| {
        for i in 0..3u32 {
            cmds.spawn(BDense {
                a: KReqDense773 { v: i },
            });
        }
    });

    // Same frame, after the drain: the query already sees them.
    let live = world.run_system(|q: Query<&KDense770>| {
        let mut n = 0usize;
        for d in &q {
            assert_eq!(d.x, DENSE_CTOR_SENTINEL);
            n += 1;
        }
        n
    });
    assert_eq!(
        live, 3,
        "the constructed dense component is visible to a query in the same frame \
         the queue drained — the commit is immediate, not deferred"
    );

    static ADDED_REQUIRER: AtomicUsize = AtomicUsize::new(0);
    static ADDED_REQUIRED: AtomicUsize = AtomicUsize::new(0);
    ADDED_REQUIRER.store(0, Ordering::Relaxed);
    ADDED_REQUIRED.store(0, Ordering::Relaxed);

    let mut builder = ScheduleBuilder::new(Arc::clone(&pool));
    builder.add_system(|q: Query<&KReqDense773, Added<KReqDense773>>| {
        for _ in &q {
            ADDED_REQUIRER.fetch_add(1, Ordering::Relaxed);
        }
    });
    builder.add_system(|q: Query<&KDense770, Added<KDense770>>| {
        for _ in &q {
            ADDED_REQUIRED.fetch_add(1, Ordering::Relaxed);
        }
    });
    let mut schedule = builder.build(&mut world);

    schedule.run(&mut world);
    assert_eq!(
        ADDED_REQUIRER.load(Ordering::Relaxed),
        3,
        "frame 1: the requirer is Added"
    );
    assert_eq!(
        ADDED_REQUIRED.load(Ordering::Relaxed),
        3,
        "frame 1: the constructed dense requirement is Added on the SAME frame — \
         its slot ticks were stamped with the spawn's own tick"
    );

    ADDED_REQUIRER.store(0, Ordering::Relaxed);
    ADDED_REQUIRED.store(0, Ordering::Relaxed);
    schedule.run(&mut world);
    assert_eq!(
        ADDED_REQUIRED.load(Ordering::Relaxed),
        0,
        "frame 2: no inserts → the constructed dense requirement is no longer Added \
         (the tick was stamped once, not re-stamped)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The REUSED-SLOT arm — the half a spawn-only suite never reaches
// ════════════════════════════════════════════════════════════════════════════

/// A DENSE required target that OWNS HEAP, so the store's registered `drop_fn`
/// is a real destructor rather than a no-op. Every other fixture here is POD,
/// under which a double-drop and a drop-of-uninit are both invisible.
#[derive(Component, Clone, Debug, PartialEq)]
#[component(storage = "dense")]
struct KDenseOwned782 {
    tag: String,
}

impl Default for KDenseOwned782 {
    fn default() -> Self {
        Self {
            tag: String::from("ke11-owned-ctor"),
        }
    }
}

/// Requires the heap-owning dense target.
#[derive(Component, Default, Clone)]
#[require(KDenseOwned782)]
#[repr(C)]
struct KReqOwned783 {
    v: u32,
}

#[derive(Bundle, Clone)]
struct BOwned {
    a: KReqOwned783,
}

/// `DenseStore::insert_with_ctor` has TWO arms, and only one of them runs on a
/// world that never despawns: the fresh-slot append. The other pops a TOMBSTONED
/// slot off the LIFO free list and constructs into bytes a previous tenant used
/// to own — the arm whose `// SAFETY:` (U1) claims the slot is *logically
/// uninitialised* because `remove` already ran `drop_at`.
///
/// That claim is unfalsifiable over POD. This test forces the pop with a
/// heap-owning payload, so under Miri a stale `String` left undropped, a
/// double-drop of the prior tenant's buffer, or a ctor writing over a live value
/// all surface as real errors instead of passing silently.
#[test]
fn reused_dense_slot_constructs_a_required_component_without_touching_the_old_value() {
    let mut world = EcsMaster::new();

    let first = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOwned {
            a: KReqOwned783 { v: 1 },
        })
        .id()
    });
    let _second = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOwned {
            a: KReqOwned783 { v: 2 },
        })
        .id()
    });

    // Tombstone `first`'s slot: the despawn walk drops its `String` and pushes
    // the slot onto the free list.
    world.run_system(move |mut cmds: Commands| {
        cmds.despawn(first);
    });
    assert!(
        !world.has_component(first, KDenseOwned782::component_id()),
        "precondition: the despawn removed the dense membership"
    );

    // A third spawn: the required ctor now takes the REUSED-slot arm.
    let third = world.run_system(|mut cmds: Commands| {
        cmds.spawn(BOwned {
            a: KReqOwned783 { v: 3 },
        })
        .id()
    });
    assert!(
        world.has_component(third, KDenseOwned782::component_id()),
        "the reused slot must carry the new tenant's membership"
    );

    let live = world.run_system(|q: Query<&KDenseOwned782>| {
        let mut n = 0usize;
        for d in &q {
            assert_eq!(
                d.tag, "ke11-owned-ctor",
                "the reused slot holds a freshly CONSTRUCTED value, not the prior \
                 tenant's leftovers"
            );
            n += 1;
        }
        n
    });
    assert_eq!(
        live, 2,
        "two live members remain (the second entity plus the third, which took the \
         freed slot) — a third would mean the free list was not reused"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// BITSET — the KERNEL defence behind the derive's compile-time refusal
// ════════════════════════════════════════════════════════════════════════════

/// A HAND-WRITTEN `impl Component` that declares `#[require]`-equivalent state
/// for a BITSET flag, bypassing the derive entirely.
///
/// This is not a hypothetical route: `Component::HAS_REQUIRES` and
/// `Component::register_required` are public trait items and
/// `component_registry::install_required` is `pub`, so the derive's const-assert
/// closes the derive door and nothing else. Both kernel sites keep a `#[cold]`
/// refusal for exactly this, and these two tests are what keep that refusal from
/// rotting into an unreachable branch nobody notices is gone.
#[derive(Clone, Copy)]
struct KHandRolledReqFlag781;

impl Component for KHandRolledReqFlag781 {
    const HAS_REQUIRES: bool = true;

    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| {
            let raw = component_registry::register_new::<Self>();
            component_registry::install_required::<Self>(raw);
            ComponentId(raw)
        })
    }

    fn register_required(builder: &mut RequiredBuilder) {
        // `KFlag771::component_id` is passed as a fn ITEM, uncalled — the
        // BUG-REQ-CYCLE-1 discipline the derive follows.
        builder.require(KFlag771::component_id, hand_rolled_flag_ctor);
    }
}

/// A ctor for a component that has no bytes. It cannot do anything, which is the
/// entire argument for refusing the declaration rather than honouring it.
///
/// # Safety
/// Vacuous: the body writes nothing, so no destination invariant is relied on.
unsafe fn hand_rolled_flag_ctor(_dst: *mut u8) {}

#[derive(Bundle, Clone, Copy)]
struct BHandFlag {
    a: KHandRolledReqFlag781,
}

/// Site 1's `Bitset` arm. The expectation pins the SITE PREFIX, not the shared
/// body of the message — the two sites feed the same `required_bitset_panic` and
/// a shared substring would let either test pass on the other's panic, which is
/// the discrimination the pre-fix file was careful to keep.
#[test]
#[should_panic(expected = "BundleColumnCache::resolve_required_missing: #[require] of a BITSET flag")]
fn kernel_refuses_a_hand_written_require_of_a_bitset_flag_at_site_1() {
    let mut world = EcsMaster::new();
    let _ = spawn_bundle(&mut world, BHandFlag {
        a: KHandRolledReqFlag781,
    });
}

/// Site 2's `Bitset` arm — a separate match on a separate walker, so it needs
/// its own test for the same reason every other claim here is made twice.
#[test]
#[should_panic(
    expected = "migrate_entity_insert (required-component constructor pass): #[require] of a BITSET flag"
)]
fn kernel_refuses_a_hand_written_require_of_a_bitset_flag_at_site_2() {
    let mut world = EcsMaster::new();
    let _ = insert_bundle_into_anchored(&mut world, BHandFlag {
        a: KHandRolledReqFlag781,
    });
}
