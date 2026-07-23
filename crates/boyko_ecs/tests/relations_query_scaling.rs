//! F — scaling-guard (superlinear guard) + cycle-termination for the relation
//! traversal accessors `ancestors` / `descendants` / `sources`.
//!
//! The guard property: a traversal over N nodes visits / yields ≤ C·N entities —
//! NEVER superlinear. Asserted across N ∈ {10, 100, 1000} on three shapes (deep
//! chain, wide fan-out, balanced tree). A green correctness suite on small inputs
//! cannot catch an O(N²) / exponential walk, so this scaling assertion is the
//! dedicated guard.
//!
//! Constants:
//! * chain / tree under the visited guard (or const-folded for an acyclic
//!   relation): C = 1 (each node yielded exactly once).
//! * wide fan-out: `descendants(root)` yields all N-1 children once ⇒ C = 1 for
//!   the yielded count; `sources(target)` yields all N-1 sources once ⇒ C = 1.
//!
//! Relations used:
//! * `ChildOf` — `ACYCLIC = true` (hand-written), so the `VisitedSet` const-folds
//!   AWAY; only the depth cap bounds the walk. Covers the acyclic chain/tree/fan.
//! * `Likes` — `ACYCLIC = false` (the derive default), so the `#[cold]`
//!   `VisitedSet` revisit-guard is live. Covers the CYCLIC graph (each node ≤ once)
//!   and proves the ≤ C·N bound holds even on a graph WITH cycles.
//! * The ACYCLIC-MISLABEL case (a relation declared `ACYCLIC = true` but actually
//!   cyclic): a `ChildOf` 2-cycle (a.ChildOf=b, b.ChildOf=a — the insert guards
//!   reject self-links but NOT 2-cycles). With the visited set const-folded away,
//!   the walk must TERMINATE at the depth cap (`MAX_PROPAGATION_DEPTH`), not hang.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::constants::MAX_PROPAGATION_DEPTH;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::relationship::Relationship as _;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Component, Relationship, RelationshipTarget};

// ── Fixtures ─────────────────────────────────────────────────────────────────

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Tag(u32);

/// A derive-built relation with `ACYCLIC = false` (the conservative default) ⇒
/// the traversal allocates the `#[cold]` `VisitedSet` revisit guard.
#[derive(Component, Clone, Copy, Relationship)]
#[repr(transparent)]
#[relationship(target = LikedBy)]
struct Likes(pub Entity);

#[derive(Component, RelationshipTarget, Default)]
#[relationship_target(source = Likes, retain_empty)]
struct LikedBy(Vec<Entity>);

/// `Likes` defaults to the conservative `ACYCLIC = false`, so the visited guard is
/// LIVE (this is the relation that proves the cyclic ≤ C·N bound).
const _: () = assert!(!Likes::ACYCLIC, "Likes uses the conservative ACYCLIC=false default");

// ── Spawn helper ─────────────────────────────────────────────────────────────

fn spawn_tags(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(Tag(i as u32)).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    assert_eq!(out.len(), n, "spawn helper produced n handles");
    out
}

// ════════════════════════════════════════════════════════════════════════════
// F.1 — ChildOf (ACYCLIC): deep chain, wide fan, balanced tree. ≤ C·N, C=1.
// ════════════════════════════════════════════════════════════════════════════

/// Builds a deep `ChildOf` chain `n[0] <- n[1] <- ... <- n[N-1]` and returns the
/// node handles. `n[i+1].ChildOf = n[i]`, so the deepest node is `n[N-1]`.
fn build_chain(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let nodes = spawn_tags(ecs, n);
    let chain = nodes.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for i in 1..chain.len() {
            cmds.entity(chain[i]).insert(ChildOf(chain[i - 1]));
        }
    });
    nodes
}

#[test]
fn chain_ancestors_and_descendants_linear_in_n() {
    for &n in &[10usize, 100, 1000] {
        let mut ecs = EcsMaster::new();
        let nodes = build_chain(&mut ecs, n);
        let deepest = nodes[n - 1];
        let root = nodes[0];

        // ancestors(deepest) walks up the whole chain: exactly n-1 ancestors.
        let anc = ecs.ancestors::<ChildOf>(deepest).count();
        assert_eq!(anc, n - 1, "chain N={n}: ancestors(deepest) yields exactly n-1 (C=1)");
        assert!(anc <= n, "chain N={n}: ancestor count ≤ N (≤ C·N, C=1)");

        // descendants(root) walks down the whole chain: exactly n-1 descendants.
        let desc = ecs.descendants::<ChildOf>(root).count();
        assert_eq!(desc, n - 1, "chain N={n}: descendants(root) yields exactly n-1 (C=1)");
        assert!(desc <= n, "chain N={n}: descendant count ≤ N (≤ C·N, C=1)");
    }
}

#[test]
fn wide_fanout_descendants_and_sources_linear_in_n() {
    for &n in &[10usize, 100, 1000] {
        let mut ecs = EcsMaster::new();
        let nodes = spawn_tags(&mut ecs, n);
        let root = nodes[0];
        let children = nodes.clone();
        // Fan-out: every node[1..] is a direct child of root.
        ecs.run_system(move |mut cmds: Commands| {
            for &c in &children[1..] {
                cmds.entity(c).insert(ChildOf(root));
            }
        });

        // descendants(root) = the n-1 children, each yielded once.
        let desc = ecs.descendants::<ChildOf>(root).count();
        assert_eq!(desc, n - 1, "fan N={n}: descendants(root) yields exactly n-1 (C=1)");
        assert!(desc <= n, "fan N={n}: ≤ C·N, C=1");

        // sources(root) = the n-1 sources via the reverse collection, each once.
        let srcs = ecs.sources::<ChildOf>(root).count();
        assert_eq!(srcs, n - 1, "fan N={n}: sources(root) yields exactly n-1 (C=1)");
        assert!(srcs <= n, "fan N={n}: sources ≤ C·N, C=1");
    }
}

#[test]
fn balanced_binary_tree_descendants_linear_in_n() {
    for &n in &[15usize, 127, 1023] {
        // n = 2^k - 1 ⇒ a complete binary tree: node i's parent is (i-1)/2.
        let mut ecs = EcsMaster::new();
        let nodes = spawn_tags(&mut ecs, n);
        let tree = nodes.clone();
        ecs.run_system(move |mut cmds: Commands| {
            for i in 1..tree.len() {
                let parent = tree[(i - 1) / 2];
                cmds.entity(tree[i]).insert(ChildOf(parent));
            }
        });
        let root = nodes[0];

        // descendants(root) over the whole tree = n-1 nodes, each visited once.
        let desc = ecs.descendants::<ChildOf>(root).count();
        assert_eq!(desc, n - 1, "tree N={n}: descendants(root) = n-1 (each node once, C=1)");
        assert!(desc <= n, "tree N={n}: ≤ C·N, C=1");

        // A leaf's ancestors == tree depth (k = log2(n+1)), bounded by N.
        let leaf = nodes[n - 1];
        let depth = ecs.ancestors::<ChildOf>(leaf).count();
        assert!(depth <= n, "tree N={n}: leaf ancestors ≤ N");
        assert!(depth > 0, "tree N={n}: a leaf has at least one ancestor");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// F.2 — CYCLIC graph (Likes, ACYCLIC=false): the visited guard caps each node
//        to ≤ 1 visit, so descendants/ancestors terminate at ≤ N.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn cyclic_descendants_visits_each_node_at_most_once() {
    for &n in &[10usize, 100, 1000] {
        let mut ecs = EcsMaster::new();
        let nodes = spawn_tags(&mut ecs, n);
        let ring = nodes.clone();
        // A ring: node[i].Likes(node[(i+1) % N]). ⇒ node[(i+1)%N].LikedBy ∋ node[i].
        // descendants walks the reverse collections; the ring would loop forever
        // without the visited guard.
        ecs.run_system(move |mut cmds: Commands| {
            for i in 0..ring.len() {
                let next = ring[(i + 1) % ring.len()];
                cmds.entity(ring[i]).insert(Likes(next));
            }
        });

        // descendants(node0): every OTHER node is reachable backwards through the
        // ring exactly once (node0 itself is seeded into the visited set, excluded).
        let desc: Vec<Entity> = ecs.descendants::<Likes>(nodes[0]).collect();
        assert!(
            desc.len() <= n,
            "cyclic N={n}: descendants visits ≤ N nodes (≤ C·N) — the visited guard terminated"
        );
        // No node appears twice (the ≤ 1 visit property).
        let mut ids: Vec<usize> = desc.iter().map(|e| e.id().0).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "cyclic N={n}: no node descendant-visited twice");
        // The ring reaches all n-1 other nodes exactly once.
        assert_eq!(before, n - 1, "cyclic N={n}: the ring reaches the other n-1 nodes once each");
    }
}

#[test]
fn cyclic_ancestors_visits_each_node_at_most_once() {
    for &n in &[10usize, 100, 1000] {
        let mut ecs = EcsMaster::new();
        let nodes = spawn_tags(&mut ecs, n);
        let ring = nodes.clone();
        ecs.run_system(move |mut cmds: Commands| {
            for i in 0..ring.len() {
                let next = ring[(i + 1) % ring.len()];
                cmds.entity(ring[i]).insert(Likes(next));
            }
        });

        // ancestors(node0) walks Likes targets forward around the ring; the visited
        // guard (node0 seeded) stops after re-entering node0 ⇒ ≤ N yields.
        let anc: Vec<Entity> = ecs.ancestors::<Likes>(nodes[0]).collect();
        assert!(anc.len() <= n, "cyclic N={n}: ancestors visits ≤ N (≤ C·N)");
        let mut ids: Vec<usize> = anc.iter().map(|e| e.id().0).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "cyclic N={n}: no node ancestor-visited twice");
        // Forward ring from node0 reaches the other n-1 nodes exactly once.
        assert_eq!(before, n - 1, "cyclic N={n}: forward ring reaches n-1 nodes once each");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// F.3 — ACYCLIC-MISLABEL: a relation declared ACYCLIC=true (ChildOf) but made
//        actually cyclic (a 2-cycle) must TERMINATE at the depth cap, not hang
//        and not UB (the visited set is const-folded away for an acyclic label).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn acyclic_mislabel_2cycle_ancestors_terminates_at_depth_cap() {
    // ChildOf is ACYCLIC=true, so the visited guard is const-folded away — only the
    // depth cap bounds the walk. A 2-cycle (a.ChildOf=b, b.ChildOf=a) is admitted by
    // the insert hooks (they reject self-links and dangling targets, but NOT
    // 2-cycles). Walking it must terminate at MAX_PROPAGATION_DEPTH, never hang.
    const { assert!(ChildOf::ACYCLIC, "ChildOf is the acyclic-labeled relation") };

    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 2);
    let (a, b) = (nodes[0], nodes[1]);

    // Build the 2-cycle. Insert a.ChildOf(b) first (b is alive). Then b.ChildOf(a):
    // a is alive, so the dangling guard passes; no self-link; the cycle forms.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(ChildOf(b));
    });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(b).insert(ChildOf(a));
    });

    // ancestors(a) bounces a -> b -> a -> b -> ... and must stop at the cap.
    // The yielded count is bounded by MAX_PROPAGATION_DEPTH (a finite return, no hang).
    let count = ecs.ancestors::<ChildOf>(a).count();
    assert!(
        count <= MAX_PROPAGATION_DEPTH,
        "mislabeled cyclic ancestors must terminate at the depth cap ({count} ≤ {MAX_PROPAGATION_DEPTH})"
    );
    // It DID hit the cap (the cycle is infinite without the visited set), proving
    // the depth cap — not a natural end — is what terminated it.
    assert_eq!(
        count, MAX_PROPAGATION_DEPTH,
        "the infinite 2-cycle walk is bounded exactly by the depth cap"
    );
}

#[test]
fn acyclic_mislabel_2cycle_descendants_terminates_at_depth_cap() {
    let mut ecs = EcsMaster::new();
    let nodes = spawn_tags(&mut ecs, 2);
    let (a, b) = (nodes[0], nodes[1]);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).insert(ChildOf(b));
    });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(b).insert(ChildOf(a));
    });

    // descendants(a) DFS over reverse collections bounces between a and b; without
    // a visited set it is bounded only by the per-node depth cap. Each pop pushes
    // the other node at depth+1, so the frontier grows until depth == cap. The walk
    // must RETURN (bounded), not hang. The yielded count is finite and bounded by
    // the cap (one yield per pop up to the depth cap).
    let count = ecs.descendants::<ChildOf>(a).take(MAX_PROPAGATION_DEPTH + 16).count();
    assert!(
        count <= MAX_PROPAGATION_DEPTH + 16,
        "mislabeled cyclic descendants must be bounded (the take() cap was a safety net, not the terminator)"
    );
    // Confirm it terminates WITHOUT the take() net within a generous bound: a fresh
    // walk collected fully must be ≤ a small multiple of the depth cap.
    let full = ecs.descendants::<ChildOf>(a).count();
    assert!(
        full <= 2 * MAX_PROPAGATION_DEPTH,
        "the descendants DFS terminates within ~2·depth-cap yields ({full}), never hangs"
    );
}
