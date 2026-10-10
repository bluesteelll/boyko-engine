//! GATE 5 — HOT-RELOAD: diff-by-`#name` patches in place, transient state on a
//! survivor SURVIVES a sibling-only change, add/remove/relink are correct,
//! move-vs-despawn does NOT cascade-despawn a moved survivor, rename = remove+add,
//! and the reconcile is scoped to the document (a live-duplicate is handled).
//!
//! Driven through the PUBLIC `UiPlugin` + `App` path (`p3_common::ReloadWorld`):
//! `finish()` runs the startup spawn, `reload()` rewrites the file and ticks the
//! watch system across the two-poll settle so one reconcile applies.
//!
//! Transient state is modelled by a test-local `#[derive(Component)]` probe
//! (`Transient`) the `.ui` text never mentions — the P4 `UiFocus`/`UiScroll`
//! stand-in. The reconcile writes ONLY the closed text-owned set (Decision 14),
//! so the probe must ride archetype migrations untouched.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use p3_common::{children_of, parent_of, ReloadWorld};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;

use boyko_ui::components::UiLayout;

/// A transient runtime component the `.ui` text never mentions (the P4
/// focus/scroll stand-in). It must survive reloads of OTHER nodes and archetype
/// migrations of its OWN node.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Transient {
    marker: u32,
}

/// Inserts a `Transient` on `e` through one apply window.
fn set_transient(rw: &mut ReloadWorld, e: Entity, marker: u32) {
    rw.app.world_mut().run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(Transient { marker });
    });
}

/// Reads a node's `Transient`, if present.
fn transient_of(rw: &ReloadWorld, e: Entity) -> Option<Transient> {
    rw.world().get_component::<Transient>(e).copied()
}

/// Reads a node's `UiLayout` `Debug` projection, if present.
fn layout_debug(rw: &ReloadWorld, e: Entity) -> Option<String> {
    rw.world().get_component::<UiLayout>(e).map(|l| format!("{l:?}"))
}

// ─────────────── 1. diff-by-#name patches a survivor IN PLACE ──────────────

#[test]
fn reload_patches_named_survivor_in_place_same_entity() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #child  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("patch", initial);
    let child_before = rw.find_named("child").expect("child exists");
    let layout_before = layout_debug(&rw, child_before).expect("child has layout");

    // Reload: change the child's width. The SAME entity must be patched.
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #child  UiLayout { layout_type: Column, width: Px(80), height: Px(40) }
";
    rw.reload(reloaded);

    let child_after = rw.find_named("child").expect("child still exists");
    assert_eq!(child_before, child_after, "the named survivor keeps its ENTITY id (patched in place)");
    let layout_after = layout_debug(&rw, child_after).expect("child has layout");
    assert_ne!(layout_before, layout_after, "the child's UiLayout was patched to the new value");
    assert!(layout_after.contains("80.0"), "width updated to Px(80): {layout_after}");
}

// ──────── 2. transient state on a survivor SURVIVES a sibling change ────────

#[test]
fn reload_survivor_transient_state_survives_sibling_change() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #b  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("transient_sibling", initial);
    let a = rw.find_named("a").expect("a exists");
    assert!(rw.find_named("b").is_some(), "b exists initially");
    // Put transient state on `a`.
    set_transient(&mut rw, a, 0xABCD);
    assert_eq!(transient_of(&rw, a), Some(Transient { marker: 0xABCD }), "transient set on a");

    // Reload: change ONLY sibling `b`. `a` must keep its entity AND its transient.
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #b  UiLayout { layout_type: Column, width: Px(99), height: Px(40) }
";
    rw.reload(reloaded);

    let a_after = rw.find_named("a").expect("a still exists");
    assert_eq!(a, a_after, "survivor `a` keeps its entity across a sibling reload");
    assert_eq!(
        transient_of(&rw, a_after),
        Some(Transient { marker: 0xABCD }),
        "survivor `a`'s transient state SURVIVES a sibling-only reload"
    );
    // And the unchanged survivor `a` is NOT re-spawned (b is still present too).
    assert!(rw.find_named("b").is_some(), "b still present after its change");
}

// ──── 3. transient state rides an archetype migration of its OWN node ───────

#[test]
fn reload_transient_survives_own_node_archetype_migration() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("transient_migrate", initial);
    let a = rw.find_named("a").expect("a exists");
    set_transient(&mut rw, a, 7);

    // Reload ADDS a component to `a` (UiSpacing) → archetype migration. The
    // transient column must ride the byte-copy migrate (Decision 14).
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
        UiSpacing { padding_left: Px(8) }
";
    rw.reload(reloaded);

    let a_after = rw.find_named("a").expect("a still exists");
    assert_eq!(a, a_after, "a keeps its entity across an add-component migration");
    assert_eq!(
        transient_of(&rw, a_after),
        Some(Transient { marker: 7 }),
        "transient state rides the archetype migration intact"
    );
    use boyko_ui::components::UiSpacing;
    assert!(
        rw.world().has_component(a_after, UiSpacing::component_id()),
        "the reload ADDED UiSpacing to a"
    );
}

// ───────────────────── 4. add a new node on reload ─────────────────────────

#[test]
fn reload_adds_new_named_node() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("add", initial);
    let root = rw.find_named("root").expect("root");
    let a = rw.find_named("a").expect("a");
    assert!(rw.find_named("c").is_none(), "c does not exist yet");

    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #c  UiLayout { layout_type: Column, width: Px(60), height: Px(60) }
";
    rw.reload(reloaded);

    assert_eq!(rw.find_named("a"), Some(a), "a survives unchanged");
    let c = rw.find_named("c").expect("c was added");
    assert_eq!(parent_of(rw.world(), c), Some(root), "the new node c is linked under root");
    let kids = children_of(rw.world(), root);
    assert!(kids.contains(&a) && kids.contains(&c), "root now has both a and c");
}

// ───────────────────── 5. remove a node on reload ──────────────────────────

#[test]
fn reload_removes_vanished_node() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #b  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("remove", initial);
    let a = rw.find_named("a").expect("a");
    let b = rw.find_named("b").expect("b");

    // Reload drops `b`.
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    rw.reload(reloaded);

    assert_eq!(rw.find_named("a"), Some(a), "a survives");
    assert!(rw.find_named("b").is_none(), "b was removed by the reload");
    assert!(!rw.world().has_entity(b), "b's entity is despawned");
    assert!(rw.world().has_entity(a), "a's entity is still live");
}

// ──── 6. move a node to a new parent whose OLD parent is DELETED (Decision 13)
//
// BUG-P3-MOVE-1 (FIXED): the reconcile now matches a named survivor against a
// document-GLOBAL `UiName` index, so a node that moves to a DIFFERENT parent is
// relocated (its `relink_if_moved` reparent fires + the drain barrier protects it
// from the doomed parent's cascade), keeping its entity id and transient state.
//
// This test asserts the SPEC (Decision 11/13: relocate, keep entity + transient).
#[test]
fn reload_move_survivor_not_cascade_despawned_when_old_parent_deleted() {
    // #mover starts under #old; the reload deletes #old AND reparents #mover under
    // #new. The despawn cascade of #old must NOT reach the moved survivor
    // (Decision 13, the drain-barrier guarantee). #mover keeps its entity AND its
    // transient state.
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #old  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #mover  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
    #new  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
";
    let mut rw = ReloadWorld::new("move", initial);
    let old = rw.find_named("old").expect("old");
    let new = rw.find_named("new").expect("new");
    let mover = rw.find_named("mover").expect("mover");
    set_transient(&mut rw, mover, 0xF00D);
    assert_eq!(parent_of(rw.world(), mover), Some(old), "mover starts under old");

    // Reload: delete #old, put #mover under #new.
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #new  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #mover  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
";
    rw.reload(reloaded);

    // The mover SURVIVED (not cascade-despawned) and moved under #new.
    let mover_after = rw.find_named("mover").expect("mover survived the move");
    assert_eq!(mover, mover_after, "mover keeps its entity id (moved, not respawned)");
    assert!(rw.world().has_entity(mover), "mover is live");
    assert_eq!(parent_of(rw.world(), mover), Some(new), "mover is now under new");
    assert_eq!(
        transient_of(&rw, mover),
        Some(Transient { marker: 0xF00D }),
        "mover's transient state survived the move-vs-despawn"
    );
    // #old is gone.
    assert!(rw.find_named("old").is_none(), "old was deleted");
    assert!(!rw.world().has_entity(old), "old's entity is despawned");
}

// ── 6a. relocation of the move-with-deleted-old-parent case (post-fix) ──────
//
// BUG-P3-MOVE-1 (FIXED): formerly this locked the DEFECTIVE respawn behavior;
// with the document-global `UiName` match it now witnesses the SPEC-compliant
// relocation (same as test 6, from the other direction): the moved node KEEPS its
// entity id and transient state, and its deleted old parent is gone. This pins
// that the cross-parent move is a relocate, not a remove+add.
#[test]
fn reload_move_with_deleted_old_parent_relocates() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #old  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #mover  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
    #new  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
";
    let mut rw = ReloadWorld::new("move_actual", initial);
    let old = rw.find_named("old").expect("old");
    let new = rw.find_named("new").expect("new");
    let mover = rw.find_named("mover").expect("mover");
    set_transient(&mut rw, mover, 0xF00D);

    rw.reload("\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #new  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #mover  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
");

    // Structure is correct: #mover ends up under #new, and #old is gone.
    let mover_after = rw.find_named("mover").expect("a #mover exists under new");
    assert_eq!(parent_of(rw.world(), mover_after), Some(new), "the relocated mover is under new");
    assert!(rw.find_named("old").is_none(), "old deleted");
    assert!(!rw.world().has_entity(old), "old despawned");
    // RELOCATION: the SAME entity, transient preserved (Decision 11/13).
    assert_eq!(mover, mover_after, "mover keeps its entity id (relocated, not respawned)");
    assert!(rw.world().has_entity(mover), "the original mover entity is live (relocated)");
    assert_eq!(
        transient_of(&rw, mover_after),
        Some(Transient { marker: 0xF00D }),
        "mover's transient state survived the cross-parent move"
    );
}

// ── 6b. plain reparent (move a named node to another EXISTING parent) ──────
//
// BUG-P3-MOVE-1 (FIXED): a named node moved to a DIFFERENT existing parent (old
// parent NOT deleted) is RELOCATED via the document-global `UiName` match, keeping
// its entity id and transient state (Decision 11/13).
#[test]
fn reload_reparent_named_node_to_existing_parent() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #p1  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #kid  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
    #p2  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
";
    let mut rw = ReloadWorld::new("reparent", initial);
    let p1 = rw.find_named("p1").expect("p1");
    let p2 = rw.find_named("p2").expect("p2");
    let kid = rw.find_named("kid").expect("kid");
    set_transient(&mut rw, kid, 0xBEEF);
    assert_eq!(parent_of(rw.world(), kid), Some(p1), "kid starts under p1");

    // Reload: move #kid from #p1 to #p2 (both parents survive).
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(200), height: Px(200) }
    #p1  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #p2  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
        #kid  UiLayout { layout_type: Column, width: Px(50), height: Px(50) }
";
    rw.reload(reloaded);

    let kid_after = rw.find_named("kid").expect("kid exists after reparent");
    assert_eq!(parent_of(rw.world(), kid_after), Some(p2), "kid is now under p2");
    // SPEC (Decision 11/13): a named node RELOCATES, keeping its entity + transient.
    assert_eq!(kid, kid_after, "reparented named node keeps its ENTITY id (relocate, not respawn)");
    assert_eq!(
        transient_of(&rw, kid_after),
        Some(Transient { marker: 0xBEEF }),
        "reparented node keeps its transient state"
    );
    let _ = (p1, p2);
}

// ──────────────── 7. rename = remove + add (NOT a patch) ────────────────────

#[test]
fn reload_rename_is_remove_plus_add() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #oldname  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("rename", initial);
    let old = rw.find_named("oldname").expect("oldname");

    // Reload renames #oldname -> #newname. Names are the diff key, so this is a
    // remove(oldname) + add(newname): a DIFFERENT entity.
    let reloaded = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #newname  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    rw.reload(reloaded);

    assert!(rw.find_named("oldname").is_none(), "oldname is gone");
    let new = rw.find_named("newname").expect("newname exists");
    assert_ne!(old, new, "rename produced a NEW entity (remove + add, not a patch)");
    assert!(!rw.world().has_entity(old), "the old-named entity was despawned");
    let root = rw.find_named("root").expect("root");
    assert_eq!(parent_of(rw.world(), new), Some(root), "the renamed node is linked under root");
}

// ─────────── 8. no-op reload (identical content) preserves everything ───────

#[test]
fn reload_identical_content_is_noop_preserves_entities() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #a  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("noop", initial);
    let root = rw.find_named("root").expect("root");
    let a = rw.find_named("a").expect("a");
    set_transient(&mut rw, a, 1);

    // Rewrite the file with byte-identical content. The size+mtime may change, so
    // the reconcile runs but finds nothing to change — entities + transient stay.
    rw.reload(initial);

    assert_eq!(rw.find_named("root"), Some(root), "root unchanged");
    assert_eq!(rw.find_named("a"), Some(a), "a unchanged");
    assert_eq!(transient_of(&rw, a), Some(Transient { marker: 1 }), "transient preserved");
}

// ──────── 8b. reconcile is SCOPED to the document (Decision 10) ────────────
//
// A FOREIGN entity outside the document's subtree carrying the SAME `UiName` as a
// document node must NOT be touched by the reconcile — the scope is the
// document's own roots/subtree, never a global `UiName` index.
#[test]
fn reload_is_scoped_to_document_foreign_uiname_untouched() {
    use boyko_ui::components::{ComputedRect, UiName};

    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #shared  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("scope", initial);
    let doc_shared = rw.find_named("shared").expect("document #shared");

    // Spawn a FOREIGN entity (NOT under the document root) with the SAME name.
    let foreign_cell = std::sync::Arc::new(std::sync::Mutex::new(None));
    let fp = std::sync::Arc::clone(&foreign_cell);
    rw.app.world_mut().run_system(move |mut cmds: Commands| {
        let id = cmds
            .spawn(UiLayout::default())
            .insert(ComputedRect::default())
            .insert(UiName::new("shared"))
            .id();
        *fp.lock().unwrap() = Some(id);
    });
    let foreign = foreign_cell.lock().unwrap().take().expect("foreign entity");
    set_transient(&mut rw, foreign, 0x5C0E);
    assert!(parent_of(rw.world(), foreign).is_none(), "foreign is a separate root, not under the doc");

    // Reload the document (remove #shared from the doc). The FOREIGN entity must
    // be untouched even though it shares the name.
    rw.reload("\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
");

    assert!(!rw.world().has_entity(doc_shared), "the DOCUMENT's #shared was removed");
    assert!(rw.world().has_entity(foreign), "the FOREIGN #shared is untouched (scoped reconcile)");
    assert_eq!(
        transient_of(&rw, foreign),
        Some(Transient { marker: 0x5C0E }),
        "the foreign entity's state is preserved (it is out of document scope)"
    );
}

// ───────────── 9. multiple reloads compose (state stays stable) ─────────────

#[test]
fn reload_multiple_reloads_compose() {
    let initial = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #keep  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
";
    let mut rw = ReloadWorld::new("multi", initial);
    let keep = rw.find_named("keep").expect("keep");
    set_transient(&mut rw, keep, 42);

    // Reload 1: add a sibling.
    rw.reload("\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #keep  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
    #temp  UiLayout { layout_type: Column, width: Px(40), height: Px(40) }
");
    assert_eq!(rw.find_named("keep"), Some(keep), "keep survives reload 1");
    assert!(rw.find_named("temp").is_some(), "temp added in reload 1");

    // Reload 2: remove the sibling, patch keep.
    rw.reload("\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(100) }
    #keep  UiLayout { layout_type: Column, width: Px(77), height: Px(40) }
");
    assert_eq!(rw.find_named("keep"), Some(keep), "keep survives reload 2 (same entity)");
    assert!(rw.find_named("temp").is_none(), "temp removed in reload 2");
    assert_eq!(
        transient_of(&rw, keep),
        Some(Transient { marker: 42 }),
        "keep's transient state survives BOTH reloads"
    );
    assert!(
        layout_debug(&rw, keep).unwrap().contains("77.0"),
        "keep was patched to width Px(77) in reload 2"
    );
}
