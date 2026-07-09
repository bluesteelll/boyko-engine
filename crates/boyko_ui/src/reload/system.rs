//! The mtime+size poll watch system (P3 Decision 7 / 8).
//!
//! An EXCLUSIVE `fn(&mut EcsMaster)` on `CoreSchedule::Main`. Exclusive because
//! the reconcile must both READ live components (no entity-yielding `Query` on
//! this engine) AND issue structural `Commands`; the body reads an OWNED
//! snapshot first (releasing the world borrow), then replays commands through
//! `world.run_system` (which flushes the `Commands` deferred buffer).
//!
//! # Strict early-return (the zero-alloc / zero-tree-read invariant)
//!
//! 1. throttle: `last_poll.elapsed() < poll_interval` → return (no syscall, no
//!    alloc, no tree read).
//! 2. `metadata()` + read `(mtime, size)`; on error → return (Decision 8).
//! 3. `(mtime, size) == (last_mtime, last_size)` → return (NO tree read, NO
//!    `query_entities`, NO snapshot — the no-change path is free).
//! 4. settle: if `(mtime, size) != pending` → record `pending`, return (wait one
//!    interval for the file to settle, Decision 8).
//! 5. confirmed-settled change → read + parse + snapshot + reconcile; on success
//!    update `(last_mtime, last_size)` and clear `pending`.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;

use crate::reload::reconcile::{apply_despawns, reconcile_ui, DespawnPlan};
use crate::reload::state::UiHotReload;
use crate::reload::tree_view::UiTreeView;
use crate::text::parser::parse_ui;
use crate::text::report::UiParseReport;

/// The hot-reload watch system. See the module docs for the strict early-return
/// sequence and the exclusive-system rationale.
//
// `clippy::needless_pass_by_ref_mut`: `resource_mut` / `run_system` are
// `&mut self`, so the `&mut EcsMaster` IS required; clippy cannot see through
// the cross-crate method calls (mirrors `ui_layout_apply`).
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_hot_reload_system(world: &mut EcsMaster) {
    // The resource may be absent (hot-reload disabled): a graceful no-op.
    if world.try_resource_mut::<UiHotReload>().is_none() {
        return;
    }

    // 1. Throttle. Read-only fields off the resource; no alloc, no tree read.
    {
        let hot = world.resource::<UiHotReload>();
        if hot.last_poll.elapsed() < hot.poll_interval {
            return;
        }
    }

    // 2. metadata() — one syscall per interval. Stamp last_poll regardless of
    //    the outcome so a missing file does not busy-poll.
    let path = world.resource::<UiHotReload>().path;
    let sig = read_signature(path);
    {
        let hot = world.resource_mut::<UiHotReload>();
        hot.last_poll = std::time::Instant::now();
    }
    let Some((mtime, size)) = sig else {
        return; // missing / unreadable: retry next interval (Decision 8)
    };

    // 3. No-change fast path — ZERO tree read, ZERO snapshot.
    {
        let hot = world.resource::<UiHotReload>();
        if hot.last_mtime == Some(mtime) && hot.last_size == size {
            return;
        }
    }

    // 4. Settle: only act when the SAME (mtime, size) is observed twice in a row
    //    (the file has been stable for >= one interval), so a torn / half-written
    //    file at a coincidentally-final size is not reconciled (Decision 8).
    {
        let hot = world.resource_mut::<UiHotReload>();
        if hot.pending != Some((mtime, size)) {
            hot.pending = Some((mtime, size));
            return; // first sighting — wait for it to settle
        }
    }

    // 5. Confirmed-settled change. Read + parse + reconcile.
    let Ok(src) = std::fs::read_to_string(path) else {
        // Vanished between metadata and read: retry next interval.
        return;
    };
    // Validity net (Decision 8): an empty / unparseable file is not reconciled —
    // do NOT update the signature, so a genuinely-broken state retries.
    if src.is_empty() {
        return;
    }
    let parsed = parse_ui(&src);
    if parsed.is_empty() && !parsed.report.is_clean() {
        return; // zero usable nodes WITH errors → broken, skip
    }

    // Build the live snapshot from the document's scoped roots, then reconcile
    // via a `(ResMut<UiHotReload>, Commands)` closure. The snapshot is OWNED, so
    // no world borrow is held while the commands are emitted; `run_system`
    // flushes the command buffer at apply.
    let roots = world.resource::<UiHotReload>().doc_roots.to_vec();
    let live = UiTreeView::build(world, &roots);
    reconcile_in_world(world, parsed, live);

    // On success, record the settled signature and clear pending.
    let hot = world.resource_mut::<UiHotReload>();
    hot.last_mtime = Some(mtime);
    hot.last_size = size;
    hot.pending = None;
}

/// Reads `(mtime, size)` for `path`, or `None` on any I/O error.
fn read_signature(path: &str) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some((mtime, meta.len()))
}

/// Runs the reconcile inside the world as a TWO-PHASE apply with a forced drain
/// barrier between the passes (Decision 13, alternative (a) — the soundness fix).
///
/// The reconcile needs `&mut UiHotReload` (the scope) AND a `Commands`. A
/// FunctionSystem cannot supply both a `ResMut` and read arbitrary live
/// components, but the live state is already in the owned `live` snapshot, so the
/// reconcile only needs the resource + Commands — both available to a
/// `(ResMut<UiHotReload>, Commands)` system.
///
/// # The drain barrier (move-vs-despawn soundness)
///
/// `run_system` ends each call with a FULL apply window (`system.apply` then
/// `drain_deferred_hook_queue`, `ecs_master.rs:2456/2465`). So:
///
/// 1. **Phase 1** (`reconcile_ui`) emits the match/patch/spawn commands and the
///    `set_parent`/`remove_parent` reparents for moved survivors, and returns the
///    [`DespawnPlan`]. The first `run_system` then DRAINS — this is the barrier:
///    the reparent's `ChildOf` overwrite/remove fires `child_of_on_replace`,
///    which enqueues `UnlinkChildCommand` into the deferred-hook queue, and the
///    barrier's drain runs that unlink. After it, every moved survivor is provably
///    absent from its old (doomed) parent's live `Children`.
/// 2. **Phase 2** (`apply_despawns`) despawns the doomed parents in a SECOND
///    `run_system`. Their cascade now reads a `Children` from which the moved
///    survivors were already unlinked, so the cascade cannot reach a survivor.
///
/// The single-batch ordering (despawn in the same queue as the reparent) is
/// unsound: the reparent's unlink is a two-stage deferral whose second stage
/// drains too late (after the cascade has already read the doomed parent's
/// `Children`).
fn reconcile_in_world(world: &mut EcsMaster, parsed: crate::text::ast::ParsedTree, live: UiTreeView) {
    use boyko_ecs::ecs::core::system::ResMut;

    // Smuggle the phase-1 result (the despawn plan + the lower-time report) out of
    // the `Send + Sync + 'static` system closure (the established Phase-11/19
    // probe pattern). The watch system is single-threaded exclusive, so this is
    // never contended; the `Mutex` only satisfies the closure's `Sync` bound.
    let plan_sink: Arc<Mutex<DespawnPlan>> = Arc::new(Mutex::new(DespawnPlan::default()));
    let report_sink: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::new()));

    // PHASE 1: match / patch / spawn / relink (the reparent detaches a moved
    // survivor from its old parent).
    {
        let plan_probe = Arc::clone(&plan_sink);
        let report_probe = Arc::clone(&report_sink);
        world.run_system(move |mut hot: ResMut<UiHotReload>, mut cmds: Commands| {
            let mut report = UiParseReport::new();
            let plan = reconcile_ui(&parsed, &mut hot, &live, &mut cmds, &mut report);
            *plan_probe.lock().expect("reconcile plan probe") = plan;
            *report_probe.lock().expect("reconcile report probe") = report;
        });
        // <-- run_system's apply + drain ran here: the DRAIN BARRIER. Every
        //     survivor's two-stage `ChildOf` unlink has now materialised.
    }

    // Surface phase-1 lower-time re-parse errors (Finding: the lowering report
    // must be reachable). Record it into the watch resource's recoverable channel
    // so a host can inspect `UiHotReload::last_report` after a reload — Principle-0
    // ECS-resident observability, no new dependency, never silently dropped.
    {
        let report = std::mem::take(&mut *report_sink.lock().expect("reconcile report probe"));
        world.resource_mut::<UiHotReload>().last_report = report;
    }

    // PHASE 2: despawn the doomed parents, post-barrier. Skip the second
    // `run_system` entirely when nothing was deleted (the common reload).
    let plan = std::mem::take(&mut *plan_sink.lock().expect("reconcile plan probe"));
    if !plan.is_empty() {
        world.run_system(move |mut cmds: Commands| {
            apply_despawns(&plan, &mut cmds);
        });
    }
}
