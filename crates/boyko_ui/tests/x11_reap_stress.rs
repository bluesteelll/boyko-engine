//! X-11 — REAP STRESS: the `.ui` hot-reload path must never adopt a dead
//! entity's state after its id has been RECYCLED onto a stranger.
//!
//! # Why this gate is constructible only now
//!
//! `UiHotReload` stores the document's root `Entity` handles (`doc_roots`) ACROSS
//! frames — they are the reconcile's scope anchor (P3 Decision 10). A handle held
//! across frames is only dangerous once ids come BACK: until EM2′ (`0afcbd7d`)
//! `Commands::spawn` minted every id through a bare `fetch_add` that never looked
//! at the free list, so a despawned id was never re-issued on the deferred route
//! and a stale `doc_roots` entry could not collide with anything. A test written
//! before that commit would have churned forever without ever reproducing the
//! collision — green from emptiness. EM2′ made the collision reachable, which is
//! what makes the guard against it testable.
//!
//! # What is guarded, and where
//!
//! `UiTreeView::build` skips a root that is not live
//! (`if !world.has_entity(entity) { continue; }`, `reload/tree_view.rs`). That one
//! line is the whole identity check: `Entity` carries a generation, a recycled id
//! is re-registered with a HIGHER one (EM3, re-proved by EM2′'s rejection-path
//! fix), so the stale handle compares unequal to the live one and `has_entity`
//! rejects it. Without the skip, the walk would snapshot whatever now occupies
//! that id — a stranger — into the document's `UiTreeView`, and the reconcile
//! would then patch the document's authored values onto it.
//!
//! # The two gates
//!
//! * [`recycled_root_id_is_not_adopted_by_the_tree_view`] drives the seam
//!   directly: build the view from a stale root whose id is PROVEN recycled onto
//!   a live stranger, and require an EMPTY view.
//! * [`reload_over_recycled_roots_leaves_the_strangers_untouched`] drives the
//!   real wiring — `UiPlugin` + `App` + the file watch — and requires the
//!   strangers to survive a reload byte-unchanged and un-UI-ified.
//!
//! # Anti-vacuity
//!
//! Both gates ASSERT that recycling actually happened before asserting anything
//! about the reconcile. If the reservoir stops re-issuing ids, these tests go RED
//! on the recycling clause rather than passing over a collision that never
//! occurred — the failure mode this repository catalogues as "green from
//! emptiness".
//!
//! # Red-first receipt, and what the mutation did NOT red
//!
//! Proved RED by deleting the three-line `has_entity` skip in
//! `crates/boyko_ui/src/reload/tree_view.rs::UiTreeView::build`; the file was
//! restored from its recorded SHA-256 afterwards (the receipt is in the merge
//! commit message). Under that mutation Gate 1 fails with
//! `the view built from a stale root must be EMPTY — it adopted 1 node(s)`.
//!
//! **Gate 2 stayed GREEN under the same mutation, and that is stated rather than
//! papered over.** The reason is a SECOND, independent mechanism: the reconcile's
//! writes go out as deferred commands keyed by the stale handle, and the kernel
//! validates the generation at the apply window (EM3), so a write aimed at a
//! recycled id is rejected there even when the view has already been poisoned.
//! Gate 2 is therefore a ROUTE-level regression gate over the real wiring — it
//! would catch a future change that reached the stranger through any path — and
//! not a mutation-proved one. Only Gate 1 pins the `has_entity` line itself.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling spawned `Entity` handles out of the `Send + Sync` one-shot system
// closure. Not engine code — the whole file is compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Component;

use boyko_ui::components::{UiLayout, UiName};
use boyko_ui::reload::tree_view::UiTreeView;
use boyko_ui::text::UiText;

use p3_common::{discover_ui_roots, spawn_dot_ui, ReloadWorld};

// ──────────────────────────────── the corpus ──────────────────────────────────

/// A two-node document: a named root and a named child, so the despawn puts at
/// least two ids on the reservoir and the reconcile has a key to match on.
const DOC_V1: &str = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(100), height: Px(200) }
    UiText { color: 4278255615, size_px: 12.5, font: 1, align: Left }
    #leaf  UiLayout { layout_type: Row, width: Px(30), height: Px(40) }
";

/// The reloaded document: the same two keys, different authored values. What the
/// reconcile would try to write onto a stranger if it adopted one.
const DOC_V2: &str = "\
version=1
#root  UiLayout { layout_type: Row, width: Px(300), height: Px(400) }
    UiText { color: 4278190335, size_px: 25.5, font: 2, align: Right }
    #leaf  UiLayout { layout_type: Column, width: Px(60), height: Px(80) }
";

/// Entities spawned per churn wave. The reservoir is a LIFO stack, so the ids a
/// despawn just released come back on the next wave; the wave is wide enough to
/// drain a small document's worth of them in one pass.
const CHURN_PER_WAVE: usize = 64;

/// How many waves to churn before giving up on a recycled id. A bound rather than
/// a loop, so a reservoir that stopped re-issuing is a RED with a message instead
/// of a hang.
const MAX_WAVES: usize = 16;

// ───────────────────────────── the stranger marker ────────────────────────────

/// A component NO `.ui` document can author and no UI system writes. Its value is
/// the whole evidence of Gate 2: if it still reads what the churn wrote, nothing
/// adopted the entity; if the entity is gone, something reaped it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
struct Stranger {
    /// The wave that spawned it, times 1000, plus its index — a value distinct
    /// per stranger and distinct from every default.
    tag: u32,
}

// ──────────────────────────────── helpers ─────────────────────────────────────

/// Despawns `victims` (cascading to their children) through `Commands`, then
/// asserts every handle is dead.
fn despawn_all(world: &mut EcsMaster, victims: &[Entity]) {
    let owned = victims.to_vec();
    world.run_system(move |mut cmds: Commands| {
        for &e in &owned {
            cmds.entity(e).despawn();
        }
    });
    for &e in victims {
        assert!(
            !world.has_entity(e),
            "the churn's premise: a despawned handle must be dead before its id can be recycled \
             onto a stranger; {e:?} is still live"
        );
    }
}

/// Churns `Stranger`-tagged spawns through `Commands` until every id in `wanted`
/// has been re-issued, returning `(recycled_id, new_handle)` for each.
///
/// Panics (RED, not a silent skip) if the reservoir has not re-issued all of them
/// within [`MAX_WAVES`] — a gate about recycling that never observed a recycle is
/// a gate that cannot fail.
fn churn_until_recycled(world: &mut EcsMaster, wanted: &[EntityId]) -> Vec<(EntityId, Entity)> {
    let mut found: Vec<(EntityId, Entity)> = Vec::new();
    for wave in 0..MAX_WAVES {
        let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
        let probe = Arc::clone(&sink);
        let base = (wave as u32 + 1) * 1000;
        world.run_system(move |mut cmds: Commands| {
            let mut out = probe.lock().expect("probe");
            for i in 0..CHURN_PER_WAVE {
                let e = cmds.spawn(Stranger { tag: base + i as u32 }).id();
                out.push(e);
            }
        });
        let spawned = sink.lock().expect("probe").clone();
        for e in spawned {
            if wanted.contains(&e.id()) && !found.iter().any(|(id, _)| *id == e.id()) {
                found.push((e.id(), e));
            }
        }
        if found.len() == wanted.len() {
            return found;
        }
    }
    panic!(
        "the reservoir did not re-issue every despawned id within {MAX_WAVES} waves of \
         {CHURN_PER_WAVE} spawns, so this gate never reproduced the collision it exists to test \
         (wanted {} ids, recycled {}): a pass here would be green from emptiness",
        wanted.len(),
        found.len()
    );
}

// ──────────────────────────────── Gate 1 ──────────────────────────────────────

/// The `UiTreeView` walk must SKIP a root handle whose id has been recycled onto
/// a live stranger, rather than snapshotting the stranger into the document.
#[test]
fn recycled_root_id_is_not_adopted_by_the_tree_view() {
    let mut world = EcsMaster::new();
    let roots = spawn_dot_ui(&mut world, DOC_V1);
    assert_eq!(roots.len(), 1, "the corpus declares exactly one root");
    let stale = roots[0];
    let stale_id = stale.id();
    let stale_gen = stale.generation();

    // The view a LIVE root produces — the thing the stale one must NOT produce.
    let live_view = UiTreeView::build(&world, &roots);
    assert_eq!(
        live_view.nodes.len(),
        2,
        "the corpus's root + leaf must both be reachable while the document is alive, or Gate 1's \
         'empty view' assertion would hold for a reason that has nothing to do with the guard"
    );

    despawn_all(&mut world, &roots);
    let recycled = churn_until_recycled(&mut world, &[stale_id]);
    let (_, stranger) = recycled[0];

    // The collision is REAL: same id, different generation, the stranger live and
    // the old handle dead.
    assert_eq!(stranger.id(), stale_id, "the stranger occupies the dead root's id");
    assert_ne!(
        stranger.generation(),
        stale_gen,
        "a recycled id must come back with a NEW generation (EM3); equal generations would mean \
         the stale handle was revived, which is the ABA defect EM2′ closed"
    );
    assert!(world.has_entity(stranger), "the stranger is live");
    assert!(!world.has_entity(stale), "the old root handle is dead");

    // The seam.
    let view = UiTreeView::build(&world, &[stale]);
    assert!(
        view.nodes.is_empty(),
        "the view built from a stale root must be EMPTY — it adopted {} node(s) instead, so the \
         document's snapshot now contains an entity that is not part of the document: {:?}",
        view.nodes.len(),
        view.nodes.iter().map(|n| n.entity).collect::<Vec<_>>()
    );
    assert!(
        !view.nodes.iter().any(|n| n.entity.id() == stale_id),
        "no node in the document's view may carry the recycled id"
    );
}

// ──────────────────────────────── Gate 2 ──────────────────────────────────────

/// Through the REAL wiring (`UiPlugin` + `App` + the file watch): a reload whose
/// stored `doc_roots` have all been recycled onto strangers must leave every
/// stranger alive, un-UI-ified and byte-unchanged.
#[test]
fn reload_over_recycled_roots_leaves_the_strangers_untouched() {
    let mut rw = ReloadWorld::new("x11_reap", DOC_V1);
    let doc_roots = discover_ui_roots(rw.world());
    assert_eq!(doc_roots.len(), 1, "the corpus declares exactly one root");
    let wanted: Vec<EntityId> = doc_roots.iter().map(|e| e.id()).collect();

    // Reap the document out from under the watch resource, which keeps its stale
    // `doc_roots` — the state a reload then reconciles against.
    despawn_all(rw.app.world_mut(), &doc_roots);
    let recycled = churn_until_recycled(rw.app.world_mut(), &wanted);
    let strangers: Vec<Entity> = recycled.iter().map(|(_, e)| *e).collect();
    let before: Vec<Stranger> = strangers
        .iter()
        .map(|&e| {
            *rw.world()
                .get_component::<Stranger>(e)
                .expect("invariant: the churn just spawned this stranger with a Stranger tag")
        })
        .collect();

    rw.reload(DOC_V2);

    // Anti-vacuity: the reload must actually have run. With no live root to match,
    // the reconcile respawns the document from scratch.
    let fresh = discover_ui_roots(rw.world());
    assert_eq!(
        fresh.len(),
        1,
        "the reload must have respawned the document (no live root survived the reap); found {} \
         root(s) — without this the gate below would pass over a reload that never happened",
        fresh.len()
    );
    assert!(
        fresh.iter().all(|e| !wanted.contains(&e.id()) || !strangers.contains(e)),
        "the respawned document must not BE one of the strangers"
    );

    for (&e, want) in strangers.iter().zip(before.iter()) {
        assert!(
            rw.world().has_entity(e),
            "a stranger holding a recycled document id was REAPED by the reload's despawn plan \
             ({e:?}); the plan may only despawn entities the document's own view produced"
        );
        let got = rw.world().get_component::<Stranger>(e).copied();
        assert_eq!(
            got,
            Some(*want),
            "a stranger's own state changed across the reload ({e:?})"
        );
        assert!(
            !rw.world().has_component(e, UiLayout::component_id()),
            "a stranger was UI-ified by the reload ({e:?}): it gained the document's `UiLayout`, \
             which is what adopting a dead entity's id looks like from the outside"
        );
        assert!(
            !rw.world().has_component(e, UiName::component_id()),
            "a stranger gained the document's `UiName` ({e:?})"
        );
        assert!(
            !rw.world().has_component(e, UiText::component_id()),
            "a stranger gained the document's `UiText` ({e:?})"
        );
    }
}
