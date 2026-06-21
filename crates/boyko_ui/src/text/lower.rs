//! Runtime lowering of a parsed `.ui` tree to the live world via `Commands`
//! (P3 §B, Decision 12).
//!
//! This is the exact runtime mirror of the `ui!` macro's `lower_node`
//! (`boyko_macros/src/lib.rs:3875-3971`), so a `.ui`-spawned tree is
//! byte/topology/archetype-identical to the equivalent `ui!`-spawned tree
//! (the initial-load equivalence gate). The macro arms are cited inline so a
//! future drift between the two paths is detectable by line:
//!
//! * spawn base — bundle fast path vs UiLayout-only inject
//!   (`lib.rs:3900-3938`); first-by-`position` for duplicate
//!   `UiLayout`/`ComputedRect` (`lib.rs:3894-3895`, Decision 12).
//! * chained inserts for the rest in declaration order (`lib.rs:3911-3930`).
//! * `#name` → `UiName` inserted LAST (`lib.rs:3940-3945`).
//! * children pre-order, parent's `add_child` after the child is materialised,
//!   so the FIFO drain orders the parent spawn before the child `ChildOf`
//!   insert (`lib.rs:3959-3961`).
//!
//! The ONLY component the loader stamps that the macro does not is the private
//! `UiSourceOrder` (Decision 11) — excluded from the equivalence gate's compared
//! set (Decision 12).

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use crate::bundles::UiNodeBundle;
use crate::components::{ComputedRect, UiLayout, UiName, UiSourceOrder};
use crate::reload::SmallRoots;
use crate::text::ast::{ParsedNode, ParsedTree};
use crate::text::dispatch::parse_and_insert;
use crate::text::report::UiParseReport;

/// Lower a parsed tree into the live world via `Commands`, producing the SAME
/// entity tree as the `ui!` macro (Decision 12 initial-load equivalence).
/// Returns the root entities in declaration order; stamps `UiSourceOrder`.
///
/// Per-node re-parse errors discovered during lowering (a bad field value, a node
/// missing its `UiLayout`) are recorded into the caller-supplied `report` so they
/// are REACHABLE (the lowering report must not be silently dropped). The value
/// grammar is identical to `parse_ui`'s, so a clean `parse_ui` implies a clean
/// lowering; callers typically pass `tree.report.clone()` (or a fresh report) and
/// inspect it after the call.
pub fn spawn_ui_tree(
    tree: &ParsedTree,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) -> SmallRoots {
    let mut roots = SmallRoots::new();
    for &root_idx in &tree.roots {
        if let Some(id) = lower_node(tree, root_idx, cmds, report) {
            roots.push(id);
        }
    }
    roots
}

/// Lower one node and its subtree (pre-order). Returns its spawned `Entity`, or
/// `None` if the node could not be spawned (no `UiLayout` — mirrors the macro's
/// validation-rejected case).
pub(crate) fn lower_node(
    tree: &ParsedTree,
    node_idx: usize,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) -> Option<Entity> {
    let node = &tree.nodes[node_idx];
    report.set_current_line(node.line_no);

    // 1. Spawn base: first-by-position UiLayout (required) + first ComputedRect
    //    (optional), mirroring the macro (Decision 12).
    let layout_idx = node.components.iter().position(|c| c.name == "UiLayout")?;
    let rect_idx = node.components.iter().position(|c| c.name == "ComputedRect");

    let layout = parse_layout(node, layout_idx, report);

    let id = if let Some(ri) = rect_idx {
        let rect = parse_rect(node, ri, report);
        // Bundle fast path (Phase-8.5 static-archetype cache): both present.
        cmds.spawn(UiNodeBundle { layout, rect }).id()
    } else {
        // UiLayout-only base; inject ComputedRect::default() (mirror lib.rs:3923).
        let id = cmds.spawn(layout).id();
        cmds.entity(id).insert(ComputedRect::default());
        id
    };

    // 2. Chained inserts for the rest in declaration order, skipping the FIRST
    //    UiLayout / ComputedRect (a duplicate is a normal insert, last-write-wins,
    //    mirroring the macro's insert chain — Decision 12).
    for (i, comp) in node.components.iter().enumerate() {
        if i == layout_idx || Some(i) == rect_idx {
            continue;
        }
        report.set_current_line(comp.line_no);
        // A per-component failure (unknown component, kind mismatch) is recorded
        // and dropped; siblings survive (Decision 6).
        let _ = parse_and_insert(comp, id, cmds, report);
    }

    // 3. #name → UiName LAST (mirror lib.rs:3940-3945).
    if let Some(name) = &node.name {
        cmds.entity(id).insert(UiName::new(&name.text));
    }

    // 4. UiSourceOrder stamp (Decision 11; gate-excluded, Decision 12).
    cmds.entity(id).insert(UiSourceOrder(node.sibling_ordinal));

    // 5. Children pre-order, then link (mirror lib.rs:3959-3961). The parent
    //    spawn was enqueued above before any child's ChildOf insert, so the FIFO
    //    drain materialises the parent first (dangling-parent guard passes).
    for &child_idx in &node.children {
        if let Some(child_id) = lower_node(tree, child_idx, cmds, report) {
            cmds.entity(id).add_child(child_id);
        }
    }

    Some(id)
}

/// Parses the node's first `UiLayout` body to a typed `UiLayout`. The component
/// is known to exist (the caller found its index).
fn parse_layout(node: &ParsedNode, idx: usize, report: &mut UiParseReport) -> UiLayout {
    let comp = &node.components[idx];
    report.set_current_line(comp.line_no);
    crate::text::dispatch::parse_ui_layout_public(&comp.body, comp.body_col, report)
}

/// Parses the node's first `ComputedRect` body to a typed `ComputedRect`.
fn parse_rect(node: &ParsedNode, idx: usize, report: &mut UiParseReport) -> ComputedRect {
    let comp = &node.components[idx];
    report.set_current_line(comp.line_no);
    crate::text::dispatch::parse_computed_rect_public(&comp.body, comp.body_col, report)
}
