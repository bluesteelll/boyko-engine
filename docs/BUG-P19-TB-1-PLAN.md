# BUG-P19-TB-1 — Fix Plan (FINAL — critic APPROVED-WITH-CHANGES folded)

**Branch** `ecs` · **File of record** `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` ·
**Oracle** Miri Tree Borrows (`-Zmiri-tree-borrows`; SB retired) · **Chosen approach** **C**
(mem::take buffers into a stack `CommandQueue`, reuse the audited `apply`).

> **CRITIC OUTCOME (APPROVED-WITH-CHANGES) — these supersede §3/§4 below:**
> - **P1 (confirmed):** on-panic disposition MUST preserve BOTH the survivors AND the home queue's
>   re-entrant pushes, re-homed as `[survivors][re-entrant]` (the exact order the current
>   `handle_panic_recovery` produces). Swap-and-drop loses re-entrant work — forbidden.
> - **P2:** REMOVE `DrainSurvivorGuard` (its raw-`*mut`-to-local-written-in-Drop is an unproven TB
>   pattern). Use an **outer `std::panic::catch_unwind`** around `temp.apply`; on `Err`, access the
>   owned `temp` by safe `&mut` + re-derive the home queue from `world`, do the P1 re-home, `resume_unwind`.
> - **I1:** the new drain-panic test (§7) MUST run under Miri-TB (it is the ONLY coverage of the unwind
>   disposition; the cascade Miri repros never panic).
> - Confirmed sound: Q3 disjoint-allocation crux (per-command fresh `&mut` in `consume_and_drop_glue`
>   ⇒ no `&`-protector spans the re-entrant push), Q2 no visibility widening, debug_asserts, outer-loop
>   equivalence/termination, W3 capacity, HOOK_DRAIN_DEPTH, cursor invariant, no hot-path alloc.
> - M1 (doc-only): the `phase14a_hooks_gate` bench measures the hook GATE, not the drain path — state
>   the perf claim as "drain is cold; Approach C adds O(1) allocation-free moves; the hot gate path is
>   byte-unchanged."
>
> **FINAL `apply_via_raw_twin` body** (replaces §4 skeleton; the `DrainSurvivorGuard` of §3 is deleted):
> ```rust
> let mut temp = CommandQueue::new();
> // SAFETY: transient &mut to the home queue for the two takes; dropped before temp.apply.
> //   World exclusively borrowed (fn contract); deferred queue at rest (cursor == 0) at a drain turn.
> unsafe {
>     let home = &mut (*world.as_ptr()).deferred_hook_queue;
>     debug_assert!(home.cursor == 0, "invariant: deferred queue at rest has cursor == 0");
>     temp.bytes = mem::take(&mut home.bytes);
>     temp.panic_recovery = mem::take(&mut home.panic_recovery);
> }
> // temp ∉ world ⇒ disjoint; re-entrant pushes hit the (empty) home queue, not temp's twin (Q3).
> // SAFETY: world exclusively borrowed; temp and *world are disjoint allocations.
> let world_ref: &mut EcsMaster = unsafe { &mut *world.as_ptr() };
> let result = std::panic::catch_unwind(AssertUnwindSafe(|| { temp.apply(world_ref); }));
> match result {
>     Ok(()) => unsafe {
>         // W3 capacity reuse: return temp's capacious drained buffer iff no re-entrant growth survived.
>         // SAFETY: exclusive world borrow; no command apply runs here.
>         let home = &mut (*world.as_ptr()).deferred_hook_queue;
>         if home.bytes.is_empty() { mem::swap(&mut home.bytes, &mut temp.bytes); }
>         debug_assert!(temp.cursor == 0 && home.cursor == 0, "invariant: cursors quiescent");
>         debug_assert!(temp.panic_recovery.is_empty() && home.panic_recovery.is_empty(),
>             "invariant: panic_recovery empty at rest on success");
>     },
>     Err(payload) => {
>         // P1 preserve-both: temp.apply's handle_panic_recovery(0) re-absorbed SURVIVORS into temp.bytes
>         // (+ drained temp.panic_recovery); the home queue holds RE-ENTRANT pushes from pre-panic commands.
>         // Re-home as [survivors][re-entrant] so a later drain APPLIES both.
>         // SAFETY: temp.apply fully unwound (no live borrow of temp); exclusive world borrow; no apply here.
>         unsafe {
>             debug_assert!(temp.panic_recovery.is_empty() && temp.cursor == 0,
>                 "invariant: temp recovery drained + cursor reset by handle_panic_recovery(0)");
>             let home = &mut (*world.as_ptr()).deferred_hook_queue;
>             temp.bytes.append(&mut home.bytes);          // temp = [survivors][re-entrant]; home empty
>             mem::swap(&mut home.bytes, &mut temp.bytes);  // home = [survivors][re-entrant]
>         }
>         std::panic::resume_unwind(payload);
>     }
> }
> // temp drops: bytes empty (swapped/moved), panic_recovery empty ⇒ no-op walk.
> ```

## 1. Verified bug
`RawCommandQueue` (`command_queue.rs:332`) caches `bytes`/`cursor`/`panic_recovery` `NonNull`s minted
ONCE in `raw()` (`:168-186`) via `&raw mut self.<field>`. `apply_via_raw_twin` (`:291`) mints the twin
from `(*world).deferred_hook_queue.raw()`. DURING the walk, `consume_and_drop_glue` (`:522`) runs a
command's `apply`, which (Phase 19 cascade despawn / self-ref+dangling reactive `remove::<ChildOf>`)
re-enters and pushes into the SAME `world.deferred_hook_queue`. `push` (`:128`) does
`self.bytes.reserve/set_len` through a fresh `&mut (*world).deferred_hook_queue` — a TB **foreign write**
that Disables the twin's cached `bytes` tag. The next `self.bytes.as_mut()` (`:477` next-turn / `:557`
compaction) reborrows the Disabled tag → **UB**. Miri chain: Reserved@`:168` → Disabled@`:135` →
UB@`:476`. All three field pointers are at risk (`push(&mut self)` retags the whole struct;
`cursor`/`panic_recovery` written at `:575`/`:383`/`:597-598` after re-entrant pushes; Miri tripped on
`bytes` first). Native passes by luck (re-derived read hits still-valid memory).

### Scope — isolated to `apply_via_raw_twin`
| Caller | Walked queue | Re-entrant push target | Same alloc? | Bug? |
|---|---|---|---|---|
| `apply_via_raw_twin` (`:291`) | `world.deferred_hook_queue` | `world.deferred_hook_queue` | YES | **YES** |
| `apply(&mut self, world)` (`:216`) | per-system `Commands` queue | `world.deferred_hook_queue` (different obj) | NO | NO |
| `Drop` (`:760`) | `self` | none (`world=None`) | n/a | NO |

## 2. Chosen approach C
In `apply_via_raw_twin`, `mem::take` the queue's `bytes` + `panic_recovery` into a STACK-LOCAL
`CommandQueue temp`, then call the existing audited `temp.apply(world)`.
- `temp` is a SEPARATE allocation from `world.deferred_hook_queue` (its Vec buffer is the moved-out old
  buffer; its Vec header is on the stack). Re-entrant pushes target the now-empty home queue (a fresh
  Vec::new + a fresh reallocation) → they CANNOT foreign-write `temp`'s twin (all three fields). Bug gone.
- The audited `apply` (single `catch_unwind` + `handle_panic_recovery(0)` + compaction + `CursorSync`) is
  reused VERBATIM — no new walk, no duplicated panic-recovery logic.
- The outer `drain_deferred_hook_queue` `while !is_empty()` loop (`ecs_master.rs:2490`) picks up the
  re-entrant pushes the next outer turn (a fresh `temp`). Turn count/order/termination unchanged.
- Also dissolves the original W5 field-alias hazard (doc `:271-276`): `temp ∉ world`, so `&mut temp` and
  `&mut *world` are disjoint and the safe `&mut self` `apply` is directly callable.

### Signature UNCHANGED
`pub(crate) unsafe fn apply_via_raw_twin(world: NonNull<EcsMaster>)` — sole caller `ecs_master.rs:2503`
untouched. Unchanged functions: `apply`, `apply_or_drop_queued_no_catch`, `handle_panic_recovery`,
`CursorSync`, `Drop`, `raw`, `push`.

## 3. New `DrainSurvivorGuard` (near `CursorSync`, ~`:336`)
Fields: `world: NonNull<EcsMaster>`, `temp: *mut CommandQueue`, `armed: bool`. `!Send`/`!Sync` by raw-ptr
auto-trait (no explicit impls → Send/Sync surface unchanged). Declared AFTER `temp` ⇒ drops BEFORE `temp`
(reverse-declaration order; `temp` still alive in the guard's `Drop`, mirrors `CursorSync`). `disarm()`
sets `armed=false`; success path disarms. `Drop` (`#[cold]`, unwind-only via `if !armed { return }`)
re-homes survivors so a later drain APPLIES them (NOT `temp`'s `world=None` `Drop`, which would silently
DROP deferred work — invariant 2).

> **OPEN — CRITIC Q1 (orchestrator flags as likely-wrong):** the architect's draft guard `Drop`
> `mem::swap`s `home.bytes ↔ temp.bytes`, which sends `temp`'s re-absorbed survivors home but moves the
> home queue's *re-entrant pushes made by already-run commands* INTO `temp`, whose `Drop` then DROPS them
> (`world=None` drop-glue). The CURRENT buggy code PRESERVES those re-entrant pushes (they sit
> contiguously past `stop_snapshot` in the same Vec and are captured by `handle_panic_recovery`'s
> `[cursor..current_stop]`). So swap-and-drop is a behavior change AND loses legitimately-enqueued
> deferred work (e.g. a `LinkChildCommand` enqueued by a hook that ran before the panic) → an
> inconsistent `Children`. **Likely resolution:** the guard must PRESERVE BOTH — append `temp`'s
> survivors to the home queue's re-entrant pushes (or vice-versa) rather than swap-and-drop. The critic
> must decide the exact disposition + ordering (match current `[survivors][re-entrant]`, or stricter
> FIFO) and whether `panic_recovery` needs the same treatment.

## 4. Rewritten `apply_via_raw_twin` (skeleton; exact SAFETY texts inline)
```
let mut temp = CommandQueue::new();
// SAFETY: transient &mut to the home queue for the two takes; dropped before temp.apply.
//   World exclusively borrowed (fn contract).
unsafe {
    let home = &mut (*world.as_ptr()).deferred_hook_queue;
    debug_assert!(home.cursor == 0, "deferred queue at rest has cursor==0");
    temp.bytes = core::mem::take(&mut home.bytes);
    temp.panic_recovery = core::mem::take(&mut home.panic_recovery);
}
let mut guard = DrainSurvivorGuard { world, temp: &raw mut temp, armed: true };
// temp ∉ world ⇒ disjoint; call the audited safe apply.
// SAFETY: world exclusively borrowed; re-entrant push targets the home queue
//   (different allocation from temp) — TB-safe.
let world_ref: &mut EcsMaster = unsafe { &mut *world.as_ptr() };
temp.apply(world_ref);
guard.disarm();
drop(guard);
// W3 capacity reuse: return temp's capacious drained buffer home iff no re-entrant
// growth survived (common case).
// SAFETY: exclusive world borrow; no command apply runs here.
unsafe {
    let home = &mut (*world.as_ptr()).deferred_hook_queue;
    if home.bytes.is_empty() { core::mem::swap(&mut home.bytes, &mut temp.bytes); }
    debug_assert!(temp.cursor == 0 && home.cursor == 0, "cursors quiescent");
    debug_assert!(temp.panic_recovery.is_empty() && home.panic_recovery.is_empty(),
        "panic_recovery empty at rest on success");
}
// temp drops: bytes empty (swapped or drained), panic_recovery empty ⇒ no-op walk.
```
Field access: `apply_via_raw_twin` is in `impl CommandQueue` (names `temp.{bytes,cursor,panic_recovery}`,
as `Drop` does at `:778`); `deferred_hook_queue` is `pub(crate)` (`ecs_master.rs:252`) with `pub(crate)`
fields (`:64-66`) — **no visibility widening** (Q2).

## 5. TB tag-flow (crux)
`W` = caller's `world` tag; `T` = stack `temp`'s tag. `temp.apply` mints twin `T_bytes/T_cursor/T_recovery`
as children of `T`, pointing into `temp` (stack header + the moved-out heap buffer). A re-entrant
`world.deferred_hook_queue.push(...)` writes the home allocation in the `W`-tree — a DIFFERENT allocation
from `T`'s pointees → NOT a foreign access to any `T_*` tag. They stay live across `:477`/`:557`. Exact
inversion of the bug. Disjoint-allocation is the load-bearing property (independent of push parentage).

## 6. Invariants preserved
1. CursorSync — UNCHANGED (inside `temp.apply`). 2. Panic-recovery — `temp.apply` re-absorbs into
`temp.bytes`; guard re-homes to `world.deferred_hook_queue` on unwind (see Q1). 3. Compaction case B —
UNCHANGED (inside `temp.apply`). 4. `apply`/`Drop` byte-unchanged. 5. Native correctness identical; cold
drain path; W3 capacity retained via success swap-back; no steady-state-iteration alloc. 6. CLAUDE.md:
SAFETY on all unsafe; no forbidden types; Send/Sync surface unchanged; English.

- `temp.apply` debug_assert `:224` (`panic_recovery non-empty ⇒ bytes non-empty`): holds — `temp.bytes`
  non-empty (drain loop calls only when `!is_empty()`), `temp.panic_recovery` empty (taken from at-rest home).
- Outer-loop turn count/order/termination: unchanged (re-entrant pushes picked up next turn, as the
  compaction did before).

## 7. Validation
Un-ignore (in order) in `tests/miri_phase19.rs`: `miri_minimal_cascade_reentrant_push` (FIRST),
`miri_cascade_inline_path`, `miri_cascade_wide_path`, `miri_recursive_despawn_three_levels`,
`miri_self_ref_and_dangling_guards`. Target 9/9 TB-clean.
```
cargo build --release -p boyko-ecs
cargo clippy --all-targets -- -D warnings
$env:MIRIFLAGS="-Zmiri-tree-borrows"
cargo +nightly miri test -p boyko-ecs --test miri_phase19 miri_minimal_cascade_reentrant_push
cargo +nightly miri test -p boyko-ecs --test miri_phase19            # 9/9
cargo test -p boyko-ecs --test command_queue_panic_recovery
cargo test -p boyko-ecs --lib commands::command_queue
cargo +nightly miri test -p boyko-ecs --test miri_phase8cd
cargo test -p boyko-ecs                                              # full suite
cargo bench --bench phase14a_hooks_gate                             # 0%-hot-path
```
**NEW mandatory test (gap):** no existing test panics a command DURING a drain (`apply_via_raw_twin`).
Add a native test: a deferred command whose `apply` panics during `drain_deferred_hook_queue`, asserting
(a) panic propagates, (b) survivors + (per Q1) the re-entrant pushes are re-homed and applied on a later
drain (not lost), (c) the panicker does not re-run. This is the only coverage of `DrainSurvivorGuard`'s
unwind path (the Miri repros don't panic).

## 8. Rejected alternatives
- **A (per-turn re-derive / dedicated drain walk):** TB-sound but duplicates ~80 lines of
  panic-recovery+CursorSync+compaction (divergence risk) OR becomes B. C reaches disjointness
  structurally with zero duplication.
- **B (refactor shared walk behind a re-derivable accessor):** touches the non-buggy `apply`/`Drop`
  (wrong blast radius for a one-caller bug) + per-access cost on the system-queue path for no benefit.
- **4th (route re-entrant pushes to a 2nd queue):** doesn't generalize; changes hook routing. C's stack
  `temp` is a per-turn auto-recycled "queue B" with no routing change.

## 9. Critic questions (architect's, + orchestrator's Q1 emphasis)
1. **Q1 (PRIORITY — see §3 box):** on-panic disposition — swap-and-drop LOSES re-entrant pushes vs the
   current preserve-both; mandate the correct disposition + ordering.
2. Q2 field visibility — confirm no widening needed.
3. Q3 TB parentage — confirm disjoint-allocation is load-bearing and `W` isn't protected across
   `temp.apply` in a way a write to its child home allocation trips.
4. Q4/Q6 — termination unchanged; whole-struct `mem::swap` vs field-wise at entry/exit (style).
5. Q5 — confirm no existing drain-panic test ⇒ §7 new test mandatory.
