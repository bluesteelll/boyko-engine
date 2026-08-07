# Architecture: Diagnostics Substrate (`boyko_diag`) — rev 1

**Status:** design, pre-implementation. **Target file:** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`.

**Provenance.** This document implements the architect's seam decision record answering the
round-3 seam review of `docs/PROFILING-SYSTEM-PLAN.md` × `docs/LOGGING-SYSTEM-PLAN.md`
(verdict: INCOMPATIBLE AS WRITTEN, findings S1–S12). The decision record is the approved
design; this document is its implementable form. It does not re-open any decision. Where a
statement in the record did not survive verification against the tree, the divergence is
recorded in §11 and the tree wins — a plan that contradicts the manifests is a plan that reds
on its own first rung.

**Scope.** ONE new crate, `crates/boyko_diag`, and ONE new edge into `boyko_threadpool`. Two
rungs: **D0** (the crate) and **D1** (the lane write sites). Everything downstream of D1 —
`boyko_log`, `profiling_abi`'s move, the 17 `W92xx` rows, the five build profiles — belongs to
the two subsystem plans and is referenced here, never restated.

**This crate's whole value is that it is small.** A shared bottom crate that accretes is the
same Principle-0 defect as two subsystems each minting their own copy, pointed the other way.
§4's growth rule is load-bearing and is quoted verbatim from the decision record.

---

## 1. Goal, and why this crate exists at all

`boyko_diag` **removes** four duplications before they are written. It adds no capability that
either subsystem could not have built for itself; it makes it impossible for them to build it
twice and disagree.

| Primitive | If each subsystem keeps its own copy |
|---|---|
| **A1 clock** | A suspend/resume produces a profiler window quarantined as `EpochBreak` and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration with no marker — two artifacts that disagree, neither of which says why. |
| **A2 lane** | The same worker is lane 5 to the profiler and lane 37 to the logger, so no reader can place a log line inside the zone it happened in — the one joint question the pair exists to answer is unanswerable by construction. |
| **A3 loss** | The profiler reports its own drops *through* the logger, so under load — precisely when profiler drops occur — the report of the loss is dropped and counted as a *logger* loss; two counters double-count one event and no rule says which is authoritative. |
| **A4 storage** | Two residency proofs over two statics with two demand-zero arguments, and the toolchain behaviour both admit is unproven (PE/COFF `.bss` placement) gets proved twice — so a toolchain change reds one gate and not the other, and the reader cannot tell which is authoritative. |

### The honest quantitative statement

From the decision record's joint cost table:

| | Profiling alone | Logging alone | Naive sum | With `boyko_diag` |
|---|---|---|---|---|
| **Total, dev** | 6.65 MiB | 3.46 MiB | 10.11 MiB | **9.33 MiB** (−0.78 MiB, −7.7 %) |
| **Total, shipping** | 0.85 MiB | 1.16 MiB | 1.95 MiB | **1.95 MiB** (**zero saving**) |
| TLS slots (diagnostics) | 1 | 1 (+`Drop`) | 2 | **1**, no `Drop` |
| `rdtsc` per {zone + log} | 2 | 1 | 3 | **3** — sharing does not reduce it |
| Allocations, first emit | 0 | ≤ 1 | ≤ 1 | **0** |
| Boot calibrations | 1 (20 ms) | 1 probe | 2 | **1** |

**The substrate saves 0.78 MiB in dev and zero bytes in shipping.** It is bought for
correctness — one lane number, one epoch, a loss report that cannot itself be dropped — not
for footprint. Neither subsystem plan may claim otherwise, and no rung of this plan is
justified by a byte count.

Two qualifications this document adds, because the table is otherwise read as stronger than it
is:

- The "1 TLS slot" row counts *diagnostics* slots. `boyko_threadpool::tls::CURRENT_WORKER_ID`
  is untouched and remains, so a worker thread holds **two** TLS `Cell`s after D1: the pool's
  own worker id and `boyko_diag::LANE`. The second exists only because `boyko_diag` sits
  *below* `boyko_threadpool` and therefore cannot call `current_worker_id()` (§3 A2).
- `boyko_diag`'s own `.bss` (≈ 42 KiB dev, §3) must be attributed to its row exactly once. The
  joint table already counts those bytes inside the two subsystems' rows; double-counting them
  would manufacture a footprint regression out of a move.

---

## 2. Crate graph

`->` = a `[dependencies]` edge. **Every edge below was re-read from the real `Cargo.toml` this
session**; deltas against the decision record are in §11.

```
NEW BOTTOM
  boyko_diag        -> {}                          std only; zero workspace, zero third-party
  boyko_log         -> boyko_diag, boyko_macros    (logging plan; not this document)

UNCHANGED LEAVES (no new edge)
  boyko_utils       -> {}                          <- STAYS ZERO-DEP (Cargo.toml:6-7, empty)
  boyko_math        -> {}
  boyko_shaderdsl   -> {}
  boyko_macros      -> syn, quote, proc-macro2
  boyko_sdf_math    -> boyko_shaderdsl             no_std; cannot and does not take boyko_diag
  boyko_image       -> {}                          + boyko_log at L8b (logging plan)
  boyko_fontbake    -> boyko_math, boyko_threadpool, ttf-parser

EXISTING + NEW EDGES (new marked +)
  boyko_threadpool  -> crossbeam-deque, crossbeam-utils, [cfg(loom)] loom,
                       +boyko_diag                 <- THE ONLY EDGE THIS PLAN ADDS (D1)
  boyko_rhi         -> boyko_utils
  boyko_rhi_vulkan  -> boyko_rhi, boyko_sdf_math, [cfg(windows)] windows-sys
  boyko_ecs         -> boyko_utils, boyko_threadpool, fixedbitset, crossbeam-queue,
                       crossbeam-utils, static_assertions, [cfg(unix)] libc, [cfg(loom)] loom
  boyko_input       -> boyko_ecs, boyko_utils, boyko_macros, [cfg(windows)] windows-sys
  boyko_scene       -> boyko_ecs, boyko_math, boyko_macros, boyko_input
  boyko_physics     -> boyko_ecs, boyko_macros, boyko_utils, boyko_threadpool,
                       boyko_sdf_math, boyko_math, boyko_scene
  boyko_serialize   -> boyko_ecs
  boyko_render      -> boyko_ecs, boyko_macros, boyko_rhi, boyko_rhi_vulkan, boyko_sdf_math,
                       boyko_scene, boyko_math, boyko_fontbake, boyko_image, bytemuck
  boyko_ui          -> boyko_ecs, boyko_macros, boyko_input, boyko_fontbake, boyko_scene,
                       boyko_math
  boyko_app         -> boyko_ecs, boyko_scene, boyko_render, boyko_rhi, boyko_rhi_vulkan,
                       boyko_input, boyko_math, boyko_sdf_math, boyko_macros
  boyko_demo        -> boyko_ecs, boyko_macros, boyko_threadpool, eframe, bytemuck,
                       log 0.4, rand, [not wasm] env_logger, [wasm] console_log + 4 more
  bench_bevy_vs_boyko -> mimalloc(opt); dev-deps: boyko_ecs, boyko_macros, boyko_threadpool,
                       bevy_ecs, criterion
  boyko-engine (root package, `.`) -> {}
```

**Acyclicity proof.** `boyko_diag` has out-degree 0 — it names no workspace crate and no
third-party crate, so no path leaves it and no cycle can pass through it. The single new edge
`boyko_threadpool -> boyko_diag` therefore cannot close a cycle: a cycle through that edge
would require a path `boyko_diag ->* boyko_threadpool`, and `boyko_diag` has no out-edges at
all. The same argument covers every later `X -> boyko_diag` edge the two subsystem plans add.
`boyko_log`'s only out-edges are `boyko_diag` (out-degree 0) and `boyko_macros` (out-edges
`syn`/`quote`/`proc-macro2`, all external), so no workspace crate is reachable from
`boyko_log` either.

**The profiling plan's two planned edges `boyko_rhi_vulkan -> boyko_utils` and
`boyko_threadpool -> boyko_utils` are withdrawn** and replaced by `-> boyko_diag`. At the D0/D1
scope only the `boyko_threadpool` one lands; `boyko_rhi_vulkan`'s arrives with profiling
rung P1.

**Two file-level consequences.**

1. `crates/boyko_image/Cargo.toml:5` — the package description reads "…Decoupled leaf utility
   crate — **no dependency on any other workspace crate**, mirrors boyko_utils' role for image
   data." Verified present and verified false the moment L8b adds `boyko_log`. The description
   is edited in the same commit as the edge, by the logging plan, not here.
2. `crates/boyko_rhi_vulkan/Cargo.toml` — its INVIOLABLE no-third-party rationale is at
   **`:7-12`** (not `:44-49`, which is the `boyko_sdf_math` *precedent* block at `:44-49`). The
   `boyko_diag` row is added at `:7-12` citing the `:44-49` precedent: an in-house, zero-dep
   sibling workspace leaf does not breach "no ash / vulkano / windows-sys / libc".

**Not an edge, and must not become one:** `boyko_utils` does not gain `boyko_log`. The logging
plan's Decision 15 sentence "`boyko_utils` depends on `boyko_log`, not the reverse" is struck.
Nothing in `boyko_utils` logs; its four modules are `bit_mask`, `identifiers`, `sparse_map`,
`type_intern`.

---

## 3. Contents

Two layers. **Layer A** is shared — both subsystems write it. **Layer B** is `profiling_abi`,
which is profiling-only and lives here for the graph reason in §5, not because it is shared.

Crate-level attributes on `crates/boyko_diag/src/lib.rs`:

```rust
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]
#![deny(missing_docs)]
```

The three `print` lints are the mute-leaf rule's mechanical half (§6): they ride the existing
`cargo clippy --all-targets -- -D warnings` gate, so no new CI job is needed.

### A1 — `clock` (+ session identity)

```rust
pub struct SessionId(pub u64, pub u64);          // 128-bit, one id for both artifact headers

#[inline] pub fn ticks() -> u64;                 // rdtsc on x86_64; Instant delta elsewhere
#[inline] pub fn ticks_per_ns() -> f64;          // published once by calibrate()
#[inline] pub fn clock_epoch() -> u32;           // bumped on a detected discontinuity
#[cold]   pub fn calibrate();                    // 16 probes / 20 ms; idempotent
#[cold]   pub fn note_forward_jump(observed: u64);
          pub fn invariant_tsc() -> bool;        // CPUID.80000007H:EDX[8], probed once
          pub fn session_id() -> SessionId;      // minted once at first touch
```

**Layout.** All state is `.bss` statics; there is no `ClockState` instance and no constructor.
The five read-mostly words share one line because `ticks_per_ns` and `clock_epoch` are read
together on every record and every window fold:

```rust
#[repr(C, align(64))]
struct ClockGlobals {
    ticks_per_ns_bits: AtomicU64,   // f64::to_bits; f64 has no atomic type
    session_lo: AtomicU64,
    session_hi: AtomicU64,
    epoch: AtomicU32,
    state: AtomicU32,               // UNCALIBRATED | RUNNING | DONE
    invariant: AtomicU32,           // UNPROBED | NO | YES
    _pad: [u8; 28],
}
static CLOCK: ClockGlobals = /* all-zero const init */;
```

64 B, one line, read-mostly. `DIAG_FLAGS` (§A3) is deliberately **not** in this struct: `raise`
dirties it, and a dirtied line shared with the clock would invalidate a line every hot reader
touches.

**Who writes what, from which thread.**

| Datum | Writer | Thread | When |
|---|---|---|---|
| `ticks_per_ns_bits`, `state` | `calibrate()` | whichever thread calls first — `boyko_log::boot` or `Profiler::arm`, both idempotent | boot, once |
| `session_lo/hi` | `session_id()` on first touch | any | first touch, once |
| `epoch` | `note_forward_jump()` | the detecting thread (profiling's fold, on the dispatcher) | rare |
| `invariant` | `invariant_tsc()` on first call | any | once |

**Memory ordering.**

- `calibrate()`: `state.compare_exchange(UNCALIBRATED, RUNNING, AcqRel, Acquire)`. The winner
  probes, `ticks_per_ns_bits.store(bits, Relaxed)`, then `state.store(DONE, Release)`. A loser
  spins `state.load(Acquire)` with `core::hint::spin_loop()` + `std::thread::yield_now()` until
  `DONE`. **Release here matches the Acquire in `ticks_per_ns()`** — it is what makes the
  probed scale visible to every later reader. No `Mutex` (clippy `disallowed-types`); a CAS +
  bounded spin on a boot-only path is the compliant shape.
- `ticks_per_ns()`: `state.load(Acquire)`; if not `DONE`, return `1.0` and
  `raise(DiagFlag::ClockUncalibrated)`. Otherwise `f64::from_bits(ticks_per_ns_bits.load(Relaxed))`
  — `Relaxed` suffices because the `Acquire` on `state` already ordered it.
- `clock_epoch()`: `epoch.load(Acquire)`. `note_forward_jump()`: `epoch.fetch_add(1, Release)`
  then `raise(DiagFlag::ClockEpochBreak)`. **Release/Acquire pairs so that a consumer which
  observes the incremented epoch also observes the counters the detector wrote before
  bumping it.** On x86-64 both lower to a plain `mov`, so the ordering costs nothing; it is
  written correctly anyway rather than relying on the ISA.

**Backends.** `ticks()` has exactly two arms and neither is FFI — this is what makes the
zero-dependency claim hold:

- `#[cfg(target_arch = "x86_64")]` → `core::arch::x86_64::_rdtsc()`.
- everything else → a monotone `std::time::Instant` delta from a lazily minted base, with
  `ticks_per_ns() == 1.0` and `invariant_tsc() == false`. On Windows `Instant` *is* QPC
  internally, so the record's "QPC fallback" is honoured without a `windows-sys` dependency;
  a hand-declared `QueryPerformanceCounter` FFI is **not** written, because a second per-OS
  backing implementation is the breach §4 exists to prevent.

**`unsafe` obligations.**

- `_rdtsc()` — `// SAFETY: the `#[cfg(target_arch = "x86_64")]` gate guarantees the RDTSC
  instruction exists (architectural on x86-64 since its introduction; no CPUID feature bit
  gates its *presence*, only its invariance). The intrinsic has no memory operands, reads no
  pointer and has no side effects, so it cannot violate any aliasing or initialisation
  invariant.`
- `__cpuid(0x8000_0007)` — `// SAFETY: leaf 0x80000007 is read ONLY after
  `__cpuid(0x8000_0000).eax >= 0x8000_0007` confirms the CPU implements that extended leaf.
  Without this guard a CPU returns the highest leaf it does implement and EDX bit 8 is read
  from unrelated data. `__cpuid` writes no memory and takes no pointer.` The two-step probe is
  mandatory, not defensive.

**`.bss` residency.** `ClockGlobals` is all-zero at const-init, so the linker emits it with a
virtual size and no raw data. 64 B.

### A2 — `lane`

```rust
pub const LANE_WORKER_MAX: u16 = 64;      // == boyko_threadpool::MAX_WORKERS
pub const LANE_DISPATCHER: u16 = 64;
pub const LANE_HOST:       u16 = 65;
pub const LANE_SPARE_BASE: u16 = 66;
pub const LANE_COUNT:      u16 = /* 80 dev / see §10 Q1 for shipping */;
pub const LANE_UNCLAIMED:  u16 = u16::MAX;

thread_local! { static LANE: Cell<u16> = const { Cell::new(LANE_UNCLAIMED) }; }  // NO Drop

#[inline] pub fn lane() -> u16;
          pub fn set_lane(id: u16);           // boyko_threadpool only, at its 3 sites
#[cold]   pub fn claim_lane() -> Option<u16>;
#[cold]   pub fn release_lane();
          pub fn lanes_leaked() -> u32;       // spares never released; printed in the census
```

**Why a second TLS slot exists at all.** The obvious design — derive the lane from
`boyko_threadpool::tls::current_worker_id()` and hold no state — is impossible: `boyko_diag` is
*below* `boyko_threadpool` and cannot name it. The slot is therefore written from above, and
the divergence risk is closed by **co-location**: every `set_lane` call sits immediately beside
an existing `set_current_worker_id` call, so a future edit that moves one and not the other is
visible in a two-line diff, and gate **DG3** asserts the two agree.

**The write sites — three, not two** (§11 F1). Verified in the tree:

| Site | Existing call | Lane written |
|---|---|---|
| `crates/boyko_threadpool/src/worker.rs:24` | `tls::set_current_worker_id(worker_id)` in `worker_main` | `worker_id as u16` (dense `0..worker_count`) |
| `crates/boyko_threadpool/src/thread_pool.rs:190` | `tls::set_current_worker_id(WORKER_ID_DISPATCHER)` in `PoolInner::install` | `LANE_DISPATCHER` |
| `crates/boyko_threadpool/src/thread_pool.rs:279` | `tls::set_current_worker_id(id)` in `InstallGuard::drop` | the saved previous lane |

The third is load-bearing and is the one the decision record's "2 existing sites" omits.
`InstallGuard::drop` restores on **both** the normal return and the unwinding path; without a
lane restore there, a dispatcher thread that panics inside `install` stays labelled
`LANE_DISPATCHER` for the rest of the process and every later diagnostic from that thread is
misattributed. `InstallGuard` already carries `prev_worker_id: Option<u32>`; the lane rides the
same `Option` rather than a fourth field.

A fourth latent site, `tls::clear_current_worker_id` (`tls.rs:101`, `#[allow(dead_code)]`, test
teardown only), gets the same treatment for symmetry; it is not on any production path.

**Worker ids are dense by construction.** `ThreadPoolBuilder::build` iterates
`deques.into_iter().enumerate()` (`thread_pool.rs:601`) and hands each thread its index, so ids
are exactly `0..worker_count` with `worker_count = requested.clamp(1, MAX_WORKERS)`
(`thread_pool.rs:554`) and `MAX_WORKERS = 64` (`thread_pool.rs:49`, unconditional — no cfg, no
feature). This is why the profiler's topology, not the logger's `hash(thread_id)` scan, is the
one that anchors: it indexes an index that is already dense and already stable.

**The claim path** for threads outside the pool (asset I/O, a script VM, a mod):

```rust
static SPARE_OWNER: [AtomicU32; (LANE_COUNT - LANE_SPARE_BASE) as usize] = /* zero = FREE */;
```

`claim_lane()` scans `LANE_SPARE_BASE..LANE_COUNT` and does
`compare_exchange_weak(FREE, CLAIMED, AcqRel, Acquire)` on each. **The `Acquire` on success
pairs with the `Release` in `release_lane`'s `store(FREE, Release)`**: it is what guarantees a
new claimant of a recycled lane observes the retiring owner's final writes to that lane's loss
cells before it begins writing them itself. `compare_exchange_weak` is correct here because the
scan is a loop anyway. On exhaustion the function returns `None`, the caller stays
`LANE_UNCLAIMED`, and `LossClass::Unclaimed` is counted — **never a panic, never a block**.

Cost, stated: the scan no longer spreads by thread-id hash, so concurrent claimants convoy on
the first free slot — bounded at `LANE_COUNT - LANE_SPARE_BASE` CAS attempts on a `#[cold]`
path taken once per thread. A thread that never calls `release_lane()` holds its spare for the
process; bounded, counted as `lanes_leaked`, printed in the census.

**No `Drop`.** `LANE` is `Cell<u16>` with a `const` initialiser, so the `thread_local!`
expansion has no lazy-init flag and registers no destructor. This is the mechanism that turns
logging's "≤ 1 allocation on first emit" into **0**, and it is why `release_lane()` is explicit
rather than automatic.

**`.bss` residency.** `SPARE_OWNER` is 14 × 4 B = 56 B in the dev profile — one line, packed
deliberately: all 14 words are touched by the same cold scan, so padding them apart would cost
14 lines to prevent false sharing on a path that runs once per thread.

### A3 — `loss`

```rust
#[repr(u8)]
pub enum LossClass { Overflow, Unclaimed, Late, Refused, Device, Sink, Rotation, Budget }
impl LossClass { pub const COUNT: usize = 8; }

#[repr(C, align(64))] pub struct LossCell  { count: AtomicU64, bytes: AtomicU64, _pad: [u8; 48] }
#[repr(C, align(64))] pub struct LossTotal { count: AtomicU64, bytes: AtomicU64, _pad: [u8; 48] }

pub enum LossStatus { Measured, Unproven, UnprovenLossy, UnprovenSampled, UnprovenUnsunk }

pub fn fold_into(total: &LossTotal, cell: &LossCell);

#[repr(u32)] pub enum DiagFlag { /* sticky bits */ }
pub fn raise(f: DiagFlag);
pub fn take_raised() -> u32;
```

**Accumulate in `u64`, never saturate.** Logging's saturating `u32` was justified by "an 8-byte
RMW is more expensive"; on x86-64 `lock xadd` costs the same at 4 and 8 bytes, so the rejection
does not survive. With `u64` the `SATURATED(>=4294967295)` token — which a census reader could
never compare against anything — disappears.

**The cells are `AtomicU64`, accessed `Relaxed` by the owner** (§11 F5). The decision record
spells the per-lane cell as a plain `u64` "single-writer, no lock prefix". A plain `u64` written
by one thread and read by another is a data race and therefore UB in the Rust abstract machine
regardless of what x86-64 does; Miri reports it. `AtomicU64` with `Relaxed` load/store lowers to
exactly the same `mov` pair with no `lock` prefix, so the performance argument is preserved
verbatim and the UB is removed. `Relaxed` is the correct strength: the cell is a pure counter
and no data is published through it — the flag in `raise`/`take_raised` carries the ordering
(below).

**Who writes what.** Every producer lane writes its own cell — single-writer by the A2 topology.
Each subsystem's consumer folds. Cells are indexed `[lane][class]`.

**Layout arithmetic**, newly derived here: `LANE_COUNT × LossClass::COUNT × 64 B` =
80 × 8 × 64 = **40 KiB** in the dev profile. A per-lane block (`[LossPair; 8]` of 16 B inside one
64-B-aligned 128-B struct) would be 10 KiB and would put one lane's eight classes in two lines
instead of eight — the 48 B of padding exists to stop *cross-thread* false sharing, and the
eight classes of one lane share a single writer, so it buys nothing between them. **Not taken
here**: it changes a type the record declares. Recorded as §10 Q3 for the architect.

**The fold, and the lost-update window in it — §10 Q2, BLOCKER.** The record's `fold_into`
clears the cell with `fetch_sub(observed)` rather than `store(0)`, so that an increment landing
between the consumer's read and its clear is not lost. That closes the *consumer* side. It does
not close the *producer* side: an owner increment is `load; add; store`, and a consumer
`fetch_sub` landing between that load and that store is overwritten by the store. The exact
interleaving, and two candidate closures with their costs, are in §10. **D0 does not ship
`fold_into` until Q2 is answered**; the ladder in §7 reflects that.

**`DiagFlag` — sticky bits.**

```rust
static DIAG_FLAGS: AtomicU32 = AtomicU32::new(0);   // own cache line, not in ClockGlobals
pub fn raise(f: DiagFlag)   { DIAG_FLAGS.fetch_or(f as u32, Ordering::Release); }
pub fn take_raised() -> u32 { DIAG_FLAGS.swap(0, Ordering::Acquire) }
```

**`Release` on `raise` pairs with `Acquire` on `take_raised`.** `Relaxed` would be enough for
the bit alone, but every `raise` is paired with a counter increment whose value the emitter
prints; the pairing is what guarantees that a consumer which observes the bit also observes the
counter. The cost is zero on x86-64 and the pairing is the reason it is written this way.

`swap(0)` — not `load` then `store(0)` — is what makes the take exact: a `raise` between a load
and a separate clear would be dropped, which is the same defect class as Q2.

**`.bss` residency.** 40 KiB (cells) + 64 B (`DIAG_FLAGS`, padded to a line) + per-subsystem
`LossTotal` arrays (2 × 8 × 64 B = 1 KiB, owned by the subsystems, not by this crate).
`boyko_diag`'s own total: **≈ 42 KiB in dev.**

### A4 — `storage` (the never-freed policy + its gate helper)

```rust
#[repr(transparent)]
pub struct SyncCells<T, const N: usize>(UnsafeCell<[T; N]>);

pub const fn assert_zero_init_eligible<T: ZeroInit>() -> bool;

#[cfg(feature = "section-gate")]
pub fn section_report(sym: &str) -> SectionReport;
```

**The policy, stated once (S12), verbatim:** *Extent known at compile time ⇒ `.bss` static.
Extent chosen at run time from config ⇒ `VmReservation`, which requires the owner to sit at or
above `boyko_ecs`.*

The boundary is forced, not chosen: `VmReservation` is `pub(crate)` in `boyko_ecs`
(`vm.rs:85`, with `reserve` at `:109` and `commit` at `:199` likewise `pub(crate)`), it has a
`Drop` (`vm.rs:263`), and its unix arm calls `libc::mmap` (`vm.rs:149`) / `libc::mprotect`
(`:242`) / `libc::munmap` (`:286`). A std-only zero-dep `boyko_diag` cannot host it without
either taking a third-party dependency (forbidden) or minting a **second** hand-declared per-OS
backing implementation against `vm.rs:12-18`'s clause: *"These cfg arms are THE per-OS backing
implementation for the whole engine."* Inventing memory backing twice is a worse Principle-0
breach than the one this crate fixes.

**`SyncCells` and its `unsafe`.**

```rust
// SAFETY: `SyncCells` grants no `&T` and no `&mut T`. Every access goes through
// `get_ptr(i)`, whose caller carries the single-writer obligation stated on that
// function. The type itself therefore adds no aliasing beyond what a raw pointer
// already permits, which is what `Sync` asserts here.
unsafe impl<T: Send, const N: usize> Sync for SyncCells<T, N> {}

/// # Safety
/// The caller guarantees that (1) `i < N`, and (2) for the lifetime of the returned
/// pointer, no other thread writes index `i`. In this crate obligation (2) is
/// discharged by A2: index `i` is written only by the thread whose `lane() == i`,
/// and a lane is owned by at most one thread at a time (the `release_lane` Release /
/// `claim_lane` Acquire pairing above).
pub unsafe fn get_ptr(&self, i: usize) -> *mut T;
```

**`assert_zero_init_eligible` — what it can and cannot express (§11 F6).** The record's comment
reads "`const`: `T: Zeroable` && extent is a const". Only the first half is expressible:

- *`T` is zero-initialisable* — expressible, via a marker trait defined **in this crate**
  (`ZeroInit`), because `bytemuck::Zeroable` is third-party and forbidden here. `ZeroInit` is
  `unsafe trait` implemented for the integer/atomic primitives and for `#[repr(C)]` structs
  whose fields are all `ZeroInit`; a type with a `Drop`, a niche (`NonZeroU32`), or a reference
  cannot implement it.
- *the extent is a const* — **not expressible, and not needed**: Rust array lengths are already
  const by construction. An extent read from `ProfilerConfig` cannot be written as `[T; n]` at
  all; it forces a `Vec` or a reservation, i.e. the other arm of the policy. The compile-time
  half of the policy is enforced by the language, and the plan must say so instead of gating it.

The compile-fail red therefore lands on the expressible half — see DG7.

**`section_report` is feature-gated, off by default (§11 F7).** It shells out to a binary
inspector, i.e. it opens a process and reads a file — precisely what §6 forbids the crate to do.
The resolution is the pattern the tree already uses twice: `boyko_rhi_vulkan`'s `goldens`
feature (`Cargo.toml:22-23`, turned on for its own tests by the self-referential dev-dependency
at `:94-99`) and `boyko_render`'s `test-readback` (`:17-27`). `section-gate` is declared the
same way, is `default = []`, and is switched on by each consumer's dev-dependency. A default
build compiles no `std::process` and no `std::fs` reference at all, which is what DG9 asserts.

**`.bss` residency argument, and its limit.** A `static X: SyncCells<T, N>` whose const
initialiser is all zeroes is emitted by the linker with a virtual size and **no raw data** —
`.bss` on ELF, and on PE/COFF a section whose `SizeOfRawData` is 0 while `VirtualSize` is `N`.
That much is mechanically checkable and DG6 checks it. What is **UNPROVEN** and must not be
claimed: that the OS leaves those pages uncommitted until touched. The image tells us the bytes
are not in the file; it does not tell us the loader is lazy. The gate proves absence of raw
data, and the plan claims exactly that and no more.

### B — `profiling_abi` (hosted, not shared)

`channel`, `scope`, `zone`, `dyn_registry`, `sample`, `lane_ring`, `macros`, `tier`. `ARM_MASK`,
`REGISTRY`, `ZoneLane`, `declare_zone!`, `counter!`, `gauge!`. Indexes `A2::lane()`, stamps with
`A1::ticks()`, counts through `A3`. Emits **no** code — every `W92xx` condition is an
`A3::raise(DiagFlag::…)` plus a counter, read and emitted by `boyko_ecs::…::profiling::fold`.

Layer B lands at profiling rung P1, not in D0/D1. It is specified in
`docs/PROFILING-SYSTEM-PLAN.md`; this document owns only its *address*.

---

## 4. Explicitly NOT owned

| Not owned | Why |
|---|---|
| Any allocator, `VmReservation` | `VmReservation` is `pub(crate)` in `boyko_ecs` (`vm.rs:85,109,199`) and its unix arm uses `libc` (`vm.rs:149`). Moving it down needs either a third-party dep (forbidden) or a **second** hand-declared per-OS backing, against `vm.rs:12-18`'s "these cfg arms are THE per-OS backing implementation for the whole engine". Inventing memory backing twice is a worse Principle-0 breach than the one this crate fixes |
| Any `boyko-####` literal, `emit_diag`, any print, any panic hook | The leaf is mute. A leaf that needs a diagnostic channel is the one edge that closes the cycle |
| Any thread, file, socket, syscall, `core::fmt` | Both consumers own their own I/O; a sink in the leaf would make `boyko_utils`-level crates carry a thread |
| `LogTarget` / `CONTROL` / the level model | Logging's taxonomy. The profiler has `ARM_MASK` and wants no levels; sharing would force one to carry the other's gate |
| Statistics: median, p95, band, `Floor`, `resolve` | Need the store, which is a `Resource` |
| `BitSet` / `SparseMap` / `TypeIntern` | Stay in `boyko_utils`. `boyko_diag` must not become a second utils |
| `ZoneId` semantics for the logger | The logger never names `ZoneId`; layer B is hosted, not exported to it |

**Growth rule** (verbatim from the decision record):

> A thing enters `boyko_diag` only if **both** subsystems *write* it **and** a disagreement
> between two copies would be observable in an artifact a reader joins. Anything one writes and
> the other only reads stays with the writer, behind a getter.

Applied at review time, the rule is a two-question checklist and both answers must be yes. A
proposal that fails it does not become a `boyko_diag` module with a comment explaining the
exception; it stays where it was.

---

## 5. Layer B: `profiling_abi` is HOSTED here, not shared

`profiling_abi` is written by the profiler and read by nobody else. By §4's growth rule it does
not qualify as shared, and it is not shared — it is **hosted**.

**The graph reason, stated plainly.** The ABI must sit below `boyko_threadpool` and
`boyko_rhi_vulkan`, because both open zones. Before this plan there was exactly one crate below
everything — `boyko_utils` — and the profiling plan put the ABI there for that reason. Two facts
close that option:

1. `boyko_utils` must keep an empty `[dependencies]` (verified `Cargo.toml:6-7`), and the
   `profiling_abi` needs A1/A2/A3, so hosting it in `boyko_utils` would drag the substrate in
   behind it and end the zero-dep property.
2. `boyko_diag` is now the bottom. Hosting the ABI there costs one module in a crate the
   profiler already depends on, and costs the logger nothing at all.

**The logger never names it.** No `boyko_log` item refers to `ZoneId`, `ZoneLane`, `ARM_MASK`,
`declare_zone!`, `counter!` or `gauge!`. Layer B is `pub` from `boyko_diag` because Rust has no
finer visibility across crates, not because it is a shared surface; the constraint is enforced
by DG10 (a grep gate over `crates/boyko_log/src`), not by the type system.

**Consequence to accept:** `boyko_diag`'s public API is larger than its shared surface. A reader
who takes "everything public in the bottom crate is shared" as a rule will be wrong. The module
is therefore named `profiling_abi`, not `abi`, and its module doc opens by saying it is hosted.

---

## 6. The mute-leaf rule

**`boyko_diag` emits no `boyko-####` code, prints nothing, installs no hook, opens no file,
spawns no thread, and does not use `core::fmt`.**

A leaf that needs a diagnostic channel is the one edge that closes the cycle: `boyko_diag` sits
below `boyko_log`, so it cannot call it, and if it could, the graph would have a cycle.
The consequence is that a condition **observed** in the leaf must be **reported** above it.

**The mechanism** is `DiagFlag` + `raise` / `take_raised` (§3 A3):

1. The leaf observes the condition — lane exhaustion, an uncalibrated clock read, a forward
   jump, a refused claim.
2. It calls `raise(DiagFlag::X)` (one `fetch_or`, Release) and increments the matching
   `LossCell` counter. It does **not** format, print, or name a code.
3. An emitter above the leaf — `boyko_ecs::…::profiling::fold` for `W92xx`, `boyko_log`'s own
   drain for its codes — calls `take_raised()` at its next fold, maps each set bit to a
   registry code, and emits it with the counter's value.

**What this costs, stated: a condition is reported at the next fold, not at the instant it
occurs.** The delay is one frame in the steady state. Three consequences follow and none is
hidden:

- A condition raised **before** any emitter exists is still reported. `DIAG_FLAGS` is `.bss`, so
  it is live from process start; a `W9201` raised at `ScheduleBuilder::try_build` before
  `LogPlugin::build` runs is emitted at the first frame's fold. This is strictly better than
  "boot the logger earlier", which is unenforceable across every host.
- A condition raised **after** the last fold — during teardown, or on the crash path after the
  drain — is **not reported at all**. The bit is set in a static nobody reads again. Named here
  rather than discovered later.
- The flag is a **bit**, not a count: N occurrences of one condition raise one bit. The count
  lives in the paired `LossCell`, and an emitter that prints the code without the counter is
  reporting "it happened" when it could report "it happened N times". Every `DiagFlag` therefore
  has exactly one paired counter, and that pairing is a table in the emitter, not a convention.

**Enforcement** is DG9 (grep + the three `deny(clippy::print_*)` lints at the crate root) and
DG10 (`boyko_log` never names layer B). Both have showable REDs in §8.

**What the rule does not claim.** It does not claim `core::fmt` is absent from the linked
symbol graph. `panic!`, `expect`, and slice bounds checks pull `core::fmt::Arguments` machinery
in regardless of anything this crate writes. The claim is about *diagnostic emission*: no format
string, no `Display`/`Debug` impl, no write to any stream. The symbol-level claim is **UNPROVEN**
and is not made (§8 DG9).

---

## 7. Implementation ladder

Two rungs. Each compiles the workspace alone; neither depends on any part of either subsystem
plan.

### D0 — `crates/boyko_diag`: clock, lane, loss, storage policy

**Creates**

| Path | Contents |
|---|---|
| `crates/boyko_diag/Cargo.toml` | `[dependencies]` **empty**; `[features] default = []`, `section-gate = []`; `[lints] workspace = true` |
| `crates/boyko_diag/src/lib.rs` | crate docs; the three `deny(clippy::print_*)`; `pub mod {clock, lane, loss, storage}` |
| `crates/boyko_diag/src/clock.rs` | A1 |
| `crates/boyko_diag/src/lane.rs` | A2 minus the write sites (D1 supplies the callers) |
| `crates/boyko_diag/src/loss.rs` | A3 **minus `fold_into`** (blocked on §10 Q2) |
| `crates/boyko_diag/src/storage.rs` | A4; `section_report` behind `section-gate` |

**Edits**: `Cargo.toml` `members` **and** `default-members` (both lists — a member absent from
`default-members` is invisible to the bare `cargo check`, which is exactly the vacuity the
2026-07 audit fixed at `Cargo.toml:4-13`).

**Does not create** `crates/boyko_diag/build.rs`. S9's `BOYKO_PROFILE` build script belongs to
the joint rung J1, and it would be the **first `build.rs` in the workspace** (verified: none
exists in any member or at the root). Landing it here would put a build-script rebuild trigger
under all 21 default members before anything reads its output.

**Gates:** DG1, DG2, DG5, DG6, DG7, DG8, DG9.

### D1 — `boyko_threadpool -> boyko_diag`; `set_lane` at its three sites

**Edits**

| Path | Change |
|---|---|
| `crates/boyko_threadpool/Cargo.toml` | `+ boyko_diag = { path = "../boyko_diag" }` — the crate's first non-crossbeam dependency; add the one-line rationale comment the house style uses on every such edge |
| `crates/boyko_threadpool/src/worker.rs:24` | beside `tls::set_current_worker_id(worker_id)`: `boyko_diag::lane::set_lane(worker_id as u16)` |
| `crates/boyko_threadpool/src/thread_pool.rs:190` | beside the `WORKER_ID_DISPATCHER` write: `set_lane(LANE_DISPATCHER)`; save the previous lane into `InstallGuard` |
| `crates/boyko_threadpool/src/thread_pool.rs:279` | in `InstallGuard::drop`, beside the restore: restore the saved lane (covers the unwinding path) |
| `crates/boyko_threadpool/src/tls.rs:101` | `clear_current_worker_id` also clears the lane to `LANE_UNCLAIMED` |
| `crates/boyko_threadpool/src/thread_pool.rs` (near `:49`) | `const _: () = assert!(boyko_diag::lane::LANE_WORKER_MAX as usize == MAX_WORKERS);` |

The const-assert lives **here**, not in `boyko_diag`: the bottom crate cannot name `MAX_WORKERS`.
The record states the equality as a comment; a comment is not a gate (DG4).

**`boyko_app` is not touched at D1.** `LANE_HOST` is written by `boyko_app::runner` boot, which
lands with the host rung in the subsystem plans.

**Gates:** DG3, DG4, and D0's gates re-run.

---

## 8. Gates

Every gate below names the concrete broken input that makes it fail. This project's signature
defect is the gate that is green because it cannot fail — it has been caught eight times in this
campaign. Where a RED could not be constructed, the row says **UNPROVEN** and asserts nothing.

| # | Gate | Showable RED |
|---|---|---|
| **DG1** | Tidy: `crates/boyko_utils/Cargo.toml` has an empty `[dependencies]`, and `crates/boyko_diag/Cargo.toml` has one too. `cargo tree -p boyko_diag -e normal,build` lists exactly one node. | Add any dep — e.g. `libc` — to either manifest ⇒ both legs red. `-e normal,build` is explicit because `boyko_diag` gains a `build.rs` at J1 and a build-dependency must not slip past a `normal`-only tree. *(S2)* |
| **DG2** | Lane placement: a fixture running on worker *k* reads `lane() == k`; an unattached thread reads `LANE_UNCLAIMED`. | Delete the `set_lane` call in `worker_main` ⇒ every worker reads `LANE_UNCLAIMED` ⇒ red. *(S3a)* |
| **DG3** | Lane/worker-id agreement, including unwind: on every thread, `lane()` maps 1:1 onto `current_worker_id()` under the documented map — before, during and after an `install`, and after an `install` whose body panicked and was caught. | Move the lane restore out of `InstallGuard::drop` into the normal-return path ⇒ the panicking leg leaves `lane() == LANE_DISPATCHER` on a thread whose `current_worker_id()` is back to its previous value ⇒ red on the unwind leg only. *(added by this plan — the third write site, §11 F1)* |
| **DG4** | `const _: () = assert!(LANE_WORKER_MAX as usize == MAX_WORKERS)` in `boyko_threadpool`. | Change `MAX_WORKERS` to 32 ⇒ compile error at `thread_pool.rs:49`. Costs one line and turns a comment into a build failure. *(added)* |
| **DG5** | Loss fold exactness: preset a lane's cell, drop N with a live producer thread running, assert the folded global advanced by **exactly** N and the cell was cleared without loss. | Replace the clearing operation with `store(0)` ⇒ an increment between load and clear is lost ⇒ the global lags the injected count ⇒ red. One gate serves both subsystems (it is profiling's G4 and logging's G11). **Blocked on §10 Q2**: the gate as written also reds on the *record's* `fetch_sub` shape under a producer whose increment is a non-atomic RMW — which is the point of Q2. *(S8)* |
| **DG6** | `.bss` residency: `section_report` over the test binary asserts every named `boyko_diag` static's section carries a virtual size with **no raw data**. | Initialise one element non-zero ⇒ raw data appears ⇒ red. **Tooling prerequisite, MEASURED:** no `llvm-readobj`, `objdump`, `nm` or `llvm-nm` is on PATH, and the active `stable-x86_64-pc-windows-gnu` toolchain ships only `rust-objcopy` / `rust-lld` in its `bin` dir — the `llvm-tools` component is **not installed**. The gate must therefore resolve its tool at start and treat absence as a **RED, never a SKIP**; `rustup component add llvm-tools` is a D0 line item. A skip-on-absent gate is green on every machine that lacks the tool, which is this one. *(S12)* |
| **DG7** | `assert_zero_init_eligible` compile-fail: a `trybuild` `compile_fail` case declaring `SyncCells<NonZeroU32, 4>` (or any `Drop` type) fails to compile with the `ZeroInit` bound error. | Remove the `T: ZeroInit` bound ⇒ it compiles ⇒ red. **Mechanism note:** the record specifies "a `#[test]` that must fail at compile time"; a `#[test]` that fails to compile fails the whole test binary's build, so it cannot be a passing gate. `trybuild` is the workspace's existing mechanism (dev-dep of `boyko_ecs`, `boyko_rhi_vulkan`, `boyko_ui`). **The "extent is a const" half is not gated** — Rust array lengths are const by construction (§3 A4), so there is no broken input to construct and no assertion is made. *(S12, refined)* |
| **DG8** | Clock epoch + sticky flag: `note_forward_jump(x)` increments `clock_epoch()` by exactly 1 and raises `DiagFlag::ClockEpochBreak`; `take_raised()` returns the bit once and 0 thereafter. | Make `raise` a plain `store` of the bit instead of `fetch_or` ⇒ a concurrent second raise clobbers the first ⇒ the two-flag leg reds. Second: replace `take_raised`'s `swap(0)` with `load` + `store(0)` and run a concurrent raiser ⇒ a raise between the two is dropped ⇒ red. *(S4, D0-runnable half)* |
| **DG9** | Mute leaf: `rg 'println!\|eprintln!\|boyko-[BEW][0-9]{4}\|std::process\|std::fs\|thread::spawn' crates/boyko_diag/src` returns zero under the default feature set. | Add one `eprintln!` to `lane.rs` ⇒ caught **twice**: by the grep and by `deny(clippy::print_stderr)` under the existing `cargo clippy --all-targets -- -D warnings`. Second: enable `section-gate` by default ⇒ `std::process` appears ⇒ red. **UNPROVEN and not asserted:** that `core::fmt` is absent from the linked symbol graph — `panic!` and bounds checks pull it in regardless (§6). *(added)* |
| **DG10** | `boyko_log` never names layer B: `rg 'ZoneId\|ZoneLane\|ARM_MASK\|declare_zone\|profiling_abi' crates/boyko_log/src` returns zero. | Add one `use boyko_diag::profiling_abi::ZoneId;` to `boyko_log` ⇒ red. Runs from the rung that creates `boyko_log`, not from D0. *(added, §5)* |
| **DG11** | Claim-path distinctness: `LANE_COUNT - LANE_SPARE_BASE` threads each get a distinct spare; the next gets `None`, and `LossClass::Unclaimed` is 1. | Replace the CAS with a load-then-store ⇒ two threads claim the same spare. **Wall-clock form is flaky by construction** (it needs a specific interleaving); the reliable form is the loom model in §9. The wall-clock leg asserts only the deterministic half — exhaustion returns `None` without panicking or blocking — and says so. *(S3, split)* |

**Gates deferred to a rung where they can fail** — listed so they are not silently dropped:

- **The join red (S3b)** — one `warn!` and one zone on the same worker must carry the *same*
  integer; giving the logger its own registry back makes them differ. Cannot run at D0/D1
  (neither subsystem exists). Lands with the first rung where both a log record and a sample
  exist (logging L5 / profiling P2).
- **The cross-artifact clock red (S4)** — after a synthetic forward jump, the profiler's window
  is quarantined **and** the log records after the jump carry the incremented `clock_epoch`.
  Same reason; same rung.
- **The `BOYKO_PROFILE` `compile_error!` red (S9)** and the **`config_tag` red (S10)** belong to
  J1/J2 and are named in the two subsystem plans.

---

## 9. Unit / property / Miri / loom surface

The crate has exactly **two concurrency objects**: the lane claim path and the loss fold.
Everything else is a const, a `.bss` read, or a single-writer TLS `Cell`.

**Loom — the lane claim path.** This is a CAS loop over a shared array with a real interleaving
question: *can two threads observe `FREE` on the same spare and both proceed?* The bounded model
is 2–3 threads over 2 spares, asserting (a) the returned ids are pairwise distinct, (b) at most
`N_SPARES` claims succeed, (c) a `release_lane` followed by a `claim_lane` on another thread
returns the released id and the claimant observes the releaser's final cell values (the
Release/Acquire pairing in §3 A2). Loom is already wired workspace-wide: `loom = "0.7"` in
`[workspace.dependencies]`, and both `boyko_ecs` and `boyko_threadpool` pull it under
`[target.'cfg(loom)'.dependencies]` with a `sync.rs` shim. `boyko_diag` follows the same shape.
**Run the loom leg in debug**: loom release binaries crash at startup on this machine
(pre-existing, unrelated to this crate).

**Loom — not for the loss fold.** The fold's question is not "which interleaving is reachable"
but "is this operation atomic at all", which loom answers trivially and misleadingly. Once §10
Q2 is resolved the fold is a pair of atomic RMWs whose correctness is a property of the
operations, not of the schedule. What the fold needs is the **exactness property test** (DG5)
with a live producer: inject N increments from a producer thread while the consumer folds
repeatedly, assert the sum of all folded deltas equals N. That is a proptest over
(n_producers, n_increments, n_folds), not a loom model.

**Miri — the loss cells and `SyncCells`.** Miri is the right instrument for both, and for two
different reasons:

- **Data-race detection on the cells.** This is what catches the plain-`u64` spelling (§11 F5):
  a plain `u64` written by an owner thread and read by a consumer thread is UB and Miri reports
  it; the `AtomicU64`/`Relaxed` spelling passes. The Miri leg is therefore not a formality — it
  is the instrument that distinguishes the two designs, and it must be run against a **two-thread**
  fixture (a single-threaded Miri run cannot see a data race, so a single-threaded leg here is a
  gate that cannot fail).
- **Aliasing (Stacked/Tree Borrows) on `SyncCells::get_ptr`.** The `unsafe impl Sync` and the
  raw-pointer discipline are exactly what Miri's borrow tracker checks. Run under
  `MIRIFLAGS=-Zmiri-tree-borrows`, matching the workspace's existing kernel Miri practice.

**Miri cannot cover** `_rdtsc` or `__cpuid` — Miri has no x86 intrinsic support. Those arms are
`#[cfg]`-excluded under `cfg(miri)` in favour of the `Instant` backend, and the intrinsic arm's
correctness rests on the two SAFETY arguments in §3 A1, not on a test. Stated rather than
papered over.

**Plain unit tests** (no special runner): const arithmetic (`LANE_SPARE_BASE < LANE_COUNT`,
`LossClass::COUNT == 8`, `size_of::<LossCell>() == 64`, `align_of::<LossCell>() == 64`), the
`LANE_UNCLAIMED` default on a fresh thread, `clock_epoch` monotonicity, `take_raised` idempotence
after a take, and `ticks()` monotonicity across two calls on one thread.

---

## 10. Open questions

**Q1 — `LANE_COUNT = 32` in the shipping profiles is unsound with a worker-anchored topology.
BLOCKER for D0: `LANE_COUNT` is a D0 const.**

S9's table sets `LANE_COUNT = 80` in `dev`/`editor` and **32** in `shipping`/`shipping-min`, and
S3 justifies 80 as "max, not sum: 64 is a hard const, +dispatcher +host +14 claimable spares".
Measured against the tree, the 32 contradicts the same sentence:

- `pub const MAX_WORKERS: usize = 64` (`thread_pool.rs:49`) is **unconditional** — no cfg, no
  feature, no profile.
- `worker_count = requested.clamp(1, MAX_WORKERS)` (`:554`) with
  `requested = available_parallelism()` by default (`:553`, `:684`).
- Worker ids are the dense `0..worker_count` (`:601`), and the lane *is* the worker id.

So a `shipping` build on any machine with more than 32 hardware threads produces lane indices
≥ 32 into arrays sized 32. Independently, `LANE_HOST = 65` is already out of range at
`LANE_COUNT = 32` on *every* machine. The floor for the worker-anchored topology is **66**
(64 workers + dispatcher + host) plus whatever spares are kept.

The 32 is inherited from logging's `MAX_LANES = 32` shipping value, which was safe there
because its lanes were CAS-claimed and unanchored. Three candidate resolutions, none taken here:

(a) `LANE_COUNT = 66` in shipping (zero spares — a non-pool thread then never gets a lane and
every diagnostic from it counts as `Unclaimed`); (b) `LANE_COUNT = 72` (six spares, ≈ 36 KiB of
`LossCell`); (c) make `MAX_WORKERS` profile-dependent in `boyko_threadpool` — the only option
that recovers the 32, and a change to the pool's public API that this plan does not schedule.
**Architect call.**

**Q2 — `fold_into`'s lost-update window is not closed by `fetch_sub`. BLOCKER for A3.**

S8 replaces `store(0)` with `fetch_sub(observed)` so an increment landing between the consumer's
read and its clear is not lost. That closes the consumer side. The producer side stays open:

```
owner:     r = cell.load()          ...                       cell.store(r + 1)
consumer:                  obs = cell.load(); cell.fetch_sub(obs)
```

The owner's `store` overwrites the consumer's subtraction; `obs` was already folded into the
total, so that one loss event is counted twice — the exact double-count S8 exists to remove.
Two closures:

(a) **Owner-side `fetch_add(1, Relaxed)`** — one `lock xadd` per *loss event*. Losses are by
definition rare (a loss is the thing being counted), so the RMW lands on the already-degraded
path, never on the hot path. Costs the "single-writer, no lock prefix" sentence, which was
never about the hot path in the first place.

(b) **Consumer-side monotone delta, no clear at all** — the cell is write-only-increasing; the
consumer keeps a per-(lane,class) `last_seen` and folds `cur.wrapping_sub(last_seen)`. The owner
keeps its plain `Relaxed` load/store pair (one `mov` each, no lock prefix), exactness is
unconditional, and `fetch_sub` disappears from the design entirely. Costs `LANE_COUNT × 8 × 8 B`
of consumer-side `last_seen` state (5 KiB dev) — owned by each consumer, not by `boyko_diag`.

(b) is the standard monotone-counter fold and preserves the record's own performance argument.
**Architect call.** DG5's RED is written against whichever lands; the record's shape as written
reds on it, which is why D0 ships `loss.rs` without `fold_into`.

**Q3 — `LossCell` padding.** 48 B of padding per (lane, class) costs 40 KiB in dev and puts one
lane's eight classes in eight lines. The padding exists to stop cross-thread false sharing, and
one lane's eight classes share a single writer. A per-lane `#[repr(C, align(64))]` block of
eight 16-B pairs is 10 KiB and two lines. Not taken here because it changes a type the record
declares. Cheap to take; the fold touches 4× fewer lines.

**Q4 — the paired-counter table for `DiagFlag`.** §6 requires every flag to have exactly one
paired counter so an emitter can print "N times" rather than "it happened". The record specifies
neither the flag set nor the pairing. The 17 `W92xx` rows fix the profiling half at logging L2;
the logging half and the substrate's own flags (`ClockEpochBreak`, `ClockUncalibrated`,
`LaneExhausted`) are unassigned. Needs a table before any emitter is written; owed by whichever
plan lands its emitter first.

**Q5 — `boyko_demo`'s third-party logging is larger than the ledger says** (§11 F2). Two use
sites (`main.rs:86` `log::Level::Info` under the wasm arm, `main.rs:113` `log::error!`) and two
backend crates (`env_logger = "0.11"` at `Cargo.toml:32`, `console_log = "1"` at `:69`) that
exist only to service the `log` facade. Deleting `log = "0.4"` alone breaks `:86` and leaves two
declared deps that pull `log` straight back into the graph. Also: `log 0.4` is in the build graph
transitively via `eframe`, `egui`, `naga`, `gpu-allocator` and `bevy_ecs` regardless, so the
tidy check can only ever be about **direct declarations** and must say so — otherwise it asserts
something demonstrably false about the workspace. SCOPE call: is the demo's ability to see
wgpu/winit diagnostics dropped, or is the third-party facade kept for the demo alone?

---

## 11. Facts verified against the tree, and where the record diverges

Verified this session by reading the files, not by transcription. `cargo` was not run.

| # | Claim | Result |
|---|---|---|
| V1 | `crates/boyko_utils/Cargo.toml` has an empty `[dependencies]` | **TRUE** — `:6-7`, table present, no entries. Four modules: `bit_mask`, `identifiers`, `sparse_map`, `type_intern` |
| V2 | `MAX_WORKERS = 64`; worker ids dense | **TRUE** — `thread_pool.rs:49` (unconditional `pub const`), clamp at `:554`, `.enumerate()` at `:601`, `debug_assert` at `worker.rs:22` |
| V3 | `VmReservation` is `pub(crate)`, has a `Drop`, unix arm uses `libc` | **TRUE** — `vm.rs:85` struct, `:109` `reserve`, `:199` `commit`, `:149` `libc::mmap`, `:242` `mprotect`, `:286` `munmap`, `:263` `impl Drop`. The single-source-of-truth clause is at **`:12-18`** (record cites `:12-17`) |
| V4 | `crates/boyko_image/Cargo.toml:5` claims no workspace dependency | **TRUE** — verbatim in the package `description`; falsified by L8b |
| V5 | `crates/boyko_diag/` does not exist | **TRUE** — absent from `crates/`, from `members` and from `default-members` |
| V6 | `92xx` is free in source | **TRUE** — source literals are `B1802`×24, `B0002`×7, `B9001`×6, `B9101`×4, `B9005`×3, `B9004`×3, `B9002`×3, `B1801`×2, `W1501`×1. No `92xx`. `docs/diagnostics/` **does not exist yet** |
| V7 | `BOYKO_PROFILE` is a free env name | **TRUE** — 39 `BOYKO_*` vars in use; none is `BOYKO_PROFILE` |
| V8 | No `rdtsc` / QPC anywhere today | **TRUE** — A1 is entirely new code; there is no existing clock site to migrate |
| V9 | No `build.rs` exists in the workspace | **TRUE** — none in any member, none at the root. `crates/boyko_diag/build.rs` would be the first |

**Where the decision record diverges from the tree.**

- **F1 — `set_lane` "at its 2 existing sites" undercounts; there are three.**
  `worker.rs:24`, `thread_pool.rs:190` (`install` entry), `thread_pool.rs:279`
  (`InstallGuard::drop`). The third covers the **unwinding** path; without it a panicking
  dispatcher keeps `LANE_DISPATCHER` for the process. The record's prose does say "entry/exit",
  so the design is right and only the count is wrong — but the count is what an implementer
  works from. §3 A2 lists all three; DG3 reds on the missing one. A fourth latent site,
  `tls.rs:101`, is test-only.

- **F2 — the `boyko_demo` ledger entry is incomplete**, and the tidy check derived from it is a
  gate that cannot fail on the backends. Detail in §10 Q5.

- **F3 — `boyko_rhi_vulkan/Cargo.toml:44-49` is the `boyko_sdf_math` precedent block, not the
  no-third-party rationale.** The rationale is at `:7-12`. The `boyko_diag` row goes at `:7-12`
  citing `:44-49`.

- **F4 — two edge-list rows are incomplete** (not wrong): `boyko_ecs` also carries `fixedbitset`,
  `static_assertions`, optional `mimalloc` and `cfg(loom)` `loom`; `boyko_render` also carries
  `bytemuck`; `boyko_fontbake` also carries `ttf-parser`; `boyko_threadpool` also carries
  `cfg(loom)` `loom`. `bench_bevy_vs_boyko` and the root package `boyko-engine` are absent from
  the list entirely — both are `members` **and** `default-members`, so any workspace-wide tidy
  check must enumerate 21 manifests, not 19. §2's list is complete.

- **F5 — `LossCell { count: u64 }` as a plain `u64` is a data race.** Written by the owner
  thread and read by the consumer thread, a non-atomic `u64` is UB in the Rust abstract machine
  irrespective of x86-64's behaviour, and Miri reports it. `AtomicU64` with `Relaxed` load/store
  lowers to the identical `mov` pair with no `lock` prefix, so the record's performance argument
  survives intact. §3 A3 specifies the atomic spelling. The same argument is cited in the record
  as one logging "already makes for `SAMPLE_CTR`" — if that spelling is also a plain integer
  read across threads, it has the same defect; out of scope here, flagged for the logging plan.

- **F6 — `assert_bss_eligible`'s "extent is a const" half is not expressible** and does not need
  to be: array lengths are const by construction in Rust. Its `T: Zeroable` half needs a marker
  trait defined in this crate, because `bytemuck` is third-party and forbidden here. §3 A4 and
  DG7.

- **F7 — `section_report` as specified violates the crate's own mute-leaf rule.** It shells out
  to a binary inspector — a process and a file. Resolved by the `section-gate` feature, default
  off, following the `boyko_rhi_vulkan` `goldens` / `boyko_render` `test-readback` precedent
  already in the tree. Additionally **MEASURED**: no `llvm-readobj` / `objdump` / `nm` /
  `llvm-nm` is on PATH and the active `stable-x86_64-pc-windows-gnu` toolchain ships only
  `rust-objcopy` and `rust-lld`, so `llvm-tools` is not installed and the whole `.bss` gate
  family (this crate's DG6, profiling G22, logging G3) cannot run on this machine as written.
  DG6 makes tool absence a RED, not a SKIP.

- **F8 — the S12 compile-fail red cannot be a `#[test]`.** A `#[test]` that fails to compile
  fails the test binary's build. `trybuild` is the workspace's existing mechanism. DG7.

**Where the record is right and the tree's own comments are stale** — recorded so a later reader
does not "correct" the record from a comment:

- `crates/boyko_scene/Cargo.toml:23-25` describes a `boyko_utils` dependency; there is none in
  its `[dependencies]`. The record correctly omits the edge.
- `crates/boyko_render/Cargo.toml:8-9` says "boyko_render depends DIRECTLY on boyko_ecs +
  boyko_rhi + boyko_rhi_vulkan + boyko_utils"; there is no `boyko_utils` entry. The record
  correctly omits it.
- `crates/boyko_ui/Cargo.toml:30` repeats the `boyko_scene -> boyko_utils` claim. Same.

None of the three is edited by this plan — they are outside its scope and are noted for whoever
owns the manifest-comment sweep.

---

## 12. Ready for review

Two rungs, one new crate, one new edge, eleven gates of which nine have a showable RED and two
are explicitly split or deferred to a rung where they can fail. Three blockers are open and
named: the shipping `LANE_COUNT` (Q1, blocks a D0 const), the fold's lost-update window (Q2,
blocks `fold_into`), and the `llvm-tools` prerequisite (F7, blocks DG6). None of them is a
design disagreement; each is a value or a mechanism the decision record did not fix.
