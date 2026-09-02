# KE16 design — axis B: the joining thread (defect B), in the A-fixed configuration

Part of the KE16 design; index and candidate table in `KE16-DESIGN.md`. Catalogue entries G1–G4,
J1–J14 in `KE16-VARIANTS-GRANULARITY-JOIN.md` and the addenda. Line numbers read at this checkout.
Revision 4: the worker joiner's park is idle-marked — rule B1-P, so a parked joiner is a lane for
foreign waves (§2.2, §2.6, §2.7; the critic's round-3 non-blocking item 7, which is a residual
under B0, §1); `join_on_worker` takes every `&Worker` in its own statement, never as an `if let`
scrutinee (§2.2; item 1); the B0 self-steal under A1 is priced by source flavour (§1); the EVT1
lane interleaving of an inline sibling system is stated (§2.5; item 8); the W-lane reference row
under B3 is `bench_thread_install`, not the W−1 row (§3; blocking item 2). Revision 3: the B0
description under A1 is corrected — the joiner's self-steal into `scratch` is a real batch, not a
pop (§1); `join_on_worker` takes a `Copy` lane holding a raw pointer (§2.2); the residue cascade
excludes the lane's bit (§2.2); the inline-nested-spawn Miri shape is a B obligation (§2.7).

Built: **B0** (default), **B1** (`ke16-b1`), **B3** (`ke16-b3`). Argued away: B2 on the worker
joiner, B1(i) registered scratch, J2 restricted helping (§5). J11 (the frame-path dispatcher parks
and polls, never helps) is preserved unchanged (§4).

## 0. Why B is decided after A, and what "the A-fixed configuration" means

Today the route-(b) joiner (a worker inside a task body, joining the scope it opened) drains its
own `injector_local[wid]` into an unregistered `scratch` (`scope.rs:448`, `:479`) and runs the batch
serially (`drain_scratch`, `:524-531`); since the wave was unreachable anyway, B changes nothing on
that route. After ANY A candidate the wave is reachable and the joiner's first act is to take ≤33
of it into `scratch` (`KE16-DESIGN-A.md` §6): the serial residue is now the production behaviour.
B0/B1/B3 are therefore measured with the Step-A winner in place; the predictions below assume A1
(the expected winner); if A2/A3/A5 win Step A beyond the band, B1 has no registered destination and
the pass stops to re-plan (index §3).

The route-(a) joiner (a non-worker thread) is not bench-only: `crates/boyko_fontbake/src/msdf/
distance.rs:433` installs from an application thread, spawns `4 × worker_count` bands
(`:433-451`, `pick_band_rows` at `:467`) and joins. So the external-joiner policy has a production
consumer, at 100 µs–1 ms bodies × 4W — the grid has that cell.

## 1. B0 — keep as is (G3)

No code. Under A1 the behaviour, read from the code as it will be after the A1 edits:

1. `is_drained()` (`scope.rs:463`) — false while chunks run.
2. stage 1 (`scope.rs:477-485`) is cfg'd out under A1 (no injector to drain).
3. `injector_global.steal_batch_and_pop(&scratch)` (`:488`) — on the frame path this can take a
   batch of other SYSTEMS the dispatcher pushed; runs them serially inside this system's join.
4. `try_steal_any` (`:534-541`): fixed `0..n` sweep, self included — on reaching `stealers[wid]` it
   batch-steals from the joining worker's OWN deque into `scratch`. This is a REAL batch, not a
   pop: crossbeam degrades a steal to `dest.pop()` only when the destination IS the source
   (`Arc::ptr_eq(self.inner, dest.inner)`, `deque.rs:987-991`), and `scratch` is a different deque
   (`scope.rs:448`). The batch is `min((len − 1)/2, 32) + 1 ≤ 33` (`deque.rs:1018`) of its own
   wave — about half of it — into the unregistered `scratch`, run serially by `drain_scratch` with
   no `is_drained` re-check inside the batch; from sibling deques it takes the same. Under LIFO
   deques the front (steal end) holds the OLDEST entries: sibling systems batch-stolen earlier come
   before the joiner's own chunks. (Revision 2 said "ONE task per probe"; that was wrong — the
   critic's non-blocking item 1.) Under `ke16-a1` (LIFO deques) that self-steal is the
   per-element path — one `SeqCst` CAS + one fence per chunk taken (`deque.rs:1077-1142`) — and,
   because `scratch` is FIFO, the reversal loop (`:1145-1156`); under `ke16-a1-fifo` it is one CAS
   per batch (`:1034-1071`). Every sibling steal into `scratch` is priced the same way.
5. park path (`:506-513`): `unpark_one_idle` then Backoff snooze, then `park_timeout(50 µs)` — ≥1 ms
   on Windows. **The parked joiner is NOT idle-marked** (its bit in `inner.idle` stays clear), so
   it is invisible to every OTHER wave's wake decision: a sibling that spawns a wave after this
   joiner's last scan wakes only idle-masked workers, and the joiner sleeps until its own last
   completer or the backstop — one lane lost per parked joiner per foreign wave, on a route where
   16 systems each join their own `par_iter`. Pre-existing, unchanged by W-d′ (which only fixes who
   wakes the joiner for its OWN scope), and a residual of this pass if B0 wins (`KE16-DESIGN-W.md`
   §3.5; the critic's round-3 non-blocking item 7). B1 closes it (§2.2, rule B1-P).

What B0 costs on the consumers, predicted: physics — each of the 6 colors' waves (64–96 chunks at
W=16 after App-1) loses about half of the wave — ≤33 chunks per own-deque grab, and ≤33 per
sibling-deque grab — to a serial run on the spawning worker before siblings (woken at the first
pushes, stealing ≤32 each) have taken the rest; `in_scheduled_system` ends above its W-lane
reference by roughly the serial residue of the widest color per step. Pool grid — `top_lane ≈ 33`
of 64 at 4W, exactly as measured on the dispatcher route today (whose joiner does the same thing
to `injector_global`): the batch, not a one-per-probe pop, is what that number is consistent with.

The case for B0 the owner asked to be reportable: the batch amortises one CAS over ≤33 tasks; the
frame path never enters the helper loop with work left; B0 is zero code. The case against: after
A1 the joiner ALWAYS reaches its own wave, and a 200 µs × 33 residue is 6.6 ms on one lane.
Decided by the Step-B numbers, not here.

## 2. B1 — the worker joiner uses its own registered deque; the external joiner steals one (G4(ii) + J1 + L2)

### 2.1 Structure

`join_workers_until_drained` (`scope.rs:440`) becomes a two-way dispatch on the ONE identity
predicate (`KE16-DESIGN-A.md` §1.1; App-6, `KE16-DESIGN-APP.md` §5):

```rust
// === KE16 B switch: ke16-b1 / ke16-b3 ===
unsafe fn join_workers_until_drained(inner: &PoolInner, shared: *const ScopeShared) {
    match tls::worker_lane_for(inner) {
        // A registered worker of THIS pool, acting as that worker, inside a task body: it owns a
        // deque every sibling can steal. `lane` is `Copy` and holds a raw pointer — no reference
        // field, so nothing here is retagged and protected for the join's duration.
        Some(lane) => join_on_worker(inner, shared, lane),
        // The dispatcher, an unattached thread, a worker of ANOTHER pool, or a worker inside an
        // `install` frame of this pool (its id is the dispatcher sentinel for that frame): no
        // deque to act through in this pool.
        None => join_external(inner, shared),
    }
}
```

`worker_lane_for(inner)` is `Some` exactly when the calling thread is a worker of `inner` acting as
that worker (D1/D2/D4 in `KE16-DESIGN-A.md` §1.7) — that is the identity check the old `on_worker =
wid < len` (`scope.rs:441-442`) lacked (J14): a pool-A worker joining a pool-B scope is external to
B, and a pool-A worker inside `pool_a.install` is external to A for that frame (`CURRENT_WORKER_ID`
is `WORKER_ID_DISPATCHER` there, `thread_pool.rs:205`, so `lane.wid` can never be the sentinel and
never indexes `inner.workers` out of bounds).

### 2.2 The worker joiner (rayon `wait_until_cold` shape: local first, then steal, one task per check)

```rust
fn join_on_worker(inner: &PoolInner, shared: *const ScopeShared, lane: tls::WorkerLane) {
    let wid = lane.wid;                                       // < worker_count by the predicate
    let self_bit = 1u64 << wid;
    let mut rng = XorShift64Star::new(splitmix64((wid as u64) ^ (shared as usize as u64)));
    let backoff = Backoff::new();
    loop {
        #[cfg(miri)] std::thread::yield_now();
        // SAFETY: as today (`scope.rs:460-463`): a transient Acquire load through a pointer the
        //   caller keeps live for the whole call.
        if unsafe { (*shared).is_drained() } { return; }
        // 1. Own deque, owner end (LIFO under ke16-a1: the newest chunk of the wave being joined).
        //    D5: the `&Worker` is consumed by `pop()` in THIS statement; the body runs in the next
        //    one. (Not an `if let` scrutinee: in Rust 2024 its temporaries live through the THEN
        //    block, and the SAFETY argument must match the syntax.)
        let popped = lane.deque().pop();
        if let Some(t) = popped { crate::worker::run_task(t); backoff.reset(); continue; }
        // 2. Global injector, batch into the OWN REGISTERED deque: the residue is re-stealable.
        //    The helper takes `&Worker` for its own duration only and runs no task body.
        let popped = crate::worker::pop_global_injector(inner, wid, lane.deque());
        if let Some(t) = popped { crate::worker::run_task(t); backoff.reset(); continue; }
        // 3. Random-start, self-skipped sibling sweep (App-3), batch into the own deque.
        let popped = crate::worker::try_steal_random(inner, wid, lane.deque(), &mut rng);
        if let Some(t) = popped { crate::worker::run_task(t); backoff.reset(); continue; }
        // 4. Nothing stealable. Snooze first; once the backoff is spent, park THE WAY `worker_main`
        //    PARKS (worker.rs:98-121) — rule B1-P: mark idle, re-poll once more with the bit set
        //    (Race C, the same reason as worker.rs:101-108), park with the backstop, unmark. While
        //    parked this thread is a claimable lane for any OTHER wave's wake decision; the last
        //    completer of ITS OWN scope unparks the same handle (W-d'). Both wakes land on one
        //    parker: a spurious return re-checks `is_drained` and re-scans, harmless.
        if backoff.is_completed() {
            crate::worker::mark_idle(&inner.idle, wid);
            let popped = crate::worker::pop_any(inner, wid, lane.deque(), &mut rng);
            if let Some(t) = popped {
                crate::worker::unmark_idle(&inner.idle, wid);
                crate::worker::run_task(t); backoff.reset(); continue;
            }
            // Re-check the scope with the bit set: the last completer's unpark (W-d') may have
            // landed between step 3 and mark_idle; parking now would sleep on a stale token only
            // until the backstop, but the check is one Acquire load and removes even that.
            if unsafe { (*shared).is_drained() } { crate::worker::unmark_idle(&inner.idle, wid); return; }
            unpark_one_idle_excluding(inner, self_bit);       // hand visible-but-unclaimed work to a SIBLING
            std::thread::park_timeout(JOIN_BACKSTOP);
            crate::worker::unmark_idle(&inner.idle, wid);
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}
```

`pop_global_injector` (gaining a `wid` parameter for the W-b cascade's self-exclusion,
`KE16-DESIGN-W.md` §2.2), `try_steal_random`, `pop_any`, `mark_idle`, `unmark_idle`, `splitmix64`
become `pub(crate)` in `worker.rs` (`XorShift64Star` already is; `mark_idle`/`unmark_idle` already
are); `drain_scratch` and `try_steal_any` are deleted under B1; `scratch` is gone from the worker
arm. Every task still runs through `run_task` (the 2026-07 audit's abort-on-fire-and-forget-panic
policy, `worker.rs:129-160`, `scope.rs:467-476`) — unchanged.

**No `&Worker` spans a task body** (D5, `KE16-DESIGN-A.md` §1.3). `lane` is a `Copy` value with a
raw pointer; each `lane.deque()` above is a fresh shared reborrow consumed by one call in its own
statement (`let popped = …;` — the `if let` form would keep the temporary alive through the THEN
block in Rust 2024, which is not UB but is not what the SAFETY text argues, so the code does not
use it); the two helpers hold their `local: &Worker` argument only while they run, and they run no
body. Inside a body this joiner runs inline — a `par_iter` of a sibling system, a nested physics
scope — the body's own `push_task` reaches the same deque through its own `worker_lane_for` +
`lane.deque()`, and at that moment the joiner holds no reference to the deque at all. That is what
the second Miri shape of `KE16-DESIGN-A.md` §1.3 exercises (§2.7).

**Rule B1-P — the parked joiner is idle-marked (new in revision 4; J15).** Step 4 above is
`worker_main`'s own park sequence (`worker.rs:98-121`: `mark_idle` → post-mark re-poll → park →
`unmark_idle`) with a timed park and two additions that are the joiner's, not the worker's: the
`is_drained` re-check with the bit set, and the pre-park `unpark_one_idle_excluding(self_bit)`
that today's joiner already issues (`scope.rs:511`). What it buys: a joiner parked inside system
S's `par_iter` join is a CLAIMABLE lane for any other wave — a sibling system's `par_iter`, a
physics color spawned elsewhere — through the same `claim_one_idle` every wake decision uses;
without it (today, B0) the parked joiner's bit is clear and it sleeps through foreign waves until
its own last completer or the ≥1 ms backstop (§1 item 5). What it costs: one `fetch_or` + one
`fetch_and` on the `idle` line per PARK, never per task; a joiner parks only after a full scan
found nothing. Invariants it keeps: (i) the worker-side W-b-1 argument (`KE16-DESIGN-W.md` §2.3
items 1–3) applies verbatim — the joiner's `fetch_or` is bracketed by two full scans and the
steal-path fence, exactly as a worker's; (ii) a joiner never parks with a non-empty own deque
(step 1 ran first; only the owner pushes to it, and the owner is the joiner); (iii) a claimed bit
is always unparked (A5-1) — the claimer unparks `inner.workers[wid].thread`, which IS this thread;
(iv) `ThreadPool::drop`'s unpark-all reaches a parked joiner as it reaches a parked worker, and the
joiner does not check `shutdown` because a thread inside a task body cannot exit — it continues
its join, which is today's behaviour for a scope open at drop (a misuse guarded by `active_scopes`).
Obligation: none new in kind — loom M2's parked-thread role IS this sequence (`mark_idle` →
re-poll → real `park`), so M2 with the production `publish_fence` covers it; a native receipt test
is in §2.7.

Properties, each of which B0 lacks:

- **No sink.** The only destination of a batch is a registered deque (`inner.stealers[wid]`); the
  residue is stealable by every sibling and by external joiners. This is crossbeam's own contract
  (`steal_batch_and_pop(dest: &Worker<T>)`) and every steal-half runtime's shape (G2).
- **Re-check per task.** `is_drained` is consulted between every task; the joiner stops helping the
  instant its scope completes (B0 drained a whole residue first).
- **Own scope first under LIFO.** Step 1 pops the joining worker's newest entries — its own chunks
  — before anything batch-stolen earlier; the FIFO row (A1-fifo) inverts this and is measured.
- **The worker loop's victim discipline** (random start, self skipped, `worker.rs:243-250`) replaces
  the joiner's fixed `0..n` self-including sweep (`scope.rs:533-541`) — App-3, delivered here.
- **The wake protocol is shared code.** Under `ke16-w-gate` the thief-residue cascade lives inside
  `pop_global_injector`/`try_steal_random`, excludes the caller's bit, and applies to the joiner for
  free (`KE16-DESIGN-W.md` §2); every wake decision passes through the fenced prologue
  (`KE16-DESIGN-W.md` §1).
- **The self-skip is by the predicate's `wid`**, never by `current_worker_id()` read separately, so
  the sweep's self is the lane whose deque is being fed (the critic's note that a `DISPATCHER` self
  is "never skipped" and only harmless through crossbeam's `ptr_eq` degradation cannot arise: the
  external arm has no deque and the worker arm has a real id).
- **A parked joiner is a lane** (B1-P, below): its idle bit is set while it parks, so a foreign
  wave's wake decision can claim it; under B0 a parked joiner is invisible until its own scope
  drains or the ≥1 ms backstop (§1 item 5).

### 2.3 The external joiner under B1 (B2's shape, where it is the only registry-free option)

```rust
fn join_external(inner: &PoolInner, shared: *const ScopeShared) {
    let mut rng = XorShift64Star::new(splitmix64(shared as usize as u64));
    let backoff = Backoff::new();
    loop {
        #[cfg(miri)] std::thread::yield_now();
        if unsafe { (*shared).is_drained() } { return; }
        // One task at a time: an external thread has no registered deque, so a batch would have to
        // land somewhere unregistered (B0's sink) or in a registry that does not exist (B1(i)).
        if let Some(t) = drain_one(|| inner.injector_global.steal()) { run_task(t); backoff.reset(); continue; }
        if let Some(t) = steal_one_random(inner, &mut rng) { run_task(t); backoff.reset(); continue; }
        if backoff.is_completed() { unpark_one_idle(inner); std::thread::park_timeout(JOIN_BACKSTOP); backoff.reset(); }
        else { backoff.snooze(); }
    }
}
```

`steal_one_random` is `try_steal_random`'s sweep with `Stealer::steal()` (one task, one CAS, one
epoch pin per successful probe, `deque.rs:637-679`) and no self-skip (an external thread has no
self). Under A2 the sweep would also probe `injector_local[idx].steal()`; B1 does not exist under
A2, but the helper is written with the same `#[cfg(feature = "ke16-a2")]` second probe as
`try_steal_random` so the code has one shape.

Cost on the external route: one CAS per task on the joiner (rayon's helper, J1); no residue, no
sink. Route (a) is the bench-only route plus the fontbake bake; the physics and ECS consumers never
take it.

### 2.4 What `scope.rs:1-21` becomes

The module doc's "Work-stealing wait" section describes B1: the joiner helps by popping its own
deque and by stealing into it; an external joiner steals one task at a time; nested scopes cannot
deadlock because a joiner always either runs a ready task or parks with a guaranteed wake (W-d′ on
route (b)) or the backstop. The paragraph `:17-21` ("not accessible here") is deleted.

### 2.5 What B1 makes more likely, and the App-8 dependency

A worker joining a `par_iter` scope inside system S can now pop, from its own deque, a sibling
system S′ that the dispatcher pushed to `injector_global` and this worker batch-stole earlier — and
run S′ INLINE inside S's body. This is reachable TODAY through the B0 joiner's `injector_global`
drain (`scope.rs:488`) and is sound: S and S′ are co-dispatched, hence conflict-free
(`schedule.rs:996-1011`); S′'s completion push and `pending_fetch_add` (`schedule.rs:1310-1316`)
happen inside S's frame and the apply-window gate `pending == running.count_ones()`
(`schedule.rs:600-602`) still fires only when both have completed; deferred commands are applied at
the drain as before; change ticks were stamped at dispatch. What breaks is one debug assertion:
`InSystemRunGuard::enter` (`tls.rs:189-195`) asserts no nesting — **App-8** (`KE16-DESIGN-APP.md`
§7) turns the flag into a depth counter. Every consumer of `is_in_system_run()` is a boolean
predicate (`time.rs:183`, `ecs_master.rs:677`, `event_writer.rs:113,157`, `event_reader.rs:111,
176,192`, `profiling/store.rs:749`, `fold.rs:72`) and reads `depth > 0` unchanged.

A diagnostics caveat, not a soundness one: two `SystemSpan`s (`zones.rs:151-204`) may then overlap
on one lane; each is an independent value and records its own interval, so nothing is lost, but an
overlap analysis that assumes one system per lane at a time sees the nesting. Recorded for the
diagnostics owner; owner question 5 in the index.

An event-lane fact of the same kind (the critic's round-3 non-blocking item 8): EVT1's "single
writer per lane" (`crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs:269-272`, and the
`Send + Sync` SAFETY block at `:524-536`) is per THREAD — the lane index comes from
`current_worker_id_or_dispatcher_lane` (`tls.rs:69-78`), which does not change when a joiner runs
S′ inline. So S′'s `send_event` calls append to S's lane, SEQUENTIALLY on one thread (sound: still
one writer per lane at any instant, which is all EVT1/EVT3 claim), but the two systems' events
INTERLEAVE within that lane: a reader that assumed per-system contiguity within a lane would see
S's events, then S′'s, then S's again. Nothing in the tree asserts that contiguity (the readers at
`event_reader.rs:111,176,192` iterate lanes as opaque sequences), and the guarantee EVT1 gives —
one writer per lane — is unchanged. Stated beside the `SystemSpan` overlap so the two per-lane
caveats travel together; the doc comment at `event_dispatcher.rs:269-272` gains one sentence.

The J1 hazard of unbounded helper depth (a chain of nested joins each helping) is unchanged from
today (rayon has it; Epic's N22 list applies) and is bounded in practice by the ECS's one scope per
`par_iter`; the depth counter's `debug_assert!(depth < 64)` makes a runaway visible.

### 2.6 Deadlock argument (unchanged in kind from today, restated for the new shape)

A joiner waits only for its own scope's `pending` to reach zero. While waiting it either (a) runs a
ready task, which completes unless it opens a nested scope — whose join helps in the same way — or
(b) parks with a wake guaranteed by the last completer (W-d′, route (b)) or by the backstop — and,
under B1-P, additionally claimable by any wave's wake decision, which can only make it wake
EARLIER. A ready task never waits on the joiner. Hence every wait is on tasks that are running or
runnable by someone who is awake or will be woken; no cycle. The B0-specific hazard
`tests/shutdown.rs:23-30` documents (a blocking task in a serial batch wedging its siblings)
disappears with the batch; a task that blocks on a cross-task primitive inside a scope remains a
misuse (as in rayon). B1-P adds no wait: a claimed joiner that finds its scope drained on wake
returns; one that finds foreign work runs it, which is case (a).

### 2.7 Obligations (index §8)

`nested_scope_does_not_deadlock` (`scope.rs:605-628`), `tests/stress.rs`, `tests/shutdown.rs`, BOTH
Miri shapes of `KE16-DESIGN-A.md` §1.3 — `nested_scope_from_worker_is_stolen_by_sibling` and
`nested_scope_inline_body_spawns_through_tls_deque_under_live_join` (the second is the B1-specific
one: a body the joiner runs inline opens a nested scope and pushes ≥129 tasks through the TLS
deque, forcing a `resize`, while the outer join is live on the same thread; run with B0 and with
B1; its receipt has a third red kind, `KE16-DESIGN-A.md` §1.3) — the App-8 ECS nested-system test,
the `install_on_same_pool_worker_is_external` row, and the two red-first gates un-ignored.

**B1-P receipt (native, new in `tests/ke16_nested_scope_occupancy.rs`):**
`parked_joiner_is_claimed_by_a_foreign_wave` — W = 2. The test thread `pool.spawn`s an OUTER task
(so it runs on a worker, X; the U1 rule). X's outer body opens `pool.scope`, spawns ONE body
(`set started; spin until F; record current_worker_id`) and, still inside the scope closure,
spin-waits on `started` BEFORE returning — so the body is running on the sibling Y when X's join
begins (Y was claimed and unparked by the first push's wake decision, and it steals the body from
the front: the joiner never pops it itself). X's join then finds nothing (own deque empty, global
empty, Y's deque empty — Y is inside the body) and, under B1-P, marks its idle bit and parks. The
test thread polls `pool.parked_mask()` — a new public read-only diagnostics accessor, one
`Acquire` load of `inner.idle`, also useful to the occupancy harness — until X's bit is set, then
`pool.spawn`s a FOREIGN task (`record current_worker_id; set F`). Y is busy, so Y's bit is clear,
and the only claimable bit is X's: `push_task` → `injector_global` → `claim_one_idle` → X. Assert
that the foreign task's receipt is X — the parked joiner was claimed, woke, unmarked, scanned,
found the foreign task in `injector_global` and ran it INLINE inside its join — and that the scope
then drains (F releases Y's body). A bounded spin (2 s) on the mask guards the test against a
regression where X never marks; under B0 the test cannot pass (X's bit is never set) and it is
compiled only with `ke16-b1`/`ke16-b3`. A RECEIPT of B1-P, not a tournament number.

## 3. B3 — the external joiner parks and never helps (J3, rayon `in_worker_cold`)

`ke16-b3` = B1's worker arm + this external arm:

```rust
fn join_external(inner: &PoolInner, shared: *const ScopeShared) {
    let backoff = Backoff::new();
    loop {
        #[cfg(miri)] std::thread::yield_now();
        if unsafe { (*shared).is_drained() } { return; }
        // The snooze is KEPT (the critic's round-2 non-blocking item 1): on the frame path the
        // dispatcher reaches this loop a few ns before the last pool decrement (§4). With the
        // external arm's unpark-before-decrement order, a check(false) -> park (consumes the
        // pending token, returns at once) -> re-check (decrement not yet landed) -> park sequence
        // would have no wake left and sleep the whole >=1 ms Windows backstop, once per frame that
        // hits the window. The snooze rounds (a few µs) outlast the ns window, so the second check
        // sees the decrement.
        if backoff.is_completed() {
            // A sibling that pushed a wave may not have raced through its wake yet; the work IS
            // visible — let a parked worker take it.
            unpark_one_idle(inner);
            std::thread::park_timeout(JOIN_BACKSTOP);
            backoff.reset();
        } else {
            backoff.snooze();
        }
    }
}
```

What it changes: the dispatcher route loses its helper lane (≤ 1/(W+1) of the wall-clock at
4W–64W); on a box where W equals the hardware thread count the helper was competing with workers,
so the loss can be smaller than 1/(W+1) or a gain. The fontbake bake is the production consumer of
this arm (§0). Route (b) — physics, ECS — is unaffected by construction. The frame-path dispatcher
(§4) is unaffected beyond the window described there.

Why it is worth a row despite the predicted loss: it deletes `steal_one_random` and the external
helper's whole loop, and an external joiner that never runs tasks removes one class of J1 hazard
(the fontbake thread running fire-and-forget tasks under the abort policy). If the dispatcher rows
are within the band of B1's, B3 wins on code size (owner question 2 decides whether the bake's lane
is a value).

**What B3 does to the physics reference rows (revision 4; the critic's round-3 blocking item 2).**
`bench_thread_install_Wminus1` is a `num_threads(W − 1)` pool PLUS the bench thread's external
joiner. Under B0/B1 that joiner helps, so the row is W lanes — the intended W-lane reference. Under
B3 the external joiner parks, so the SAME row becomes W − 1 lanes, while `bench_thread_install`
(W workers, the joiner parked) becomes exactly W lanes. The two rows therefore do NOT coincide
under B3; they differ by one lane in the direction that would LOOSEN the acceptance line by up to
1/(W−1) ≈ 6.7 % at W=16 if the W−1 row were kept as the reference. The primary reference is
selected by LANE COUNT, not by row name: `bench_thread_install_Wminus1` when the shipped B keeps
the external helper (B0, B1), `bench_thread_install` when it parks (B3) — `KE16-DESIGN-APP.md` §11,
`KE16-DESIGN-MEASUREMENT.md` §7. Under B3 the ratio `Wminus1 / bench_thread_install ≈ W/(W−1)` is
itself a receipt that the external joiner parked.

Under W-d′ the external joiner keeps today's unconditional unpark-before-decrement (the target's
lifetime is inside `ScopeShared`), so every completion wakes it and it re-checks; the park is
therefore not the ≥1 ms backstop in practice but one wake per completion — the cost today's route
(a) already pays — and the lost-wakeup window of that order persists on this arm (`KE16-DESIGN-W.md`
§3.5), masked by the snooze on the frame path.

## 4. J11 — the frame-path dispatcher is preserved

`Schedule::run` installs once (`schedule.rs:412`), the executor loop parks on `PARK_TIMEOUT` (100 µs
at `:69`, ≥1 ms on Windows) between rounds (`:683`), returns when every system completed
(`:645-647`), and only then does the frame scope drop (`thread_pool.rs:238`). Nothing on axis B
touches `executor_main_loop`. One precision the catalogue's "returns on its first `is_drained()`"
lacks: a system's worker pushes its ECS completion and increments the ECS `pending`
(`schedule.rs:1310-1316`) BEFORE the pool's `complete_task` runs (the `Scope::spawn` wrapper,
`scope.rs:335`, runs after the closure returns), so the dispatcher can observe the ECS gate, drain,
finish the loop and reach `Scope::drop` while the last pool decrement is a few nanoseconds away. It
then enters `join_external` for that window: under B0/B1 it probes empty queues and snoozes; under
B3 it snoozes (§3) — and only a park that outlasts the decrement is woken by the (unconditional,
external-arm) `complete_task` unpark, whose token may already have been consumed by an earlier
park in the same window (the external-arm window, `KE16-DESIGN-W.md` §3.5). The snooze makes that
sequence need a ≥ several-µs stall between the ECS completion push and the pool decrement — two
adjacent lines in the same closure — so it is not expected to occur; the physics
`in_scheduled_system` MAD flag (`KE16-DESIGN-MEASUREMENT.md` §4) is where a ≥1 ms outlier would
show if it did. No frame-path behaviour changes beyond that window, and no candidate makes the
dispatcher a lane during a frame.

The executor's wake for intermediate system completions is the same `waker.unpark()` in
`complete_task` (`schedule.rs:665-671` comment); it stays unconditional for external joiners, so
W-d′ does not lengthen the apply window (`KE16-DESIGN-W.md` §3.5).

## 5. Argued away

| Variant | Why | Reopened if |
|---|---|---|
| B2 on the worker joiner (G1) | under A1 the worker joiner owns a registered deque, so a batch into it is not a sink; steal-one only adds a CAS per stolen task (Dinan: steal-one degrades; HotSLAW `[P]`); its one property B1 lacks — never holding more than one foreign task — is not a throughput property | B1's dispatcher-route rows show contention that a per-task CAS would reduce (no mechanism for that is known) |
| B1(i) registered scratch (G4(i)) | a stack-lived deque whose `Stealer` must be published into `inner.stealers` for the join's duration and withdrawn before the frame returns, while a sibling may be mid-`steal_batch_and_pop` on it: a hazard-pointer or epoch protocol on the registry, i.e. one more shared structure on every steal; the external arm of B1 gets "no sink" with none of it | A2/A3/A5 wins Step A beyond the band (then the worker joiner also has no registered deque) — a re-plan point, not a fallback taken silently |
| J2 restricted helping (theft chain, isolation, libomp TSC, own barrier) | strictly less reachable work by design (Lace: 20× vs 36× with random stealing added `[P]`); libomp's default-on Task Scheduling Constraint (round 2) restricts thieves too and exists for tied-task semantics we do not have; the TLS-corruption hazard it cures does not exist here (App-8 shows the only nesting-sensitive state is a boolean) | a consumer shows priority inversion the LIFO own-first order does not fix |
| J4–J13 | core handoff, compensation, fibers, deque suspension, continuations, retraction, wait-free counters, scheduler-observed or time-gated blocking — not applicable to a CPU-bound fixed pool, or not buildable (catalogue; the axis-32 user-space block-notification cell is a triple abandonment, N59–N61) | not by a number on this grid |

## 6. The cross-pool joiner (J14 / App-6 interplay)

A pool-A worker that opens a scope on pool B (holding a B handle) and joins is, under §2.1, EXTERNAL
to B: it never touches A's deque from inside B's join (rayon's `in_worker_cross` helps in A instead;
this design does not — running A's tasks inside a B join would make B's scope wait on A's
throughput, and no production caller does this). Under B1 it helps B one task at a time; under B3
it parks. Its tasks land in B's `injector_global` (today's arm, `worker.rs:373`, kept under every A).
Test: `tests/cross_pool_routing.rs` join-side row (index §8).
