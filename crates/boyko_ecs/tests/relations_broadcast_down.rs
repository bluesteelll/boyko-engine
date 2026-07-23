//! Down-broadcast triggers — `PropagationMode::Down` fan-out over a `Broadcast`
//! relation's reverse collection (Decision 6 / critic W4), plus the MANDATORY
//! scaling-guard (reviewer A2).
//!
//! A `Down` trigger fired at `root` fans out to `root` + every descendant via
//! `E::Broadcast`'s reverse index (an explicit-stack DFS). This suite pins:
//!
//! * B — every node fires exactly once; `propagate(false)` at a MID-TREE node
//!   prunes ONLY that node's subtree (siblings + their descendants still fire);
//!   `Up` / `None` triggers are UNCHANGED.
//! * C — the `≤ C·N` fire-count bound on deep / wide / balanced trees for
//!   N ∈ {10, 100, 1000}; cyclic termination (each node ≤ once) for a
//!   `!ACYCLIC` broadcast relation; bounded termination (depth cap) for an
//!   `ACYCLIC`-mislabeled cycle (built over `ChildOf`, the only `ACYCLIC = true`
//!   relation the public API exposes — the `#[derive(Relationship)]` macro does
//!   not emit an `acyclic` attribute, so a derived relation is always
//!   `ACYCLIC = false`).
//!
//! # Why `static` counters
//!
//! A `TriggerFn` is a bare `unsafe fn` pointer — it cannot capture; each test
//! owns private module-level `static` counters and its own trigger type. The
//! per-entity fire bookkeeping uses a `Mutex<Vec<usize>>` keyed by id so the
//! prune-isolation assertion can name exactly which nodes fired.
//!
//! Mirrors `relations_bubbling.rs` (the `Toward<R>` bubble) and
//! `feature2_observers_behavioral.rs` (the `propagate(false)` stop pattern),
//! retargeted to the `Down` fan-out.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::propagate::propagate;
use boyko_ecs::ecs::core::component::observers::traversal::{ChildOfTraversal, PropagationMode};
use boyko_ecs::ecs::core::component::observers::trigger::{Trigger, TriggerContext, TriggerFn};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::relationship::Relationship;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship as RelationshipDerive, RelationshipTarget};

const SEQ: Ordering = Ordering::SeqCst;

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// Spawns `n` markers; returns now-live handles (one apply window).
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(Tag(i as u32)).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    for &e in &out {
        assert!(ecs.has_entity(e), "spawned live after apply");
    }
    out
}

// ════════════════════════════════════════════════════════════════════════════
// B.1 — Down reaches root + ALL descendants, each fired exactly once
// ════════════════════════════════════════════════════════════════════════════

/// A `Down` broadcast over `ChildOf` — fans out to every transitive child.
struct DownAll;
impl Trigger for DownAll {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

// Per-entity fire record (id -> count) for B.1 / B.2.
fn b1_fires() -> &'static Mutex<Vec<usize>> {
    static F: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(Vec::new()))
}
unsafe fn b1_record(_w: DeferredEcsMaster<'_>, ctx: TriggerContext, _e: *const u8) {
    b1_fires().lock().expect("lock").push(ctx.target.id().0);
}

/// Builds `root → {a, b}`, `a → {a1, a2}`, `b → {b1}` (ChildOf), all live.
/// Returns the six handles `[root, a, b, a1, a2, b1]`.
fn build_asymmetric_tree(ecs: &mut EcsMaster) -> [Entity; 6] {
    let e = spawn_entities(ecs, 6);
    let (root, a, b, a1, a2, b1) = (e[0], e[1], e[2], e[3], e[4], e[5]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(root).add_child(a);
        cmds.entity(root).add_child(b);
        cmds.entity(a).add_child(a1);
        cmds.entity(a).add_child(a2);
        cmds.entity(b).add_child(b1);
    });
    [root, a, b, a1, a2, b1]
}

#[test]
fn down_broadcast_reaches_root_and_all_descendants_once() {
    let mut ecs = EcsMaster::new();
    let [root, a, b, a1, a2, b1] = build_asymmetric_tree(&mut ecs);
    for &e in &[root, a, b, a1, a2, b1] {
        ecs.observe_entity_event::<DownAll>(e, b1_record);
    }
    b1_fires().lock().expect("lock").clear();

    ecs.trigger::<DownAll>(root, DownAll);

    let mut fired = b1_fires().lock().expect("lock").clone();
    fired.sort_unstable();
    let mut expected: Vec<usize> =
        [root, a, b, a1, a2, b1].iter().map(|e| e.id().0).collect();
    expected.sort_unstable();
    assert_eq!(
        fired, expected,
        "Down broadcast fires at root + every descendant EXACTLY once (no dup, no miss)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// B.2 — W4 PRUNE ISOLATION (critical): propagate(false) at a MID-TREE node `a`
//       prunes ONLY a's subtree; sibling `b` and b's descendant `b1` STILL fire.
// ════════════════════════════════════════════════════════════════════════════

/// The id of the node that prunes (set per test); its observer calls
/// `propagate(false)` so its own subtree is skipped.
static B2_PRUNE_ID: AtomicUsize = AtomicUsize::new(usize::MAX);

fn b2_fires() -> &'static Mutex<Vec<usize>> {
    static F: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(Vec::new()))
}
unsafe fn b2_record(_w: DeferredEcsMaster<'_>, ctx: TriggerContext, _e: *const u8) {
    let id = ctx.target.id().0;
    b2_fires().lock().expect("lock").push(id);
    if id == B2_PRUNE_ID.load(SEQ) {
        // Prune THIS node's subtree only — its descendants must NOT fire, but
        // siblings and their descendants must (the per-node propagate snapshot).
        propagate(false);
    }
}

#[test]
fn prune_at_mid_node_isolates_subtree_siblings_still_receive() {
    let mut ecs = EcsMaster::new();
    let [root, a, b, a1, a2, b1] = build_asymmetric_tree(&mut ecs);
    for &e in &[root, a, b, a1, a2, b1] {
        ecs.observe_entity_event::<DownAll>(e, b2_record);
    }
    b2_fires().lock().expect("lock").clear();
    // Prune at `a` → a's children a1, a2 must NOT fire; root, a, b, b1 must.
    B2_PRUNE_ID.store(a.id().0, SEQ);

    ecs.trigger::<DownAll>(root, DownAll);

    let mut fired = b2_fires().lock().expect("lock").clone();
    fired.sort_unstable();
    let mut expected: Vec<usize> = [root, a, b, b1].iter().map(|e| e.id().0).collect();
    expected.sort_unstable();
    assert_eq!(
        fired, expected,
        "W4: propagate(false) at `a` prunes ONLY a's subtree (a1,a2 absent); the sibling `b` \
         AND b's descendant `b1` STILL receive — no cross-subtree leak",
    );
    assert!(
        !fired.contains(&a1.id().0) && !fired.contains(&a2.id().0),
        "a's descendants (a1,a2) were pruned",
    );
    assert!(
        fired.contains(&b1.id().0),
        "the sibling subtree's descendant b1 was NOT leaked into the prune (W4 isolation)",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// B.3 — regression: an `Up` bubble and a `None` trigger are UNCHANGED by the
//       Down machinery (the const-fold leaves their code generation intact).
// ════════════════════════════════════════════════════════════════════════════

/// An `Up` bubble (AUTO_PROPAGATE) over ChildOf — pre-broadcast behavior.
struct UpBubble;
impl Trigger for UpBubble {
    const AUTO_PROPAGATE: bool = true;
    const PROPAGATION: PropagationMode = PropagationMode::Up;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}
/// A `None` (target-only) trigger.
struct NoneEv;
impl Trigger for NoneEv {
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

static B3_UP: AtomicUsize = AtomicUsize::new(0);
static B3_NONE: AtomicUsize = AtomicUsize::new(0);
unsafe fn b3_up(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    B3_UP.fetch_add(1, SEQ);
}
unsafe fn b3_none(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    B3_NONE.fetch_add(1, SEQ);
}

#[test]
fn up_bubble_and_none_trigger_unchanged_by_down_machinery() {
    let mut ecs = EcsMaster::new();
    let [root, a, _b, a1, _a2, _b1] = build_asymmetric_tree(&mut ecs);

    // Up bubble from a1 → a1, a, root = 3 hops (the linear ChildOf walk).
    ecs.observe_entity_event::<UpBubble>(root, b3_up);
    ecs.observe_entity_event::<UpBubble>(a, b3_up);
    ecs.observe_entity_event::<UpBubble>(a1, b3_up);
    B3_UP.store(0, SEQ);
    ecs.trigger::<UpBubble>(a1, UpBubble);
    assert_eq!(
        B3_UP.load(SEQ),
        3,
        "Up bubble walks a1 → a → root (3 hops) — UNCHANGED by the Down broadcast addition",
    );

    // None trigger at root → ONLY root fires (no descent, no bubble).
    ecs.observe_entity_event::<NoneEv>(root, b3_none);
    ecs.observe_entity_event::<NoneEv>(a, b3_none);
    ecs.observe_entity_event::<NoneEv>(a1, b3_none);
    B3_NONE.store(0, SEQ);
    ecs.trigger::<NoneEv>(root, NoneEv);
    assert_eq!(
        B3_NONE.load(SEQ),
        1,
        "None trigger fires at the target only (no descent/bubble) — pre-broadcast behavior",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// C — SCALING-GUARD (reviewer A2, MANDATORY): Down fire count ≤ C·N (C = 1 per
//     node under the visited guard) on deep chain, wide fan-out, balanced tree.
// ════════════════════════════════════════════════════════════════════════════

// A GLOBAL Down observer counts every fired node. Each scaling test uses its OWN
// trigger type + counter so the three tests stay independent when run in PARALLEL
// in the same binary (a shared static counter would race across concurrent tests).
//
// Three distinct Down triggers over ChildOf — one per shape.
struct DownDeep;
impl Trigger for DownDeep {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}
struct DownWide;
impl Trigger for DownWide {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}
struct DownBalanced;
impl Trigger for DownBalanced {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

static C_DEEP: AtomicUsize = AtomicUsize::new(0);
static C_WIDE: AtomicUsize = AtomicUsize::new(0);
static C_BALANCED: AtomicUsize = AtomicUsize::new(0);
unsafe fn c_deep(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    C_DEEP.fetch_add(1, SEQ);
}
unsafe fn c_wide(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    C_WIDE.fetch_add(1, SEQ);
}
unsafe fn c_balanced(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    C_BALANCED.fetch_add(1, SEQ);
}

/// Builds a DEEP ChildOf chain of `n` nodes (root → c1 → c2 → …). Returns the
/// root. Each node parents the next.
fn build_deep_chain(ecs: &mut EcsMaster, n: usize) -> Entity {
    let e = spawn_entities(ecs, n);
    let chain = e.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for w in chain.windows(2) {
            cmds.entity(w[0]).add_child(w[1]);
        }
    });
    e[0]
}

/// Builds a WIDE fan-out: one root with `n - 1` direct children. Returns root.
fn build_wide(ecs: &mut EcsMaster, n: usize) -> Entity {
    let e = spawn_entities(ecs, n);
    let root = e[0];
    let kids: Vec<Entity> = e[1..].to_vec();
    ecs.run_system(move |mut cmds: Commands| {
        for &k in &kids {
            cmds.entity(root).add_child(k);
        }
    });
    root
}

/// Builds a roughly-balanced binary tree of `n` nodes (node `i`'s parent is
/// `(i-1)/2`). Returns root (node 0).
fn build_balanced(ecs: &mut EcsMaster, n: usize) -> Entity {
    let e = spawn_entities(ecs, n);
    let nodes = e.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for i in 1..n {
            let parent = nodes[(i - 1) / 2];
            cmds.entity(parent).add_child(nodes[i]);
        }
    });
    e[0]
}

/// Measures the Down fire count for trigger `E` on a tree built by `build` with
/// `n` nodes. `make` constructs the (ZST) event; `counter` is `E`'s private
/// counter (reset before, read after). Each scaling test passes its OWN `E` +
/// counter, so concurrent execution never races.
fn measure_down_fires<E: Trigger>(
    build: impl Fn(&mut EcsMaster, usize) -> Entity,
    n: usize,
    runner: TriggerFn,
    make: impl Fn() -> E,
    counter: &AtomicUsize,
) -> usize {
    let mut ecs = EcsMaster::new();
    ecs.observe::<E>(runner);
    let root = build(&mut ecs, n);
    counter.store(0, SEQ);
    ecs.trigger::<E>(root, make());
    counter.load(SEQ)
}

#[test]
fn scaling_guard_deep_chain_linear() {
    for &n in &[10usize, 100, 1000] {
        let fires = measure_down_fires(build_deep_chain, n, c_deep, || DownDeep, &C_DEEP);
        assert_eq!(
            fires, n,
            "deep chain of {n}: Down fires exactly once per node (= N, C = 1) — linear, no blowup",
        );
    }
}

#[test]
fn scaling_guard_wide_fanout_linear() {
    for &n in &[10usize, 100, 1000] {
        let fires = measure_down_fires(build_wide, n, c_wide, || DownWide, &C_WIDE);
        assert_eq!(
            fires, n,
            "wide fan-out of {n}: Down fires exactly once per node (root + {} children) — linear",
            n - 1,
        );
    }
}

#[test]
fn scaling_guard_balanced_tree_linear() {
    for &n in &[10usize, 100, 1000] {
        let fires =
            measure_down_fires(build_balanced, n, c_balanced, || DownBalanced, &C_BALANCED);
        assert_eq!(
            fires, n,
            "balanced tree of {n}: Down fires exactly once per node (= N, C = 1) — linear",
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// C.cyclic — a deliberately CYCLIC `!ACYCLIC` Broadcast relation: the Down
//            broadcast TERMINATES with each node fired ≤ once (the VisitedSet).
//            A derive-built relation is ALWAYS `ACYCLIC = false`, so this
//            exercises the cold function-local visited set.
// ════════════════════════════════════════════════════════════════════════════

/// A generic (NON-ChildOf) relation used to build a cyclic broadcast graph. The
/// derive emits `ACYCLIC = false`, so the Down DFS allocates the visited set.
#[derive(Component, Clone, Copy, RelationshipDerive)]
#[repr(transparent)]
#[relationship(target = PointedAtBy, allow_self_referential)]
struct PointsTo(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = PointsTo, retain_empty)]
struct PointedAtBy(Vec<Entity>);

/// A Down broadcast over the cyclic `PointsTo` relation.
struct DownCyclic;
impl Trigger for DownCyclic {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal; // never read (Down)
    type Broadcast = PointsTo;
}

fn cyc_fires() -> &'static Mutex<Vec<usize>> {
    static F: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(Vec::new()))
}
unsafe fn cyc_record(_w: DeferredEcsMaster<'_>, ctx: TriggerContext, _e: *const u8) {
    cyc_fires().lock().expect("lock").push(ctx.target.id().0);
}

#[test]
fn cyclic_non_acyclic_down_broadcast_terminates_each_node_once() {
    // Compile-time: a derived relation is conservative `ACYCLIC = false`.
    const { assert!(!<PointsTo as Relationship>::ACYCLIC) };

    let mut ecs = EcsMaster::new();
    const N: usize = 6;
    let nodes = spawn_entities(&mut ecs, N);
    let ring = nodes.clone();
    // Ring: node[i] PointsTo node[(i+1) % N]. The Down broadcast over PointsTo at
    // node[0] descends node[i] → its sources (node[i-1]) → … chasing the ring.
    ecs.run_system(move |mut cmds: Commands| {
        for i in 0..N {
            cmds.entity(ring[i]).insert(PointsTo(ring[(i + 1) % N]));
        }
    });
    for &e in &nodes {
        ecs.observe_entity_event::<DownCyclic>(e, cyc_record);
    }
    cyc_fires().lock().expect("lock").clear();

    // MUST terminate (the VisitedSet caps each node at one fire). A non-terminating
    // walk would hang (CI wall-clock catches it).
    ecs.trigger::<DownCyclic>(nodes[0], DownCyclic);

    let fired = cyc_fires().lock().expect("lock").clone();
    // Each node fired AT MOST once (the ≤ C·N guarantee under the visited guard).
    let mut seen = fired.clone();
    seen.sort_unstable();
    seen.dedup();
    assert!(
        fired.len() <= N,
        "cyclic !ACYCLIC Down broadcast fires ≤ N times ({} ≤ {N}) — terminated, no blowup",
        fired.len(),
    );
    assert_eq!(
        fired.len(),
        seen.len(),
        "each node in the cycle fired at MOST once (VisitedSet dedup) — got duplicates: {fired:?}",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// C.mislabeled — an ACYCLIC=true relation (ChildOf) over a CYCLE built by raw
//                inserts: the Down broadcast TERMINATES at the depth cap
//                (bounded, no infinite loop) since the visited guard const-folds
//                away. ChildOf is the only public `ACYCLIC = true` relation.
// ════════════════════════════════════════════════════════════════════════════

/// A Down broadcast over `ChildOf` (ACYCLIC = true → no visited set; depth cap
/// alone bounds the walk).
struct DownChildOf;
impl Trigger for DownChildOf {
    const PROPAGATION: PropagationMode = PropagationMode::Down;
    type Traversal = ChildOfTraversal;
    type Broadcast = ChildOf;
}

static MIS_FIRES: AtomicUsize = AtomicUsize::new(0);
unsafe fn mis_count(_w: DeferredEcsMaster<'_>, _c: TriggerContext, _e: *const u8) {
    MIS_FIRES.fetch_add(1, SEQ);
}

#[test]
fn acyclic_mislabeled_cycle_down_broadcast_terminates_at_depth_cap() {
    // Compile-time: ChildOf is ACYCLIC = true (the const-folded depth-cap path).
    const { assert!(<ChildOf as Relationship>::ACYCLIC) };

    let mut ecs = EcsMaster::new();
    // Build a 3-node ChildOf CYCLE by raw inserts (the engine only guards a direct
    // self-reference; a longer cycle is a documented footgun, so we can construct
    // one here to exercise the depth-cap termination). a → b → c → a.
    let nodes = spawn_entities(&mut ecs, 3);
    let (a, b, c) = (nodes[0], nodes[1], nodes[2]);
    ecs.run_system(move |mut cmds: Commands| {
        // Build the reverse index for each edge: ChildOf(parent) on the child.
        // a is child of c, b is child of a, c is child of b → a cycle in Children.
        cmds.entity(a).insert(ChildOf(c));
        cmds.entity(b).insert(ChildOf(a));
        cmds.entity(c).insert(ChildOf(b));
    });
    // Confirm the cyclic reverse index is built (each has one child).
    for &e in &[a, b, c] {
        assert!(
            ecs.get_component::<Children>(e).map(|ch| ch.len()).unwrap_or(0) == 1,
            "each cyclic node has exactly one child (the ring edge)",
        );
    }

    ecs.observe::<DownChildOf>(mis_count);
    MIS_FIRES.store(0, SEQ);

    // MUST terminate at MAX_PROPAGATION_DEPTH (bounded) — NOT hang. The visited
    // guard const-folds away (ACYCLIC), so the depth cap alone bounds the walk.
    ecs.trigger::<DownChildOf>(a, DownChildOf);

    let fires = MIS_FIRES.load(SEQ);
    // Bounded: the depth cap is MAX_PROPAGATION_DEPTH; the walk fires at most
    // O(depth) nodes over the 3-cycle, never unbounded.
    assert!(fires > 0, "the mislabeled-cycle Down broadcast fired at least the root");
    assert!(
        fires <= boyko_ecs::ecs::constants::MAX_PROPAGATION_DEPTH + 8,
        "ACYCLIC-mislabeled cycle TERMINATES at the depth cap (fired {fires} ≤ \
         MAX_PROPAGATION_DEPTH + slack) — bounded, no infinite loop",
    );
}
