//! O4 gates: the [`ConstraintGraph`] islands + greedy-coloring partition is a
//! PURE pre-compute (Decision 2 / Decision 7) — it builds islands (connected
//! components over DYNAMIC bodies, Box2D's ground rule) and greedy-colors the
//! manifolds so NO color shares a dynamic body, and it does so deterministically
//! and allocation-free in steady state. O4 does NOT consume the partition: the
//! shipped [`SoftStepSolver`](boyko_physics::SoftStepSolver) still solves in
//! manifold order, so the simulation output is byte-identical whether the colored
//! flag is on or off (the campaign 0%-gate).
//!
//! These are the tester's exhaustive gates (the in-`resources.rs` `graph_*` tests
//! are SANITY checks on hand-built graphs). The load-bearing gate is the
//! **coloring invariant** (Gate 1): on 1000 random graphs, no color contains two
//! manifolds sharing a dynamic body (re-scan). The **island == BFS reference**
//! gate (Gate 2) cross-checks the union-find partition against an independent BFS
//! over the SAME ground rule. The **bit-determinism** gate (Gate 3) proves the
//! partition is a pure function of its input with no stale-scratch leak. The
//! **CSR invariants** gate (Gate 7) checks `color_start` / `island` monotonicity,
//! island-id range, and the static/sentinel == `NO_ISLAND` contract.
//!
//! The pure `build()` path here is Miri-compatible: it touches only `Vec` scratch
//! (no pool, no int-to-ptr) so `cargo miri test -p boyko-physics --lib` plus this
//! suite's `build()` calls exercise the partition under the interpreter (the
//! schedule-driven path is proven separately — it aborts under Miri at the pool
//! int-to-ptr, a documented pool artifact).

// Test-only: `HashSet` is the ORACLE model here — the reference "no color reuses a
// dynamic body" / injectivity checker the constraint-graph coloring is differentially
// verified against — and `Arc<Mutex<…>>` is the established probe for smuggling a spawned
// `Entity` out of the `Send + Sync` one-shot system closure. The solver's own structures
// stay VM-native; this file is compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::collections::{HashSet, VecDeque};

use boyko_physics::manifold::{BodyIndex, Manifold, SDF_SENTINEL};
use boyko_physics::resources::ConstraintGraph;

use proptest::prelude::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Builds an empty manifold between two dense body rows (the partition reads only
/// `body_a` / `body_b`).
fn edge(a: u32, b: u32) -> Manifold {
    Manifold::new(BodyIndex(a), BodyIndex(b))
}

/// The dynamic predicate the production stage uses (systems.rs:693): a row is
/// dynamic iff it is a real in-range row with non-zero inverse mass. Here a row is
/// dynamic iff it is in `dynamic_rows` AND `< n_bodies` — the `u32::MAX` sentinel
/// and any out-of-range row is non-dynamic (ground) by construction.
fn make_is_dynamic(dynamic_rows: HashSet<u32>, n_bodies: u32) -> impl Fn(u32) -> bool {
    move |row: u32| row < n_bodies && dynamic_rows.contains(&row)
}

/// Re-scans the produced coloring and asserts NO color shares a dynamic body (the
/// O4 load-bearing invariant). Returns `Ok(())` or a descriptive failure for
/// `prop_assert!`. Also confirms every manifold appears in exactly one color.
fn check_coloring_invariant(
    g: &ConstraintGraph,
    manifolds: &[Manifold],
    is_dynamic: &impl Fn(u32) -> bool,
) -> Result<(), String> {
    let mut total = 0usize;
    for c in 0..g.n_colors() {
        let mut seen: HashSet<u32> = HashSet::new();
        for &mi in g.color(c) {
            let m = &manifolds[mi as usize];
            for &row in &[m.body_a.0, m.body_b.0] {
                if is_dynamic(row) && !seen.insert(row) {
                    return Err(format!(
                        "coloring invariant BROKEN: color {c} reuses dynamic body {row}"
                    ));
                }
            }
        }
        total += g.color(c).len();
    }
    if total != manifolds.len() {
        return Err(format!(
            "partition incomplete: {total} manifolds colored, expected {}",
            manifolds.len()
        ));
    }
    Ok(())
}

/// Independent BFS reference over the SAME ground rule (Decision 2): two bodies
/// share a component ONLY when BOTH are dynamic; a static/sentinel body never
/// connects components. Returns, for each dynamic row, a canonical component
/// representative (the smallest dynamic row reachable through dyn-dyn edges). A
/// non-dynamic row maps to `u32::MAX`.
///
/// This is structurally distinct from the union-find under test (an explicit
/// adjacency-list BFS, not path-compressed disjoint-set), so a match validates the
/// build's connected-component partition against a second algorithm.
fn bfs_components(
    manifolds: &[Manifold],
    n_bodies: u32,
    is_dynamic: &impl Fn(u32) -> bool,
) -> Vec<u32> {
    // Adjacency over dyn-dyn edges only.
    let n = n_bodies as usize;
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for m in manifolds {
        let a = m.body_a.0;
        let b = m.body_b.0;
        if is_dynamic(a) && is_dynamic(b) && a < n_bodies && b < n_bodies {
            adj[a as usize].push(b);
            adj[b as usize].push(a);
        }
    }
    // Component rep = smallest dynamic row reachable. BFS from each unvisited
    // dynamic root in ascending row order.
    let mut rep = vec![u32::MAX; n];
    for start in 0..n_bodies {
        if !is_dynamic(start) || rep[start as usize] != u32::MAX {
            continue;
        }
        // New component rooted at `start` (the smallest unvisited dynamic row).
        let mut q = VecDeque::new();
        q.push_back(start);
        rep[start as usize] = start;
        while let Some(cur) = q.pop_front() {
            for &nb in &adj[cur as usize] {
                if rep[nb as usize] == u32::MAX {
                    rep[nb as usize] = start;
                    q.push_back(nb);
                }
            }
        }
    }
    rep
}

/// Asserts the build's `island_of` partition matches the BFS reference's
/// connected components: two dynamic rows are in the same island iff they are in
/// the same BFS component (and same-component-ness is what matters, not the id
/// value — the build compacts ids in root order, the BFS uses the smallest row as
/// rep). Returns `Ok(())` or a failure string.
fn check_islands_match_bfs(
    g: &ConstraintGraph,
    manifolds: &[Manifold],
    n_bodies: u32,
    is_dynamic: &impl Fn(u32) -> bool,
) -> Result<(), String> {
    let rep = bfs_components(manifolds, n_bodies, is_dynamic);
    let dyn_rows: Vec<u32> = (0..n_bodies).filter(|&r| is_dynamic(r)).collect();

    // 1) Static/sentinel rows are NO_ISLAND; dynamic rows have a valid island id.
    for row in 0..n_bodies {
        let iof = g.island_of(row);
        if is_dynamic(row) {
            if iof >= g.n_islands() {
                return Err(format!(
                    "dynamic row {row} has island_of {iof} >= n_islands {}",
                    g.n_islands()
                ));
            }
        } else if iof != ConstraintGraph::NO_ISLAND {
            return Err(format!("non-dynamic row {row} should be NO_ISLAND, got {iof}"));
        }
    }

    // 2) Same-component relation: for every ordered pair of dynamic rows, the
    //    build's island equality must match the BFS component equality.
    for &i in &dyn_rows {
        for &j in &dyn_rows {
            let same_build = g.island_of(i) == g.island_of(j);
            let same_bfs = rep[i as usize] == rep[j as usize];
            if same_build != same_bfs {
                return Err(format!(
                    "island/BFS mismatch for ({i},{j}): build_same={same_build} bfs_same={same_bfs}"
                ));
            }
        }
    }

    // 3) The island count equals the number of distinct BFS components.
    let distinct: HashSet<u32> = dyn_rows.iter().map(|&r| rep[r as usize]).collect();
    if g.n_islands() as usize != distinct.len() {
        return Err(format!(
            "n_islands {} != distinct BFS components {}",
            g.n_islands(),
            distinct.len()
        ));
    }
    Ok(())
}

/// Validates the CSR + range invariants (Gate 7) by re-reading the public
/// accessors: every color/island slice is non-empty-coherent, every manifold
/// appears exactly once across colors AND exactly once across islands (or under no
/// island for a static-static degenerate edge), and id ranges hold.
fn check_csr_invariants(
    g: &ConstraintGraph,
    manifolds: &[Manifold],
    is_dynamic: &impl Fn(u32) -> bool,
) -> Result<(), String> {
    // Every manifold appears in exactly one color (a multiset count over colors).
    let mut color_seen = vec![0u32; manifolds.len()];
    for c in 0..g.n_colors() {
        for &mi in g.color(c) {
            if (mi as usize) >= manifolds.len() {
                return Err(format!("color {c} references out-of-range manifold {mi}"));
            }
            color_seen[mi as usize] += 1;
        }
    }
    for (mi, &n) in color_seen.iter().enumerate() {
        if n != 1 {
            return Err(format!("manifold {mi} appears in {n} colors, expected 1"));
        }
    }

    // Every manifold with a dynamic side appears in exactly one island; a
    // static-static degenerate edge appears in none.
    let mut island_seen = vec![0u32; manifolds.len()];
    for i in 0..g.n_islands() {
        for &mi in g.island(i) {
            if (mi as usize) >= manifolds.len() {
                return Err(format!("island {i} references out-of-range manifold {mi}"));
            }
            island_seen[mi as usize] += 1;
        }
    }
    for (mi, &n) in island_seen.iter().enumerate() {
        let m = &manifolds[mi];
        let has_dyn = is_dynamic(m.body_a.0) || is_dynamic(m.body_b.0);
        let expected = u32::from(has_dyn);
        if n != expected {
            return Err(format!(
                "manifold {mi} (has_dyn={has_dyn}) appears in {n} islands, expected {expected}"
            ));
        }
    }

    // Out-of-range color/island return empty slices (the accessor guard).
    if !g.color(g.n_colors()).is_empty() {
        return Err("color(n_colors) must be empty".into());
    }
    if !g.island(g.n_islands()).is_empty() {
        return Err("island(n_islands) must be empty".into());
    }
    Ok(())
}

/// Snapshots the full public partition state for a bit-determinism compare.
#[derive(Clone, PartialEq, Eq, Debug)]
struct PartitionSnapshot {
    n_colors: u32,
    n_islands: u32,
    island_of: Vec<u32>,
    colors: Vec<Vec<u32>>,
    islands: Vec<Vec<u32>>,
}

fn snapshot(g: &ConstraintGraph, n_bodies: u32) -> PartitionSnapshot {
    PartitionSnapshot {
        n_colors: g.n_colors(),
        n_islands: g.n_islands(),
        island_of: (0..n_bodies).map(|r| g.island_of(r)).collect(),
        colors: (0..g.n_colors()).map(|c| g.color(c).to_vec()).collect(),
        islands: (0..g.n_islands()).map(|i| g.island(i).to_vec()).collect(),
    }
}

// ── A proptest strategy: random manifold sets over a mix of dyn/static/sentinel ─

/// One random scene: `n_bodies` total rows, a random subset marked dynamic, and a
/// random list of manifolds whose endpoints are random rows OR the SDF sentinel.
#[derive(Clone, Debug)]
struct Scene {
    n_bodies: u32,
    dynamic_rows: HashSet<u32>,
    manifolds: Vec<Manifold>,
}

impl Scene {
    fn is_dynamic(&self) -> impl Fn(u32) -> bool + '_ {
        let set = self.dynamic_rows.clone();
        let n = self.n_bodies;
        move |row: u32| row < n && set.contains(&row)
    }
}

/// Generates a random scene: 0..=32 bodies (covers empty/1/all-coincident counts),
/// each body independently dynamic or static (~70% dynamic), and 0..=64 manifolds
/// whose endpoints are random rows in `[0, n_bodies)` OR the `u32::MAX` sentinel
/// (~10% of endpoints), so the ground rule + sentinel handling are exercised.
///
/// DOMAIN NOTE: production manifolds always have `body_a != body_b` — they come
/// from `ContactPairs`, which broadphase emits with `i < j` (a body never contacts
/// itself), and the SDF stage pairs a real body against the sentinel. A self-loop
/// `(row, row)` is therefore OUT of the partition's input domain; the production
/// `debug_assert_coloring` re-scan correctly flags a same-dynamic-body-twice color
/// (a self-loop would mean a body racing itself in the future colored solve). So
/// the strategy DROPS self-loops over real rows (a sentinel-sentinel degenerate
/// stays — `u32::MAX` is never a dynamic node, so it can never trip the assert).
fn arb_scene() -> impl Strategy<Value = Scene> {
    (0u32..=32).prop_flat_map(|n_bodies| {
        // Per-body dynamic flag.
        let dyn_flags = proptest::collection::vec(prop::bool::weighted(0.7), n_bodies as usize);
        // A row endpoint: a real row in range, or the sentinel.
        let endpoint = move || {
            if n_bodies == 0 {
                // No real rows — every endpoint is the sentinel (degenerate).
                Just(SDF_SENTINEL.0).boxed()
            } else {
                prop_oneof![
                    9 => 0..n_bodies,
                    1 => Just(SDF_SENTINEL.0),
                ]
                .boxed()
            }
        };
        // Drop self-loops over a REAL row (out of domain); a (sentinel, sentinel)
        // pair survives (the sentinel is never a dynamic node, so it is fine).
        let raw_pairs = proptest::collection::vec(
            (endpoint(), endpoint()).prop_filter(
                "no self-loop over a real row (out of the manifold domain)",
                |&(a, b)| a != b || a == SDF_SENTINEL.0,
            ),
            0..=64,
        );
        (Just(n_bodies), dyn_flags, raw_pairs).prop_map(|(n_bodies, flags, raw)| {
            let dynamic_rows: HashSet<u32> = flags
                .iter()
                .enumerate()
                .filter(|&(_, &d)| d)
                .map(|(i, _)| i as u32)
                .collect();
            let manifolds = raw.into_iter().map(|(a, b)| edge(a, b)).collect();
            Scene {
                n_bodies,
                dynamic_rows,
                manifolds,
            }
        })
    })
}

// ── Gate 1: coloring invariant on 1000 random graphs (load-bearing) ──────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// THE load-bearing O4 correctness gate: over 1000 random graphs (mixed
    /// dynamic/static/sentinel rows, varied body counts incl. 0/1/all-coincident),
    /// no color contains two manifolds sharing a dynamic body (full re-scan), and
    /// every manifold is colored exactly once.
    #[test]
    fn coloring_invariant_holds_on_random_graphs(scene in arb_scene()) {
        let is_dynamic = scene.is_dynamic();
        let mut g = ConstraintGraph::default();
        g.build(&scene.manifolds, scene.n_bodies as usize, &is_dynamic);
        if let Err(e) = check_coloring_invariant(&g, &scene.manifolds, &is_dynamic) {
            return Err(TestCaseError::fail(format!(
                "{e}\nSCENE: n_bodies={} dynamic={:?} manifolds={:?}",
                scene.n_bodies,
                scene.dynamic_rows,
                scene.manifolds.iter().map(|m| (m.body_a.0, m.body_b.0)).collect::<Vec<_>>(),
            )));
        }
    }
}

// ── Gate 2: island partition == independent BFS reference ────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// The build's `island_of` partition matches an independent BFS over the SAME
    /// ground rule (static/sentinel never connects). Same-component-ness, island
    /// count, and the NO_ISLAND contract all match the reference.
    #[test]
    fn islands_match_bfs_reference(scene in arb_scene()) {
        let is_dynamic = scene.is_dynamic();
        let mut g = ConstraintGraph::default();
        g.build(&scene.manifolds, scene.n_bodies as usize, &is_dynamic);
        if let Err(e) = check_islands_match_bfs(&g, &scene.manifolds, scene.n_bodies, &is_dynamic) {
            return Err(TestCaseError::fail(format!(
                "{e}\nSCENE: n_bodies={} dynamic={:?} manifolds={:?}",
                scene.n_bodies,
                scene.dynamic_rows,
                scene.manifolds.iter().map(|m| (m.body_a.0, m.body_b.0)).collect::<Vec<_>>(),
            )));
        }
    }
}

// ── Gate 7: CSR invariants (monotonicity / range / one-color-one-island) ─────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// CSR + range invariants on random graphs: every manifold appears in exactly
    /// one color and (if it has a dynamic side) exactly one island; ids in range;
    /// out-of-range accessors return empty.
    #[test]
    fn csr_invariants_hold_on_random_graphs(scene in arb_scene()) {
        let is_dynamic = scene.is_dynamic();
        let mut g = ConstraintGraph::default();
        g.build(&scene.manifolds, scene.n_bodies as usize, &is_dynamic);
        if let Err(e) = check_csr_invariants(&g, &scene.manifolds, &is_dynamic) {
            return Err(TestCaseError::fail(format!(
                "{e}\nSCENE: n_bodies={} dynamic={:?} manifolds={:?}",
                scene.n_bodies,
                scene.dynamic_rows,
                scene.manifolds.iter().map(|m| (m.body_a.0, m.body_b.0)).collect::<Vec<_>>(),
            )));
        }
    }
}

// ── Gate 3: partition bit-determinism (run-to-run + warm-reuse vs fresh) ──────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Same input → bit-identical partition, three ways: (a) two builds into a
    /// FRESH graph; (b) two builds into the SAME warmed graph (capacity reused);
    /// (c) the warmed-graph result equals the fresh-graph result. Proves the
    /// partition is a pure function of its input with NO stale-scratch leak.
    #[test]
    fn partition_is_bit_deterministic(scene in arb_scene()) {
        let is_dynamic = scene.is_dynamic();
        let n = scene.n_bodies as usize;

        // (a) Fresh graph, built once.
        let mut fresh = ConstraintGraph::default();
        fresh.build(&scene.manifolds, n, &is_dynamic);
        let snap_fresh = snapshot(&fresh, scene.n_bodies);

        // (b) A second fresh graph — run-to-run determinism.
        let mut fresh2 = ConstraintGraph::default();
        fresh2.build(&scene.manifolds, n, &is_dynamic);
        let snap_fresh2 = snapshot(&fresh2, scene.n_bodies);
        prop_assert_eq!(&snap_fresh, &snap_fresh2, "run-to-run on fresh graphs must be bit-identical");

        // (c) A warmed graph: build a DIFFERENT-shape scene first (a single big
        // clique over the first rows, all-dynamic) to dirty every scratch buffer,
        // then build the real scene on top. Result must equal the fresh build —
        // no stale scratch leaks across builds.
        let mut warm = ConstraintGraph::default();
        if scene.n_bodies >= 2 {
            let mut dirty = Vec::new();
            for a in 0..scene.n_bodies {
                for b in (a + 1)..scene.n_bodies {
                    dirty.push(edge(a, b));
                }
            }
            let all_dyn = |row: u32| row < scene.n_bodies;
            warm.build(&dirty, n, all_dyn);
        } else {
            // Degenerate: dirty with a sentinel-only manifold.
            warm.build(&[edge(SDF_SENTINEL.0, SDF_SENTINEL.0)], n, &is_dynamic);
        }
        // Now the real build on the warmed graph.
        warm.build(&scene.manifolds, n, &is_dynamic);
        let snap_warm = snapshot(&warm, scene.n_bodies);
        prop_assert_eq!(
            &snap_fresh, &snap_warm,
            "warm-buffer-reuse build must equal the fresh build (no stale-scratch leak)"
        );
    }
}

// ── Deterministic edge cases (gate-list explicit corners) ────────────────────

#[test]
fn empty_graph_is_empty_partition() {
    let mut g = ConstraintGraph::default();
    let is_dynamic = make_is_dynamic(HashSet::new(), 0);
    g.build(&[], 0, &is_dynamic);
    assert_eq!(g.n_colors(), 0, "no manifolds → no colors");
    assert_eq!(g.n_islands(), 0, "no bodies → no islands");
    assert_eq!(g.island_of(0), ConstraintGraph::NO_ISLAND, "no row 0 → NO_ISLAND");
    assert!(g.color(0).is_empty());
    assert!(g.island(0).is_empty());
}

#[test]
fn single_dynamic_body_no_manifolds_one_island_no_colors() {
    // One dynamic body, no contacts: it has NO island manifolds and there are no
    // colors, but the body is still its own (zero-manifold) island id 0 only if it
    // appears in a manifold. With no manifolds it is a NO_ISLAND singleton (no
    // edge files it under any island) — matches the BFS reference.
    let mut g = ConstraintGraph::default();
    let is_dynamic = make_is_dynamic([0u32].into_iter().collect(), 1);
    g.build(&[], 1, &is_dynamic);
    assert_eq!(g.n_colors(), 0, "no manifolds → no colors");
    // A dynamic row that participates in NO manifold gets an island id (it is its
    // own union-find root), but no manifolds file under it.
    let iof = g.island_of(0);
    assert_ne!(iof, ConstraintGraph::NO_ISLAND, "a dynamic singleton has an island id");
    assert!(g.island(iof).is_empty(), "but its island holds no manifolds");
}

#[test]
fn all_static_bodies_no_islands_one_color() {
    // Two static bodies touching: a static-static degenerate edge. Neither is
    // dynamic → no island (NO_ISLAND both), the edge files under no island, and it
    // still gets a color (it must appear in exactly one color — the partition is
    // total over manifolds).
    let mut g = ConstraintGraph::default();
    let is_dynamic = make_is_dynamic(HashSet::new(), 2); // both static
    let manifolds = [edge(0, 1)];
    g.build(&manifolds, 2, &is_dynamic);
    assert_eq!(g.n_islands(), 0, "no dynamic bodies → no islands");
    assert_eq!(g.island_of(0), ConstraintGraph::NO_ISLAND);
    assert_eq!(g.island_of(1), ConstraintGraph::NO_ISLAND);
    // The static-static edge still must be colored exactly once (total partition).
    assert_eq!(g.n_colors(), 1, "the one edge occupies one color");
    assert_eq!(g.color(0), &[0], "the static-static edge is in color 0");
    check_coloring_invariant(&g, &manifolds, &is_dynamic).expect("coloring invariant");
    check_csr_invariants(&g, &manifolds, &is_dynamic).expect("CSR invariants");
}

#[test]
fn sentinel_body_b_is_ground_one_color_for_many() {
    // Many dynamic bodies each contacting the SDF sentinel (body_b == u32::MAX).
    // The sentinel is ground (non-dynamic): it imposes no occupancy, so every
    // (dyn, sentinel) edge — sharing the sentinel — fits in ONE color, and the
    // sentinel never becomes an island node.
    let n = 10u32;
    let dyn_rows: HashSet<u32> = (0..n).collect();
    let is_dynamic = make_is_dynamic(dyn_rows, n);
    let manifolds: Vec<Manifold> = (0..n).map(|r| edge(r, SDF_SENTINEL.0)).collect();
    let mut g = ConstraintGraph::default();
    g.build(&manifolds, n as usize, &is_dynamic);

    assert_eq!(
        g.n_colors(),
        1,
        "shared sentinel ground imposes no occupancy → all in one color"
    );
    assert_eq!(g.n_islands(), n, "each dyn-vs-sentinel body is its own island");
    assert_eq!(
        g.island_of(SDF_SENTINEL.0),
        ConstraintGraph::NO_ISLAND,
        "the sentinel row is never an island node"
    );
    check_coloring_invariant(&g, &manifolds, &is_dynamic).expect("coloring invariant");
}

#[test]
fn full_clique_needs_n_minus_1_colors() {
    // A complete graph K_n over n dynamic bodies: every vertex has degree n-1, so a
    // greedy edge coloring of K_n needs at least n-1 colors (the chromatic index of
    // K_n is n-1 for even n, n for odd n; greedy first-fit here yields exactly the
    // per-vertex degree bound). Non-vacuity: forces a LARGE color count and a dense
    // re-scan of the invariant.
    let n = 8u32;
    let dyn_rows: HashSet<u32> = (0..n).collect();
    let is_dynamic = make_is_dynamic(dyn_rows, n);
    let mut manifolds = Vec::new();
    for a in 0..n {
        for b in (a + 1)..n {
            manifolds.push(edge(a, b));
        }
    }
    let mut g = ConstraintGraph::default();
    g.build(&manifolds, n as usize, &is_dynamic);

    assert_eq!(g.n_islands(), 1, "K_n is one connected island");
    assert!(
        g.n_colors() >= n - 1,
        "K_{n} needs >= n-1 colors, got {}",
        g.n_colors()
    );
    check_coloring_invariant(&g, &manifolds, &is_dynamic).expect("coloring invariant");
    check_islands_match_bfs(&g, &manifolds, n, &is_dynamic).expect("islands == BFS");
    check_csr_invariants(&g, &manifolds, &is_dynamic).expect("CSR invariants");
}

#[test]
fn duplicate_manifold_pairs_each_get_own_color() {
    // Two identical (0,1) edges (a duplicate contact between the same dynamic pair):
    // they share both dynamic bodies, so they CANNOT share a color — the second
    // forces a new color. Proves the coloring keys on bodies, not on distinct pairs.
    let n = 2u32;
    let dyn_rows: HashSet<u32> = (0..n).collect();
    let is_dynamic = make_is_dynamic(dyn_rows, n);
    let manifolds = [edge(0, 1), edge(0, 1)];
    let mut g = ConstraintGraph::default();
    g.build(&manifolds, n as usize, &is_dynamic);

    assert_eq!(g.n_colors(), 2, "duplicate same-body edges need distinct colors");
    check_coloring_invariant(&g, &manifolds, &is_dynamic).expect("coloring invariant");
}
