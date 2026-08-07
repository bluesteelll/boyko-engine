# Architecture: Engine Logging & Diagnostics (`boyko_log`)

> Target file: `docs/LOGGING-SYSTEM-PLAN.md`. Status: **DRAFT v2 — revised against `architecture-critic` review of v1.** Changelog and findings disposition at the end.

## Changelog v1 → v2 (summary; per-finding table at the bottom)

Four soundness defects in v1's own pseudocode are fixed by protocol changes, not by wording: the drain now **copies before it frees** (B1), record length is a **runtime quantity** (B2), the ring has an **explicit non-straddling wrap protocol** (B3), and `fmtv` — the re-entrancy hazard — is **deleted and replaced by `dsp!`, which formats in argument position, before any lane state is touched** (B4). Three gates that could not fail are replaced with gates that have a named red state (B5, B6, B7). The `[vk-validation]` migration is **withdrawn**: validation output is a gate oracle and stays on a synchronous, never-dropped channel (B8). `LogHandle` is gone, `LogFilter` is gone, `fmtv` is gone, the `RATE` index is dense, the lane has three cache-line partitions instead of two, and the enforcement mechanism is an in-repo tidy test rather than a clippy key whose liveness this repository has already measured as untrustworthy.

### Answers to the critic's eight questions

1. **Drain order** — the drain copies each record out of the ring into a staging arena, *then* advances `read`, *then* sorts, *then* decodes from staging. `read` never advances over bytes the sink still intends to read. (§Algorithms C)
2. **Encoded length** — `LogArgs::encoded_len(&self) -> usize`, a runtime method that const-folds for all-fixed tuples. `&str` encodes as `u16` length + bytes. `fmtv` no longer exists; a `Display` is rendered by `dsp!` into a caller-owned stack buffer **in argument position**, so the ring is never open while user code runs, and an overrun is a truncation of a `&str` that has already been produced. (§Decision 1a, §Decision 13)
3. **Wrap** — records never straddle. Deterministic shared rule: `LANE_BYTES - off < HEADER_BYTES ⇒ both sides wrap`; otherwise a PAD record (null `site`, `len` = tail) consumes the tail. (§Algorithms A5)
4. **Re-entrancy** — forbidden *structurally*: nothing between lane acquisition and the `Release` store can call user code, because argument encoding is over already-materialised POD and `&str`. A re-entrancy `debug_assert` guard backs it. (§Decision 13)
5. **Sink rate** — stated as a design number (≈ 500 K records·s⁻¹·lane before drop at the default geometry) and **gated at L3** by `sink_sustained_rate`, which must show the drop threshold red. (§Decision 10, §Metrics)
6. **Perturbation** — a mandatory ABBA-counterbalanced logger-on/logger-off frame-time gate with an interleaved zero control in the same sitting. (§Metrics, gate P1)
7. **`[vk-validation]`** — stays synchronous. The migration is withdrawn. (§Decision 9)
8. **Handle / flush-without-consumer** — `LogHandle` is deleted; `boot()`/`shutdown()` are free functions over process-lifetime statics. `flush()` reads `SINK_STATE` first and returns `FlushResult::NoConsumer` immediately when nothing can ever acknowledge. There is no `join`-with-timeout because std does not provide one; shutdown observes a sink-exited atomic with a bounded spin and then **detaches**. (§Decision 12)

---

## Goal

Replace 83 shipping print sites (179 raw occurrences across 36 files under `crates/**/src/**`, of which 58 are legitimate `boyko_shaderdsl` CLI stdout and 23 are in-`src` `#[cfg(test)]` modules), 5 hand-rolled `AtomicBool` once-latches, and 9 ad-hoc `boyko-####` codes in three incompatible text formats, with **one in-house subsystem** that obeys the engine's own principles.

**Functional**
- Any thread — Chase-Lev worker, dispatcher, OS/window thread, asset I/O — emits a diagnostic without a lock, an allocation, or a syscall.
- Every `Warn`/`Error` carries a registry code that is documented, uniquely numbered, mechanically proven non-orphan, and explainable.
- Loss is counted and reported; never silent.
- **Evidence channels are synchronous.** Measurement lines and validation-layer messages do not travel on the async path (Decisions 9 and 9b).
- The **absence** of records on an armed target is reported as `UNPROVEN`, never as `clean`.

**Performance targets** — every row has a control measured in the same sitting; a number without a control is not a measurement, and this repository has measured its own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol.

| Metric | Target | Control that can go red |
|---|---|---|
| Compile-disabled site | no `emit_impl` symbol reference in the object file | the armed variant of the same fixture **must** show the symbol (§Metrics G1) |
| Runtime-disabled site | ≤ 3 ns | enabled variant of the same site, same bench |
| Enabled, 0 args | ≤ 15 ns median | runtime-disabled, same sitting |
| Enabled, 2×u32 | ≤ 20 ns median | as above |
| Allocations on the producer path | **0**, proven by a **per-thread** counting allocator | armed sink thread must show > 0 on its own thread |
| Syscalls per record | **0**; one `write` per drain per byte sink | — |
| Producer working set | ≤ 4 cache lines | — |
| Sustained rate before drop | ≥ 500 K records·s⁻¹·lane at default geometry | `sink_sustained_rate` must find the drop knee |
| Frame-time perturbation, logger idle | not resolvable above the sitting's floor | ABBA + interleaved zero control (gate P1) |
| Resident memory | `claimed_lanes × 16 KiB`; `LANES` in `.bss`, gated | section gate G3 |
| Fully-off build | `size_of_val(&LANES) == 0`, no sink thread, no panic hook | build leg G2 |

Published reference band: Quill 8-9 ns, NanoLog 7 ns median (both vendor-published, deferred-format); spdlog 242 ns (caller-side format). The ~30× gap **is** caller-side formatting. That single fact organises this design.

---

## Context and constraints

### Affected subsystems
New crate `boyko_log`, depending on `std` only, below everything. `boyko_threadpool`, `boyko_utils`, `boyko_ecs`, `boyko_rhi`, … all depend on it. No cycles. Lane identity is minted **by `boyko_log`**, which is what keeps the threadpool able to log.

### Invariants preserved
1. `clippy.toml`'s `disallowed-types` — no `HashMap`/`HashSet`/`Mutex`/`RwLock`/`Rc`/`RefCell` in this crate at all. The one lock in the design (`OUT_LOCK`, §Decision 9b) is an `AtomicBool` spin lock, is off every frame path, and is **registered in `docs/HOT-PATH-EXCEPTIONS.md`** rather than hidden behind the type ban's letter.
2. **The stdout/stderr machine API.** Verified: `scripts/golden.ps1:201` runs the render test as `cargo … --nocapture > "$valLog" 2>&1` and `:226` scans that merged file for `\[vk-validation\]`, printing `VALIDATION: clean (0 messages)` in green at zero. `crates/boyko_app/tests/vg_occ_split_timing.rs` parses `VB-P1d ` from that same merged stream with `contains`. Seventeen test files depend on these. **Both contracts survive byte-for-byte and remain synchronous** (Decisions 9, 9b).
3. Existing `#[should_panic(expected = "boyko-B0002")]` assertions match on a substring, so normalising `error[boyko-B0002]: …` → `boyko-B0002: …` is safe.
4. Codes are never renumbered, never reused. `B9003` is a permanent gap.
5. `#[cold] + #[inline(never)]` on diagnostic helpers (`crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:1-6`).
6. **No new hang class.** `vb_bench_totality_gate.rs:44-53` records that this repository *has no kill-after-timeout pattern to borrow*. Every wait in this design is bounded and, where no acknowledgement is structurally possible, returns immediately with a reason.

### Constraints inherited from the audits
- `current_worker_id_or_dispatcher_lane()` maps `WORKER_ID_UNATTACHED` → lane `0` (`crates/boyko_threadpool/src/tls.rs:69-78`, read this session). Window thread, present thread, driver callback thread and test harness threads all land there. Reusing that router would make lane 0 MPSC. `boyko_log` mints its own lanes.
- `ThreadLanePair<E>` (`crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:63-140`) is the house layout precedent, including the manual `unsafe impl Sync` + SAFETY-block style.
- The counting-allocator gate is **process-global** and already produced an impossible negative delta from a sibling test thread; `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK`. A permanently resident sink thread cannot be serialised by a test-local lock (M18).
- `clippy.toml:21-25` states, from an empirical 2026-07 check, that **clippy silently ignores a config path it cannot resolve** — an unresolvable entry emits nothing. Any new clippy key is therefore a claim requiring a liveness proof (M23).

---

## Key decisions

### Decision 1: Deferred formatting — POD payload + `&'static LogSite` + monomorphised decoder
**What.** The call site writes a 20-byte packed header containing a `*const LogSite` and a POD encoding of the arguments. Formatting happens on the sink via `site.decode: unsafe fn(*const u8, usize, &mut LogFormatter)`, monomorphised per *argument-tuple type* (Rust dedups identical monomorphisations, giving Quill's `log_statement` sharing for free).

**Why.** spdlog is asynchronous and still costs 242 ns median because it formats on the caller; Quill/NanoLog cost 7-9 ns because they do not. `core::fmt::Arguments<'a>` borrows its temporaries and cannot outlive the call, so the C++ varargs-capture trick has no Rust analogue.

**Alternatives rejected.** *defmt linker-section interning* — depends on ELF section semantics and a linker script; this toolchain is windows-gnu / PE-COFF, and an 8-byte `&'static` pointer is already free and portable. *`tracing`* — `event!` emits a static callsite, an `Interest` atomic load and `__is_enabled()` even when disabled, then dispatches through `Box<dyn Layer>`; Bevy's own docs concede the runtime-filter cost. *Caller-side format into a stack buffer* — 100+ ns and re-imports `core::fmt` codegen into 83+ sites.

**Trade-off.** Arguments must be `LogValue` (POD + `&str`). A `Display` is rendered by `dsp!` at the call site, and that cost is **visible in the source**, which is the point.

### Decision 1a: Record length is a runtime quantity — `encoded_len(&self)`, not `const ENCODED_LEN` *(fixes B2)*
**What.**
```rust
pub trait LogValue: private::Sealed {
    /// Upper bound known at compile time; `usize::MAX` means "dynamic".
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;                 // folds to a const for POD
    unsafe fn encode(&self, dst: *mut u8) -> usize; // returns bytes written
}
pub trait LogArgs { fn encoded_len(&self) -> usize; unsafe fn encode(&self, dst: *mut u8) -> usize; }
```
For an all-POD tuple every `encoded_len` is a constant and LLVM folds the sum to an immediate — the const case loses nothing. `&str` encodes as `u16` length + bytes, capped at `MAX_STR_BYTES = 256` with the `STR_TRUNCATED` flag.

**Why.** v1's `const ENCODED_LEN` was undefined for the two implementors that produce large records, which meant the space reservation the entire overflow protocol, the Error reserve and `MAX_RECORD_BYTES` rest on was undefined exactly where it mattered.

**Trade-off.** One non-foldable add per dynamic argument. Measured by `log_enabled_str32` against `log_enabled_2u32`.

### Decision 2: Three gates, `&&`-chained — two const-folded, one byte load
```rust
if T::STATIC_CEILING as u8 >= LVL as u8            // const: per-target compile ceiling
    && $crate::GLOBAL_CEILING as u8 >= LVL as u8   // const IN boyko_log (see N27)
    && $crate::runtime_ceiling(T::ID) >= LVL as u8 // Relaxed u8 load
{ $crate::emit_impl(...) }
```
**Why.** The `&&` short-circuit is what guarantees arguments are never evaluated — `log`'s exact shape, and why its docs warn against side-effecting arguments. Unreal explicitly does *not* give this guarantee and documents `UE_LOG_ACTIVE` as the workaround. The per-target compile ceiling is Unreal's two-verbosity model, which `log`/`tracing` lack. The runtime gate is one `Relaxed` load from `static CEILINGS: [AtomicU8; 256]` — puffin's measured ~1 ns `AtomicBool` shape, generalised.

**Alternatives rejected.** *Cargo features for the ceiling* — features are additive and unified; one crate enabling `max_level_trace` re-enables it for everyone. `option_env!` in a `const fn` has no such failure mode; staleness is closed by `build.rs` emitting `cargo:rerun-if-env-changed=BOYKO_LOG_MAX_LEVEL`. **`GLOBAL_CEILING` is a `const` item inside `boyko_log`, referenced as `$crate::GLOBAL_CEILING`; the `option_env!` is never expanded into a caller crate**, where no rerun directive exists (N27).

**Trade-off.** Changing the ceiling rebuilds the workspace. `MAX_TARGETS = 256` is a hard cap.

### Decision 3: Statics in `.bss`; `Off == 0`; a genuinely-off build *(extends v1, fixes M21)*
**What.** `LANE_BYTES = 16 KiB`, `MAX_LANES = 128`, both `option_env!`-overridable. The lane array is a static, sized by a `const`:
```rust
pub const LANE_ARRAY_LEN: usize = if (GLOBAL_CEILING as u8) == 0 { 0 } else { MAX_LANES };
static LANES: [Lane; LANE_ARRAY_LEN] = [Lane::NEW; LANE_ARRAY_LEN];
```
`boot()` is a no-op when `GLOBAL_CEILING == Off`: no sink thread, no panic hook, no `RATE` traffic.

**Why — four properties, each load-bearing.**
1. No boot check on the hot path; a heap block behind an `AtomicPtr` would cost an `Acquire` load plus a null branch per record and create a "not booted" state at every site.
2. The unbooted default is correct and free: `.bss` is zero, `Level::Off == 0`, so every target reads `Off`, the gate fails, nothing happens.
3. Demand-zero paging: resident cost is `claimed_lanes × 16 KiB`, typically 8-12 lanes ≈ 128-192 KiB.
4. **`BOYKO_LOG_MAX_LEVEL=off` is a real off switch**, not merely a site-folding switch: zero lanes, zero threads, zero hooks.

**Gated, not assumed** (M21): `.bss` residency of a `MaybeUninit` static on PE/COFF is a toolchain behaviour, so gate **G3** runs `llvm-readobj --sections` (or `objdump -h`) over the test binary and asserts the section owning `LANES` carries a size with no raw data. Gate **G2** is a separate CI leg built with `BOYKO_LOG_MAX_LEVEL=off` asserting `size_of_val(&LANES) == 0` and that no sink thread is spawned.

**Honest floor when "on"**: 2 MiB reserved `.bss`, 8 KiB `RATE`, one OS thread, one process-global panic hook, one `VmReservation`-backed `LogRing` when the ECS seam is enabled, and a mandatory dependency edge from every crate. That is the cost of the system existing; it is stated, not smoothed.

### Decision 4: SPSC byte ring per lane; claim-on-first-use; consumer-only reclaim
**What.** Each lane is a single-producer/single-consumer byte ring. A thread claims a lane by `load`-then-CAS scan on `owner` (`FREE → token`) on first emit; a TLS guard's `Drop` stores `RETIRING`; the sink stores `FREE` once it observes `RETIRING && read == write`.

**Why.**
- True SPSC is why the threadpool's lane router is not reused: it maps every non-pool thread to lane 0, which would put the window thread, the present thread and every test-harness thread on one ring.
- The retire protocol closes the thread-exit hazard the research names as the most common lock-free-logger bug. It is trivially sound here: the ring is a `static` that never moves and records are POD with no `Drop`, so a retired-undrained lane leaks nothing.
- The producer caches the opposite cursor. The one published measurement on this question found padding **alone** made a ring *slower* — both threads still read the opposite cursor every operation — and only opposite-cursor caching *plus* padding moved throughput from ~32 to ~440 M ops·s⁻¹. We do both and treat padding as a hypothesis with an ablation bench, matching this repo's own `reference-componentpool-cache-stagger` lesson.

**Claim scan is `load`-then-CAS** (M10): `if owner.load(Relaxed) == FREE { try CAS }`. An unconditional `compare_exchange` over 128 lanes takes every occupied lane's line exclusive and invalidates up to 127 producers — the exact defect this repo already fixed at `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:36-51` ("load first, store once"). The scan additionally starts at `hash(thread_id) % MAX_LANES` so concurrent claimants do not convoy on lane 0.

**Alternatives rejected.** *Double-buffer + wholesale swap* (`EventBuffer::swap_and_flatten`) — needs a quiescence point that boot code, the present thread and a driver callback do not have. *One MPMC ring* — CAS on every push, reintroducing the contention the per-lane design removes by construction.

### Decision 5: Overflow drops, counts, and reports — with an Error-reserved tail and a synchronous fallback
**What.** Never block. On insufficient space: `dropped.fetch_add(1, Relaxed)`, `dropped_bytes.fetch_add(need, Relaxed)`, return. The sink emits one synthetic `boyko-W0102` per drain per lane and clears with `fetch_sub(observed)` — never `store`, because a producer may increment concurrently. The last `LANE_BYTES/8 = 2 KiB` is reserved for `Level::Error`; the limit is selected branchlessly on `level == Error`.

**Lane-exhaustion fallback** (M26): a thread that cannot claim a lane does **not** silently drop `Warn`/`Error`. It falls back to the synchronous channel (§Decision 9b) for those two levels only, and counts `Info`/`Debug`/`Trace` into `UNLANED_DROPPED`. The cost is paid only in the exhausted case, and a test harness that exhausts lanes therefore cannot lose a severe record.

**Why.** Blocking on `error!` inside a driver callback under a storm is a deadlock. Silent loss turns a logger into a source of false confidence — the exact class this campaign exists to kill.

**Alternatives rejected.** *Block-on-full* (spdlog's default) — a mutex by another name. *Overrun-oldest* — destroys the record that reported the cause in favour of the one that reported the consequence.

### Decision 6: One diagnostic-code registry, kept honest by seven mechanical checks
**What.** `crates/boyko_log/src/codes.rs` holds a single `codes! { … }` invocation generating: a `pub const` per code, a **dense** `static DIAGNOSTICS: [DiagInfo; N]` sorted by number, a dense `code_idx` per code (the `RATE` index — M12), and `explain()`. A literal `"boyko-…"` outside the registry is a build failure. Class is a **type** property: `warn!` takes `WarnCode`, `error!` takes `ErrorCode`, `PanicCode` is distinct — a class mismatch does not compile.

**The seven checks** live in `crates/boyko_log/tests/code_registry.rs` — an **integration** test, because `cargo test --workspace --lib` does not build `tests/`, a blind spot that cost this repo four commits.

| # | Check | Corpus (fixes B6) | Red state that must be demonstrated once |
|---|---|---|---|
| 0 | **Corpus is non-empty**: `files_scanned ≥ 500`, and the pinned sentinel `boyko-W1501` is found | all | point the walker at a wrong root → red |
| 1 | Numbers strictly increasing ⇒ no duplicates (also a `const _: () = assert!`) | registry | add a duplicate |
| 2 | `docs/diagnostics/<code>.md` exists, non-empty, has `## What happened` / `## Why` / `## How to fix` | `docs/diagnostics/` | delete a section heading |
| 3 | **No orphans**: every code identifier appears ≥1× in **`crates/**/src/**.rs` only — excluding `codes.rs` and excluding all of `docs/`** | `.rs` sources | register a code, emit it nowhere |
| 4 | **No undeclared**: every `boyko-[BEW]\d{4}` literal in any `.rs`/`.md` resolves to a registry entry | `.rs` + `.md` | write `boyko-W9003` in a doc |
| 5 | Every `W`/`E` code is observed by ≥1 test, with `tests/untested_codes.txt` (a **data file**, not code, excluded from its own scan) checked **in both directions** | `crates/**/tests/**`, `#[should_panic(expected=` | allowlist a code that has a test |
| 6 | Panic-class `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!` | `.rs` sources | emit a `B` code from a `warn!` |

**Why the corpus rules changed.** v1's check #3 was vacuous: check #2 *mandates* a doc file naming the code, and v1's scan included `.md`, so every registered code was trivially non-orphan. v1's check #5 was self-defeating: the allowlist named identifiers and lived inside the file being scanned, so the bidirectional leg fired on every entry. Both are corrected above, and check #0 closes the third failure in the same family — a walker that resolves its root badly scans zero files and reports zero orphans, green. rustc's tidy pins a sentinel for exactly this reason.

**Prior art.** None of Bevy / flecs / UE / Unity / spdlog / Quill / NanoLog ships a code registry. The prior art is compilers: rustc's numbered codes with a mandatory long-form `.md` and its eight tidy checks; Clang's named groups; MSVC's opaque numbers with per-code pages. rustc's experience is that the number is worthless without the mandatory explanation *and* the orphan check.

**Block map** — defined *around* existing occupancy, because codes are never renumbered:

| Block | Domain | Occupied today |
|---|---|---|
| `00xx` | ECS core / system params | `B0002` |
| `01xx` | `boyko_log` itself | new |
| `02xx` | `boyko_threadpool` | new |
| `03xx` | memory (`VmReservation`, `ComponentPool`) | new |
| `04xx`–`09xx` | components, query, change detection, events, assets, serialize | new |
| `11xx`–`14xx` | input, scene, physics, math/sdf | new |
| `15xx` | schedule sets & ordering | `W1501` |
| `18xx` | app / plugins | `B1801`, `B1802` |
| `20xx`–`27xx` | RHI, RHI-Vulkan, render, shaderdsl, UI, fontbake, image, GPU columns | new |
| `30xx` | host / runner | new |
| `90xx` | schedule **build** (historical) | `B9001`, `B9002`, `B9004`, `B9005`; **`B9003` permanent gap** |
| `91xx` | world binding | `B9101` |

The `15xx`/`90xx` split is a historical artifact, documented as such, and **must not be tidied**: renumbering would break the book, the `#[should_panic]` assertions and the never-reuse rule simultaneously.

### Decision 7: `Info`/`Debug`/`Trace` carry NO code; `Warn`/`Error` MUST
Different macro arities enforce it. A code is a promise of documentation, stability and an explanation; extending it to trace chatter makes the registry meaningless and check 2 unenforceable, and making it optional reproduces today's state — nine codes across thousands of diagnostics.

### Decision 8: Rate limiting is a declared property of the CODE, and `Once` degrades to a pure load *(fixes M11, M12)*
**What.** Each `W`/`E` code declares `Every` / `Once` / `EveryN(n)` / `MinIntervalMs(ms)` in the registry. State lives in `static RATE: [RateSlot; MAX_CODES]` indexed by the **dense `code_idx`** carried in `LogSite`, not by the code number (v1 indexed a 512-entry array with numbers up to 9101 — an out-of-bounds read on the hot path).

**`Once` after firing is a pure `Relaxed` load and a not-taken branch — no store, no RMW.**
```
Once:  if fired.load(Relaxed) { return; }          // steady state: load only
       if !fired.swap(true, Relaxed) { emit }      // exactly one RMW, ever
```
**Why this matters.** The audit found five hand-rolled latches with two implementations, one wrong: `crates/boyko_render/src/render_path_config.rs:311-313` and `:335-337` execute `swap(true, Relaxed)` on a shared line **inside an `#[inline]` per-frame reader, every frame forever** once the divergence holds. v1's replacement kept a per-frame `suppressed.fetch_add` — the same defect wearing a policy name. The `Once` count of suppressed occurrences is therefore **not reported**, and the record says so: `(further occurrences suppressed, uncounted)`. `EveryN`/`MinInterval` do RMW by necessity; they are opt-in and their cost is documented at the declaration site.

**Layout.** `RateSlot` is 64 B, one per cache line — four unrelated codes sharing a line (v1's 16 B slot) is false sharing between subsystems that have nothing to do with each other. `MAX_CODES = 512` ⇒ 32 KiB, in the same `.bss` regime as `LANES`.

### Decision 9: Measurement output does NOT go through the logger
`report!` is a separate, explicitly **synchronous** macro: formats on the calling thread, takes `OUT_LOCK`, writes one `write_all` to **stdout**, unbuffered, in order. It carries `VB-P1d` / `VB-P4` / `VB-SV0-S1.5` / the R0 table and nothing else.

**Why.** Those lines are a machine API parsed by 17 test files; `runner.rs:3084-3085` already documents them as byte-frozen. Routing them through an asynchronous, timestamp-merged, droppable channel would reorder them against libtest's output, make them droppable under load, and lose them on a crash. A benchmark whose output can vanish is not a benchmark.

### Decision 9b: The validation messenger stays synchronous — the v1 migration is WITHDRAWN *(fixes B8, resolves the E12 conflict)*
**What.** `crates/boyko_rhi_vulkan/src/debug.rs`'s callback does **not** move to a lane. It keeps a synchronous, ordered, never-dropped write on the same `OUT_LOCK`/`report!`-class channel that `[vk-validation]` uses today. The only change is the removal of the per-message `to_string_lossy()` allocation: the `CStr` bytes are written directly with one `write_all`. Counters are untouched — they are the gate's oracle.

**Why.** Verified this session at `scripts/golden.ps1:201,226`: the scan runs over the child's *merged* stdout+stderr file and prints `VALIDATION: clean (0 messages)` in green at zero. Today the message is on the wire before `vkQueueSubmit` returns. Behind a 16 KiB lane drained ≤ 8 ms later, three loss modes are all reachable *in exactly the runs the gate exists for*: a storm overflows the lane (a storm is what an error looks like); an error preceding a driver abort loses everything undrained; a rate policy suppresses. Each yields green. **Decision 9's own rule — a gate whose evidence can vanish is worse than no gate — applies here verbatim, and v1 violated it.**

**The E12 conflict is resolved, not finessed.** "No lock, no syscall" is a rule about **frame-hot paths**. A validation callback under an enabled validation layer is not one: validation is off by default, and when on, the run is already an order of magnitude slower. Losing the message costs more than the lock. `OUT_LOCK` is registered in `docs/HOT-PATH-EXCEPTIONS.md` with this argument, alongside the existing `UiParseReport` entry.

**What is *added*:** `boyko-E2101` (below) and the `LOG-CENSUS`, both of which make *absence* loud rather than making presence prettier.

### Decision 10: One dedicated sink thread; adaptive park; `Manual` mode for hermetic tests
**What.** Default `SinkMode::Thread`: one thread, started at `boot()`, draining all lanes, staging, sorting by `tsc`, formatting once, fanning out. Park policy is **adaptive**: `park_timeout(0)` — immediate re-drain — while any lane yielded records last pass; `park_timeout(8 ms)` when every lane was empty. Producers **never** unpark (that is a syscall per record); `flush()` and `shutdown()` do. `SinkMode::Manual` requires an explicit `drain()`; it exists for hermetic tests, CLI tools and the zero-alloc gate, and for nothing else.

**Throughput, stated rather than deferred** (M19). At the default geometry, a lane holds `16 KiB / ~40 B` ≈ 400 records; with immediate re-drain under load, per-lane sustained capacity is bounded by the sink's formatting rate, not by the park interval. Design number: **≥ 500 K records·s⁻¹ aggregate** with a `core::fmt` cost of ~1-2 µs per formatted record on one thread. Consequence, stated plainly: **a `trace!` inside a per-entity loop is lossy by construction** — at 15 ns/record a single producer can offer 66 M records·s⁻¹ against a consumer two orders of magnitude slower. Gate **`sink_sustained_rate`** at L3 measures the knee and must show a nonzero drop count above it; the plan is not allowed to ship with the knee unmeasured.

**Alternatives rejected.** *Drain from an ECS system at `Last`* — makes the consumer MPSC when combined with any other driver, and ties log liveness to a running schedule, so boot and shutdown diagnostics vanish. *Drain from the frame loop* — a syscall in the frame.

### Decision 11: `rdtsc` on x86_64 with a boot-time invariance probe
`Instant::now()` is ~25 ns/call and ~60 ns/pair on this box's QPC — *measured*, at `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:226-240`. A 60 ns clock inside a 15 ns budget is not a design. Tracy uses `rdtsc` guarded by an invariant-TSC check. AVX2 baseline ⇒ Haswell+ ⇒ invariant TSC since Nehalem.

**Stated as uncontrolled** (N30): `boyko-W0101` (invariant TSC absent) **has no reachable red state on any machine this engine targets.** It is a defensive branch, not a gate, and is listed in `tests/untested_codes.txt` with that reason — which is exactly what check 5's allowlist is for.

**Limitation printed by the sink**: this is a *host* clock, not comparable to a Vulkan GPU timestamp without `VK_EXT_calibrated_timestamps` (per Khronos, device timestamps "cannot be compared even across separate submits within the same run"). Cross-domain correlation is out of scope; it belongs to the profiling plan.

### Decision 12: No `LogHandle`; explicit `boot`/`shutdown`; `flush` never waits on a consumer that cannot answer *(fixes M16, M17)*
**What.**
- `boot(cfg) -> Result<(), LogBootError>`; state lives in process-lifetime statics. **There is no handle**, because v1's handle was `!Send + !Sync` (so it could not be a `Resource`, which requires `Send + Sync`) and its `Drop` would have shut the logger down at the end of `Plugin::build`.
- `shutdown()` is explicit, idempotent, called by `App` teardown and by the process-exit path.
- `flush() -> FlushResult` reads `SINK_STATE` **first**: `NotBooted` / `Manual` / `Exited` ⇒ return `FlushResult::NoConsumer` immediately. Only in `Running` does it bump `FLUSH_REQ`, unpark, and spin to a 2 s deadline, after which it direct-writes `boyko-E0105` and returns `FlushResult::TimedOut`.
- `shutdown()` sets an exit flag, unparks, then spins on `SINK_EXITED` to a 2 s deadline. **There is no `join`-with-timeout, because `std::thread::JoinHandle::join` does not have one** — v1 asserted a facility std does not provide, in the one place it promised "no new hang class". On timeout the thread is **detached** and `boyko-E0108` is written synchronously.

**Why it matters.** This repo has dozens of `#[should_panic(expected = "boyko-B…")]` tests and a panic hook that flushes. With v1's unconditional 2 s deadline, every such test in an unbooted binary would have paid the full timeout — a self-inflicted 60-second test suite.

### Decision 13: `fmtv` is deleted; `dsp!` formats in argument position *(fixes B4, and B2's second half)*
**What.** v1's `fmtv(&x)` ran user `Display::fmt` *while the ring tail held a partially-written record and `write` had not advanced*. A nested emit from that `Display` — or from anything it calls — would overwrite the outer record and publish one `len` for two interleaved payloads, decoded by the wrong function pointer. An unwind through the same window left the ring in the same state. The SAFETY clause "two producers on one lane is unrepresentable" is a proof about *threads* and does not touch this.

**Replacement.**
```rust
/// Renders `Display` into a caller-owned stack buffer and yields `&str`.
/// Expands in ARGUMENT POSITION, so it runs to completion BEFORE `emit_impl`
/// is called and before any lane state is touched. Overflow truncates and
/// sets STR_TRUNCATED; it can never overrun a ring.
#[macro_export] macro_rules! dsp { ($e:expr) => {...}; ($e:expr, $n:literal) => {...} }
// warn!(Render, codes::W2201, "material {} rejected", dsp!(mat_id));
```
Rust evaluates arguments before the call, and the temporary lives to the end of the enclosing statement, so the `&str` is valid for the whole emit. **Nothing between lane acquisition and the `Release` store can call user code**, because encoding operates on already-materialised POD and `&str`. Backed by `debug_assert!(!IN_EMIT.replace(true))` in `emit_impl` — the guard exists to catch a future violation, not because one is representable today.

**Trade-off.** A `Display` costs `core::fmt` at the call site. That is the honest price and it is legible in the source, which is precisely why v1's invisible version was worse.

### Decision 14: `CEILINGS` is the single owner of the runtime level; `LogFilter` is deleted *(fixes M14)*
v1 had `LogFilter { ceilings: [Level; 256], dirty: bool }` mirroring `CEILINGS`, synced by a hand-rolled flag — two sources of truth for one datum, with a public `set_target_level()` writing only one of them, so the next unrelated `dirty` flip would silently push a stale value over the live one. It also re-offended the "capability/state is not a bare bool" rule.

**Replacement.** `CEILINGS` is authoritative. The UI and any system read and write it through `boyko_log::target_level(id)` / `set_target_level(id, lvl)`. There is no ECS mirror, no `dirty`, and no sync system. Change detection is not needed because there is nothing to reconcile.

### Decision 15: Engine target IDs are compile-time-unique; downstream IDs are boot-checked *(fixes M15)*
v1 hand-assigned `id = $id:literal` per target with a boot collision check that could only fire if *both* colliders registered — and nothing forced registration, so an unregistered target still gated against `CEILINGS[ID]` and never tripped.

**Replacement.** Engine targets 0..=95 are declared in **one** `targets! { … }` table in `boyko_log`, generating each unit type and a `const _: () = assert!(strictly_increasing)` — collisions in the engine namespace **do not compile**. Registry check 7 (added to the same integration test) asserts every `LogTarget` impl in the workspace resolves to a table row. Downstream IDs 96..=255 keep `define_target!` with the boot check `boyko-E0104`; the honour system is confined to code we do not own, and that is stated.

`boyko_utils::TypeIntern` is **not** usable here: `ID` must be a `const` for gate (a) to fold, and `boyko_utils` depends on `boyko_log`, not the reverse. Recorded so the next reader does not re-derive it.

---

## Data structures

```rust
// ─────────────────────────── boyko_log/src/level.rs ───────────────────────────

/// Severity. Lower value = more severe. `Off` is a THRESHOLD value only; no
/// record is ever emitted at `Off`, which is why `.bss`-zero == "all disabled".
#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Off = 0, Error = 1, Warn = 2, Info = 3, Debug = 4, Trace = 5 }
```
No `Fatal`: a fatal condition panics through an existing `#[cold] fn … -> !` helper and the panic hook flushes. Two spellings of "die" is one too many.

```rust
// ─────────────────────────── boyko_log/src/site.rs ────────────────────────────

/// Immutable per-call-site metadata. One `'static` per macro expansion,
/// referenced by pointer, so the record pays 8 B instead of re-carrying
/// file/line/fmt/code. Dereferenced ONLY by the sink.
pub struct LogSite {
    pub target:   TargetId,     // 2
    pub level:    Level,        // 1
    pub class:    u8,           // 1  b'B' | b'E' | b'W' | 0
    pub code:     u16,          // 2  printed number; 0 when the level has none
    pub code_idx: u16,          // 2  DENSE registry index — the RATE[] index (M12)
    pub line:     u32,          // 4
    pub file:     &'static str, // 16
    pub fmt:      &'static str, // 16 the format literal, printed by the sink
    /// Monomorphised per ARGUMENT-TUPLE type; identical tuples share one
    /// instantiation. Cold: called on the sink thread only.
    pub decode:   unsafe fn(*const u8, usize, &mut LogFormatter),
}

// ────────────────────────── boyko_log/src/record.rs ───────────────────────────

/// 20 B, PACKED — the ring is byte-oriented and records are never aligned, so
/// alignment padding would be pure waste. `code`, `level` and `lane` are NOT
/// duplicated here (v1 did): the sink holds the site pointer and knows the lane
/// it is draining, so three bytes bought nothing (N32).
///
/// `site` is a REAL pointer field so provenance round-trips; a null `site` is
/// the PAD sentinel (§Algorithms A5).
#[repr(C, packed)]
struct RecordHeader {
    site:  *const LogSite, // 8
    tsc:   u64,            // 8  raw rdtsc; scaled by the sink
    len:   u16,            // 2  TOTAL record bytes incl. header — the walk step
    flags: u8,             // 1  STR_TRUNCATED | SUPPRESSED_FOLLOWS | TOO_LARGE
    _pad:  u8,             // 1
}
const HEADER_BYTES: usize = 20;
const _: () = assert!(core::mem::size_of::<RecordHeader>() == HEADER_BYTES);

/// Hard cap, checked at RUNTIME against `encoded_len` — not asserted as
/// unreachable. Five 256-byte `&str`s exceed 1 KiB, and the plan allows
/// 12-element tuples, so v1's "unreachable, debug_assert'ed" described a
/// debug-build panic reachable from safe user code (N29). Over-cap records are
/// DROPPED with the `TOO_LARGE` flag and counted, in every profile.
const MAX_RECORD_BYTES: usize = 2048;
const MAX_STR_BYTES:    usize = 256;

// ─────────────────────────── boyko_log/src/lane.rs ────────────────────────────

/// One SPSC byte ring. THREE cache-line partitions, not two: statistics are
/// written by the producer AND cleared by the consumer, so they are a third
/// partition — putting them on the producer line (v1) meant the consumer
/// RMW'd the producer's hot line on every drain, most often precisely during
/// the drop storm the counters exist to measure (M9).
#[repr(C, align(64))]
pub(crate) struct Lane {
    // ── line 0: PRODUCER-owned ───────────────────────────────────────────────
    /// Absolute wrapping byte counter; `off = write & MASK`. `Release`-stored;
    /// this store is the happens-before edge that publishes the payload.
    write:        AtomicU32,   // 4
    /// Producer's private cache of `read`; refreshed (Acquire) only when the
    /// cached value says "full". This half is what buys throughput; padding
    /// alone measured SLOWER (Decision 4).
    read_cached:  Cell<u32>,   // 4
    _pad0:        [u8; 56],

    // ── line 1: CONSUMER-owned ───────────────────────────────────────────────
    read:         AtomicU32,   // 4  Release-stored AFTER staging (B1)
    write_cached: Cell<u32>,   // 4
    _pad1:        [u8; 56],

    // ── line 2: SHARED (producer adds, consumer subtracts / frees) ───────────
    dropped:       AtomicU32,  // 4  Relaxed; cleared with fetch_sub(observed)
    dropped_bytes: AtomicU32,  // 4
    /// FREE / RETIRING / owner token. `load`-then-CAS on claim (M10).
    owner:         AtomicU32,  // 4
    _pad2:         [u8; 52],

    // ── payload ──────────────────────────────────────────────────────────────
    buf: UnsafeCell<[MaybeUninit<u8>; LANE_BYTES]>,
}
const _: () = assert!(core::mem::align_of::<Lane>() == 64);
const _: () = assert!(core::mem::offset_of!(Lane, read)    == 64);
const _: () = assert!(core::mem::offset_of!(Lane, dropped) == 128,
    "statistics are a third partition: producer adds, consumer subtracts");

// SAFETY (manual Sync for Lane):
//   1. WRITE side: exactly one thread — the one whose CAS moved `owner` from
//      FREE to its token — ever writes `buf` or `write`. A second claimant's
//      CAS fails, so two PRODUCER THREADS on one lane is unrepresentable.
//   1b. Re-entrant emit on ONE thread is separately excluded: no user code can
//      run between lane acquisition and the `Release` store, because `dsp!`
//      runs in argument position and encoding operates on POD and `&str` only
//      (Decision 13). `debug_assert`ed by IN_EMIT.
//   2. READ side: exactly one thread (the sink) reads `buf` and writes `read`.
//      `SinkMode::Manual` documents `drain()` as single-caller and asserts it.
//   3. Payload visibility: bytes written before `write.store(_, Release)` are
//      visible to a thread observing that value via `Acquire`. The consumer
//      never reads past its observed `w`, AND never advances `read` over bytes
//      it has not yet copied out (Algorithms C).
//   4. Retire: the TLS guard's `Drop` runs on the producer thread after its
//      last write; the consumer stores FREE only after observing
//      `RETIRING && read == write`, so no producer write can follow a reclaim.
unsafe impl Sync for Lane {}

pub(crate) const MAX_LANES:  usize = 128;        // option_env!-overridable
pub(crate) const LANE_BYTES: usize = 16 * 1024;  // power of two: MASK arithmetic
const ERROR_RESERVE: usize = LANE_BYTES / 8;
const _: () = assert!(LANE_BYTES.is_power_of_two());

pub const LANE_ARRAY_LEN: usize = if (GLOBAL_CEILING as u8) == 0 { 0 } else { MAX_LANES };
static LANES: [Lane; LANE_ARRAY_LEN] = [Lane::NEW; LANE_ARRAY_LEN];

// ────────────────────────── boyko_log/src/target.rs ───────────────────────────

#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)] pub struct TargetId(pub u16);

pub trait LogTarget: 'static {
    const NAME: &'static str;
    const ID: TargetId;           // const: required for gate folding
    const STATIC_CEILING: Level;  // const: Unreal's compile-time ceiling
}
pub const MAX_TARGETS: usize = 256;

/// One `Relaxed` byte load per enabled-check. 256 B; one line touched.
/// `.bss`-zero == `Off` == disabled until boot arms it.
static CEILINGS: [AtomicU8; MAX_TARGETS] = [const { AtomicU8::new(0) }; MAX_TARGETS];

/// ONE pointer publishes name+len together — v1's `AtomicPtr<u8>` lost the
/// length of a `&'static str` (N28). Read by the sink only.
pub struct TargetInfo { pub name: &'static str }
static TARGETS: [AtomicPtr<TargetInfo>; MAX_TARGETS] = ...;

// ────────────────────────── boyko_log/src/codes.rs ────────────────────────────

#[repr(transparent)] #[derive(Clone, Copy)] pub struct WarnCode  { num: u16, idx: u16 }
#[repr(transparent)] #[derive(Clone, Copy)] pub struct ErrorCode { num: u16, idx: u16 }
#[repr(transparent)] #[derive(Clone, Copy)] pub struct PanicCode { num: u16, idx: u16 }
// Distinct newtypes ⇒ `warn!(T, codes::E2101, ..)` does not compile.

#[repr(u8)] pub enum RatePolicy { Every, Once, EveryN(u16), MinIntervalMs(u16) }

pub struct DiagInfo {
    pub number: u16, pub class: u8,
    pub summary: &'static str,   // one line, embedded, printable from a message
    pub rate: RatePolicy,
    pub doc: &'static str,       // "docs/diagnostics/W1501.md" — check 2's target
}
static DIAGNOSTICS: [DiagInfo; N];       // dense, sorted; index == code_idx
const MAX_CODES: usize = 512;

/// 64 B — one code per cache line. v1's 16 B slot false-shared four unrelated
/// subsystems' codes on one line (Decision 8).
#[repr(C, align(64))]
struct RateSlot { fired: AtomicBool, count: AtomicU32, last_tsc: AtomicU64, suppressed: AtomicU32, _pad: [u8; 43] }
static RATE: [RateSlot; MAX_CODES];      // 32 KiB .bss

// ─────────────────── boyko_ecs seam: the ECS-visible surface ──────────────────

/// The durable, displayable log. Backed by the engine's own storage — a
/// `VmReservation`-backed byte column, NOT a `Box<[u8]>` heap side-store, which
/// is the shape Principle 0 was re-stated to forbid even inside a `Resource`
/// (M13). Fixed capacity, reserved at plugin build, never grows.
#[derive(Resource)]
pub struct LogRing {
    lines: VmColumn<LogLine>,  // engine storage
    arena: VmColumn<u8>,       // engine storage
    head: u32, len: u32, arena_cursor: u32,
}
#[repr(C)] pub struct LogLine { start: u32, len: u16, code: u16, level: u8, target: u8 }

/// Monotonic counters; zero per-frame allocation. Mirrors HostFrameStats.
#[derive(Resource, Clone, Copy, Default)]
pub struct LogStats {
    pub emitted: u64, pub dropped: u64, pub dropped_bytes: u64,
    pub suppressed: u64, pub unlaned_dropped: u64,
    pub lanes_claimed: u32, pub lanes_retired: u32,
}
// NOTE: there is no `LogFilter`. `CEILINGS` is the single owner (Decision 14).
```

---

## Public API

```rust
// ── declaration ───────────────────────────────────────────────────────────────
/// ENGINE targets: one central table, IDs 0..=95, uniqueness proven at COMPILE
/// time by a strictly-increasing const assert (Decision 15).
targets! {
    (0,  Ecs,        "ecs",           Level::Info),
    (1,  Schedule,   "schedule",      Level::Info),
    (2,  Threadpool, "threadpool",    Level::Warn),
    // …
}
/// DOWNSTREAM targets: IDs 96..=255, boot-checked (`boyko-E0104`).
#[macro_export] macro_rules! define_target { ($vis:vis $Ty:ident, name=$n:literal, id=$id:literal, ceiling=$c:expr) => {...} }

// ── emission ──────────────────────────────────────────────────────────────────
#[macro_export] macro_rules! error { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! warn  { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! info  { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! debug { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! trace { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }

/// Renders a `Display` into a caller-owned stack buffer, in ARGUMENT POSITION.
#[macro_export] macro_rules! dsp { ($e:expr) => {...}; ($e:expr, $n:literal) => {...} }

/// SYNCHRONOUS, ordered, never dropped, never rate-limited, no code, stdout,
/// under OUT_LOCK. Machine-parsed measurement lines ONLY (Decision 9).
#[macro_export] macro_rules! report { ($fmt:literal $(, $a:expr)*) => {...} }

/// SYNCHRONOUS, ordered, never dropped, stderr, under the SAME OUT_LOCK.
/// Gate-oracle channel: the Vulkan messenger and the lane-exhaustion fallback
/// for Warn/Error (Decisions 9b, 5). Not for ordinary diagnostics.
pub fn write_oracle_line(prefix: &str, body: &[u8]);

// ── values ────────────────────────────────────────────────────────────────────
pub trait LogValue: private::Sealed {
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}
// impls: {i,u}{8,16,32,64,128}, f32, f64, bool, char, &'static str, &str.

// ── lifecycle ─────────────────────────────────────────────────────────────────
pub struct LogConfig {
    pub sink_mode: SinkMode,            // Thread (default) | Manual
    pub console:   Option<ConsoleSink>, // stderr; stream/colour/level floor
    pub file:      Option<FileSink>,    // path, max_bytes
    pub callback:  Option<CallbackSink>,// fn(&FormattedRecord, *mut ()) + ctx
    pub ecs_ring:  bool,
    pub default_ceilings: [Level; MAX_TARGETS],
}
pub enum LogBootError { AlreadyBooted, TargetIdCollision { id: u16, a: &'static str, b: &'static str }, SinkOpen(std::io::Error) }
pub enum FlushResult { Flushed, NoConsumer, TimedOut }

pub fn boot(cfg: LogConfig) -> Result<(), LogBootError>;  // no handle (Decision 12)
pub fn shutdown();                                        // idempotent
pub fn flush() -> FlushResult;                            // never waits on a dead consumer
pub fn drain();                                           // SinkMode::Manual only
pub fn target_level(id: TargetId) -> Level;
pub fn set_target_level(id: TargetId, lvl: Level);        // single owner (Decision 14)
pub fn explain(code: u16) -> Option<&'static DiagInfo>;
pub fn census() -> CensusIter<'static>;                   // MEASURED / UNPROVEN per target
pub fn name_current_thread(name: &'static str);           // cosmetic, cold, once

// ── ECS seam (boyko_ecs) ──────────────────────────────────────────────────────
pub struct LogPlugin { pub config: LogConfig }
impl Plugin for LogPlugin { fn build(&self, app: &mut App); }
// inserts LogRing / LogStats; adds `log_drain_system` to `Last` (ECS ring feed
// only — the sink thread owns the byte sinks). Registers `shutdown` on teardown.
```

No `Vec`, `Box<dyn>`, `HashMap` or internal type appears in any signature.

---

## Algorithms for critical paths

### A. `emit` — the producer hot path

```
1. GATE (inlined into the caller)
   a. T::STATIC_CEILING       >= LVL    — const, folded
   b. $crate::GLOBAL_CEILING  >= LVL    — const, folded
   c. CEILINGS[T::ID].load(Relaxed) >= LVL  — 1 B L1 load + cmp
   Fail ⇒ nothing. Arguments NEVER evaluated (&& short-circuit).
   [Arguments, incl. any `dsp!`, are evaluated HERE, before step 2.]

2. RATE (Warn/Error only)
   Once && fired.load(Relaxed)  ⇒ return          // pure load, no store (M11)
   EveryN / MinInterval         ⇒ policy RMW
   Every                        ⇒ skip

3. LANE  = TLS `MY_LANE`. Cold miss ⇒ claim (§B). Claim failure ⇒
   Warn/Error: write_oracle_line() (synchronous fallback, M26); else
   UNLANED_DROPPED.fetch_add(1, Relaxed); return.

4. SIZE   need = HEADER_BYTES + args.encoded_len()          // runtime (B2)
   need > MAX_RECORD_BYTES ⇒ drop + TOO_LARGE count; return  // N29

5. SPACE + WRAP  (records never straddle — B3)
   w    = write.load(Relaxed);  off = w & MASK
   tail = LANE_BYTES - off
   // Rule shared VERBATIM by producer and consumer:
   if tail < HEADER_BYTES { pad = tail }                     // implicit wrap
   else if tail < need    { pad = tail }                     // explicit PAD record
   else                   { pad = 0 }
   limit = LANE_BYTES - if level == Error { 0 } else { ERROR_RESERVE }  // branchless
   free  = limit - (w - read_cached)                          // one slot reserved
   if free < pad + need {
       read_cached = read.load(Acquire); recompute free;
       if still short { dropped.fetch_add(1); dropped_bytes.fetch_add(need); return }
   }
   if pad >= HEADER_BYTES { write PAD header (site = null, len = pad) at off }
   w += pad; off = w & MASK       // now off == 0 or tail >= need

6. WRITE   write_unaligned(off, RecordHeader{ site, tsc: rdtsc(), len: need, flags })
           args.encode(off + HEADER_BYTES)
7. PUBLISH write.store(w + need, Release)
```

- **Complexity** O(1); O(len) memcpy for inline `&str`.
- **Cache** strictly sequential streaming writes into the ring tail. Working set: `CEILINGS` line, producer line, 1-2 ring-tail lines. `LANES` and `CEILINGS` have compile-time-known addresses — no pointer chase.
- **Branching** 3 predicted-not-taken gates + 1 rate + 1 wrap + 1 space. `limit` is a branchless select on `level == Error`.
- **Inlining** steps 1-2 `#[inline]` (must fold). Steps 3-7 in `#[inline(never)] fn emit_impl<A: LogArgs>` — monomorphised per argument-tuple type. Blanket `#[inline(always)]` would replicate ~60 instructions at every site and bloat L1i, which principle 7 forbids on measurement grounds.

### B. Lane claim / retire

```
CLAIM (cold, once per thread):
  start = spread(thread_id) % MAX_LANES
  for i in start..start+MAX_LANES (mod):
    if LANES[i].owner.load(Relaxed) != FREE { continue }        // load first (M10)
    if LANES[i].owner.compare_exchange(FREE, token, Acquire, Relaxed).is_ok() {
        MY_LANE.set(i); install TLS guard; return i }
  ⇒ exhausted: MY_LANE.set(NONE)   // step 3 above handles the fallback

RETIRE (TLS guard Drop, producer thread, after its last write):
  owner.store(RETIRING, Release)

RECLAIM (consumer, per drain, after staging):
  if owner.load(Acquire) == RETIRING && read == write { owner.store(FREE, Release) }
```

### C. Drain — staged copy BEFORE the free *(fixes B1)*

```
STAGE_BYTES = 256 KiB (preallocated, sink-owned, reused every drain)

for each lane:
  w = write.load(Acquire); r = read
  while r != w and staging has room:
      off = r & MASK
      if LANE_BYTES - off < HEADER_BYTES { r += LANE_BYTES - off; continue }  // shared wrap rule
      hdr = read_unaligned::<RecordHeader>(buf + off)     // TYPED copy: provenance-preserving
      if hdr.site.is_null() { r += hdr.len; continue }    // PAD
      copy_nonoverlapping(buf + off + HEADER_BYTES, stage + s + HEADER_BYTES, hdr.len - HEADER_BYTES)
      write::<RecordHeader>(stage + s, hdr)               // TYPED write: provenance-preserving
      push (hdr.tsc, lane, s) into the preallocated index scratch
      s += hdr.len; r += hdr.len
  read.store(r, Release)          // ← ONLY NOW is the ring space published free
  if dropped > 0 { synthesise boyko-W0102; dropped.fetch_sub(observed) }
  reclaim if RETIRING && r == w
sort_unstable_by_key(tsc)         // over preallocated 16 B triples; no allocation
for each: (*site.decode)(stage + s + HEADER_BYTES, len, &mut fmt)   // reads STAGING
one write_all per byte sink; push formatted lines to the ECS handoff ring
```

**Why the order changed.** v1 advanced `read` at step 3 and decoded at step 6 from an `offset` **into the ring** — bytes the producer was licensed to overwrite in between. The sink would then read a torn header and call `decode` through 8 arbitrary bytes reinterpreted as a function pointer. v1's tests could not see it: both the ordering test and the overflow test drive a quiesced producer. The staged copy makes the window structurally absent, and adds a bound: a drain never stages more than `STAGE_BYTES`, so a hot lane is drained across several passes rather than in one unbounded burst.

**Provenance.** The header — the only field carrying a pointer — is moved by a **typed** `read_unaligned`/`write` pair, never by a byte memcpy, so `site`'s provenance round-trips by construction rather than by relying on per-byte provenance tracking. Payloads are pointer-free POD and move by `copy_nonoverlapping`. Gated by Miri under Tree Borrows (test 14).

- **Complexity** O(R log R) per drain for R records, entirely off the frame thread.
- **Stated limitation** cross-lane ordering is *approximate*: a record written after lane A's snapshot may carry an earlier `tsc` than one already staged from lane B. Inherent to any non-blocking merge (Quill has the same property) and printed in the sink's header line.

### D. `flush`

```
1. match SINK_STATE.load(Acquire) {
       NotBooted | Manual | Exited => return FlushResult::NoConsumer,   // M16
       Running => {}
   }
2. epoch = FLUSH_REQ.fetch_add(1, AcqRel) + 1
3. unpark(sink)
4. spin_backoff until FLUSH_ACK.load(Acquire) >= epoch, deadline = now + 2 s
5. on timeout: write_oracle_line("boyko-E0105: log flush timed out"); TimedOut
```
Step 1 is what keeps `#[should_panic]` tests in an unbooted binary at zero cost. Step 5 is non-negotiable: the profiling audit's central finding is that an unbounded blocking wait converts an instrumentation gap into an unkillable hang, and that this repository has no kill-after-timeout pattern to borrow. This design does not add a second one.

---

## Multithreading model

| Datum | Sharing | Ordering | Why |
|---|---|---|---|
| `Lane::buf` | SPSC | none (guarded by `write`) | payload published by the cursor's Release |
| `Lane::write` | P→C | `Release` / `Acquire` | the happens-before edge for the payload |
| `Lane::read` | C→P | `Release` (after staging) / `Acquire` | frees space only once bytes are copied out (B1) |
| `read_cached` / `write_cached` | private `Cell` | none | the half that actually buys throughput |
| `Lane::owner` | MPMC (claim) | `load` then CAS `Acquire`; `Release` on retire/free | contended once per thread lifetime |
| `dropped`, `dropped_bytes` | P adds, C subtracts | `Relaxed` | own cache line; `fetch_sub(observed)` never loses a concurrent add |
| `CEILINGS[i]` | MP-read, rare write | `Relaxed` | a stale ceiling for one record is documented as acceptable |
| `RATE[idx].fired` | MP | `Relaxed` load; one lifetime `swap` | steady state is a pure load (M11) |
| `FLUSH_REQ` / `FLUSH_ACK` / `SINK_STATE` / `SINK_EXITED` | 2-way | `AcqRel` / `Acquire` | completion must be observed, not guessed |
| `OUT_LOCK` | MP spin | `Acquire`/`Release` | only `report!`, the messenger and the exhaustion fallback |
| Sink array, `LogConfig` | boot-published | one `Release` at boot | never mutated after boot |

**Data-race freedom.** No lane has two producer *threads* (CAS from FREE confers exclusive write rights). No lane has two producers *re-entrantly* on one thread (no user code runs inside the open window — Decision 13, `debug_assert`ed). No lane has two consumers (one sink thread; `Manual` asserts single-call). Payload visibility rests on the `Release`/`Acquire` cursor pair, and the consumer never reads past its observed `w` **nor advances `read` over bytes it has not staged**. Reclaim is ordered by `RETIRING` being stored after the producer's last write and observed only after the consumer has drained to `write`.

**`Send`/`Sync`.** `Lane: Sync` via the documented manual impl. `TargetId`, `WarnCode`, `ErrorCode`, `PanicCode`, `Level`: `Copy + Send + Sync`. `LogRing`, `LogStats`: `Send + Sync`, ordinary `Resource`s. **No `!Send` handle exists** (Decision 12).

---

## What this system can and cannot substitute for — the sync-validation confrontation

The audit established, from source, that `is_instance_extension_present(global, VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME)` at `crates/boyko_rhi_vulkan/src/device.rs:2110` queries `vkEnumerateInstanceExtensionProperties` with `pLayerName == NULL`, which returns the implementation's own extensions plus implicitly-enabled layers' — never those of an explicitly-requested layer. `VK_EXT_validation_features` is supplied by `VK_LAYER_KHRONOS_validation`. Therefore `sync_validation_available` is always false, the `VkValidationFeaturesEXT` node is never chained, and `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT` is never requested. This matches the measured fact that a genuine missed barrier produced **19 messages (= baseline), zero `SYNC-HAZARD`, and a byte-identical golden**.

**A logger is a transport. It changes where a message goes and has no opinion on whether the message exists.** Therefore, explicitly:

1. **Routing validation output through `boyko_log` does not make a missed barrier visible.** Not one hazard becomes detectable. Any sentence of the form "the validation layer would have told us" remains false on this machine, before and after this plan.
2. **It would make the deadness easier to miss** — a clean, colour-coded log reads as evidence of a clean run. **And it would make the evidence droppable.** That is why v1's migration is withdrawn (Decision 9b): the channel stays synchronous, and the only change is deleting an allocation.
3. **The logger's legitimate contribution is to make ABSENCE loud.** Two mandatory mechanisms:
   - **`boyko-E2101`** — emitted at boot when validation is *requested* but the features node was not chained. A liveness claim about the channel, not about the frame. It is a **two-sided gate** (§Metrics G7): it must fire on this machine today, and it must **not** fire in a fixture where the node is chained.
   - **The `LOG-CENSUS`** — at every `flush()` and at shutdown, one line per armed target: `LOG-CENSUS target=vk-validation level=Warn records=0 dropped=0 status=UNPROVEN`. A target that has never delivered a record is `UNPROVEN`, never `clean`. **Extended per the critic: `status=UNPROVEN` also covers a target whose records were dropped** — `records=N dropped=M status=UNPROVEN(lossy)`. This is the direct translation of "a gate that cannot fail is a defect" into the logging system.
4. **The underlying fix is out of scope and belongs to the RHI** (`pLayerName = "VK_LAYER_KHRONOS_validation"`). This plan does not fix it, does not claim to, and adds `E2101` so the gap has a coded, documented, greppable identity until it is fixed.
5. **What no logger can substitute for:** an absent source. Sync-validation, the layer's `INFO`/`VERBOSE` severities (structurally excluded at `debug.rs:125-126`), GPU pipeline statistics (hard-wired to `0` at `rhi_impl/device.rs:1013`), and per-system CPU timing (does not exist) are four separate named gaps, and none is closed by this plan.

---

## Integration

### New
- **Crate `boyko_log`** — `level.rs`, `target.rs`, `site.rs`, `record.rs`, `lane.rs`, `codes.rs` (generated), `rate.rs`, `sync_out.rs` (`OUT_LOCK`, `report!`, `write_oracle_line`), `sink/{mod,console,file,callback,ecs}.rs`, `macros.rs`, `tsc.rs`, `build.rs`.
- **`docs/diagnostics/<code>.md`** — one per code; check 2's target; published by `doc-writer`.
- **`crates/boyko_ecs/src/ecs/core/log/`** — `LogPlugin`, `LogRing`, `LogStats`, `log_drain_system`.
- **`crates/boyko_log/tests/`** — `code_registry.rs` (7 checks), `print_census.rs` (the tidy-style print ban), `untested_codes.txt` (data).
- **`docs/HOT-PATH-EXCEPTIONS.md`** — one new entry: `OUT_LOCK`.

### Migration ledger — machine-generated, not hand-tabled *(fixes M22)*
v1's table covered ~14 files against a measured **179 occurrences across 36 files** under `crates/**/src/**` (this session's grep). The migration is driven by a generated ledger, `docs/diagnostics/PRINT-CENSUS.md`, regenerated by the same walker that backs the enforcement test, with every site classified into exactly one of:

| Class | Count (measured) | Disposition |
|---|---|---|
| CLI binary stdout (`boyko_shaderdsl/src/bin/*`) | 58 | **Keep.** One crate-level `#![allow]` + rationale per bin, not per site. |
| In-`src` `#[cfg(test)]` modules | 23 (`sdf_math/brick/tests.rs` 3, `rhi_vulkan/compute/tests.rs` 16, `physics/solver/colored_tests.rs` 4) | **Keep.** Excluded by the walker's `#[cfg(test)]`-region rule. |
| Measurement lines (`runner.rs` ~20) | 20 | → **`report!`**, byte-frozen (Decision 9). |
| Validation messenger (`debug.rs` 1) | 1 | → **`write_oracle_line`**, synchronous (Decision 9b). |
| Everything else (production diagnostics) | ~77 | → `error!`/`warn!`/`info!` with codes. |

Named production files v1 omitted and that the ledger covers: `rhi_vulkan/present/targets.rs` (7), `render/texture.rs` (7), `app/{host_dump,hzb_dump,vg_census_dump,vb_probe_dump,vb_cull_probe}.rs` (14), `app/plugins.rs` (3), `app/gpu_scene/mod.rs` (3), `app/host.rs` (2), `physics/soft/self_collision.rs` (3), `ui/layout.rs` (2), `rhi/handle.rs` (2), `serialize/load.rs` (2), `ecs/asset/server.rs` (1), `ecs/ecs_master/system_api.rs` (1), `image/{png,inflate}.rs` (2), `render/{bindless,mesh_geometry_table,light_system,render_path_config,gpu_system}.rs` (8), `threadpool/worker.rs` (1), `ecs/schedule/schedule_builder.rs` (2).

### Behaviour changes worth naming

| Site | Change |
|---|---|
| `boyko_threadpool/src/worker.rs:157-168` | `abort_on_task_panic` → `error!(codes::E0201, …)` + **`flush()` before `abort()`** |
| `boyko_ecs/.../schedule_builder.rs:1334-1350` | `warn_if_empty` → `warn!(Schedule, codes::W1501, …)`; text normalised (substring-safe) |
| `boyko_ecs/.../params/diagnostics.rs:53` | `error[boyko-B0002]:` → `boyko-B0002:` (substring-safe) |
| `boyko_ecs/.../events/event_buffer.rs` | overflow emits `warn!(codes::W0701, type_name, lane, attempted, dropped)`; the `Result` is unchanged. Those four fields currently exist only inside an `EcsError` nobody reads |
| `boyko_ecs/.../query_type_registry.rs:124-144` | `warn!(codes::W0501)` at 75 % occupancy; the terminal `panic!` gains `B0502`. 1023 silent mints then a process kill is not a diagnostic |
| `boyko_rhi_vulkan/src/debug.rs:104-116` | **stays synchronous**; only the `to_string_lossy()` allocation is removed. Counters untouched |
| `boyko_rhi_vulkan/src/device.rs:2110` | add `error!(codes::E2101)` when validation is requested but the node was not chained |
| `boyko_rhi_vulkan/src/device.rs:3100,3158,3189` | drop `#[cfg(debug_assertions)]` → `warn!(codes::W2102)`, `RatePolicy::Once`. **A release-build degrade-to-disabled must be observable** — settling the two-doctrine conflict in favour of `boyko_app/src/host.rs:228-233`'s written argument |
| `boyko_render/src/render_path_config.rs:311-337` | delete both hand-rolled latches (the per-frame `swap` bug) → `warn!` + `Once` |
| `boyko_render/src/light_system.rs:397,456` | delete latches → `warn!(codes::W2201, dropped_count)`; **the dropped count is now reported**, which the one-shot latch never did |
| `boyko_render/src/{bindless,mesh_geometry_table}.rs` | ad-hoc `"WARN: "` → `warn!(codes::W2202)`; keep `debug_assert!(false)` |
| `boyko_render/src/gpu_system.rs:399-404` | → `error!(codes::E2203)`. The `System` trait's missing error channel stops mattering: the logger is a side channel available from any thread |
| `boyko_image/src/{png.rs:206, inflate.rs:656}` | → `warn!(codes::W2601/W2602)`; decoding continues |
| `boyko_app/src/runner.rs` (20 measurement sites) | → `report!`, text unchanged |

### Enforcement *(fixes M23)*
**Primary: an in-repo tidy-style test**, `crates/boyko_log/tests/print_census.rs`, which walks `crates/*/src/**.rs`, excludes `src/bin/` and `#[cfg(test)]` regions, asserts a non-empty corpus, and fails on any `println!`/`eprintln!`/`print!`/`eprint!` outside `tests/print_allowlist.txt` — with the allowlist checked in **both** directions. We own it, and it can be shown red in one line.

**Secondary: `clippy.toml`'s `disallowed-macros`**, added only after a **shown-red canary**: `clippy.toml:21-25` records, empirically, that clippy *silently ignores a config path it cannot resolve*. The L8 gate compiles a deliberate `println!` and records the observed diagnostic in the plan's own gate log; if the key is inert on the pinned clippy, the entry is dropped and the tidy test stands alone. Independently noted: the lint cannot see `stdout().write_all`, `io::Write` on a raw handle, or `libc::write`, so it could never have carried the migration claim by itself.

### Compatibility
`Arena` / `ComponentPool` / `UnitId` untouched. `LogRing` uses `VmReservation`-backed columns (M13). `golden.ps1:226`'s `[vk-validation]` grep: preserved *and still synchronous*. `vg_occ_split_timing.rs`'s `VB-P1d` parse: preserved and still synchronous, and now under `OUT_LOCK` so the sink cannot interleave inside a line (M24).

---

## Implementation plan

Each rung is independently green (`cargo clippy --all-targets -- -D warnings` + `cargo test --workspace`).

| # | What | Where |
|---|---|---|
| **L0** | Skeleton; `Level`, `LogTarget`, `TargetId`, `targets!` table, `CEILINGS`, the five macros with the 3-gate expansion, `GLOBAL_CEILING` const + `build.rs`. **No sink.** | `src/{level,target,macros}.rs` |
| **L0-gate** | **G1** symbol gate (disabled fixture has no `emit_impl` reference; armed fixture does). **G4** three-way side-effect probe: (a) compile-ceiling-below with runtime armed to Trace ⇒ 0; (b) both armed ⇒ **1000**; (c) runtime ceiling `Off` ⇒ 0. Debug *and* release. **G2** separate build leg `BOYKO_LOG_MAX_LEVEL=off`. | `tests/gates_disabled.rs`, CI leg |
| **L1** | `Lane` (3 partitions), layout asserts, load-then-CAS claim, retire, wrap protocol, `LogValue`/`LogArgs`, `dsp!`, drop counting, `MAX_RECORD_BYTES` runtime check. | `src/{lane,record,site}.rs` |
| **L1-gate** | Per-thread zero-alloc gate; loom model of claim/retire + cursor pair; wrap-boundary proptest; overflow test asserting `dropped > 0`; **G3** `.bss` section gate; **G5** distinct-`decode`-symbol upper bound (N31). | `tests/`, `scripts/section_gate.ps1` |
| **L2** | `codes!`, `DiagInfo`, dense `code_idx`, the three code newtypes, `docs/diagnostics/` seeded with the 9 grandfathered codes + the `B9003` gap note, `explain()`. | `src/codes.rs`, `docs/diagnostics/` |
| **L2-gate** | The **seven** registry checks (integration test). Each must be **shown red once** against a deliberately broken registry; the observed failure text is recorded in the plan's gate log. | `tests/code_registry.rs` |
| **L3** | `sync_out.rs` (`OUT_LOCK`, `report!`, `write_oracle_line`), `tsc.rs`, sink thread with adaptive park, staged drain, console sink → stderr, `flush`/`shutdown` with `SINK_STATE`, panic-hook chaining. | `src/{sync_out,tsc,sink/*}.rs` |
| **L3-gate** | Flush-without-consumer returns `NoConsumer` immediately; flush-timeout returns within 2 s; shutdown detaches rather than joining; **`sink_sustained_rate`** finds the drop knee (M19); **M24** concurrency test — sink flooding while `report!` prints, parser must still resolve. | `tests/`, `benches/` |
| **L4** | File sink + size cap (`W0103`), rate limiter, `LOG-CENSUS` incl. `UNPROVEN(lossy)`, `SinkMode::Manual`. | `src/{sink/file,rate}.rs` |
| **L4-gate** | `Once` steady state performs **no store** (assembly/`perf` check + a shared-line contention bench); census reports `UNPROVEN` at 0 records **and** at `dropped > 0`. | `tests/`, `benches/` |
| **L5** | ECS seam: `LogPlugin`, `LogRing` on `VmReservation`, `LogStats`, `log_drain_system`. | `crates/boyko_ecs/.../log/` |
| **L5-gate** | **P1** perturbation gate: logger-on vs logger-off frame time, ABBA-counterbalanced, interleaved zero control, same sitting; `NOT RESOLVED` if inside the floor (M20). | `crates/boyko_app/tests/` |
| **L6** | Migrate `boyko_ecs` + `boyko_threadpool`; `W1501`, `B0002` normalisation, `W0701`, `W0501`/`B0502`, `E0201`. | as tabled |
| **L7** | Migrate `boyko_rhi_vulkan` **except the messenger**; `E2101`; `W2102` ungated in release; census wiring. | as tabled |
| **L7-gate** | **G7**, two-sided: `E2101` fires on this machine on every validation-on run, and does **not** fire in a fixture where the node is chained. Channel liveness is proved by an **ordinary validation error from a deliberately invalid call** — the historical `mip_levels: 12` on a 512×512 image — with the **baseline of 19 messages accounted for**. A forced *hazard* is explicitly **not** the control: this machine has been measured unable to produce `SYNC-HAZARD` (M25). | `crates/boyko_rhi_vulkan/tests/` |
| **L8a** | Migrate `boyko_render`, `boyko_image`, `boyko_serialize`, `boyko_physics`. | ledger |
| **L8b** | Migrate `boyko_app`; measurement lines → `report!`. | ledger |
| **L8c** | Ledger reaches zero unclassified sites; enable `print_census.rs`; run the clippy `disallowed-macros` canary and record the result. | `tests/`, `clippy.toml` |
| **L9** | `boyko_ui` console widget over `LogRing`. **Deferred to the UI plan**; only the `LogRing` contract is fixed here. | `crates/boyko_ui/` |

---

## Metrics and validation

### Benchmarks (`crates/boyko_log/benches/emit.rs`, criterion, `harness = false`)
Every row runs against a control **in the same sitting**.

| Bench | Target | Control |
|---|---|---|
| `log_disabled_runtime` | ≤ 3 ns | the same site enabled |
| `log_enabled_0args` / `_2u32` / `_str32` | ≤ 15 / 20 / 30 ns | runtime-disabled |
| `log_enabled_rate_once_fired` | ≤ 5 ns, **and no store** | `Every` policy |
| `sink_sustained_rate` | finds the drop knee; reports records·s⁻¹ | zero-record idle sink |
| `lane_padding_ablation` | padded+cached vs padded-only vs neither | — |
| `frame_time_logger_on_off` (gate P1) | not resolvable above the floor | interleaved zero control, ABBA |

**`log_disabled_compile` is deleted** (B7). A compile-disabled site optimises to nothing and the loop body is empty, so the bench measured the control against the control; it could not go red, and adding `black_box` around the arguments would falsify the very "0 argument evaluations" property under test. The property is proved by **G1** (symbol gate) and **G4** (side-effect probe), both of which have named red states.

### Mandatory tests
1. **G4 — three-way gate separability** (§L0-gate). Each gate has its own red state; the enabled leg must reach **1000**, not merely `0` when disabled.
2. **G1 — symbol gate.** Disabled fixture: no `emit_impl` symbol. Armed fixture: symbol present. Red state: delete a gate.
3. **Zero allocations on the producer path**, via a **per-thread** counting allocator (`thread_local! { static N: Cell<u64> = const { Cell::new(0) } }` — const-init, no `Drop`, no TLS registration, no allocation). This covers `SinkMode::Thread`, which the process-global counter structurally cannot: `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK` after observing a negative delta from a sibling thread, and a permanently resident sink thread cannot be serialised by a test-local lock (M18). The process-global variant is retained as a second, `Manual`-mode gate, with its limitation stated.
4. **Overflow drops and counts** — fill a lane, assert `dropped > 0`, exactly one `W0102` per drain with matching counts.
5. **Error reserve** — flood with `Trace`, assert a subsequent `Error` still lands.
6. **Wrap protocol** — proptest over record sizes crossing every tail offset in `LANE_BYTES-32 ..= LANE_BYTES`; assert no byte is written outside the lane (poison the neighbouring lane's guard bytes and check them), and that producer and consumer agree on every PAD.
7. **Staged drain under a LIVE producer** (B1's red state) — a producer running at full rate while the sink drains; assert every decoded record is byte-identical to what was offered. **v1's design fails this test; v1's tests never ran it, because both drove a quiesced producer.**
8. **Lane claim/retire** — 200 short-lived threads against 128 lanes; assert every lane eventually returns to `FREE`, **and** assert `Warn`/`Error` from unlaned threads reached the synchronous fallback (M26). Reclaim is asynchronous, so the assertion is "eventually, within a bounded flush", not "immediately".
9. **Flush without a consumer** returns `NoConsumer` immediately; **flush timeout** returns within 2 s with `E0105`; **shutdown** detaches on timeout with `E0108`.
10. **Panic hook flushes** — `catch_unwind` around a panic after an `error!`.
11. **Registry: the seven checks**, each shown red once during development.
12. **Rate policies** — `Once` (incl. the no-store property), `EveryN`, `MinInterval`, `suppressed_since_last`.
13. **Census** — `UNPROVEN` at 0 records **and** `UNPROVEN(lossy)` at `dropped > 0`.
14. **Miri (Tree Borrows)** — ring, claim CAS, typed header round-trip incl. `*const LogSite` provenance, staged copy.
15. **loom** — claim/retire and the cursor pair. (Loom *release* binaries crash at startup on this box, pre-existing; run loom in debug.)
16. **Machine-API preservation, concurrent** (M24) — `report!` output byte-identical to today's `VB-P1d`/`VB-P4` lines **while the sink is flooding the same merged stream**; `vg_occ_split_timing.rs`'s parser must still resolve. Verified target: `golden.ps1:201` merges with `2>&1`, and `vg_occ_split_timing.rs` parses that merged stream.
17. **`[vk-validation]` prefix preserved and synchronous** — `golden.ps1`'s grep matches, and the message is on the wire before the frame returns.

### Property-based
- Random `(level, target, arg-tuple)` sequences round-trip byte-identically through `encode`/`decode`.
- Random fill/drain interleavings: `emitted == drained + dropped`, exactly, always.

### `debug_assert!` invariants
`len <= MAX_RECORD_BYTES`; `len == HEADER_BYTES + args.encoded_len()`; `write - read <= LANE_BYTES`; `MY_LANE < MAX_LANES || == NONE`; `!IN_EMIT.replace(true)` (re-entrancy); `drain()` only under `Manual`; `TargetId < MAX_TARGETS`; `code_idx < MAX_CODES`; `boot()` at most once; `codes!` strictly increasing (also a compile-time `const _: () = assert!`).

---

## Edge cases

| # | Case | Behaviour |
|---|---|---|
| E1 | Log before `boot()` | `CEILINGS` is `.bss`-zero = `Off`; one L1 load, not-taken branch. Correct and free. |
| E2 | Log after shutdown | Lanes accept; nothing drains; `dropped` climbs; census reports lossy. Shutdown flushes first. |
| E3 | Record over `MAX_RECORD_BYTES` | **Runtime** check, every profile: dropped, `TOO_LARGE` flag, counted. Not a debug panic reachable from safe code (N29). |
| E4 | Ring exactly full | One-slot-reserved convention distinguishes full from empty without a third variable. |
| E5 | Tail too short for a record | PAD record if `tail >= HEADER_BYTES`, else the shared implicit-wrap rule. Both sides apply the same rule (B3). |
| E6 | 129th concurrent logging thread | `Warn`/`Error` go to the synchronous fallback; lower levels count into `UNLANED_DROPPED`; census reports it (M26). |
| E7 | Thread dies mid-record | Impossible to publish a partial record: header+payload write and the `Release` store are straight-line with no yield point. A thread killed between them leaves `write` unmoved; the bytes are overwritten. |
| E8 | `tsc` wrap | 64-bit invariant TSC at ~3 GHz wraps in ~195 years. Not handled; stated. |
| E9 | Non-monotonic clock across sockets | Merge order degrades to approximate — already the documented property. Single-socket assumption stated. |
| E10 | `&str` > 256 B | Truncated, `STR_TRUNCATED`, sink appends `…[truncated]`. |
| E11 | Two engine targets claim one ID | **Does not compile** (Decision 15). Downstream collision ⇒ `boyko-E0104` at boot, naming both. |
| E12 | File sink hits `max_bytes` | One `boyko-W0103`; writing stops; other sinks continue. No rotation in v1. |
| E13 | Validation storm | Synchronous channel, allocation-free, one `write_all` per message under `OUT_LOCK` (Decision 9b). Never dropped. |
| E14 | Panic inside a sink | The sink catches, direct-writes, continues. A sink must not kill the process. |
| E15 | `flush()` from two threads | Distinct epochs via `AcqRel fetch_add`; both return. |
| E16 | Drop order at shutdown | `shutdown()` → `flush()` → exit flag → unpark → bounded spin on `SINK_EXITED` → detach on timeout → sinks close. Idempotent; safe from `App` teardown, the panic hook and process exit. |

---

## Open questions

1. **`report!` schema.** The tree has a flat-TOML + `schema_version` artifact convention (`vb_probe_dump.rs:183-194`) and **no timing output uses it**. Should `report!` gain a second schema-versioned TOML form alongside the byte-frozen text? SCOPE call; this plan preserves the text verbatim and adds nothing.
2. **`--explain` delivery.** This plan embeds a one-line `summary` and requires `docs/diagnostics/<code>.md` (check 2). It does not add a `boyko-explain` binary. rustc's registry stays honest partly *because* three consumers read one table; we have two. Does the owner want the third?
3. **`.bss` budget.** 128 × 16 KiB = 2 MiB reserved, demand-zero, ~128-192 KiB typical resident, plus 32 KiB `RATE`. Acceptable, or default to 64 × 8 KiB (512 KiB)?
4. **Sink thread in shipping builds.** One extra OS thread, idle-parking at 125 Hz. Gate P1 measures its perturbation; if P1 comes back `NOT RESOLVED` the thread is free, and if it does not, the default flips to `Manual` driven by the runner. Confirm the thread is acceptable pending P1.
5. **The `15xx` / `90xx` block split** is grandfathered and tidying is forbidden. Confirm — renumbering breaks the book, the `#[should_panic]` assertions and the never-reuse rule at once.
6. **The one-line RHI fix for sync-validation** (`pLayerName = "VK_LAYER_KHRONOS_validation"`) is deliberately not in this plan. It has a large blast radius: sync-validation coming alive would surface real hazards and could turn every golden run red. Pull it into L7, or keep it a separate RHI item coded by `E2101`?
7. **`OUT_LOCK` as a registered hot-path exception.** It is a spin lock taken by `report!` (~20 cold sites), the validation messenger (off by default) and the lane-exhaustion fallback. Confirm the `docs/HOT-PATH-EXCEPTIONS.md` entry rather than an attempt to make it lock-free — a lock-free ordered multi-writer line channel is strictly more machinery for a path that is cold by construction.

---

## Checklist

**Structure** — goal in perf+functional terms ✔ · concrete targets with named red-state controls ✔ · every decision justified via perf/cache/parallelism ✔ · alternatives rejected with reasons ✔ · trade-offs listed ✔
**Data structures** — every field typed + commented ✔ · `repr`/`align`/`packed`/`transparent` where it matters ✔ · three-partition cache-line split ✔ · sizes pinned by `const _: () = assert!` ✔ · false-sharing padding specified for `Lane` and `RateSlot` ✔
**API** — minimal ✔ · no internal types in signatures ✔ · lifetimes trivial ✔ · no `dyn` in the hot path ✔ · generics where specialisation is needed (`emit_impl<A: LogArgs>`) ✔
**Multithreading** — model explicit ✔ · every atomic's ordering stated ✔ · the one sync point bounded **and** short-circuited when no consumer exists ✔ · partitioning described ✔ · `Send`/`Sync` consistent ✔ · race-freedom argued, including the single-thread re-entrant case ✔
**Correctness** — 16 edge cases ✔ · lane-`owner` protocol replaces generation checks ✔ · drop order (E16) ✔ · `unsafe` invariants in the `Sync` SAFETY block and per algorithm ✔
**Integration** — machine-generated ledger, not a hand table ✔ · API changes explicit ✔ · `Arena`/`ComponentPool`/`UnitId` untouched ✔ · 13 rungs ✔
**Validation** — 17 tests ✔ · proptests ✔ · benches with controls ✔ · `debug_assert!` invariants ✔ · every gate has a named red state and must be shown red once ✔

---

## Findings disposition

| # | Finding | Disposition | Where in v2 |
|---|---|---|---|
| **B1** | Drain frees the ring before it reads it | **FOLDED** | §Algorithms C — staged typed-header + payload copy, `read.store` moved after staging; test 7 drives a **live** producer (v1's tests could not see it) |
| **B2** | `const ENCODED_LEN` undefined for `&str`/`fmtv` | **FOLDED** | §Decision 1a — `encoded_len(&self)`; `fmtv` deleted (see B4) |
| **B3** | Ring never wraps | **FOLDED** | §Algorithms A5 — non-straddling records, shared wrap rule, PAD sentinel; test 6 proptests every tail offset |
| **B4** | `fmtv` makes emit re-entrant on one thread | **FOLDED** | §Decision 13 — `fmtv` deleted, `dsp!` runs in argument position; SAFETY clause 1b; `IN_EMIT` debug guard |
| **B5** | L0 gate cannot fail; probe is one-sided; gates not separable | **FOLDED** | §L0-gate G4 — three separable legs, enabled leg must reach 1000; G2 separate build leg for `GLOBAL_CEILING` |
| **B6** | Registry check 3 vacuous; check 5 self-defeating; no corpus assertion | **FOLDED** | §Decision 6 — check 3 scans `.rs` only, excludes `docs/` and `codes.rs`; allowlist moved to a data file excluded from its own scan; check 0 asserts corpus size + a pinned sentinel |
| **B7** | `log_disabled_compile` bench cannot fail | **FOLDED** | §Metrics — bench deleted; replaced by G1 symbol gate + G4 probe |
| **B8** | Async `[vk-validation]` manufactures a false-clean gate | **FOLDED** | §Decision 9b — migration **withdrawn**, channel stays synchronous; E12 conflict resolved explicitly; only the allocation is removed |
| **M9** | Consumer writes land on the producer's line | **FOLDED** | §Data structures `Lane` — third partition for statistics + `owner`; new `offset_of` assert |
| **M10** | Claim scan CASes every occupied lane | **FOLDED** | §Algorithms B — `load`-then-CAS + spread start index |
| **M11** | `Once` still does a per-frame RMW | **FOLDED** | §Decision 8 — steady state is a pure `Relaxed` load, no store; suppressed count for `Once` explicitly not reported |
| **M12** | `RATE[code]` indexes 512 with numbers up to 9101 | **FOLDED** | §Decision 8 / §`LogSite` — dense `code_idx` carried in the site |
| **M13** | `LogRing`'s `Box<[u8]>` is a heap side-store | **FOLDED** | §Data structures — `VmColumn`-backed, engine storage |
| **M14** | `LogFilter` is a divergent mirror with a `dirty` bool | **FOLDED** | §Decision 14 — `LogFilter` deleted; `CEILINGS` is the single owner |
| **M15** | Target IDs are an honour system | **FOLDED** | §Decision 15 — central `targets!` table, compile-time uniqueness for 0..=95; check 7; `TypeIntern` non-applicability recorded |
| **M16** | `flush()` stalls 2 s with no consumer; `join` has no timeout | **FOLDED** | §Decision 12 / §Algorithms D — `SINK_STATE` short-circuit returns `NoConsumer`; shutdown spins bounded then **detaches**; `E0108` |
| **M17** | `LogHandle: !Send` conflicts with `LogPlugin`; nothing owns it | **FOLDED** | §Decision 12 — handle deleted; `boot`/`shutdown` free functions over statics |
| **M18** | Zero-alloc gate cannot cover the shipping config | **FOLDED** | §Metrics test 3 — per-thread const-init counting allocator covers `Thread` mode; process-global variant kept for `Manual` with its limitation stated |
| **M19** | No sustained-rate number; sizing deferred | **FOLDED** | §Decision 10 — design number stated (≥ 500 K rec·s⁻¹ aggregate), adaptive park, `trace!`-in-a-loop declared lossy; L3 gate `sink_sustained_rate` must find the knee |
| **M20** | No perturbation control for the logger itself | **FOLDED** | §Metrics gate P1 (L5-gate) — ABBA + interleaved zero control, `NOT RESOLVED` inside the floor |
| **M21** | No genuinely-off configuration; `.bss` claim ungated | **FOLDED** | §Decision 3 — `LANE_ARRAY_LEN = 0` when `GLOBAL_CEILING == Off`, no thread, no hook; gates G2 (off build) and G3 (section check); "on" floor stated verbatim |
| **M22** | Integration table omits ~40 production sites | **FOLDED** | §Integration — machine-generated `PRINT-CENSUS.md` ledger; measured 179/36 classified into five dispositions; migration split L6/L7/L8a/L8b/L8c; per-crate allows, not per-site |
| **M23** | `disallowed-macros` claimed without a liveness proof | **FOLDED** | §Integration Enforcement — in-repo tidy test is primary; clippy is secondary and only after a shown-red canary; `clippy.toml:21-25`'s silent-ignore finding cited; the lint's blind spots named |
| **M24** | `report!` and the sink share one fd unsynchronised | **FOLDED** | §Decision 9/9b — both take `OUT_LOCK`; console sink → stderr; test 16 runs the **concurrent** state |
| **M25** | L7 control is one this machine cannot show | **FOLDED** | §L7-gate — forced hazard replaced by an ordinary validation error (`mip_levels: 12`) with the baseline-19 accounted; `E2101` made a two-sided gate |
| **M26** | 129th-thread loss is designed in; test 5 one-sided | **FOLDED** | §Decision 5 / E6 — synchronous fallback for `Warn`/`Error` on claim failure; test 8 asserts the fallback delivered and states reclaim is eventual |
| **N27** | `option_env!` evaluation site unspecified | **FOLDED** | §Decision 2 — `GLOBAL_CEILING` is a `const` in `boyko_log`, referenced as `$crate::GLOBAL_CEILING`; never expanded in a caller crate |
| **N28** | `AtomicPtr<u8>` loses the `&str` length | **FOLDED** | §Data structures — `AtomicPtr<TargetInfo>` publishes name+len with one pointer |
| **N29** | `MAX_RECORD_BYTES` reachable from safe code | **FOLDED** | §Data structures / E3 — raised to 2048 **and** checked at runtime in every profile; dropped + `TOO_LARGE` + counted, not a debug panic |
| **N30** | `W0101` has no showable red state | **FOLDED** | §Decision 11 — written down as uncontrolled and listed in `tests/untested_codes.txt` with that reason |
| **N31** | `decode` monomorphisation count is an unmeasured claim | **FOLDED** | §L1-gate G5 — distinct-`decode`-symbol count asserted against an upper bound; the prose claim is removed |
| **N32** | Header duplicates `code`/`level` | **FOLDED** | §Data structures — header shrunk 24 → **20 B packed**; `code`, `level` and `lane` removed (site holds two, the drain knows the third) |

**Refuted: none.** Every finding was either a defect in v1's own pseudocode, a gate that could not fail, or a claim v1 made without a control. The design's core — deferred formatting, POD record + `&'static LogSite`, `.bss` statics with `Off == 0`, per-lane SPSC, the code registry, `report!` as a separate synchronous channel, and the `UNPROVEN`-not-`clean` census — is carried forward unchanged, as the review directed.