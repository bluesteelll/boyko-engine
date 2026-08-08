# Substrate A2 — `lane`

<!-- CONTRACT
provides: substrate/lane-registry       # the ONE lane topology both subsystems index by
provides: substrate/lane-write-sites    # the THREE set_lane sites, and why the third is load-bearing
assumes:  substrate/crate-graph         # boyko_diag sits BELOW boyko_threadpool; that is why a TLS slot exists
-->

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §3 A2 (in full), §10 Q1 (the shipping
> `LANE_COUNT` blocker, in full) and §11 F1. Diff against that file until the monoliths are
> retired.

**The consequence of not sharing this, in one line:** the same worker is lane 5 to the profiler
and lane 37 to the logger, so no reader can place a log line inside the zone it happened in —
the one joint question the pair exists to answer becomes unanswerable by construction.

---

## API and constants

```rust
pub const LANE_WORKER_MAX: u16 = 64;      // == boyko_threadpool::MAX_WORKERS
pub const LANE_DISPATCHER: u16 = 64;
pub const LANE_HOST:       u16 = 65;
pub const LANE_SPARE_BASE: u16 = 66;
pub const LANE_COUNT:      u16 = 80;   // ALL profiles — Q1 RESOLVED, see below
pub const LANE_UNCLAIMED:  u16 = u16::MAX;

thread_local! { static LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) }; }  // NO Drop

#[inline] pub fn lane() -> u16;
          pub fn set_lane(id: u16);           // boyko_threadpool only, at its 3 sites
#[cold]   pub fn claim_lane() -> Option<u16>;
#[cold]   pub fn release_lane();
          pub fn lanes_leaked() -> u32;       // spares never released; printed in the census
```

One worker therefore carries **ONE integer** in both artifacts. What this file owns is the
topology and its two names, `boyko_diag::lane()` and `LANE_COUNT`. **The renames each subsystem
must make to arrive at them (S11) are that subsystem's own work and are not restated here** — a
bottom-layer file that enumerates its consumers' edits has made itself stale on the next rename
of a type it does not own.

---

## Why a second TLS slot exists at all

The obvious design — derive the lane from `boyko_threadpool::tls::current_worker_id()` and hold
no state — **is impossible**: `boyko_diag` is *below* `boyko_threadpool` and cannot name it.

The slot is therefore written from above, and the divergence risk that creates is closed by
**co-location**: every `set_lane` call sits *immediately beside* an existing
`set_current_worker_id` call, so a future edit that moves one and not the other is visible in a
two-line diff, and gate **DG3** asserts the two agree.

The cost is recorded honestly in [`00-GOAL.md`](00-GOAL.md): a worker thread holds **two** TLS
`Cell`s after D1, the pool's own worker id and `boyko_diag::LANE`.

---

## The write sites — three, not two

The decision record says "at its 2 existing sites". **There are three.** All three verified in
the tree this session:

| Site | Existing call | Lane written |
|---|---|---|
| `crates/boyko_threadpool/src/worker.rs:24` | `tls::set_current_worker_id(worker_id)` in `worker_main` | `worker_id as u16` (dense `0..worker_count`) |
| `crates/boyko_threadpool/src/thread_pool.rs:190` | `tls::set_current_worker_id(tls::WORKER_ID_DISPATCHER)` in `PoolInner::install` | `LANE_DISPATCHER` |
| `crates/boyko_threadpool/src/thread_pool.rs:279` | `tls::set_current_worker_id(id)` in `InstallGuard::drop` | the saved previous lane |

**The third is load-bearing and is the one the record omits.** `InstallGuard::drop` restores on
**both** the normal return and the **unwinding** path; without a lane restore there, a dispatcher
thread that panics inside `install` stays labelled `LANE_DISPATCHER` for the rest of the process
and **every later diagnostic from that thread is misattributed**.

`InstallGuard` already carries `prev_worker_id: Option<u32>` (`thread_pool.rs:272`, set at
`:202`, consumed at `:278`); **the lane rides the same `Option` rather than a fourth field.**

A **fourth latent site**, `tls::clear_current_worker_id` (`tls.rs:101`, `#[allow(dead_code)]` at
`:100`, test teardown only), gets the same treatment for symmetry; **it is not on any production
path.**

The record's *prose* does say "entry/exit", so the design is right and only the count is wrong —
but the count is what an implementer works from, and an implementer who lands two sites ships a
process-lifetime misattribution that no green gate would have caught. **DG3 reds on the missing
one.**

---

## Worker ids are dense by construction

This is why the profiler's topology — not the logger's `hash(thread_id)` scan — is the one that
anchors: **it indexes an index that is already dense and already stable.**

| Fact | Site | Verified |
|---|---|---|
| `pub const MAX_WORKERS: usize = 64` — **unconditional**, no cfg, no feature | `thread_pool.rs:49` | ✔ |
| `let worker_count = requested.clamp(1, MAX_WORKERS)` | `thread_pool.rs:554` | ✔ |
| `requested = available_parallelism()` by default | `thread_pool.rs:684` (doc at `:681`) | ✔ |
| ids are exactly `0..worker_count`, handed out by `deques.into_iter().enumerate()` | `thread_pool.rs:602` | ✔ |
| the entry point asserts the id is in range | `worker.rs:22` | ✔ |

**Two citation corrections against the source plan, verified this session.** The record and the
substrate plan both cite the `enumerate()` at **`:601`**; it is at **`:602`**. *(Second-order
correction, re-read from the tree: `:600` is `let stack_size = self.stack_size;` and **`:601` is
blank** — the earlier form of this parenthetical put the `stack_size` line on `:601` and was
itself off by one, i.e. it named the wrong line while correcting a wrong line.)* And
`worker.rs:22` is
`debug_assert!((worker_id as usize) < inner.workers.len())` — an assertion against the *actual*
worker count, not against `MAX_WORKERS`. That is the stronger statement and it supports the
density claim better than the record's paraphrase does, but an implementer told to look for a
`MAX_WORKERS` comparison at that line will not find one.

---

## The claim path — threads outside the pool

For asset I/O, a script VM, a mod:

```rust
static SPARE_OWNER: [AtomicU32; (LANE_COUNT - LANE_SPARE_BASE) as usize] = /* zero = FREE */;
```

`claim_lane()` scans `LANE_SPARE_BASE..LANE_COUNT` and does
`compare_exchange_weak(FREE, CLAIMED, AcqRel, Acquire)` on each.

**The `Acquire` on success pairs with the `Release` in `release_lane`'s `store(FREE, Release)`**:
it is what guarantees a new claimant of a recycled lane observes the retiring owner's final
writes to that lane's loss cells before it begins writing them itself.
`compare_exchange_weak` is correct here because the scan is a loop anyway.

**On exhaustion the function returns `None`, the caller stays `LANE_UNCLAIMED`, and
`LossClass::Unclaimed` is counted — never a panic, never a block.** Exhaustion is non-terminal.

**Cost, stated:** the scan no longer spreads by thread-id hash, so concurrent claimants
**convoy** on the first free slot — bounded at `LANE_COUNT - LANE_SPARE_BASE` CAS attempts on a
`#[cold]` path taken once per thread. A thread that never calls `release_lane()` holds its spare
for the process; bounded, counted as `lanes_leaked`, printed in the census.

---

## No `Drop`

`LANE` is `Cell<u16>` with a **`const` initialiser**, so the `thread_local!` expansion has no
lazy-init flag and **registers no destructor**.

**This is the mechanism that turns logging's "≤ 1 allocation on first emit" into 0**, and it is
why `release_lane()` is explicit rather than automatic. Reinstating a `Drop`-carrying TLS guard
is the showable red on logging's zero-allocation leg.

---

## `.bss` residency

`SPARE_OWNER` is 14 × 4 B = **56 B** in the dev profile — one line, **packed deliberately**: all
14 words are touched by the same cold scan, so padding them apart would cost 14 lines to prevent
false sharing on a path that runs once per thread.

The `LANE` TLS `Cell` costs 2 B of TLS per thread and no `.bss`.

---

## Q1 — `LANE_COUNT = 32` in the shipping profiles is UNSOUND. **BLOCKER for D0.**

`LANE_COUNT` is a D0 const, so this blocks the rung.

S9's table sets `LANE_COUNT = 80` in `dev`/`editor` and **32** in `shipping`/`shipping-min`, and
S3 justifies the 80 as *"max, not sum: 64 is a hard const, +dispatcher +host +14 claimable
spares"*. **Measured against the tree, the 32 contradicts that same sentence:**

- `pub const MAX_WORKERS: usize = 64` (`thread_pool.rs:49`) is **unconditional** — no cfg, no
  feature, no profile.
- `worker_count = requested.clamp(1, MAX_WORKERS)` (`:554`) with
  `requested = available_parallelism()` by default (`:684`).
- Worker ids are the dense `0..worker_count` (`:602`), and **the lane *is* the worker id**.

So a `shipping` build **on any machine with more than 32 hardware threads produces lane indices
≥ 32 into arrays sized 32**. Independently, **`LANE_HOST = 65` is already out of range at
`LANE_COUNT = 32` on *every* machine.** The floor for the worker-anchored topology is **66**
(64 workers + dispatcher + host) plus whatever spares are kept.

**Where the 32 came from:** it is inherited from logging's `MAX_LANES = 32` shipping value,
which was safe *there* because those lanes were CAS-claimed and **unanchored**. Anchoring the
topology to worker ids is what invalidates it.

Three candidate resolutions were tabled:

| | Resolution | Cost |
|---|---|---|
| **(a)** | `LANE_COUNT = 66` in shipping | zero spares — a non-pool thread then never gets a lane and every diagnostic from it counts as `Unclaimed` |
| **(b)** | `LANE_COUNT = 72` | six spares, ≈ 36 KiB of `LossCell` |
| **(c)** | make `MAX_WORKERS` profile-dependent in `boyko_threadpool` | the only option that recovers the 32, and a change to the pool's **public API** that this plan does not schedule |

### RESOLVED — `LANE_COUNT = 80`, in EVERY profile, with no profile axis at all

**None of the three, and deliberately so: the defect is the profile axis itself.** Q1 exists
because `LANE_COUNT` was made profile-dependent while the quantity it indexes — `MAX_WORKERS = 64`
(`thread_pool.rs:49`) — is unconditional. Any per-profile value re-opens the same class the moment
someone edits one side; a single const closes it by construction.

80 is not a new number: it is the value S3 already justified for `dev`, and its arithmetic is the
topology's own — **64 workers (`0..63`) + dispatcher (64) + host (65) = 66, plus 14 spares = 80.**

**(c) is refused on principle, not on cost.** It caps the SHIPPED engine's worker count at 32 to
save diagnostics memory — inverting the engine's first principle for a subsystem that is OFF by
default. A 64-thread machine would lose half its workers so that a disabled feature could be
smaller.

**(a) is refused on a measured need.** Production spawns **zero** threads outside the pool today
(measured: every `thread::spawn` in `crates/*/src` is under `#[cfg(test)]`), but the corpus
*plans* at least one — `boyko_log`'s sink thread under `SinkMode::Thread` — and OS/driver
callbacks (the Vulkan validation messenger, a window proc) arrive on threads the engine never
created. With zero spares every record from those counts as `Unclaimed`, so the shipping profile
would lose attribution for the logger's own consumer.

**The cost, stated where a reader meets it: 80 lanes × 8 classes × 64 B = 40 KiB of `LossCell`
`.bss`, in every profile.** What makes that affordable in `shipping` is S13, not frugality: an
unclaimed lane is never touched, and an untouched all-zero `.bss` page is reserved address space
rather than resident memory. **The memory argument that motivated the original 32 does not survive
S13** — it was trading correctness for bytes a flag-off process never faults in.
