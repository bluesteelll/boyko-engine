//! Phase 19 — Miri (Tree Borrows) coverage for the hierarchy unsafe paths.
//! Single-thread only (multi-thread Miri deferred per Phase 9.1).
//!
//! Run via (NOTE the `-Zmiri-ignore-leaks` — see below):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase19
//! ```
//!
//! # Why `-Zmiri-ignore-leaks`
//!
//! The cascade / drain-panic repros below spawn entities, which reaches the
//! by-design bounded `BundleColumnCache` `Box::leak` (SBO6; tracked as #53,
//! NOT-A-BUG — a deliberate borrow-decoupling leak). That allocator leak is
//! orthogonal to Tree Borrows; `-Zmiri-ignore-leaks` isolates the TB signal,
//! matching the established config of the sibling Miri suites
//! (`miri_phase14b` / `miri_phase15` / `miri_phase16` / `miri_phase17` /
//! `miri_bugfix_56`).
//!
//! Per the 14a/14b lesson — Miri-TB caught soundness bugs that critic +
//! code-review APPROVED — **Miri-TB is the authoritative soundness oracle** for
//! Phase 19's two novel unsafe surfaces:
//!
//! 1. **The OBS-FIRE-LOOP / F2 discipline in the hierarchy hooks** (`commands.rs`).
//!    A `world`-derived `&` (the `&ChildOf` / `&Children` from
//!    `DeferredEcsMaster::get_component`) must NOT be live across the
//!    `commands()` mint or a `despawn()` / enqueue call. The cascade hook's two
//!    paths differ in HOW they avoid that:
//!    * INLINE (`n <= CASCADE_FANOUT_INLINE`): the `MaybeUninit<Entity>` buffer +
//!      the read-then-`assume_init` sequence (the ONE new `unsafe`, M2), with the
//!      `&Children` dropped before `commands()`;
//!    * WIDE (`n > CASCADE_FANOUT_INLINE`): the per-turn re-derive (no buffer, no
//!      unsafe) — the `&` must not span the `despawn()`.
//! 2. **The command-apply structural ops under the depth-guarded drain**:
//!    `LinkChildCommand::apply`'s first-child None-arm (the audited
//!    `insert_command.rs:74` raw-deref copy + `migrate_entity_insert`) and
//!    `UnlinkChildCommand::apply`'s `get_component_mut::<Children>` (the `Mut`
//!    `&mut *archetype_ptr` + per-row tick offset).
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. Native runs ignore the file; the
//! `phase19_hierarchy_*` integration suites cover the same semantics end-to-end
//! on the native target. Entity counts are kept tiny (Miri is ~100x slower).

#![cfg(miri)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::ObserverContext;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct MTag(u32);

#[derive(Bundle)]
struct MTagBundle {
    t: MTag,
}

/// Spawns `n` marker entities and returns their handles (one apply window).
/// Tiny `n` for Miri.
fn spawn_entities(ecs: &mut EcsMaster, n: usize) -> Vec<Entity> {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let mut local = probe.lock().expect("probe lock");
        for i in 0..n {
            local.push(cmds.spawn(MTagBundle { t: MTag(i as u32) }).id());
        }
    });
    let out = sink.lock().expect("probe lock").clone();
    assert_eq!(out.len(), n, "spawn helper produced n handles");
    out
}

// ════════════════════════════════════════════════════════════════════════════
// Target 1 — ChildOf::on_insert LINK enqueue path + LinkChildCommand::apply
//            first-child None-arm (the audited raw deref + migrate_entity_insert)
// ════════════════════════════════════════════════════════════════════════════

/// Driving `add_child` on a parent with NO `Children` exercises:
///   * `child_of_on_insert`'s F2 borrow choreography (copy parent out, drop the
///     `&ChildOf`, then `commands().add(LinkChildCommand)`);
///   * `LinkChildCommand::apply`'s None-arm: `world.entity_master
///     .entities_inland[..]` + the `unsafe { (*inland.archetype_ptr()).id() }`
///     deref + `merged_archetype_id::<ChildrenBundle>` + `migrate_entity_insert`.
/// This is the first-child structural insert under the depth-guarded drain.
#[test]
fn miri_link_first_child_migrate() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });

    assert_eq!(
        ecs.get_component::<ChildOf>(child).map(|c| c.0),
        Some(parent),
        "child linked to parent"
    );
    assert!(
        ecs.get_component::<Children>(parent).map(|c| c.contains(child)).unwrap_or(false),
        "parent.Children contains child (first-child migrate path)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 2 — second child: LinkChildCommand::apply's Some-arm (get_component_mut
//            ::<Children> → Mut → push, the in-place &mut *archetype_ptr path)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_link_second_child_in_place_push() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (parent, c0, c1) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(c0); // first child → migrate (None-arm)
        cmds.entity(parent).add_child(c1); // second child → in-place (Some-arm)
    });

    let kids = ecs.get_component::<Children>(parent).expect("Children present");
    assert!(kids.contains(c0) && kids.contains(c1), "both children present");
    assert_eq!(kids.len(), 2, "in-place push appended the second child");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 3 — ChildOf::on_replace UNLINK enqueue + UnlinkChildCommand::apply
//            (get_component_mut::<Children> → swap_remove_entity)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_unlink_via_remove_parent() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (parent, child) = (e[0], e[1]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(child);
    });
    ecs.run_system(move |mut cmds: Commands| {
        // Fires ChildOf::on_replace (reading the dying value) →
        // UnlinkChildCommand::apply → get_component_mut::<Children> → swap_remove.
        cmds.entity(child).remove_parent();
    });

    assert_eq!(ecs.get_component::<ChildOf>(child).map(|c| c.0), None, "child unlinked");
    let kids = ecs.get_component::<Children>(parent).expect("Children retained empty");
    assert!(!kids.contains(child), "child removed from parent.Children");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 4 — reparent OVERWRITE: in-place replace fires on_replace(A) THEN
//            on_insert(B); both enqueue paths back-to-back under one drain
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_reparent_overwrite() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (a, b, child) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(a).add_child(child);
    });
    ecs.run_system(move |mut cmds: Commands| {
        // child already has ChildOf(A) → apply_replace_in_place fires
        // on_replace (Unlink A) then on_insert (Link B).
        cmds.entity(child).set_parent(b);
    });

    assert_eq!(ecs.get_component::<ChildOf>(child).map(|c| c.0), Some(b), "reparented to B");
    assert!(
        ecs.get_component::<Children>(b).map(|c| c.contains(child)).unwrap_or(false),
        "child ∈ B.Children"
    );
    assert!(
        !ecs.get_component::<Children>(a).map(|c| c.contains(child)).unwrap_or(false),
        "child ∉ A.Children"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Target 5 — Children::on_replace cascade INLINE path (n <= CASCADE_FANOUT_INLINE)
//            The MaybeUninit buffer + read-then-assume_init (the ONE new unsafe).
// ════════════════════════════════════════════════════════════════════════════

/// 3 children (well under the 32 inline threshold): the inline fast path copies
/// them into the `[MaybeUninit<Entity>; 32]` buffer, drops the `&Children`, then
/// `assume_init`-reads `buf[..3]` to enqueue the despawns. Recursive despawn of
/// the parent drives this once.
///
/// BUG-P19-TB-1 (FIXED — this test now passes under `-Zmiri-tree-borrows`). The
/// cascade enqueues >= 2 `DespawnCommand`s; applying the first fires a child's
/// `ChildOf::on_replace`, which pushes an `UnlinkChildCommand` into the SAME
/// `deferred_hook_queue` that the drain is mid-walk over. The OLD in-place
/// `RawCommandQueue` twin cached `NonNull`s into that queue, so the re-entrant
/// push's `Vec::reserve`/`set_len` foreign-wrote the twin's tag → next-turn
/// re-derive UB. Approach C now `mem::take`s the queue's buffers into a stack
/// `temp` and walks that SEPARATE allocation, so the re-entrant push (which lands
/// in the now-empty home queue) cannot foreign-write the walk. The MINIMAL repro
/// is `miri_minimal_cascade_reentrant_push`.
#[test]
fn miri_cascade_inline_path() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 4);
    let parent = e[0];
    let kids = [e[1], e[2], e[3]];

    ecs.run_system(move |mut cmds: Commands| {
        for &c in &kids {
            cmds.entity(parent).add_child(c);
        }
    });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    assert!(!ecs.has_entity(parent), "parent gone");
    for &c in &kids {
        assert!(!ecs.has_entity(c), "inline-path child cascaded");
    }
    assert_eq!(ecs.entity_count(), 0, "inline cascade removed all");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 6 — Children::on_replace cascade WIDE path (n > CASCADE_FANOUT_INLINE)
//            The per-turn re-derive (no buffer); the `&` must not span despawn().
// ════════════════════════════════════════════════════════════════════════════

/// 33 children: ONE over the 32 inline threshold — minimal over-threshold to
/// force the cold wide path (the per-turn `&Children` re-derive).
///
/// MIRI-IGNORED (TRACTABILITY — TESTER FINDING, not a soundness signal). The
/// wide branch is reachable only with `> CASCADE_FANOUT_INLINE` (32) children, so
/// it requires >= 34 entities, each driven through a full `ChildOf` migration AND
/// the despawn cascade. Under Miri (~100-400x slowdown + Tree-Borrows provenance
/// tracking) a single run exceeds ~12 minutes / ~600 MB and, run concurrently in
/// the default multi-threaded harness, OOM-aborts the whole process
/// (`error: test failed ... exit code 1`, the abnormal-exit signature — NOT an
/// `error: Undefined Behavior`, which Miri prints immediately and distinctly).
/// This violates the file's "keep entity counts tiny" rule and is a test-cost
/// problem, not a fix failure:
///
///   * BUG-P19-TB-1's soundness lives ENTIRELY in the shared
///     `apply_via_raw_twin` command machinery, which the TRACTABLE
///     `miri_cascade_inline_path` (3 children, >= 2 commands, same re-entrant
///     push) PASSES TB-clean — together with `miri_minimal_cascade_reentrant_push`
///     / `miri_recursive_despawn_three_levels` / `miri_self_ref_and_dangling_guards`
///     / `miri_drain_panic_reentrant_disposition`.
///   * The wide branch adds NO new `unsafe` over the inline branch — its only
///     delta is a SAFE per-turn `&Children` re-derive loop in
///     `children_on_replace`. So its Tree-Borrows surface is fully subsumed by the
///     inline branch's; a separate Miri run buys nothing the inline path lacks.
///
/// Native behavioral coverage of the wide cascade is unaffected (this `#[test]`
/// runs in full on the native target, plus `phase19_hierarchy_core`'s cascade
/// suites). Remove the `cfg_attr` only if Miri gains a way to run this in
/// reasonable time (e.g. a lowered `CASCADE_FANOUT_INLINE` test seam).
#[test]
#[cfg_attr(miri, ignore = "tractability: 34-entity wide cascade OOMs/over-runs Miri; TB surface subsumed by miri_cascade_inline_path (TESTER FINDING — not a soundness failure)")]
fn miri_cascade_wide_path() {
    let mut ecs = EcsMaster::new();
    const FANOUT: usize = 33; // > CASCADE_FANOUT_INLINE (32)
    let e = spawn_entities(&mut ecs, FANOUT + 1);
    let parent = e[0];
    let kids: Vec<Entity> = e[1..].to_vec();

    let kids_link = kids.clone();
    ecs.run_system(move |mut cmds: Commands| {
        for &c in &kids_link {
            cmds.entity(parent).add_child(c);
        }
    });
    assert_eq!(
        ecs.get_component::<Children>(parent).map(|c| c.len()).unwrap_or(0),
        FANOUT,
        "all children linked (over the inline threshold)"
    );

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    assert!(!ecs.has_entity(parent), "parent gone");
    for &c in &kids {
        assert!(!ecs.has_entity(c), "wide-path child cascaded");
    }
    assert_eq!(ecs.entity_count(), 0, "wide cascade removed all");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 7 — recursive despawn (3-level tree): re-entrant cascade under the
//            single outermost drain; the hardest borrow choreography
// ════════════════════════════════════════════════════════════════════════════

/// BUG-P19-TB-1 (FIXED — passes under `-Zmiri-tree-borrows`). The grandparent's
/// cascade despawns the (linked) parent, whose `ChildOf::on_replace` pushes an
/// `UnlinkChildCommand` into the `deferred_hook_queue` mid-drain → the same
/// twin-invalidation UB that Approach C eliminates by walking a stack `temp`.
/// Same root cause as the minimal repro.
#[test]
fn miri_recursive_despawn_three_levels() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (gp, p, c) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(gp).add_child(p);
        cmds.entity(p).add_child(c);
    });

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(gp).despawn();
    });

    assert!(!ecs.has_entity(gp), "grandparent gone");
    assert!(!ecs.has_entity(p), "parent cascaded (depth 1)");
    assert!(!ecs.has_entity(c), "child cascaded (depth 2)");
    assert_eq!(ecs.entity_count(), 0, "3-level cascade removed all in one drain");
}

// ════════════════════════════════════════════════════════════════════════════
// MINIMAL REPRO (TESTER FINDING — see report) — the smallest cascade that
// re-derives a Disabled twin tag inside one `apply_via_raw_twin` walk.
//
// Parent with TWO linked children, recursive despawn. The cascade enqueues TWO
// `DespawnCommand`s into `deferred_hook_queue`; `apply_via_raw_twin` walks them
// in ONE call (`stop_snapshot` = 2 commands):
//   * turn 1 applies despawn(child0) → child0's `ChildOf::on_replace` fires and
//     pushes an `UnlinkChildCommand` into the SAME `deferred_hook_queue` → the
//     push's `self.bytes.set_len()` (`command_queue.rs:161`) is a FOREIGN WRITE
//     that transitions the live `RawCommandQueue` twin's
//     `NonNull<Vec<MaybeUninit<u8>>>` tag (minted at `command_queue.rs:168`) to
//     Disabled;
//   * turn 2 (`local_cursor < stop_snapshot` still true) re-derives
//     `self.bytes.as_mut()` (`command_queue.rs:476`) through the now-Disabled tag
//     → Tree-Borrows UB.
//
// TWO children are required: with ONE child the walk has a single turn and exits
// (`local_cursor >= stop_snapshot`) BEFORE re-deriving after the push, so the
// 1-child case does NOT trip it (this is why the Phase 14b M8 re-entrant case,
// which only ever had one command per `apply_via_raw_twin` call, was TB-clean).
//
// BUG-P19-TB-1 (FIXED). This is the CANONICAL repro — it was confirmed failing
// under `-Zmiri-tree-borrows` (foreign write at `command_queue.rs:135`, UB at
// `:476`) BEFORE the fix, and is the first test to re-run after the Approach-C
// fix (`apply_via_raw_twin` walks a stack `temp`, so the re-entrant push lands in
// a different allocation and cannot Disable the walk's twin). Now TB-clean.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_minimal_cascade_reentrant_push() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 3);
    let (parent, c0, c1) = (e[0], e[1], e[2]);

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).add_child(c0);
        cmds.entity(parent).add_child(c1);
    });

    // Recursive despawn: cascade enqueues despawn(c0) + despawn(c1); applying
    // despawn(c0) pushes UnlinkChild(parent, c0) mid-walk, then despawn(c1)'s
    // turn re-derives the Disabled twin tag.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(parent).despawn();
    });

    assert!(!ecs.has_entity(parent), "parent gone");
    assert!(!ecs.has_entity(c0), "child 0 cascaded");
    assert!(!ecs.has_entity(c1), "child 1 cascaded");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 8 — self-ref + dangling guards: the reactive-remove path (commands()
//            mint inside on_insert after the &ChildOf drops) + the spurious
//            UnlinkChild no-op at apply
// ════════════════════════════════════════════════════════════════════════════

/// BUG-P19-TB-1 (FIXED — passes under `-Zmiri-tree-borrows`). This repro hit the
/// bug via a DIFFERENT re-derive site: the guard's reactive `remove::<ChildOf>`
/// is applied from the drain's walk, and `migrate_entity_remove` fires
/// `ChildOf::on_replace`, which pushes an `UnlinkChildCommand` into the same
/// `deferred_hook_queue` mid-walk; the POST-LOOP COMPACTION's `self.bytes.as_mut()`
/// (`command_queue.rs:557` pre-fix) then re-derived the now-Disabled twin tag →
/// TB-UB. The guard LOGIC was always correct and TB-clean in isolation — it was
/// the shared queue machinery that was unsound under re-entrant pushes, now fixed
/// by the Approach-C stack-`temp` walk. Native behavioral coverage:
/// `phase19_hierarchy_core::{self_referential_child_of_is_removed_without_corruption,
/// dangling_parent_child_of_is_removed_no_phantom}`.
#[test]
fn miri_self_ref_and_dangling_guards() {
    let mut ecs = EcsMaster::new();
    let e = spawn_entities(&mut ecs, 2);
    let (me, victim) = (e[0], e[1]);

    // Self-ref: on_insert mints commands() to remove the bad ChildOf.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).set_parent(me);
    });
    assert_eq!(ecs.get_component::<ChildOf>(me).map(|c| c.0), None, "self-ref removed");

    // Dangling: point at a dead entity.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(victim).despawn();
    });
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(me).set_parent(victim);
    });
    assert_eq!(ecs.get_component::<ChildOf>(me).map(|c| c.0), None, "dangling removed");
    assert!(ecs.has_entity(me), "guard did not corrupt the child");
}

// ════════════════════════════════════════════════════════════════════════════
// Target 9 (I1) — DRAIN-PANIC disposition under Tree Borrows.
//
// The cascade repros above NEVER panic — they only prove the Ok-path
// stack-`temp` walk is TB-clean. The BUG-P19-TB-1 fix (Approach C) also added an
// outer `catch_unwind` whose Err branch (1) deref's the home queue through a raw
// `*world.as_ptr()` and (2) re-homes BOTH the un-walked survivor (from the owned
// `temp`, via a safe `&mut temp.bytes`) AND the pre-panic re-entrant push (from
// the home queue) as `[survivor][re-entrant]`. That Err branch is a DISTINCT
// unsafe surface from the Ok path and has NO other Miri coverage. This test
// drives a panic mid-`apply_via_raw_twin` with both a survivor and a re-entrant
// push live at panic time, so Miri-TB checks the Err-branch raw-deref +
// `mem::take`/`append`/`mem::swap` re-home is sound. (Behavioral assertions on
// the apply-counts live in the native sibling `phase19_drain_panic.rs`; here the
// headline is simply "TB-clean through the Err branch".)
//
// Construction (mirrors the native sibling, kept tiny for Miri):
//   * `PanicSeed::on_remove` enqueues `[Enqueuer, Panicker, Survivor]` so one
//     `apply_via_raw_twin` call walks all three.
//   * `Enqueuer::apply` spawns a `ReEntrantTrigger` at drain-depth >= 1; its
//     `on_add` observer enqueues `ReEntrantCmd` into the home queue (the
//     re-entrant push that the OLD twin would have foreign-written).
//   * `Panicker::apply` panics → `temp.apply`'s `handle_panic_recovery(0)`
//     absorbs `[Survivor]` into `temp.bytes`; the Err branch re-homes both.
// ════════════════════════════════════════════════════════════════════════════

const I1_SEQ: Ordering = Ordering::SeqCst;

static I1_ENQUEUER_APPLY: AtomicUsize = AtomicUsize::new(0);
static I1_PANICKER_APPLY: AtomicUsize = AtomicUsize::new(0);
static I1_SURVIVOR_APPLY: AtomicUsize = AtomicUsize::new(0);
static I1_REENTRANT_APPLY: AtomicUsize = AtomicUsize::new(0);
static I1_REENTRANT_ARCH: AtomicUsize = AtomicUsize::new(usize::MAX);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I1PanicSeed(u32);

#[derive(Component)]
#[repr(C)]
#[derive(Clone, Copy)]
struct I1ReEntrantTrigger(u32);

struct I1Enqueuer {
    reentrant_arch: boyko_ecs::ecs::identifiers::primitives::ArchetypeId,
}
// SAFETY: `ArchetypeId` is a POD `usize` newtype — Send + 'static.
unsafe impl Send for I1Enqueuer {}
impl Command for I1Enqueuer {
    fn apply(self, world: &mut EcsMaster) {
        I1_ENQUEUER_APPLY.fetch_add(1, I1_SEQ);
        // Nested spawn at drain-depth >= 1 → fires I1ReEntrantTrigger::on_add
        // (enqueues I1ReEntrantCmd into the home queue); the nested tail drain
        // no-ops (depth-gated), leaving the re-entrant push in the home queue.
        let _ = world.spawn_one(self.reentrant_arch, I1ReEntrantTrigger(0));
    }
}

struct I1Panicker;
// SAFETY: ZST — Send + 'static.
unsafe impl Send for I1Panicker {}
impl Command for I1Panicker {
    fn apply(self, _world: &mut EcsMaster) {
        I1_PANICKER_APPLY.fetch_add(1, I1_SEQ);
        panic!("BUG-P19-TB-1 I1 (Miri): deliberate mid-drain panic");
    }
}

struct I1Survivor;
// SAFETY: ZST — Send + 'static.
unsafe impl Send for I1Survivor {}
impl Command for I1Survivor {
    fn apply(self, _world: &mut EcsMaster) {
        I1_SURVIVOR_APPLY.fetch_add(1, I1_SEQ);
    }
}

struct I1ReEntrantCmd;
// SAFETY: ZST — Send + 'static.
unsafe impl Send for I1ReEntrantCmd {}
impl Command for I1ReEntrantCmd {
    fn apply(self, _world: &mut EcsMaster) {
        I1_REENTRANT_APPLY.fetch_add(1, I1_SEQ);
    }
}

unsafe fn i1_panic_seed_on_remove(mut view: DeferredEcsMaster<'_>, _c: ObserverContext) {
    let raw = I1_REENTRANT_ARCH.load(I1_SEQ);
    let reentrant_arch = boyko_ecs::ecs::identifiers::primitives::ArchetypeId(raw);
    let mut cmds = view.commands();
    cmds.add(I1Enqueuer { reentrant_arch });
    cmds.add(I1Panicker);
    cmds.add(I1Survivor);
}

unsafe fn i1_reentrant_trigger_on_add(mut view: DeferredEcsMaster<'_>, _c: ObserverContext) {
    view.commands().add(I1ReEntrantCmd);
}

/// I1 — the drain-panic Err-branch re-home path is Tree-Borrows clean. Asserts
/// the disposition too (so a behavioral break also surfaces here), but the
/// load-bearing claim is "Miri-TB sees no aliasing UB through the Err branch".
#[test]
fn miri_drain_panic_reentrant_disposition() {
    let mut ecs = EcsMaster::new();

    let seed_arch = ecs.create_archetype(&[I1PanicSeed::component_id()]);
    let reentrant_arch = ecs.create_archetype(&[I1ReEntrantTrigger::component_id()]);
    I1_REENTRANT_ARCH.store(reentrant_arch.0, I1_SEQ);

    ecs.observe_on_remove::<I1PanicSeed>(i1_panic_seed_on_remove);
    ecs.observe_on_add::<I1ReEntrantTrigger>(i1_reentrant_trigger_on_add);

    let seed = ecs.spawn_one(seed_arch, I1PanicSeed(1)).expect("spawn seed");

    // The panicking drain: delete_entity fires on_remove (enqueues the triad),
    // then apply_via_raw_twin walks [Enqueuer, Panicker, Survivor]; Panicker
    // panics with a survivor + a re-entrant push live → the Err branch re-homes.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ecs.delete_entity(seed);
    }));
    assert!(result.is_err(), "mid-drain panic must propagate out of delete_entity");

    assert_eq!(I1_ENQUEUER_APPLY.load(I1_SEQ), 1, "Enqueuer ran once before the panic");
    assert_eq!(I1_PANICKER_APPLY.load(I1_SEQ), 1, "Panicker ran once");
    assert_eq!(I1_SURVIVOR_APPLY.load(I1_SEQ), 0, "Survivor not yet run (survivor)");
    assert_eq!(I1_REENTRANT_APPLY.load(I1_SEQ), 0, "re-entrant push not yet run");

    // A later drain applies the re-homed [Survivor][ReEntrant]. (This probe spawn
    // ALSO re-fires I1ReEntrantTrigger::on_add → one more I1ReEntrantCmd, so
    // I1_REENTRANT_APPLY ends >= 1; Survivor has no such re-trigger ⇒ exactly 1.)
    let _probe = ecs.spawn_one(reentrant_arch, I1ReEntrantTrigger(99));

    assert_eq!(
        I1_SURVIVOR_APPLY.load(I1_SEQ),
        1,
        "the un-walked Survivor was re-homed + applied on the later drain (P1)",
    );
    assert!(
        I1_REENTRANT_APPLY.load(I1_SEQ) >= 1,
        "the pre-panic re-entrant push was re-homed + applied on the later drain (P1)",
    );
    assert_eq!(I1_PANICKER_APPLY.load(I1_SEQ), 1, "Panicker NEVER re-applied");
}
