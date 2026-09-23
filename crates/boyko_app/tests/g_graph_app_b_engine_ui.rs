//! **G-GRAPH app (b) (plan gate UG-13, Tier A): the system inventory of the headless
//! `EnginePlugins` + UI app, and the structural half of ruling B5 — `ui_bind_apply` once, and the
//! UI composition adds only UI and input systems.**
//!
//! # What is measured, on this tree
//!
//! App (b) is `b2_common::app_b`: `EnginePlugins` plus TODAY's UI plugin composition, because the
//! design's `UiPlugins` (ED19) does not exist until UI2 (B2 plan correction PC3). Main's and Fixed's
//! system-name multisets are pinned in `b2_common::graph_pins::{MAIN_B, FIXED_B}` (exact equality,
//! re-derived with attribution), and B5's name-level half is checked:
//!
//! * `boyko_ui::binding::bind_system::ui_bind_apply` is scheduled exactly once;
//! * (b) − (a), against app (a)'s PINNED inventory, holds only `boyko_ui::` and `boyko_input::`
//!   systems, and (a) − (b) is empty — composing the UI neither added nor removed an engine system.
//!
//! # What is NOT measured (Tier B, not landed at B2)
//!
//! The exclusive set (target `{ui_bind_apply}`), `UiBindSet` after `GameplaySet` and the other
//! edges, and one writer per resource column need a read-only kernel seam (B2 plan correction PC2).
//! Today's exclusive systems in this app, found by reading their `fn(&mut EcsMaster)` signatures —
//! recorded in the B2 report, not pinned here, because a signature-derived classifier is a replica
//! of the kernel's kind rule: EIGHT of `boyko_ui`'s thirteen `fn(&mut EcsMaster)` systems —
//! `ui_bind_discovery`, `ui_bind_apply`, `ui_dispatch_system::<A>`,
//! `ui_refreeze_fixed_snapshot::<A>`, `ui_focus_system`, `ui_tween_reap`, `ui_bar_discovery` and
//! `ui_bar_apply` — plus the NonSend-param systems of the engine half. The other five
//! (`ui_layout_apply`, `ui_hot_reload_system`, and the three `ui_world_*` systems) are registered by
//! no plugin this app composes (layout by none at all; hot reload is off). That thirteen is plan
//! 00's "Exclusive UI systems 13" baseline, a crate-wide count; UG-13's app (b) schedules eight.
//!
//! # Red-first
//!
//! `BOYKO_B2_CANARY=g_graph_exclusive` must go RED through the name inventory (see app (a)'s file
//! for why that is not yet an exclusivity check).

mod b2_common;

use b2_common::graph_pins::{FIXED_A, FIXED_B, main_a, main_b};
use b2_common::{
    MESHES, UI_FIXTURE, app_b, assert_fixture_parses_clean, diff, extraction_self_test,
    fixed_len_is_positive, inventory, maybe_add_exclusive_canary, minus, new_app, order, owned,
    paste_ready, steal_and_build,
};

/// The one exclusive system B5 allows app (b).
const UI_BIND_APPLY: &str = "boyko_ui::binding::bind_system::ui_bind_apply";

#[test]
fn g_graph_app_b_engine_ui() {
    assert_fixture_parses_clean();
    let mut app = new_app();
    extraction_self_test(app.pool());
    app_b(&mut app, Some(UI_FIXTURE), MESHES);
    maybe_add_exclusive_canary(&mut app);
    let built = steal_and_build(&mut app);

    let main = inventory(&built.main);
    let fixed = inventory(&built.fixed);
    println!(
        "G-GRAPH (b) Main len={} distinct={}",
        built.main.len(),
        main.len()
    );
    for (n, c) in &main {
        println!("  main  ×{c} {n}");
    }
    println!(
        "G-GRAPH (b) Fixed len={} distinct={}",
        built.fixed.len(),
        fixed.len()
    );
    for (n, c) in &fixed {
        println!("  fixed ×{c} {n}");
    }
    println!("G-GRAPH (b) Main post-topological order (printed, not pinned):");
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

    // B5, name level.
    let bind_apply = main
        .iter()
        .find(|(n, _)| n == UI_BIND_APPLY)
        .map_or(0, |(_, c)| *c);
    println!("G-GRAPH (b) B5: `{UI_BIND_APPLY}` ×{bind_apply}");
    if bind_apply != 1 {
        violations.push(format!(
            "B5: `{UI_BIND_APPLY}` is scheduled ×{bind_apply}, expected ×1"
        ));
    }
    let a_main = main_a();
    let a_fixed = owned(FIXED_A);
    let added: Vec<(String, u32)> = minus(&main, &a_main)
        .into_iter()
        .chain(minus(&fixed, &a_fixed))
        .collect();
    let removed: Vec<(String, u32)> = minus(&a_main, &main)
        .into_iter()
        .chain(minus(&a_fixed, &fixed))
        .collect();
    println!(
        "G-GRAPH (b) − (a): {} system(s) added by the UI composition",
        added.iter().map(|x| x.1).sum::<u32>()
    );
    for (n, c) in &added {
        println!("  + ×{c} {n}");
        if !(n.starts_with("boyko_ui::") || n.starts_with("boyko_input::")) {
            violations.push(format!("B5: the UI composition added `{n}`, which is neither a boyko_ui nor a boyko_input system"));
        }
    }
    for (n, c) in &removed {
        violations.push(format!(
            "B5: the UI composition removed engine system `{n}` ×{c}"
        ));
    }

    violations.extend(diff("MAIN_B", &main, &main_b()));
    violations.extend(diff("FIXED_B", &fixed, &owned(FIXED_B)));

    if !violations.is_empty() {
        paste_ready("MAIN_B", &main);
        paste_ready("FIXED_B", &fixed);
        for v in &violations {
            println!("G-GRAPH VIOLATION: {v}");
        }
        panic!(
            "G-GRAPH (b): {} violation(s). A system added or removed by a merge is re-derived in that \
             merge's commit with the commit that added or removed it named; it is never pasted blind.",
            violations.len()
        );
    }
    println!(
        "G-GRAPH (b): GREEN — Main {} systems, Fixed {} systems, ui_bind_apply ×1, (b)−(a) UI/input \
         only; exclusive set, edges and one-writer NOT checked (Tier B, PC2)",
        built.main.len(),
        built.fixed.len()
    );
}
