//! **C3b of the tree broadphase: what one query does on the three G4 scenes** — the counting
//! driver of the query-cost investigation (`docs/measurements/2026-09-22-broadphase-tree/
//! analysis.md`, section 8, item 1).
//!
//! Window 4 measured the J-scale query at 334 ns per queried row where the design's arithmetic
//! has 100–150 (`levers/broadphase/04-DESIGN-REV2.md`: "~10–17 8-wide tests per query" and
//! 7.7 pairs per row). A wall clock cannot say which factor is off — how many tests a query
//! runs, or what one test costs — and this driver answers the first half with counts alone:
//! per query, the 8-wide box and exact tests, the children descended into, the leaf candidates,
//! the candidates a per-lane box test would keep, the exact hits and the emitted pairs; per
//! tree, the levels, the leaf occupancy, the child counts and the per-level box figures. It
//! prints them next to the design's figures. **No number it prints is a time.**
//!
//! The scenes are the bench's, from the module the bench itself includes
//! (`benches/support/bp_g4_scenes.rs`): `bp_g4_scene/tree/j100` (the J snapshot),
//! `bp_g4_uniform/tree/1000` and `bp_g4_disparity/tree/1000`. Each is driven to the bench's
//! steady state (`TREE_WARM_STEPS` untimed steps, then the step a timed iteration is) and its
//! query pass is counted by [`BroadphaseTree::count_query_pass`], which replays that step's pass
//! through the counting kernel and checks every row's emitted count against the step's segment.
//!
//! What it asserts is that the counts are of the step the bench times, not the counts' values:
//! the step's pair set is AllPairs'; the replayed pass, the wide rows and the static list add up
//! to the step's pair count; every row is a Q row or a member; and the walk's own identities hold
//! (every visited node but the root was descended into; the query row finds itself; a Q–Q pair
//! is found from both ends and emitted from one; `exact ≤ box hits ≤ candidates ≤ 8 · leaves`).
//!
//! **The leaf-list query (C3b, F1).** The step now runs [`QueryKernel::LeafList`], and the
//! replay above is the per-row walk it replaced, so the replay's figures are the "before". The
//! driver adds, per counted scene:
//! * G-LL1: the step's query stage re-run under both kernels writes the same bytes (the stream
//!   and every `(seg, nrev, nfwd)`), on the bench's own scenes;
//! * the pass's own counts ([`LeafListCounts`]: collection box tests, candidates, prefilter
//!   chunks, kept exact tests, emitted partners, sort shifts), printed beside the walk's;
//! * G-LL3: on the three G4 scenes the counts equal the design's model, which was validated
//!   row by row against this replay (`sim.py`, 0 mismatching rows of 1 240 / 1 000 / 1 004). The
//!   max-row cut is the pin's reason: the oracle and G-LL1 cannot see it (a candidate it drops
//!   emits nothing), and without it J keeps 20 377 exact tests, not 15 575. Since L10b C0 the
//!   model's J figures are pinned on the J snapshot with contact reuse off, the default's until
//!   L9 C4; the default snapshot's J pin is the tree's reading (`G_LL3_PINS`).
//!
//! A second test counts the same at the G4 small sizes (17, 64, 128, 256), over a scene with a
//! multi-leaf static tree, and at 10k and 100k (the leaf list's fallback count; no oracle there),
//! the C3b review's W5.
//!
//! Compiled only under the non-default `bp-query-counts` feature. Without it this file is
//! empty and `running 0 tests` is the expected output — it is not a pass of anything.
//!
//! ```text
//! cargo test -p boyko-physics --features bp-query-counts --test bp_query_counts -- --nocapture
//! ```
#![cfg(feature = "bp-query-counts")]

#[path = "../benches/support/bp_g4_scenes.rs"]
mod scenes;

use boyko_physics::broadphase_tree::counts::{LeafListCounts, QueryCounts, RowQueryCounts, TreeShape};
use boyko_physics::broadphase_tree::{BroadphaseTree, QueryKernel, all_pairs_into};
use boyko_physics::resources::{BodyState, ContactPairs};
use boyko_physics::systems::body_bounding_radius;

/// With this variable set to a directory, each counted scene is also written there: the rows'
/// bits as the tree reads them and the replay's per-row counts, so an out-of-tree model of the
/// kernel can be checked against this replay row by row before it counts anything else.
const DUMP_DIR_VAR: &str = "BP_QUERY_COUNTS_DUMP";

use scenes::{
    J_SNAPSHOT_STEPS, TREE_WARM_STEPS, disparity_scene, j_snapshot, j_snapshot_with_reuse,
    make_dynamic, scene,
};

/// The design's 8-wide tests per query at J (`04-DESIGN-REV2.md`, the Query row of the
/// critical-path table): "~10–17".
const DESIGN_TESTS_PER_QUERY: (f64, f64) = (10.0, 17.0);

/// The design's pairs per row (`04-DESIGN-REV2.md`, D3.5: `|L| ≈ 7.7·m`), J's 9 559 pairs over
/// its 1 240 boxes.
const DESIGN_PAIRS_PER_ROW: f64 = 7.7;

/// One metric's distribution over a pass's queried rows.
struct Dist {
    mean: f64,
    p50: u32,
    p95: u32,
    max: u32,
}

/// Mean, nearest-rank p50 and p95, and max of `values`.
fn dist(mut values: Vec<u32>) -> Dist {
    assert!(!values.is_empty(), "anti-vacuity: a distribution over no rows");
    values.sort_unstable();
    let n = values.len();
    let rank = |p: usize| values[(p * n).div_ceil(100) - 1];
    let sum: u64 = values.iter().map(|&v| u64::from(v)).sum();
    Dist { mean: sum as f64 / n as f64, p50: rank(50), p95: rank(95), max: values[n - 1] }
}

/// Reads one metric off a walk.
type Metric = fn(&QueryCounts) -> u32;

/// The metrics of one walk, in print order.
const METRICS: [(&str, Metric); 7] = [
    ("8-wide tests (box + exact)", QueryCounts::tests),
    ("  box tests (internal nodes)", |c| c.box_tests),
    ("  exact tests (leaf nodes)", |c| c.leaf_tests),
    ("nodes descended (box-lane hits)", |c| c.descents),
    ("leaf candidates (live lanes tested)", |c| c.leaf_candidates),
    ("leaf candidates a lane box would keep", |c| c.leaf_box_hits),
    ("exact hits (accept calls)", |c| c.exact_hits),
];

/// A counted scene.
struct Counted {
    label: String,
    rows_total: usize,
    pairs: usize,
    members: u64,
    rows: Vec<RowQueryCounts>,
    static_pairs: u64,
    active: TreeShape,
    statics: TreeShape,
    /// The step's leaf-list pass.
    leaf_list: LeafListCounts,
    /// Active leaf nodes the step's pass handed to the per-row walk (a collection over the cap).
    fallback_leaves: u64,
}

/// Copies a query stage's bytes out of the tree (the library hands them out borrowed), so the
/// two kernels' stages can be held side by side.
fn copy_stage<I: Iterator<Item = (u32, u32, u32)>>((stream, records): (&[u32], I)) -> (Vec<u32>, Vec<(u32, u32, u32)>) {
    (stream.to_vec(), records.collect())
}

/// Drives `bodies` to the bench's steady state and counts that step's query pass, checking the
/// step's pair set against AllPairs when `oracle` is set (not at 100k rows).
fn count(label: &str, bodies: &[BodyState], oracle: bool) -> Counted {
    let n = bodies.len();
    let mut want = ContactPairs::with_capacity(0);
    if oracle {
        all_pairs_into(bodies, &mut want);
        assert!(!want.pairs().is_empty(), "anti-vacuity: {label} must produce pairs");
    }

    let mut tree = BroadphaseTree::with_capacity(n);
    tree.set_brute_max_rows(0);
    assert_eq!(tree.query_kernel(), QueryKernel::LeafList, "the default kernel is the leaf list");
    let mut out = ContactPairs::with_capacity(0);
    // `TREE_WARM_STEPS` untimed steps, then one more: the step every timed iteration repeats.
    let mut fallback_before = 0;
    for step in 0..=TREE_WARM_STEPS {
        if step == TREE_WARM_STEPS {
            fallback_before = tree.diag().fallback_leaves;
        }
        tree.step_direct(bodies, &mut out);
    }
    if oracle {
        assert_eq!(out.pairs(), want.pairs(), "{label}: the tree's steady-state pair set is AllPairs'");
    }
    assert!(!out.pairs().is_empty(), "anti-vacuity: {label} must produce pairs");
    let fallback_leaves = tree.diag().fallback_leaves - fallback_before;

    // G-LL1 on the bench's scene: the step's query stage under each kernel, byte for byte. The
    // leaf-list re-run leaves the pass's counts as the step's.
    let leaf_list_stage = copy_stage(tree.query_stage(QueryKernel::LeafList));
    let row_walk_stage = copy_stage(tree.query_stage(QueryKernel::RowWalk));
    assert!(!leaf_list_stage.0.is_empty(), "anti-vacuity: {label}'s stream holds segments");
    assert_eq!(leaf_list_stage.0, row_walk_stage.0, "{label}: G-LL1, the leaf list's stream is the per-row walk's");
    assert_eq!(leaf_list_stage.1, row_walk_stage.1, "{label}: G-LL1, the leaf list's records are the per-row walk's");
    let leaf_list = tree.leaf_list_counts();

    let mut rows = Vec::with_capacity(n);
    let totals = tree.count_query_pass(|r| rows.push(r));
    let d = tree.diag();
    assert_eq!(totals.rows as usize, rows.len(), "{label}: one record per replayed row");
    assert!(totals.rows > 0, "anti-vacuity: {label} replayed no query");
    assert_eq!(
        totals.pairs(),
        out.pairs().len() as u64,
        "{label}: replayed emissions + wide emissions + the static list are the step's pairs"
    );
    assert_eq!((d.wide_rows, d.excluded_rows), (0, 0), "{label}: every row is Normal");
    assert_eq!(
        u64::from(totals.rows) + d.members,
        n as u64,
        "{label}: every row is a Q row or a member"
    );

    let mut exact_active = 0u64;
    let mut emitted_active = 0u64;
    for r in &rows {
        for c in [&r.active, &r.statics] {
            if c.tests() > 0 {
                assert_eq!(c.tests(), c.descents + 1, "{label} row {}: every node but the root was descended into", r.row);
            }
            assert!(
                c.exact_hits <= c.leaf_box_hits
                    && c.leaf_box_hits <= c.leaf_candidates
                    && c.leaf_candidates <= 8 * c.leaf_tests,
                "{label} row {}: exact ≤ box hits ≤ candidates ≤ 8·leaves: {c:?}",
                r.row
            );
        }
        assert!(r.active.exact_hits >= 1, "{label} row {}: a Q row finds itself", r.row);
        assert_eq!(r.emitted_statics, r.statics.exact_hits, "{label} row {}: static partners are all kept", r.row);
        exact_active += u64::from(r.active.exact_hits);
        emitted_active += u64::from(r.emitted_active);
    }
    assert_eq!(
        exact_active,
        u64::from(totals.rows) + 2 * emitted_active,
        "{label}: each Q row finds itself once and each Q-Q pair from both ends"
    );

    if let Ok(dir) = std::env::var(DUMP_DIR_VAR) {
        dump(std::path::Path::new(&dir), label, bodies, &rows);
    }

    // The pass's counts are of the same step: every Q row outside a fallback leaf emitted what
    // the walk emits.
    if fallback_leaves == 0 {
        assert_eq!(leaf_list.leaves, u64::from(totals.rows.div_ceil(8)), "{label}: every active leaf node was answered by the leaf list");
        assert_eq!(leaf_list.rows, u64::from(totals.rows), "{label}: every Q row was answered by the leaf list");
        assert_eq!(leaf_list.emitted, totals.emitted, "{label}: the leaf list emitted the walk's partners");
    }

    Counted {
        label: label.to_owned(),
        rows_total: n,
        pairs: out.pairs().len(),
        members: d.members,
        rows,
        static_pairs: totals.static_pairs,
        active: tree.active_shape(),
        statics: tree.static_shape(),
        leaf_list,
        fallback_leaves,
    }
}

/// The design's model of the leaf-list pass on the three G4 scenes (`sim.py`, C3b design §7,
/// G-LL3): collection box tests (active tree), active candidates, static candidates, prefilter
/// chunks, kept exact tests (static included), emitted partners.
///
/// **Re-pinned at L10b C0; reason: L9 C4: contact reuse on by default.** [`j_snapshot`] takes the
/// default configuration, so C4 moved the J snapshot's [`J_SNAPSHOT_STEPS`]-step trajectory and
/// J's figures with it. C4 did not re-pin this feature-gated leg, which compiles to `running 0
/// tests` in the workspace run. All three scenes were re-derived by this test's own rule, the
/// G-LL3 assert below, from its `left` line, on msvc release and debug, with
/// `cargo test -p boyko-physics --features bp-query-counts --test bp_query_counts`:
/// * `bp_g4_scene/tree/j100`: old `[1953, 5663, 155, 6216, 15575, 9564]`, new
///   `[1983, 5787, 155, 6368, 16120, 9549]`. The new figures are the tree's reading on the
///   reuse-on snapshot. `sim.py` lives out of tree and was not re-run, so they are not the
///   model's. The old figures are [`G_LL3_J_REUSE_OFF`], which the reuse-off snapshot must still
///   match bit for bit.
/// * `bp_g4_uniform/tree/1000` and `bp_g4_disparity/tree/1000`: re-derived and unchanged. Their
///   bodies are built directly, with no schedule, so contact reuse cannot reach them.
///
/// **Re-pin rule.** The figures move only with a commit that changes the J trajectory or the
/// leaf-list kernel by design. That commit re-reads all three from the G-LL3 assert's `left`
/// line and records old, new, the command and the reason here. In a commit that claims bit
/// identity, a change is a defect, never a re-pin.
const G_LL3_PINS: [(&str, [u64; 6]); 3] = [
    ("bp_g4_scene/tree/j100", [1983, 5787, 155, 6368, 16120, 9549]),
    ("bp_g4_uniform/tree/1000", [1228, 3465, 0, 3848, 7200, 2400]),
    ("bp_g4_disparity/tree/1000", [1487, 4096, 0, 4560, 11252, 6706]),
];

/// G-LL3 on the J snapshot with contact reuse OFF (`j_snapshot_with_reuse(Some(false))`): the
/// exact narrowphase's trajectory, the default's until L9 C4. These are the design model's
/// figures (`sim.py`) that [`G_LL3_PINS`] held for J until L10b C0, and this snapshot must
/// still reproduce them bit for bit.
///
/// **Re-pin rule.** [`G_LL3_PINS`]'s, except that no contact-reuse change may move it: with reuse
/// off no reuse code runs.
const G_LL3_J_REUSE_OFF: (&str, [u64; 6]) =
    ("bp_g4_scene/tree/j100/reuse-off", [1953, 5663, 155, 6216, 15575, 9564]);

/// The six G-LL3 figures of a pass.
fn g_ll3_figures(c: &LeafListCounts) -> [u64; 6] {
    [
        c.collect_box_active,
        c.cands_active,
        c.cands_static,
        c.prefilter_chunks,
        c.kept_active + c.kept_static,
        c.emitted,
    ]
}

/// The before/after table: per queried row, the per-row walk's work (the replay, every Q row)
/// against the leaf-list pass's (its own counts, per row it answered: a fallback leaf's rows are
/// the walk's).
fn print_before_after(counted: &[Counted]) {
    println!("\n## The per-row walk (before, every Q row) against the leaf-list pass (after, per row it answered)\n");
    println!(
        "| scene | Q rows | members | static levels | fallback leaves | walk: 8-wide tests (box + exact) | walk: exact tests | leaf list: collection box tests | leaf list: prefilter chunks | leaf list: kept exact tests (static) | leaf list: 8-wide tests | tests after / before | candidates / active leaf | emitted | sort shifts |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for c in counted {
        let rows = c.rows.len() as f64;
        let walk_tests: u64 = c.rows.iter().map(|r| u64::from(r.both().tests())).sum();
        let walk_exact: u64 = c.rows.iter().map(|r| u64::from(r.both().leaf_tests)).sum();
        let ll = &c.leaf_list;
        let ll_rows = ll.rows as f64;
        let ll_tests = ll.collect_box_active + ll.collect_box_static + ll.prefilter_chunks + ll.kept_active + ll.kept_static;
        println!(
            "| {} | {} | {} | {} | {} | {:.2} | {:.2} | {:.3} | {:.3} | {:.2} ({:.2}) | {:.2} | {:.3} | {:.2} | {:.3} | {:.2} |",
            c.label,
            c.rows.len(),
            c.members,
            c.statics.levels,
            c.fallback_leaves,
            walk_tests as f64 / rows,
            walk_exact as f64 / rows,
            (ll.collect_box_active + ll.collect_box_static) as f64 / ll_rows,
            ll.prefilter_chunks as f64 / ll_rows,
            (ll.kept_active + ll.kept_static) as f64 / ll_rows,
            ll.kept_static as f64 / ll_rows,
            ll_tests as f64 / ll_rows,
            (ll_tests as f64 / ll_rows) / (walk_tests as f64 / rows),
            if ll.leaves > 0 { ll.cands_active as f64 / ll.leaves as f64 } else { 0.0 },
            ll.emitted as f64 / ll_rows,
            ll.sort_shifts as f64 / ll_rows,
        );
    }
}

/// Writes `<label>.rows.bin` (per body row, little-endian: the bits of `x`, `y`, `z` and
/// `body_bounding_radius`, then `1` if the row was queried, else `0`: 20 B a row) and
/// `<label>.counts.csv` (the replay's per-row counts, in replay order = active-leaf slot order)
/// into `dir`, with `/` in the label replaced by `_`.
fn dump(dir: &std::path::Path, label: &str, bodies: &[BodyState], rows: &[RowQueryCounts]) {
    let slug = label.replace('/', "_");
    let mut queried = vec![0u32; bodies.len()];
    for r in rows {
        queried[r.row as usize] = 1;
    }
    let mut bin = Vec::with_capacity(bodies.len() * 20);
    for (body, &q) in bodies.iter().zip(&queried) {
        let p = body.position;
        for v in [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), body_bounding_radius(body).to_bits(), q] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(dir.join(format!("{slug}.rows.bin")), bin).expect("dump: rows.bin is writable");
    let mut csv = String::from(
        "row,a_box,a_leaf,a_desc,a_cand,a_lanebox,a_exact,a_emit,s_box,s_leaf,s_desc,s_cand,s_lanebox,s_exact,s_emit\n",
    );
    for r in rows {
        let (a, s) = (&r.active, &r.statics);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.row,
            a.box_tests,
            a.leaf_tests,
            a.descents,
            a.leaf_candidates,
            a.leaf_box_hits,
            a.exact_hits,
            r.emitted_active,
            s.box_tests,
            s.leaf_tests,
            s.descents,
            s.leaf_candidates,
            s.leaf_box_hits,
            s.exact_hits,
            r.emitted_statics,
        ));
    }
    std::fs::write(dir.join(format!("{slug}.counts.csv")), csv).expect("dump: counts.csv is writable");
}

/// One `| metric | mean | p50 | p95 | max |` row.
fn line(name: &str, values: Vec<u32>) -> String {
    let d = dist(values);
    format!("| {name} | {:.2} | {} | {} | {} |", d.mean, d.p50, d.p95, d.max)
}

/// The per-query tables of a counted scene: both trees, then each tree.
fn print_queries(c: &Counted) {
    let pick = |f: fn(&RowQueryCounts) -> QueryCounts| c.rows.iter().map(f).collect::<Vec<_>>();
    let walks: [(&str, Vec<QueryCounts>); 3] = [
        ("both trees (one queried row)", pick(RowQueryCounts::both)),
        ("active tree (the Q rows)", pick(|r| r.active)),
        ("static tree (the members)", pick(|r| r.statics)),
    ];
    let emitted: [Vec<u32>; 3] = [
        c.rows.iter().map(RowQueryCounts::emitted).collect(),
        c.rows.iter().map(|r| r.emitted_active).collect(),
        c.rows.iter().map(|r| r.emitted_statics).collect(),
    ];
    for ((title, walk), emitted) in walks.iter().zip(emitted) {
        println!("\n**{}: per query, {title}** ({} queries)\n", c.label, walk.len());
        println!("| metric | mean | p50 | p95 | max |");
        println!("|---|---|---|---|---|");
        for (name, f) in METRICS {
            println!("{}", line(name, walk.iter().map(f).collect()));
        }
        println!("{}", line("emitted pairs (into the segment)", emitted));
    }
}

/// The shape table of one tree.
fn print_shape(label: &str, which: &str, s: &TreeShape) {
    println!(
        "\n**{label}: {which} tree** — levels {}, leaf slots {}, live {}",
        s.levels, s.leaf_slots, s.live
    );
    if s.levels == 0 {
        println!("\n(empty)");
        return;
    }
    println!("\nleaf occupancy (leaf nodes by live lanes 0..8): {:?}", s.leaf_occupancy);
    println!("internal child counts (internal nodes by non-empty lanes 0..8): {:?}", s.child_counts);
    println!(
        "\n| level | nodes | children | children/node | Σ child area / Σ node area | mean ratio | max ratio | Σ child vol / Σ node vol |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for (level, l) in s.level.iter().enumerate().take(s.levels as usize) {
        println!(
            "| {level}{} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            if level == 0 { " (leaves over rows)" } else { "" },
            l.nodes,
            l.children,
            f64::from(l.children) / f64::from(l.nodes),
            l.area_ratio(),
            l.area_ratio_mean,
            l.area_ratio_max,
            l.volume_ratio(),
        );
    }
}

#[test]
fn query_counts_on_the_g4_scenes() {
    let mut uniform = scene(1_000);
    make_dynamic(&mut uniform);
    let mut disparity = disparity_scene(1_000);
    make_dynamic(&mut disparity);
    let snapshot = j_snapshot();
    let snapshot_reuse_off = j_snapshot_with_reuse(Some(false));

    let counted = [
        count(&format!("bp_g4_scene/tree/j{J_SNAPSHOT_STEPS}"), &snapshot, true),
        count("bp_g4_uniform/tree/1000", &uniform, true),
        count("bp_g4_disparity/tree/1000", &disparity, true),
    ];

    // G-LL3: the pass's counts are the design model's.
    for (c, (label, pins)) in counted.iter().zip(G_LL3_PINS) {
        assert_eq!(c.label, label, "the pins are in scene order");
        assert_eq!(c.fallback_leaves, 0, "{label}: no fallback at 1 000 rows");
        assert_eq!(c.leaf_list.collect_box_static, 0, "{label}: a one-level or empty static tree needs no box test");
        assert_eq!(
            g_ll3_figures(&c.leaf_list),
            pins,
            "{label}: G-LL3, [collection box tests, active candidates, static candidates, prefilter chunks, kept exact tests, emitted] = the model's; full counts {:?}",
            c.leaf_list
        );
    }

    // G-LL3 on the reuse-off J snapshot: the pre-C4 figures, bit for bit.
    let (label, pins) = G_LL3_J_REUSE_OFF;
    let reuse_off = count(&format!("bp_g4_scene/tree/j{J_SNAPSHOT_STEPS}/reuse-off"), &snapshot_reuse_off, true);
    assert_eq!(reuse_off.label, label, "the reuse-off pin names its scene");
    assert_eq!(reuse_off.fallback_leaves, 0, "{label}: no fallback at 1 241 rows");
    assert_eq!(reuse_off.leaf_list.collect_box_static, 0, "{label}: a one-level static tree needs no box test");
    assert_eq!(
        g_ll3_figures(&reuse_off.leaf_list),
        pins,
        "{label}: G-LL3 with contact reuse off, the pre-C4 figures; full counts {:?}",
        reuse_off.leaf_list
    );
    // Anti-vacuity: the reuse setting reaches the snapshot, so the two J arms count different
    // bodies.
    assert_ne!(
        g_ll3_figures(&counted[0].leaf_list),
        pins,
        "anti-vacuity: the default J snapshot (contact reuse on) differs from the reuse-off one"
    );

    println!("\n## Summary against the design (04-DESIGN-REV2.md)\n");
    println!(
        "| scene | rows | Q rows | members | pairs | static-list pairs | 8-wide tests / query (mean) | vs design 10 / 17 | emitted pairs / Q row | vs design 7.7 | active levels | static levels |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    for c in &counted {
        let tests = dist(c.rows.iter().map(|r| r.both().tests()).collect()).mean;
        let emitted = dist(c.rows.iter().map(RowQueryCounts::emitted).collect()).mean;
        println!(
            "| {} | {} | {} | {} | {} | {} | {:.2} | {:.2}x / {:.2}x | {:.3} | {:.2}x | {} | {} |",
            c.label,
            c.rows_total,
            c.rows.len(),
            c.members,
            c.pairs,
            c.static_pairs,
            tests,
            tests / DESIGN_TESTS_PER_QUERY.0,
            tests / DESIGN_TESTS_PER_QUERY.1,
            emitted,
            emitted / DESIGN_PAIRS_PER_ROW,
            c.active.levels,
            c.statics.levels,
        );
    }
    for c in &counted {
        println!("\n## {}", c.label);
        print_queries(c);
        print_shape(&c.label, "active", &c.active);
        print_shape(&c.label, "static", &c.statics);
    }
    print_before_after(&counted);
    for c in counted.iter().chain([&reuse_off]) {
        println!("\n{}: leaf-list pass {:?}", c.label, c.leaf_list);
    }
}

/// The C3b review's W5: the same counts at the G4 small sizes (where the brute threshold and the
/// AUTO crossovers are read), over a scene whose static tree has several leaf nodes (the shape
/// of the compaction and admission cells), and at 10k and 100k rows, where the leaf list's
/// fallback count is the question (no oracle at 100k).
#[test]
fn leaf_list_counts_at_the_small_and_large_sizes_and_over_a_multi_leaf_static_tree() {
    let mut counted = Vec::new();
    for n in [17usize, 64, 128, 256, 10_000, 100_000] {
        let oracle = n <= 10_000;
        let mut uniform = scene(n);
        make_dynamic(&mut uniform);
        counted.push(count(&format!("bp_g4_uniform/tree/{n}"), &uniform, oracle));
        let mut disparity = disparity_scene(n);
        make_dynamic(&mut disparity);
        counted.push(count(&format!("bp_g4_disparity/tree/{n}"), &disparity, oracle));
    }
    // Every fourth body of the uniform lattice dynamic, the rest static: after the warm-up the
    // statics are members and the dynamics query a static tree of many leaf nodes.
    for n in [256usize, 1_000] {
        let mut mixed = scene(n);
        for (i, body) in mixed.iter_mut().enumerate() {
            body.inv_mass = if i % 4 == 0 { 1.0 } else { 0.0 };
        }
        let c = count(&format!("mixed_static/tree/{n}"), &mixed, true);
        assert!(c.statics.levels >= 2, "{}: a static tree with internal nodes", c.label);
        assert!(c.leaf_list.collect_box_static > 0 && c.leaf_list.cands_static > 0, "{}: the static collection ran", c.label);
        counted.push(c);
    }
    print_before_after(&counted);
    for c in &counted {
        println!("\n{}: leaf-list pass {:?}", c.label, c.leaf_list);
    }
}
