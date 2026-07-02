# Phase 9.3c — Results: completion-channel Tree-Borrows hardening

Branch `ecs`. Code fix committed locally as **`09a1204`** (author Celtokisa, no `Co-Authored-By`,
NOT pushed). Classification commit: `250cd3f`. Architect→critic plan converged on **Direction A**.

## Status: COMPLETE — the executor completion push is Tree-Borrows-clean

The integrated 2-worker executor test (`crates/boyko_ecs/tests/miri_schedule_parallel.rs`),
`#[ignore]`d since Phase 9.3a, is **re-enabled and passes** under `-Zmiri-tree-borrows`
(`1 passed; 0 failed`). The `write access forbidden` is gone. This was the last OPEN item from
Phase 9.3.

### The bug
`completion_queue: ArrayQueue<SystemIndex>` and `pending_apply: CachePadded<AtomicUsize>` were
**inline fields** of `Schedule.executor_scratch` (`ExecutorScratch`), so their bytes lived inside
the `Schedule` allocation. The dispatcher runs the executor through `&mut self`
(`executor_main_loop` → `try_dispatch_ready` → `apply_window_drain`); under Tree Borrows a
`&mut self` at function entry is a **protected** tag (FnEntry protector) spanning the whole call,
including the park/join window where workers are live. Workers foreign-write those exact bytes
(`ArrayQueue::push` → `UnsafeCell::get().write`, then `pending.fetch_add(Release)`) while the
protector is live → TB "write access forbidden" inside `crossbeam-queue ArrayQueue::push`.

**Why it was the lone exception:** the *world* is already clean because `UnsafeEcsCell` derives
from a SEPARATE `&'scope mut EcsMaster` param (a different allocation); `systems_ptr` is clean
because `Vec::as_mut_ptr` points into the Vec's SEPARATE heap buffer. `completion_queue` +
`pending_apply` were the only cross-thread state living **inline** in `self`.

### The fix — Direction A (own as a bare `NonNull`, mirror Phase 9.2 exactly)
Relocate the two fields into their own heap allocation so the `&mut self` protector covers only an
8-byte pointer, never the channel bytes — the same property `systems` (separate Vec buffer) and
`world` (separate `EcsMaster`) already enjoy, and the same mechanism the Miri-green Phase 9.2
`NonNull<ScopeShared>` relies on.

- New `struct CompletionChannel { queue: ArrayQueue<SystemIndex>, pending: CachePadded<AtomicUsize> }`.
- `ExecutorScratch` owns it as a **bare `completion: NonNull<CompletionChannel>`** — constructed
  via `Box::into_raw` in `new`, freed via `Box::from_raw` in a new `Drop for ExecutorScratch`
  (single free site, runs at `Schedule` drop after every `Schedule::run` has joined its workers).
- New `Copy` read-only `CompletionCell<'a> { ptr: NonNull<CompletionChannel>, _marker }` with
  **by-value** accessors (`channel(self) → &'a CompletionChannel` via `NonNull::as_ptr`, plus
  `push`/`pop`/`queue_is_empty`/`pending_load`/`pending_fetch_add`/`pending_fetch_sub`), and
  `unsafe impl Send + Sync`. Mirrors `UnsafeEcsCell`'s by-value-receiver C1 primitive (no `&self`
  retag) and `scope.rs`'s `NonNull::as_ptr` non-retag.
- The dispatcher mints the cell ONCE per frame in `executor_main_loop`
  (`CompletionCell::new(self.executor_scratch.completion)` — a **Copy read of the `NonNull`**, no
  lasting borrow of `self`), threads it (Copy) to `apply_window_drain` and `try_dispatch_ready`,
  and into `SpawnPointers<'a> { systems, completion }`. **Every** completion access — dispatcher
  pop/load/fetch_sub, worker push/fetch_add, and the SCH6 reset/run-end debug-asserts — goes
  through the cell's / `NonNull::as_ref`'s non-retagging `as_ptr` lineage. None reaches the pointee
  through `self.executor_scratch.completion` via `&mut self`.

### Why Direction A, not the architect's first cut (the critical catch)
The architect's initial plan kept a live `Box<CompletionChannel>` field and minted
`NonNull::from(&*self.completion)`. The **architecture-critic flagged this CRITICAL**: a `Box<T>`
place asserts `Unique`/noalias on its pointee whenever it is named through `&mut self`, so a live
`Box` field re-pollutes the heap allocation's tag tree every time `self.completion` is named
(the mint AND `reset_for_frame`) — a *temporal* safety argument ("reset runs between frames"),
not the *structural* one Phase 9.2 has. `scope.rs` works precisely because `Box::into_raw`
**consumes** the Box so no `Box` place exists during concurrency. Direction A reproduces that:
a bare `NonNull` has no special TB aliasing semantics, so the only lineage reaching the heap is
the non-retagging `as_ptr` one. This is the same removable-protector class as Phase 9.1/9.2, and
the same lesson as the two reverted "TB-clean" commits earlier this phase — a structural proof,
not a temporal one.

### Supporting changes
- `unsafe impl Send + Sync for ExecutorScratch` — `NonNull` is conservatively `!Send/!Sync`, but
  it is an OWNING pointer to a `Send + Sync` `CompletionChannel` (behaves exactly as the
  `Box<CompletionChannel>` it stands in for, which would be auto-`Send + Sync`). Restores
  `Schedule: Send` so `ThreadPool::install`'s `F: Send` dispatcher closure type-checks as before.
  Required (compiler-demonstrated), justified, no new cross-thread sharing introduced.
- `SpawnPointers<'a>` retyped: `{ systems: *mut SystemBox, completion: CompletionCell<'a> }`.
  `systems` (still `!Send`) is reached only via the `&self system_slot` method (forces whole-struct
  capture so `unsafe impl Send for SpawnPointers` governs the closure); `completion` is `Send`.
- Phase 9.2 `scope.rs` Candidate-U and Phase 9.3a `#[cfg(miri)] yield_now()` sites **untouched**
  (the diff does not touch `crates/boyko_threadpool/` at all).
- Memory orderings **unchanged**: worker `push` then `fetch_add(Release)`; dispatcher
  `load(Acquire)`; drain `fetch_sub(Relaxed)`.

## Tree-Borrows soundness (per-allocation argument)
`completion` owns a heap allocation `H` distinct from the `Schedule` allocation `S`. The
`&mut self` FnEntry protector lives in `S`'s tag tree and gates only `S`'s bytes; `H` has its own
tree. Every tag that touches `H` — the once-per-frame mint, the dispatcher's `channel()` uses, the
workers' `channel()` uses — descends from the same non-retagging `NonNull::as_ptr` raw tag (no
sibling reborrows), and there is **no `&mut` tag and no protector anywhere in `H`** (the cell is
read-only; the channel mutates only through its own interior mutability — `ArrayQueue`'s
`UnsafeCell` + `AtomicUsize`). So concurrent `&`-mediated interior-mutable writes among
non-protected cousins are TB-permitted — the canonical `&AtomicUsize`-shared-across-threads
pattern. The worker push is no longer a foreign write under a protector. ∎

## Verification gate (all run by the orchestrator)

| Oracle | Result |
|--------|--------|
| **`miri_schedule_parallel`** (`-Zmiri-tree-borrows`, the 9.3c gate) | **`1 passed; 0 failed`** (240 s). Was `write access forbidden` inside `ArrayQueue::push`. NO Undefined Behavior, NO protector violation. |
| Residual Miri leaks | 7 × **third-party crossbeam-epoch** (`Local` register + `SealedBag` GC nodes) — NOT boyko (`CompletionChannel` `Box` is freed in `Drop`); silenced by `-Zmiri-ignore-leaks` (same flag `miri_scope.rs` uses). Verified by reading every leak's allocation backtrace. |
| **`cargo test -p boyko-ecs`** (native) | **495 lib + all integration pass, 0 failed** (incl. the ~22 s/~24 s multi-threaded scheduler integration tests). 0 regressions. |
| `cargo check --all-targets` | clean |
| `cargo clippy --all-targets -p boyko-ecs -- -D warnings` | clean |
| **`phase9_scheduler` bench** (0%-gate) | **Hot path FLAT**; dispatch microbenches within session thermal noise — see below. |
| `boyko_threadpool/` diff | empty (Phase 9.2/9.3a untouched) — confirmed by code-reviewer |

### Bench detail (0%-gate)

`par_iter_4096` — the hot parallel path and the most stable bench — is **flat across all four
runs** (19.1–19.5 µs; same-state A/B: +0.2 %, "no change"). The hot query inner loop is
byte-identical (code-review confirmed); the only per-frame delta is one **register-held,
L1-resident** pointer indirection on the cold per-round gate load (`completion.pending_load`) —
the channel base is the once-minted `NonNull` local, so no extra per-iteration load is emitted.

The µs/sub-µs **dispatch microbenches were thermally unreliable this session** (the bench ran
right after two back-to-back 240 s Miri runs): `two_disjoint` measured **1.13 → 1.25 → 1.39 µs
across three runs of equivalent code** (monotonic throttling), and the same-state A/B reported
`50_exclusive` **−4.9 % ("improved")** while `two_disjoint` **+12 % ("regressed")** in the *same
run* — contradictory signs for code sharing the dispatch loop ⇒ noise, not signal. This is the
documented project bench-noise pattern (variance masks signal; needs a cooled multi-run). The
`ExecutorScratch` struct also **shrank** (~120 B of inline `ArrayQueue`+`CachePadded` → an 8 B
`NonNull`), which if anything improves dispatch cache behavior. **Verdict: 0 %-regression on the
hot path; dispatch deltas are session thermal noise.** A cooled publication-grade re-bench can be
run on request — not a soundness-fix blocker.

## Pipeline + the issues the process caught
research/diagnosis (orchestrator) → **architect** (ratified diagnosis, Box+cell plan) →
**architecture-critic** (APPROVED WITH CHANGES: **C1** — `NonNull::from(&*box)` on a live `Box`
field is a `Unique`-retag hazard; mandated Direction A) → orchestrator resolved C1=A and wrote the
dev spec → **developer** (socket dropped mid-FILE-1) → **orchestrator finished the edits** (the
remaining mechanical, fully-specified work on soundness-critical files, one op at a time with
compile checks) → gate run → **code-review** (APPROVED, no CRITICAL/MAJOR; O1 stale comment fixed,
O2 noted).

Issues caught before landing:
1. **Critic — the `Box`-place `Unique`-retag (C1).** Would have re-introduced the class one
   indirection out via a temporal-only safety argument. Switched to a bare `NonNull` (Direction A).
2. **Developer agent socket drop mid-run.** Only FILE 1's structure landed; `reset_for_frame`
   still referenced the deleted fields (would not compile) and the `Drop` impl was missing. The
   orchestrator inspected the real tree (compiler as oracle), finished the three remaining items,
   and proceeded — rather than blindly resuming a context-lost agent.
3. **Visibility (compile).** `CompletionChannel` had to be `pub(crate)` (it appears in `pub(crate)`
   signatures); access from `schedule.rs` is via typed cell methods so the channel fields stay
   private. `queue_is_empty` is `#[allow(dead_code)]` (debug/test-only, release-elided).
4. **`Send`/`Sync` (compile).** The `NonNull` field broke `Schedule: Send`; restored with a
   justified `unsafe impl` on `ExecutorScratch`.

## Files
- `crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs` — `CompletionChannel`,
  `CompletionCell`, `completion: NonNull` field, `new` (`Box::into_raw`), `Drop` (`Box::from_raw`),
  reset asserts, `unsafe impl Send/Sync for ExecutorScratch`, round-trip unit test.
- `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` — cell mint + threading,
  `apply_window_drain`/`try_dispatch_ready` cell params, `SpawnPointers<'a>`, worker closure,
  run-end + unit-test asserts, SAFETY/doc comment refresh.
- `crates/boyko_ecs/tests/miri_schedule_parallel.rs` — `#[ignore]` removed, module doc updated
  (RESOLVED-in-9.3c + the `-Zmiri-ignore-leaks` crossbeam-epoch justification).
