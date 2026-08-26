//! The diff-by-`UiName` (/ `UiSourceOrder` for anonymous) reconciler
//! (P3 §C / §D, Decision 9 / 10 / 11 / 13 / 14).
//!
//! Run top-down from the document's scoped roots (Decision 10). A surviving node
//! is matched by its `UiName` (named) or stored `UiSourceOrder` (anonymous),
//! patched set-if-changed (Decision 14), and recursed; a new key is spawned via
//! the full lowering (§B); a vanished key is despawned with the move-vs-despawn
//! guarantee (Decision 13, alternative (a) — the FORCED DRAIN BARRIER).
//!
//! # The two-phase apply (Decision 13, alternative (a))
//!
//! The reconcile is split into two command batches with a drain between them:
//!
//! * **Phase 1 ([`reconcile_ui`])** — the top-down match/patch/spawn pass.
//!   Survivors that moved are reparented (`set_parent` / `remove_parent`); the
//!   reparent's `ChildOf` overwrite itself unlinks the survivor from its OLD
//!   (possibly doomed) parent. It returns the [`DespawnPlan`] (the doomed
//!   parents) WITHOUT despawning them.
//! * **drain barrier** — the caller drains phase 1 (one full apply window: each
//!   `run_system` ends with `system.apply` + `drain_deferred_hook_queue`,
//!   `ecs_master.rs:2456/2465`). This MATERIALISES the two-stage `ChildOf`
//!   reparent unlink — `InsertCommand<ChildOf>::apply` (the `set_parent`
//!   overwrite) / `RemoveCommand<ChildOf>::apply` (the `remove_parent`) fires
//!   `child_of_on_replace`, which ENQUEUES an `UnlinkChildCommand` into
//!   `deferred_hook_queue`; only the apply-window drain runs that unlink
//!   (`hierarchy/commands.rs:268-276`, `ecs_master.rs:1370`). After the barrier
//!   the moved survivor is provably absent from its old (doomed) parent's live
//!   `Children`.
//! * **Phase 2 ([`apply_despawns`])** — despawns the doomed parents. Their
//!   `children_on_replace` cascade now reads a `Children` collection from which
//!   every moved survivor was already unlinked, so the cascade cannot reach a
//!   survivor.
//!
//! The original single-batch ordering (despawn on the SAME queue as the reparent)
//! was UNSOUND: the unlink is a two-stage deferral and its second stage drains
//! too late — `DespawnCommand::apply` → `delete_entity_core` fires the cascade
//! BEFORE `delete_entity` drains the pending `UnlinkChildCommand`
//! (`ecs_master.rs:1365` runs the cascade, `:1370` drains after), so the doomed
//! parent's `Children` still contained the survivor at cascade time.
//!
//! A previously-considered `remove_children(survivors)`-in-phase-1 step is NOT
//! used: it is BOTH redundant (the barrier already detaches the moved survivor
//! via its own reparent unlink) AND actively harmful — a survivor's `set_parent`
//! to its NEW parent is enqueued before the `remove_children`, so by apply time
//! its `ChildOf` already points at the NEW parent and a `RemoveCommand<ChildOf>`
//! would unlink it from the new parent, orphaning it. The barrier alone is the
//! guarantee.
//!
//! Transient components (`UiFocus`/`UiScroll`/`UiHover`, P4+) and the private
//! `UiSourceOrder` are NEVER written, so they are preserved by omission and ride
//! the byte-copy archetype migration when an add/remove migrates a survivor
//! (Decision 14).

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use crate::components::{
    ComputedClip, ContentSize, StackIndex, UiAbsolute, UiAlign, UiName, UiNineSlice, UiRoot,
    UiSourceOrder, UiSpacing, UiSpriteAnim, UiSpriteSheet,
};
use crate::reload::state::UiHotReload;
use crate::reload::tree_view::{LiveNode, UiTreeView};
use crate::text::ast::{CompKind, ParsedNode, ParsedTree};
use crate::text::dispatch::{parse_bind_text, parse_bind_value, parse_ui_layout_public, BindParse};
use crate::text::lower::{lower_node, BindCtx};
use crate::text::report::UiParseReport;

/// The doomed (vanished) parents to despawn in phase 2, after the drain barrier
/// has materialised every survivor's `ChildOf` unlink (Decision 13, alt (a)).
///
/// Carries the doomed entities in the order they were discovered (children before
/// their own doomed parents is NOT required — each despawn cascades its own live
/// subtree, and survivors were unlinked in phase 1; a doomed parent and a doomed
/// descendant despawned twice is the idempotent stale-handle no-op per EC11).
#[derive(Default)]
pub struct DespawnPlan {
    doomed: Vec<Entity>,
}

impl DespawnPlan {
    /// Whether the plan has any doomed parents to despawn (lets the caller skip
    /// the second `run_system` entirely on the common no-deletion reload).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.doomed.is_empty()
    }
}

/// Phase 1 of the reconcile: diff a freshly-parsed tree against the live tree,
/// diffing by `UiName` (named) / `UiSourceOrder` (anonymous). Operates over the
/// document's scoped roots (`hot.doc_roots`, Decision 10). Patches survivors
/// set-if-changed, spawns new keys, relinks moved keys (the reparent itself
/// detaches a survivor from its old, possibly doomed, parent). Transient
/// components are never touched (Decision 14).
///
/// Returns the [`DespawnPlan`] — the doomed parents — WITHOUT despawning them.
/// The caller MUST drain (one full apply window) before applying the plan via
/// [`apply_despawns`], so the survivor unlinks materialise before the despawn
/// cascade reads the doomed parents' `Children` (Decision 13, alternative (a)).
///
/// Per-component re-parse errors discovered while patching/lowering are surfaced
/// into the caller's `report` (Finding: the lowering report must be reachable).
///
/// `live` is an owned snapshot built BEFORE this call (the watch system releases
/// the world borrow, then issues these commands), so no world borrow is held
/// while `cmds` is populated.
pub fn reconcile_ui(
    parsed: &ParsedTree,
    hot: &mut UiHotReload,
    live: &UiTreeView,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) -> DespawnPlan {
    // The document's live roots (those whose snapshot parent is None and which
    // are in the scoped root set).
    let live_roots: Vec<Entity> = hot.doc_roots.to_vec();

    // BUG-P3-MOVE-1 fix: a document-global `UiName -> Entity` index over EVERY
    // live node in the scoped view (not just one parent's children). A named node
    // that moves to a DIFFERENT parent is matched here (the per-parent index alone
    // never finds it, so it would otherwise be respawned and its old subtree
    // cascade-despawned). Sorted for binary search; the defensive lowest-id rule
    // for a duplicate `UiName` mirrors the per-parent index (Decision 10).
    let global_named = build_global_named_index(live, report);

    // The set of live entities CLAIMED as a survivor during the recursion (matched
    // locally or globally-via-move). A doomed candidate is only genuinely doomed if
    // it was never claimed by ANY parent — a cross-parent move claims the survivor
    // under its NEW parent while its OLD parent's pass would otherwise mark it
    // doomed. Finalised after the full recursion.
    let mut claimed: Vec<Entity> = Vec::new();
    let mut doomed_candidates: Vec<Entity> = Vec::new();

    // GUI #27: a two-pass bind context seeded with the live survivors' names, so a
    // `#name` source on a NEW or PATCHED node resolves against both live survivors
    // and newly-spawned nodes (forward references included). Resolved after the
    // full recursion records every name.
    let mut bind_ctx = BindCtx::with_seed(&global_named);

    let mut new_roots = Vec::new();
    reconcile_children(
        parsed,
        &parsed.roots,
        &live_roots,
        None,
        live,
        &global_named,
        cmds,
        report,
        &mut new_roots,
        &mut claimed,
        &mut doomed_candidates,
        &mut bind_ctx,
    );

    // Pass 2: resolve every deferred `#name`-source bind (new-key spawns + survivor
    // re-patches) now that all names are recorded.
    bind_ctx.resolve(cmds, report);

    // Finalise the despawn plan: a candidate is doomed ONLY if it was not claimed
    // (as a moved survivor) by some other parent's pass. This is what makes a
    // cross-parent move RELOCATE the survivor instead of respawning it
    // (BUG-P3-MOVE-1). A claimed entity nested under a doomed parent is detached by
    // its own `relink_if_moved` reparent, which the drain barrier materialises
    // before phase 2 despawns the doomed parent (Decision 13).
    claimed.sort_by_key(|e| e.id().0);
    let mut plan = DespawnPlan::default();
    for cand in doomed_candidates {
        if claimed.binary_search_by_key(&cand.id().0, |e| e.id().0).is_err() {
            plan.doomed.push(cand);
        }
    }

    // The document's root scope tracks the post-reconcile root set so a
    // subsequent reload reconciles against the right anchors.
    let mut roots = crate::reload::state::SmallRoots::new();
    for r in new_roots {
        roots.push(r);
    }
    hot.set_doc_roots(roots);
    plan
}

/// Builds the document-global `UiName -> Entity` index over every live node in
/// the scoped view (BUG-P3-MOVE-1 fix). Sorted by `UiName` for binary search.
/// A duplicate `UiName` across the document resolves to the lowest `Entity` id
/// (defensive, mirrors the per-parent rule, Decision 10) and surfaces the
/// corruption into `report`; the loser is left untouched (never despawned here).
fn build_global_named_index(live: &UiTreeView, report: &mut UiParseReport) -> Vec<(UiName, Entity)> {
    let mut named: Vec<(UiName, Entity)> = Vec::new();
    for node in &live.nodes {
        let Some(name) = node.name else { continue };
        if let Some(pos) = named.iter().position(|(k, _)| *k == name) {
            if node.entity.id().0 < named[pos].1.id().0 {
                named[pos].1 = node.entity;
            }
            report.error(0, 0, "duplicate live UiName in the document scope");
        } else {
            named.push((name, node.entity));
        }
    }
    named.sort_by_key(|(k, _)| *k);
    named
}

/// Phase 2 of the reconcile: despawn the doomed parents collected in phase 1
/// (Decision 13, alternative (a)). MUST run AFTER a drain barrier so every
/// survivor's `ChildOf` unlink (a two-stage deferral) has materialised and the
/// despawn cascade cannot reach a moved-away survivor.
pub fn apply_despawns(plan: &DespawnPlan, cmds: &mut Commands) {
    for &doomed in &plan.doomed {
        cmds.entity(doomed).despawn();
    }
}

/// Reconciles one parent's child set. `new_children` are AST node indices;
/// `live_children` are the scoped live child entities; `live_parent` is the live
/// parent entity (`None` at the document root). `global_named` is the
/// document-global `UiName` index (BUG-P3-MOVE-1: a named node that moved here
/// from a DIFFERENT parent is matched through it). Appends the resulting child
/// entity ids (matched or spawned) to `out_ids`. Every claimed survivor is
/// recorded into `claimed`; every unmatched live child with a `.ui` key is
/// recorded into `doomed_candidates`. The caller finalises the despawn plan by
/// removing claimed entities from the candidates (a cross-parent move claims the
/// survivor under its NEW parent, so its OLD parent must not despawn it). A
/// moved survivor's reparent (`relink_if_moved`, emitted in this pass) detaches
/// it from its old (doomed) parent, and the caller's drain barrier materialises
/// that unlink before phase 2 despawns the doomed parents (Decision 13, alt (a)).
#[allow(clippy::too_many_arguments)]
fn reconcile_children(
    parsed: &ParsedTree,
    new_children: &[usize],
    live_children: &[Entity],
    live_parent: Option<Entity>,
    live: &UiTreeView,
    global_named: &[(UiName, Entity)],
    cmds: &mut Commands,
    report: &mut UiParseReport,
    out_ids: &mut Vec<Entity>,
    claimed: &mut Vec<Entity>,
    doomed_candidates: &mut Vec<Entity>,
    bind_ctx: &mut BindCtx,
) {
    // Build the per-parent diff indices over the scoped live children
    // (Decision 9 / 11). Named: a sorted Vec<(UiName, Entity)> + binary search.
    // Anonymous: keyed by UiSourceOrder. The defensive duplicate handling
    // (Decision 10) keeps the lowest-id winner and preserves the loser.
    let mut named: Vec<(UiName, Entity)> = Vec::new();
    let mut anon: Vec<(u32, Entity)> = Vec::new();
    for &child in live_children {
        let Some(node) = live.get(child) else { continue };
        if let Some(name) = node.name {
            // Defensive: a scoped duplicate UiName under one parent — lowest id
            // wins, the loser is preserved (not despawned) and the corruption is
            // surfaced (Decision 10).
            if let Some(pos) = named.iter().position(|(k, _)| *k == name) {
                let existing = named[pos].1;
                if child.id().0 < existing.id().0 {
                    named[pos].1 = child;
                }
                report.error(node_line(live, child), 0, "duplicate live UiName under one parent");
            } else {
                named.push((name, child));
            }
        } else if let Some(so) = node.source_order {
            anon.push((so.0, child));
        }
        // A scoped child with neither key is left unmatched-but-present (it is
        // not a `.ui`-managed node); it is never despawned by this pass.
    }
    named.sort_by_key(|(k, _)| *k);

    // Track which live children were matched LOCALLY (under THIS parent) so the
    // despawn-candidate phase can find the unmatched ones. A node matched here is
    // also recorded into the document-global `claimed` set.
    let mut matched: Vec<Entity> = Vec::new();

    for &new_idx in new_children {
        let new = &parsed.nodes[new_idx];
        let live_match = match &new.name {
            Some(name_str) => {
                // Binary search the per-parent named index first (Decision 9 — the
                // common sibling case). A debug cross-check guards the Ord.
                let key = UiName::new(&name_str.text);
                let local = named
                    .binary_search_by(|(k, _)| k.cmp(&key))
                    .ok()
                    .map(|i| named[i].1);
                debug_assert_eq!(
                    local,
                    named.iter().find(|(k, _)| *k == key).map(|(_, e)| *e),
                    "invariant: binary-search hit equals linear-scan hit (UiName Ord)"
                );
                // BUG-P3-MOVE-1: fall back to the document-global index for a node
                // that moved here from a DIFFERENT parent (relocate, not respawn).
                local.or_else(|| {
                    global_named
                        .binary_search_by(|(k, _)| k.cmp(&key))
                        .ok()
                        .map(|i| global_named[i].1)
                })
            }
            None => anon
                .iter()
                .find(|(o, _)| *o == new.sibling_ordinal)
                .map(|(_, e)| *e),
        };

        match live_match {
            Some(entity) => {
                // Claim the survivor document-globally (so its OLD parent's pass,
                // which still lists it as a live child, does not despawn it).
                if !claimed.contains(&entity) {
                    claimed.push(entity);
                }
                matched.push(entity);
                // Patch text-owned components set-if-changed (Decision 14), incl.
                // re-resolving any `#name`-source data-bind (GUI #27).
                patch_node(new, entity, live, cmds, report, bind_ctx);
                // Re-key the anonymous ordinal if it shifted (Decision 11).
                set_source_order_if_changed(new.sibling_ordinal, entity, live, cmds);
                // Relink if the node moved to a different parent (the reparent's
                // `ChildOf` overwrite detaches it from its old — possibly doomed —
                // parent; the drain barrier materialises the unlink before phase 2,
                // Decision 13).
                relink_if_moved(entity, live_parent, live, cmds);
                out_ids.push(entity);

                // Recurse into the survivor's children.
                let live_children_of = live
                    .get(entity)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                let mut child_ids = Vec::new();
                reconcile_children(
                    parsed,
                    &new.children,
                    &live_children_of,
                    Some(entity),
                    live,
                    global_named,
                    cmds,
                    report,
                    &mut child_ids,
                    claimed,
                    doomed_candidates,
                    bind_ctx,
                );
            }
            None => {
                // A new key: spawn its full subtree via the §B lowering (it
                // stamps UiSourceOrder + UiName + the chained inserts, and records
                // its `#name`s + defers any `#name`-source bind into `bind_ctx`,
                // GUI #27). Link it under the live parent if there is one.
                if let Some(new_id) = lower_node(parsed, new_idx, cmds, report, bind_ctx) {
                    if let Some(parent) = live_parent {
                        cmds.entity(parent).add_child(new_id);
                    }
                    out_ids.push(new_id);
                }
            }
        }
    }

    // Despawn-CANDIDATE phase: every scoped live child not matched UNDER THIS
    // parent is a candidate for despawn. The caller finalises by dropping any
    // candidate that was CLAIMED (as a moved survivor) by another parent's pass
    // (BUG-P3-MOVE-1) — so a cross-parent move relocates rather than despawns.
    // The actual `despawn` is deferred to phase 2 (`apply_despawns`), AFTER the
    // caller's drain barrier (Decision 13, alternative (a)).
    for &child in live_children {
        if matched.contains(&child) {
            continue;
        }
        // A scoped child without a `.ui` key is not `.ui`-managed: leave it.
        let Some(node) = live.get(child) else { continue };
        if node.name.is_none() && node.source_order.is_none() {
            continue;
        }
        doomed_candidates.push(child);
    }
}

/// Relinks `entity` to `new_parent` if its live parent differs (a move).
fn relink_if_moved(
    entity: Entity,
    new_parent: Option<Entity>,
    live: &UiTreeView,
    cmds: &mut Commands,
) {
    let current = live.get(entity).and_then(|n| n.parent);
    match (current, new_parent) {
        (a, b) if a == b => {} // unchanged
        (_, Some(parent)) => {
            // Reparent: ChildOf overwrite (old-link unlink before new-link insert
            // at the FIFO drain). Never despawn-and-respawn (preserves transient
            // state).
            cmds.entity(entity).set_parent(parent);
        }
        (Some(_), None) => {
            // Moved to the document root: drop the ChildOf link.
            cmds.entity(entity).remove_parent();
        }
        (None, None) => {}
    }
}

/// Patches `UiSourceOrder` if the survivor's declaration ordinal shifted
/// (Decision 11). Set-if-changed: an unchanged ordinal is never written.
fn set_source_order_if_changed(
    ordinal: u32,
    entity: Entity,
    live: &UiTreeView,
    cmds: &mut Commands,
) {
    let live_so = live.get(entity).and_then(|n| n.source_order);
    if live_so != Some(UiSourceOrder(ordinal)) {
        cmds.entity(entity).insert(UiSourceOrder(ordinal));
    }
}

/// The line of a live node (for error attribution). Live nodes have no source
/// line, so this returns 0.
#[inline]
fn node_line(_live: &UiTreeView, _entity: Entity) -> usize {
    0
}

/// Patches the text-owned component set of a survivor set-if-changed
/// (Decision 14 / §D). Writes ONLY the closed text-owned set; transient
/// components + `UiSourceOrder` are preserved by omission. `ComputedRect` is
/// layout output — EXCLUDED (a spawn-time seed only; layout overwrites it).
fn patch_node(
    new: &ParsedNode,
    entity: Entity,
    live: &UiTreeView,
    cmds: &mut Commands,
    report: &mut UiParseReport,
    bind_ctx: &mut BindCtx,
) {
    let Some(node) = live.get(entity) else { return };
    report.set_current_line(new.line_no);

    patch_ui_layout(new, node, entity, cmds, report);
    patch_unit_struct::<UiSpacing>(new, "UiSpacing", node.spacing, entity, cmds, report);
    patch_unit_struct::<UiAlign>(new, "UiAlign", node.align, entity, cmds, report);
    patch_unit_struct::<UiAbsolute>(new, "UiAbsolute", node.absolute, entity, cmds, report);
    patch_unit_struct::<ContentSize>(new, "ContentSize", node.content_size, entity, cmds, report);
    patch_unit_struct::<ComputedClip>(new, "ComputedClip", node.clip, entity, cmds, report);
    // UI-ADVANCED S6 — the sprite vocabulary. `UiSpriteCursor` is absent on
    // purpose: it is not text-owned, so it is preserved by omission exactly like
    // the transient components, and a reload that edits an animation's `fps`
    // therefore does not reset the running phase (the `on_add` hook does not
    // re-fire on a re-insert — MEASURED).
    patch_unit_struct::<UiNineSlice>(new, "UiNineSlice", node.nine_slice, entity, cmds, report);
    patch_unit_struct::<UiSpriteSheet>(
        new,
        "UiSpriteSheet",
        node.sprite_sheet,
        entity,
        cmds,
        report,
    );
    patch_unit_struct::<UiSpriteAnim>(new, "UiSpriteAnim", node.sprite_anim, entity, cmds, report);
    patch_stack_index(new, node.stack_index, entity, cmds, report);
    patch_ui_root(new, node, entity, cmds);
    patch_ui_name(new, node, entity, cmds);
    // GUI #27: re-patch the data-bind components on a survivor. The bind columns
    // are NOT in the live snapshot (transient/render-facing), so this is an
    // unconditional re-insert from the authored text (last-write-wins) rather than
    // a set-if-changed — the cost is a cold re-parse + one insert on a reloaded
    // survivor, and it closes the stale-`#name`-source gap (a survivor whose named
    // target was respawned re-resolves to the new entity). A numeric source
    // re-inserts in place; a `#name` source defers to pass 2.
    patch_bind_text(new, entity, cmds, report, bind_ctx);
    patch_bind_value(new, entity, cmds, report, bind_ctx);
}

/// Re-patches a survivor's `BindText` from the authored text (GUI #27). Present →
/// re-insert (numeric) or defer (`#name`).
///
/// A `BindText` DELETED from the file is NOT removed: the bind columns are not in
/// the live snapshot (render-facing/transient), so presence cannot be read here,
/// and the reconcile's policy for non-snapshotted components is preserve-by-
/// omission. Removing a deleted-from-file bind on reload is a documented #27
/// limitation (the `ui!` and full-reload paths are the route for that).
fn patch_bind_text(
    new: &ParsedNode,
    entity: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
    bind_ctx: &mut BindCtx,
) {
    let Some(comp) = new.components.iter().find(|c| c.name == "BindText") else {
        return;
    };
    if comp.kind != CompKind::Struct {
        report.error(comp.line_no, comp.body_col, "BindText must use the struct form `BindText { .. }`");
        return;
    }
    report.set_current_line(comp.line_no);
    match parse_bind_text(&comp.body, comp.body_col, comp.line_no, report) {
        BindParse::Resolved(bind) => {
            cmds.entity(entity).insert(bind);
        }
        BindParse::Deferred { comp: bind, name, line_no, body_col } => {
            bind_ctx.defer_text(entity, bind, name, line_no, body_col);
        }
    }
}

/// Re-patches a survivor's `BindValue` from the authored text (GUI #27). Same
/// policy as [`patch_bind_text`] (present → re-insert/defer; deleted → preserved
/// by omission, the documented #27 limitation).
fn patch_bind_value(
    new: &ParsedNode,
    entity: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
    bind_ctx: &mut BindCtx,
) {
    let Some(comp) = new.components.iter().find(|c| c.name == "BindValue") else {
        return;
    };
    if comp.kind != CompKind::Struct {
        report.error(comp.line_no, comp.body_col, "BindValue must use the struct form `BindValue { .. }`");
        return;
    }
    report.set_current_line(comp.line_no);
    match parse_bind_value(&comp.body, comp.body_col, comp.line_no, report) {
        BindParse::Resolved(bind) => {
            cmds.entity(entity).insert(bind);
        }
        BindParse::Deferred { comp: bind, name, line_no, body_col } => {
            bind_ctx.defer_value(entity, bind, name, line_no, body_col);
        }
    }
}

/// `UiLayout` is always present on a survivor (a node requires it). Re-parse the
/// authored value and set-if-changed by `Debug` equality (matches the gate's
/// comparison; the layout components are POD but most do not derive `PartialEq`,
/// so `Debug` is the total deterministic projection).
fn patch_ui_layout(
    new: &ParsedNode,
    node: &LiveNode,
    entity: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) {
    let Some(comp) = new.components.iter().find(|c| c.name == "UiLayout") else {
        // A reload that drops UiLayout entirely is degenerate; leave the live one.
        return;
    };
    report.set_current_line(comp.line_no);
    let parsed = parse_ui_layout_public(&comp.body, comp.body_col, report);
    match node.layout {
        Some(live_val) if debug_eq(&live_val, &parsed) => {} // unchanged
        _ => {
            cmds.entity(entity).insert(parsed);
        }
    }
}

/// Generic patch for a struct-form text-owned component reconstructed from the
/// AST and compared to the live value by `Debug`. `live_val` is the snapshot's
/// `Option<C>`. Absent-live + in-text → insert; present-both → set-if-changed;
/// present-live + absent-text → remove (Decision 14, read-guarded).
fn patch_unit_struct<C>(
    new: &ParsedNode,
    name: &str,
    live_val: Option<C>,
    entity: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) where
    C: TextStruct,
{
    let in_text = new.components.iter().find(|c| c.name == name);
    match (in_text, live_val) {
        (Some(comp), live) => {
            report.set_current_line(comp.line_no);
            let parsed = C::parse(&comp.body, comp.body_col, report);
            match live {
                Some(lv) if debug_eq(&lv, &parsed) => {} // unchanged
                _ => {
                    parsed.insert(entity, cmds);
                }
            }
        }
        (None, Some(_)) => {
            // Deleted from the file → remove.
            C::remove(entity, cmds);
        }
        (None, None) => {}
    }
}

/// `StackIndex` patch (tuple newtype). Derives `PartialEq`, so an exact compare
/// is used.
fn patch_stack_index(
    new: &ParsedNode,
    live_val: Option<StackIndex>,
    entity: Entity,
    cmds: &mut Commands,
    report: &mut UiParseReport,
) {
    let in_text = new.components.iter().find(|c| c.name == "StackIndex");
    match (in_text, live_val) {
        (Some(comp), live) => {
            report.set_current_line(comp.line_no);
            let parsed = match comp.body.trim().parse::<u32>() {
                Ok(n) => StackIndex(n),
                Err(_) => {
                    // A lower-time re-parse failure: record it on the reachable
                    // report (never silently swallowed) and fall back to default.
                    report.error(comp.line_no, comp.body_col, "StackIndex: expected a u32");
                    StackIndex::default()
                }
            };
            if live != Some(parsed) {
                cmds.entity(entity).insert(parsed);
            }
        }
        (None, Some(_)) => {
            cmds.entity(entity).remove::<StackIndex>();
        }
        (None, None) => {}
    }
}

/// `UiRoot` is a ZST marker: presence-only patch.
fn patch_ui_root(new: &ParsedNode, node: &LiveNode, entity: Entity, cmds: &mut Commands) {
    let in_text = new.components.iter().any(|c| c.name == "UiRoot");
    match (in_text, node.is_root) {
        (true, false) => {
            cmds.entity(entity).insert(UiRoot);
        }
        (false, true) => {
            cmds.entity(entity).remove::<UiRoot>();
        }
        _ => {}
    }
}

/// `UiName` is the diff key — a survivor matched by name already agrees on it,
/// so this is a no-op for named survivors. An anonymous-matched node that gained
/// a name in the new text is treated by the reconcile as a new key (it would not
/// match by ordinal-then-name); this guard only inserts a name if the live node
/// lacked one and the new node has one (rare degenerate path).
fn patch_ui_name(new: &ParsedNode, node: &LiveNode, entity: Entity, cmds: &mut Commands) {
    if let Some(name_str) = &new.name {
        let want = UiName::new(&name_str.text);
        if node.name != Some(want) {
            cmds.entity(entity).insert(want);
        }
    }
}

/// `Debug`-string equality — the gate's comparison method for POD layout
/// components that mostly do not derive `PartialEq`.
#[inline]
fn debug_eq<T: core::fmt::Debug>(a: &T, b: &T) -> bool {
    // A small per-call allocation on the cold reload path; acceptable.
    format!("{a:?}") == format!("{b:?}")
}

/// A text-owned struct component the patcher can reconstruct from the AST,
/// insert, and remove. Implemented for the closed text-owned set so
/// [`patch_unit_struct`] is generic without reflection.
trait TextStruct: core::fmt::Debug + Sized {
    /// Reconstructs the component from its AST body.
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self;
    /// Inserts `self` onto `entity`.
    fn insert(self, entity: Entity, cmds: &mut Commands);
    /// Removes this component type from `entity`.
    fn remove(entity: Entity, cmds: &mut Commands);
}

impl TextStruct for UiSpacing {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_spacing_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiSpacing>();
    }
}

impl TextStruct for UiAlign {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_align_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiAlign>();
    }
}

impl TextStruct for UiAbsolute {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_absolute_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiAbsolute>();
    }
}

impl TextStruct for ContentSize {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_content_size_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<ContentSize>();
    }
}

impl TextStruct for ComputedClip {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_computed_clip_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<ComputedClip>();
    }
}

impl TextStruct for UiNineSlice {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_nine_slice_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiNineSlice>();
    }
}

impl TextStruct for UiSpriteSheet {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_sprite_sheet_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiSpriteSheet>();
    }
}

impl TextStruct for UiSpriteAnim {
    fn parse(body: &str, body_col: u16, report: &mut UiParseReport) -> Self {
        crate::text::dispatch::parse_ui_sprite_anim_public(body, body_col, report)
    }
    fn insert(self, entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).insert(self);
    }
    /// Removing the ANIMATION leaves its dense [`UiSpriteCursor`] row behind —
    /// 8 B, inert without the animation (the flipbook needs all three
    /// components), and self-healing (a re-added animation gets a fresh `Default`
    /// cursor from the `on_add` hook). The symmetric `on_remove` hook that would
    /// tidy it is unlandable on this kernel: it also fires on a DESPAWN's
    /// per-component pass, where the deferred removal then panics
    /// `RemoveCommand::apply: stale entity` — MEASURED, and a liveness guard does
    /// not help because the entity is still live at hook time. See
    /// `crate::sprite::ui_sprite_anim_on_add` and `docs/OPEN-QUESTIONS.md`.
    fn remove(entity: Entity, cmds: &mut Commands) {
        cmds.entity(entity).remove::<UiSpriteAnim>();
    }
}

