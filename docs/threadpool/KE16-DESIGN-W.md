# KE16 design — axis W/C: wake protocol, completion path, batch spawn

Part of the KE16 design; index and candidate table in `KE16-DESIGN.md`. Catalogue entries W0–W21,
G20 in `KE16-VARIANTS-WAKE-LOCALITY-SCHED.md` and the addenda. Line numbers read at this checkout.
Revision 4: the STEAL row of the RMW table is split by SOURCE FLAVOUR — a batch steal from a LIFO
deque is one `SeqCst` CAS plus one `SeqCst` fence PER STOLEN ELEMENT, from a FIFO deque one CAS per
batch (§0; the critic's round-3 blocking item 1); `claim_one_idle` has ONE signature (§1.1, §1.3);
the loom calibration copies are `#[should_panic(expected = …)]` (§1.3, §2.5); the `Worker::push`
resize note is on the SPAWN row (§0); `worker_wake_handle`'s expression type-checks (§3.2); the
parked-joiner visibility residual is stated and, under B1, closed by the idle-marked park (§3.5).
Revision 3: W-a carries the production StoreLoad barrier (`publish_fence`) and is no longer
"zero behavioural change" (§1); the thief-residue cascade excludes the caller's own idle bit (§2.2,
§2.5); M1c's fallback is named for what it proves (§3.7); `par_chunk` is a batch caller (§4.2).

Ships in the base: **W-a**. Built: **W-b** (`ke16-w-gate`), **W-d′** (`ke16-w-count`), **App-4**
(`ke16-c-batch`), **W-f** (`ke16-w-fanout`). Argued away: W-c, W-e, W9, W17 (§6).

## 0. The RMWs and barriers on each path, recounted from the code (the refuters disputed the "4 shared RMWs")

Multi-writer = a cache line written by more than one thread (a transfer per touch when contended);
single-writer = written by one thread only (stays Modified in that core's L1); local barrier = a
fence instruction that orders this core's own accesses (`mfence` on x86-64: ~30–40 cycles, no
line transfer). Axis 38.

| Path | Operation | Site | Line | Class |
|---|---|---|---|---|
| SPAWN | `pending.fetch_add(1, AcqRel)` | `scope.rs:132` | `ScopeShared.pending` — every completer writes it | multi |
| SPAWN (today, A2, A3, A5) | `Injector::push`: `SeqCst` `compare_exchange_weak` on the tail index | crossbeam `deque.rs:1409-1414` (0.8.7) | `injector_local[wid]` tail — under P0 route (b) the owner is the only writer | single (P0/A2) / multi (A3) |
| SPAWN (today, A2, A3, A5) | `slot.state.fetch_or(WRITE, Release)` | `deque.rs:1431` | the block slot | single (P0/A2) |
| SPAWN (A1, A1-fifo) | `Worker::push`: `back`/`front` loads + `fence(Release)` + `back.store(Relaxed)` | `deque.rs:395-429` | `back` — owner-only | **no RMW** (a no-op fence + a `MOV`). Allocation note: when `len >= cap` the push calls `resize(2 × cap)` (`deque.rs:405-411`: a new buffer, an `epoch::pin`, a deferred free of the old one); `MIN_CAP = 64` (`:16`), so after App-1 the 64-chunk physics wave fits and the two 96-chunk waves grow ONCE per wave on the spawn path; a LIFO `pop` shrinks the buffer when `len < cap/4` and `cap > MIN_CAP` (`:530-534`), so a 96-chunk deque shrinks once on the way down. The `Injector` path allocates one `Block` per `BLOCK_CAP = 63` pushes (`:1201-1203`, `:1403-1404`), so A1 is net FEWER allocations per wave than today — but a profile spike at the 65th push is the resize, not a defect |
| SPAWN, today | `wake_rotor.fetch_add(1, Relaxed)` | `worker.rs:324` | every spawner writes it | multi (and, by accident, the producer's StoreLoad barrier) |
| SPAWN, after W-a | `publish_fence()` = `fence(SeqCst)` | `worker.rs` (`unpark_one_idle` prologue; `KE16-DESIGN-A.md` §1.8) | — | **local barrier**, one per wake decision on EVERY arm |
| SPAWN | `idle.load(Acquire)` | `worker.rs:326` | read of a line every parking worker writes | shared read |
| SPAWN (a bit set) | `idle.compare_exchange_weak` + `thread.unpark()` | `worker.rs:338-343` | multi + the target's parker line (`state.swap(NOTIFIED, Release)`, `std/src/sys/sync/thread_parking/futex.rs:88-97`) + `WakeByAddressSingle` iff the target was PARKED | multi + syscall |
| **POP (owner, FIFO deque)** | `front.fetch_add(1, SeqCst)` | `deque.rs:462` | `front` — every successful thief CASes it | **multi, one per executed own task** |
| **POP (owner, LIFO deque)** | `back.store(Relaxed)` + `fence(SeqCst)` + `front.load(Relaxed)`; a `front` CAS only when popping the LAST element | `deque.rs:488-494`, `:500-517` | `back` (owner-only) written; `front` (the thieves' line) READ | **local barrier + one shared READ of `front` per executed own task** (a line transfer when a thief CASed it since, no invalidation of the thieves' copies); a multi-writer CAS once per wave |
| **STEAL from a FIFO deque** (A1-fifo; today's deques) | `epoch::pin` (`lock cmpxchg` on the thief's OWN participant line on x86) + copy the batch + ONE `front.compare_exchange(SeqCst)` for the whole batch (`Steal::Retry` and a full re-copy if the owner popped meanwhile — owner and thieves share `front` under FIFO); residue published by `fence(Release)` + `dest.back.store(Relaxed)` | `deque.rs:1034-1071` (CAS at `:1061`), residue `:1160-1170`; crossbeam-epoch `internal.rs:394-440` | `front` | **multi, ONE per batch of ≤33** on success; each failed attempt is a wasted transfer + re-copy, retried without backoff by `drain_one` (`worker.rs:262-279`) |
| **STEAL from a LIFO deque** (A1) | `epoch::pin` + `front.compare_exchange(SeqCst)` for the FIRST element (`:1082`), then PER ADDITIONAL ELEMENT: `fence(SeqCst)` (`:1101`) + `back.load(Acquire)` + a buffer-swap check + `front.compare_exchange(SeqCst)` (`:1121`) — a failed CAS or an emptied queue ends the batch early with what was taken (`:1129-1131`); a reversal loop only when the DESTINATION is FIFO (`:1145-1156` — the B0 joiner's `scratch`, `scope.rs:448`; never on the worker path under A1, where every deque is LIFO); residue as above | `deque.rs:1077-1142` | `front` | **multi, ONE per STOLEN ELEMENT — up to 33 CASes + 32 fences per batch**, i.e. the same shared-RMW count as 33 single `Stealer::steal()`s; the batch saves victim probes and epoch pins, not CASes (the critic's round-3 blocking item 1) |
| STEAL from `injector_global` (every A; A3's whole steal side) | ONE `head.index.compare_exchange_weak(SeqCst)` per batch (`:1851`) + per-slot state loads; the `fence(SeqCst)` at `:1821` runs only on the path that can conclude "empty" (`new_head & HAS_NEXT == 0`) | `deque.rs:1795-1913` | the injector `head` — every thief | multi, one per batch of ≤33 |
| STEAL one (`Stealer::steal()`, B1's external arm) | `epoch::pin` + `fence(SeqCst)` (`:647`) + one `front` CAS (`:670`) | `deque.rs:637-680` | `front` | multi, one per task |
| COMPLETE | `waker.unpark()` = `state.swap(NOTIFIED, Release)` + `WakeByAddressSingle` iff PARKED | `scope.rs:158`; `futex.rs:88-97` | the joiner's parker line — every completer writes it | multi |
| COMPLETE | `pending.fetch_sub(1, AcqRel)` | `scope.rs:159` | as above | multi |
| PARK (worker) | `idle.fetch_or(bit, Release)` → re-poll (steal-path fences) → `park()` → `idle.fetch_and(!bit, Release)` | `worker.rs:99-120` | one line for all workers | multi |
| PARK (joiner) | `unpark_one_idle` (fence + `idle` load [+ rotor RMW + CAS + unpark]) + `park_timeout(50 µs)` | `scope.rs:511-512` | as SPAWN's wake | multi |

So the spawn path today is **four `lock`-prefixed RMWs, two of them multi-writer**; the completion
path is **two multi-writer RMWs plus a conditional syscall, unconditional**; the owner's own-task
execution path pays **one multi-writer RMW per task under FIFO deques** (today's constructor,
`thread_pool.rs:595`) and **one local barrier per task under LIFO**; for a flat wave the completion
path runs exactly as often as the spawn path. Axis A removes the two single-writer spawn ops (A1)
or moves them to a multi-writer line (A3).

**The end discipline is a two-sided trade, not a one-sided saving** (revision 4, the critic's
round-3 blocking item 1). Per task of a wave, let `s` be the fraction of tasks executed by a thief
rather than by the spawning owner. Shared-line RMWs on the `front` line per task:

| deque flavour | owner runs its own task | thief runs a stolen task | per-task total |
|---|---|---|---|
| LIFO (`ke16-a1`) | 0 (local fence + a shared read) | 1 CAS (per element) | `s` |
| FIFO (`ke16-a1-fifo`) | 1 `fetch_add` | 1 CAS per batch ≈ 1/33 | `(1 − s) + s/33` |

The shapes this pass fixes are ONE spawner and W−1 thieves: a `par_iter` wave of W chunks
(`batches_per_thread = 1`), a physics color of 4W–6W chunks. There `s ≈ (W−1)/W ≈ 0.94` at W=16:
LIFO ≈ 0.94 shared RMWs per task, FIFO ≈ 0.09 — FIFO is predicted to have the LIGHTER steal path on
exactly the consumer shape, by an order of magnitude in RMW count. What the count does not price,
and only the measurement can: under FIFO the owner's `fetch_add` and the thieves' batch CAS hit the
SAME end of the deque, so a thief's batch CAS fails whenever the owner popped between the copy and
the CAS and the whole batch is re-copied and retried without backoff (`drain_one`, `worker.rs:
262-279`) — with 15 thieves on one victim at 1 µs bodies that is a contention mode LIFO does not
have (its owner works the `back`; the ends meet only on the last element). Also unpriced: the LIFO
owner-first order on the B1 joiner (its own newest chunk before a batch-stolen sibling system) and
chunk locality. Hence the a1 / a1f decision is MEASURED, symmetrically, on the 1–10 µs × 64W
worker-route cells and on physics (`KE16-DESIGN-MEASUREMENT.md` §7 Step A rules 3–4), never
derived from either count. Revision 3's "the LIFO pop trades a contended RMW for a local fence" was
true of the owner's side only and is retracted as a ranking argument (`KE16-DESIGN-A.md` §1.4).

A steal LIMIT under LIFO (`steal_batch_with_limit_and_pop(local, L)`, `L < 33`) is NOT a lever for
this cost: the CAS count is one per stolen ELEMENT regardless of `L` (`deque.rs:1077-1142`), so `L`
only changes how many tasks a thief holds per probe (distribution) and how many probes it makes —
the same total. rayon's LIFO-plus-`steal()` (`L = 1`) pays the identical per-element CAS. Not built;
the `top_lane` receipt would show hoarding if the batch of 33 were a distribution problem.

This axis removes the multi-writer RMWs of the spawn and completion paths:

| After | SPAWN (nobody parked) | COMPLETE (per task) |
|---|---|---|
| base + W-a | `pending` RMW + `fence(SeqCst)` + `idle` load (+ A's queue op) | unchanged |
| + W-b | `pending` RMW + `len()` (two loads); the fence and the `idle` load only on a ≤1 push | unchanged |
| + W-d′ (route b) | unchanged | `pending` RMW only; the last completer adds one parker swap + a conditional syscall |
| + App-4 | one `pending` RMW per WAVE; one wake decision per wave | unchanged |

## 1. W-a — the fenced wake prologue, rotor RMW behind the mask load (W6/E16). Ships in the base, no feature

### 1.1 The change

`unpark_one_idle` (`worker.rs:320-349`) today does `wake_rotor.fetch_add` at `:324` before
`idle.load` at `:326`. After:

```rust
pub(crate) fn unpark_one_idle(inner: &PoolInner) -> bool {
    unpark_one_idle_excluding(inner, 0)
}

/// `exclude` masks bits the caller must never claim — its OWN bit when the caller is a worker
/// inside the post-`mark_idle` re-poll (`worker.rs:104`, before `unmark_idle` at `:105`): a thief
/// that finds residue there must hand the cascade to a SIBLING, not wake itself (§2.2).
pub(crate) fn unpark_one_idle_excluding(inner: &PoolInner, exclude: u64) -> bool {
    publish_fence();                                                // the StoreLoad barrier, KE16-DESIGN-A.md §1.8
    let mut mask = inner.idle.load(Ordering::Acquire) & !exclude;
    if mask == 0 { return false; }                                   // busy pool: one fence + one load, no RMW
    let start = (inner.wake_rotor.fetch_add(1, Ordering::Relaxed) % 64) as u32;
    loop {
        match claim_one_idle(inner, mask, start) {                   // ONE CAS attempt (AcqRel / Acquire), shared with A5 and W-f
            Some(id) => { inner.workers[id as usize].thread.unpark(); return true; }
            None => { mask = inner.idle.load(Ordering::Acquire) & !exclude; if mask == 0 { return false; } }
        }
    }
}

/// THE ONE claim core (revision 4: one signature for W-a, A5, W-f and the loom M2/M2b/M2c/M4
/// transcriptions — the critic's round-3 non-blocking item 3). Picks the lowest set bit of `mask`
/// at or above `start` (wrapping) and tries ONCE to clear it with `compare_exchange_weak(mask,
/// mask & !bit, AcqRel, Acquire)`. `Some(id)` = the bit was claimed and the caller MUST unpark
/// worker `id`; `None` = the CAS lost (the caller re-reads the mask and decides again). Never loops,
/// never unparks.
#[inline]
pub(crate) fn claim_one_idle(inner: &PoolInner, mask: u64, start: u32) -> Option<u32>
```

Orderings unchanged where they existed: the rotor is `Relaxed` (its own comment: "no data is
published through it"), the mask load is `Acquire` (pairs with `mark_idle`'s `Release` `fetch_or`),
the claim CAS is `AcqRel`/`Acquire`. NEW: `publish_fence()` (`fence(SeqCst)`, through the
`crate::sync` shim so loom sees it) precedes the load.

### 1.2 What this is and is not

**Not "zero behavioural change."** Today the producer-side StoreLoad barrier of the Race-C
protocol (the SB litmus of loom M2's fidelity note, `tests/loom_pool.rs:143-166`) is supplied by
accident: the `Injector::push` CAS + `fetch_or` and the rotor `lock xadd` are all full barriers on
x86, none placed for that purpose. W-a removes the rotor RMW from the busy path and, under A1, the
push itself becomes a plain store — so without the fence the busy-path wake decision would be
`MOV back; MOV idle`, which x86-TSO reorders (the critic's blocking item 1;
`KE16-DESIGN-A.md` §1.8). W-a therefore REPLACES one contended-line RMW with one local full
barrier. The wake-target sequence is identical whenever a wake happens (the rotor advances once per
wake instead of once per push — a different but equally fair rotation). The C11 argument becomes
formal for the first time: producer `publish; fence(SeqCst); load idle` against consumer
`fetch_or; fence(SeqCst) (steal path); load back` is the fenced SB litmus, which forbids both loads
reading stale. On x86 the fence is an `mfence` (~30–40 cycles); on the injector arms it is
redundant with the `lock`-prefixed ops and kept so the protocol has one shape and one loom model.

### 1.3 Obligation — loom M2 re-attributed, and calibrated

`tests/loom_pool.rs` M2 (`:200-265`): the producer's `fence(Ordering::SeqCst)` at `:217`, today
commented "injector push transport fence", becomes a call to the real `publish_fence()` exported
through `loom_exports` (one line, forwards to `crate::sync::fence(SeqCst)` — C1: the model drives
production code for the producer's half of the litmus). The worker's fence at `:237` keeps its
transport attribution, corrected to name the STEAL path that still supplies it under every A
(`Injector::steal_batch_and_pop`'s explicit fence at `deque.rs:1821`, inside the `new_head &
HAS_NEXT == 0` empty-check path — the one the litmus needs; `epoch::pin` at `:1006` in
`Stealer::steal_batch_and_pop`). The module's fidelity note (`:143-166`) is rewritten accordingly
(index §7). **Calibration, run at Step 0 and recorded:** a copy `loom_m2_calibration_no_producer_
fence_is_lost` with the `publish_fence()` call deleted MUST go red (the file's own note says the
window reproduced without it, `:147-152`; the reading is re-taken, not trusted). The copy is
annotated `#[should_panic(expected = "M2: lost wake")]` where `"M2: lost wake"` is the literal
prefix of the model's lost-wake oracle `panic!` message — a bare `#[should_panic]` would pass on a
loom deadlock report or any unrelated assertion and the copy could "go red as required" without its
oracle ever firing (the critic's round-3 non-blocking item 5). The claim core is factored into
`claim_one_idle(inner, mask, start) -> Option<u32>` (§1.1; one CAS attempt) shared by A5
(`KE16-DESIGN-A.md` §4.1) and W-f (§5); M2/M2b's transcription is re-synchronised to it.

## 2. W-b — wake iff the pre-push length was ≤ 1, plus the thief-residue cascade (W1 + FJP `signalWork`)

### 2.1 The rule

- **Push gate (the ≤1 rule).** A push wakes one worker iff the destination queue held **at most
  one** task immediately before the push. The length is read before the push in every A arm
  (`KE16-DESIGN-A.md` §1.2): `Worker::len()` under A1 — `back.load(Relaxed)` (the owner's own
  line) + `front.load(SeqCst)` (the thieves' line: every successful steal CASes `front`; this is a
  LOAD, not an RMW, and `push` already loads `front` at `deque.rs:398`, so no line is touched that
  the push does not touch anyway) — and `Injector::len()` under A2/A3/A5 and for the dispatcher's
  `injector_global` pushes (three `SeqCst` loads in a stable-tail loop, `deque.rs:1978-2010`).
  Why ≤1 and not "was empty": a queue holding exactly one task when the length is read can be
  emptied by a thief before the push lands (the critic's interleaving); a second wake on the 1→2
  transition covers that thief's departure. This is the JDK 8 `ForkJoinPool.WorkQueue.push` rule —
  `n = s − b` (the pre-push size) and `if (n <= 1) signalWork` `[R:S]` (the critic's read; not
  re-opened here) — and the reason rayon's `new_jobs` wakes on `!queue_was_empty` OR "too few awake
  idle" (`sleep/mod.rs:265-271` at this checkout `[L]`): both cover the same race, rayon with a
  packed counter every idle transition writes (W2's cost, not taken), FJP with a second wake, which
  costs nothing here because a wake whose mask load finds no bit is one fence + one load.
- **Thief-residue cascade.** After a successful `steal_batch_and_pop` into its own registered
  deque, a thief whose deque now holds a residue (`local.len() > 0`) wakes one worker OTHER THAN
  ITSELF. This is the ≤1 rule applied to the thief's OWN queue (its 0→k transition) and is FJP's
  cascade (each activated worker activates another; O(log W) to full activation `[S]`). The site is
  shared: `drain_one`'s callers in `pop_global_injector` (`worker.rs:227-229`) and
  `try_steal_random` (`:252`), so the worker loop AND the B1 joiner cascade identically. The
  self-exclusion is load-bearing: both helpers are reached from the post-`mark_idle` re-poll
  (`worker.rs:104`) BEFORE `unmark_idle` (`:105`), where the caller's own bit is still set; an
  unmasked claim there can pick the caller (rotor-dependent), leaving a stale token on its own
  parker and W−1 parked siblings unwoken — the chain, W-b's only fan-out for silent pushes, broken
  at hop one (the critic's non-blocking item 2). The residue store itself is a plain store
  (`deque.rs:1160-1170`), so this wake goes through the fenced prologue like every other (§1).
- **Kept.** The post-`mark_idle` re-poll (`worker.rs:101-108`, Race C) and the joiner's pre-park
  `unpark_one_idle` (`scope.rs:511`). Removed: nothing else; the wake-on-every-push is what the gate
  replaces.

### 2.2 The switch point — ONE helper, and the cascade site

```rust
/// The wake decision after a push, shared by every placement arm (`push_task`, `spawn_batch`, the
/// dispatcher's pushes). `pre_len` is the destination queue's length read immediately before the
/// push. Under `ke16-w-gate` this is the FJP `signalWork` rule; otherwise the wake is unconditional.
/// The StoreLoad barrier lives inside `unpark_one_idle` (§1), so a gated-out push pays neither it
/// nor the mask load.
#[inline]
pub(crate) fn wake_after_push(inner: &PoolInner, pre_len: usize) {
    // === KE16 W switch: ke16-w-gate ===
    #[cfg(feature = "ke16-w-gate")]
    if pre_len > 1 { return; }
    #[cfg(not(feature = "ke16-w-gate"))]
    let _ = pre_len;
    unpark_one_idle(inner);
}
```

```rust
// pop_global_injector(inner, wid, local) / try_steal_random(inner, wid, local, rng), after Steal::Success(t):
#[cfg(feature = "ke16-w-gate")]
if !local.is_empty() { unpark_one_idle_excluding(inner, 1u64 << wid); }   // residue exists: hand the cascade to a SIBLING
```

`pop_global_injector` gains the `wid` parameter (it has none today, `worker.rs:227`); the B1 joiner
passes its lane's `wid` (`KE16-DESIGN-B.md` §2.2). Under B1-P the exclusion is LOAD-BEARING for the
joiner exactly as for a worker: its post-`mark_idle` re-poll (`pop_any`) reaches these helpers with
its own bit set, and an unmasked cascade there could claim the joiner itself (revision 4; before
B1-P the joiner's bit was never set and the exclusion was a no-op on that path). One helper serves
both callers.

The dispatcher's pushes (frame path, `schedule.rs:1272` → `injector_global`) take the same gate:
the first two systems of a round wake one worker each (pre-length 0 and 1); the batch-steal of the
rest into those workers' deques leaves residue → each wakes the next → O(log W) chain. The
critic's frame-path case — "a worker takes the last system between the length read and the push" —
is a pre-length of 1 → a wake regardless.

### 2.3 The invariant (weak), why it holds, and what the gate deliberately gives up

**W-b-1 (weak).** At no time does a task sit in a queue of the scan set (a registered deque or
`injector_global`) while every worker of the pool is parked and no wake is pending (a claimed bit
whose `unpark` has been or is about to be issued). Equivalently: every pushed task is eventually
run, and the delay before some awake worker finds it is bounded by one task body.

**Not claimed (the strong form the round-1 text asserted, refuted by the critic).** "Every non-empty
queue has, since its last empty→non-empty transition, issued one wake or been batch-stolen by a
thief that applied the residue rule." With check-then-push and no coupling to thieves this is false
whenever ≥ 2 thieves empty a queue of ≥ 2 between the spawner's length read and its push: the push
sees `pre_len ≥ 2`, wakes nobody, and the queue holds the new task with no wake issued for it.

**Why the weak form holds.**

1. A worker parks only after `pop_any` over every queue → `mark_idle` → `pop_any` again found
   nothing (`worker.rs:93-117`); the two `pop_any`s bracket its `fetch_or`, and the crossbeam STEAL
   path supplies the consumer-side `SeqCst` fence of the SB litmus before every sibling `back` load
   (`KE16-DESIGN-A.md` §1.8).
2. A task in worker X's registered deque was pushed by X itself (Chase-Lev owner contract) inside a
   task body; X's next act is its join (B0/B1: it consults its own deque first) or its loop's own
   pop (`worker.rs:62`). X does not park with a non-empty own deque. So "every worker parked with a
   task in a deque" cannot arise from the owner's side.
3. Let a task `t` be in queue `Q` (a deque of a NON-parked owner, or `injector_global`) while every
   OTHER worker is parked. Consider the pushes into `Q` since it was last empty. The push that took
   `Q` from 0 to 1 and the push that took it from 1 to 2 each read a pre-length ≤ 1 and each issued
   a wake decision — `publish_fence()` then the mask load: a bit was claimed (a wake is pending —
   done) or the mask was 0 at that load. In the latter case every worker that parked afterwards did
   its post-`mark_idle` re-poll after its `fetch_or`; by the fenced SB litmus (the production
   `publish_fence` on the producer, the steal-path fence on the consumer — loom M2) either that
   re-poll saw `Q` non-empty (it does not park) or the push's mask load saw its bit (a claim —
   contradiction). Hence if `Q` still holds `t`, some worker is awake or a wake is pending.
4. The thieves that drained `Q` between a read and a push (the strong form's counterexample) are by
   construction awake and hold tasks; when their bodies end they re-scan every queue and find `t`.
   The bound is one body of the slowest such thief; the residue cascade then re-fans out — to a
   sibling, never to the cascading thief itself (§2.1).

**What the gate gives up, priced.** (i) The one-body stall of item 4 — on the grid it can appear on
the 100 µs–1 ms × W cells as a start-of-wave delay for a few chunks; on physics as a per-color
latency of ≤ one chunk when the previous color's last thieves are still running; the measurement
decides whether it costs anything. (ii) A push into a queue holding ≥ 2 never wakes even with W−1
workers parked; the cascade recovers them one `unpark` latency at a time (Gast's λ term per hop).
On Windows an `unpark` of a parked thread is a `WakeByAddressSingle` and the wake latency is the
scheduler's, tens of µs; the chain to W=16 is log₂ 16 = 4 hops. That is the cost W-f (§5) is built
to measure against.

The last-searcher race Tokio names (N14: a "wake only if no searcher" rule with no compensating
wake) does not arise: there is no searcher count and no suppression keyed on searchers — the gate is
keyed on the queue's pre-length alone, and the cascade on the thief's queue state alone.

### 2.4 Cost and prediction

Spawn path, `pre_len ≥ 2` push: `pending` RMW + `len()` — the fence and the `idle` load are gone (a
shared line every parking worker writes, read by every spawner today). `pre_len ≤ 1` push: as W-a.
On a wave of N tasks with siblings parked: ≤ 2 claims + ≤ 2 syscalls on the spawner instead of N.
On the grid: the worker route at 1 µs × {4W, 64W} gains up to (N−2) × (fence + idle load [+ CAS +
unpark]) when workers are parked at the wave start (criterion's back-to-back iterations keep them
spinning, so the pool grid understates this; the physics consumer, whose 6 colors leave workers
parked between steps, is where it shows). Risk: the 100 µs–1 ms × W cells, where the chain's 4
hops of Windows wake latency plus the one-body stall may exceed the W serial unparks the spawner
paid before.

### 2.5 Obligation — loom M4, built to DISCRIMINATE

Shape (new in `tests/loom_pool.rs`, `#[cfg(feature = "ke16-w-gate")]`):

- **Transport:** a toy owner queue = `Mutex<VecDeque<u32>>` + a SEPARATE loom `AtomicUsize len`.
  The owner's push is `let pre = len.load(Acquire); { lock; push_back }; len.fetch_add(1, AcqRel);
  gate(pre)` — the snapshot is a distinct operation from the push, so the check-then-push race the
  real deque has is expressible (a length whose `fetch_add` were atomic with the push would remove
  the race and pass vacuously — the critic's warning). A thief's take is `{ lock; pop_front }` then
  `len.fetch_sub(1, AcqRel)`; a thief takes up to two items, runs one and keeps one in its own toy
  slot (`AtomicUsize`), applying the residue rule (claim + unpark, EXCLUDING its own bit) when the
  slot is non-empty.
- **Gate:** `gate(pre) = if pre <= 1 { publish_fence(); let m = idle.load(Acquire) & !exclude;
  if m != 0 { if let Some(w) = claim_one_idle_model(&idle, m, start) { threads[w].unpark() } } }` —
  the real `publish_fence`, the transcribed one-attempt claim (the same `claim_one_idle` shape M2
  already uses, `Option<u32>`), plus loom's REAL `Thread::unpark` on the thief's handle. A second
  copy of the model with `if pre == 0` (the pure-empty gate) is the calibration
  `loom_m4_calibration_empty_gate_is_lost`, annotated `#[should_panic(expected = "M4: lost wake")]`
  (the literal prefix of the lost-wake oracle's message): it MUST go red on the critic's
  interleaving (thief T1 pops the single item after the owner's snapshot of 1; the owner pushes with
  no wake; T1 parks after finding nothing; T2 was never woken). The no-self-exclusion copy
  `loom_m4_calibration_no_self_exclusion_claims_self` is annotated `#[should_panic(expected =
  "cascade claimed self")]`. Neither calibration is a bare `#[should_panic]`.
- **Thieves (2):** the real `mark_idle`/`unmark_idle` with the steal-path `fence(SeqCst)`
  discipline, loom's real `park()` (loom 0.7.2 persists a token issued before the park:
  `loom-0.7.2/src/rt/thread.rs:153-165` `set_unparked` → `Runnable { unparked: true }`, consumed by
  `rt::park`, `src/rt/mod.rs:87-107`), the post-`mark_idle` re-poll — which, when it finds two
  items, keeps one and CASCADES from inside the re-poll with its own bit still set (the shape of
  `worker.rs:104` → `try_steal_random` → residue) — then `park`, then `unmark_idle`, loop.
- **Owner:** three gated pushes, then `done.store(true)`, then the model's SHUTDOWN: `shutdown
  .store(true)` and `unpark` on every thief (this is what terminates parked thieves; production's
  `shutdown_and_join` does the same, `thread_pool.rs:454-470`).
- **Oracle.** A thief that returns from `park()` checks its own bit in `idle`: if the bit is STILL
  set, nobody claimed it — it was woken by the shutdown unpark only — and if any item then remains
  in the owner's queue or in the other thief's slot, that is a lost wake: `panic!`. A claimed bit
  (cleared by a gate or cascade wake) is a legitimate wake. This distinguishes "woken for work" from
  "woken to die" without a snapshot assertion that would false-trigger between a push and its
  claim. loom's deadlock detection covers every interleaving in which the shutdown unpark is
  somehow not reached. **Self-claim oracle:** a cascade that claims the cascading thief's OWN bit
  panics ("cascade claimed self") — the reading that the exclusion is doing its job; a calibration
  copy without the exclusion must trip it.
- **Assertions:** every interleaving terminates; all three items were run exactly once; no
  lost-wake panic; no self-claim panic. `LOOM_MAX_PREEMPTIONS=3` as for M1–M3. If the 3-item
  model's state space is too large, split into M4a (2 items: the gate race) and M4b (3 items: one
  thief steals two with residue and must cascade to the other).

## 3. W-d′ — count-gated completion with a `PoolInner`-owned, loom-visible wake target (W20 / E28)

### 3.1 The refuted premise

`scope.rs:149-151`: "learning we are last would require reading `pending` after the sub — too
late". `fetch_sub` returns the previous value; "we are last" is `prev == 1`, known before any
further access. The real constraint (`:139-148`) is that `ScopeShared` may be freed the instant the
joiner observes zero, so the WAKE TARGET must live outside it. rayon (`CountLatch::set`: registry +
worker index read before the swap; the registry outlives the scope) and std (`ScopeData` in an
`Arc`) both ship the count gate (N66, `[S]`).

### 3.2 Data — the wake target's TYPE (the critic's round-2 blocking item 3)

`crate::sync` (`src/sync.rs`) gains one shimmed name:

```rust
/// KE16 W-d'. The type of a count-gated wake target: a per-worker handle owned by `PoolInner`
/// (`WorkerHandle.thread`), pointed at from `ScopeShared` and unparked by the last completer AFTER
/// its decrement. Native: a transparent alias of the handle `WorkerHandle` already holds, so the
/// pointer is `&inner.workers[wid].thread` with no cast and no new allocation. Under loom: a
/// counting newtype over `loom::thread::Thread` so the M1c model, which cannot build a `PoolInner`
/// (crossbeam-coupled, see the shim's scope note above), can own the target itself and count its
/// unparks — the model still drives the REAL `complete_task` (C1).
#[cfg(not(loom))]
pub(crate) type WakeHandle = std::thread::Thread;

#[cfg(loom)]
pub(crate) struct WakeHandle { inner: loom::thread::Thread, unparks: loom::sync::atomic::AtomicUsize }
#[cfg(loom)]
impl WakeHandle {
    pub(crate) fn new(t: loom::thread::Thread) -> Self { … }
    pub(crate) fn unpark(&self) { self.unparks.fetch_add(1, Ordering::AcqRel); self.inner.unpark(); }
    pub(crate) fn unparks(&self) -> usize { self.unparks.load(Ordering::Acquire) }
}
```

The `sync.rs:43-48` note ("`WorkerHandle.thread` is a `std::thread::Thread` … distinct from the
shimmed `ScopeShared.waker`") stays true and gains: under `cfg(not(loom))` `WakeHandle` IS
`std::thread::Thread`, so `WorkerHandle.thread: Thread` is a `WakeHandle` by identity; under
`cfg(loom)` no production `WakeHandle` is ever constructed (§3.3).

`ScopeShared` (`scope.rs:44-72`) gains one field, written once in `ScopeShared::new` and never
again:

```rust
/// KE16 W-d'. The count-gated wake target: non-null iff the thread that opened the scope was a
/// registered worker of the scope's pool acting as that worker (`tls::worker_lane_for` was `Some`
/// at scope creation — the ONE identity predicate, `KE16-DESIGN-A.md` §1.1). Then it points at
/// `inner.workers[wid].thread`, owned by `PoolInner` and alive independently of this allocation,
/// and the last completer unparks it AFTER the decrement. Null for the dispatcher, an unattached
/// thread, a worker of ANOTHER pool, or a worker inside an `install` frame of this pool: then
/// `waker` is the target and the unpark must precede the decrement, as today.
pub(crate) joiner_wake: *const crate::sync::WakeHandle,
```

`ScopeShared::new(waker, joiner_wake)`; `install` (`thread_pool.rs:224`) and `scope` (`:269`) pass
`self.worker_wake_handle(tls::worker_lane_for(self))`:

```rust
impl PoolInner {
    /// The W-d' wake target for a joiner that is worker `lane.wid` of this pool; null when the
    /// joiner is external. ONE `cfg(loom)` pair in production code, documented here: under loom
    /// no pool can be built and no scope is ever opened through this path (the models construct
    /// `ScopeShared` directly with a model-owned target, M1c), so the loom arm returns null and is
    /// never executed — the same compiles-never-runs status as the `crate::sync::thread::current()`
    /// waker calls at `thread_pool.rs:224,269`.
    #[cfg(not(loom))]
    #[inline]
    fn worker_wake_handle(&self, lane: Option<tls::WorkerLane>) -> *const crate::sync::WakeHandle {
        // `WorkerHandle.thread: Thread` (`thread_pool.rs:88-92`) and `WakeHandle = Thread` under
        // `cfg(not(loom))`, so the coercion is `&Thread as *const Thread` — no deref, no cast
        // through another type (`std::thread::Thread` has no `Deref`; revision 4).
        match lane {
            Some(l) => &self.workers[l.wid as usize].thread as *const crate::sync::WakeHandle,
            None => core::ptr::null(),
        }
    }
    #[cfg(loom)]
    #[inline]
    fn worker_wake_handle(&self, _lane: Option<tls::WorkerLane>) -> *const crate::sync::WakeHandle {
        core::ptr::null()
    }
}
```

The `WorkerLane` from the predicate carries a `wid < worker_count` by construction
(`KE16-DESIGN-A.md` §1.1), so the index cannot be the dispatcher sentinel (`install` on a same-pool
worker rewrites `CURRENT_WORKER_ID` to `WORKER_ID_DISPATCHER`, `thread_pool.rs:205`; the predicate
returns `None` there and the scope is external).

### 3.3 The switch point

```rust
#[inline]
pub(crate) fn complete_task(&self) {
    // === KE16 W switch: ke16-w-count ===
    #[cfg(feature = "ke16-w-count")]
    {
        // Copy the target out of `*self` BEFORE the decrement: after it the joiner may free this
        // allocation. A null target means an external joiner — today's order below.
        let target = self.joiner_wake;
        if !target.is_null() {
            if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                // SAFETY: `target` points at `inner.workers[wid].thread`, owned by the `PoolInner`
                //   of the pool this task belongs to, which is alive: the thread executing this
                //   completion is either a worker of that pool (holding its own `Arc<PoolInner>`
                //   for its whole life, `worker.rs:21-37`) or a joiner running this task inline
                //   inside an `install`/`scope` frame of that pool, whose `&'scope PoolInner`
                //   (`scope.rs:250`) outlives the frame. No other thread ever executes a task of
                //   the pool (tasks live only in its queues). The pointee is a `Thread` handle
                //   (an `Arc` inside `std`), valid even if the worker thread has since exited.
                //   Only `PoolInner`-owned memory is touched; `*self` is not accessed after the
                //   decrement.
                unsafe { (*target).unpark() };
            }
            return;
        }
    }
    // External joiner (or the feature off): today's order, for today's reason (`waker` lives here).
    self.waker.unpark();
    self.pending.fetch_sub(1, Ordering::AcqRel);
}
```

Orderings: the decrement stays `AcqRel` (the joiner's `is_drained` `Acquire` load pairs with it,
loom M1); the `unpark` is `std`'s `swap(NOTIFIED, Release)` (`futex.rs:88-97`), which pairs with the
joiner's `park_timeout` `Acquire` (`:66-83`) on its own parker. The `Thread::unpark` on
`WorkerHandle.thread` (`thread_pool.rs:88-92`) is the same handle `unpark_one_idle` uses; a worker
inside a join is not idle (no bit set), so `unpark_one_idle` never targets it and the two wake
sources do not interfere; a stale token from an earlier claim only makes one later park return at
once (a re-check, harmless).

### 3.4 The lost-wakeup window this closes on route (b)

Today (unpark BEFORE decrement): the completer stores the token while the joiner is running; the
joiner's next `park_timeout` consumes it (`fetch_sub(1, Acquire) == NOTIFIED`, `futex.rs:67-69`)
and returns at once; the joiner re-checks `is_drained` — still false, the completer has not
decremented yet; the joiner parks again; the decrement lands; nobody wakes the joiner → it sleeps
until the backstop, ≥1 ms on Windows. With the decrement FIRST, the joiner's check-then-park is
race-free against the wake: a check that sees `pending ≥ 1` precedes the last decrement in the
RMW total order, whose `unpark` either finds the joiner parked (wakes it) or not yet parked (leaves
a token the next park consumes). No interleaving loses the wake. The 50 µs constant at
`scope.rs:512` stays as a defensive bound (it costs nothing when the wake arrives) and its comment
says so (index §7).

### 3.5 The external arm keeps its window — stated, priced, and where its fix is recorded

The external joiner (the dispatcher on the frame path; the fontbake bake thread; a cross-pool
worker; a worker inside an `install` frame) keeps today's unpark-before-decrement, because the only
target that outlives `ScopeShared` for it is not available without a new lifetime decision. The
window of §3.4 therefore PERSISTS on that arm: the `miri_scope.rs` suite, whose three tests all join
from the test thread (`miri_scope.rs:127,168,204`), keeps its documented ~1/16 many-seeds liveness
timeout (`:56-59`), and that reading is NOT a W-d′ gate (§3.7). On the ECS frame path the window is
reachable only in the ns gap of `KE16-DESIGN-B.md` §4 and is masked by the joiner's Backoff snooze
(kept under every B arm); on the fontbake bake it costs at most one ≥1 ms backstop per bake. The
executor's per-system wake (`schedule.rs:665-671`) keeps arriving per completion, so the
apply-window latency does not change.

The fix, recorded for KE17 and not built here: give external joiners a lifetime-independent target
too — a per-thread `'static` `WakeHandle` slot (`Box::leak` of `thread::current()` once per thread
that ever joins externally, held in a `thread_local! Cell<*const WakeHandle>`; bounded by the thread
count; never freed) — with decrement-then-unpark kept UNCONDITIONAL for the frame scope (every
completion still wakes the dispatcher). Not this pass because it changes the frame path's completion
order, which is an ECS per-system-wake decision, for a window that no production number has been
shown to hit.

**A second residual, and where it is closed (revision 4, the critic's round-3 non-blocking item
7).** A joiner parked inside `join_workers_until_drained` is, TODAY and under B0, never
idle-marked: its bit in `inner.idle` is clear, so it is invisible to every OTHER wave's wake
decision — a sibling that spawns a wave after the joiner's last scan wakes only idle-masked
workers, and the joiner sleeps until its own last completer (W-d′) or the ≥1 ms Windows backstop.
On the physics route, where each of the 16 systems joins its own `par_iter`, that is a lane lost
per parked joiner per foreign wave. W-d′ does not change it (it fixes only WHO wakes the joiner for
its OWN scope). Under B0 it is a residual of this pass, recorded here and in `KE16-DESIGN-B.md` §1.
Under B1/B3 it is closed by the worker joiner's idle-marked park (`KE16-DESIGN-B.md` §2.2, rule
B1-P): the joiner parks exactly the way `worker_main` parks — `mark_idle` → post-mark re-poll →
`park_timeout` → `unmark_idle` — so a foreign wave's `claim_one_idle` can claim its bit and unpark
it, and it helps like any worker; the park protocol is the one loom M2 models for a worker, so no
new model is needed.

### 3.6 Cost

Route (b) per completion: one multi-writer RMW (`pending`) instead of two + a conditional syscall;
the last completer of a scope pays one parker swap + at most one syscall. For a physics step
(6 colors × 64–96 chunks) that is ~500 fewer parker-line transfers and up to ~500 fewer syscalls
per step when the joiner parks — which under B1 it does only after exhausting stealable work.

### 3.7 Obligations

**loom M1c** (new in `tests/loom_pool.rs`, `#[cfg(feature = "ke16-w-count")]`): `loom_exports`
gains `pub use crate::sync::WakeHandle` and `LoomScopeShared::new_worker_joined(waker: Thread,
target: *const WakeHandle)`. The model owns `let target = Box::new(WakeHandle::new(thread::
current()))` OUTSIDE the shared block (it outlives the joins), registers N = 2 tasks, spawns two
completers that each pop a toy item and call the REAL gated `complete_task`, and joins with loom's
REAL `park()`: `while !shared.is_drained() { thread::park(); }` — sound under loom 0.7.2 because a
token issued before the park persists (`rt/thread.rs:153-165`, `rt/mod.rs:87-107`; the M1 comment's
"#246" claim is corrected in the same commit, index §7). Assertions: the model terminates (a lost
wake is a loom deadlock report); `target.unparks() == 1` (exactly one wake, which by the `== 1`
gate is the last completer's); `completed == N`.

**If loom nevertheless reports a false deadlock on the park** and the joiner has to be modelled
with the M1 yield re-poll instead: say so in the results file, by name. A yield re-poll joiner
always terminates, so that model cannot observe a lost wake at all; `unparks() == 1` then proves
only that the last completer called `unpark`, not that a parked joiner is woken. In that case the
loom row is recorded as **"M1c-count: green (count only)"**, never as "M1c green", and the
route-(b) many-seeds Miri run below is the SOLE liveness gate for W-d′ (the critic's non-blocking
item 4).

**Miri, the route-(b) liveness gate.** `tests/miri_scope.rs::nested_scope_from_worker_is_stolen_by_
sibling` (`KE16-DESIGN-A.md` §1.3) is built so that its ONLY scope join is the worker's: the test
thread uses `pool.spawn` + an `AtomicBool` spin-wait, never a scope. With `ke16-w-count` that join
is count-gated, and the gate is: `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation
-Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-many-seeds=0..32"` shows ZERO liveness
timeouts on that test. A timeout there is a lost wake in the gated arm — a defect to fix, not a
reason to switch designs (W17 keeps the unpark-before-decrement order and would show the same
timeout). The existing three tests keep their external-arm ~1/16 and are recorded as "expected,
unchanged".

**Native:** a test where a pool-A worker joins a pool-B scope and is B's last completer inline
(exercises the external arm on a cross-pool joiner); `install_on_same_pool_worker_is_external`
(`KE16-DESIGN-APP.md` §5) under `ke16-w-count` — the process must not abort.

## 4. App-4 — batch spawn (G20 / E23): `Scope::spawn_batch`

### 4.1 API

```rust
impl<'scope> Scope<'scope> {
    /// Spawn `n` tasks as one wave: one `pending` RMW for the wave, `n` queue insertions, and the
    /// wake decision taken ONCE, right after the FIRST push (the wave's 0->1 transition), so a
    /// parked sibling is already on its way while the remaining pushes land; the remaining pushes
    /// are silent. `bodies` must yield at most `n` closures (debug-asserted); fewer is allowed and
    /// corrected.
    pub fn spawn_batch<I, F>(&self, n: usize, bodies: I)
    where I: IntoIterator<Item = F>, F: FnOnce() + Send + 'scope;
}
```

Body: `register_tasks(n)` = `pending.fetch_add(n, AcqRel)`; for each body: the wrapper + `Box` +
transmute exactly as `spawn` (`scope.rs:313-362`, factored into `fn prepare(&self, f) ->
TaskHandle` so `spawn` and `spawn_batch` share one wrapper), then `push_task_no_wake(inner, task)`
(the A arm without the trailing wake, returning the pre-push length); **after the first push**: the
wave's wake — `wake_after_push(inner, pre_len_of_first)` (under `ke16-w-fanout`: `wake_up_to(inner,
n)`, §5); the remaining `n − 1` pushes are silent. `k` = bodies pushed; `if k < n {
pending.fetch_sub(n − k, AcqRel) }` (sound: the joiner has not begun joining — same thread — and the
counter is exact; a spurious last-completer wake is harmless); `debug_assert!(k <= n)`.

Why after the FIRST push and not the last (the critic's round-2 non-blocking item 2): a wake after
the last push serialises the spawner's `n × (Box + push)` before any sibling can start — at 1 µs ×
64W that is ~60 µs of publication against a ~64 µs ideal wave, a possible 2× regression. The
transition IS the first push; Tchiboukdjian's fill bound (`[P]`) is about how fast a published pile
is consumed, not about the spawner's own critical path. The saving (N−1 `pending` RMWs and N−1 wake
decisions, each a fence + a load under W-a) is unchanged. The silent pushes are sound under the
weak invariant: the first push carried the fenced wake decision; the later ones land in a queue an
awake thief is already draining or about to (§2.3 item 3 with the 0→1 push being the batch's first).

### 4.2 Callers (the c1 scope — all four production `pool.scope` drivers)

- `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:393-448` (`for_each_impl`): per archetype,
  `n_chunks = entity_count.div_ceil(chunk_size)` (the loop at `:395-448` is exactly that many
  iterations), then `scope.spawn_batch(n_chunks, (0..n_chunks).map(|i| { let start = i *
  chunk_size; let end = ((i+1) * chunk_size).min(entity_count); move || unsafe {
  run_chunk_owned(...) } }))`. One RMW per archetype per wave.
- `crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs:139-260` (`par_for_each_chunk_impl`, the
  second production `par_*` driver — the critic's non-blocking item 3): the same shape, a
  `while start < entity_count` loop at `:243-260` with one `scope.spawn` per `[start, end)`, so
  `n_chunks = entity_count.div_ceil(chunk_size)` per archetype above `MIN_ARCHETYPE_FOR_PARALLEL`
  (`:180`); it calls `spawn_batch` with the same iterator shape as `par_iter`.
- `crates/boyko_physics/src/solver/colored.rs:2667-2700`, `soft/colored.rs:1020-1060`, `resources.rs`
  (`emit_passes`, the `pool.scope` sites at `:1688` and `:1760`): the chunk-cutting loop is a
  deterministic function of the CSR (`group_start`, slot `target`); run it once to COUNT cuts (no
  allocation, O(groups)), then `spawn_batch(count, ...)` re-running the same cut loop as the
  iterator. The count pass is bounded by the color's group count and is far below the solve's
  cost. Bit-identity of the solve is chunk-count- and chunk-shape-independent
  (`colored.rs:2635-2638`), so the `{1, N}` oracle is unchanged.
- **Deliberately per-task:** `crates/boyko_fontbake/src/msdf/distance.rs:433-451` (the MSDF bake:
  an `install` from an application thread, `4 × worker_count` bands, `pick_band_rows` at `:467`).
  It is the external route, tool-time, not on any tournament harness, and its band count is
  computed by the same `while` shape; converting it is a one-line follow-up if c1 ships, recorded
  here so a c1 verdict does not leave a production wave un-batched by accident.

### 4.3 Cost and prediction

Removes N−1 multi-writer `pending` RMWs per wave and N−1 wake decisions (under W-a: N−1 fences +
N−1 `idle` loads; under W-b: N−1 `len()` loads). At W tasks of 1 µs that is 15 × 40–100 ns ≈
0.6–1.5 µs against the 4.82 µs cell. At ≥100 µs bodies: invisible. If no cell improves beyond the
band, the API is not shipped (index §2).

### 4.4 Obligations

Unit tests for k < n, k == n, k > n (debug panic); `scope_multi_drain_frees_once` (`scope.rs:652`)
extended with a batch wave; the physics `{1, N}` bit-identity tests unchanged and green; a unit
test that the wake decision was taken after the first push (a counting shim on `unpark_one_idle`
under `cfg(test)`, or the occupancy test's `max_in_flight` at W=4 with a 1 ms body, which reaches
> 1 only if a sibling started before the spawner's last push).

## 5. W-f — wake fan-out count on a batch push (W16 / E27)

`wake_up_to(inner, k)`, called right after the batch's FIRST push: `publish_fence()` once, then
repeat at most `k` times: `mask = idle.load(Acquire); if mask == 0 { break }`, claim one bit
(`claim_one_idle`, one CAS attempt) and `unpark` it. Cost: up to `min(k, popcount(idle))` CAS +
syscalls on the spawner's critical path, serially, BEFORE its remaining pushes (the pushes are ~50 ns
each, the syscalls ~1–5 µs each on Windows when the target is parked, so the order does not matter
for the woken workers — they arrive after all pushes have landed). Against W-b's chain: W serial
syscalls on the spawner versus 4 hops of scheduler wake latency. Prediction: loses at 1 µs × W (the
syscalls are the wave), may win at 100 µs–1 ms × W where all siblings are parked at the wave
boundary and the chain's latency is paid before any parallelism starts. Built only under
`ke16-c-batch` (the wave size `k` is known at one place only there); measured at W and, if the bench
box allows, at a second W via `ThreadPoolBuilder::num_threads(4)` on the grid. Wicked's
`notify_all` on a Dispatch wave (P24, round 2) is the shipped precedent for a wave-conditioned count.

## 6. Argued away, with the number that would reopen them

| Variant | Why not built | Reopened if |
|---|---|---|
| W-c decoupled spin (W11/W21/E18/E31) | .NET's form spins on a semaphore word that every enqueue posts `[S]`; W-b removes exactly that per-push signal, so W-c here would need a new "work exists" word written on every push — a shared-line write the criterion forbids on the spawn path; the idle scan (≤11 rounds × (W−1) `Stealer` probes) runs between waves, not on any consumer's critical path at W=16; Go #28808 is a 56-core datum | after A1+B1+W-b the physics `in_scheduled_system` still trails its W-lane reference beyond the band AND a profile of the worker loop attributes the gap to `try_steal_random` probes during wave boundaries |
| W-e throttled wake (W15) | a knob (the interval) with a latency floor of one interval per additional waiter; addresses no-op wakes that W-b removes at the source | W-b's grid shows a cell where the residue cascade over-wakes (a worker woken, finds nothing, parks) — visible as a regression at 10 µs × 4W with no gain elsewhere |
| W9 silent worker spawn | one wake per wave is the minimum a wave needs to start; W-b pays at most two; the fully silent form leaves the wave serial until the spawner's join, which for `par_iter` is immediate but for a fire-and-forget spawn is never — and the un-fenced rayon internal push (`KE16-DESIGN-A.md` §1.8) is the measured shape of that loss | never — W-b subsumes it |
| W17 joiner-published flag inside `ScopeShared` | keeps the unpark-before-decrement order and hence the lost-wakeup window of §3.4; it cannot serve as W-d′'s fallback because the route-(b) many-seeds gate would fail on it identically | never as a fallback; only if W-d′'s target-lifetime argument is found unsound (then the whole count gate is out, not replaced by a flag) |
| W2 searcher cap (Go/Tokio); rayon's awake-idle counter | needs a searching/idle state per worker (one more multi-writer counter on the park path) and the compensating wake on the searching→not-searching transition (N14); the ≤1 gate + the residue cascade give the same "no wake when someone will find it" property without the counter | W-b over-wakes measurably (see W-e's condition) |
| A cheaper barrier than `mfence` for `publish_fence` (a `lock`-prefixed RMW on a private stack word; rayon-core 1.13.0 itself uses `fence(SeqCst)`) | Rust has no portable spelling of it; LLVM lowers `fence(SeqCst)` to `mfence` on x86-64; the difference is ~10–20 cycles per wake decision, below the 1 µs cells' band | a profile shows `publish_fence` in the top-3 of the spawn path on a consumer |
| W3 rayon sleepy handshake, W7 wake-all, W8 relay, W10 heartbeat, W12 feedback, W19 WAITPKG; the .NET `TaskReplicator` task cascade (G22) | catalogue reasons; none cheaper than W-b on the spawn path; the replica cascade adds a queue insertion per started replica for a fan-out W-b's residue rule gets from the steals that happen anyway | not by a number on this grid |
