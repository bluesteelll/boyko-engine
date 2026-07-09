> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14a — Architecture Critic, Round 1

**Verdict: NEEDS REWORK.** One critical soundness gap in Q3 (C1), one
critical aliasing-scope gap in Q1 (C2), one critical methodology gap in
the 0%-regression gate (C3), plus 5 important issues. The three
architect-flagged focus areas: Q1 is mostly sound but `get_component_mut`
is a user-violable footgun that should be tightened/removed for 14a; Q3
has a genuine double-drive + unproven nested-panic interaction; the Wave 1
0% gate passes vacuously and proves nothing.

All findings were verified against the actual source (apply loop,
migration helpers, `ComponentMask` repr, dispatch sites) — file:line at
the end.

---

## 🔴 Critical (blockers — implementation must not start)

### C1. Q3 drain is double-driven through `delete_entity`, and the nested-apply panic contract is unproven

**Where:** §5.4, §4.5; real `delete_entity` (ecs_master.rs:886-926) +
`DespawnCommand::apply` (despawn_command.rs:32-43).

`DespawnCommand::apply` is a thin wrapper calling
`world.delete_entity(self.entity)`. With the plan's two drive points
(direct method drains; every command's apply body drains), despawn
double-drives:

```
CommandQueue::apply (per-system queue; holds raw into self.bytes + world_ptr)
  └─ consume_and_drop_glue → DespawnCommand::apply(world)
       └─ delete_entity(world)            ← fires on_remove, enqueues into deferred_hook_queue
            └─ drain_deferred_hook_queue() ← drive (a): NESTED apply on world.deferred_hook_queue
       └─ (DespawnCommand::apply end)      ← drive (b): drains AGAIN (empty early-out)
```

The redundant second drain is harmless only via the empty-bytes
early-out — but it reveals the design has not decided **who owns the
drain**. Worse: `delete_entity` is reachable both from the **direct API**
(no command queue on the stack) and from `DespawnCommand::apply` (a full
`CommandQueue::apply` + `catch_unwind` + `CursorSync` already live). The
plan asserts the nested drain is sound "because it is a fresh apply on a
different queue" (§5.3) but never proves the **outer-`catch_unwind`
interaction**: if a deferred command panics during the nested drain
inside `delete_entity`, the panic unwinds through `delete_entity` →
`DespawnCommand::apply` → caught by the **outer** per-system queue's
`catch_unwind` (command_queue.rs:244), which runs
`handle_panic_recovery(0)` on the **per-system** queue. But the **nested**
`deferred_hook_queue` already ran its own `handle_panic_recovery` during
its unwind. This two-level panic-recovery interaction never existed in
Phase 12.5/12.6 (one level only). A deferred command's panic can leave
`deferred_hook_queue.panic_recovery` populated (the nested
`handle_panic_recovery` re-absorbs into `bytes` when `start == 0`, which
it is) → on the **next frame** the deferred queue replays survivors that
were enqueued by a hook whose triggering entity may no longer exist.
**Double-apply / apply-against-stale-entity is reachable.**

**Fix:** Pick **one** drain owner and prove the panic interaction.
Strongly consider the flecs-style alternative the architect listed but
did not pursue: a **depth counter + a single drain at the outermost apply
boundary** (e.g. in `Schedule::apply_window_drain` after each
`system.apply(world)`, and once at the end of each direct-API public
method). That collapses nesting to one level, eliminates the
double-drive, and makes the panic story identical to today's
single-level `catch_unwind`. The "drain inside every command's apply
body" option must be dropped or proven.

### C2. Q1 `get_component_mut` returns `&mut` into archetype storage with only a *documented* obligation — and the migration sites hold `&mut Archetype` across the hook fire point

**Where:** §3.1, §9 SAFETY-1(c); real apply sites: `SpawnAtCommand::apply`
(spawn_at_command.rs:178-247), `apply_replace_in_place`
(insert_command.rs:138-168), `migrate_entity_remove`
(migration_helpers.rs:438-519).

The walk: at the **spawn** site the `&mut Archetype` reborrow ends (NLL)
before §4.4 fires hooks after line 247 — argument (b) holds there.
**But** `get_component_mut::<Sibling>(e)` resolves a fresh `*mut` via
`get_component_raw_mut` (ecs_master.rs:986-1021), reborrowing
`&mut *inland.archetype_ptr()` (line 1007) → `&mut Sibling` into a pool
buffer. At the **migration sites**, the hook (fired pre-line-505 per
§4.3) runs while `source: &mut Archetype` (migration_helpers.rs:438) and
`target: &mut Archetype` (line 439) are **still live** (used at 505/508
*after* the fire point). The plan's §4.5 drops the borrow only for the
**despawn** site, not for the migration sites. Minting
`NonNull<EcsMaster>` from `&mut *world` while `source`/`target` (derived
from `world.archetype_master_mut()`) are live, then re-deriving `&mut`
through the view, is a **stacked/tree-borrows violation** against those
live reborrows.

The plan's §9 SAFETY-1(b) "drop the `&mut Archetype` reborrow before
firing hooks" is stated as a universal invariant but is only realizable
at 2 of the 6 sites as sketched. And `get_component_mut` is a `pub` API
whose non-aliasing requirement is **documented, not statically
enforced** — a safe-looking `pub fn` that a user can call to produce
UB is a soundness hole, not a documented `unsafe` contract (principle #8).

**Fix:** (1) For the migration sites, restructure so every `&mut
Archetype` derived from `world` is provably dead before
`NonNull::from(&mut *world)` is minted and any hook fires — i.e. "read
dying bytes into owned storage → drop all archetype borrows → fire hooks
→ re-resolve borrows → drop_at/move_out". This is a non-trivial rewrite
of `migrate_entity_remove`, not a drop-in. (2) **Recommended for 14a:**
drop `get_component_mut` from the view. Expose **read-only
`get_component` + `resource`/`resource_mut` + deferred `commands()`**
only. This still covers the canonical patterns (on_remove: decrement a
resource counter via `resource_mut`; read the dying value via
`get_component`). Defer mutable **component** access to 14b behind a
proven non-aliasing story. Bevy's `DeferredWorld` permits component
mutation only because its `World`/`DeferredWorld` split means the
structural op does not hold a `&mut` into the same storage when hooks
fire; boyko's migration sites *do*.

### C3. The Wave 1 "0%-regression" gate measures a dead-code-eliminated empty block

**Where:** §7 Wave 1 Step 3-4, §1 bar, §13.3.

Wave 1 wires `if !archetype.flags.is_empty() { /* empty */ }` while
`flags` is *always* `empty()` until Wave 2 Step 7. An `if` with an empty
body guarding nothing is a textbook DCE target — LLVM deletes it. So
Wave 1's "0%" is guaranteed by construction and proves nothing about the
Wave 4 gate, where the branch guards a real `#[cold]` call and `flags` is
a genuinely runtime-variable `u16` (OR-computed from a cold table) the
optimizer cannot prove zero. The plan elevates the Wave 1 bench to a
go/no-go gate (§10) — a vacuous pass gives false confidence and lets a
real regression slip to Wave 6.

**Fix:** Move the load-bearing bench to where dispatch is real. Either
(a) in Wave 1, populate `flags` non-zero via a test-only path and make
the trigger fn a non-eliminable `#[inline(never)]` no-op (measures an
honest load+branch+cold-call-not-taken); or (b) reorder so dispatch
wiring + bench gate land together in Wave 4, with Wave 1 reduced to
"field added, layout assertions pass". Verify the not-taken arrangement
via `cargo asm` on a real Wave-4 site, not a Wave-1 empty block.

---

## 🟡 Important

### W1. The despawn-site `to_vec()` is a per-despawn heap allocation on a path that is NOT always cold

**Where:** §4.5, §12. The `archetype.component_ids().to_vec()` is "cold"
per-archetype but fires on **every despawn of every entity** in any
archetype that has *any* remove/despawn hook — i.e. the intended steady
state for any hooked archetype (the canonical "on_remove counter" pattern
the plan uses to justify Q1!). One alloc+free per despawned entity
violates principles #1/#5 on a path the user *will* exercise heavily.
**Fix:** stack fixed buffer (the codebase already uses
`[ComponentId; MAX_MIGRATION_COLUMNS]` on the stack at
migration_helpers.rs:69) or iterate `component_ids()` directly under a
short-lived shared borrow dropped before the `&mut` re-resolve (the same
dance C2 requires — solve together). Resolve in the plan, not a
follow-up.

### W2. The "`flags: u16` lands in existing padding, zero size growth" claim is most likely FALSE

**Where:** §3.3, Q4. `signature: ArchetypeSignature` has align **32**
(its `ComponentMask` is `#[repr(align(32))]`, component_mask.rs:7), so its
size is a multiple of 32 and the offset after it is already 8-aligned →
`arena: *const Arena` today starts immediately with **zero padding**.
Inserting `flags: u16` before `arena` adds 2 B + 6 B realign = **+8
bytes**, not zero. Harmless functionally (Archetype is ~8 KB) but the
stated justification is wrong and invites a bad "optimization" later.
**Fix:** correct the justification (or move `flags` to genuine padding if
any exists near `id`); add a **hard** `const _: () =
assert!(size_of::<Archetype>() == <measured>)` tripwire. Also: Wave 2
Step 5 says "re-assert ComponentLayout is still 56 B" — there is **no**
such assertion in source today (component_registry.rs:90 is doc-only); it
must be **added**, not re-asserted.

### W3. `ArchetypeFlags` staleness is a silent-skip footgun for hand-impl `Component` + the Phase 8.5 slab cache

**Where:** §6.4, §4.6. The "derive path is staleness-immune" claim does
not extend to **hand-written `impl Component`** (which the test suite +
any foreign type use — ecs_master.rs:2129-2137 hand-impls `component_id()`
with no hook install). A hand-impl component + runtime
`register_component_hooks` *after* an archetype exists silently skips. The
debug-assert catches the runtime-builder case in debug only. A silently
dropped lifecycle callback is a severe correctness surprise for a feature
whose entire value is "the callback fires." **Fix:** make the staleness
check a **release** check at `register_component_hooks` (the scan is cold,
one-time), OR enforce a world-level "frozen after first spawn" flag, OR
provide + require a manual install shim for hand-impls. Pick one; don't
leave "silently skipped" as an accepted release outcome.

### W4. ALLOC1 (`IN_SYSTEM_RUN`) interaction is unstated but load-bearing

**Where:** §9 (missing). A hook's deferred command that triggers an arena
allocation (spawn into a new archetype) would trip ALLOC1's
`debug_assert!(!IN_SYSTEM_RUN)` **if** the flag were set during apply.
Verified it is **not**: `InSystemRunGuard` wraps only `run_unsafe`
(schedule.rs:605-618), dropped before `system.apply` runs (schedule.rs:339,
516). So hooks *can* allocate — a positive, but unstated. If a future
refactor moved `apply` inside the guard, hooks break. **Fix:** add a
SAFETY invariant documenting hooks execute with `IN_SYSTEM_RUN == false`
+ a `debug_assert!(!is_in_system_run())` tripwire at the top of
`drain_deferred_hook_queue`.

### W5. The `mem::take` + restore drain changes panic semantics vs the claimed `CommandQueue::Drop` mirror

**Where:** §5.4, §9 SAFETY-5. `CommandQueue::Drop` (command_queue.rs:711)
takes the *recovery* buffer in `Drop` (no restore). The plan's proposal
takes the *bytes* into a **local** `CommandQueue`, applies, restores
capacity. If that local `apply` panics, the local is dropped during
unwind — its `Drop` runs drop-glue on survivors, which are then **gone**
(dropped, not re-absorbed into `world.deferred_hook_queue`), while the
field was left empty by `mem::take`. A panic during the nested drain
**silently discards** surviving deferred commands. **Fix:** collapses into
C1 — if the drain owner is the outermost boundary running
`deferred_hook_queue.apply(world)` via the proven `RawCommandQueue`
raw-twin (command_queue.rs:155-173), the take-restore dance disappears.
Specify the raw-twin approach; prove no field aliasing.

---

## 🟢 Optional

- **O1.** Add a one-line `ComponentHooks: Send + Sync` note for the new
  `static HOOKS` table (matches the rigor every other `static` gets).
- **O2.** The five near-identical `trigger_on_*` fns should be a
  declarative macro (Phase 10 precedent); verify `#[cold]` actually
  demotes via `cargo asm`.
- **O3.** The Q7 on_replace+on_insert *placement* inside the live-borrow
  `for_each_component_bytes` closure (insert_command.rs:138) is subject to
  the same C2 aliasing rework — address under C2, not as independently
  settled.

---

## Positive (preserve these)

- **Q5 parallel cold `HOOKS` table** — faithfully mirrors the verified
  `LAYOUTS` precedent; keeps `ComponentLayout` untouched. Right call.
- **Q3's rejection of Bevy front-jump** to avoid touching the Miri-clean
  `while local_cursor < stop_snapshot` + `CursorSync` + `catch_unwind`
  machinery is correct judgment. The *next-pass* instinct is right; only
  the *drive-point ownership* (C1) needs rework.
- **Deferring 14b Observers + not coupling to `EventDispatcher`** —
  research §6 is correct.
- **`#[cold] #[inline(never)]` trigger fns + per-archetype bit-test gate
  concept** — right I-cache discipline, consistent with existing
  `migrate_entity_*` / `handle_panic_recovery`.
- **PRE-DROP firing for remove/despawn** — correctly placed against the
  real `drop_at` (migration_helpers.rs:505) and `remove_entity`
  (ecs_master.rs:906).
- **Drop-order of `deferred_hook_queue` after `arena`** — correctly
  mirrors the `query_state_cache` C5 placement; sound reasoning.

---

## Open questions for the architect (Round 2)

1. **Drain ownership (C1):** outermost apply boundary (one place), or
   every command + every direct method? Commit to one; the plan's "both"
   double-drives despawn.
2. **Does `get_component_mut` survive 14a (C2)?** Given the migration
   sites hold `&mut Archetype` across the fire point, is the
   drop-all-borrows-before-firing rewrite in scope for 14a, or is
   read-only `get_component` + `resource_mut` + deferred-commands the
   correct conservative 14a surface (mutable component access → 14b)?
3. **Re-entrant deferred drain (C1/§5.2#3):** with the raw-twin approach
   the outer `while !deferred_hook_queue.is_empty()` sees newly-appended
   bytes; with `mem::take`-into-local it does NOT. Settle with W5.
4. **Staleness in release (W3):** acceptable silent-skip, or panic in
   release when a live archetype already hosts the component?
5. **What does a `migrate_entity_remove` hook observe?** At the
   pre-`drop_at` fire point the entity's `EntityInland` still points at
   the **source** archetype (repoint at 518-519). State explicitly that
   the hook reads the *source* (dying) value so the test asserts against
   the source row.

---

## Files verified during this review

- `command_queue.rs` (apply loop 203-251, `RawCommandQueue` raw-twin
  155-173, `CursorSync` 291-319, panic recovery 526-567, Drop 693-721)
- `command.rs` (`consume_and_drop_glue` 127-168, W3' cursor 149)
- `spawn_at_command.rs` (apply 106-248; `&mut Archetype` 178)
- `migration_helpers.rs` (`migrate_entity_remove` 412-520, `drop_at` 505;
  `migrate_entity_insert` 151-404 — both hold `source`/`target` `&mut`
  across the body)
- `insert_command.rs` (`apply_replace_in_place` 97-173; closure reborrow 138)
- `despawn_command.rs` (apply → `delete_entity` 32-43)
- `ecs_master.rs` (layout/drop-order 116-252, `delete_entity` 886-926,
  `get_component_raw_mut` 986-1021, SEND1 2063-2093)
- `archetype.rs` (layout 109-170, offset-0 assert 170, `create_by_ids`
  203-232, `register_component_inplace` 281-284)
- `archetype_signature.rs` / `component_mask.rs` (`#[repr(align(32))]`
  line 7 — load-bearing for W2)
- `component_registry.rs` (`LAYOUTS` 141-188, `ComponentLayout` 56 B
  doc-only 90)
- `component.rs` (trait — widening target)
- `schedule.rs` (`apply_window_drain` 315-339, `system.apply` 339/516,
  `InSystemRunGuard` 605)
- `boyko_threadpool/src/tls.rs` (`IN_SYSTEM_RUN` 37, guard 138-166 —
  load-bearing for W4)
- `boyko_macros/src/lib.rs` (derive `component_id()` 53-80 — REG target)
