# Substrate — implementation ladder, gates, and the tree-verification record

<!-- CONTRACT
provides: substrate/ladder-d0-d1          # the two rungs both subsystem ladders wait on
provides: substrate/gates-dg              # DG1-DG12, each with a showable RED, + the test surface
exports:  substrate/tree-verification     # V1-V9, the divergences, and what is still open
assumes:  substrate/crate-graph           # D0/D1 land the edges; DG1 gates the zero-dep property
assumes:  substrate/mute-leaf-rule        # DG9/DG10 are its mechanical half
assumes:  substrate/clock-source          # DG8's subject; DG12's RED is calibrate() at static init
assumes:  substrate/lane-registry         # DG2/DG11's subject; Q1 blocks a D0 const
assumes:  substrate/lane-write-sites      # D1's edit table; DG3's unwind leg
assumes:  substrate/loss-vocabulary       # DG5's subject; the Miri data-race leg
assumes:  substrate/loss-fold             # Q2 RESOLVED (b): monotone counter, delta at the consumer
assumes:  substrate/never-freed-storage   # DG7's subject
assumes:  substrate/section-report        # DG6's subject and its RED-not-SKIP rule
-->

**No `seam/` assumption is declared, and that is the repair.** This file used to assume
`seam/free-when-off` — an edge pointing *up*, out of the bottom layer. The no-boot-work property
below is a property **of `boyko_diag`**: the crate touches nothing at process start, which is
checkable against the crate alone and is gated by DG12 without reference to any decision above.
Stating it as a borrowed obligation inverted the layering, and once this file's siblings declare
their own edges the inversion closes a cycle rather than merely reading oddly. The joint decision
that makes the property *matter* is recorded above this area; the property itself is ours.

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §7, §8, §9, §10 Q5, §11 and §12, plus the
> no-boot-work obligation and gate DG12 authored at the corpus split.
> Diff against that file until the monoliths are retired.

---

## 1. Implementation ladder

Two rungs. **Each compiles the workspace alone**; neither depends on any part of either
subsystem plan. Both subsystem ladders wait on them: D0/D1 land **before** logging L0 and
**before** profiling rung 1.

### D0 — `crates/boyko_diag`: clock, lane, loss, storage policy

**Creates**

| Path | Contents |
|---|---|
| `crates/boyko_diag/Cargo.toml` | `[dependencies]` **empty**; `[features] default = []`, `section-gate = []`; `[lints] workspace = true` |
| `crates/boyko_diag/src/lib.rs` | crate docs; the three `deny(clippy::print_*)`; `pub mod {clock, lane, loss, storage}` |
| `crates/boyko_diag/src/clock.rs` | [A1](01-CLOCK.md) |
| `crates/boyko_diag/src/lane.rs` | [A2](02-LANE.md) minus the write sites (D1 supplies the callers) |
| `crates/boyko_diag/src/loss.rs` | [A3](03-LOSS.md) **in full** — Q2 resolved (b): the cell is monotone and never cleared, so `delta_since` ships at D0 and `fold_into` does not exist |
| `crates/boyko_diag/src/storage.rs` | [A4](04-STORAGE.md); `section_report` behind `section-gate` |

**Edits**: the root `Cargo.toml`'s **`members` (`:2`) AND `default-members` (`:13`)** — *both*
lists. A member absent from `default-members` is invisible to the bare `cargo check`, which is
exactly the vacuity the 2026-07 audit fixed; the audit's own comment explaining it is at
`Cargo.toml:4-12`, immediately above the `default-members` line it added.

**Line item (D0) — no boot work: nothing in `boyko_diag` is touched, calibrated, spawned or
committed at process start.** Concretely: `calibrate()` is not called from a static
initialiser, from `lib.rs`, or from any `#[ctor]`-shaped construct; no lane buffer is written;
no `SPARE_OWNER` slot is claimed; `session_id()` is not minted eagerly. Every one-time cost runs
on the **enable path** — `boyko_log::enable()` / `Profiler::arm()` — which runs at launch,
before the game loop. Gated by **DG12**.

**Does not create** `crates/boyko_diag/build.rs`. S9's `BOYKO_PROFILE` build script belongs to
the **joint rung J1**, and it would be **the first `build.rs` in the workspace** (verified: none
exists in any member or at the root). Landing it here would put a build-script rebuild trigger
under all 21 default members **before anything reads its output**.

**Gates:** DG1, DG2, DG5, DG6, DG7, DG8, DG9, DG12.

### D1 — `boyko_threadpool -> boyko_diag`; `set_lane` at its three sites

**Edits**

| Path | Change |
|---|---|
| `crates/boyko_threadpool/Cargo.toml` | `+ boyko_diag = { path = "../boyko_diag" }` — the crate's first non-crossbeam dependency; add the one-line rationale comment the house style uses on every such edge |
| `crates/boyko_threadpool/src/worker.rs:24` | beside `tls::set_current_worker_id(worker_id)`: `boyko_diag::lane::set_lane(worker_id as u16)` |
| `crates/boyko_threadpool/src/thread_pool.rs:190` | beside the `WORKER_ID_DISPATCHER` write: `set_lane(LANE_DISPATCHER)`; save the previous lane into `InstallGuard` |
| `crates/boyko_threadpool/src/thread_pool.rs:279` | in `InstallGuard::drop`, beside the restore: restore the saved lane (**covers the unwinding path**) |
| `crates/boyko_threadpool/src/tls.rs:101` | `clear_current_worker_id` also clears the lane to `LANE_UNCLAIMED` |
| `crates/boyko_threadpool/src/thread_pool.rs` (near `:49`) | `const _: () = assert!(boyko_diag::lane::LANE_WORKER_MAX as usize == MAX_WORKERS);` |

**Creates**: `crates/boyko_threadpool/tests/diag_lane.rs` — DG2/DG3's runnable legs. It lives in
`boyko_threadpool`, not in `boyko_diag`, because the property under test is a property of the
*edge*: `boyko_diag` cannot name a worker, and a gate placed in the bottom crate could only
re-read the slot it just wrote. `boyko_diag` is a normal dependency of the package, so a `tests/`
target names it directly with no dev-dependency added.

**The lane is SAVED into the guard, never derived from `prev_worker_id` on the way out.** The
derivation looks total — worker `k` ↦ `k`, `WORKER_ID_DISPATCHER` ↦ `LANE_DISPATCHER`,
`WORKER_ID_UNATTACHED` ↦ `LANE_UNCLAIMED` — and it is wrong on two live cases: the host thread
carries `LANE_HOST` while its worker id is `WORKER_ID_UNATTACHED`, and a thread holding a spare
from `claim_lane` carries a lane the pool never wrote. Both come out of `install` as
`LANE_UNCLAIMED`, and the spare case is worse than a mislabel: `release_lane` reads the TLS to
find the slot to free, so the spare is stranded for the process. `InstallGuard` therefore carries
**one `Option<(u32, u16)>`** rather than an `Option<u32>` beside an `Option<u16>` — a shape in
which "restored the id but not the lane" is not expressible.

**The const-assert lives HERE, not in `boyko_diag`:** the bottom crate cannot name
`MAX_WORKERS`. The record states the equality as a *comment*; **a comment is not a gate** (DG4).

**Line item (D1) — no boot work:** `set_lane` writes a TLS `Cell` and **nothing else** —
it does not calibrate, does not claim a spare, does not touch a loss cell. A worker thread
starting under flags-off must leave every `boyko_diag` static untouched.

**`boyko_app` is NOT touched at D1.** `LANE_HOST` is written by `boyko_app::runner` boot, which
lands with the host rung in the subsystem plans.

**Gates:** DG3, DG4, DG12, and D0's gates re-run.

---

## 2. Gates

**Every gate below names the concrete broken input that makes it fail.** This project's signature
defect is the gate that is green because it *cannot* fail — it has been caught eight times in
this campaign. Where a RED could not be constructed, the row says **UNPROVEN** and asserts
nothing.

| # | Gate | Showable RED |
|---|---|---|
| **DG1** | Tidy: `crates/boyko_utils/Cargo.toml` has an empty `[dependencies]`, and `crates/boyko_diag/Cargo.toml` has one too. `cargo tree -p boyko_diag -e normal,build` lists exactly one node. | Add any dep — e.g. `libc` — to either manifest ⇒ both legs red. **`-e normal,build` is explicit** because `boyko_diag` gains a `build.rs` at J1 and a build-dependency must not slip past a `normal`-only tree. *(S2)* |
| **DG2** | Lane placement: a task body running on worker *k* reads `lane() == k`; an unattached thread reads `LANE_UNCLAIMED`. **Assert the documented MAP, not raw equality** — see the dispatcher note below the table. | **MEASURED RED (D1):** delete the `set_lane` call in `worker_main` ⇒ 4096 of 4096 task bodies disagree, first offender `worker_id=0 lane=65535`, and exactly one test of the six fails. *(S3a)* |
| **DG3** | Lane/worker-id agreement, **including unwind**: `lane()` maps onto `current_worker_id()` under the documented map — before, during and after an `install`, **after an `install` whose body panicked and was caught**, and **for a thread that entered `install` holding a claimed spare**. | **Two MEASURED REDs (D1), each hitting one leg and only that leg.** (a) Move the lane restore out of `InstallGuard::drop` into the normal-return path ⇒ the panicking leg finds `lane() == 64` (`LANE_DISPATCHER`) where it must be `65535`, while the normal-return leg stays **green** — the split the gate exists to make. (b) Derive the restored lane from `prev_worker_id` instead of saving it ⇒ the spare leg finds `65535` where it must be `66`, and nothing else moves. *(added by this plan — the third write site, F1; leg (b) added at implementation)* |
| **DG4** | `const _: () = assert!(LANE_WORKER_MAX as usize == MAX_WORKERS)` in `boyko_threadpool`. | **MEASURED RED (D1):** set `MAX_WORKERS = 32` ⇒ `error[E0080]: evaluation panicked: assertion failed: boyko_diag::lane::LANE_WORKER_MAX as usize == MAX_WORKERS` at `thread_pool.rs:58`. One line, and the comment became a build failure. *(added)* |
| **DG5** | **Loss fold exactness**: preset a lane's cell, drop N with a **live producer thread running**, assert the folded global advanced by **exactly** N and the cell was cleared without loss. | Replace the clearing operation with `store(0)` ⇒ an increment between load and clear is lost ⇒ the global lags the injected count ⇒ red. **One gate serves both subsystems** (profiling G4b = logging G11). **Blocked on Q2**: the gate as written *also* reds on the **record's own `fetch_sub` shape** under a producer whose increment is a non-atomic RMW — which is the point of Q2. *(S8)* |
| **DG6** | **`.bss` residency**: `section_report` over the test binary asserts every named `boyko_diag` static's section carries a virtual size with **no raw data**. | Initialise one element non-zero ⇒ raw data appears ⇒ red. **Tooling prerequisite, MEASURED:** no `llvm-readobj` / `objdump` / `nm` / `llvm-nm` is on PATH, and the active `stable-x86_64-pc-windows-gnu` toolchain ships only `rust-objcopy` / `rust-lld` — `llvm-tools` is **not installed**. The gate **resolves its tool at start and treats absence as a RED, never a SKIP**; `rustup component add llvm-tools` is a D0 line item. A skip-on-absent gate is green on every machine that lacks the tool, **which is this one**. **CANNOT CLAIM:** that the OS leaves those pages uncommitted — the image proves absence of raw data and nothing more. *(S12)* |
| **DG7** | `assert_zero_init_eligible` **compile-fail**: a `trybuild` `compile_fail` case declaring `SyncCells<NonZeroU32, 4>` (or any `Drop` type) fails to compile with the `ZeroInit` bound error. | Remove the `T: ZeroInit` bound ⇒ it compiles ⇒ red. **Mechanism note (F8):** the record specifies "a `#[test]` that must fail at compile time"; a `#[test]` that fails to compile fails the whole test binary's build, so it cannot be a *passing* gate. `trybuild` is the workspace's existing mechanism (dev-dep of `boyko_ecs:55`, `boyko_rhi_vulkan:82`, `boyko_ui:45`). **The "extent is a const" half is NOT gated** — Rust array lengths are const by construction, so there is no broken input to construct and **no assertion is made**. *(S12, refined)* |
| **DG8** | **Clock epoch + sticky flag**: `note_forward_jump(x)` increments `clock_epoch()` by exactly 1 and raises `DiagFlag::ClockEpochBreak`; `take_raised()` returns the bit once and 0 thereafter. | Make `raise` a plain `store` of the bit instead of `fetch_or` ⇒ a concurrent second raise clobbers the first ⇒ the two-flag leg reds. **Second:** replace `take_raised`'s `swap(0)` with `load` + `store(0)` and run a concurrent raiser ⇒ a raise between the two is dropped ⇒ red. *(S4, D0-runnable half)* |
| **DG9** | **Mute leaf, in two legs a byte scan can honestly decide.** (a) `rg 'println!\|eprintln!\|boyko-[BEW][0-9]{4}\|thread::spawn' crates/boyko_diag/src` returns zero. (b) `rg 'std::process\|std::fs' crates/boyko_diag/src -g '!storage.rs'` returns zero. **There is no "under the default feature set" qualifier, and there must not be:** `rg` reads bytes and cannot evaluate a `#[cfg]`. The one process/file text D0 licenses is `section_report`'s body, which D0 places in `storage.rs` (`:49` above) behind `section-gate` ([`04-STORAGE.md`](04-STORAGE.md) `:30-31`, body at `:137-138`) — so leg (b) excludes that file **by path**, which a scan can do, rather than by feature, which a scan cannot. With the old pattern a spec-conforming D0 spelled naturally (`use std::process::Command;`) made the gate red on a *correct* implementation. | (a) Add one `eprintln!` to `lane.rs` ⇒ caught **twice**: by the grep and by `deny(clippy::print_stderr)` under the existing `cargo clippy --all-targets -- -D warnings`. (b) Add `use std::fs;` to `lane.rs` ⇒ red, on a file no feature selection can excuse. **The former second RED is WITHDRAWN:** "enable `section-gate` by default" edits `crates/boyko_diag/Cargo.toml` (`:44` above), which is not under `crates/boyko_diag/src`, so the scan output is **byte-identical across that flip** — an asserted RED no reader could construct, which is exactly what the rule at `:99-102` exists to stop. **Its compile-observing replacement is UNPROVEN and is NOT asserted:** the natural instrument — a `trybuild` `compile_fail` case (DG7's mechanism) proving `section_report` unresolvable — is defeated at its natural site by the self-referential dev-dep that switches `section-gate` ON for `boyko_diag`'s own test binary ([`04-STORAGE.md`](04-STORAGE.md) `:147`), and moved out of that site its result depends on which packages a single `cargo` invocation unifies features across. **CANNOT CLAIM** that a default build compiles no `std::process` and no `std::fs`: that property is established by the `#[cfg(feature = "section-gate")]` guard itself, not by any byte scan, so DG9 must not be cited as its proof. **Residual, stated:** leg (b)'s path exclusion is total, so a `std::fs` use added to the *ungated* part of `storage.rs` is outside every leg of this gate; review of that one file is a human obligation. **UNPROVEN and not asserted:** that `core::fmt` is absent from the linked symbol graph — `panic!` and bounds checks pull it in regardless. *(added)* |
| **DG10** | **`boyko_log` never names layer B**: `rg 'ZoneId\|ZoneLane\|ARM_MASK\|declare_zone\|profiling_abi' crates/boyko_log/src` returns zero. | Add one `use boyko_diag::profiling_abi::ZoneId;` to `boyko_log` ⇒ red. **Runs from the rung that creates `boyko_log`, not from D0.** *(added)* |
| **DG11** | **Claim-path distinctness**: `LANE_COUNT - LANE_SPARE_BASE` threads each get a distinct spare; the next gets `None`, and `LossClass::Unclaimed` is 1. | Replace the CAS with a load-then-store ⇒ two threads claim the same spare. **The wall-clock form is FLAKY BY CONSTRUCTION** (it needs a specific interleaving); the reliable form is the **loom model** below. The wall-clock leg asserts only the deterministic half — **exhaustion returns `None` without panicking or blocking** — and says so. *(S3, split)* |
| **DG12** | **No boot work** (added at the split): with both runtime flags off, a process that starts, runs N frames and exits has (a) never entered `calibrate()`, (b) never written a `SPARE_OWNER`/`LossCell`/`DIAG_FLAGS` byte — i.e. no `boyko_diag` **shared static**; the TLS `LANE` `Cell` is **excluded and must be**, because it is the one write D1 does mandate (`worker.rs:24`, `:77` above) and DG2/DG3 assert it has happened, and it costs 2 B of per-thread TLS and no `.bss` ([`02-LANE.md`](02-LANE.md) `:29`, `:160`). D1's own no-boot-work line item (`:86-88` above) already states the predicate this way; only this enumeration disagreed with it, and at D1 it could not have been green on the implementation D1 prescribes. (c) minted no `SessionId`. Observed by a `#[cfg(test)]` counter on each entry point plus DG6's image probe over the same binary. | Call `calibrate()` from a static initialiser, or from D1's `set_lane` path ⇒ leg (a) reds on the first worker spawn. **Second:** pre-touch the lane buffers in `lib.rs` ⇒ leg (b) reds. **CANNOT CLAIM** that the pages are physically uncommitted — DG12 proves *nothing wrote them*, DG6 proves *nothing is in the image*; **whether the loader commits an untouched page is UNPROVEN in this corpus** and is not asserted by either. |

### The dispatcher executes tasks — DG2/DG3 assert the MAP, and a raw-equality form is flaky

Found by building D1, not by reading the plan. `Scope::drop` does **not** park: it blocks by work
stealing, taking from the injector and the sibling stealers and running what it takes **inline on
the calling thread** (`crates/boyko_threadpool/src/scope.rs`, "Work-stealing wait", and
`worker::run_task` is `pub(crate)` precisely so that path can reuse it). A task that lands there
executes with `current_worker_id() == WORKER_ID_DISPATCHER` and `lane() == LANE_DISPATCHER` — a
**correct** pairing that `lane as u32 == id` reports as a defect, because the sentinel is
`u32::MAX - 1` and the lane is 64.

**MEASURED, and this is the part worth keeping:** the first draft of the gate asserted raw
equality, ran **green with zero disagreements**, and reported exactly one on a later run. It was
flaky by construction and its own first result said otherwise. Across ~24 further runs of the same
binary the dispatcher executed **zero** of the 4096 tasks, so the honest statement is that the
path is *documented and reachable*, **not** that it is *covered* — and the gate's failure message
now carries the offending `(worker_id, lane)` pair rather than a bare count, so the next
occurrence explains itself instead of being re-derived.

Consequence for every later consumer, not just this gate: **the dispatcher's lane is not a
"nothing happens here" lane.** Work is attributed to it, so a reader that treats `LANE_DISPATCHER`
as bookkeeping-only will silently drop real samples.

### Gates deferred to a rung where they can fail

Listed so they are **not silently dropped**:

- **The join red (S3b)** — one `warn!` and one zone on the same worker must carry the **same**
  integer; giving the logger its own registry back makes them differ. Cannot run at D0/D1
  (neither subsystem exists). Lands with the first rung where both a log record and a sample
  exist — **logging L5 / profiling P2**.
- **The cross-artifact clock red (S4)** — after a synthetic forward jump, the profiler's window
  is quarantined **and** the log records after the jump carry the incremented `clock_epoch`.
  Same reason; same rung.
- **The `BOYKO_PROFILE` `compile_error!` red (S9)** and the **`config_tag` red (S10)** belong to
  **J1/J2** and are named in the two subsystem plans.

---

## 3. Unit / property / Miri / loom surface

The crate has exactly **two concurrency objects**: the lane claim path and the loss fold.
Everything else is a const, a `.bss` read, or a single-writer TLS `Cell`.

**Loom — the lane claim path.** This is a CAS loop over a shared array with a real interleaving
question: *can two threads observe `FREE` on the same spare and both proceed?* The bounded model
is 2–3 threads over 2 spares, asserting (a) the returned ids are pairwise distinct, (b) at most
`N_SPARES` claims succeed, (c) a `release_lane` followed by a `claim_lane` on another thread
returns the released id **and the claimant observes the releaser's final cell values** (the
Release/Acquire pairing in [`02-LANE.md`](02-LANE.md)). Loom is already wired workspace-wide:
`loom = "0.7"` in `[workspace.dependencies]` (root `Cargo.toml:62`), and both `boyko_ecs`
(`Cargo.toml:38-39`) and `boyko_threadpool` (`:20-21`) pull it under
`[target.'cfg(loom)'.dependencies]` with a `sync.rs` shim. `boyko_diag` follows the same shape.
**Run the loom leg in debug**: loom release binaries crash at startup on this machine
(pre-existing, unrelated to this crate).

> ⚠️ **The cited precedent does not compile, MEASURED at D1.** `RUSTFLAGS=--cfg loom cargo check
> -p boyko-threadpool --lib` fails with `error[E0599]: no method named get_mut found for struct
> loom::sync::atomic::AtomicPtr<T>` at `crates/boyko_threadpool/src/scope.rs:185`; loom 0.7.2
> offers `with_mut`, not `get_mut`. `-p boyko-ecs` fails on the **same** error, because it reaches
> the same lib — so **both** precedents this paragraph names are dead, and `rg loom
> .github/workflows` returns **nothing**, so no CI leg would ever have said so. Verified not to be
> a D1 regression by `git stash`-ing the D1 diff and reproducing the identical error at `93dbcf8`.
>
> What this costs the paragraph above: the sentence "loom is already wired workspace-wide" is true
> of the **manifests** and false of the **build**. `boyko_diag` may still copy the manifest shape —
> that part is verified — but it must not inherit the claim that a working model runs beside it.
> Whoever writes the `claim_lane` model is the first person in this workspace to compile a loom
> leg since it broke, and should expect to fix `scope.rs:185` first. Raised with the owner in
> [`docs/OPEN-QUESTIONS.md`](../../OPEN-QUESTIONS.md); **not repaired here** — it is outside D1's
> subject, it sits in an `unsafe` `Drop`, and a green `cargo check` under `--cfg loom` would still
> not be a *run* model.

**Loom — NOT for the loss fold.** The fold's question is not "which interleaving is reachable"
but "is this operation atomic at all", **which loom answers trivially and misleadingly**. Once
Q2 is resolved the fold is a pair of atomic RMWs whose correctness is a property of the
*operations*, not of the *schedule*. What the fold needs is the **exactness property test**
(DG5) with a **live producer**: inject N increments from a producer thread while the consumer
folds repeatedly, assert the sum of all folded deltas equals N. That is a proptest over
`(n_producers, n_increments, n_folds)`, not a loom model.

**Miri — the loss cells and `SyncCells`.** Miri is the right instrument for both, for two
*different* reasons:

- **Data-race detection on the cells.** This is what catches the plain-`u64` spelling (F5): a
  plain `u64` written by an owner thread and read by a consumer thread is UB and Miri reports
  it; the `AtomicU64`/`Relaxed` spelling passes. **The Miri leg is therefore not a formality — it
  is the instrument that distinguishes the two designs**, and it **must** run against a
  **two-thread** fixture. *A single-threaded Miri run cannot see a data race, so a
  single-threaded leg here is a gate that cannot fail.*
- **Aliasing (Stacked/Tree Borrows) on `SyncCells::get_ptr`.** The `unsafe impl Sync` and the
  raw-pointer discipline are exactly what Miri's borrow tracker checks. Run under
  `MIRIFLAGS=-Zmiri-tree-borrows`, matching the workspace's existing kernel Miri practice.

**Miri CANNOT cover** `_rdtsc` or `__cpuid` — Miri has no x86 intrinsic support. Those arms are
`#[cfg]`-excluded under `cfg(miri)` in favour of the `Instant` backend, and the intrinsic arm's
correctness rests on the two SAFETY arguments in [`01-CLOCK.md`](01-CLOCK.md), **not on a test**.
Stated rather than papered over.

**Plain unit tests** (no special runner): const arithmetic (`LANE_SPARE_BASE < LANE_COUNT`,
`LossClass::COUNT == 8`, `size_of::<LossCell>() == 64`, `align_of::<LossCell>() == 64`), the
`LANE_UNCLAIMED` default on a fresh thread, `clock_epoch` monotonicity, `take_raised`
idempotence after a take, and `ticks()` monotonicity across two calls on one thread.

---

## 4. Q5 — `boyko_demo`'s third-party logging is larger than the ledger says (F2)

**SCOPE call for the owner.**

Verified this session:

| Item | Site |
|---|---|
| `log::Level::Info` (wasm arm, routing records to the browser console) | `crates/boyko_demo/src/main.rs:86` |
| `log::error!` | `crates/boyko_demo/src/main.rs:113` |
| `log = "0.4"` — the direct facade declaration | `crates/boyko_demo/Cargo.toml:28` |
| `env_logger = "0.11"` | `crates/boyko_demo/Cargo.toml:32` |
| `console_log = "1"` | `crates/boyko_demo/Cargo.toml:69` |

`env_logger` and `console_log` **exist only to service the `log` facade**. Deleting `log = "0.4"`
alone breaks `:86` and leaves **two declared deps that pull `log` straight back into the graph**.

**And the harder fact:** `log 0.4` is in the build graph **transitively** via `eframe`, `egui`,
`naga`, `gpu-allocator` and `bevy_ecs` **regardless**. So the tidy check can only ever be about
**DIRECT declarations**, and it **must say so** — otherwise it asserts something demonstrably
false about the workspace, which is a gate that lies rather than a gate that fails.

**The call:** is the demo's ability to see wgpu/winit diagnostics dropped, or is the third-party
facade kept for the demo alone?

---

## 5. Facts verified against the tree, and where the record diverges

Verified by reading the files, not by transcription. **`cargo` was not run.**

| # | Claim | Result |
|---|---|---|
| V1 | `crates/boyko_utils/Cargo.toml` has an empty `[dependencies]` | **TRUE** — `:6`, table present, no entries. Four modules: `bit_mask`, `identifiers`, `sparse_map`, `type_intern` |
| V2 | `MAX_WORKERS = 64`; worker ids dense | **TRUE** — `thread_pool.rs:49` (unconditional `pub const`), clamp at `:554`, `.enumerate()` at **`:602`** (record and plan say `:601`), `debug_assert` at `worker.rs:22` — **against `inner.workers.len()`, not `MAX_WORKERS`** |
| V3 | `VmReservation` is `pub(crate)`, has a `Drop`, unix arm uses `libc` | **TRUE** — `vm.rs:85` struct, `:109` `reserve`, `:190` `os_len`, `:199` `commit`, `:149` `libc::mmap`, `:242` `mprotect`, `:263` `impl Drop`, `:286` `munmap`. The single-source-of-truth clause is at **`:12-18`** (record cites `:12-17`) |
| V4 | `crates/boyko_image/Cargo.toml:5` claims no workspace dependency | **TRUE** — verbatim in the package `description`; falsified by L8b |
| V5 | `crates/boyko_diag/` does not exist | **TRUE** — absent from `crates/`, from `members` and from `default-members` |
| V6 | `92xx` is free in source | **TRUE** — a fresh scan of `crates/*/src` + `src` returns **zero** `92xx` literals. In-use literals are `B1802`×24, `B0002`×7, `B9001`×6, `B9101`×4, `B9005`×3, `B9004`×3, `B9002`×3, `B1801`×2, `W1501`×1 |
| V7 | `BOYKO_PROFILE` is a free env name | **TRUE** — zero occurrences across `crates/`, `src/`, `scripts/` and `.github/`. **Count correction:** the record says "39 `BOYKO_*` vars in use"; a fresh scan of `crates/*/src` + `src` finds **28 distinct** `BOYKO_*` names. The conclusion is unchanged; the denominator is not what the record says, and the seam's route-(a) argument (an env var, matching the existing switches) is stated against 28 |
| V8 | No `rdtsc` / QPC anywhere today | **TRUE** — A1 is entirely new code; there is no existing clock site to migrate |
| V9 | No `build.rs` exists in the workspace | **TRUE** — none in any member, none at the root. `crates/boyko_diag/build.rs` would be the first |

### Where the decision record diverges from the tree

- **F1 — `set_lane` "at its 2 existing sites" undercounts; there are THREE.** `worker.rs:24`,
  `thread_pool.rs:190` (`install` entry), `thread_pool.rs:279` (`InstallGuard::drop`). The third
  covers the **unwinding** path; without it a panicking dispatcher keeps `LANE_DISPATCHER` for
  the process. The record's *prose* does say "entry/exit", so the design is right and **only the
  count is wrong** — but the count is what an implementer works from.
  [`02-LANE.md`](02-LANE.md) lists all three; **DG3 reds on the missing one.** A fourth latent
  site, `tls.rs:101`, is test-only.

- **F2 — the `boyko_demo` ledger entry is incomplete**, and the tidy check derived from it is a
  gate that **cannot fail on the backends**. Detail in §4 above.

- **F3 — `boyko_rhi_vulkan/Cargo.toml:44-49` is the `boyko_sdf_math` PRECEDENT block, not the
  no-third-party rationale.** The rationale is at **`:7-12`** and the `boyko_diag` row goes
  there, citing `:44-49`. *(A second-order "correction" written earlier this session gave the
  block as `:7-13` and claimed `:13` carried the closing sentence. Re-read from the file: `:12`
  is "# it adds no third-party dependency." and **`:13` is `[features]`**, a TOML table header.
  The correction was itself wrong, in a repair whose whole subject was a wrong line citation —
  which is why the rule for this class is to print the lines, not to reason about them.)*

- **F4 — two edge-list rows are incomplete** (not *wrong*): `boyko_ecs` also carries
  `fixedbitset`, `static_assertions`, optional `mimalloc` and `cfg(loom)` `loom`; `boyko_render`
  also carries `bytemuck`; `boyko_fontbake` also carries `ttf-parser`; `boyko_threadpool` also
  carries `cfg(loom)` `loom`. `bench_bevy_vs_boyko` and the root package `boyko-engine` are
  absent from the record's list entirely — **both are `members` AND `default-members`**, so any
  workspace-wide tidy check must enumerate **21 manifests, not 19** (20 crates + the root
  package). [`00-GOAL.md`](00-GOAL.md)'s list is complete.

- **F5 — `LossCell { count: u64 }` as a plain `u64` is a DATA RACE.** Written by the owner thread
  and read by the consumer thread, a non-atomic `u64` is UB in the Rust abstract machine
  **irrespective of x86-64's behaviour**, and Miri reports it. `AtomicU64` with `Relaxed`
  load/store lowers to the identical `mov` pair with **no `lock` prefix**, so the record's
  performance argument **survives intact**. [`03-LOSS.md`](03-LOSS.md) specifies the atomic
  spelling. **The same argument is cited in the record as one logging "already makes for
  `SAMPLE_CTR`" — if that spelling is also a plain integer read across threads, it has the same
  defect.** Out of scope here; flagged for the logging plan.

- **F6 — `assert_bss_eligible`'s "extent is a const" half is NOT EXPRESSIBLE** and does not need
  to be: array lengths are const by construction in Rust. Its `T: Zeroable` half needs a marker
  trait defined **in this crate**, because `bytemuck` is third-party and forbidden here.
  [`04-STORAGE.md`](04-STORAGE.md) and DG7.

- **F7 — `section_report` as specified VIOLATES the crate's own mute-leaf rule.** It shells out
  to a binary inspector — a process and a file. Resolved by the `section-gate` feature, default
  off, following the `boyko_rhi_vulkan` `goldens` (`Cargo.toml:22-23`, dev-dep at `:94-99`) /
  `boyko_render` `test-readback` (`:17-27`) precedent already in the tree. **Additionally
  MEASURED:** no `llvm-readobj` / `objdump` / `nm` / `llvm-nm` is on PATH and the active
  `stable-x86_64-pc-windows-gnu` toolchain ships only `rust-objcopy` and `rust-lld`, so
  `llvm-tools` is **not installed** and the whole `.bss` gate family (DG6, profiling G22a/G22b, logging
  G3) **cannot run on this machine as written.** DG6 makes tool absence a **RED, not a SKIP**.

- **F8 — the S12 compile-fail red CANNOT be a `#[test]`.** A `#[test]` that fails to compile
  fails the test binary's build. `trybuild` is the workspace's existing mechanism. DG7.

### Where the record is right and the tree's own comments are STALE

Recorded so a later reader does not "correct" the record **from a comment**:

- `crates/boyko_scene/Cargo.toml:23-25` describes a `boyko_utils` dependency ("reusable
  collections (declared per the S2 scope; kept even though v1 uses only the kernel scratch…)");
  **there is none in its `[dependencies]`.** The record correctly omits the edge.
- `crates/boyko_render/Cargo.toml:7-9` says "boyko_render depends DIRECTLY on boyko_ecs +
  boyko_rhi + boyko_rhi_vulkan + boyko_utils"; **there is no `boyko_utils` entry.** The record
  correctly omits it. *(The substrate plan cites `:8-9`; the sentence begins at `:7`.)*
- `crates/boyko_ui/Cargo.toml:30` repeats the `boyko_scene -> boyko_utils` claim. Same.

**None of the three is edited by this plan** — they are outside its scope and are noted for
whoever owns the manifest-comment sweep.

---

## 6. Ready for review

Two rungs, one new crate, one new edge, **twelve gates** of which ten have a showable RED and two
are explicitly split or deferred to a rung where they can fail.

**Three blockers are open and named:**

| Blocker | Blocks | Owner |
|---|---|---|
| ~~the shipping `LANE_COUNT`~~ ([Q1](02-LANE.md)) | ~~a D0 const~~ | **RESOLVED — 80 in every profile, no profile axis** |
| ~~the fold's lost-update window~~ ([Q2](03-LOSS.md)) | ~~`fold_into`~~ | **RESOLVED — (b) monotone counter; `delta_since` ships, `fold_into` deleted** |
| the `llvm-tools` prerequisite ([F7](04-STORAGE.md)) | DG6, and the whole `.bss` gate family | a D0 line item (`rustup component add llvm-tools`) |

Two further open items are recorded rather than blocking: [Q3](03-LOSS.md) (the `LossCell`
padding, cheap to take) and [Q4](03-LOSS.md) (the `DiagFlag` ↔ counter pairing table, owed by
whichever plan lands its emitter first). One SCOPE call goes to the owner: [Q5](#4--q5--boyko_demos-third-party-logging-is-larger-than-the-ledger-says-f2).

**None of them is a design disagreement**; each is a value or a mechanism the decision record did
not fix.
