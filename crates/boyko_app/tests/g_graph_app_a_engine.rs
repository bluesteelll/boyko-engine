//! **G-GRAPH app (a) (plan gate UG-13, Tier A): the system inventory of a headless `EnginePlugins`
//! app, and the structural half of ruling B5 — no UI system without `UiPlugins`.**
//!
//! # What is measured, on this tree
//!
//! Main's and Fixed's system-name multisets and their `len()`, read from schedules built exactly
//! as `App::finish()` builds them (`b2_common::steal_and_build`). Pinned in
//! `b2_common::graph_pins::{MAIN_A, FIXED_A}`; the rule is exact equality, re-derived with
//! attribution. B5's structural half for this app: no system name under `boyko_ui::`.
//!
//! # What is NOT measured (Tier B, not landed at B2)
//!
//! The design's G-GRAPH also checks the exclusive-system set (target ∅ for this app), every set
//! and ordering edge, and one writer per resource column. None of that is reachable from outside
//! `boyko_ecs` on this tree — `SystemKind::runs_on_dispatcher` is `pub(crate)`, the systems' metas
//! sit in `pub(crate) systems`, the conflict graph is `pub(crate)`, and set membership is not kept
//! after build — so it needs a read-only kernel seam (B2 plan correction PC2). Pinning a replica of
//! the kernel's kind rule from signatures would check the replica, not the kernel, and is refused.
//!
//! # Red-first
//!
//! `BOYKO_B2_CANARY=g_graph_exclusive` adds one `fn(&mut EcsMaster)` to Main before the steal and
//! must go RED, naming the unexpected system. ⚠ It reds through the NAME inventory, not through an
//! exclusivity check: a concurrent canary reds identically. The in-binary self-test proves the
//! inventory reads real names.
//!
//! This process also prints app (a)'s id-census readings (`ID-CENSUS app=a`) at the config phase
//! and after the build — H(a), the term the derived union bound U subtracts.

mod b2_common;

use b2_common::census_scan::id_line;
use b2_common::graph_pins::{FIXED_A, main_a};
use b2_common::{
    MESHES, app_a, diff, extraction_self_test, fixed_len_is_positive, inventory,
    maybe_add_exclusive_canary, new_app, order, owned, paste_ready, steal_and_build,
};

#[test]
fn g_graph_app_a_engine() {
    let mut app = new_app();
    extraction_self_test(app.pool());
    app_a(&mut app, MESHES);
    maybe_add_exclusive_canary(&mut app);
    id_line("a", "config", 0);
    let built = steal_and_build(&mut app);
    id_line("a", "after_build", 1);

    let main = inventory(&built.main);
    let fixed = inventory(&built.fixed);
    println!(
        "G-GRAPH (a) Main len={} distinct={}",
        built.main.len(),
        main.len()
    );
    for (n, c) in &main {
        println!("  main  ×{c} {n}");
    }
    println!(
        "G-GRAPH (a) Fixed len={} distinct={}",
        built.fixed.len(),
        fixed.len()
    );
    for (n, c) in &fixed {
        println!("  fixed ×{c} {n}");
    }
    println!("G-GRAPH (a) Main post-topological order (printed, not pinned):");
    for (i, n) in order(&built.main).iter().enumerate() {
        println!("  {i:>3} {n}");
    }

    let mut violations: Vec<String> = Vec::new();
    if built.main.is_empty() {
        violations.push("ANTI-VACUITY: Main is empty".to_string());
    }
    if !fixed_len_is_positive(&built) {
        violations.push("ANTI-VACUITY: Fixed is empty (see steal_and_build's ⚠)".to_string());
    }
    let ui: Vec<&(String, u32)> = main
        .iter()
        .chain(&fixed)
        .filter(|(n, _)| n.starts_with("boyko_ui::"))
        .collect();
    println!("G-GRAPH (a) B5: UI systems = {}", ui.len());
    for (n, _) in &ui {
        violations.push(format!(
            "B5: app (a) composes no UI, yet `{n}` is scheduled"
        ));
    }
    violations.extend(diff("MAIN_A", &main, &main_a()));
    violations.extend(diff("FIXED_A", &fixed, &owned(FIXED_A)));

    if !violations.is_empty() {
        paste_ready("MAIN_A", &main);
        paste_ready("FIXED_A", &fixed);
        for v in &violations {
            println!("G-GRAPH VIOLATION: {v}");
        }
        panic!(
            "G-GRAPH (a): {} violation(s). A system added or removed by a merge is re-derived in that \
             merge's commit with the commit that added or removed it named; it is never pasted blind.",
            violations.len()
        );
    }
    println!(
        "G-GRAPH (a): GREEN — Main {} systems, Fixed {} systems, no UI system; exclusive set, edges \
         and one-writer NOT checked (Tier B, PC2)",
        built.main.len(),
        built.fixed.len()
    );
}
