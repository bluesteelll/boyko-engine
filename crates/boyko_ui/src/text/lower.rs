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
use crate::text::ast::{CompKind, ParsedComponent, ParsedNode, ParsedTree};
use crate::text::dispatch::{parse_and_insert, parse_bind_text, parse_bind_value, BindParse};
use crate::text::report::UiParseReport;

/// A deferred `.ui` data-bind whose `source` was authored as a `#name` reference
/// (GUI #27). The whole component insert is deferred to pass 2 (no sentinel
/// reaches the world): the `#name` is resolved against the load-local name index
/// AFTER every node has spawned, so a FORWARD reference (a binding to a node
/// declared later) resolves identically to a backward one.
enum BindFixup {
    /// A `BindText` awaiting its source entity.
    Text {
        widget: Entity,
        comp: crate::binding::components::BindText,
        name: UiName,
        line_no: usize,
        body_col: u16,
    },
    /// A `BindValue` awaiting its source entity.
    Value {
        widget: Entity,
        comp: crate::binding::components::BindValue,
        name: UiName,
        line_no: usize,
        body_col: u16,
    },
}

/// The transient two-pass bind-resolution context (GUI #27), cold load-local.
///
/// `names` is the `#name → Entity` index built AS nodes spawn (the same
/// sorted-`Vec` + binary-search shape the reconcile's `build_global_named_index`
/// uses — Principle 0, one name-index structure). `fixups` are the deferred
/// `#name`-source binds resolved against `names` in pass 2. On the reload path the
/// caller pre-seeds `names` with the live survivors so a new subtree can bind to
/// an existing node.
pub(crate) struct BindCtx {
    /// `(UiName, Entity)` pairs; sorted before pass-2 binary search.
    names: Vec<(UiName, Entity)>,
    /// The deferred `#name`-source binds.
    fixups: Vec<BindFixup>,
}

impl BindCtx {
    /// A fresh empty context (the initial-load path).
    #[inline]
    pub(crate) fn new() -> Self {
        Self { names: Vec::new(), fixups: Vec::new() }
    }

    /// A context pre-seeded with the live survivors' names (the reload path), so a
    /// newly-spawned subtree can bind a `#name` to an existing live node.
    #[inline]
    pub(crate) fn with_seed(seed: &[(UiName, Entity)]) -> Self {
        Self { names: seed.to_vec(), fixups: Vec::new() }
    }

    /// Records a node's `#name → Entity` mapping as it spawns (pass 1). On the
    /// reload path the reconcile also records SURVIVOR names through this so a
    /// `#name` source can resolve to a matched survivor.
    #[inline]
    pub(crate) fn record_name(&mut self, name: UiName, entity: Entity) {
        self.names.push((name, entity));
    }

    /// Defers a `BindText` whose `#name` source resolves in pass 2 (the reload
    /// survivor-patch path uses this to re-resolve a named source).
    #[inline]
    pub(crate) fn defer_text(
        &mut self,
        widget: Entity,
        comp: crate::binding::components::BindText,
        name: UiName,
        line_no: usize,
        body_col: u16,
    ) {
        self.fixups.push(BindFixup::Text { widget, comp, name, line_no, body_col });
    }

    /// Defers a `BindValue` whose `#name` source resolves in pass 2.
    #[inline]
    pub(crate) fn defer_value(
        &mut self,
        widget: Entity,
        comp: crate::binding::components::BindValue,
        name: UiName,
        line_no: usize,
        body_col: u16,
    ) {
        self.fixups.push(BindFixup::Value { widget, comp, name, line_no, body_col });
    }

    /// Resolves every deferred `#name`-source bind against the spawned name index
    /// (pass 2), inserting the completed component on a hit and recording a
    /// recoverable per-line error on an unknown name (the component is then never
    /// inserted — no sentinel reaches the world).
    pub(crate) fn resolve(self, cmds: &mut Commands, report: &mut UiParseReport) {
        let mut names = self.names;
        names.sort_by_key(|(k, _)| *k);
        for fixup in self.fixups {
            match fixup {
                BindFixup::Text { widget, mut comp, name, line_no, body_col } => {
                    match lookup_name(&names, name) {
                        Some(src) => {
                            comp.source = src;
                            cmds.entity(widget).insert(comp);
                        }
                        None => unknown_source(name, line_no, body_col, report),
                    }
                }
                BindFixup::Value { widget, mut comp, name, line_no, body_col } => {
                    match lookup_name(&names, name) {
                        Some(src) => {
                            comp.source = src;
                            cmds.entity(widget).insert(comp);
                        }
                        None => unknown_source(name, line_no, body_col, report),
                    }
                }
            }
        }
    }
}

/// Binary-searches the sorted name index; a `debug_assert!` cross-checks the
/// hit against a linear scan (the reconcile's `UiName` Ord invariant). For a
/// duplicated name the binary search may land anywhere in the run; this is a
/// document-authoring bug surfaced elsewhere (the reconcile's duplicate-name
/// rule), tolerated here by resolving to whichever hit the search returns.
fn lookup_name(names: &[(UiName, Entity)], key: UiName) -> Option<Entity> {
    let hit = names.binary_search_by(|(k, _)| k.cmp(&key)).ok().map(|i| names[i].1);
    debug_assert_eq!(
        hit.is_some(),
        names.iter().any(|(k, _)| *k == key),
        "invariant: binary-search hit matches linear-scan presence (UiName Ord)"
    );
    hit
}

/// Records a recoverable unknown-`#name`-source error (the rest of the file still
/// loads; the bind component is simply never inserted).
#[cold]
#[inline(never)]
fn unknown_source(name: UiName, line_no: usize, body_col: u16, report: &mut UiParseReport) {
    report.error(line_no, body_col, format!("unknown #name source {:?}", name.as_str()));
}

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
    // GUI #27: two-pass. Pass 1 spawns every node, recording `#name → Entity` and
    // deferring any `#name`-source bind; pass 2 resolves the deferred binds
    // (forward references resolve because all names are recorded before any is
    // resolved).
    let mut ctx = BindCtx::new();
    for &root_idx in &tree.roots {
        if let Some(id) = lower_node(tree, root_idx, cmds, report, &mut ctx) {
            roots.push(id);
        }
    }
    ctx.resolve(cmds, report);
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
    ctx: &mut BindCtx,
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
    //    mirroring the macro's insert chain — Decision 12). The data-bind
    //    components are routed through the two-pass bind path (GUI #27), not the
    //    generic dispatch, because a `#name` source defers the insert to pass 2.
    for (i, comp) in node.components.iter().enumerate() {
        if i == layout_idx || Some(i) == rect_idx {
            continue;
        }
        report.set_current_line(comp.line_no);
        match comp.name.as_str() {
            "BindText" => lower_bind_text(comp, id, cmds, report, ctx),
            "BindValue" => lower_bind_value(comp, id, cmds, report, ctx),
            // A per-component failure (unknown component, kind mismatch) is recorded
            // and dropped; siblings survive (Decision 6).
            _ => {
                let _ = parse_and_insert(comp, id, cmds, report);
            }
        }
    }

    // 3. #name → UiName LAST (mirror lib.rs:3940-3945). Also record it into the
    //    bind index so a `#name`-source bind (anywhere in the document) resolves
    //    to this entity in pass 2.
    if let Some(name) = &node.name {
        let ui_name = UiName::new(&name.text);
        cmds.entity(id).insert(ui_name);
        ctx.record_name(ui_name, id);
    }

    // 4. UiSourceOrder stamp (Decision 11; gate-excluded, Decision 12).
    cmds.entity(id).insert(UiSourceOrder(node.sibling_ordinal));

    // 5. Children pre-order, then link (mirror lib.rs:3959-3961). The parent
    //    spawn was enqueued above before any child's ChildOf insert, so the FIFO
    //    drain materialises the parent first (dangling-parent guard passes).
    for &child_idx in &node.children {
        if let Some(child_id) = lower_node(tree, child_idx, cmds, report, ctx) {
            cmds.entity(id).add_child(child_id);
        }
    }

    Some(id)
}

/// Lowers a `BindText` component (GUI #27): a numeric `source` inserts now; a
/// `#name` source defers the whole insert to pass 2 via a [`BindFixup`].
fn lower_bind_text(
    comp: &ParsedComponent,
    widget: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
    ctx: &mut BindCtx,
) {
    if comp.kind != CompKind::Struct {
        report.error(comp.line_no, comp.body_col, "BindText must use the struct form `BindText { .. }`");
        return;
    }
    match parse_bind_text(&comp.body, comp.body_col, comp.line_no, report) {
        BindParse::Resolved(bind) => {
            cmds.entity(widget).insert(bind);
        }
        BindParse::Deferred { comp, name, line_no, body_col } => {
            ctx.fixups.push(BindFixup::Text { widget, comp, name, line_no, body_col });
        }
    }
}

/// Lowers a `BindValue` component (GUI #27): same two-pass policy as
/// [`lower_bind_text`].
fn lower_bind_value(
    comp: &ParsedComponent,
    widget: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
    ctx: &mut BindCtx,
) {
    if comp.kind != CompKind::Struct {
        report.error(comp.line_no, comp.body_col, "BindValue must use the struct form `BindValue { .. }`");
        return;
    }
    match parse_bind_value(&comp.body, comp.body_col, comp.line_no, report) {
        BindParse::Resolved(bind) => {
            cmds.entity(widget).insert(bind);
        }
        BindParse::Deferred { comp, name, line_no, body_col } => {
            ctx.fixups.push(BindFixup::Value { widget, comp, name, line_no, body_col });
        }
    }
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
