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
//! Compiled only under the non-default `bp-query-counts` feature. Without it this file is
//! empty and `running 0 tests` is the expected output — it is not a pass of anything.
//!
//! ```text
//! cargo test -p boyko-physics --features bp-query-counts --test bp_query_counts -- --nocapture
//! ```
#![cfg(feature = "bp-query-counts")]

#[path = "../benches/support/bp_g4_scenes.rs"]
mod scenes;

use boyko_physics::broadphase_tree::counts::{QueryCounts, RowQueryCounts, TreeShape};
use boyko_physics::broadphase_tree::{BroadphaseTree, all_pairs_into};
use boyko_physics::resources::{BodyState, ContactPairs};
use boyko_physics::systems::body_bounding_radius;

/// With this variable set to a directory, each counted scene is also written there: the rows'
/// bits as the tree reads them and the replay's per-row counts, so an out-of-tree model of the
/// kernel can be checked against this replay row by row before it counts anything else.
const DUMP_DIR_VAR: &str = "BP_QUERY_COUNTS_DUMP";

use scenes::{J_SNAPSHOT_STEPS, TREE_WARM_STEPS, disparity_scene, j_snapshot, make_dynamic, scene};

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
}

/// Drives `bodies` to the bench's steady state and counts that step's query pass.
fn count(label: &str, bodies: &[BodyState]) -> Counted {
    let n = bodies.len();
    let mut oracle = ContactPairs::with_capacity(0);
    all_pairs_into(bodies, &mut oracle);
    assert!(!oracle.pairs().is_empty(), "anti-vacuity: {label} must produce pairs");

    let mut tree = BroadphaseTree::with_capacity(n);
    tree.set_brute_max_rows(0);
    let mut out = ContactPairs::with_capacity(0);
    // `TREE_WARM_STEPS` untimed steps, then one more: the step every timed iteration repeats.
    for _ in 0..=TREE_WARM_STEPS {
        tree.step_direct(bodies, &mut out);
    }
    assert_eq!(out.pairs(), oracle.pairs(), "{label}: the tree's steady-state pair set is AllPairs'");

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

    Counted {
        label: label.to_owned(),
        rows_total: n,
        pairs: out.pairs().len(),
        members: d.members,
        rows,
        static_pairs: totals.static_pairs,
        active: tree.active_shape(),
        statics: tree.static_shape(),
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

    let counted = [
        count(&format!("bp_g4_scene/tree/j{J_SNAPSHOT_STEPS}"), &snapshot),
        count("bp_g4_uniform/tree/1000", &uniform),
        count("bp_g4_disparity/tree/1000", &disparity),
    ];

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
}
