//! G — Miri (Tree Borrows) coverage for the relation JOIN's only new unsafe:
//! `Related::fetch` target resolution (`related.rs`).
//!
//! Run via (matches the sibling `miri_relations.rs` flags):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! rustup run nightly-2026-05-20-x86_64-pc-windows-gnu cargo miri test -p boyko-ecs --test miri_relations_query
//! ```
//!
//! The join's `fetch` performs, per source row:
//!   1. read the source's own `R` FK (`r_base + row*stride`),
//!   2. resolve `entities_inland[target.id]` (the first dependent random load),
//!   3. generation-check the target slot,
//!   4. read the TARGET archetype's `component_mask()` + `entity_count()` THROUGH
//!      the raw `archetype_ptr` (the reviewer's O1 concern — the target archetype
//!      differs from the source archetype),
//!   5. build a TRANSIENT inner `D::Fetch` against the target archetype and gather
//!      `D` at the target row.
//!
//! THE point: the target-archetype read is a SHARED read through a raw slab pointer
//! that is DISTINCT from the source archetype's pointer the cursor holds. Under Tree
//! Borrows this must be a clean shared reborrow (no foreign-write protector
//! violation, no aliasing of the source cursor's provenance).
//!
//! `#![cfg(miri)]` — only compiles under Miri. Entity counts are TINY.

#![cfg(miri)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::filter::With;
use boyko_ecs::ecs::core::iters::query::relation::Related;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: i64,
}

/// A SECOND component that the child carries but the parent does NOT — this forces
/// the parent and child into DIFFERENT archetypes, so the join's target-archetype
/// read goes through a distinct raw slab pointer (the O1 surface).
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct ChildOnly {
    k: u32,
}

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct MTag(u32);

/// Spawns one `Pos`-only entity (the parent archetype) and returns its handle.
fn spawn_pos(ecs: &mut EcsMaster, x: i64) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        *probe.lock().expect("probe") = Some(cmds.spawn(Pos { x }).id());
    });
    sink.lock().expect("probe").expect("spawned")
}

/// Spawns one `MTag`-only entity and returns its handle.
fn spawn_tag(ecs: &mut EcsMaster, t: u32) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        *probe.lock().expect("probe") = Some(cmds.spawn(MTag(t)).id());
    });
    sink.lock().expect("probe").expect("spawned")
}

// ════════════════════════════════════════════════════════════════════════════
// G.1 — cross-archetype target resolution: parent (Pos only) vs child
//        (Pos + ChildOnly + ChildOf). The join reads the PARENT archetype's
//        mask/count through a raw ptr distinct from the child's archetype.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_related_cross_archetype_target_read_is_tb_clean() {
    let mut ecs = EcsMaster::new();

    // Parent: archetype {Pos} (x = 7).
    let parent = spawn_pos(&mut ecs, 7);

    // Child: archetype {Pos, ChildOnly, ChildOf} — DIFFERENT from the parent's.
    let p = parent;
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(Pos { x: 999 })
            .insert(ChildOnly { k: 1 })
            .insert(ChildOf(p))
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let _child = sink.lock().expect("probe").expect("spawned");

    // The join: for the child row, read the PARENT's Pos through the parent's
    // (distinct) archetype pointer. Drive a full iteration under TB.
    let mut seen = Vec::new();
    for parent_pos in ecs
        .query::<Related<ChildOf, &Pos>, With<ChildOf>>()
        .iter()
    {
        seen.push(parent_pos.map(|p| p.x));
    }
    assert_eq!(seen, vec![Some(7)], "join reads the parent's Pos cross-archetype");
}

// ════════════════════════════════════════════════════════════════════════════
// G.2 — target lacks the inner component: the cross-archetype mask check returns
//        None (the `matches_component_set` on the TARGET archetype). Exercises the
//        raw target-mask read on a target archetype that does NOT host `Pos`.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_related_target_without_component_yields_none_tb_clean() {
    let mut ecs = EcsMaster::new();

    // Parent: archetype {MTag} — NO Pos.
    let parent = spawn_tag(&mut ecs, 0);
    // Child: archetype {Pos, ChildOf}.
    let p = parent;
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(Pos { x: 5 }).insert(ChildOf(p)).id();
        *probe.lock().expect("probe") = Some(e);
    });
    let _child = sink.lock().expect("probe").expect("spawned");

    let mut seen = Vec::new();
    for parent_pos in ecs
        .query::<Related<ChildOf, &Pos>, With<ChildOf>>()
        .iter()
    {
        seen.push(parent_pos.map(|p| p.x));
    }
    // The parent archetype does not host Pos ⇒ the raw target-mask check yields None.
    assert_eq!(seen, vec![None], "target lacks Pos ⇒ None (raw target-mask read clean)");
}

// ════════════════════════════════════════════════════════════════════════════
// G.3 — multiple children of distinct-archetype parents, several rows in one
//        iteration: the transient inner D::Fetch is rebuilt PER ROW against the
//        per-row target archetype (no stale fetch carried across rows).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_related_per_row_fetch_rebuild_across_targets_tb_clean() {
    let mut ecs = EcsMaster::new();

    // Two parents in the same {Pos} archetype, different rows.
    let pa = spawn_pos(&mut ecs, 10);
    let pb = spawn_pos(&mut ecs, 20);

    // Children point at distinct parents; the join rebuilds the inner fetch per
    // row against each parent's row in the target archetype.
    let (a, b) = (pa, pb);
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut s = probe.lock().expect("probe");
        s.push(cmds.spawn(MTag(1)).insert(ChildOf(a)).id());
        s.push(cmds.spawn(MTag(2)).insert(ChildOf(b)).id());
    });
    let _children = sink.lock().expect("probe").clone();

    let mut seen: Vec<i64> = ecs
        .query::<Related<ChildOf, &Pos>, With<ChildOf>>()
        .iter()
        .filter_map(|p| p.map(|p| p.x))
        .collect();
    seen.sort_unstable();
    assert_eq!(seen, vec![10, 20], "each child resolves its own parent's Pos row (per-row rebuild)");
}
