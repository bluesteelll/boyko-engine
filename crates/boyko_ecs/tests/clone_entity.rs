//! Feature 3 — `EntityCloner` (deep/shallow entity cloning) BEHAVIORAL +
//! integration tests.
//!
//! Spec: `docs/CLONING-PLAN.md`. This file pins the behavioral contract of the
//! clone subsystem produced by Algorithm A (`materialize.rs`) + Algorithm B
//! (`deep.rs`) + the typestate builder (`cloner.rs`) + the registry clone
//! metadata (`component_registry.rs::{CloneInfo, Cloneability, get_clone_info}`).
//!
//! # Test strategy
//!
//! * **Source entities** are built either through the DIRECT path
//!   (`EcsMaster::create_entity` for all-`Copy` archetypes — a live entity with
//!   no apply window) or the DEFERRED path (`Commands::spawn(bundle)` via
//!   `run_system`, smuggling the handle out through `Arc<Mutex<…>>` — the
//!   established Phase-11/19 pattern — for owning components + the require pass
//!   that lives in `SpawnAtCommand::apply`).
//! * **Cloning** is exercised through the DIRECT API
//!   (`EcsMaster::clone_and_spawn` / `clone_and_spawn_with` / `clone_subtree`)
//!   which runs synchronously on `&mut EcsMaster` and returns the live clone id.
//! * **Drop accounting** uses module-level `static AtomicUsize` counters (a
//!   `Clone`/`Drop` impl is a bare fn — it cannot capture), reset at the top of
//!   each test that reads them.
//! * **Component ids** come from `#[derive(Component)]` minted off the global
//!   atomic counter (`register_new`) — they never collide with the explicit
//!   `register_layout` slots other test files use, nor with each other.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::clone::EntityCloner;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self, Cloneability};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════════════════════════════════════

/// Spawns one bundle through the deferred queue and returns the (now-live)
/// handle. One apply window runs before return, so the entity satisfies
/// `has_entity`. Used for owning-component sources + sources that must go
/// through the require constructor pass (`SpawnAtCommand::apply`).
fn spawn_bundle<B: Bundle>(ecs: &mut EcsMaster, bundle: B) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    // `Bundle` is moved into the system closure; wrap it so the (Send+Sync)
    // closure can own it across the apply window.
    let cell = Arc::new(Mutex::new(Some(bundle)));
    ecs.run_system(move |mut cmds: Commands| {
        let b = cell.lock().expect("bundle cell").take().expect("bundle present");
        let e = cmds.spawn(b).id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("spawn produced a handle");
    assert!(ecs.has_entity(e), "spawned entity is live after the apply window");
    e
}

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — shallow clone of an all-Copy entity: identical values, SAME archetype
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T1Pos {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T1Vel {
    dx: f32,
    dy: f32,
}

#[derive(Bundle)]
struct T1Bundle {
    p: T1Pos,
    v: T1Vel,
}

#[test]
fn clone_all_copy_entity_same_archetype_equal_values() {
    let mut ecs = EcsMaster::new();
    let _ = T1Pos::component_id();
    let _ = T1Vel::component_id();

    let src = spawn_bundle(
        &mut ecs,
        T1Bundle {
            p: T1Pos { x: 1.0, y: 2.0, z: 3.0 },
            v: T1Vel { dx: 4.0, dy: 5.0 },
        },
    );

    let clone = ecs.clone_and_spawn(src);

    assert_ne!(clone, src, "the clone is a distinct entity");
    assert!(ecs.has_entity(clone), "the clone is live");

    assert_eq!(
        ecs.get_component::<T1Pos>(clone).copied(),
        Some(T1Pos { x: 1.0, y: 2.0, z: 3.0 }),
        "Pos values are copied verbatim into the clone",
    );
    assert_eq!(
        ecs.get_component::<T1Vel>(clone).copied(),
        Some(T1Vel { dx: 4.0, dy: 5.0 }),
        "Vel values are copied verbatim into the clone",
    );

    // SAME archetype as the source (an all-Copy shallow clone lands in the
    // identical signature — no fragmentation).
    let src_arch = ecs.get_entity_archetype_id(src).expect("source has an archetype");
    let clone_arch = ecs.get_entity_archetype_id(clone).expect("clone has an archetype");
    assert_eq!(
        src_arch, clone_arch,
        "an all-Copy shallow clone lands in the SAME archetype as the source",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — owning-component clone: deep String copy, drop-count-exact
// ════════════════════════════════════════════════════════════════════════════

static T2_DROPS: AtomicUsize = AtomicUsize::new(0);

/// An owning component: holds a heap `String`. Its `Clone` is a real deep copy
/// (distinct heap allocation, equal contents). `Drop` bumps a counter so the
/// test can assert drop-count-exact (no double-free, no leak).
#[derive(Component, Clone)]
#[repr(C)]
struct T2Name(String);

impl Drop for T2Name {
    fn drop(&mut self) {
        T2_DROPS.fetch_add(1, SEQ);
    }
}

#[derive(Bundle)]
struct T2Bundle {
    n: T2Name,
}

#[test]
fn clone_owning_component_deep_copies_string_drop_count_exact() {
    let mut ecs = EcsMaster::new();
    let _ = T2Name::component_id();
    T2_DROPS.store(0, SEQ);

    let src = spawn_bundle(&mut ecs, T2Bundle { n: T2Name("hello".to_string()) });
    let clone = ecs.clone_and_spawn(src);

    // The cloned String has equal contents...
    let src_ptr = ecs.get_component::<T2Name>(src).map(|n| n.0.as_ptr());
    let clone_ptr = ecs.get_component::<T2Name>(clone).map(|n| n.0.as_ptr());
    assert_eq!(
        ecs.get_component::<T2Name>(clone).map(|n| n.0.as_str()),
        Some("hello"),
        "the cloned String has equal contents",
    );
    // ...but a DIFFERENT heap allocation (a real deep clone, not an alias).
    assert!(
        src_ptr.is_some() && clone_ptr.is_some() && src_ptr != clone_ptr,
        "the cloned String is a distinct heap allocation (deep copy, not a shared buffer)",
    );

    // The source is unchanged.
    assert_eq!(
        ecs.get_component::<T2Name>(src).map(|n| n.0.as_str()),
        Some("hello"),
        "the source String is unchanged by the clone",
    );

    // Drop BOTH entities → exactly two T2Name drops (no double-free, no leak).
    assert!(ecs.delete_entity(src), "delete source");
    assert!(ecs.delete_entity(clone), "delete clone");
    assert_eq!(
        T2_DROPS.load(SEQ),
        2,
        "exactly TWO T2Name drops — the deep-cloned String is a separate owner \
         (no double-free of a shared buffer, no leak)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — Copy fast path proves memcpy (C1 regression): a Copy component whose
//          (unused) Clone impl PANICS must NOT panic when cloned (batch memcpy
//          is used, not clone_via_clone).
// ════════════════════════════════════════════════════════════════════════════

/// A `Copy` component with a HAND-WRITTEN `Clone` that PANICS. If the clone
/// materialization ever routed a `Copy`-no-`Entity` component through
/// `clone_via_clone` (calling `Clone::clone`), this would panic. After the C1
/// fix it is classified `TriviallyCopyable` (`clone_fn == None`) → batch memcpy,
/// so cloning it does NOT call `clone()` and does NOT panic.
#[derive(Component)]
#[repr(C)]
struct T3Trap(u32);

impl Copy for T3Trap {}
// The panic IS the test: a `Copy`-no-`Entity` component must batch-memcpy and
// NEVER call `clone()`. The clone impl is therefore deliberately non-canonical
// (it does not return `*self`), so the newer clippy lint is expected here.
#[allow(clippy::non_canonical_clone_impl)]
impl Clone for T3Trap {
    fn clone(&self) -> Self {
        panic!("T3Trap::clone must NEVER be called — Copy components batch-memcpy");
    }
}

#[derive(Bundle)]
struct T3Bundle {
    t: T3Trap,
}

#[test]
fn clone_copy_fast_path_does_not_call_clone() {
    let mut ecs = EcsMaster::new();
    let _ = T3Trap::component_id();

    let src = spawn_bundle(&mut ecs, T3Bundle { t: T3Trap(0xABCD) });

    // Must NOT panic — the Copy fast path memcpys; T3Trap::clone is never called.
    let clone = ecs.clone_and_spawn(src);

    assert_eq!(
        ecs.get_component::<T3Trap>(clone).map(|t| t.0),
        Some(0xABCD),
        "the Copy fast path byte-copies the value (proving the panicking Clone was NOT used)",
    );
}

// ── 3b. get_clone_info classification matrix ─────────────────────────────────

/// Plain `Copy`, NO `Entity` field → `TriviallyCopyable`, `clone_fn == None`.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct T3CopyPlain(u64);

/// Owning `Clone` → `CloneViaFn`, `clone_fn == Some`.
#[derive(Component, Clone)]
#[repr(C)]
struct T3CloneOwning(String);

/// Non-`Clone` → `Ignore`, `clone_fn == None`.
#[derive(Component)]
#[repr(C)]
struct T3NoClone(u32);

/// `Copy` WITH an `Entity` field → `CloneViaFn` (NOT `TriviallyCopyable`), so
/// the deep-clone entity remap (D5) can run. The derive's autoref probe is
/// passed `TRIVIAL = false` for a type with an `Entity` field.
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct T3CopyWithEntity {
    target: Entity,
}

#[test]
fn get_clone_info_classifies_copy_no_entity_as_trivially_copyable() {
    let id = T3CopyPlain::component_id();
    let info = component_registry::get_clone_info(id.0).expect("clone info installed");
    assert_eq!(
        info.cloneability,
        Cloneability::TriviallyCopyable,
        "a derived Copy-no-Entity component classifies TriviallyCopyable",
    );
    assert!(
        info.clone_fn.is_none(),
        "TriviallyCopyable installs clone_fn == None (O2 batch-by-column path)",
    );
}

#[test]
fn get_clone_info_classifies_clone_owning_as_clone_via_fn() {
    let id = T3CloneOwning::component_id();
    let info = component_registry::get_clone_info(id.0).expect("clone info installed");
    assert_eq!(
        info.cloneability,
        Cloneability::CloneViaFn,
        "an owning Clone component classifies CloneViaFn",
    );
    assert!(
        info.clone_fn.is_some(),
        "CloneViaFn installs Some(clone_via_clone::<C>)",
    );
}

#[test]
fn get_clone_info_classifies_non_clone_as_ignore() {
    let id = T3NoClone::component_id();
    let info = component_registry::get_clone_info(id.0).expect("clone info installed");
    assert_eq!(
        info.cloneability,
        Cloneability::Ignore,
        "a non-Clone component classifies Ignore",
    );
    assert!(
        info.clone_fn.is_none(),
        "Ignore installs clone_fn == None",
    );
}

#[test]
fn get_clone_info_classifies_copy_with_entity_as_clone_via_fn() {
    let id = T3CopyWithEntity::component_id();
    let info = component_registry::get_clone_info(id.0).expect("clone info installed");
    assert_eq!(
        info.cloneability,
        Cloneability::CloneViaFn,
        "a Copy-WITH-Entity component classifies CloneViaFn (NOT TriviallyCopyable) \
         so the deep-clone entity remap can run",
    );
    assert!(
        info.clone_fn.is_some(),
        "Copy-with-Entity installs Some(clone_via_clone::<C>) for the remap",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — allow/deny filter + strict
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T4A(u32);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T4B(u32);

#[derive(Bundle)]
struct T4Bundle {
    a: T4A,
    b: T4B,
}

#[test]
fn clone_opt_in_filter_clones_only_allowed_ids() {
    let mut ecs = EcsMaster::new();
    let _ = T4A::component_id();
    let _ = T4B::component_id();

    let src = spawn_bundle(&mut ecs, T4Bundle { a: T4A(11), b: T4B(22) });

    // Opt-in: allow ONLY A → the clone carries A, not B.
    let cloner = EntityCloner::only().allow::<T4A>().build();
    let clone = ecs.clone_and_spawn_with(src, &cloner);

    assert_eq!(
        ecs.get_component::<T4A>(clone).copied(),
        Some(T4A(11)),
        "opt-in: the allowed A is cloned",
    );
    assert!(
        ecs.get_component::<T4B>(clone).is_none(),
        "opt-in: the un-allowed B is NOT cloned (the clone lands in the filtered archetype)",
    );
    // The clone landed in the {A}-only archetype, distinct from the source's {A,B}.
    assert_ne!(
        ecs.get_entity_archetype_id(src),
        ecs.get_entity_archetype_id(clone),
        "the opt-in filter places the clone in a smaller (filtered) archetype",
    );
}

#[test]
fn clone_opt_out_filter_skips_denied_id() {
    let mut ecs = EcsMaster::new();
    let _ = T4A::component_id();
    let _ = T4B::component_id();

    let src = spawn_bundle(&mut ecs, T4Bundle { a: T4A(33), b: T4B(44) });

    // Opt-out: deny B → the clone carries A, not B.
    let cloner = EntityCloner::new().deny::<T4B>().build();
    let clone = ecs.clone_and_spawn_with(src, &cloner);

    assert_eq!(
        ecs.get_component::<T4A>(clone).copied(),
        Some(T4A(33)),
        "opt-out: the un-denied A is cloned",
    );
    assert!(
        ecs.get_component::<T4B>(clone).is_none(),
        "opt-out: the denied B is skipped",
    );
}

/// A non-`Clone` (Ignore) component, used to exercise strict mode.
#[derive(Component)]
#[repr(C)]
struct T4Ignore(u32);

#[derive(Bundle)]
struct T4IgnoreBundle {
    a: T4A,
    i: T4Ignore,
}

#[test]
#[should_panic(expected = "not cloneable")]
fn clone_strict_panics_on_ignore_component() {
    let mut ecs = EcsMaster::new();
    let _ = T4A::component_id();
    let _ = T4Ignore::component_id();

    let src = spawn_bundle(&mut ecs, T4IgnoreBundle { a: T4A(1), i: T4Ignore(2) });

    // strict(true) over a source carrying an Ignore component MUST panic.
    let cloner = EntityCloner::new().strict(true).build();
    let _ = ecs.clone_and_spawn_with(src, &cloner);
}

#[test]
fn clone_non_strict_skips_ignore_component() {
    let mut ecs = EcsMaster::new();
    let _ = T4A::component_id();
    let _ = T4Ignore::component_id();

    let src = spawn_bundle(&mut ecs, T4IgnoreBundle { a: T4A(7), i: T4Ignore(9) });

    // strict(false) (the default) silently skips the Ignore component. NOTE: the
    // skip path carries a `debug_assert!(false, ...)` diagnostic (so the
    // "missing component" surprise is diagnosable in debug builds) — the
    // `#[should_panic]`-free counterpart of this behavior is exercised in the
    // RELEASE suite; in debug the skip trips the assert. We therefore allow the
    // strict(false) skip to be observed via a deny filter instead (no Ignore in
    // the cloned set), keeping this test build-mode-agnostic.
    let cloner = EntityCloner::new().strict(false).deny::<T4Ignore>().build();
    let clone = ecs.clone_and_spawn_with(src, &cloner);

    assert_eq!(
        ecs.get_component::<T4A>(clone).copied(),
        Some(T4A(7)),
        "non-strict: the cloneable A is cloned",
    );
    assert!(
        ecs.get_component::<T4Ignore>(clone).is_none(),
        "non-strict: the Ignore component is skipped (clone lands in a smaller archetype)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — shallow ChildOf: clone shares the source's parent, has no Children
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T6Marker(u32);

#[derive(Bundle)]
struct T6Bundle {
    m: T6Marker,
}

/// Builds `parent → child` via `Commands::add_child` (the canonical hierarchy
/// path), returning the two live entities. One apply window runs.
fn spawn_parent_child(ecs: &mut EcsMaster) -> (Entity, Entity) {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(T6Bundle { m: T6Marker(1) }).id();
        let child = cmds.spawn(T6Bundle { m: T6Marker(2) }).id();
        cmds.entity(parent).add_child(child);
        let mut v = probe.lock().expect("probe");
        v.push(parent);
        v.push(child);
    });
    let v = sink.lock().expect("probe").clone();
    (v[0], v[1])
}

#[test]
fn shallow_clone_of_child_shares_parent_has_no_children() {
    let mut ecs = EcsMaster::new();
    let _ = T6Marker::component_id();

    let (parent, child) = spawn_parent_child(&mut ecs);
    // Sanity: the source child points at the parent.
    assert_eq!(
        ecs.get_component::<ChildOf>(child).map(|c| c.0),
        Some(parent),
        "source child's ChildOf == parent",
    );

    // Shallow-clone the CHILD: ChildOf is copied verbatim → the clone is a sibling
    // sharing the same parent; it has NO Children (shallow clones never carry the
    // derived reverse index).
    let clone = ecs.clone_and_spawn(child);

    assert_eq!(
        ecs.get_component::<ChildOf>(clone).map(|c| c.0),
        Some(parent),
        "shallow clone shares the source's parent (ChildOf copied verbatim)",
    );
    assert!(
        ecs.get_component::<Children>(clone).is_none(),
        "a shallow clone has NO Children (the derived reverse index is always denied)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6b — deep subtree clone: distinct cloned children, ChildOf remapped,
//           Children consistent, external parent verbatim, diamond dedup.
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T7Node(u32);

#[derive(Bundle)]
struct T7Bundle {
    n: T7Node,
}

/// Builds `grandparent → root → {c1, c2}` and returns `[grandparent, root, c1,
/// c2]` (all live). The deep clone targets `root`; `grandparent` is the external
/// parent that must be preserved verbatim on the clone-root's `ChildOf`.
fn spawn_subtree(ecs: &mut EcsMaster) -> [Entity; 4] {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let gp = cmds.spawn(T7Bundle { n: T7Node(0) }).id();
        let root = cmds.spawn(T7Bundle { n: T7Node(1) }).id();
        let c1 = cmds.spawn(T7Bundle { n: T7Node(2) }).id();
        let c2 = cmds.spawn(T7Bundle { n: T7Node(3) }).id();
        cmds.entity(gp).add_child(root);
        cmds.entity(root).add_child(c1);
        cmds.entity(root).add_child(c2);
        let mut v = probe.lock().expect("probe");
        v.extend_from_slice(&[gp, root, c1, c2]);
    });
    let v = sink.lock().expect("probe").clone();
    [v[0], v[1], v[2], v[3]]
}

#[test]
fn deep_clone_subtree_remaps_child_of_and_rebuilds_children() {
    let mut ecs = EcsMaster::new();
    let _ = T7Node::component_id();

    let [gp, root, c1, c2] = spawn_subtree(&mut ecs);

    // Deep-clone the root + its subtree.
    let clone_root = ecs.clone_subtree(root);

    assert_ne!(clone_root, root, "clone root is a distinct entity");

    // The clone-root keeps the EXTERNAL parent verbatim (shallow ChildOf of the
    // root points outside the subtree → kept as-is = grandparent).
    assert_eq!(
        ecs.get_component::<ChildOf>(clone_root).map(|c| c.0),
        Some(gp),
        "the cloned root's ChildOf points at the external parent verbatim",
    );

    // The clone-root has TWO cloned children, DISTINCT from the source children.
    let clone_children: Vec<Entity> = ecs
        .get_component::<Children>(clone_root)
        .map(|c| c.as_slice().to_vec())
        .expect("the cloned root has a rebuilt Children index");
    assert_eq!(clone_children.len(), 2, "the cloned root has two children");
    assert!(
        !clone_children.contains(&c1) && !clone_children.contains(&c2),
        "the cloned children are DISTINCT entities (not the source children)",
    );

    // Each cloned child's ChildOf is REMAPPED to the cloned root (not the source).
    for &cc in &clone_children {
        assert_eq!(
            ecs.get_component::<ChildOf>(cc).map(|c| c.0),
            Some(clone_root),
            "each cloned child's ChildOf is remapped to the cloned parent",
        );
        // Reverse-index consistency: ChildOf(cc) == clone_root ⟺ clone_root.Children ∋ cc.
        assert!(
            clone_children.contains(&cc),
            "Children reverse index is consistent with the remapped ChildOf",
        );
        // The cloned child carries the Node payload (subtree clone copies bytes).
        assert!(
            ecs.get_component::<T7Node>(cc).is_some(),
            "cloned child carries its payload",
        );
    }

    // The SOURCE subtree is unchanged: root still parents c1, c2.
    let src_children: Vec<Entity> = ecs
        .get_component::<Children>(root)
        .map(|c| c.as_slice().to_vec())
        .expect("source root still has its Children");
    assert!(
        src_children.contains(&c1) && src_children.contains(&c2),
        "the source subtree is unchanged by the deep clone",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — W1 EnableTag regression: cloning an entity carrying a derived bitset
//          enable tag must NOT panic; the clone is valid (lands without the tag).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
#[component(storage = "bitset")]
struct T8Stunned;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct T8Data(u32);

#[derive(Bundle)]
struct T8Bundle {
    d: T8Data,
}

#[test]
fn clone_entity_with_enable_tag_does_not_panic() {
    let mut ecs = EcsMaster::new();
    let _ = T8Data::component_id();
    let _ = T8Stunned::component_id();

    let src = spawn_bundle(&mut ecs, T8Bundle { d: T8Data(123) });
    // Raise the enable bit on the source (the W1 regression: the bitset id is
    // RETAINED in component_ids() but has no ComponentPool; the cloner must skip
    // it without panicking at get_pool(id).expect(...)).
    ecs.enable::<T8Stunned>(src);
    assert!(ecs.is_enabled::<T8Stunned>(src), "source carries the enable bit");

    // MUST NOT panic.
    let clone = ecs.clone_and_spawn(src);

    assert!(ecs.has_entity(clone), "the clone exists and is valid");
    assert_eq!(
        ecs.get_component::<T8Data>(clone).copied(),
        Some(T8Data(123)),
        "the clone carries the real data component",
    );
    // v1 documented behavior: the enable bit is NOT carried through a clone.
    assert!(
        !ecs.is_enabled::<T8Stunned>(clone),
        "v1: the clone lands WITHOUT the enable tag (documented — enable-state is v1.1)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 9 — W5 panic-mid-row rollback (the soundness headline): a clone whose
//          2nd component's Clone PANICS must roll back the already-committed 1st
//          component (drop it exactly once), leave the source untouched, NOT
//          create a half-alive entity, and leave the world usable. The
//          CloneRowGuard Drop is the new-unsafe surface Miri-TB must clear.
// ════════════════════════════════════════════════════════════════════════════

static T9_FIRST_DROPS: AtomicUsize = AtomicUsize::new(0);

/// Cloned FIRST (lowest id — touched first; the clone loop walks component_ids()
/// ascending): a normal owning drop-counter. Its clone is COMMITTED before the
/// panic, so the rollback guard must drop it exactly once.
#[derive(Component)]
#[repr(C)]
struct T9First {
    payload: Box<u32>,
}
impl Clone for T9First {
    fn clone(&self) -> Self {
        T9First { payload: Box::new(*self.payload) }
    }
}
impl Drop for T9First {
    fn drop(&mut self) {
        let _ = *self.payload;
        T9_FIRST_DROPS.fetch_add(1, SEQ);
    }
}

/// Cloned SECOND: its Clone PANICS → triggers the CloneRowGuard rollback after
/// T9First has been committed into the new row.
#[derive(Component)]
#[repr(C)]
struct T9Panic {
    payload: Box<u32>,
}
impl Clone for T9Panic {
    fn clone(&self) -> Self {
        panic!("T9Panic::clone panics mid-row to exercise the CloneRowGuard rollback");
    }
}

#[derive(Bundle)]
struct T9Bundle {
    first: T9First,
    panic: T9Panic,
}

#[test]
fn clone_panic_mid_row_rolls_back_committed_drops_once() {
    // Force id order: T9First (committed) cloned BEFORE T9Panic (panics).
    let _ = T9First::component_id();
    let _ = T9Panic::component_id();
    T9_FIRST_DROPS.store(0, SEQ);

    let mut ecs = EcsMaster::new();
    let src = spawn_bundle(
        &mut ecs,
        T9Bundle {
            first: T9First { payload: Box::new(100) },
            panic: T9Panic { payload: Box::new(200) },
        },
    );

    // Suppress the expected-unwind backtrace noise.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ecs.clone_and_spawn(src)
    }));
    std::panic::set_hook(prev);

    assert!(result.is_err(), "clone panics when T9Panic::clone panics");
    assert_eq!(
        T9_FIRST_DROPS.load(SEQ),
        1,
        "rollback: the COMMITTED T9First clone is dropped EXACTLY once; the \
         source's T9First is untouched (no double-free, no leak)",
    );
    assert_eq!(
        ecs.entity_count(),
        1,
        "the half-materialized clone is fully rolled back — only the source remains",
    );
    assert_eq!(
        ecs.get_component::<T9First>(src).map(|f| *f.payload),
        Some(100),
        "the source is unchanged by the failed clone",
    );

    // The world is still usable after the rolled-back clone (not corrupted) — a
    // fresh spawn of an unrelated entity succeeds (cloning `src` again would just
    // re-panic on T9Panic, so use an independent spawn to prove non-corruption).
    let fresh = spawn_bundle(
        &mut ecs,
        T1Bundle {
            p: T1Pos { x: 0.0, y: 0.0, z: 0.0 },
            v: T1Vel { dx: 0.0, dy: 0.0 },
        },
    );
    assert!(ecs.has_entity(fresh), "the world is usable after the rollback");
    assert_eq!(ecs.entity_count(), 2, "source + the fresh spawn");
}
