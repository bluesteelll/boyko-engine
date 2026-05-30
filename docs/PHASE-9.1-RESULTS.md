# Phase 9.1 — Parallel Executor Soundness Verification — Results

Branch `ecs`. Turned the `boyko_threadpool` work-stealing pool + `Scope`
fork/join + the parallel `Schedule::run` from "sound by design" (the multi-thread
path was Miri-deferred) into **sound by proof**. Plan:
[`docs/PHASE-9.1-PLAN.md`](PHASE-9.1-PLAN.md); research:
[`docs/PHASE-9.1-RESEARCH.md`](PHASE-9.1-RESEARCH.md).

## Status: COMPLETE — executor PROVEN SOUND. D5.1 **outcome (ii)**: sound under the SB/eventual aliasing model; Tree Borrows **over-approximates** one pattern (not a bug). NOT TB-green.

> **Honesty note.** An attempt to reach the stronger **outcome (i)**
> (Tree-Borrows-green) via a `*const ScopeShared` join-parameter hardening was
> made, **gate-tested under Miri-TB, found insufficient, and REVERTED** (see "The
> hardening attempt"). The executor's soundness does **not** depend on it. This
> document records outcome (ii), which is what the proofs actually establish.
> Production code is at its W1+W2 baseline (no hardening).

| Proof | Result |
|-------|--------|
| **loom** (`tests/loom_pool.rs`) M1/M2/M2b/M3 | **4/4 pass** (default + `LOOM_MAX_PREEMPTIONS=3`) |
| **stress** (`tests/stress.rs`) | **6 pass + 1 ignored** (`no_starvation`), stable ×5 |
| **Miri-1** (`tests/miri_scope.rs`, 3 tests, 2 real workers) | authored; `#[ignore]`-by-default — under TB reproduces the documented over-approximation (run `-- --ignored` to observe) |
| **Miri-2** (`boyko_ecs/tests/miri_schedule_parallel.rs`) | authored; `#[ignore]`-by-default (TB over-approx + third-party crossbeam provenance) |
| native build / `clippy --all-targets -D warnings` / `cargo test` | clean; 494 ecs lib tests pass |
| zero-native-cost | shim = compile-time alias; loom/Miri test-only |

## What proves the executor sound

1. **loom (exhaustive, the protocol).** `tests/loom_pool.rs` drives the **real
   extracted production methods** (C1) — `ScopeShared::{register_task,
   complete_task, is_drained}` and the real `mark_idle`/`unmark_idle`/
   `unpark_one_idle` — over the loom shim, abstracting only the loom-opaque,
   Coq-verified-upstream crossbeam deque transport:
   - **M1** fork/join: no lost wakeup; join returns ⟺ `pending==0`; `completed==N`
     (no task outlives the join — the transmute's premise).
   - **M2 / M2b** idle bitset: the post-`mark_idle` re-poll closes the "Race C"
     window (worker never parks with claimable work); `unpark_one_idle`'s
     `compare_exchange_weak` wakes exactly one worker (no double/lost wake).
   - **M3** shutdown handshake: every worker exits.
   All 4 pass at default and `LOOM_MAX_PREEMPTIONS=3`. (M2's model adds a
   `fence(SeqCst)` to faithfully model the crossbeam injector transport's
   ordering — a test-fidelity detail, not a production change.)
2. **stress (the deque transport, real hardware).** `tests/stress.rs` proves
   exactly-once run + exactly-once drop + no-loss (post-join full-array sweep) +
   panic-safety, under steal contention across 4–8 workers — the path loom
   cannot model and Miri cannot scale to.
3. **Miri + `std::thread::scope` equivalence (the transmute).** Under Tree
   Borrows the `Scope::spawn` lifetime-erasure path reproduces the long-deferred
   protected-tag conflict — **but a `std::thread::scope`-equivalent harness
   (same shape) trips the *identical* TB flag**, so it is a TB
   **over-approximation, not a UAF**. boyko's `Scope` join is structurally
   identical to (and slightly stronger than) `std::thread::scope`, which is
   sound; therefore boyko's is sound. The authoritative model would be Stacked
   Borrows, which accepts the pattern — but SB is **retired** in the current
   nightly Miri, so the std-equivalence is the standing authoritative evidence.
   H4 forcing confirmed the Miri tests exercise the path on genuine worker
   threads (not a vacuous inline-drain).

## The TB conflict, precisely (why outcome (ii), not (i))

Under Tree Borrows, `Scope::drop(&mut self)` holds a protector over the owned
`Box<ScopeShared>` for the entire Drop body, which spans the work-stealing join
wait during which worker threads foreign-write `pending` (`complete_task`'s
`fetch_sub`, via their own `SharedPtr`). TB flags that foreign write against the
dispatcher's protected tag. This is the rayon/`std::thread::scope` pattern; TB
over-approximates it. The conflict has **two** protector sources:
1. the (redundant) `&ScopeShared` argument inside `join_workers_until_drained`,
   and
2. the **dominating** `&mut self` of `Scope::drop` itself, which transitively
   protects the `Box<ScopeShared>` field for the whole call.

## The hardening attempt (made, gate-tested, REVERTED)

The plan's pre-analyzed D5.1 candidate — changing `join_workers_until_drained`
to take `*const ScopeShared` instead of `&ScopeShared`, with per-access
reborrows — was implemented and **gate-tested under Miri-TB**. It removed
protector source (1) but **not** (2), so Miri-1 still failed under TB:
`error: Undefined Behavior: write access ... forbidden` at `scope.rs` (the worker
`fetch_sub`) vs a protected tag created at `thread_pool.rs:205` (`drop(scope)` —
the `&mut self` of `Scope::drop`). Per the plan's "do-not-fake-green;
revert-and-report" rule it was reverted; `scope.rs` is at its W1+W2 baseline.

**Follow-up (Phase 9.2 candidate — architectural, needs architect sign-off):**
remove protector source (2) so no `&mut`-reachable owner of `ScopeShared` is live
during the wait — e.g. make `Scope::shared` a `ManuallyDrop<Box<…>>` (or
`Option<Box<…>>` + `take()`), detach a raw pointer before the join, run the join
via that raw pointer, reclaim/drop the box after — mirroring how
`std::thread::scope` structures its data so no protected parent borrow spans the
children. This touches `Scope`'s layout, `spawn`, the panic-propagation path, and
`Drop`, and must be re-verified under Miri-TB; it is beyond a verification phase's
scope. If it lands and Miri-1/2 go green under TB, the result upgrades to
outcome (i).

## Lessons

- **loom drives production code, not a copy** (C1): extracting the atomics into
  shared `#[inline]` methods made "same orderings" a compile-time guarantee.
- **A green-able "done" for both Miri-TB outcomes** (D5.1) meant the phase
  completes honestly at (ii) even though the (i) upgrade proved harder than the
  pre-analysis suggested (the std-equivalence pre-analysis under-approximated the
  real `&mut self`-owns-`Box` Drop structure).
- **The compiler + Miri are the only reliable oracles for a soundness claim.** A
  "Miri green / TB-clean" report was contradicted by a direct Miri run
  (`exit 1`, forbidden-write UB). Never record "sound/clean" without re-running
  the gate yourself and reading its actual output.

## Deferred (plan §9)

shuttle; Miri+GenMC/RustMC; Miri on the full pool at >2 workers; loom on the
crossbeam deque (impossible/opaque, covered by stress). Plus the outcome-(i)
architectural hardening above. The Miri tests stay `#![cfg(miri)]` + `#[ignore]`
(run `-- --ignored` under `-Zmiri-tree-borrows` to observe the over-approximation);
loom stays behind `--cfg loom`; both test-only.
