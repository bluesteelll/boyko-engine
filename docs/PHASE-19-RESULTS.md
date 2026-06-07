# Phase 19 — Hierarchies / Parent-Child (CORE) — Results

Branch `ecs`. Status: **LANDED + verified.** Full pipeline: research → architect (R1+R2) → critic (R1)
→ developer (W1-W5) → code-review → tester → **BUG-P19-TB-1 discovered** → architect → critic → developer
→ tester (Miri-TB verified). Design of record: `docs/PHASE-19-PLAN.md` (+ R2 addendum) /
`docs/PHASE-19-RESEARCH.md`. Bug fix of record: `docs/BUG-P19-TB-1-PLAN.md`.

## What shipped
The Bevy-0.16 relationship model on the Phase 14a/14b hook substrate:
- **`ChildOf(Entity)`** — `#[repr(transparent)]` foreign key on the **child** (source of truth). Inserting
  links, overwriting reparents, removing unlinks.
- **`Children(Vec<Entity>)`** — `#[repr(transparent)]` reverse collection on the **parent**, maintained
  reactively by `ChildOf`'s component hooks. Read-only to users (`as_slice`/`len`/`is_empty`/`contains`).
- **Hook wiring:** `ChildOf` registers `on_insert` (LINK) + `on_replace` (UNLINK); `Children` registers
  `on_replace` (the recursive-despawn CASCADE). All hooks follow the OBS-FIRE-LOOP / F2 discipline (no
  `world`-derived `&` held across a `commands()` mint).
- **`EntityCommands`/`Commands` ergonomics:** `add_child`, `add_children`, `set_parent`, `remove_parent`,
  `remove_children`, `clear_children`, `despawn_without_children`; `Commands::add_child(parent, child)`.
  The whole relationship is driven by `ChildOf` insert/remove — user code never writes `Children`.
- **Default-recursive despawn** via `Children::on_replace` firing in `delete_entity` (the #20106-correct
  pre-remove order, reading the CURRENT collection); opt-out `despawn_without_children` via a
  `CASCADE_SUPPRESS` thread-local scoped to the fire (not the drain).

New module: `crates/boyko_ecs/src/ecs/core/hierarchy/{mod.rs, commands.rs, bundles.rs}`.

## Key design decisions (all critic-vetted)
- **C1 — Bundle-newtype insert primitive.** `Bundle` is sealed and all insert machinery is `B: Bundle`-bound,
  so a bare `Children` can't reuse it. Private 1-field `ChildrenBundle(Children)` / `ChildOfBundle(ChildOf)`
  route the first-child insert through the audited `migrate_entity_insert` verbatim → **zero new structural
  unsafe**. Because `boyko-macros` is a dev-dependency (kept out of the library build graph by the
  minimal-dependency principle), `#[derive(Bundle)]` is unavailable in `src/`; the `Bundle` impl is
  hand-written to mirror the derive output exactly (the established internal hand-impl pattern).
- **C2 — hand-`component_id()` MUST trigger install.** `install_hooks::<C>` fires only from inside
  `component_id()`; both `ChildOf`/`Children` replicate the derive body (`register_new` →
  `if HAS_HOOKS { install_hooks }`). A C2 install-probe unit test (with negative asserts) guards the
  silent-no-op footgun.
- **M2 — the ONE new `unsafe`** in the feature: the `MaybeUninit` cascade buffer's `assume_init` on the
  inline fast path (`n <= CASCADE_FANOUT_INLINE = 32`). The wide path re-derives per turn (no buffer, no
  unsafe). The C1 None-arm raw-deref is a verbatim copy of the audited `insert_command.rs` pattern.
- **Keep-empty `Children`** (no remove-on-empty) — avoids archetype thrash on `0↔1↔0` child oscillation.
- **`swap_remove`** for unlink — no sibling-order contract (documented).

## BUG-P19-TB-1 — a pre-existing latent Tree-Borrows UB the cascade exposed
The tester's Miri-TB run found a real soundness bug — **not in hierarchy code**, but in the deferred
command-queue re-entrancy machinery (`command_queue.rs` `apply_via_raw_twin`). The `RawCommandQueue` twin
cached `NonNull`s into `world.deferred_hook_queue`; a re-entrant `push` during the drain (a Phase 19
cascade/guard hook enqueuing into the SAME queue) did `Vec::reserve`/`set_len` through a fresh
`&mut …deferred_hook_queue` — a TB foreign write that Disabled the twin's tag → the next reborrow was UB.
Phase 19's cascade is the **first workload** where the walked queue and the re-entrant-push target are the
same `CommandQueue` (the `apply` and `Drop` callers push to a *different* queue). The recurring boyko bug
class (cached pointer invalidated by a sibling `&mut` foreign write under TB — cf. Phase 9.3c, 14a F2).

**Fix (Approach C):** `apply_via_raw_twin` `mem::take`s the queue's buffers into a stack-local
`CommandQueue temp` and runs the audited `temp.apply(world)`. Because `temp` is a **separate allocation**,
re-entrant pushes (which target the now-empty home queue) cannot foreign-write `temp`'s twin — the bug
vanishes structurally and the audited walk is reused (no duplication). An outer `catch_unwind` re-homes
**both** survivors and re-entrant pushes as `[survivors][re-entrant]` on a mid-drain panic (critic P1 —
the draft swap-and-drop would have lost re-entrant deferred work). The critic rejected a Drop-guard design
(raw-`*mut`-to-local-written-in-Drop = an unproven TB pattern) in favor of the outer-catch (P2). Net: a
single-function change, signature unchanged, no other caller touched.

## Verification
- **Miri-TB (the oracle, `-Zmiri-tree-borrows -Zmiri-ignore-leaks`):** the canonical repro
  `miri_minimal_cascade_reentrant_push` and the inline cascade path pass; the new drain-panic test
  `miri_drain_panic_reentrant_disposition` (I1 — the only coverage of the fix's unwind disposition) passes;
  shared-drain suites `miri_phase8cd` (11/11) and `miri_phase14b` (10/10) remain clean. The wide-cascade
  path (`n > 32`) is Miri-ignored for tractability (a 34-entity cascade OOMs under Miri) but runs fully
  native; its TB surface is subsumed by the inline path (the wide branch's only delta is a *safe* per-turn
  re-derive). [logged, not a silent cap]
- **Behavioral:** 918 tests debug / 903 release (boyko-ecs); 983 workspace. Zero regressions vs the
  pre-Phase-19 baseline (+1 = the new drain-panic test; the +17 core + 1 property hierarchy tests + the
  trybuild re-bless landed earlier in the gate). Coverage: link/unlink, reparent both paths
  (fresh-migrate + overwrite-in-place), recursive despawn (depth ≥ 2 + wide fanout), despawn_without_children,
  self-ref + dangling guards, #20106 read-before-remove, reparent atomicity, re-entrancy single-drain,
  clear/remove_children, keep-empty, cascade-suppress scoping, and a 256-case property test of the
  bidirectional invariant `c.ChildOf==p ⟺ p.Children∋c`.
- **0%-gate (`phase14a_hooks_gate`):** flat. The hierarchy adds nothing to the hot iteration path; the
  fix is on the cold drain path which the gate bench never reaches (the no-hooks gate leaves the deferred
  queue empty). A hierarchy-free program never mints `ChildOf`/`Children` ids → the `ArchetypeFlags` gate
  raises no bit.
- **clippy `--all-targets -- -D warnings`:** green.

## Net unsafe accounting
Phase 19 feature: **exactly one** new `unsafe` (the M2 `assume_init`) + one verbatim copy of the audited
`insert_command.rs` raw-deref (C1 None-arm) + the hand-written `bundles.rs` `from_raw_parts`
(soundness-identical to every `#[derive(Bundle)]`; forced by the dev-only macro crate). BUG-P19-TB-1 fix:
no net new `unsafe` (the new blocks are transient `&mut` derivations + the audited `apply` reuse, each with
a SAFETY comment).

## Deferred (future work, out of CORE scope)
Transform propagation (`GlobalTransform = parent × local`), change-detection-gated partial propagation,
parallel two-phase tree walk, deep cycle detection (only self-ref is guarded — deep `ChildOf` cycles are a
documented footgun), `iter_descendants` (today: `Query<&Children>` + manual recursion), a generic
`Relationship`/`RelationshipTarget` trait (parent-child is the only concrete instance).
